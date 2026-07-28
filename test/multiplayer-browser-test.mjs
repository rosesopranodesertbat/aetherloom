import assert from 'node:assert/strict';
import { webcrypto } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { runInNewContext } from 'node:vm';
import {
  CAST_EVENT_TYPE,
  DAMAGE_EVENT_TYPE,
  INPUT_FRAME_BYTES,
  JoinNonceStore,
  MAX_SNAPSHOT_FRAME_BYTES,
  PROJECTILE_IMPACT_EVENT_TYPE,
  ProtocolError,
  ResumeOwnership,
  ResumeTokenStore,
  SingleFlight,
  VIEWER_COOLDOWN_BYTES,
  authoritativeCooldownReadyAt,
  bilinearHeight,
  composeTemplate,
  decodeSnapshotFrame,
  demoResumeToken,
  encodeInputFrame,
  firstPersonCamera,
  firstPersonMuzzle,
  gamepadLookDelta,
  isJoinNonce,
  isNewerSequence,
  isResumeToken,
  joinAttemptKey,
  movementToWorld,
  nextPacedDeadline,
  normalizeControlPlane,
  pendingInputDisplacement,
  predictedBrowserAxisDisplacement,
  predictViewerPosition,
  randomJoinNonce,
  resumeSessionKey,
  sequenceAcknowledges,
  shapeFlightInput,
  socketCloseMessage,
  validateRoom,
  viewerProjectilePresentation,
  wirePitchToRadians,
  wireYawToCoreRadians,
  wireYawToRenderRadians,
  wrappedClockDelta,
  yawAfterLookDelta,
} from '../site/multiplayer.js';

const root = fileURLToPath(new URL('..', import.meta.url));
const browserMatchRoomSource = await readFile(
  new URL('../server/cloudflare/src/browser-match-room.ts', import.meta.url),
  'utf8',
);
const gatewaySource = await readFile(
  new URL('../server/cloudflare/src/gateway.ts', import.meta.url),
  'utf8',
);
const demoWebSocketRouteSource = gatewaySource.slice(
  gatewaySource.indexOf('async function routeDemoBrowserWebSocket('),
  gatewaySource.indexOf('async function advanceIsland('),
);

function mapStorage(entries = new Map()) {
  return {
    getItem: (key) => entries.get(key) ?? null,
    setItem: (key, value) => entries.set(key, value),
    removeItem: (key) => entries.delete(key),
  };
}

class DeterministicLockManager {
  constructor() {
    this.held = new Set();
    this.requests = [];
  }

  request(name, options, callback) {
    this.requests.push({ name, options });
    if (this.held.has(name)) return Promise.resolve(callback(null));
    this.held.add(name);
    return Promise.resolve(callback({ name, mode: options.mode }))
      .finally(() => this.held.delete(name));
  }
}

function snapshotFixture() {
  const byteLength = 24 + 2 * 30 + VIEWER_COOLDOWN_BYTES + 24 + 20;
  const bytes = new Uint8Array(byteLength);
  const view = new DataView(bytes.buffer);
  view.setUint8(0, 3);
  view.setUint8(1, 2);
  view.setUint16(2, byteLength, true);
  view.setUint32(4, 0x1020_3040, true);
  view.setUint32(8, 0xffff_fffe, true);
  view.setUint16(12, 0xfff0, true);
  view.setUint8(14, 2);
  view.setUint8(15, 2);
  view.setUint8(16, 1);
  view.setUint8(17, 1);
  view.setUint16(18, 3, true);
  view.setUint32(20, 0x1234_5678, true);

  let offset = 24;
  view.setUint8(offset, 2);
  view.setUint8(offset + 1, 1);
  view.setUint8(offset + 2, 1);
  view.setUint8(offset + 3, 3);
  view.setInt32(offset + 4, -1_234, true);
  view.setInt32(offset + 8, 222, true);
  view.setInt32(offset + 12, 5_678, true);
  view.setUint16(offset + 16, 0xff00, true);
  view.setInt16(offset + 18, -4_096, true);
  view.setUint16(offset + 20, 75, true);
  view.setUint16(offset + 22, 100, true);
  view.setUint16(offset + 24, 9, true);
  view.setUint32(offset + 26, 100, true);

  offset += 30;
  view.setUint8(offset, 5);
  view.setUint8(offset + 1, 2);
  view.setUint8(offset + 2, 1);
  view.setUint8(offset + 3, 0);
  view.setInt32(offset + 4, 7_654, true);
  view.setInt32(offset + 8, 333, true);
  view.setInt32(offset + 12, -3_210, true);
  view.setUint16(offset + 16, 0x0100, true);
  view.setInt16(offset + 18, 2_048, true);
  view.setUint16(offset + 20, 100, true);
  view.setUint16(offset + 22, 100, true);
  view.setUint16(offset + 24, 12, true);
  view.setUint32(offset + 26, 101, true);

  offset += 30;
  view.setUint16(offset, 12, true);
  view.setUint16(offset + 7 * 2, 345, true);

  offset += VIEWER_COOLDOWN_BYTES;
  view.setUint32(offset, 800, true);
  view.setUint8(offset + 4, 2);
  view.setUint8(offset + 5, 0);
  view.setUint16(offset + 6, 48, true);
  view.setInt32(offset + 8, 50, true);
  view.setInt32(offset + 12, 600, true);
  view.setInt32(offset + 16, -75, true);
  view.setUint16(offset + 20, 0x2222, true);
  view.setInt16(offset + 22, 4_096, true);

  offset += 24;
  view.setUint32(offset, 900, true);
  view.setUint8(offset + 4, CAST_EVENT_TYPE);
  view.setUint8(offset + 5, 2);
  view.setUint8(offset + 6, 5);
  view.setUint8(offset + 7, 7);
  view.setInt32(offset + 8, 444, true);
  view.setInt32(offset + 12, 555, true);
  view.setInt32(offset + 16, -555, true);
  return bytes;
}

test('input encoder writes the exact 22-byte v3 wire contract with spells, flight, and snapshot ack', () => {
  const bytes = encodeInputFrame({
    sequence: 0x7856_3412,
    moveX: -127,
    moveY: 127,
    moveVertical: -64,
    yaw: 0xbeef,
    pitch: 63,
    cast: true,
    spell: 0,
    clientClockMs: 0x1234,
    snapshotAck: 0xfedc_ba98,
  });
  assert.equal(bytes.byteLength, INPUT_FRAME_BYTES);
  assert.deepEqual(Array.from(bytes), [
    3, 1, 22, 0,
    0x12, 0x34, 0x56, 0x78,
    0x81, 0x7f, 0xc0, 0x3f,
    0xef, 0xbe, 1, 0, 0x34, 0x12,
    0x98, 0xba, 0xdc, 0xfe,
  ]);
  const noCast = encodeInputFrame({
    sequence: 1,
    spell: 7,
  });
  assert.equal(noCast[14], 0);
  assert.equal(noCast[15], 0xff, 'an idle input must not imply Firebolt');
  const mend = encodeInputFrame({
    sequence: 2,
    cast: true,
    spell: 7,
  });
  assert.equal(mend[14], 1);
  assert.equal(mend[15], 7);
  assert.throws(
    () => encodeInputFrame({ sequence: 0, moveX: 0, moveY: 0, yaw: 0, clientClockMs: 0 }),
    ProtocolError,
  );
  assert.throws(
    () => encodeInputFrame({ sequence: 1, moveX: 128, moveY: 0, yaw: 0, clientClockMs: 0 }),
    ProtocolError,
  );
  assert.throws(
    () => encodeInputFrame({
      sequence: 1,
      moveX: 0,
      moveY: 0,
      moveVertical: -128,
      yaw: 0,
      pitch: 0,
      clientClockMs: 0,
    }),
    ProtocolError,
  );
  assert.throws(
    () => encodeInputFrame({ sequence: 1, cast: true, spell: 1 }),
    ProtocolError,
  );
  assert.throws(
    () => encodeInputFrame({ sequence: 1, cast: false, spell: 13 }),
    ProtocolError,
  );
});

test('browser and authoritative host pin the same acknowledged input contract', () => {
  assert.match(browserMatchRoomSource, /const INPUT_FRAME_BYTES = 22;/u);
  assert.match(browserMatchRoomSource, /view\.getUint32\(18, true\)/u);
  assert.match(browserMatchRoomSource, /const MAX_UNACKED_SNAPSHOTS = 8;/u);
  assert.match(browserMatchRoomSource, /outstanding >= MAX_UNACKED_SNAPSHOTS/u);
  assert.match(
    demoWebSocketRouteSource,
    /publicWebSocketProtocol\(request, "aetherloom\.v3"\)/u,
  );
});

test('snapshot decoder reads all compact records from an offset view', () => {
  const fixture = snapshotFixture();
  const padded = new Uint8Array(fixture.byteLength + 7);
  padded.set(fixture, 3);
  const snapshot = decodeSnapshotFrame(padded.subarray(3, 3 + fixture.byteLength));
  assert.equal(snapshot.tick, 0x1020_3040);
  assert.equal(snapshot.snapshotSeq, 0xffff_fffe);
  assert.equal(snapshot.viewerSlot, 2);
  assert.equal(snapshot.matchFlags, 3);
  assert.equal(snapshot.acknowledgedInputSequence, 0x1234_5678);
  assert.deepEqual(snapshot.players[0], {
    slot: 2,
    team: 1,
    status: 1,
    flags: 3,
    xcm: -1_234,
    ycm: 222,
    zcm: 5_678,
    yaw: 0xff00,
    pitch: -4_096,
    health: 75,
    maxHealth: 100,
    score: 9,
    entity: 100,
    spellCooldownTicks: [12, 0, 0, 0, 0, 0, 0, 345, 0, 0, 0, 0, 0],
  });
  assert.deepEqual(snapshot.projectiles[0], {
    entity: 800,
    owner: 2,
    ttl: 48,
    xcm: 50,
    ycm: 600,
    zcm: -75,
    yaw: 0x2222,
    pitch: 4_096,
  });
  assert.deepEqual(snapshot.events[0], {
    eventId: 900,
    type: CAST_EVENT_TYPE,
    actor: 2,
    target: 5,
    spell: 7,
    xcm: 444,
    ycm: 555,
    zcm: -555,
  });
});

test('snapshot decoder preserves authoritative firebolt impact coordinates', () => {
  const fixture = snapshotFixture();
  const eventOffset = 24 + 2 * 30 + VIEWER_COOLDOWN_BYTES + 24;
  fixture[eventOffset + 4] = PROJECTILE_IMPACT_EVENT_TYPE;
  fixture[eventOffset + 6] = 0xff;
  fixture[eventOffset + 7] = 0;
  assert.deepEqual(decodeSnapshotFrame(fixture).events[0], {
    eventId: 900,
    type: PROJECTILE_IMPACT_EVENT_TYPE,
    actor: 2,
    target: 0xff,
    spell: 0,
    xcm: 444,
    ycm: 555,
    zcm: -555,
  });
});

test('snapshot decoder rejects truncation, padding, reserved bits, duplicates, and impossible health', () => {
  const fixture = snapshotFixture();
  for (let length = 0; length < fixture.byteLength; length += 1) {
    assert.throws(() => decodeSnapshotFrame(fixture.subarray(0, length)), ProtocolError);
  }

  const padded = new Uint8Array(fixture.byteLength + 1);
  padded.set(fixture);
  assert.throws(() => decodeSnapshotFrame(padded), ProtocolError);
  assert.throws(
    () => decodeSnapshotFrame(new Uint8Array(MAX_SNAPSHOT_FRAME_BYTES + 1)),
    /1,200-byte/u,
  );

  const reserved = fixture.slice();
  reserved[24 + 2 * 30 + VIEWER_COOLDOWN_BYTES + 5] = 1;
  assert.throws(() => decodeSnapshotFrame(reserved), /Projectile reserved/u);

  const unknownMatchFlags = fixture.slice();
  new DataView(unknownMatchFlags.buffer).setUint16(18, 1 << 2, true);
  assert.throws(() => decodeSnapshotFrame(unknownMatchFlags), /match flags/u);

  const unknownPlayerFlags = fixture.slice();
  unknownPlayerFlags[24 + 3] = 1 << 3;
  assert.throws(() => decodeSnapshotFrame(unknownPlayerFlags), /Player flags/u);

  const invalidStatus = fixture.slice();
  invalidStatus[24 + 2] = 0;
  assert.throws(() => decodeSnapshotFrame(invalidStatus), /status is out of range/u);

  const invalidPlayerPitch = fixture.slice();
  new DataView(invalidPlayerPitch.buffer).setInt16(24 + 18, 16_385, true);
  assert.throws(() => decodeSnapshotFrame(invalidPlayerPitch), /Player pitch/u);

  const invalidProjectilePitch = fixture.slice();
  new DataView(invalidProjectilePitch.buffer).setInt16(
    24 + 2 * 30 + VIEWER_COOLDOWN_BYTES + 22,
    -16_385,
    true,
  );
  assert.throws(() => decodeSnapshotFrame(invalidProjectilePitch), /Projectile pitch/u);

  const nonCastEventDetail = fixture.slice();
  nonCastEventDetail[
    24 + 2 * 30 + VIEWER_COOLDOWN_BYTES + 24 + 4
  ] = DAMAGE_EVENT_TYPE;
  assert.throws(() => decodeSnapshotFrame(nonCastEventDetail), /reserved/u);

  const unsupportedCastSpell = fixture.slice();
  unsupportedCastSpell[
    24 + 2 * 30 + VIEWER_COOLDOWN_BYTES + 24 + 7
  ] = 1;
  assert.throws(() => decodeSnapshotFrame(unsupportedCastSpell), ProtocolError);

  const unsupportedImpactSpell = fixture.slice();
  unsupportedImpactSpell[
    24 + 2 * 30 + VIEWER_COOLDOWN_BYTES + 24 + 4
  ] = PROJECTILE_IMPACT_EVENT_TYPE;
  assert.throws(() => decodeSnapshotFrame(unsupportedImpactSpell), /projectile impact/u);

  const duplicateSlot = fixture.slice();
  duplicateSlot[24 + 30] = duplicateSlot[24];
  assert.throws(() => decodeSnapshotFrame(duplicateSlot), /duplicate player slot/u);

  const impossibleHealth = fixture.slice();
  const view = new DataView(impossibleHealth.buffer);
  view.setUint16(24 + 20, 101, true);
  assert.throws(() => decodeSnapshotFrame(impossibleHealth), /health exceeds/u);

  const missingViewer = fixture.slice();
  missingViewer[14] = 99;
  assert.throws(() => decodeSnapshotFrame(missingViewer), /viewer player/u);
});

test('sequence and wrapped clocks handle unsigned rollover', () => {
  assert.equal(isNewerSequence(0, 0xffff_ffff), true);
  assert.equal(isNewerSequence(0xffff_ffff, 0), false);
  assert.equal(isNewerSequence(7, 7), false);
  assert.equal(sequenceAcknowledges(7, 7), true);
  assert.equal(sequenceAcknowledges(0, 0xffff_ffff), false);
  assert.equal(sequenceAcknowledges(1, 0xffff_ffff), true);
  assert.equal(sequenceAcknowledges(0xffff_ffff, 1), false);
  assert.equal(wrappedClockDelta(5, 65_530), 11);
});

test('the 64 Hz deadline pacer skips missed slots instead of bursting after a stall', () => {
  const interval = 1_000 / 64;
  assert.equal(nextPacedDeadline(100, 101, interval), 100 + interval);
  assert.equal(nextPacedDeadline(100, 1_600, interval), 1_600 + interval);
  let deadline = 0;
  let sends = 0;
  for (const now of [16, 32, 48, 1_548, 1_564, 1_580]) {
    if (now >= deadline) {
      sends += 1;
      deadline = nextPacedDeadline(deadline, now, interval);
    }
  }
  assert.equal(sends, 6, 'a long stall must not enqueue catch-up sends');
});

test('close messages preserve safe server diagnostics without exposing arbitrary reasons', () => {
  assert.match(socketCloseMessage(1008, 'sustained input flood'), /sustained input flood/u);
  assert.doesNotMatch(socketCloseMessage(1008, 'private internal detail'), /private internal/u);
  assert.match(socketCloseMessage(4003, 'staging session expired'), /15-minute staging session/u);
  assert.equal(socketCloseMessage(1012, 'staging match host restarted'), null);
  assert.equal(socketCloseMessage(4004, 'input stream timed out'), null);
});

test('wire pitch spans a truthful first-person vertical aim range', () => {
  assert.equal(wirePitchToRadians(0), 0);
  assert.ok(Math.abs(wirePitchToRadians(16_384) - Math.PI / 2) < 1e-9);
  assert.ok(Math.abs(wirePitchToRadians(-16_384) + Math.PI / 2) < 1e-9);
  assert.throws(() => wirePitchToRadians(16_385), ProtocolError);
});

test('first-person camera uses the immediate yaw/pitch reticle ray', () => {
  const level = firstPersonCamera({ x: 10, y: 20, z: 30, yaw: 0, pitch: 0 });
  assert.equal(level.ex, 10);
  assert.equal(level.ey, 23);
  assert.equal(level.ez, 30);
  assert.equal(level.cx, 90);
  assert.equal(level.cy, 23);
  assert.equal(level.cz, 30);

  const quarterTurn = firstPersonCamera({
    x: 10,
    y: 20,
    z: 30,
    yaw: 16_384,
    pitch: 0,
  });
  assert.ok(quarterTurn.cz > quarterTurn.ez);
  assert.ok(Math.abs(quarterTurn.cx - quarterTurn.ex) < 1e-9);

  const upward = firstPersonCamera({ x: 10, y: 20, z: 30, yaw: 0, pitch: 8_192 });
  assert.ok(upward.cy > upward.ey);
  assert.ok(upward.cx > upward.ex);

  const vertical = firstPersonCamera({
    x: 10,
    y: 20,
    z: 30,
    yaw: 16_384,
    pitch: 16_384,
  });
  const forward = [
    vertical.cx - vertical.ex,
    vertical.cy - vertical.ey,
    vertical.cz - vertical.ez,
  ];
  const up = [vertical.ux, vertical.uy, vertical.uz];
  assert.ok(forward.every(Number.isFinite));
  assert.ok(up.every(Number.isFinite));
  assert.ok(Math.abs(forward[0] * up[0] + forward[1] * up[1] + forward[2] * up[2]) < 1e-9);
});

test('first-person muzzle and local projectile easing stay aligned with the reticle ray', () => {
  const viewer = { x: 10, y: 20, z: 30, yaw: 0, pitch: 0 };
  assert.deepEqual(firstPersonMuzzle(viewer), {
    x: 16,
    y: 21.5,
    z: 31.25,
    yaw: 0,
    pitch: 0,
  });

  const quarterTurn = firstPersonMuzzle({ ...viewer, yaw: 16_384 });
  assert.ok(Math.abs(quarterTurn.x - 8.75) < 1e-9);
  assert.equal(quarterTurn.y, 21.5);
  assert.ok(Math.abs(quarterTurn.z - 36) < 1e-9);

  for (const yaw of [0, 16_384, 41_000]) {
    for (const pitch of [-10_950, 0, 10_950]) {
      const pose = { ...viewer, yaw, pitch };
      const camera = firstPersonCamera(pose);
      const muzzle = firstPersonMuzzle(pose);
      const fromEye = [
        muzzle.x - camera.ex,
        muzzle.y - camera.ey,
        muzzle.z - camera.ez,
      ];
      const forward = [
        (camera.cx - camera.ex) / 80,
        (camera.cy - camera.ey) / 80,
        (camera.cz - camera.ez) / 80,
      ];
      const up = [camera.ux, camera.uy, camera.uz];
      const right = [
        forward[1] * up[2] - forward[2] * up[1],
        forward[2] * up[0] - forward[0] * up[2],
        forward[0] * up[1] - forward[1] * up[0],
      ];
      const depth = fromEye.reduce(
        (sum, component, index) => sum + component * forward[index],
        0,
      );
      const vertical = fromEye.reduce(
        (sum, component, index) => sum + component * up[index],
        0,
      );
      const horizontal = fromEye.reduce(
        (sum, component, index) => sum + component * right[index],
        0,
      );
      assert.ok(depth > 0);
      assert.ok(Math.abs(vertical / depth) < Math.tan(1.16 / 2));
      assert.ok(
        Math.abs(horizontal / depth) <
          Math.tan(1.16 / 2) * 9 / 16,
        'the hand muzzle must remain visible even at a portrait aspect ratio',
      );
    }
  }

  const position = { x: 100, y: 50, z: 200, yaw: 0, pitch: 0 };
  const remote = viewerProjectilePresentation(
    position,
    { owner: 3, ttl: 410, yaw: 0, pitch: 0 },
    2,
  );
  assert.deepEqual(remote, position);
  assert.notEqual(remote, position);

  const fresh = viewerProjectilePresentation(
    position,
    { owner: 2, ttl: 410, yaw: 0, pitch: 0 },
    2,
  );
  const halfway = viewerProjectilePresentation(
    position,
    { owner: 2, ttl: 404, yaw: 0, pitch: 0 },
    2,
  );
  const halfTick = viewerProjectilePresentation(
    position,
    { owner: 2, ttl: 410, yaw: 0, pitch: 0 },
    2,
    0.5,
  );
  const oneTick = viewerProjectilePresentation(
    position,
    { owner: 2, ttl: 410, yaw: 0, pitch: 0 },
    2,
    1,
  );
  const nearlySettled = viewerProjectilePresentation(
    position,
    { owner: 2, ttl: 410, yaw: 0, pitch: 0 },
    2,
    11.5,
  );
  const settled = viewerProjectilePresentation(
    position,
    { owner: 2, ttl: 398, yaw: 0, pitch: 0 },
    2,
  );
  assert.ok(Math.abs(fresh.x - 104.6) < 1e-9);
  assert.ok(Math.abs(fresh.y - 49.2) < 1e-9);
  assert.ok(Math.abs(fresh.z - 201.25) < 1e-9);
  assert.ok(fresh.x > halfTick.x && halfTick.x > oneTick.x);
  assert.ok(Math.abs((fresh.x - halfTick.x) - (halfTick.x - oneTick.x)) < 1e-12);
  assert.ok(Math.abs(halfway.x - 102.3) < 1e-9);
  assert.ok(Math.abs(halfway.y - 49.6) < 1e-9);
  assert.ok(Math.abs(halfway.z - 200.625) < 1e-9);
  assert.ok(Math.abs(nearlySettled.x - 100.19166666666666) < 1e-9);
  assert.deepEqual(settled, position);
});

test('acknowledging an input preserves the same predicted pose without a snapshot sawtooth', () => {
  const sentAt = 100;
  const history = [
    { sequence: 1, sentAt, x: 127, y: 127, z: 0 },
    { sequence: 2, sentAt: sentAt + 1_000 / 64, x: 127, y: 127, z: 0 },
  ];
  const sampleAt = sentAt + 2_000 / 64;
  const beforeAck = pendingInputDisplacement(history, sampleAt);
  const afterAck = pendingInputDisplacement(history.slice(1), sampleAt);
  assert.equal(beforeAck.x, 4);
  assert.ok(Math.abs(beforeAck.y - 2.4) < 1e-12);
  assert.equal(afterAck.x, 2);
  assert.ok(Math.abs(afterAck.y - 1.2) < 1e-12);
  assert.equal(2 + afterAck.x, beforeAck.x);
  assert.ok(Math.abs(1.2 + afterAck.y - beforeAck.y) < 1e-12);
});

test('prediction mirrors host expansion and Rust truncation for partial axes', () => {
  for (const [axis, planar, vertical] of [
    [1, 0, 0],
    [63, 0.8, 0.4],
    [90, 1.4, 0.8],
    [126, 1.8, 1],
    [127, 2, 1.2],
  ]) {
    assert.ok(Math.abs(predictedBrowserAxisDisplacement(axis, 10) - planar) < 1e-12);
    assert.ok(Math.abs(predictedBrowserAxisDisplacement(axis, 6) - vertical) < 1e-12);
    assert.ok(Math.abs(predictedBrowserAxisDisplacement(-axis, 10) + planar) < 1e-12);
    assert.ok(Math.abs(predictedBrowserAxisDisplacement(-axis, 6) + vertical) < 1e-12);
  }
});

test('first-person prediction follows bilinear terrain between authoritative snapshots', () => {
  const heights = new Float32Array([
    0, 10, 20,
    20, 30, 40,
    40, 50, 60,
  ]);
  assert.equal(bilinearHeight(heights, 3, 10, 20, 5, 5), 15);
  assert.equal(bilinearHeight(heights, 3, 10, 20, 15, 15), 45);
  assert.equal(bilinearHeight(heights, 3, 10, 20, -1, 5), 0);
  const heightAt = (x, z) => bilinearHeight(heights, 3, 10, 20, x, z);
  const predicted = predictViewerPosition(
    { x: 5, y: 49, z: 5, yaw: 0, pitch: 0, altitudeCm: 0 },
    { x: 2, y: 1, z: 3 },
    heightAt,
  );
  // The original altitude is preserved while the ground rises continuously
  // from 15 to 23 world units under the predicted horizontal displacement.
  assert.deepEqual(predicted, {
    x: 7,
    y: 58,
    z: 8,
    yaw: 0,
    pitch: 0,
    altitudeCm: 10,
  });

  const floor = predictViewerPosition(
    { x: 5, y: 49, z: 5, yaw: 0, pitch: 0, altitudeCm: 0 },
    { x: 0, y: -1, z: 0 },
    heightAt,
  );
  assert.equal(floor.y, 49);
  assert.equal(floor.altitudeCm, 0);

  const ceiling = predictViewerPosition(
    { x: 5, y: 479, z: 5, yaw: 0, pitch: 0, altitudeCm: 4_300 },
    { x: 0, y: 1, z: 0 },
    heightAt,
  );
  assert.equal(ceiling.y, 479);
  assert.equal(ceiling.altitudeCm, 4_300);
});

test('camera-relative movement is quantized into server world axes', () => {
  // The core wire convention is 0 = +X and one quarter-turn = +Z.
  assert.deepEqual(movementToWorld(0, 127, 0), { x: 127, y: 0, vertical: 0 });
  assert.deepEqual(movementToWorld(0, 127, 16_384), { x: 0, y: 127, vertical: 0 });
  assert.deepEqual(movementToWorld(127, 0, 0), { x: 0, y: 127, vertical: 0 });
  assert.deepEqual(movementToWorld(127, 0, 16_384), { x: -127, y: 0, vertical: 0 });
  assert.deepEqual(
    movementToWorld(0, 127, 0, 8_192),
    { x: 90, y: 0, vertical: 90 },
  );
  assert.deepEqual(
    movementToWorld(0, 0, 0, 0, 127),
    { x: 0, y: 0, vertical: 127 },
  );
  const diagonal = movementToWorld(90, 90, 8_192);
  assert.ok(Math.hypot(diagonal.x, diagonal.y) <= 128);
  assert.equal(diagonal.vertical, 0);
});

test('flight input preserves forward authority while reducing strafe dominance', () => {
  assert.deepEqual(shapeFlightInput(0, 0), { strafe: 0, thrust: 0 });
  assert.deepEqual(shapeFlightInput(1, 1), { strafe: 0.55, thrust: 1 });
  assert.deepEqual(shapeFlightInput(-1, -1), { strafe: -0.55, thrust: -1 });
  assert.deepEqual(shapeFlightInput(0.5, -0.25), { strafe: 0.275, thrust: -0.25 });
  assert.throws(() => shapeFlightInput(1.01, 0), ProtocolError);
  assert.throws(() => shapeFlightInput(0, Number.NaN), ProtocolError);
});

test('positive horizontal look turns toward first-person screen right', () => {
  assert.equal(yawAfterLookDelta(0, 70), 70);
  assert.equal(yawAfterLookDelta(0xffff, 2), 1);
  assert.equal(yawAfterLookDelta(0, -1), 0xffff);
});

test('gamepad look is render-rate independent, deadzoned, and hitch bounded', () => {
  const axis = 0.5;
  const unitsPerSecond = 49_920;
  const at64Hz = Array.from(
    { length: 64 },
    () => gamepadLookDelta(axis, 1_000 / 64, unitsPerSecond),
  ).reduce((sum, value) => sum + value, 0);
  const at128Hz = Array.from(
    { length: 128 },
    () => gamepadLookDelta(axis, 1_000 / 128, unitsPerSecond),
  ).reduce((sum, value) => sum + value, 0);
  assert.ok(Math.abs(at64Hz - at128Hz) < 1e-9);
  assert.ok(Math.abs(at64Hz - axis * unitsPerSecond) < 1e-9);
  assert.equal(gamepadLookDelta(0.159, 16, unitsPerSecond), 0);
  assert.equal(
    gamepadLookDelta(2, 16, unitsPerSecond),
    gamepadLookDelta(1, 16, unitsPerSecond),
  );
  assert.equal(
    gamepadLookDelta(1, 250, unitsPerSecond),
    gamepadLookDelta(1, 50, unitsPerSecond),
  );
  assert.throws(() => gamepadLookDelta(0, -1, unitsPerSecond), ProtocolError);
});

test('authoritative cooldown timing removes measured snapshot transit delay', () => {
  assert.equal(authoritativeCooldownReadyAt(1_000, 0, 80), 1_000);
  assert.equal(
    authoritativeCooldownReadyAt(1_000, 128, 80),
    1_960,
  );
  assert.equal(
    authoritativeCooldownReadyAt(1_000, 1, 1_000),
    1_000,
    'age compensation must never move readiness into the past',
  );
  assert.throws(
    () => authoritativeCooldownReadyAt(1_000, -1, 0),
    ProtocolError,
  );
});

test('wire yaw points camera, carpet, and cast in the same direction', () => {
  const origin = [1_280, 512, 1_280];
  const localNose = new Float32Array(14);
  localNose[0] = origin[0];
  localNose[1] = origin[1];
  localNose[2] = origin[2] + 1;
  localNose[3] = localNose[4] = localNose[5] = 1;
  for (const [yaw, expectedX, expectedZ] of [
    [0, 1, 0],
    [16_384, 0, 1],
    [32_768, -1, 0],
    [49_152, 0, -1],
  ]) {
    const core = wireYawToCoreRadians(yaw);
    assert.ok(Math.abs(Math.cos(core) - expectedX) < 1e-6);
    assert.ok(Math.abs(Math.sin(core) - expectedZ) < 1e-6);
    const model = composeTemplate(
      localNose,
      origin,
      [100, 50, 200],
      wireYawToRenderRadians(yaw),
    );
    assert.ok(Math.abs(model[0] - (100 + expectedX)) < 1e-6);
    assert.ok(Math.abs(model[2] - (200 + expectedZ)) < 1e-6);
  }
});

test('model composition rotates preview-local geometry around the world origin', () => {
  const template = new Float32Array(14);
  template.set([
    1_280, 512, 1_281,
    2, 3, 4,
    0.2, 0.4, 0.6,
    0, 0.1, 0.2,
    0.8, 2,
  ]);
  const result = composeTemplate(template, [1_280, 512, 1_280], [100, 50, 200], Math.PI / 2);
  assert.ok(Math.abs(result[0] - 101) < 1e-6);
  assert.equal(result[1], 50);
  assert.ok(Math.abs(result[2] - 200) < 1e-6);
  assert.ok(Math.abs(result[9] - Math.PI / 2) < 1e-6);
  assert.deepEqual(Array.from(result.slice(3, 9)), [2, 3, 4, 0.2, 0.4, 0.6].map(Math.fround));
});

test('the Wasm visual templates are finite and copied before the next preview overwrites them', async () => {
  const wasm = await readFile(new URL('../site/sim.wasm', import.meta.url));
  const { instance } = await WebAssembly.instantiate(wasm, {});
  const sim = instance.exports;
  assert.equal(sim.instStride(), 14);
  const copy = (scene, variant = 0) => {
    const count = sim.previewScene(scene, variant);
    assert.ok(count > 0);
    return new Float32Array(sim.memory.buffer, sim.instPtr(), count * 14).slice();
  };
  const player = copy(20);
  const retained = player.slice();
  const rival = copy(21);
  const firebolts = Array.from({ length: 24 }, (_, variant) => copy(25, variant));
  assert.deepEqual(player, retained);
  assert.notDeepEqual(player, rival);
  assert.notDeepEqual(firebolts[0], firebolts[1]);
  assert.notDeepEqual(firebolts[23], firebolts[0]);
  const visualDistance = (left, right) => {
    assert.equal(left.length, right.length);
    let squared = 0;
    for (let index = 0; index < left.length; index += 14) {
      for (const field of [0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 13]) {
        squared += (left[index + field] - right[index + field]) ** 2;
      }
      for (const field of [9, 10, 11]) {
        squared += (
          Math.sin(left[index + field]) - Math.sin(right[index + field])
        ) ** 2;
        squared += (
          Math.cos(left[index + field]) - Math.cos(right[index + field])
        ) ** 2;
      }
    }
    return Math.sqrt(squared);
  };
  const animationSteps = firebolts.map((frame, variant) =>
    visualDistance(frame, firebolts[(variant + 1) % firebolts.length]));
  const ordinaryMaximum = Math.max(...animationSteps.slice(0, -1));
  assert.ok(
    animationSteps.at(-1) <= ordinaryMaximum * 1.25,
    `firebolt loop seam ${animationSteps.at(-1)} exceeds ordinary step ${ordinaryMaximum}`,
  );
  for (const template of [player, rival, ...firebolts]) {
    assert.equal(template.length % 14, 0);
    assert.ok(template.every(Number.isFinite));
  }
});

test('room and control-plane validation keeps share URLs credential-free', () => {
  assert.equal(validateRoom(' Duel-01 '), 'duel-01');
  assert.equal(validateRoom('duel-32'), 'duel-32');
  assert.throws(() => validateRoom('duel-00'), /duel-01 through duel-32/u);
  assert.throws(() => validateRoom('duel-33'), /duel-01 through duel-32/u);
  assert.throws(() => validateRoom('private-room'), /duel-01 through duel-32/u);
  assert.equal(
    normalizeControlPlane('https://example.workers.dev/path/?secret=nope#fragment'),
    'https://example.workers.dev/path',
  );
  assert.throws(() => normalizeControlPlane('http://example.com'), /HTTPS/u);
  assert.throws(() => normalizeControlPlane('https://user:pass@example.com'), /credentials/u);
  assert.equal(
    resumeSessionKey('https://example.workers.dev/path', ' Duel-24 '),
    'aetherloom.demo.resume.https%3A%2F%2Fexample.workers.dev%2Fpath.duel-24',
  );
  assert.equal(
    joinAttemptKey('https://example.workers.dev/path/?secret=nope#fragment', ' Duel-24 '),
    'aetherloom.demo.join.https%3A%2F%2Fexample.workers.dev%2Fpath.duel-24',
  );
  assert.equal(isResumeToken('demo_resume_0123456789abcdefghijklmn'), true);
  assert.equal(isResumeToken('demo_resume_corrupt'), false);
  assert.equal(isJoinNonce('demo_join_0123456789abcdefghijklmn'), true);
  assert.equal(isJoinNonce('demo_join_corrupt'), false);
});

test('fresh join nonces and resume credentials match a deterministic browser-server vector', async () => {
  const cryptoProvider = {
    getRandomValues(bytes) {
      for (let index = 0; index < bytes.length; index += 1) bytes[index] = index;
      return bytes;
    },
    subtle: webcrypto.subtle,
  };
  const nonce = randomJoinNonce(cryptoProvider);
  assert.equal(nonce, 'demo_join_AAECAwQFBgcICQoLDA0ODxAR');
  assert.equal(isJoinNonce(nonce), true);
  assert.equal(
    await demoResumeToken(' Duel-24 ', nonce, cryptoProvider),
    'demo_resume_KOnFEg8fnhwwOM84LrvYeUZi',
  );
  assert.notEqual(
    await demoResumeToken('duel-25', nonce, cryptoProvider),
    'demo_resume_KOnFEg8fnhwwOM84LrvYeUZi',
  );
  assert.throws(() => randomJoinNonce({}), /Secure randomness/u);
  await assert.rejects(
    demoResumeToken('duel-24', 'demo_join_corrupt', cryptoProvider),
    /join attempt is invalid/u,
  );
});

test('resume tokens stay per-tab, remain scoped, reject corruption, and tolerate unavailable storage', () => {
  const firstSession = mapStorage();
  const secondSession = mapStorage();
  const first = new ResumeTokenStore(firstSession);
  const second = new ResumeTokenStore(secondSession);
  const key = resumeSessionKey('https://example.workers.dev', 'duel-24');
  const otherRoom = resumeSessionKey('https://example.workers.dev', 'duel-25');
  const token = 'demo_resume_0123456789abcdefghijklmn';
  first.save(key, token);
  assert.equal(first.load(key), token);
  assert.equal(second.load(key), undefined, 'another tab must be able to open player two');
  assert.equal(second.load(otherRoom), undefined);
  first.clear(key);
  assert.equal(first.load(key), undefined);

  const corruptSession = mapStorage(new Map([[key, 'demo_resume_corrupt']]));
  assert.equal(new ResumeTokenStore(corruptSession).load(key), undefined);
  assert.equal(corruptSession.getItem(key), null);

  const unavailable = {
    getItem() { throw new Error('unavailable'); },
    setItem() { throw new Error('unavailable'); },
    removeItem() { throw new Error('unavailable'); },
  };
  const memoryOnly = new ResumeTokenStore(unavailable);
  memoryOnly.save(key, token);
  assert.equal(memoryOnly.load(key), token);
  memoryOnly.clear(key);
  assert.equal(memoryOnly.load(key), undefined);
});

test('join nonce retries are idempotent in one tab and reject corrupt cloned state', () => {
  const entries = new Map();
  const storage = mapStorage(entries);
  const store = new JoinNonceStore(storage);
  const key = joinAttemptKey('https://example.workers.dev', 'duel-24');
  const nonce = 'demo_join_0123456789abcdefghijklmn';
  store.save(key, nonce);
  assert.equal(store.load(key), nonce);
  assert.equal(entries.get(key), nonce);

  const unpersistedEntries = new Map();
  const unpersisted = new JoinNonceStore(mapStorage(unpersistedEntries));
  unpersisted.save(key, 'demo_join_abcdefghijklmnopqrstuvwx', { persist: false });
  assert.equal(unpersistedEntries.has(key), false);

  const unavailable = {
    getItem() { throw new Error('unavailable'); },
    setItem() { throw new Error('unavailable'); },
    removeItem() { throw new Error('unavailable'); },
  };
  const memoryOnly = new JoinNonceStore(unavailable);
  memoryOnly.save(key, nonce);
  assert.equal(memoryOnly.load(key), nonce);

  const corruptEntries = new Map([[key, 'demo_join_corrupt']]);
  assert.equal(new JoinNonceStore(mapStorage(corruptEntries)).load(key), undefined);
  assert.equal(corruptEntries.has(key), false);
  assert.throws(() => store.save(key, 'demo_join_corrupt'), /nonce is invalid/u);
  store.clear(key);
  assert.equal(store.load(key), undefined);
});

test('Web Locks prevent a cloned session from resuming the same staging player', async () => {
  const room = 'duel-24';
  const scope = resumeSessionKey('https://example.workers.dev', room);
  const joinKey = joinAttemptKey('https://example.workers.dev', room);
  const token = 'demo_resume_0123456789abcdefghijklmn';
  const nonce = 'demo_join_0123456789abcdefghijklmn';
  const originalEntries = new Map([[scope, token], [joinKey, nonce]]);
  const clonedEntries = new Map(originalEntries);
  assert.equal(new ResumeTokenStore(mapStorage(originalEntries)).load(scope), token);
  assert.equal(new ResumeTokenStore(mapStorage(clonedEntries)).load(scope), token);
  assert.equal(new JoinNonceStore(mapStorage(clonedEntries)).load(joinKey), nonce);

  const lockManager = new DeterministicLockManager();
  const original = new ResumeOwnership(lockManager);
  const clone = new ResumeOwnership(lockManager);
  assert.equal(original.supported, true);
  assert.equal(await original.claim(scope, token), true);
  assert.equal(await original.claim(scope, token), true, 'the owning tab may reconnect');
  assert.equal(await clone.claim(scope, token), false, 'a cloned tab must create another player');
  assert.deepEqual(lockManager.requests[0].options, {
    ifAvailable: true,
    mode: 'exclusive',
  });
  assert.match(lockManager.requests[0].name, /^aetherloom:resume:/u);

  const independentToken = 'demo_resume_abcdefghijklmnopqrstuvwx';
  assert.equal(await clone.claim(scope, independentToken), true);
  assert.equal(await new ResumeOwnership(null).claim(scope, token), false);
  assert.equal(
    await new ResumeOwnership({ request() { throw new Error('unavailable'); } }).claim(scope, token),
    false,
  );
});

test('concurrent joins share one admission request and permit a later retry', async () => {
  const flight = new SingleFlight();
  let calls = 0;
  let release;
  const gate = new Promise((resolve) => { release = resolve; });
  const first = flight.run(async () => {
    calls += 1;
    await gate;
    return 'joined';
  });
  const second = flight.run(() => {
    calls += 1;
    return 'duplicate';
  });
  assert.equal(first, second);
  assert.equal(calls, 0);
  await Promise.resolve();
  assert.equal(calls, 1);
  release();
  assert.deepEqual(await Promise.all([first, second]), ['joined', 'joined']);
  assert.equal(await flight.run(() => {
    calls += 1;
    return 'retried';
  }), 'retried');
  assert.equal(calls, 2);
  flight.reset();
});

test('local-file multiplayer links hand off to staging without forwarding credentials', async () => {
  const html = await readFile(new URL('../site/multiplayer.html', import.meta.url), 'utf8');
  const source = html.match(
    /<script id="file-staging-handoff">([\s\S]*?)<\/script>/u,
  )?.[1];
  assert.ok(source, 'the early local-file handoff script must exist');
  const run = (href) => {
    const replacements = [];
    const parsed = new URL(href);
    runInNewContext(source, {
      URL,
      window: {
        location: {
          href,
          protocol: parsed.protocol,
          replace: (target) => replacements.push(target),
        },
      },
    });
    return replacements;
  };
  assert.deepEqual(
    run('file:///tmp/multiplayer.html?room=duel-24&controlPlane=https://evil.example/#secret'),
    ['https://staging.aetherloom-staging.pages.dev/multiplayer.html?room=duel-24'],
  );
  assert.deepEqual(
    run('file:///tmp/multiplayer.html?room=private-room&resumeToken=secret#secret'),
    ['https://staging.aetherloom-staging.pages.dev/multiplayer.html'],
  );
  assert.deepEqual(
    run('https://staging.aetherloom-staging.pages.dev/multiplayer.html?room=duel-24'),
    [],
  );
});

test('valid room resumes keep the same short-lived credential', () => {
  const resumeBranch = browserMatchRoomSource.slice(
    browserMatchRoomSource.indexOf('if (resumeToken !== undefined && resumeHash !== undefined)'),
    browserMatchRoomSource.indexOf('const players = this.playerRows()'),
  );
  assert.match(resumeBranch, /UPDATE players SET last_seen_at_ms = \? WHERE slot = \?/u);
  assert.match(resumeBranch, /presentClaim\(room, resumed, resumeToken\)/u);
  assert.doesNotMatch(resumeBranch, /SET resume_hash/u);
});

test('the standalone page advertises staging limits and leaves the offline entrypoint intact', async () => {
  const [html, source, index] = await Promise.all([
    readFile(new URL('../site/multiplayer.html', import.meta.url), 'utf8'),
    readFile(new URL('../site/multiplayer.js', import.meta.url), 'utf8'),
    readFile(new URL('../site/index.html', import.meta.url), 'utf8'),
  ]);
  assert.match(html, /<h1>Multiplayer Systems Test<\/h1>/u);
  assert.match(html, /casual staging, not competitive/u);
  assert.match(html, /duel-01 through duel-32/u);
  assert.match(html, /src="\.\/multiplayer\.js"/u);
  assert.match(html, /id="file-staging-handoff"/u);
  assert.match(source, /safeBrowserStorage\('sessionStorage'\)/u);
  assert.match(source, /safeBrowserLocks\(\)/u);
  assert.doesNotMatch(source, /safeBrowserStorage\('localStorage'\)/u);
  assert.match(source, /new JoinNonceStore\(sessionStorage\)/u);
  assert.match(source, /new ResumeOwnership\(safeBrowserLocks\(\)\)/u);
  assert.match(source, /await this\.resumeOwnership\.claim/u);
  assert.match(source, /joined\.resumeToken !== attempt\.resumeToken/u);
  assert.match(source, /this\.joinFlight\.run/u);
  assert.match(source, /\[\s*'aetherloom\.v3',/u);
  assert.match(source, /`aetherloom\.auth\.\$\{joined\.ticket\}`/u);
  assert.match(source, /previewScene\(scene, variant\)[\s\S]*?\.slice\(\)/u);
  assert.match(source, /hurt: 0/u);
  assert.match(source, /WORLD_UNITS_PER_CM = 0\.1/u);
  assert.match(source, /snapshot\.viewerSlot !== this\.joinSlot/u);
  assert.match(source, /socket\.readyState === WebSocket\.CONNECTING/u);
  assert.match(source, /this\.sentClocks\.get\(snapshot\.echoClock\)/u);
  assert.match(source, /if \(player\.slot === this\.latestSnapshot\.viewerSlot\) continue;/u);
  assert.match(source, /const FIRST_PERSON_EYE_HEIGHT = 3;/u);
  assert.match(source, /const FIRST_PERSON_FOV = 1\.16;/u);
  assert.match(source, /const eyeY = viewer\.y \+ FIRST_PERSON_EYE_HEIGHT;/u);
  assert.match(source, /fov: FIRST_PERSON_FOV/u);
  assert.doesNotMatch(source, /FIREBALL_TEMPLATE_PITCH/u);
  assert.match(
    source,
    /fireboltTemplate[\s\S]*?wirePitchToRadians\(position\.pitch\),/u,
  );
  assert.match(source, /firstSeenElapsedTicks[\s\S]*?Math\.max\(0, time -/u);
  assert.match(source, /\* 128 \/ 1_000/u);
  assert.match(source, /viewerSlot,\s*elapsedTicks,/u);
  assert.match(source, /shapeFlightInput\(horizontal, vertical\)/u);
  assert.match(source, /gamepadLookDelta\([\s\S]*?GAMEPAD_YAW_UNITS_PER_SECOND/u);
  assert.doesNotMatch(source, /pad\.axes\[2\][\s\S]{0,80}\* 780/u);
  assert.match(source, /event\.movementY/u);
  assert.match(source, /this\.inputHistory/u);
  assert.match(source, /nextPacedDeadline/u);
  assert.match(html, /Drag sky to aim/u);
  assert.match(html, /<kbd>Space<\/kbd> climb/u);
  assert.match(html, /data-key="ascend"/u);
  assert.doesNotMatch(
    html,
    /body\[data-connection="connected"\] \.stats \{ display: none; \}/u,
  );
  assert.match(index, /from '\.\/game\.js'/u);
  assert.match(index, /href="\.\/multiplayer\.html"/u);
  assert.match(browserMatchRoomSource, /const CORE_PROJECTILE_IMPACT_EVENT = 10;/u);
  assert.match(
    browserMatchRoomSource,
    /detailIndex = event\.kind === CORE_PROJECTILE_IMPACT_EVENT \? 3 : 0/u,
  );
  assert.match(browserMatchRoomSource, /await demoResumeToken\(roomCode, joinNonce\)/u);
  assert.match(
    browserMatchRoomSource,
    /presentClaim\(room, player, nextResumeToken, true\)/u,
  );
  assert.match(browserMatchRoomSource, /CREATE TABLE IF NOT EXISTS consumed_join_nonces/u);
  assert.match(gatewaySource, /\.\.\.\(joinNonce === undefined \? \{\} : \{ joinNonce \}\)/u);
  assert.match(
    gatewaySource,
    /admission !== undefined &&[\s\S]*?resumeToken === undefined &&[\s\S]*?!claim\.createdPlayer[\s\S]*?refundDemoAdmission/u,
  );
});
