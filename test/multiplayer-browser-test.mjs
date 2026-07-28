import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  DAMAGE_EVENT_TYPE,
  INPUT_FRAME_BYTES,
  MAX_SNAPSHOT_FRAME_BYTES,
  ProtocolError,
  bilinearHeight,
  composeTemplate,
  decodeSnapshotFrame,
  encodeInputFrame,
  firstPersonCamera,
  isNewerSequence,
  movementToWorld,
  nextPacedDeadline,
  normalizeControlPlane,
  pendingInputDisplacement,
  predictedBrowserAxisDisplacement,
  predictViewerPosition,
  socketCloseMessage,
  validateRoom,
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

function snapshotFixture() {
  const byteLength = 24 + 2 * 30 + 24 + 20;
  const bytes = new Uint8Array(byteLength);
  const view = new DataView(bytes.buffer);
  view.setUint8(0, 2);
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
  view.setUint8(offset + 4, DAMAGE_EVENT_TYPE);
  view.setUint8(offset + 5, 2);
  view.setUint8(offset + 6, 5);
  view.setUint8(offset + 7, 0);
  view.setInt32(offset + 8, 444, true);
  view.setInt32(offset + 12, 555, true);
  view.setInt32(offset + 16, -555, true);
  return bytes;
}

test('input encoder writes the exact 22-byte v2 wire contract with 3D flight and snapshot ack', () => {
  const bytes = encodeInputFrame({
    sequence: 0x7856_3412,
    moveX: -127,
    moveY: 127,
    moveVertical: -64,
    yaw: 0xbeef,
    pitch: 63,
    cast: true,
    clientClockMs: 0x1234,
    snapshotAck: 0xfedc_ba98,
  });
  assert.equal(bytes.byteLength, INPUT_FRAME_BYTES);
  assert.deepEqual(Array.from(bytes), [
    2, 1, 22, 0,
    0x12, 0x34, 0x56, 0x78,
    0x81, 0x7f, 0xc0, 0x3f,
    0xef, 0xbe, 1, 0, 0x34, 0x12,
    0x98, 0xba, 0xdc, 0xfe,
  ]);
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
});

test('browser and authoritative host pin the same acknowledged input contract', () => {
  assert.match(browserMatchRoomSource, /const INPUT_FRAME_BYTES = 22;/u);
  assert.match(browserMatchRoomSource, /view\.getUint32\(18, true\)/u);
  assert.match(browserMatchRoomSource, /const MAX_UNACKED_SNAPSHOTS = 8;/u);
  assert.match(browserMatchRoomSource, /outstanding >= MAX_UNACKED_SNAPSHOTS/u);
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
    type: DAMAGE_EVENT_TYPE,
    actor: 2,
    target: 5,
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
  reserved[24 + 2 * 30 + 5] = 1;
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
    24 + 2 * 30 + 22,
    -16_385,
    true,
  );
  assert.throws(() => decodeSnapshotFrame(invalidProjectilePitch), /Projectile pitch/u);

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
  assert.ok(level.cx > level.ex);
  assert.ok(Math.abs(level.cy - level.ey) < 1e-9);
  assert.ok(Math.abs(level.cz - level.ez) < 1e-9);

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

test('acknowledging an input preserves the same predicted pose without a snapshot sawtooth', () => {
  const sentAt = 100;
  const history = [
    { sequence: 1, sentAt, x: 127, y: 127, z: 0 },
    { sequence: 2, sentAt: sentAt + 1_000 / 64, x: 127, y: 127, z: 0 },
  ];
  const sampleAt = sentAt + 2_000 / 64;
  const beforeAck = pendingInputDisplacement(history, sampleAt);
  const afterAck = pendingInputDisplacement(history.slice(1), sampleAt);
  assert.equal(beforeAck.x, 3.2);
  assert.equal(beforeAck.y, 2);
  assert.equal(afterAck.x, 1.6);
  assert.equal(afterAck.y, 1);
  assert.equal(1.6 + afterAck.x, beforeAck.x);
  assert.equal(1 + afterAck.y, beforeAck.y);
});

test('prediction mirrors host expansion and Rust truncation for partial axes', () => {
  for (const [axis, planar, vertical] of [
    [1, 0, 0],
    [63, 0.6, 0.4],
    [90, 1, 0.6],
    [126, 1.4, 0.8],
    [127, 1.6, 1],
  ]) {
    assert.ok(Math.abs(predictedBrowserAxisDisplacement(axis, 8) - planar) < 1e-12);
    assert.ok(Math.abs(predictedBrowserAxisDisplacement(axis, 5) - vertical) < 1e-12);
    assert.ok(Math.abs(predictedBrowserAxisDisplacement(-axis, 8) + planar) < 1e-12);
    assert.ok(Math.abs(predictedBrowserAxisDisplacement(-axis, 5) + vertical) < 1e-12);
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
  assert.deepEqual(movementToWorld(0, 127, 0), { x: 127, y: 0 });
  assert.deepEqual(movementToWorld(0, 127, 16_384), { x: 0, y: 127 });
  assert.deepEqual(movementToWorld(127, 0, 0), { x: 0, y: 127 });
  assert.deepEqual(movementToWorld(127, 0, 16_384), { x: -127, y: 0 });
  const diagonal = movementToWorld(90, 90, 8_192);
  assert.ok(Math.hypot(diagonal.x, diagonal.y) <= 128);
});

test('positive horizontal look turns toward first-person screen right', () => {
  assert.equal(yawAfterLookDelta(0, 70), 70);
  assert.equal(yawAfterLookDelta(0xffff, 2), 1);
  assert.equal(yawAfterLookDelta(0, -1), 0xffff);
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
  const copy = (scene) => {
    const count = sim.previewScene(scene, 0);
    assert.ok(count > 0);
    return new Float32Array(sim.memory.buffer, sim.instPtr(), count * 14).slice();
  };
  const player = copy(20);
  const retained = player.slice();
  const rival = copy(21);
  const firebolt = copy(25);
  assert.deepEqual(player, retained);
  assert.notDeepEqual(player, rival);
  for (const template of [player, rival, firebolt]) {
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
  assert.match(source, /\[\s*'aetherloom\.v2',/u);
  assert.match(source, /`aetherloom\.auth\.\$\{joined\.ticket\}`/u);
  assert.match(source, /previewScene\(scene, 0\)[\s\S]*?\.slice\(\)/u);
  assert.match(source, /hurt: 0/u);
  assert.match(source, /WORLD_UNITS_PER_CM = 0\.1/u);
  assert.match(source, /snapshot\.viewerSlot !== this\.joinSlot/u);
  assert.match(source, /socket\.readyState === WebSocket\.CONNECTING/u);
  assert.match(source, /this\.sentClocks\.get\(snapshot\.echoClock\)/u);
  assert.match(source, /if \(player\.slot === this\.latestSnapshot\.viewerSlot\) continue;/u);
  assert.match(source, /const eyeY = viewer\.y \+ 7\.3;/u);
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
});
