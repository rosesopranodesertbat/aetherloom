import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  DAMAGE_EVENT_TYPE,
  INPUT_FRAME_BYTES,
  MAX_SNAPSHOT_FRAME_BYTES,
  ProtocolError,
  composeTemplate,
  decodeSnapshotFrame,
  encodeInputFrame,
  isNewerSequence,
  movementToWorld,
  normalizeControlPlane,
  validateRoom,
  wireYawToCoreRadians,
  wireYawToRenderRadians,
  wrappedClockDelta,
} from '../site/multiplayer.js';

const root = fileURLToPath(new URL('..', import.meta.url));
const browserMatchRoomSource = await readFile(
  new URL('../server/cloudflare/src/browser-match-room.ts', import.meta.url),
  'utf8',
);

function snapshotFixture() {
  const byteLength = 24 + 2 * 24 + 16 + 16;
  const bytes = new Uint8Array(byteLength);
  const view = new DataView(bytes.buffer);
  view.setUint8(0, 1);
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
  view.setUint16(20, 0, true);
  view.setUint16(22, 0, true);

  let offset = 24;
  view.setUint8(offset, 2);
  view.setUint8(offset + 1, 1);
  view.setUint8(offset + 2, 0);
  view.setUint8(offset + 3, 3);
  view.setInt32(offset + 4, -1_234, true);
  view.setInt32(offset + 8, 5_678, true);
  view.setUint16(offset + 12, 0xff00, true);
  view.setUint16(offset + 14, 75, true);
  view.setUint16(offset + 16, 100, true);
  view.setUint16(offset + 18, 9, true);
  view.setUint32(offset + 20, 100, true);

  offset += 24;
  view.setUint8(offset, 5);
  view.setUint8(offset + 1, 2);
  view.setUint8(offset + 2, 0);
  view.setUint8(offset + 3, 0);
  view.setInt32(offset + 4, 7_654, true);
  view.setInt32(offset + 8, -3_210, true);
  view.setUint16(offset + 12, 0x0100, true);
  view.setUint16(offset + 14, 100, true);
  view.setUint16(offset + 16, 100, true);
  view.setUint16(offset + 18, 12, true);
  view.setUint32(offset + 20, 101, true);

  offset += 24;
  view.setUint32(offset, 800, true);
  view.setUint8(offset + 4, 2);
  view.setUint8(offset + 5, 0);
  view.setUint16(offset + 6, 48, true);
  view.setInt32(offset + 8, 50, true);
  view.setInt32(offset + 12, -75, true);

  offset += 16;
  view.setUint32(offset, 900, true);
  view.setUint8(offset + 4, DAMAGE_EVENT_TYPE);
  view.setUint8(offset + 5, 2);
  view.setUint8(offset + 6, 5);
  view.setUint8(offset + 7, 0);
  view.setInt32(offset + 8, 444, true);
  view.setInt32(offset + 12, -555, true);
  return bytes;
}

test('input encoder writes the exact 20-byte little-endian wire contract with snapshot ack', () => {
  const bytes = encodeInputFrame({
    sequence: 0x7856_3412,
    moveX: -127,
    moveY: 127,
    yaw: 0xbeef,
    cast: true,
    clientClockMs: 0x1234,
    snapshotAck: 0xfedc_ba98,
  });
  assert.equal(bytes.byteLength, INPUT_FRAME_BYTES);
  assert.deepEqual(Array.from(bytes), [
    1, 1, 20, 0,
    0x12, 0x34, 0x56, 0x78,
    0x81, 0x7f, 0xef, 0xbe,
    1, 0, 0x34, 0x12,
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
});

test('browser and authoritative host pin the same acknowledged input contract', () => {
  assert.match(browserMatchRoomSource, /const INPUT_FRAME_BYTES = 20;/u);
  assert.match(browserMatchRoomSource, /view\.getUint32\(16, true\)/u);
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
  assert.deepEqual(snapshot.players[0], {
    slot: 2,
    team: 1,
    status: 0,
    flags: 3,
    xcm: -1_234,
    zcm: 5_678,
    yaw: 0xff00,
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
    zcm: -75,
  });
  assert.deepEqual(snapshot.events[0], {
    eventId: 900,
    type: DAMAGE_EVENT_TYPE,
    actor: 2,
    target: 5,
    xcm: 444,
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
  reserved[22] = 1;
  assert.throws(() => decodeSnapshotFrame(reserved), /extension field/u);

  const duplicateSlot = fixture.slice();
  duplicateSlot[24 + 24] = duplicateSlot[24];
  assert.throws(() => decodeSnapshotFrame(duplicateSlot), /duplicate player slot/u);

  const impossibleHealth = fixture.slice();
  const view = new DataView(impossibleHealth.buffer);
  view.setUint16(24 + 14, 101, true);
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

test('camera-relative movement is quantized into server world axes', () => {
  // The core wire convention is 0 = +X and one quarter-turn = +Z.
  assert.deepEqual(movementToWorld(0, 127, 0), { x: 127, y: 0 });
  assert.deepEqual(movementToWorld(0, 127, 16_384), { x: 0, y: 127 });
  assert.deepEqual(movementToWorld(127, 0, 16_384), { x: 127, y: 0 });
  const diagonal = movementToWorld(90, 90, 8_192);
  assert.ok(Math.hypot(diagonal.x, diagonal.y) <= 128);
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
  assert.match(source, /\[\s*'aetherloom\.v1',/u);
  assert.match(source, /`aetherloom\.auth\.\$\{joined\.ticket\}`/u);
  assert.match(source, /previewScene\(scene, 0\)[\s\S]*?\.slice\(\)/u);
  assert.match(source, /hurt: 0/u);
  assert.match(source, /WORLD_UNITS_PER_CM = 0\.1/u);
  assert.match(source, /snapshot\.viewerSlot !== this\.joinSlot/u);
  assert.match(source, /socket\.readyState === WebSocket\.CONNECTING/u);
  assert.match(source, /this\.sentClocks\.get\(snapshot\.echoClock\)/u);
  assert.match(html, /Drag sky to aim/u);
  assert.doesNotMatch(
    html,
    /body\[data-connection="connected"\] \.stats \{ display: none; \}/u,
  );
  assert.match(index, /from '\.\/game\.js'/u);
  assert.match(index, /href="\.\/multiplayer\.html"/u);
});
