#!/usr/bin/env node

import { randomBytes } from "node:crypto";

import {
  decodeSnapshotFrame,
  encodeInputFrame,
  nextPacedDeadline,
} from "../../site/multiplayer.js";
import {
  mintHmacTicket,
  readWorkerSecretBundle,
} from "./smoke-control-plane-authenticated.mjs";

const controlPlane = new URL(process.argv[2] ?? "");
const origin = new URL(process.argv[3] ?? "");
const secretsFile = process.argv[4];
const serviceKeyId = process.argv[5];
const expectedBuild = process.argv[6];
if (controlPlane.protocol !== "https:" || origin.protocol !== "https:") {
  throw new Error("control-plane and origin arguments must use HTTPS");
}
if (
  typeof secretsFile !== "string" ||
  typeof serviceKeyId !== "string" ||
  !/^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$/u.test(serviceKeyId) ||
  !/^[0-9a-f]{32}$/u.test(expectedBuild ?? "") ||
  /^0{32}$/u.test(expectedBuild)
) {
  throw new Error(
    "usage: smoke-browser-multiplayer.mjs <control-plane> <origin> <mode-0600-secrets> <service-key-id> <build-hash>",
  );
}
const room = `smoke-${randomBytes(16).toString("hex")}`;
const workerSecrets = await readWorkerSecretBundle(secretsFile, {
  service: serviceKeyId,
});

function serviceTicket() {
  const now = Math.floor(Date.now() / 1_000);
  return mintHmacTicket(
    {
      v: 1,
      iss: "aetherloom-control-plane",
      aud: "aetherloom-services",
      purpose: "service",
      sub: "browser-multiplayer-smoke",
      iat: now,
      nbf: now - 2,
      exp: now + 120,
      nonce: randomBytes(16).toString("hex"),
      scopes: ["deployment:verify"],
    },
    workerSecrets.service,
  );
}

const healthResponse = await fetch(new URL("/healthz", controlPlane), {
  headers: { origin: origin.origin },
  signal: AbortSignal.timeout(10_000),
});
const health = await healthResponse.json();
if (
  !healthResponse.ok ||
  health?.environment !== "staging" ||
  health?.buildHash !== expectedBuild
) {
  throw new Error("multiplayer smoke target is not the expected staging build");
}

async function join({ resumeToken, joinNonce } = {}) {
  const response = await fetch(
    new URL("/internal/v1/demo/smoke/join", controlPlane),
    {
      method: "POST",
      headers: {
        authorization: `Bearer ${serviceTicket()}`,
        "content-type": "application/json",
        origin: origin.origin,
      },
      body: JSON.stringify({
        room,
        ...(resumeToken === undefined ? {} : { resumeToken }),
        ...(joinNonce === undefined ? {} : { joinNonce }),
      }),
      signal: AbortSignal.timeout(10_000),
    },
  );
  const payload = await response.json();
  if (!response.ok) {
    throw new Error(
      `multiplayer join failed (${response.status}): ${
        payload?.error?.code ?? "unknown_error"
      }`,
    );
  }
  if (
    typeof payload.webSocketUrl !== "string" ||
    typeof payload.ticket !== "string" ||
    payload.tickHz !== 128 ||
    payload.inputHz !== 64 ||
    payload.snapshotHz !== 32 ||
    payload.competitive !== false ||
    payload.progression !== "disabled"
  ) {
    throw new Error("multiplayer join returned an invalid staging contract");
  }
  return payload;
}

function open(joined) {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(joined.webSocketUrl, [
      "aetherloom.v3",
      `aetherloom.auth.${joined.ticket}`,
    ]);
    socket.binaryType = "arraybuffer";
    const timeout = setTimeout(() => {
      socket.close();
      reject(new Error("multiplayer WebSocket did not open in time"));
    }, 10_000);
    socket.addEventListener(
      "open",
      () => {
        clearTimeout(timeout);
        if (socket.protocol !== "aetherloom.v3") {
          socket.close();
          reject(new Error("multiplayer WebSocket selected the wrong protocol"));
          return;
        }
        resolve(socket);
      },
      { once: true },
    );
    socket.addEventListener(
      "error",
      () => {
        clearTimeout(timeout);
        reject(new Error("multiplayer WebSocket failed to connect"));
      },
      { once: true },
    );
  });
}

function observe(socket) {
  const snapshots = [];
  const waiters = new Set();
  socket.addEventListener("message", async (event) => {
    try {
      const payload =
        event.data instanceof Blob ? await event.data.arrayBuffer() : event.data;
      const snapshot = decodeSnapshotFrame(payload);
      snapshots.push(snapshot);
      if (snapshots.length > 256) snapshots.shift();
      for (const waiter of [...waiters]) waiter(snapshot);
    } catch (error) {
      for (const waiter of [...waiters]) waiter(undefined, error);
    }
  });
  return {
    snapshots,
    waitFor(predicate, timeoutMs = 10_000) {
      for (const snapshot of snapshots) {
        if (predicate(snapshot)) return Promise.resolve(snapshot);
      }
      return new Promise((resolve, reject) => {
        const timeout = setTimeout(() => {
          waiters.delete(check);
          reject(new Error("timed out waiting for an authoritative multiplayer snapshot"));
        }, timeoutMs);
        const check = (snapshot, error) => {
          if (error !== undefined) {
            clearTimeout(timeout);
            waiters.delete(check);
            reject(error);
          } else if (snapshot !== undefined && predicate(snapshot)) {
            clearTimeout(timeout);
            waiters.delete(check);
            resolve(snapshot);
          }
        };
        waiters.add(check);
      });
    },
  };
}

function startInput(socket, yaw, observer) {
  let sequence = 1;
  let cast = false;
  let spell = 0;
  let moveVertical = 0;
  let pitch = 0;
  let timer;
  let stopped = false;
  const interval = 1_000 / 64;
  let deadline = performance.now() + interval;
  const send = () => {
    socket.send(
      encodeInputFrame({
        sequence,
        moveX: 0,
        moveY: 0,
        moveVertical,
        yaw,
        pitch,
        cast,
        spell,
        clientClockMs: Date.now() & 0xffff,
        snapshotAck: observer.snapshots.at(-1)?.snapshotSeq ?? 0,
      }),
    );
    cast = false;
    sequence = sequence === 0xffff_ffff ? 1 : sequence + 1;
  };
  send();
  const pace = () => {
    if (stopped) return;
    const now = performance.now();
    if (now >= deadline) {
      send();
      deadline = nextPacedDeadline(deadline, now, interval);
    }
    timer = setTimeout(pace, Math.max(1, deadline - performance.now()));
  };
  timer = setTimeout(pace, interval);
  return {
    castSpell(requestedSpell = 0) {
      spell = requestedSpell;
      cast = true;
    },
    fly(vertical, lookPitch = pitch) {
      moveVertical = vertical;
      pitch = lookPitch;
    },
    stop() {
      stopped = true;
      clearTimeout(timer);
    },
  };
}

const firstJoinNonce = `demo_join_${randomBytes(18).toString("base64url")}`;
const secondJoinNonce = `demo_join_${randomBytes(18).toString("base64url")}`;
const initialJoin = await join({ joinNonce: firstJoinNonce });
const idempotentJoin = await join({ joinNonce: firstJoinNonce });
const firstJoin = await join({ resumeToken: initialJoin.resumeToken });
const secondJoin = await join({ joinNonce: secondJoinNonce });
if (
  initialJoin.accountId !== idempotentJoin.accountId ||
  initialJoin.slot !== idempotentJoin.slot ||
  initialJoin.resumeToken !== idempotentJoin.resumeToken ||
  idempotentJoin.playerCount !== 1 ||
  initialJoin.matchId !== firstJoin.matchId ||
  initialJoin.accountId !== firstJoin.accountId ||
  initialJoin.slot !== firstJoin.slot ||
  initialJoin.resumeToken !== firstJoin.resumeToken ||
  firstJoin.playerCount !== 1 ||
  firstJoin.matchId !== secondJoin.matchId ||
  firstJoin.slot === secondJoin.slot ||
  secondJoin.playerCount !== 2
) {
  throw new Error("multiplayer resume did not preserve one player before allocating the second");
}
const bySlot = [firstJoin, secondJoin].sort((left, right) => left.slot - right.slot);
if (bySlot[0].slot !== 0 || bySlot[1].slot !== 1) {
  throw new Error("fresh multiplayer smoke room did not allocate duel slots zero and one");
}

const firstSocket = await open(firstJoin);
const secondSocket = await open(secondJoin);
const firstObserver = observe(firstSocket);
const secondObserver = observe(secondSocket);
const firstInput = startInput(
  firstSocket,
  firstJoin.slot === 0 ? 0 : 32_768,
  firstObserver,
);
const secondInput = startInput(
  secondSocket,
  secondJoin.slot === 0 ? 0 : 32_768,
  secondObserver,
);

try {
  await Promise.all([
    firstObserver.waitFor((snapshot) => snapshot.playerCount === 2),
    secondObserver.waitFor((snapshot) => snapshot.playerCount === 2),
  ]);
  const casterInput = firstJoin.slot === 0 ? firstInput : secondInput;
  const targetSlot = 1;
  casterInput.castSpell(0);
  const [firstDamage, secondDamage] = await Promise.all([
    firstObserver.waitFor(
      (snapshot) =>
        snapshot.players.some(
          (player) => player.slot === targetSlot && player.health < 100,
        ) &&
        snapshot.events.some((event) => event.type === 4) &&
        snapshot.events.some((event) => event.type === 10 && event.spell === 0),
    ),
    secondObserver.waitFor(
      (snapshot) =>
        snapshot.players.some(
          (player) => player.slot === targetSlot && player.health < 100,
        ) &&
        snapshot.events.some((event) => event.type === 4) &&
        snapshot.events.some((event) => event.type === 10 && event.spell === 0),
    ),
  ]);
  const health = firstDamage.players.find((player) => player.slot === targetSlot)?.health;
  if (
    health === undefined ||
    secondDamage.players.find((player) => player.slot === targetSlot)?.health !== health
  ) {
    throw new Error("multiplayer clients disagreed on authoritative damage");
  }
  const targetInput = secondJoin.slot === targetSlot ? secondInput : firstInput;
  const targetObserver = secondJoin.slot === targetSlot
    ? secondObserver
    : firstObserver;
  targetInput.castSpell(7);
  const healed = await targetObserver.waitFor(
    (snapshot) =>
      snapshot.players.some(
        (player) => player.slot === targetSlot && player.health === 100,
      ) &&
      snapshot.events.some(
        (event) => event.type === 3 && event.spell === 7,
      ),
  );
  if (
    healed.players.find((player) => player.slot === targetSlot)?.health !== 100
  ) {
    throw new Error("authoritative Mend did not restore the damaged target");
  }
  const flightInput = firstJoin.slot === 0 ? firstInput : secondInput;
  const flightObserver = firstJoin.slot === 0 ? firstObserver : secondObserver;
  flightInput.fly(127);
  const climbed = await flightObserver.waitFor((snapshot) =>
    (snapshot.players.find((player) => player.slot === 0)?.ycm ?? 0) >= 64);
  const climbedY = climbed.players.find((player) => player.slot === 0)?.ycm ?? 0;
  flightInput.fly(-127);
  const descended = await flightObserver.waitFor((snapshot) => {
    const y = snapshot.players.find((player) => player.slot === 0)?.ycm;
    return y !== undefined && y < climbedY;
  });
  flightInput.fly(0);
  const descendedY = descended.players.find((player) => player.slot === 0)?.ycm;
  if (descendedY === undefined || descendedY >= climbedY) {
    throw new Error("authoritative multiplayer altitude did not descend");
  }
  await flightObserver.waitFor(
    (snapshot) => snapshot.tick >= firstDamage.tick + 64,
  );
  const compactPitch = 63;
  const expectedPitch = Math.round((compactPitch * 16_384) / 127);
  flightInput.fly(0, compactPitch);
  flightInput.castSpell(0);
  const [firstPitched, secondPitched] = await Promise.all([
    firstObserver.waitFor((snapshot) =>
      snapshot.projectiles.some(
        (projectile) =>
          projectile.owner === 0 &&
          projectile.pitch === expectedPitch,
      )),
    secondObserver.waitFor((snapshot) =>
      snapshot.projectiles.some(
        (projectile) =>
          projectile.owner === 0 &&
          projectile.pitch === expectedPitch,
      )),
  ]);
  flightInput.fly(0, 0);
  if (
    !firstPitched.projectiles.some(
      (projectile) => projectile.owner === 0 && projectile.pitch === expectedPitch,
    ) ||
    !secondPitched.projectiles.some(
      (projectile) => projectile.owner === 0 && projectile.pitch === expectedPitch,
    )
  ) {
    throw new Error("multiplayer clients disagreed on authoritative projectile pitch");
  }
  console.log(
    JSON.stringify({
      status: "ok",
      room,
      matchId: firstJoin.matchId,
      players: 2,
      tickHz: 128,
      inputHz: 64,
      snapshotHz: 32,
      targetHealth: health,
      climbedY,
      descendedY,
      pitchedProjectilePitch: expectedPitch,
      bothClientsObservedDamage: true,
      authoritativeMend: true,
      idempotentFreshJoin: true,
      stableResume: true,
      buildHash: expectedBuild,
    }),
  );
} finally {
  firstInput.stop();
  secondInput.stop();
  firstSocket.close(1000, "smoke complete");
  secondSocket.close(1000, "smoke complete");
}
