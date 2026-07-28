import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  BrowserMatchSimulation,
  selectSnapshotProjectiles,
} from "../src/browser-match-runtime.ts";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const bytes = await readFile(join(root, "generated", "aetherloom-worker-match.wasm"));
const roomSource = await readFile(join(root, "src", "browser-match-room.ts"), "utf8");
const module = new WebAssembly.Module(bytes);

test("a first socket validates the authoritative core before consuming or accepting", () => {
  const start = roomSource.indexOf("private async acceptPlayerWebSocket");
  const end = roomSource.indexOf("private ensureSimulation", start);
  const source = roomSource.slice(start, end);
  const simulation = source.indexOf("const simulation = this.ensureSimulation();");
  const consumeTicket = source.indexOf("INSERT INTO consumed_join_nonces");
  const acceptSocket = source.indexOf("this.ctx.acceptWebSocket");
  assert.ok(simulation >= 0);
  assert.ok(simulation < consumeTicket);
  assert.ok(consumeTicket < acceptSocket);
  assert.equal(
    source.indexOf(
      "this.ensureSimulation()",
      simulation + "const simulation = this.ensureSimulation();".length,
    ),
    -1,
    "the core must not be initialized for the first time after accepting the socket",
  );
});

test("snapshot budgeting never starves the viewer's newest projectiles", () => {
  const projectiles = Array.from({ length: 40 }, (_, index) => ({
    entityKey: `0:${index + 1}`,
    ownerPlayerId: index === 0 || index === 39 ? 2 : index % 2,
    teamId: index % 2,
    xCm:
      index === 1
        ? 1
        : index === 3 || index === 4
          ? 10_005
          : 10_000 + index,
    yCm: 0,
    zCm: 0,
    velocityXCmPerTick: 0,
    velocityYCmPerTick: 0,
    velocityZCmPerTick: 0,
    yaw: 0,
    pitch: 0,
    lifetimeTicks: index === 0 ? 1 : index + 1,
    flags: 0,
  }));
  const selected = selectSnapshotProjectiles(
    projectiles,
    { playerId: 2, xCm: 0, yCm: 0, zCm: 0 },
    new Set(["0:4"]),
    31,
  );
  assert.deepEqual(
    selected.slice(0, 2).map((projectile) => projectile.entityKey).sort(),
    ["0:1", "0:40"],
  );
  assert.equal(selected.length, 31);
  assert.equal(
    selected.some((projectile) => projectile.entityKey === "0:2"),
    true,
    "the nearest hostile shot must survive newer distant shots",
  );
  assert.equal(selected.some((projectile) => projectile.entityKey === "0:39"), false);
  assert.ok(
    selected.findIndex((projectile) => projectile.entityKey === "0:4") <
      selected.findIndex((projectile) => projectile.entityKey === "0:5"),
    "an already visible projectile must win an otherwise equivalent tie",
  );
});

test("browser-match Wasm export and snapshot pin the same version-three ABI", () => {
  const instance = new WebAssembly.Instance(module, {});
  const exports = instance.exports as {
    memory: WebAssembly.Memory;
    worker_match_abi_version: () => number;
    worker_match_init: (seedLow: number, seedHigh: number) => number;
    worker_match_snapshot_ptr: () => number;
    worker_match_snapshot_len: () => number;
  };
  assert.equal(exports.worker_match_abi_version(), 3);
  assert.equal(exports.worker_match_init(0, 0), 0);
  const words = new Int32Array(
    exports.memory.buffer,
    exports.worker_match_snapshot_ptr(),
    exports.worker_match_snapshot_len(),
  );
  assert.equal(words[1], exports.worker_match_abi_version());
});

test("browser-match Wasm advances the real fixed-spawn authoritative core", () => {
  const match = new BrowserMatchSimulation(module, 7, 0);
  match.addHuman(0, 0);
  match.addHuman(1, 1);
  const initial = match.snapshot();
  assert.deepEqual(
    initial.players.map(({ playerId, xCm, zCm, health }) => ({
      playerId,
      xCm,
      zCm,
      health,
    })),
    [
      { playerId: 0, xCm: -250, zCm: 0, health: 100 },
      { playerId: 1, xCm: 250, zCm: 0, health: 100 },
    ],
  );

  match.submitInput(0, 2_047, 0, 0, 0, 0, false, null);
  const first = match.advanceTick();
  const second = match.advanceTick();
  const stale = match.advanceTick();
  assert.equal(first.tick, 1);
  assert.equal(first.players[0]?.xCm, -240);
  assert.equal(second.players[0]?.xCm, -230);
  assert.equal(
    stale.players[0]?.xCm,
    -230,
    "an open socket without a fresh frame must auto-neutralize after two ticks",
  );
});

test("browser-match Wasm exposes authoritative projectile damage events", () => {
  const match = new BrowserMatchSimulation(module, 11, 0);
  match.addHuman(0, 0);
  match.addHuman(1, 1);
  match.submitInput(0, 0, 0, 0, 0, 0, true, 0);

  let damaged = false;
  for (let tick = 0; tick < 32; tick += 1) {
    const snapshot = match.advanceTick();
    if (snapshot.events.some((event) => event.kind === 4)) {
      damaged = true;
      assert.ok((snapshot.players[1]?.health ?? 100) < 100);
      break;
    }
    match.submitInput(0, 0, 0, 0, 0, 0, false, null);
  }
  assert.equal(damaged, true);
});

test("browser-match Wasm exposes authoritative Mend without a projectile", () => {
  const match = new BrowserMatchSimulation(module, 12, 0);
  match.addHuman(0, 0);
  match.addHuman(1, 1);
  match.submitInput(0, 0, 0, 0, 0, 0, true, 0);
  for (let tick = 0; tick < 32; tick += 1) {
    const snapshot = match.advanceTick();
    if ((snapshot.players[1]?.health ?? 100) < 100) break;
    match.submitInput(0, 0, 0, 0, 0, 0, false, null);
  }
  assert.ok((match.snapshot().players[1]?.health ?? 100) < 100);
  match.submitInput(1, 0, 0, 0, 32_768, 0, true, 7);
  const healed = match.advanceTick();
  assert.equal(healed.players[1]?.health, 100);
  assert.equal(healed.players[1]?.cooldownTicks.length, 13);
  assert.equal(healed.players[1]?.cooldownTicks[0], 0);
  assert.equal(healed.players[1]?.cooldownTicks[7], 896);
  assert.equal(
    healed.players[1]?.cooldownTicks.every(
      (cooldown, spell) => spell === 7 || cooldown === 0,
    ),
    true,
  );
  assert.equal(healed.projectiles.length, 0);
  assert.ok(healed.events.some((event) => event.kind === 3 && event.data[0] === 7));
});

test("browser-match Wasm keeps vertical flight and pitch authoritative", () => {
  const match = new BrowserMatchSimulation(module, 17, 0);
  match.addHuman(0, 0);
  match.submitInput(0, 0, 0, 2_047, 0, 8_192, false, null);
  const climbed = match.advanceTick();
  assert.equal(climbed.players[0]?.yCm, 6);
  assert.equal(climbed.players[0]?.pitch, 8_192);

  match.submitInput(0, 0, 0, -2_047, 0, -8_192, false, null);
  const descended = match.advanceTick();
  assert.equal(descended.players[0]?.yCm, 0);
  assert.equal(descended.players[0]?.pitch, -8_192);
});
