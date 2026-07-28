// AETHERLOOM — standalone browser multiplayer systems test.
//
// Authoritative gameplay stays on the staging match service. This client owns
// presentation, local input sampling, modest prediction, and remote
// interpolation only. The existing offline game remains entirely separate.
import { Renderer } from './engine.js';

export const DEFAULT_CONTROL_PLANE = 'https://aetherloom-control-plane-staging.calum-maciver.workers.dev';
export const INPUT_FRAME_BYTES = 22;
export const SNAPSHOT_HEADER_BYTES = 24;
export const PLAYER_RECORD_BYTES = 30;
export const VIEWER_COOLDOWN_BYTES = 26;
export const PROJECTILE_RECORD_BYTES = 24;
export const EVENT_RECORD_BYTES = 20;
export const CAST_EVENT_TYPE = 3;
export const DAMAGE_EVENT_TYPE = 4;
export const PROJECTILE_IMPACT_EVENT_TYPE = 10;
export const MAX_SNAPSHOT_FRAME_BYTES = 1_200;
export const MULTIPLAYER_SPELLS = Object.freeze([
  Object.freeze({ id: 0, name: 'Firebolt', key: '1', cooldownMs: 280 }),
  Object.freeze({ id: 7, name: 'Mend', key: '8', cooldownMs: 7_000 }),
]);

const PROTOCOL_VERSION = 3;
const INPUT_MESSAGE_TYPE = 1;
const SNAPSHOT_MESSAGE_TYPE = 2;
const INPUT_ACTION_CAST = 1;
const INSTANCE_STRIDE = 14;
const PARTICLE_STRIDE = 8;
const PREVIEW_Y = 512;
const RIVAL_SCENE = 21;
const FIREBOLT_SCENE = 25;
const FIREBOLT_TEMPLATE_COUNT = 24;
const FIREBOLT_TEMPLATE_FRAME_MS = 1_000 / 60;
const MAX_PLAYERS = 128;
const MAX_PROJECTILES = 255;
const MAX_EVENTS = 255;
const SPELL_SLOT_COUNT = 13;
const WORLD_UNITS_PER_CM = 0.1;
const CARPET_CLEARANCE = 7.5;
const PROJECTILE_CLEARANCE = 3.2;
const FIRST_PERSON_EYE_HEIGHT = 3;
const FIRST_PERSON_FOV = 1.16;
const FIRST_PERSON_MAX_PITCH = Math.round(1.05 / (Math.PI * 0.5) * 16_384);
const MOUSE_LOOK_UNITS_PER_PIXEL = 23;
const TOUCH_LOOK_UNITS_PER_PIXEL = 14;
const GAMEPAD_LOOK_DEADZONE = 0.16;
const GAMEPAD_YAW_UNITS_PER_SECOND = 780 * 64;
const GAMEPAD_PITCH_UNITS_PER_SECOND = 620 * 64;
const MAX_GAMEPAD_LOOK_FRAME_MS = 50;
const STRAFE_INPUT_SCALE = 0.55;
const PROJECTILE_LIFETIME_TICKS = 410;
const VIEWER_MUZZLE_EASE_TICKS = 12;
const VIEWER_MUZZLE_FORWARD = 6;
const VIEWER_MUZZLE_RIGHT = 1.25;
const VIEWER_MUZZLE_DOWN = 1.5;
const AUTHORITATIVE_MUZZLE_FORWARD = 1.4;
const AUTHORITATIVE_MUZZLE_HEIGHT = 2.3;
const FIREBOLT_PRESENTATION_SPEED = 220;
const INPUT_INTERVAL_MS = 1_000 / 64;
const CORE_MOVE_AXIS = 2_047;
const INPUT_HOLD_TICKS = 2;
const PLAYER_PLANAR_SPEED_CM_PER_TICK = 10;
const PLAYER_VERTICAL_SPEED_CM_PER_TICK = 6;
const MAX_PREDICTION_HISTORY = 256;
const MAX_PITCH = 16_384;
const PLAYER_MIN_ALTITUDE_CM = 0;
const PLAYER_MAX_ALTITUDE_CM = 4_300;
const MAX_RECONCILIATION_OFFSET = 96;
const RECONCILIATION_DECAY_MS = 105;
const MAX_IMPACTS = 32;
const MAX_HEALS = 32;
const MAX_SEEN_EVENTS = 512;
const MULTIPLAYER_SPELL_IDS = new Set(MULTIPLAYER_SPELLS.map((spell) => spell.id));

export class ProtocolError extends Error {
  constructor(code, message) {
    super(message);
    this.name = 'ProtocolError';
    this.code = code;
  }
}

class JoinError extends Error {
  constructor(status, code, message) {
    super(message);
    this.name = 'JoinError';
    this.status = status;
    this.code = code;
  }
}

function protocolAssert(condition, code, message) {
  if (!condition) throw new ProtocolError(code, message);
}

function integerIn(value, minimum, maximum, label) {
  protocolAssert(Number.isInteger(value), 'invalid_input', `${label} must be an integer.`);
  protocolAssert(value >= minimum && value <= maximum, 'input_out_of_range', `${label} is out of range.`);
  return value;
}

function bytesOf(payload) {
  if (payload instanceof ArrayBuffer) return new Uint8Array(payload);
  if (ArrayBuffer.isView(payload)) {
    return new Uint8Array(payload.buffer, payload.byteOffset, payload.byteLength);
  }
  throw new ProtocolError('not_binary', 'Snapshot payload must be binary.');
}

export function encodeInputFrame({
  sequence,
  moveX = 0,
  moveY = 0,
  moveVertical = 0,
  yaw = 0,
  pitch = 0,
  cast = false,
  spell = 0,
  clientClockMs = 0,
  snapshotAck = 0,
}) {
  integerIn(sequence, 1, 0xffff_ffff, 'sequence');
  integerIn(moveX, -127, 127, 'moveX');
  integerIn(moveY, -127, 127, 'moveY');
  integerIn(moveVertical, -127, 127, 'moveVertical');
  integerIn(yaw, 0, 0xffff, 'yaw');
  integerIn(pitch, -127, 127, 'pitch');
  integerIn(spell, 0, 12, 'spell');
  protocolAssert(
    !cast || MULTIPLAYER_SPELL_IDS.has(spell),
    'unsupported_spell',
    'Spell is not implemented by the staging duel.',
  );
  integerIn(clientClockMs, 0, 0xffff, 'clientClockMs');
  integerIn(snapshotAck, 0, 0xffff_ffff, 'snapshotAck');
  const bytes = new Uint8Array(INPUT_FRAME_BYTES);
  const view = new DataView(bytes.buffer);
  view.setUint8(0, PROTOCOL_VERSION);
  view.setUint8(1, INPUT_MESSAGE_TYPE);
  view.setUint16(2, INPUT_FRAME_BYTES, true);
  view.setUint32(4, sequence, true);
  view.setInt8(8, moveX);
  view.setInt8(9, moveY);
  view.setInt8(10, moveVertical);
  view.setInt8(11, pitch);
  view.setUint16(12, yaw, true);
  view.setUint8(14, cast ? INPUT_ACTION_CAST : 0);
  view.setUint8(15, cast ? spell : 0xff);
  view.setUint16(16, clientClockMs, true);
  view.setUint32(18, snapshotAck, true);
  return bytes;
}

function rejectDuplicate(set, value, label) {
  protocolAssert(!set.has(value), 'duplicate_record', `Snapshot contains duplicate ${label} ${value}.`);
  set.add(value);
}

export function decodeSnapshotFrame(payload) {
  const bytes = bytesOf(payload);
  protocolAssert(bytes.byteLength >= SNAPSHOT_HEADER_BYTES, 'truncated_header', 'Snapshot header is truncated.');
  protocolAssert(
    bytes.byteLength <= MAX_SNAPSHOT_FRAME_BYTES,
    'oversized_frame',
    'Snapshot exceeds the 1,200-byte gameplay frame limit.',
  );
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  protocolAssert(view.getUint8(0) === PROTOCOL_VERSION, 'bad_version', 'Unsupported snapshot protocol version.');
  protocolAssert(view.getUint8(1) === SNAPSHOT_MESSAGE_TYPE, 'bad_type', 'Unexpected binary message type.');
  const declaredLength = view.getUint16(2, true);
  protocolAssert(declaredLength === bytes.byteLength, 'bad_length', 'Snapshot length does not match its header.');

  const playerCount = view.getUint8(15);
  const projectileCount = view.getUint8(16);
  const eventCount = view.getUint8(17);
  protocolAssert((view.getUint16(18, true) & ~0b11) === 0, 'reserved_bits', 'Snapshot match flags contain unknown bits.');
  protocolAssert(playerCount <= MAX_PLAYERS, 'too_many_players', 'Snapshot player count exceeds the client limit.');
  protocolAssert(projectileCount <= MAX_PROJECTILES, 'too_many_projectiles', 'Snapshot projectile count exceeds the client limit.');
  protocolAssert(eventCount <= MAX_EVENTS, 'too_many_events', 'Snapshot event count exceeds the client limit.');
  const expectedLength = SNAPSHOT_HEADER_BYTES
    + playerCount * PLAYER_RECORD_BYTES
    + VIEWER_COOLDOWN_BYTES
    + projectileCount * PROJECTILE_RECORD_BYTES
    + eventCount * EVENT_RECORD_BYTES;
  protocolAssert(expectedLength === bytes.byteLength, 'record_length_mismatch', 'Snapshot record counts do not match its length.');
  const players = [];
  const projectiles = [];
  const events = [];
  const slots = new Set();
  const playerEntities = new Set();
  const projectileEntities = new Set();
  const eventIds = new Set();
  let offset = SNAPSHOT_HEADER_BYTES;

  for (let index = 0; index < playerCount; index += 1, offset += PLAYER_RECORD_BYTES) {
    const slot = view.getUint8(offset);
    const entity = view.getUint32(offset + 26, true);
    const status = view.getUint8(offset + 2);
    const flags = view.getUint8(offset + 3);
    const pitch = view.getInt16(offset + 18, true);
    rejectDuplicate(slots, slot, 'player slot');
    rejectDuplicate(playerEntities, entity, 'player entity');
    protocolAssert(status >= 1 && status <= 5, 'invalid_status', 'Player status is out of range.');
    protocolAssert((flags & ~0b111) === 0, 'reserved_bits', 'Player flags contain unknown bits.');
    protocolAssert(
      pitch >= -MAX_PITCH && pitch <= MAX_PITCH,
      'input_out_of_range',
      'Player pitch is out of range.',
    );
    const health = view.getUint16(offset + 20, true);
    const maxHealth = view.getUint16(offset + 22, true);
    protocolAssert(maxHealth === 0 || health <= maxHealth, 'invalid_health', 'Player health exceeds maximum health.');
    players.push({
      slot,
      team: view.getUint8(offset + 1),
      status,
      flags,
      xcm: view.getInt32(offset + 4, true),
      ycm: view.getInt32(offset + 8, true),
      zcm: view.getInt32(offset + 12, true),
      yaw: view.getUint16(offset + 16, true),
      pitch,
      health,
      maxHealth,
      score: view.getUint16(offset + 24, true),
      entity,
    });
  }
  protocolAssert(
    slots.has(view.getUint8(14)),
    'missing_viewer',
    'Snapshot does not contain its viewer player.',
  );
  const viewerSlot = view.getUint8(14);
  const viewer = players.find((player) => player.slot === viewerSlot);
  const spellCooldownTicks = [];
  for (let spell = 0; spell < SPELL_SLOT_COUNT; spell += 1) {
    spellCooldownTicks.push(view.getUint16(offset + spell * 2, true));
  }
  offset += VIEWER_COOLDOWN_BYTES;
  viewer.spellCooldownTicks = spellCooldownTicks;

  for (let index = 0; index < projectileCount; index += 1, offset += PROJECTILE_RECORD_BYTES) {
    protocolAssert(view.getUint8(offset + 5) === 0, 'reserved_bits', 'Projectile reserved field is not zero.');
    const entity = view.getUint32(offset, true);
    const pitch = view.getInt16(offset + 22, true);
    rejectDuplicate(projectileEntities, entity, 'projectile entity');
    protocolAssert(
      pitch >= -MAX_PITCH && pitch <= MAX_PITCH,
      'input_out_of_range',
      'Projectile pitch is out of range.',
    );
    projectiles.push({
      entity,
      owner: view.getUint8(offset + 4),
      ttl: view.getUint16(offset + 6, true),
      xcm: view.getInt32(offset + 8, true),
      ycm: view.getInt32(offset + 12, true),
      zcm: view.getInt32(offset + 16, true),
      yaw: view.getUint16(offset + 20, true),
      pitch,
    });
  }

  for (let index = 0; index < eventCount; index += 1, offset += EVENT_RECORD_BYTES) {
    const type = view.getUint8(offset + 4);
    const detail = view.getUint8(offset + 7);
    if (type === CAST_EVENT_TYPE || type === PROJECTILE_IMPACT_EVENT_TYPE) {
      protocolAssert(
        MULTIPLAYER_SPELL_IDS.has(detail),
        'unsupported_spell',
        'Snapshot spell event names an unsupported spell.',
      );
      protocolAssert(
        type !== PROJECTILE_IMPACT_EVENT_TYPE || detail === 0,
        'unsupported_spell',
        'Snapshot projectile impact names an unsupported spell.',
      );
    } else {
      protocolAssert(detail === 0, 'reserved_bits', 'Event reserved field is not zero.');
    }
    const eventId = view.getUint32(offset, true);
    rejectDuplicate(eventIds, eventId, 'event id');
    events.push({
      eventId,
      type,
      actor: view.getUint8(offset + 5),
      target: view.getUint8(offset + 6),
      spell:
        type === CAST_EVENT_TYPE || type === PROJECTILE_IMPACT_EVENT_TYPE
          ? detail
          : null,
      xcm: view.getInt32(offset + 8, true),
      ycm: view.getInt32(offset + 12, true),
      zcm: view.getInt32(offset + 16, true),
    });
  }

  return {
    version: view.getUint8(0),
    type: view.getUint8(1),
    byteLength: declaredLength,
    tick: view.getUint32(4, true),
    snapshotSeq: view.getUint32(8, true),
    echoClock: view.getUint16(12, true),
    viewerSlot,
    playerCount,
    projectileCount,
    eventCount,
    matchFlags: view.getUint16(18, true),
    acknowledgedInputSequence: view.getUint32(20, true),
    players,
    projectiles,
    events,
  };
}

export function isNewerSequence(candidate, previous) {
  if (previous === null || previous === undefined) return true;
  const distance = (candidate - previous) >>> 0;
  return distance !== 0 && distance < 0x8000_0000;
}

export function sequenceAcknowledges(acknowledged, candidate) {
  integerIn(acknowledged, 0, 0xffff_ffff, 'acknowledged sequence');
  integerIn(candidate, 1, 0xffff_ffff, 'candidate sequence');
  return acknowledged !== 0 &&
    (acknowledged === candidate || isNewerSequence(acknowledged, candidate));
}

export function wrappedClockDelta(now, echoed) {
  integerIn(now, 0, 0xffff, 'now');
  integerIn(echoed, 0, 0xffff, 'echoed');
  return (now - echoed + 0x1_0000) & 0xffff;
}

export function normalizeControlPlane(value) {
  const url = new URL(value || DEFAULT_CONTROL_PLANE);
  const local = url.hostname === 'localhost' || url.hostname === '127.0.0.1' || url.hostname === '[::1]';
  if (url.protocol !== 'https:' && !(local && url.protocol === 'http:')) {
    throw new Error('The control plane must use HTTPS, except on localhost.');
  }
  if (url.username || url.password) throw new Error('The control-plane URL cannot include credentials.');
  url.search = '';
  url.hash = '';
  url.pathname = url.pathname.replace(/\/+$/, '');
  return url.toString().replace(/\/$/, '');
}

export function validateRoom(value) {
  const room = String(value ?? '').trim().toLowerCase();
  if (!/^duel-(?:0[1-9]|[12][0-9]|3[0-2])$/u.test(room)) {
    throw new Error('Public staging rooms are duel-01 through duel-32.');
  }
  return room;
}

export function resumeSessionKey(controlPlane, room) {
  return `aetherloom.demo.resume.${encodeURIComponent(normalizeControlPlane(controlPlane))}.${validateRoom(room)}`;
}

export function joinAttemptKey(controlPlane, room) {
  return `aetherloom.demo.join.${encodeURIComponent(normalizeControlPlane(controlPlane))}.${validateRoom(room)}`;
}

export function isResumeToken(value) {
  return typeof value === 'string' && /^demo_resume_[A-Za-z0-9_-]{24}$/u.test(value);
}

export function isJoinNonce(value) {
  return typeof value === 'string' && /^demo_join_[A-Za-z0-9_-]{24}$/u.test(value);
}

function bytesToBase64Url(bytes) {
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/gu, '-').replace(/\//gu, '_').replace(/=+$/u, '');
}

export function randomJoinNonce(cryptoProvider = globalThis.crypto) {
  if (!cryptoProvider?.getRandomValues) {
    throw new Error('Secure randomness is unavailable for the staging join.');
  }
  const bytes = cryptoProvider.getRandomValues(new Uint8Array(18));
  return `demo_join_${bytesToBase64Url(bytes)}`;
}

export async function demoResumeToken(room, joinNonce, cryptoProvider = globalThis.crypto) {
  const validRoom = validateRoom(room);
  if (!isJoinNonce(joinNonce) || !cryptoProvider?.subtle?.digest) {
    throw new Error('The staging join attempt is invalid.');
  }
  const input = new TextEncoder().encode(
    `aetherloom-demo-resume:v1:${validRoom}:${joinNonce}`,
  );
  const digest = new Uint8Array(await cryptoProvider.subtle.digest('SHA-256', input));
  return `demo_resume_${bytesToBase64Url(digest).slice(0, 24)}`;
}

export class ResumeTokenStore {
  constructor(storage = null) {
    this.storage = storage;
    this.memory = new Map();
  }

  load(key) {
    if (this.storage) {
      try {
        const token = this.storage.getItem(key);
        if (isResumeToken(token)) {
          this.memory.set(key, token);
          return token;
        }
        this.memory.delete(key);
        if (token !== null) this.storage.removeItem(key);
        return undefined;
      } catch {
        // Storage can be unavailable in privacy modes and opaque origins.
      }
    }
    return this.memory.get(key);
  }

  save(key, token, { persist = true } = {}) {
    if (!isResumeToken(token)) throw new Error('Control plane returned an invalid resume token.');
    this.memory.set(key, token);
    if (persist && this.storage) {
      try { this.storage.setItem(key, token); }
      catch { /* The in-memory copy still supports reconnecting this page. */ }
    }
  }

  clear(key) {
    this.memory.delete(key);
    if (this.storage) {
      try { this.storage.removeItem(key); }
      catch { /* Storage is an optional reconnect optimization. */ }
    }
  }
}

export class JoinNonceStore {
  constructor(storage = null) {
    this.storage = storage;
    this.memory = new Map();
  }

  load(key) {
    if (this.storage) {
      try {
        const nonce = this.storage.getItem(key);
        if (isJoinNonce(nonce)) {
          this.memory.set(key, nonce);
          return nonce;
        }
        this.memory.delete(key);
        if (nonce !== null) this.storage.removeItem(key);
        return undefined;
      } catch {
        // The in-memory attempt still makes retries idempotent in this page.
      }
    }
    return this.memory.get(key);
  }

  save(key, nonce, { persist = true } = {}) {
    if (!isJoinNonce(nonce)) throw new Error('The staging join nonce is invalid.');
    this.memory.set(key, nonce);
    if (persist && this.storage) {
      try { this.storage.setItem(key, nonce); }
      catch { /* The current page can still retry the same attempt. */ }
    }
  }

  clear(key) {
    this.memory.delete(key);
    if (this.storage) {
      try { this.storage.removeItem(key); }
      catch { /* Storage is optional. */ }
    }
  }
}

export class ResumeOwnership {
  constructor(lockManager = null) {
    this.lockManager = lockManager;
    this.held = new Map();
  }

  get supported() {
    return typeof this.lockManager?.request === 'function';
  }

  async claim(scope, token) {
    const name = `aetherloom:resume:${scope}:${token}`;
    if (this.held.has(name)) return true;
    if (!this.supported) return false;
    return new Promise((resolve) => {
      let settled = false;
      const settle = (value) => {
        if (settled) return;
        settled = true;
        resolve(value);
      };
      try {
        const requested = this.lockManager.request(
          name,
          { ifAvailable: true, mode: 'exclusive' },
          async (lock) => {
            if (!lock) {
              settle(false);
              return;
            }
            let release;
            const held = new Promise((done) => { release = done; });
            this.held.set(name, release);
            settle(true);
            await held;
            this.held.delete(name);
          },
        );
        Promise.resolve(requested).catch(() => settle(false));
      } catch {
        settle(false);
      }
    });
  }
}

export class SingleFlight {
  constructor() {
    this.current = null;
  }

  run(task) {
    if (this.current !== null) return this.current;
    const pending = Promise.resolve().then(task);
    const tracked = pending.finally(() => {
      if (this.current === tracked) this.current = null;
    });
    this.current = tracked;
    return tracked;
  }

  reset() {
    this.current = null;
  }
}

export function movementToWorld(moveX, moveY, yaw, pitch = 0, moveVertical = 0) {
  integerIn(moveX, -127, 127, 'moveX');
  integerIn(moveY, -127, 127, 'moveY');
  integerIn(moveVertical, -127, 127, 'moveVertical');
  integerIn(yaw, 0, 0xffff, 'yaw');
  integerIn(pitch, -MAX_PITCH, MAX_PITCH, 'pitch');
  const radians = wireYawToCoreRadians(yaw);
  const pitchRadians = wirePitchToRadians(pitch);
  const level = Math.cos(pitchRadians);
  const strafe = moveX / 127;
  const forward = moveY / 127;
  const lift = moveVertical / 127;
  const worldX = clamp(Math.round((
    Math.cos(radians) * level * forward -
    Math.sin(radians) * strafe
  ) * 127), -127, 127);
  const worldY = clamp(Math.round((
    Math.sin(pitchRadians) * forward +
    lift
  ) * 127), -127, 127);
  const worldZ = clamp(Math.round((
    Math.sin(radians) * level * forward +
    Math.cos(radians) * strafe
  ) * 127), -127, 127);
  return {
    x: worldX === 0 ? 0 : worldX,
    y: worldZ === 0 ? 0 : worldZ,
    vertical: worldY === 0 ? 0 : worldY,
  };
}

export function shapeFlightInput(strafe, thrust) {
  protocolAssert(
    Number.isFinite(strafe) &&
    Number.isFinite(thrust) &&
    Math.abs(strafe) <= 1 &&
    Math.abs(thrust) <= 1,
    'invalid_input',
    'Flight input is out of range.',
  );
  return {
    strafe: strafe * STRAFE_INPUT_SCALE,
    thrust,
  };
}

export function wirePitchToRadians(pitch) {
  protocolAssert(
    Number.isFinite(pitch) && pitch >= -MAX_PITCH && pitch <= MAX_PITCH,
    'input_out_of_range',
    'pitch is out of range.',
  );
  return pitch / MAX_PITCH * Math.PI * 0.5;
}

function firstPersonBasis(yawValue, pitchValue) {
  const yaw = wireYawToCoreRadians(yawValue);
  const pitch = wirePitchToRadians(pitchValue);
  const level = Math.cos(pitch);
  return {
    forward: [
      Math.cos(yaw) * level,
      Math.sin(pitch),
      Math.sin(yaw) * level,
    ],
    right: [-Math.sin(yaw), 0, Math.cos(yaw)],
    up: [
      -Math.cos(yaw) * Math.sin(pitch),
      Math.cos(pitch),
      -Math.sin(yaw) * Math.sin(pitch),
    ],
  };
}

export function firstPersonCamera(viewer) {
  protocolAssert(
    viewer &&
    [viewer.x, viewer.y, viewer.z, viewer.yaw, viewer.pitch].every(Number.isFinite),
    'invalid_camera',
    'First-person viewer pose is invalid.',
  );
  const { forward, up } = firstPersonBasis(viewer.yaw, viewer.pitch);
  const eyeX = viewer.x;
  const eyeY = viewer.y + FIRST_PERSON_EYE_HEIGHT;
  const eyeZ = viewer.z;
  return {
    ex: eyeX,
    ey: eyeY,
    ez: eyeZ,
    cx: eyeX + forward[0] * 80,
    cy: eyeY + forward[1] * 80,
    cz: eyeZ + forward[2] * 80,
    ux: up[0],
    uy: up[1],
    uz: up[2],
  };
}

export function firstPersonMuzzle(viewer) {
  protocolAssert(
    viewer &&
    [viewer.x, viewer.y, viewer.z, viewer.yaw, viewer.pitch].every(Number.isFinite),
    'invalid_camera',
    'First-person viewer pose is invalid.',
  );
  const { forward, right, up } = firstPersonBasis(viewer.yaw, viewer.pitch);
  const eye = [viewer.x, viewer.y + FIRST_PERSON_EYE_HEIGHT, viewer.z];
  return {
    x:
      eye[0] +
      forward[0] * VIEWER_MUZZLE_FORWARD +
      right[0] * VIEWER_MUZZLE_RIGHT -
      up[0] * VIEWER_MUZZLE_DOWN,
    y:
      eye[1] +
      forward[1] * VIEWER_MUZZLE_FORWARD +
      right[1] * VIEWER_MUZZLE_RIGHT -
      up[1] * VIEWER_MUZZLE_DOWN,
    z:
      eye[2] +
      forward[2] * VIEWER_MUZZLE_FORWARD +
      right[2] * VIEWER_MUZZLE_RIGHT -
      up[2] * VIEWER_MUZZLE_DOWN,
    yaw: viewer.yaw,
    pitch: viewer.pitch,
  };
}

export function viewerProjectilePresentation(
  position,
  projectile,
  viewerSlot,
  elapsedTicks = PROJECTILE_LIFETIME_TICKS - projectile.ttl,
) {
  if (projectile.owner !== viewerSlot) return { ...position };
  const elapsed = clamp(
    elapsedTicks,
    0,
    VIEWER_MUZZLE_EASE_TICKS,
  );
  const amount = 1 - elapsed / VIEWER_MUZZLE_EASE_TICKS;
  const { forward, right, up } = firstPersonBasis(
    projectile.yaw,
    projectile.pitch,
  );
  const presentationOffset = [
    forward[0] * VIEWER_MUZZLE_FORWARD +
      right[0] * VIEWER_MUZZLE_RIGHT -
      up[0] * VIEWER_MUZZLE_DOWN -
      forward[0] * AUTHORITATIVE_MUZZLE_FORWARD,
    FIRST_PERSON_EYE_HEIGHT +
      forward[1] * VIEWER_MUZZLE_FORWARD +
      right[1] * VIEWER_MUZZLE_RIGHT -
      up[1] * VIEWER_MUZZLE_DOWN -
      AUTHORITATIVE_MUZZLE_HEIGHT,
    forward[2] * VIEWER_MUZZLE_FORWARD +
      right[2] * VIEWER_MUZZLE_RIGHT -
      up[2] * VIEWER_MUZZLE_DOWN -
      forward[2] * AUTHORITATIVE_MUZZLE_FORWARD,
  ];
  return {
    ...position,
    x: position.x + presentationOffset[0] * amount,
    y: position.y + presentationOffset[1] * amount,
    z: position.z + presentationOffset[2] * amount,
  };
}

export function pendingInputDisplacement(history, time) {
  protocolAssert(Array.isArray(history) && Number.isFinite(time), 'invalid_prediction', 'Prediction inputs are invalid.');
  const displacement = { x: 0, y: 0, z: 0 };
  for (const input of history) {
    protocolAssert(
      input &&
      [input.sentAt, input.x, input.y, input.z].every(Number.isFinite),
      'invalid_prediction',
      'Prediction history contains an invalid sample.',
    );
    const amount = clamp((time - input.sentAt) / INPUT_INTERVAL_MS, 0, 1);
    displacement.x += predictedBrowserAxisDisplacement(
      input.x,
      PLAYER_PLANAR_SPEED_CM_PER_TICK,
    ) * amount;
    displacement.y += predictedBrowserAxisDisplacement(
      input.y,
      PLAYER_VERTICAL_SPEED_CM_PER_TICK,
    ) * amount;
    displacement.z += predictedBrowserAxisDisplacement(
      input.z,
      PLAYER_PLANAR_SPEED_CM_PER_TICK,
    ) * amount;
  }
  return displacement;
}

export function predictedBrowserAxisDisplacement(compactAxis, speedCmPerTick) {
  integerIn(compactAxis, -127, 127, 'compactAxis');
  protocolAssert(
    Number.isInteger(speedCmPerTick) && speedCmPerTick >= 0,
    'invalid_prediction',
    'Prediction speed is invalid.',
  );
  // Mirror the host's int8 -> i16 rounding and Rust's signed integer division
  // (truncation toward zero) before applying the two-tick browser hold.
  const coreAxis = Math.round(compactAxis * CORE_MOVE_AXIS / 127);
  const velocityCmPerTick = Math.trunc(coreAxis * speedCmPerTick / CORE_MOVE_AXIS);
  return velocityCmPerTick * INPUT_HOLD_TICKS * WORLD_UNITS_PER_CM;
}

export function predictViewerPosition(target, displacement, heightAt) {
  protocolAssert(
    target &&
    displacement &&
    [
      target.x,
      target.y,
      target.z,
      target.altitudeCm,
      displacement.x,
      displacement.y,
      displacement.z,
    ]
      .every(Number.isFinite) &&
    typeof heightAt === 'function',
    'invalid_prediction',
    'Predicted viewer position is invalid.',
  );
  const x = target.x + displacement.x;
  const z = target.z + displacement.z;
  const groundDelta = heightAt(x, z) - heightAt(target.x, target.z);
  protocolAssert(Number.isFinite(groundDelta), 'invalid_prediction', 'Predicted terrain height is invalid.');
  const authoritativeAltitude = target.altitudeCm * WORLD_UNITS_PER_CM;
  const predictedAltitude = clamp(
    authoritativeAltitude + displacement.y,
    PLAYER_MIN_ALTITUDE_CM * WORLD_UNITS_PER_CM,
    PLAYER_MAX_ALTITUDE_CM * WORLD_UNITS_PER_CM,
  );
  return {
    ...target,
    x,
    y: target.y + (predictedAltitude - authoritativeAltitude) + groundDelta,
    z,
    altitudeCm: predictedAltitude / WORLD_UNITS_PER_CM,
  };
}

export function bilinearHeight(heights, width, cellSize, worldSize, x, z) {
  protocolAssert(
    heights instanceof Float32Array &&
    Number.isInteger(width) &&
    width >= 2 &&
    heights.length >= width * width &&
    Number.isFinite(cellSize) &&
    cellSize > 0 &&
    Number.isFinite(worldSize) &&
    worldSize > 0 &&
    Number.isFinite(x) &&
    Number.isFinite(z),
    'invalid_terrain',
    'Terrain sampling inputs are invalid.',
  );
  if (x < 0 || z < 0 || x >= worldSize || z >= worldSize) return 0;
  const gridX = clamp(x / cellSize, 0, width - 1);
  const gridZ = clamp(z / cellSize, 0, width - 1);
  const x0 = Math.floor(gridX);
  const z0 = Math.floor(gridZ);
  const x1 = Math.min(x0 + 1, width - 1);
  const z1 = Math.min(z0 + 1, width - 1);
  const amountX = gridX - x0;
  const amountZ = gridZ - z0;
  const top = heights[z0 * width + x0]
    + (heights[z0 * width + x1] - heights[z0 * width + x0]) * amountX;
  const bottom = heights[z1 * width + x0]
    + (heights[z1 * width + x1] - heights[z1 * width + x0]) * amountX;
  return top + (bottom - top) * amountZ;
}

export function nextPacedDeadline(previousDeadline, now, interval) {
  protocolAssert(
    Number.isFinite(previousDeadline) && Number.isFinite(now) && Number.isFinite(interval) && interval > 0,
    'invalid_pacer',
    'Input pacing values must be finite and positive.',
  );
  const scheduled = previousDeadline + interval;
  return scheduled <= now ? now + interval : scheduled;
}

export function socketCloseMessage(code, reason = '') {
  const safeReasons = new Map([
    ['sustained input flood', 'The match rejected a sustained input flood. Reload before trying again.'],
    ['join ticket replayed', 'This one-time match ticket was already used. Join the room again.'],
  ]);
  if (code === 1008 && safeReasons.has(reason)) return safeReasons.get(reason);
  return new Map([
    [1002, 'The match protocol was rejected. Reload after the client and server builds match.'],
    [1003, 'The match rejected the binary input format.'],
    [1008, 'The match rejected this connection for a policy reason. Join again; if it repeats, report the room code.'],
    [4003, 'This 15-minute staging session ended. Choose Join to start a new test session.'],
    [4001, 'This room session was opened in another tab. Join here again to take it over.'],
  ]).get(code) ?? null;
}

function appendTemplate(
  destination,
  instanceCount,
  template,
  templateOrigin,
  worldPosition,
  yaw,
  pitch = 0,
) {
  const templateCount = template.length / INSTANCE_STRIDE;
  const capacity = destination.length / INSTANCE_STRIDE;
  if (instanceCount + templateCount > capacity) return instanceCount;
  const sine = Math.sin(yaw);
  const cosine = Math.cos(yaw);
  const pitchSine = Math.sin(pitch);
  const pitchCosine = Math.cos(pitch);
  for (let index = 0; index < templateCount; index += 1) {
    const source = index * INSTANCE_STRIDE;
    const target = (instanceCount + index) * INSTANCE_STRIDE;
    const localX = template[source] - templateOrigin[0];
    const localY = template[source + 1] - templateOrigin[1];
    const localZ = template[source + 2] - templateOrigin[2];
    const pitchedY = localY * pitchCosine + localZ * pitchSine;
    const pitchedZ = -localY * pitchSine + localZ * pitchCosine;
    destination[target] = worldPosition[0] + localX * cosine + pitchedZ * sine;
    destination[target + 1] = worldPosition[1] + pitchedY;
    destination[target + 2] = worldPosition[2] - localX * sine + pitchedZ * cosine;
    for (let field = 3; field < INSTANCE_STRIDE; field += 1) {
      destination[target + field] = template[source + field];
    }
    destination[target + 9] = template[source + 9] + yaw;
    destination[target + 10] = template[source + 10] - pitch;
  }
  return instanceCount + templateCount;
}

export function composeTemplate(template, templateOrigin, worldPosition, yaw) {
  protocolAssert(template instanceof Float32Array, 'invalid_template', 'Template must be a Float32Array.');
  protocolAssert(template.length % INSTANCE_STRIDE === 0, 'invalid_template', 'Template has an invalid stride.');
  protocolAssert(
    templateOrigin.length === 3 && worldPosition.length === 3 && Number.isFinite(yaw),
    'invalid_transform',
    'Template transform is invalid.',
  );
  const result = new Float32Array(template.length);
  appendTemplate(result, 0, template, templateOrigin, worldPosition, yaw, 0);
  return result;
}

function number(value, label, minimum, maximum) {
  if (!Number.isInteger(value) || value < minimum || value > maximum) {
    throw new Error(`Join response has an invalid ${label}.`);
  }
  return value;
}

function string(value, label, maximum = 4096) {
  if (typeof value !== 'string' || value.length === 0 || value.length > maximum) {
    throw new Error(`Join response has an invalid ${label}.`);
  }
  return value;
}

function parseJoinResponse(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error('Control plane returned an invalid join response.');
  }
  const ticket = string(value.ticket, 'ticket');
  if (!/^[A-Za-z0-9._~-]+$/u.test(ticket)) throw new Error('Join ticket cannot be used as a WebSocket protocol.');
  const webSocketUrl = new URL(string(value.webSocketUrl, 'WebSocket URL'));
  const local = webSocketUrl.hostname === 'localhost' || webSocketUrl.hostname === '127.0.0.1' || webSocketUrl.hostname === '[::1]';
  if (webSocketUrl.protocol !== 'wss:' && !(local && webSocketUrl.protocol === 'ws:')) {
    throw new Error('Match WebSocket must use WSS, except on localhost.');
  }
  const resumeToken = string(value.resumeToken, 'resume token', 36);
  if (!isResumeToken(resumeToken)) throw new Error('Join response has an invalid resume token.');
  const result = {
    matchId: string(value.matchId, 'match id', 128),
    accountId: string(value.accountId, 'account id', 128),
    slot: number(value.slot, 'slot', 0, 255),
    resumeToken,
    webSocketUrl: webSocketUrl.toString(),
    ticket,
    tickHz: number(value.tickHz, 'tick rate', 1, 256),
    inputHz: number(value.inputHz, 'input rate', 1, 128),
    snapshotHz: number(value.snapshotHz, 'snapshot rate', 1, 128),
  };
  if (result.tickHz !== 128 || result.inputHz !== 64 || result.snapshotHz !== 32) {
    throw new Error('Staging returned unexpected simulation or network rates.');
  }
  return result;
}

function nowMs() {
  return typeof performance !== 'undefined' ? performance.now() : Date.now();
}

function clock16() {
  return Math.floor(nowMs()) & 0xffff;
}

function clamp(value, low, high) {
  return Math.max(low, Math.min(high, value));
}

export function wireYawToCoreRadians(yaw) {
  return yaw / 0x1_0000 * Math.PI * 2;
}

export function wireYawToRenderRadians(yaw) {
  return Math.PI * 0.5 - wireYawToCoreRadians(yaw);
}

export function yawAfterLookDelta(yaw, delta) {
  integerIn(yaw, 0, 0xffff, 'yaw');
  protocolAssert(Number.isFinite(delta), 'invalid_input', 'look delta must be finite.');
  return (yaw + Math.round(delta) + 0x1_0000) & 0xffff;
}

export function gamepadLookDelta(axis, elapsedMs, unitsPerSecond) {
  protocolAssert(Number.isFinite(axis), 'invalid_input', 'gamepad look axis must be finite.');
  protocolAssert(
    Number.isFinite(elapsedMs) && elapsedMs >= 0,
    'invalid_input',
    'gamepad look elapsed time must be finite and non-negative.',
  );
  protocolAssert(
    Number.isFinite(unitsPerSecond) && unitsPerSecond >= 0,
    'invalid_input',
    'gamepad look rate must be finite and non-negative.',
  );
  const shapedAxis = Math.abs(axis) < GAMEPAD_LOOK_DEADZONE
    ? 0
    : clamp(axis, -1, 1);
  return shapedAxis
    * unitsPerSecond
    * Math.min(elapsedMs, MAX_GAMEPAD_LOOK_FRAME_MS)
    / 1_000;
}

export function authoritativeCooldownReadyAt(
  receivedAt,
  remainingTicks,
  rttMs = 0,
) {
  protocolAssert(
    Number.isFinite(receivedAt) &&
    Number.isInteger(remainingTicks) &&
    remainingTicks >= 0 &&
    remainingTicks <= 0xffff &&
    Number.isFinite(rttMs) &&
    rttMs >= 0,
    'invalid_cooldown',
    'Authoritative cooldown timing is invalid.',
  );
  if (remainingTicks === 0) return receivedAt;
  const estimatedSnapshotAge = Math.min(rttMs * 0.5, 250);
  return Math.max(
    receivedAt,
    receivedAt + remainingTicks * 1_000 / 128 - estimatedSnapshotAge,
  );
}

function interpolateYaw(from, to, amount) {
  const difference = ((to - from + 0x8000) & 0xffff) - 0x8000;
  return (from + difference * amount + 0x1_0000) % 0x1_0000;
}

function hashString(value) {
  let hash = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash >>> 0 || 1;
}

function hashUnit(seed) {
  let value = seed >>> 0;
  value ^= value >>> 16;
  value = Math.imul(value, 0x7feb352d);
  value ^= value >>> 15;
  value = Math.imul(value, 0x846ca68b);
  value ^= value >>> 16;
  return (value >>> 0) / 0x1_0000_0000;
}

function randomRoom() {
  const values = new Uint32Array(1);
  if (globalThis.crypto?.getRandomValues) globalThis.crypto.getRandomValues(values);
  else values[0] = Math.floor(Math.random() * 0xffff_ffff);
  return `duel-${String(values[0] % 32 + 1).padStart(2, '0')}`;
}

function sampleTrack(track, time, duration) {
  if (!track) return null;
  const amount = clamp((time - track.from) / Math.max(duration, 1), 0, 1);
  return {
    x: track.previous.x + (track.target.x - track.previous.x) * amount,
    y: track.previous.y + (track.target.y - track.previous.y) * amount,
    z: track.previous.z + (track.target.z - track.previous.z) * amount,
    yaw: interpolateYaw(track.previous.yaw, track.target.yaw, amount),
    pitch: track.previous.pitch + (track.target.pitch - track.previous.pitch) * amount,
  };
}

class MultiplayerApp {
  constructor(ui, sim, renderer, controlPlane) {
    this.ui = ui;
    this.sim = sim;
    this.renderer = renderer;
    this.controlPlane = controlPlane;
    this.terrainWidth = sim.terrainWidth();
    this.cellSize = sim.cellSize();
    this.worldSize = sim.worldSize();
    this.worldCenter = this.worldSize * 0.5;
    this.templateOrigin = [this.worldCenter, PREVIEW_Y, this.worldCenter];
    this.heights = new Float32Array(sim.memory.buffer, sim.heightPtr(), this.terrainWidth ** 2);
    this.templates = {
      rival: this.copyPreviewTemplate(RIVAL_SCENE),
      firebolt: Array.from(
        { length: FIREBOLT_TEMPLATE_COUNT },
        (_, variant) => this.copyPreviewTemplate(FIREBOLT_SCENE, variant),
      ),
    };
    this.instances = new Float32Array(sim.instCapacity() * INSTANCE_STRIDE);
    this.particles = new Float32Array(sim.partCapacity() * PARTICLE_STRIDE);
    this.instanceCount = 0;
    this.particleCount = 0;
    this.playerTracks = new Map();
    this.projectileTracks = new Map();
    this.impacts = [];
    this.heals = [];
    this.seenEvents = new Set();
    this.seenEventOrder = [];
    this.keys = new Set();
    this.touchKeys = new Set();
    this.sequence = 1;
    this.yaw = 0;
    this.pitch = 0;
    this.yawInitialized = false;
    this.lastRenderAt = null;
    this.castPulse = false;
    this.mouseCastHeld = false;
    this.touchCastHeld = false;
    this.gamepadCastHeld = false;
    this.selectedSpell = MULTIPLAYER_SPELLS[0].id;
    this.castCues = [];
    this.nextCastCueId = 1;
    this.localCastReadyAt = new Map();
    this.latestInput = { x: 0, y: 0, vertical: 0 };
    this.inputHistory = [];
    this.acknowledgedInputSequence = 0;
    this.reconciliation = { x: 0, y: 0, z: 0, from: nowMs() };
    this.touchAim = null;
    this.sentClocks = new Map();
    this.ws = null;
    this.joinGeneration = 0;
    this.joinFlight = new SingleFlight();
    this.joinAbortController = null;
    const sessionStorage = safeBrowserStorage('sessionStorage');
    this.resumeTokens = new ResumeTokenStore(sessionStorage);
    this.joinNonces = new JoinNonceStore(sessionStorage);
    this.resumeOwnership = new ResumeOwnership(safeBrowserLocks());
    this.liveResumeTokens = new Map();
    this.reconnectAttempt = 0;
    this.reconnectTimer = null;
    this.inputTimer = null;
    this.inputDeadline = 0;
    this.joinedRoom = null;
    this.joinSlot = null;
    this.latestSnapshot = null;
    this.latestSnapshotSeq = null;
    this.rtt = null;
    this.protocolFailures = 0;
    this.lastError = null;
    this.interpolationMs = 1_000 / 32;
    this.snapshotJitterMs = 0;
    this.lastSnapshotAt = null;
    this.bindControls();
  }

  copyPreviewTemplate(scene, variant = 0) {
    const count = this.sim.previewScene(scene, variant);
    if (count <= 0 || count > this.sim.instCapacity() || this.sim.instStride() !== INSTANCE_STRIDE) {
      throw new Error(`Model preview scene ${scene} is unavailable.`);
    }
    // previewScene owns the shared Wasm render buffer. Copy immediately:
    // another preview call overwrites it.
    return new Float32Array(
      this.sim.memory.buffer,
      this.sim.instPtr(),
      count * INSTANCE_STRIDE,
    ).slice();
  }

  setArenaRoom(room) {
    this.sim.init(hashString(`multiplayer:${room}`), 1);
    this.heights = new Float32Array(
      this.sim.memory.buffer,
      this.sim.heightPtr(),
      this.terrainWidth ** 2,
    );
    // The extracted authoritative core currently has a flat base terrain.
    // Rendering the legacy campaign heightmap here would move targets and
    // projectiles away from their server collision coordinates.
    this.heights.fill(0);
    this.renderer.uploadHeights(this.heights, 0, this.terrainWidth - 1);
    this.sim.clearDirty?.();
  }

  heightAt(x, z) {
    return bilinearHeight(
      this.heights,
      this.terrainWidth,
      this.cellSize,
      this.worldSize,
      x,
      z,
    );
  }

  worldPosition(xcm, ycm, zcm, clearance) {
    // The legacy render world uses decimetre-scale units: the 20-unit carpet
    // is roughly two metres long. The authoritative service speaks integer cm.
    // Do not clamp: the authoritative core has no arena clamp, so doing so in
    // presentation would make the visible player diverge from the server.
    const x = this.worldCenter + xcm * WORLD_UNITS_PER_CM;
    const z = this.worldCenter + zcm * WORLD_UNITS_PER_CM;
    return { x, y: this.heightAt(x, z) + clearance + ycm * WORLD_UNITS_PER_CM, z };
  }

  setConnection(state, message) {
    document.body.dataset.connection = state;
    this.ui.connection.textContent = message;
  }

  showError(error) {
    const message = error instanceof Error ? error.message : String(error);
    this.lastError = message;
    this.ui.error.textContent = message;
  }

  clearError() {
    this.lastError = null;
    this.ui.error.textContent = '';
  }

  sessionKey(room) {
    return resumeSessionKey(this.controlPlane, room);
  }

  joinKey(room) {
    return joinAttemptKey(this.controlPlane, room);
  }

  async loadResumeToken(room) {
    const key = this.sessionKey(room);
    const live = this.liveResumeTokens.get(key);
    if (live !== undefined) return live;
    const stored = this.resumeTokens.load(key);
    if (stored === undefined) return undefined;
    if (!this.resumeOwnership.supported) {
      this.resumeTokens.clear(key);
      this.joinNonces.clear(this.joinKey(room));
      return undefined;
    }
    if (!await this.resumeOwnership.claim(key, stored)) {
      // sessionStorage can be cloned into a duplicated tab. Only the tab that
      // owns this short-lived token may resume its player.
      this.resumeTokens.clear(key);
      this.joinNonces.clear(this.joinKey(room));
      return undefined;
    }
    this.liveResumeTokens.set(key, stored);
    return stored;
  }

  async saveResumeToken(room, token) {
    const key = this.sessionKey(room);
    const owned = this.resumeOwnership.supported
      ? await this.resumeOwnership.claim(key, token)
      : false;
    this.liveResumeTokens.set(key, token);
    this.resumeTokens.save(key, token, { persist: owned });
  }

  clearResumeToken(room) {
    const key = this.sessionKey(room);
    this.liveResumeTokens.delete(key);
    this.resumeTokens.clear(key);
  }

  clearJoinAttempt(room) {
    this.joinNonces.clear(this.joinKey(room));
  }

  async prepareFreshJoin(room) {
    const joinKey = this.joinKey(room);
    const sessionKey = this.sessionKey(room);
    let nonce = this.joinNonces.load(joinKey);
    for (let attempt = 0; attempt < 2; attempt += 1) {
      if (nonce === undefined) nonce = randomJoinNonce();
      const resumeToken = await demoResumeToken(room, nonce);
      const owned = this.resumeOwnership.supported
        ? await this.resumeOwnership.claim(sessionKey, resumeToken)
        : false;
      if (!this.resumeOwnership.supported || owned) {
        this.joinNonces.save(joinKey, nonce, { persist: owned });
        this.liveResumeTokens.set(sessionKey, resumeToken);
        this.resumeTokens.save(sessionKey, resumeToken, { persist: owned });
        return { nonce, resumeToken };
      }
      // A duplicated tab inherited the first tab's in-flight attempt. It must
      // create an independent player instead of replaying that attempt.
      this.joinNonces.clear(joinKey);
      this.resumeTokens.clear(sessionKey);
      nonce = undefined;
    }
    throw new Error('This tab could not acquire an independent staging player.');
  }

  async requestJoin(room, { resumeToken, joinNonce } = {}, signal) {
    const controller = new AbortController();
    const cancel = () => controller.abort();
    if (signal?.aborted) controller.abort();
    else signal?.addEventListener('abort', cancel, { once: true });
    const timeout = setTimeout(() => controller.abort(), 12_000);
    try {
      const response = await fetch(`${this.controlPlane}/v1/demo/join`, {
        method: 'POST',
        mode: 'cors',
        credentials: 'omit',
        cache: 'no-store',
        referrerPolicy: 'no-referrer',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          room,
          ...(resumeToken ? { resumeToken } : {}),
          ...(joinNonce ? { joinNonce } : {}),
        }),
        signal: controller.signal,
      });
      let payload;
      try { payload = await response.json(); }
      catch { throw new Error(`Control plane returned HTTP ${response.status} without a valid response.`); }
      if (!response.ok) {
        throw new JoinError(
          response.status,
          typeof payload?.error?.code === 'string' ? payload.error.code : 'join_failed',
          payload?.error?.message || `Could not join the room (HTTP ${response.status}).`,
        );
      }
      return parseJoinResponse(payload);
    } catch (error) {
      if (error?.name === 'AbortError') throw new Error('The staging control plane did not respond in time.');
      throw error;
    } finally {
      clearTimeout(timeout);
      signal?.removeEventListener('abort', cancel);
    }
  }

  join(options = {}) {
    return this.joinFlight.run(async () => {
      const controller = new AbortController();
      this.joinAbortController = controller;
      try {
        return await this.performJoin(options, controller.signal);
      } finally {
        if (this.joinAbortController === controller) this.joinAbortController = null;
      }
    });
  }

  async performJoin({ reconnecting = false } = {}, signal) {
    let room;
    try {
      room = validateRoom(reconnecting ? this.joinedRoom : this.ui.room.value);
    } catch (error) {
      this.showError(error);
      return;
    }

    const generation = ++this.joinGeneration;
    clearTimeout(this.reconnectTimer);
    this.stopInput();
    if (this.ws) {
      const previous = this.ws;
      this.ws = null;
      previous.close(1000, 'rejoining');
    }
    this.clearError();
    this.ui.join.disabled = true;
    this.setConnection(reconnecting ? 'reconnecting' : 'connecting', reconnecting ? 'Reconnecting to room…' : 'Requesting a staging slot…');
    if (!reconnecting) {
      this.joinedRoom = room;
      this.setArenaRoom(room);
      this.latestSnapshot = null;
      this.latestSnapshotSeq = null;
      this.sequence = 1;
      this.inputHistory.length = 0;
      this.acknowledgedInputSequence = 0;
      this.reconciliation = { x: 0, y: 0, z: 0, from: nowMs() };
      this.playerTracks.clear();
      this.projectileTracks.clear();
      this.impacts.length = 0;
      this.heals.length = 0;
      this.castCues.length = 0;
      this.localCastReadyAt.clear();
      this.seenEvents.clear();
      this.seenEventOrder.length = 0;
      updateRoomUrl(room);
    }

    try {
      const resumeToken = await this.loadResumeToken(room);
      let joined;
      if (resumeToken !== undefined) {
        try {
          joined = await this.requestJoin(room, { resumeToken }, signal);
        } catch (error) {
          if (
            error instanceof JoinError &&
            (error.code === 'invalid_resume_token' || error.code === 'resume_expired')
          ) {
            this.clearResumeToken(room);
            if (reconnecting) {
              throw new JoinError(
                error.status,
                error.code,
                'This staging session expired. Choose Join to open a new player.',
              );
            }
          } else {
            throw error;
          }
        }
      }
      if (joined === undefined) {
        const attempt = await this.prepareFreshJoin(room);
        try {
          joined = await this.requestJoin(
            room,
            { joinNonce: attempt.nonce },
            signal,
          );
        } catch (error) {
          if (error instanceof JoinError && error.status >= 400 && error.status < 500) {
            this.clearResumeToken(room);
            this.clearJoinAttempt(room);
          }
          throw error;
        }
        if (joined.resumeToken !== attempt.resumeToken) {
          throw new Error('The staging join did not match this tab’s idempotent attempt.');
        }
        this.clearJoinAttempt(room);
      }
      if (generation !== this.joinGeneration) return;
      await this.saveResumeToken(room, joined.resumeToken);
      this.joinSlot = joined.slot;
      this.interpolationMs = 1_000 / joined.snapshotHz;
      this.openSocket(joined, generation);
    } catch (error) {
      if (generation !== this.joinGeneration) return;
      this.showError(error);
      this.setConnection('error', reconnecting ? 'Reconnect failed' : 'Could not join staging');
      this.ui.join.disabled = false;
      const resumeEnded =
        error instanceof JoinError &&
        (error.code === 'invalid_resume_token' || error.code === 'resume_expired');
      if (resumeEnded) {
        this.ui.lobby.classList.remove('joined');
        this.ui.leave.hidden = true;
      } else if (reconnecting) {
        this.scheduleReconnect();
      }
    }
  }

  openSocket(joined, generation) {
    // Snapshot sequence freshness is scoped to a socket stream. A hibernated
    // room may restart its sequence counter while preserving the match.
    this.latestSnapshotSeq = null;
    this.yawInitialized = false;
    this.inputHistory.length = 0;
    this.acknowledgedInputSequence = 0;
    this.reconciliation = { x: 0, y: 0, z: 0, from: nowMs() };
    // A resumed socket can receive the room's short reliable-event replay.
    // Keep the same-match de-duplication cursor here; fresh joins and Leave
    // clear it explicitly.
    this.rtt = null;
    this.snapshotJitterMs = 0;
    this.lastSnapshotAt = null;
    this.interpolationMs = 1_000 / joined.snapshotHz;
    this.sentClocks.clear();
    let socket;
    try {
      socket = new WebSocket(joined.webSocketUrl, [
        'aetherloom.v3',
        `aetherloom.auth.${joined.ticket}`,
      ]);
    } catch (error) {
      this.showError(error);
      this.setConnection('error', 'Could not open match connection');
      this.ui.join.disabled = false;
      this.scheduleReconnect();
      return;
    }
    socket.binaryType = 'arraybuffer';
    this.ws = socket;
    this.ui.leave.hidden = false;
    const openTimeout = setTimeout(() => {
      if (
        generation === this.joinGeneration &&
        this.ws === socket &&
        socket.readyState === WebSocket.CONNECTING
      ) {
        this.showError(new Error('The match connection did not open in time.'));
        try { socket.close(4000, 'opening timed out'); }
        catch { /* The close handler or browser network timeout will recover. */ }
      }
    }, 10_000);

    socket.addEventListener('open', () => {
      clearTimeout(openTimeout);
      if (generation !== this.joinGeneration || this.ws !== socket) {
        socket.close(1000, 'superseded');
        return;
      }
      if (socket.protocol !== 'aetherloom.v3') {
        socket.close(1002, 'subprotocol mismatch');
        return;
      }
      this.protocolFailures = 0;
      this.reconnectAttempt = 0;
      this.clearError();
      this.setConnection('connected', `Connected · room ${this.joinedRoom}`);
      this.ui.lobby.classList.add('joined');
      this.ui.leave.hidden = false;
      this.ui.join.disabled = false;
      this.startInput(joined.inputHz);
    });

    socket.addEventListener('message', async (event) => {
      if (generation !== this.joinGeneration || this.ws !== socket) return;
      try {
        const payload = event.data instanceof Blob ? await event.data.arrayBuffer() : event.data;
        this.handleSnapshot(decodeSnapshotFrame(payload));
        this.protocolFailures = 0;
      } catch (error) {
        this.protocolFailures += 1;
        this.showError(error instanceof ProtocolError
          ? `Ignored malformed snapshot: ${error.message}`
          : error);
        if (this.protocolFailures >= 3) {
          socket.close(1002, 'repeated malformed snapshots');
        }
      }
    });

    socket.addEventListener('close', (event) => {
      clearTimeout(openTimeout);
      if (generation !== this.joinGeneration || this.ws !== socket) return;
      this.ws = null;
      this.stopInput();
      if (event.code === 4003 && this.joinedRoom) {
        this.clearResumeToken(this.joinedRoom);
      }
      const terminal = socketCloseMessage(event.code, event.reason);
      if (terminal) {
        this.ui.lobby.classList.remove('joined');
        this.ui.leave.hidden = true;
        this.ui.join.disabled = false;
        this.showError(new Error(terminal));
        this.setConnection('error', 'Match connection closed');
        return;
      }
      this.setConnection('reconnecting', 'Connection lost · preparing resume');
      this.scheduleReconnect();
    });

    socket.addEventListener('error', () => {
      if (generation === this.joinGeneration && this.ws === socket) {
        this.setConnection('reconnecting', 'Match connection interrupted');
      }
    });
  }

  scheduleReconnect() {
    clearTimeout(this.reconnectTimer);
    const delays = [500, 1_000, 2_000, 4_000, 8_000];
    const delay = delays[Math.min(this.reconnectAttempt, delays.length - 1)];
    this.reconnectAttempt += 1;
    this.reconnectTimer = setTimeout(() => this.join({ reconnecting: true }), delay);
  }

  leave() {
    this.joinAbortController?.abort();
    this.joinAbortController = null;
    this.joinFlight.reset();
    this.joinGeneration += 1;
    clearTimeout(this.reconnectTimer);
    this.stopInput();
    if (this.ws) this.ws.close(1000, 'player left');
    this.ws = null;
    this.latestSnapshot = null;
    this.latestSnapshotSeq = null;
    this.inputHistory.length = 0;
    this.acknowledgedInputSequence = 0;
    this.playerTracks.clear();
    this.projectileTracks.clear();
    this.impacts.length = 0;
    this.heals.length = 0;
    this.castCues.length = 0;
    this.localCastReadyAt.clear();
    this.castPulse = false;
    this.mouseCastHeld = false;
    this.touchCastHeld = false;
    this.gamepadCastHeld = false;
    this.seenEvents.clear();
    this.seenEventOrder.length = 0;
    this.joinedRoom = null;
    this.joinSlot = null;
    this.sentClocks.clear();
    this.ui.lobby.classList.remove('joined');
    this.ui.leave.hidden = true;
    this.ui.join.disabled = false;
    this.setConnection('offline', 'Offline — choose a room');
    this.updateStats();
    if (document.pointerLockElement === this.ui.canvas) document.exitPointerLock?.();
  }

  startInput(rate) {
    this.stopInput();
    const interval = 1_000 / rate;
    this.sendInput();
    this.inputDeadline = nowMs() + interval;
    const pace = () => {
      if (this.inputTimer === null) return;
      const now = nowMs();
      if (now >= this.inputDeadline) {
        this.sendInput();
        this.inputDeadline = nextPacedDeadline(this.inputDeadline, now, interval);
      }
      this.inputTimer = setTimeout(pace, Math.max(1, this.inputDeadline - nowMs()));
    };
    this.inputTimer = setTimeout(pace, interval);
  }

  stopInput() {
    clearTimeout(this.inputTimer);
    this.inputTimer = null;
    this.inputDeadline = 0;
  }

  sampleInput() {
    let horizontal = (this.keys.has('d') || this.keys.has('arrowright') || this.touchKeys.has('d') ? 1 : 0)
      - (this.keys.has('a') || this.keys.has('arrowleft') || this.touchKeys.has('a') ? 1 : 0);
    let vertical = (this.keys.has('w') || this.keys.has('arrowup') || this.touchKeys.has('w') ? 1 : 0)
      - (this.keys.has('s') || this.keys.has('arrowdown') || this.touchKeys.has('s') ? 1 : 0);
    let altitude = (this.keys.has(' ') || this.touchKeys.has('ascend') ? 1 : 0)
      - (
        this.keys.has('shift') ||
        this.keys.has('control') ||
        this.touchKeys.has('descend')
          ? 1
          : 0
      );
    const pads = navigator.getGamepads?.() || [];
    const pad = Array.from(pads).find(Boolean);
    if (pad) {
      const dead = (value) => Math.abs(value) < GAMEPAD_LOOK_DEADZONE ? 0 : value;
      const padX = dead(pad.axes[0] || 0);
      const padY = -dead(pad.axes[1] || 0);
      if (Math.hypot(padX, padY) > Math.hypot(horizontal, vertical)) {
        horizontal = padX;
        vertical = padY;
      }
      const controllerAltitude = Number(Boolean(pad.buttons[0]?.pressed))
        - Number(Boolean(pad.buttons[1]?.pressed));
      if (controllerAltitude !== 0) altitude = controllerAltitude;
      this.gamepadCastHeld = Boolean(pad.buttons[7]?.pressed);
    } else {
      this.gamepadCastHeld = false;
    }
    const shaped = shapeFlightInput(horizontal, vertical);
    return {
      x: clamp(Math.round(shaped.strafe * 127), -127, 127),
      y: clamp(Math.round(shaped.thrust * 127), -127, 127),
      vertical: clamp(Math.round(altitude * 127), -127, 127),
    };
  }

  sendInput() {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) return;
    const localMovement = this.sampleInput();
    const movement = movementToWorld(
      localMovement.x,
      localMovement.y,
      this.yaw,
      this.pitch,
      localMovement.vertical,
    );
    this.latestInput = movement;
    const cast =
      this.castPulse ||
      this.mouseCastHeld ||
      this.touchCastHeld ||
      this.gamepadCastHeld;
    this.castPulse = false;
    const sentAt = nowMs();
    const localReadyAt = this.localCastReadyAt.get(this.selectedSpell) ?? 0;
    if (cast && sentAt >= localReadyAt) {
      const viewer = this.predictedViewer(sentAt);
      if (viewer && this.selectedSpell === 0) {
        this.castCues.push({
          id: this.nextCastCueId,
          sequence: this.sequence,
          at: firstPersonMuzzle(viewer),
          bornAt: sentAt,
          spell: this.selectedSpell,
        });
        this.nextCastCueId = this.nextCastCueId === 0xffff_ffff
          ? 1
          : this.nextCastCueId + 1;
        if (this.castCues.length > 8) this.castCues.splice(0, this.castCues.length - 8);
      }
      const spell = MULTIPLAYER_SPELLS.find((candidate) =>
        candidate.id === this.selectedSpell);
      this.localCastReadyAt.set(
        this.selectedSpell,
        sentAt + (spell?.cooldownMs ?? 250),
      );
    }
    const sentClock = Math.floor(sentAt) & 0xffff;
    const frame = encodeInputFrame({
      sequence: this.sequence,
      moveX: movement.x,
      moveY: movement.y,
      moveVertical: movement.vertical,
      yaw: this.yaw,
      pitch: clamp(Math.round(this.pitch / MAX_PITCH * 127), -127, 127),
      cast,
      spell: this.selectedSpell,
      clientClockMs: sentClock,
      snapshotAck: this.latestSnapshotSeq ?? 0,
    });
    this.sentClocks.set(sentClock, sentAt);
    this.inputHistory.push({
      sequence: this.sequence,
      sentAt,
      x: movement.x,
      y: movement.vertical,
      z: movement.y,
    });
    if (this.inputHistory.length > MAX_PREDICTION_HISTORY) {
      this.inputHistory.splice(0, this.inputHistory.length - MAX_PREDICTION_HISTORY);
    }
    for (const [value, recordedAt] of this.sentClocks) {
      if (sentAt - recordedAt > 10_000) this.sentClocks.delete(value);
    }
    this.sequence = this.sequence === 0xffff_ffff ? 1 : this.sequence + 1;
    try { this.ws.send(frame); }
    catch { this.ws.close(1011, 'input send failed'); }
  }

  updateTrack(map, key, target, receivedAt) {
    const existing = map.get(key);
    const current = sampleTrack(existing, receivedAt, this.interpolationMs) || target;
    const next = {
      ...existing,
      previous: current,
      target,
      from: receivedAt,
      seenAt: receivedAt,
    };
    map.set(key, next);
    return next;
  }

  handleSnapshot(snapshot) {
    if (snapshot.viewerSlot !== this.joinSlot) {
      throw new ProtocolError('viewer_mismatch', 'Snapshot viewer does not match the authenticated slot.');
    }
    if (!isNewerSequence(snapshot.snapshotSeq, this.latestSnapshotSeq)) return;
    const receivedAt = nowMs();
    const previousViewer = this.predictedViewer(receivedAt);
    if (this.lastSnapshotAt !== null) {
      const observedInterval = receivedAt - this.lastSnapshotAt;
      const deviation = Math.abs(observedInterval - 1_000 / 32);
      this.snapshotJitterMs = this.snapshotJitterMs * 0.82 + deviation * 0.18;
      this.interpolationMs = clamp(
        1_000 / 32 + this.snapshotJitterMs * 1.5,
        1_000 / 32,
        1_000 * 6 / 128,
      );
    }
    this.lastSnapshotAt = receivedAt;
    this.latestSnapshotSeq = snapshot.snapshotSeq;
    this.latestSnapshot = snapshot;
    this.acknowledgedInputSequence = snapshot.acknowledgedInputSequence;
    if (this.acknowledgedInputSequence !== 0) {
      this.inputHistory = this.inputHistory.filter((entry) =>
        isNewerSequence(entry.sequence, this.acknowledgedInputSequence));
    }
    const echoedAt = snapshot.echoClock === 0
      ? undefined
      : this.sentClocks.get(snapshot.echoClock);
    if (echoedAt !== undefined) {
      this.sentClocks.delete(snapshot.echoClock);
      const roundTrip = receivedAt - echoedAt;
      if (roundTrip >= 0 && roundTrip <= 10_000) {
        this.rtt = this.rtt === null
          ? roundTrip
          : this.rtt * 0.76 + roundTrip * 0.24;
      }
    }

    for (const player of snapshot.players) {
      const target = this.worldPosition(player.xcm, player.ycm, player.zcm, CARPET_CLEARANCE);
      target.yaw = player.yaw;
      target.pitch = player.pitch;
      target.altitudeCm = player.ycm;
      this.updateTrack(this.playerTracks, player.slot, target, receivedAt);
      if (player.slot === snapshot.viewerSlot && !this.yawInitialized) {
        this.yaw = player.yaw;
        this.pitch = player.pitch;
        this.yawInitialized = true;
      }
    }
    const viewerPlayer = snapshot.players.find(
      (player) => player.slot === snapshot.viewerSlot,
    );
    if (viewerPlayer?.spellCooldownTicks?.length === 13) {
      for (const spell of MULTIPLAYER_SPELLS) {
        this.localCastReadyAt.set(
          spell.id,
          authoritativeCooldownReadyAt(
            receivedAt,
            viewerPlayer.spellCooldownTicks[spell.id],
            this.rtt ?? 0,
          ),
        );
      }
    }
    const liveSlots = new Set(snapshot.players.map((player) => player.slot));
    for (const slot of this.playerTracks.keys()) if (!liveSlots.has(slot)) this.playerTracks.delete(slot);

    for (const projectile of snapshot.projectiles) {
      const target = this.worldPosition(
        projectile.xcm,
        projectile.ycm,
        projectile.zcm,
        CARPET_CLEARANCE,
      );
      target.yaw = projectile.yaw;
      target.pitch = projectile.pitch;
      const track = this.updateTrack(
        this.projectileTracks,
        projectile.entity,
        target,
        receivedAt,
      );
      if (!Number.isFinite(track.firstSeenAt)) {
        track.firstSeenAt = receivedAt;
        track.firstSeenElapsedTicks =
          PROJECTILE_LIFETIME_TICKS - projectile.ttl;
        if (projectile.owner === snapshot.viewerSlot) {
          this.consumeCastCue(0);
        }
      }
    }
    const liveProjectiles = new Set(snapshot.projectiles.map((projectile) => projectile.entity));
    for (const entity of this.projectileTracks.keys()) {
      if (!liveProjectiles.has(entity)) this.projectileTracks.delete(entity);
    }

    for (const event of snapshot.events) {
      if (this.seenEvents.has(event.eventId)) continue;
      this.seenEvents.add(event.eventId);
      this.seenEventOrder.push(event.eventId);
      if (this.seenEventOrder.length > MAX_SEEN_EVENTS) {
        this.seenEvents.delete(this.seenEventOrder.shift());
      }
      if (event.type === PROJECTILE_IMPACT_EVENT_TYPE && event.spell === 0) {
        this.spawnImpact(event, receivedAt);
      }
      if (
        event.type === CAST_EVENT_TYPE &&
        event.spell === 0 &&
        event.actor === snapshot.viewerSlot
      ) {
        this.consumeCastCue(0);
      }
      if (event.type === CAST_EVENT_TYPE && event.spell === 7) {
        this.spawnHeal(event, receivedAt);
      }
    }
    if (
      viewerPlayer?.spellCooldownTicks?.[0] === 0 &&
      this.acknowledgedInputSequence !== 0
    ) {
      this.castCues = this.castCues.filter((cue) =>
        cue.spell !== 0 ||
        !sequenceAcknowledges(this.acknowledgedInputSequence, cue.sequence));
    }

    const reconciledViewer = this.rawPredictedViewer(receivedAt);
    if (previousViewer && reconciledViewer) {
      this.reconciliation = {
        x: clamp(
          previousViewer.x - reconciledViewer.x,
          -MAX_RECONCILIATION_OFFSET,
          MAX_RECONCILIATION_OFFSET,
        ),
        y: clamp(
          previousViewer.y - reconciledViewer.y,
          -MAX_RECONCILIATION_OFFSET,
          MAX_RECONCILIATION_OFFSET,
        ),
        z: clamp(
          previousViewer.z - reconciledViewer.z,
          -MAX_RECONCILIATION_OFFSET,
          MAX_RECONCILIATION_OFFSET,
        ),
        from: receivedAt,
      };
    }

    this.updateStats();
  }

  consumeCastCue(spell) {
    const index = this.castCues.findIndex((cue) => cue.spell === spell);
    if (index >= 0) this.castCues.splice(index, 1);
  }

  spawnImpact(event, time) {
    const at = this.worldPosition(
      event.xcm,
      event.ycm,
      event.zcm,
      PROJECTILE_CLEARANCE,
    );
    this.impacts.push({ eventId: event.eventId, at, bornAt: time });
    if (this.impacts.length > MAX_IMPACTS) this.impacts.splice(0, this.impacts.length - MAX_IMPACTS);
  }

  spawnHeal(event, time) {
    const at = this.worldPosition(event.xcm, event.ycm, event.zcm, CARPET_CLEARANCE);
    this.heals.push({ eventId: event.eventId, at, bornAt: time });
    if (this.heals.length > MAX_HEALS) this.heals.splice(0, this.heals.length - MAX_HEALS);
  }

  rawPredictedViewer(time) {
    if (!this.latestSnapshot) return null;
    const slot = this.latestSnapshot.viewerSlot;
    const track = this.playerTracks.get(slot);
    if (!track) return null;
    const displacement = pendingInputDisplacement(this.inputHistory, time);
    const predicted = predictViewerPosition(
      track.target,
      displacement,
      (x, z) => this.heightAt(x, z),
    );
    predicted.yaw = this.yaw;
    predicted.pitch = this.pitch;
    return predicted;
  }

  reconciliationAt(time) {
    const amount = Math.exp(-(time - this.reconciliation.from) / RECONCILIATION_DECAY_MS);
    return {
      x: this.reconciliation.x * amount,
      y: this.reconciliation.y * amount,
      z: this.reconciliation.z * amount,
    };
  }

  predictedViewer(time) {
    const predicted = this.rawPredictedViewer(time);
    if (!predicted) return null;
    const correction = this.reconciliationAt(time);
    predicted.x += correction.x;
    predicted.y += correction.y;
    predicted.z += correction.z;
    return predicted;
  }

  composeInstances(time) {
    let count = 0;
    if (this.latestSnapshot) {
      for (const player of this.latestSnapshot.players) {
        // First-person presentation never submits the local carpet/rider to the
        // world pass. Remote players remain ordinary authoritative instances.
        if (player.slot === this.latestSnapshot.viewerSlot) continue;
        const position = sampleTrack(this.playerTracks.get(player.slot), time, this.interpolationMs);
        if (!position) continue;
        count = appendTemplate(
          this.instances,
          count,
          this.templates.rival,
          this.templateOrigin,
          [position.x, position.y, position.z],
          wireYawToRenderRadians(position.yaw),
          wirePitchToRadians(position.pitch),
        );
      }
      for (const projectile of this.latestSnapshot.projectiles) {
        const track = this.projectileTracks.get(projectile.entity);
        const tracked = sampleTrack(
          track,
          time,
          this.interpolationMs,
        );
        if (!tracked) continue;
        const elapsedTicks =
          (track?.firstSeenElapsedTicks ?? PROJECTILE_LIFETIME_TICKS - projectile.ttl) +
          Math.max(0, time - (track?.firstSeenAt ?? time)) * 128 / 1_000;
        const position = viewerProjectilePresentation(
          tracked,
          projectile,
          this.latestSnapshot.viewerSlot,
          elapsedTicks,
        );
        const fireboltTemplate =
          this.templates.firebolt[
            (Math.floor(time / FIREBOLT_TEMPLATE_FRAME_MS) + projectile.entity) %
              this.templates.firebolt.length
          ];
        count = appendTemplate(
          this.instances,
          count,
          fireboltTemplate,
          this.templateOrigin,
          [position.x, position.y, position.z],
          wireYawToRenderRadians(position.yaw),
          wirePitchToRadians(position.pitch),
        );
      }
    }
    this.instanceCount = count;
  }

  composeParticles(time) {
    let count = 0;
    const capacity = this.particles.length / PARTICLE_STRIDE;
    this.castCues = this.castCues.filter((cue) => time - cue.bornAt < 180);
    for (const cue of this.castCues) {
      const age = Math.max(0, (time - cue.bornAt) / 1_000);
      const remaining = clamp(1 - age / 0.18, 0, 1);
      const yaw = wireYawToCoreRadians(cue.at.yaw);
      const pitch = wirePitchToRadians(cue.at.pitch);
      const level = Math.cos(pitch);
      const forward = [
        Math.cos(yaw) * level,
        Math.sin(pitch),
        Math.sin(yaw) * level,
      ];
      const travelled = age * FIREBOLT_PRESENTATION_SPEED;
      for (let index = 0; index < 11 && count < capacity; index += 1) {
        const seed = cue.id ^ Math.imul(index + 1, 0x9e37_79b9);
        const phase = hashUnit(seed) * Math.PI * 2;
        const spread = hashUnit(seed ^ 0xa511_e9b3);
        const trail = Math.min(travelled, index * 0.42);
        const radius = spread * (0.22 + age * 2.4);
        const base = count * PARTICLE_STRIDE;
        this.particles[base] =
          cue.at.x +
          forward[0] * (travelled - trail) +
          Math.cos(phase) * radius;
        this.particles[base + 1] =
          cue.at.y +
          forward[1] * (travelled - trail) +
          Math.sin(phase * 1.7) * radius;
        this.particles[base + 2] =
          cue.at.z +
          forward[2] * (travelled - trail) +
          Math.sin(phase) * radius;
        this.particles[base + 3] = (index < 3 ? 2.8 : 1.2 + spread) * (0.45 + remaining);
        this.particles[base + 4] = 1.65;
        this.particles[base + 5] = index < 3 ? 0.92 : 0.34 + spread * 0.28;
        this.particles[base + 6] = index < 3 ? 0.38 : 0.04;
        this.particles[base + 7] = remaining * remaining;
        count += 1;
      }
    }
    this.impacts = this.impacts.filter((impact) => time - impact.bornAt < 900);
    for (const impact of this.impacts) {
      const age = (time - impact.bornAt) / 1_000;
      const life = 0.82;
      const remaining = clamp(1 - age / life, 0, 1);
      for (let index = 0; index < 26 && count < capacity; index += 1) {
        const seed = impact.eventId ^ Math.imul(index + 1, 0x9e37_79b9);
        const angle = hashUnit(seed) * Math.PI * 2;
        const spread = hashUnit(seed ^ 0xa511_e9b3);
        const lift = hashUnit(seed ^ 0x63d8_3595);
        const base = count * PARTICLE_STRIDE;
        let radius;
        let y;
        let size;
        let red;
        let green;
        let blue;
        let alpha;
        if (index < 10) {
          radius = (34 + spread * 46) * age;
          y = impact.at.y + (22 + lift * 54) * age - 38 * age * age;
          size = (1.1 + spread * 1.8) * (0.25 + remaining);
          red = 1.75; green = 0.66 + lift * 0.34; blue = 0.08; alpha = remaining * remaining;
        } else if (index < 19) {
          radius = (8 + spread * 20) * age;
          y = impact.at.y + 2 + (13 + lift * 26) * age;
          size = (3.6 + spread * 5.2) * (0.4 + remaining * 0.75);
          red = 1.28; green = 0.22 + lift * 0.28; blue = 0.025; alpha = remaining * 0.82;
        } else {
          radius = (5 + spread * 14) * age;
          y = impact.at.y + 8 + (22 + lift * 18) * age;
          size = (5.2 + spread * 7.4) * (0.45 + age * 1.4);
          red = 0.31; green = 0.24; blue = 0.24; alpha = remaining * remaining * 0.34;
        }
        this.particles[base] = impact.at.x + Math.cos(angle) * radius;
        this.particles[base + 1] = y;
        this.particles[base + 2] = impact.at.z + Math.sin(angle) * radius;
        this.particles[base + 3] = Math.max(size, 0.05);
        this.particles[base + 4] = red;
        this.particles[base + 5] = green;
        this.particles[base + 6] = blue;
        this.particles[base + 7] = alpha;
        count += 1;
      }
    }
    this.heals = this.heals.filter((heal) => time - heal.bornAt < 1_050);
    for (const heal of this.heals) {
      const age = Math.max(0, (time - heal.bornAt) / 1_000);
      const remaining = clamp(1 - age / 1.05, 0, 1);
      for (let index = 0; index < 30 && count < capacity; index += 1) {
        const seed = heal.eventId ^ Math.imul(index + 1, 0x7f4a_7c15);
        const angle = hashUnit(seed) * Math.PI * 2;
        const spread = hashUnit(seed ^ 0x68bc_21eb);
        const phase = hashUnit(seed ^ 0x02e5_be93);
        const radius =
          2 +
          spread * 8 +
          Math.sin(age * 7 + phase * Math.PI * 2) * 1.4;
        const base = count * PARTICLE_STRIDE;
        this.particles[base] = heal.at.x + Math.cos(angle) * radius;
        this.particles[base + 1] =
          heal.at.y - 3 + phase * 9 + age * (9 + spread * 16);
        this.particles[base + 2] = heal.at.z + Math.sin(angle) * radius;
        this.particles[base + 3] =
          (1.2 + spread * 2.1) * (0.55 + remaining);
        this.particles[base + 4] = 0.24 + spread * 0.18;
        this.particles[base + 5] = 1.25;
        this.particles[base + 6] = 0.43 + phase * 0.27;
        this.particles[base + 7] = remaining * remaining;
        count += 1;
      }
    }
    this.particleCount = count;
  }

  camera(time) {
    const viewer = this.predictedViewer(time);
    if (!viewer) {
      return {
        ex: this.worldCenter - 95,
        ey: this.heightAt(this.worldCenter, this.worldCenter) + 120,
        ez: this.worldCenter - 95,
        cx: this.worldCenter,
        cy: this.heightAt(this.worldCenter, this.worldCenter) + 20,
        cz: this.worldCenter,
        ux: 0,
        uy: 1,
        uz: 0,
      };
    }
    return firstPersonCamera(viewer);
  }

  render(time) {
    const elapsedMs = this.lastRenderAt === null ? 0 : Math.max(0, time - this.lastRenderAt);
    this.lastRenderAt = time;
    if (this.ws?.readyState === WebSocket.OPEN) {
      const pads = navigator.getGamepads?.() || [];
      const pad = Array.from(pads).find(Boolean);
      if (pad) {
        this.yaw = yawAfterLookDelta(
          this.yaw,
          gamepadLookDelta(
            pad.axes[2] || 0,
            elapsedMs,
            GAMEPAD_YAW_UNITS_PER_SECOND,
          ),
        );
        this.pitch = clamp(
          this.pitch - Math.round(gamepadLookDelta(
            pad.axes[3] || 0,
            elapsedMs,
            GAMEPAD_PITCH_UNITS_PER_SECOND,
          )),
          -FIRST_PERSON_MAX_PITCH,
          FIRST_PERSON_MAX_PITCH,
        );
      }
    }
    this.composeInstances(time);
    this.composeParticles(time);
    this.renderer.frame(
      this.camera(time),
      this.particles,
      this.particleCount,
      this.instances,
      this.instanceCount,
      {
        fov: FIRST_PERSON_FOV,
        time: time / 1_000,
        sun: [0.53, 0.72, 0.38],
        sunI: 1,
        fog: [0.70, 0.82, 0.95],
        fogD: 0.00072,
        skipLo: -1,
        skipHi: -1,
        exposure: 1.12,
        bloomStrength: 0.48,
        waterAlpha: 1,
        hurt: 0,
        castles: [],
      },
    );
  }

  updateStats() {
    const snapshot = this.latestSnapshot;
    this.ui.players.textContent = snapshot ? String(snapshot.playerCount) : '—';
    this.ui.tick.textContent = snapshot ? snapshot.tick.toLocaleString('en-US') : '—';
    this.ui.rtt.textContent = this.rtt === null ? '—' : `${Math.round(this.rtt)} ms`;
    const player = snapshot?.players.find((entry) => entry.slot === snapshot.viewerSlot);
    this.ui.health.textContent = player && player.maxHealth
      ? `${player.health}/${player.maxHealth}`
      : '—';
  }

  selectSpell(id) {
    const spell = MULTIPLAYER_SPELLS.find((candidate) => candidate.id === id);
    if (!spell) throw new Error('That spell is not authoritative in the staging duel yet.');
    this.selectedSpell = spell.id;
    for (const button of document.querySelectorAll('.spell[data-spell]')) {
      const selected = Number(button.dataset.spell) === spell.id;
      button.classList.toggle('selected', selected);
      button.setAttribute('aria-pressed', String(selected));
    }
    this.ui.touchFire.textContent = spell.name;
  }

  cycleSpell(direction) {
    const index = MULTIPLAYER_SPELLS.findIndex((spell) => spell.id === this.selectedSpell);
    const next = (index + direction + MULTIPLAYER_SPELLS.length) % MULTIPLAYER_SPELLS.length;
    this.selectSpell(MULTIPLAYER_SPELLS[next].id);
  }

  bindControls() {
    const movementKeys = new Set([
      'w', 'a', 's', 'd',
      'arrowup', 'arrowdown', 'arrowleft', 'arrowright',
      ' ', 'shift', 'control',
    ]);
    addEventListener('keydown', (event) => {
      if (event.target instanceof HTMLInputElement) return;
      const key = event.key.toLowerCase();
      if (movementKeys.has(key)) {
        this.keys.add(key);
        event.preventDefault();
      }
      if (key === 'f' && !event.repeat) {
        this.castPulse = true;
        event.preventDefault();
      }
      const selected = MULTIPLAYER_SPELLS.find((spell) => spell.key === event.key);
      if (selected && !event.repeat) {
        this.selectSpell(selected.id);
        event.preventDefault();
      }
      if (key === 'q' && !event.repeat) {
        this.cycleSpell(-1);
        event.preventDefault();
      }
      if (key === 'e' && !event.repeat) {
        this.cycleSpell(1);
        event.preventDefault();
      }
    });
    addEventListener('keyup', (event) => this.keys.delete(event.key.toLowerCase()));
    const clearHeldControls = () => {
      this.keys.clear();
      this.touchKeys.clear();
      this.castPulse = false;
      this.mouseCastHeld = false;
      this.touchCastHeld = false;
      this.gamepadCastHeld = false;
      this.touchAim = null;
      for (const button of document.querySelectorAll('.touch-controls .active')) {
        button.classList.remove('active');
      }
    };
    addEventListener('blur', clearHeldControls);
    document.addEventListener('visibilitychange', () => {
      if (document.hidden) clearHeldControls();
    });
    document.addEventListener('mousemove', (event) => {
      if (document.pointerLockElement === this.ui.canvas) {
        this.yaw = yawAfterLookDelta(
          this.yaw,
          event.movementX * MOUSE_LOOK_UNITS_PER_PIXEL,
        );
        this.pitch = clamp(
          this.pitch - Math.round(event.movementY * MOUSE_LOOK_UNITS_PER_PIXEL),
          -FIRST_PERSON_MAX_PITCH,
          FIRST_PERSON_MAX_PITCH,
        );
      }
    });
    this.ui.canvas.addEventListener('mousedown', (event) => {
      if (event.button !== 0 || document.body.dataset.connection !== 'connected') return;
      if (document.pointerLockElement !== this.ui.canvas) {
        try {
          const pending = this.ui.canvas.requestPointerLock?.();
          pending?.catch?.(() => {});
        } catch {
          // Older implementations return void or throw synchronously.
        }
        return;
      }
      this.mouseCastHeld = true;
    });
    addEventListener('mouseup', (event) => {
      if (event.button === 0) this.mouseCastHeld = false;
    });
    document.addEventListener('pointerlockchange', () => {
      if (document.pointerLockElement !== this.ui.canvas) this.mouseCastHeld = false;
    });
    this.ui.canvas.addEventListener('pointerdown', (event) => {
      if (event.pointerType === 'mouse' || document.body.dataset.connection !== 'connected') return;
      event.preventDefault();
      this.touchAim = { id: event.pointerId, x: event.clientX, y: event.clientY };
      this.ui.canvas.setPointerCapture?.(event.pointerId);
    });
    this.ui.canvas.addEventListener('pointermove', (event) => {
      if (!this.touchAim || this.touchAim.id !== event.pointerId) return;
      event.preventDefault();
      const delta = event.clientX - this.touchAim.x;
      const deltaY = event.clientY - this.touchAim.y;
      this.touchAim.x = event.clientX;
      this.touchAim.y = event.clientY;
      this.yaw = yawAfterLookDelta(this.yaw, delta * TOUCH_LOOK_UNITS_PER_PIXEL);
      this.pitch = clamp(
        this.pitch - Math.round(deltaY * TOUCH_LOOK_UNITS_PER_PIXEL),
        -FIRST_PERSON_MAX_PITCH,
        FIRST_PERSON_MAX_PITCH,
      );
    });
    const finishTouchAim = (event) => {
      if (this.touchAim?.id === event.pointerId) this.touchAim = null;
    };
    this.ui.canvas.addEventListener('pointerup', finishTouchAim);
    this.ui.canvas.addEventListener('pointercancel', finishTouchAim);
    this.ui.canvas.addEventListener('lostpointercapture', finishTouchAim);

    for (const button of document.querySelectorAll('.touch-controls button[data-key]')) {
      const key = button.dataset.key;
      const press = (event) => {
        event.preventDefault();
        this.touchKeys.add(key);
        button.classList.add('active');
        button.setPointerCapture?.(event.pointerId);
      };
      const release = (event) => {
        event.preventDefault();
        this.touchKeys.delete(key);
        button.classList.remove('active');
      };
      button.addEventListener('pointerdown', press);
      button.addEventListener('pointerup', release);
      button.addEventListener('pointercancel', release);
      button.addEventListener('lostpointercapture', release);
    }
    const castDown = (event) => {
      event.preventDefault();
      this.touchCastHeld = true;
      this.ui.touchFire.classList.add('active');
      this.ui.touchFire.setPointerCapture?.(event.pointerId);
    };
    const castUp = (event) => {
      event.preventDefault();
      this.touchCastHeld = false;
      this.ui.touchFire.classList.remove('active');
    };
    this.ui.touchFire.addEventListener('pointerdown', castDown);
    this.ui.touchFire.addEventListener('pointerup', castUp);
    this.ui.touchFire.addEventListener('pointercancel', castUp);
    this.ui.touchFire.addEventListener('lostpointercapture', castUp);
    for (const button of document.querySelectorAll('.spell[data-spell]')) {
      button.addEventListener('click', () => this.selectSpell(Number(button.dataset.spell)));
    }
    this.selectSpell(this.selectedSpell);
  }

  publicState() {
    return {
      ready: true,
      connection: document.body.dataset.connection,
      controlPlane: this.controlPlane,
      room: this.joinedRoom || this.ui.room.value,
      viewerSlot: this.latestSnapshot?.viewerSlot ?? null,
      playerCount: this.latestSnapshot?.playerCount ?? 0,
      tick: this.latestSnapshot?.tick ?? null,
      snapshotSeq: this.latestSnapshotSeq,
      acknowledgedInputSequence: this.acknowledgedInputSequence,
      rttMs: this.rtt === null ? null : Math.round(this.rtt),
      yaw: this.yaw,
      pitch: this.pitch,
      selectedSpell: this.selectedSpell,
      viewer: this.predictedViewer(nowMs()),
      malformedFrames: this.protocolFailures,
      instanceCount: this.instanceCount,
      particleCount: this.particleCount,
      error: this.lastError,
    };
  }
}

function updateRoomUrl(room) {
  const url = new URL(location.href);
  url.searchParams.set('room', room);
  history.replaceState(null, '', url);
}

function safeBrowserStorage(name) {
  try {
    const storage = globalThis[name];
    return storage &&
      typeof storage.getItem === 'function' &&
      typeof storage.setItem === 'function' &&
      typeof storage.removeItem === 'function'
      ? storage
      : null;
  } catch {
    return null;
  }
}

function safeBrowserLocks() {
  try {
    return typeof globalThis.navigator?.locks?.request === 'function'
      ? globalThis.navigator.locks
      : null;
  } catch {
    return null;
  }
}

let app = null;
let bootPromise = null;

const testApi = {
  encodeInputFrame,
  decodeSnapshotFrame,
  composeTemplate,
  bilinearHeight,
  firstPersonCamera,
  isNewerSequence,
  movementToWorld,
  nextPacedDeadline,
  predictedBrowserAxisDisplacement,
  predictViewerPosition,
  socketCloseMessage,
  wrappedClockDelta,
  normalizeControlPlane,
  validateRoom,
  wirePitchToRadians,
  yawAfterLookDelta,
  gamepadLookDelta,
  snapshot: () => app?.publicState() || {
    ready: false,
    connection: 'booting',
    error: null,
  },
  whenReady: () => bootPromise,
  join: () => app?.join(),
  leave: () => app?.leave(),
};

async function boot() {
  const element = (id) => document.getElementById(id);
  const ui = {
    canvas: element('view'),
    connection: element('connection'),
    lobby: element('lobby'),
    players: element('players'),
    health: element('health'),
    tick: element('tick'),
    rtt: element('rtt'),
    room: element('room'),
    share: element('share'),
    join: element('join'),
    leave: element('leave'),
    error: element('error'),
    touchFire: element('touchFire'),
    boot: element('boot'),
  };
  ui.join.disabled = true;
  const params = new URLSearchParams(location.search);
  const initialRoom = (() => {
    try { return validateRoom(params.get('room') || randomRoom()); }
    catch { return randomRoom(); }
  })();
  ui.room.value = initialRoom;
  updateRoomUrl(initialRoom);

  let controlPlane;
  try {
    controlPlane = normalizeControlPlane(params.get('controlPlane') || DEFAULT_CONTROL_PLANE);
  } catch (error) {
    ui.error.textContent = error.message;
    document.body.dataset.connection = 'error';
    ui.connection.textContent = 'Invalid control-plane address';
    throw error;
  }
  if (params.has('controlPlane')) {
    const safePageUrl = new URL(location.href);
    safePageUrl.searchParams.set('controlPlane', controlPlane);
    history.replaceState(null, '', safePageUrl);
  }

  const response = await fetch('./sim.wasm', { cache: 'no-cache' });
  if (!response.ok) throw new Error(`Could not load sim.wasm (HTTP ${response.status}).`);
  const { instance } = await WebAssembly.instantiate(await response.arrayBuffer(), {});
  const sim = instance.exports;
  const required = [
    'init', 'previewScene', 'instPtr', 'instCapacity', 'instStride',
    'partCapacity', 'partStride', 'heightPtr', 'terrainWidth', 'cellSize',
    'worldSize', 'camera', 'buildMeshes',
  ];
  const missing = required.filter((name) => typeof sim[name] !== 'function');
  if (missing.length) throw new Error(`sim.wasm is missing multiplayer presentation exports: ${missing.join(', ')}`);
  if (sim.instStride() !== INSTANCE_STRIDE || sim.partStride() !== PARTICLE_STRIDE) {
    throw new Error(`Presentation ABI mismatch: expected ${INSTANCE_STRIDE}/${PARTICLE_STRIDE}-float records.`);
  }

  const renderer = new Renderer(ui.canvas, sim);
  await renderer.init(sim.terrainWidth(), sim.cellSize(), 0);
  app = new MultiplayerApp(ui, sim, renderer, controlPlane);
  app.setArenaRoom(initialRoom);
  ui.join.disabled = false;
  ui.boot.classList.add('done');
  setTimeout(() => ui.boot.remove(), 400);

  ui.room.addEventListener('input', () => {
    ui.room.value = ui.room.value.toLowerCase().replace(/[^a-z0-9-]/gu, '');
  });
  ui.room.addEventListener('keydown', (event) => {
    if (event.key === 'Enter' && !ui.join.disabled) app.join();
  });
  ui.join.addEventListener('click', () => app.join());
  ui.leave.addEventListener('click', () => app.leave());
  ui.share.addEventListener('click', async () => {
    try {
      const room = validateRoom(ui.room.value);
      updateRoomUrl(room);
      await navigator.clipboard.writeText(location.href);
      const previous = ui.share.textContent;
      ui.share.textContent = 'Copied';
      setTimeout(() => { ui.share.textContent = previous; }, 1_200);
    } catch (error) {
      app.showError(error);
    }
  });

  const resize = () => renderer.resize();
  addEventListener('resize', resize);
  if (window.ResizeObserver) new ResizeObserver(resize).observe(ui.canvas);
  const loop = (time) => {
    app.render(time);
    requestAnimationFrame(loop);
  };
  requestAnimationFrame(loop);
  return app.publicState();
}

if (typeof window !== 'undefined') {
  window.__multiplayerTest = testApi;
}

if (typeof document !== 'undefined') {
  bootPromise = boot().catch((error) => {
    const message = error instanceof Error ? error.message : String(error);
    const bootElement = document.getElementById('boot');
    if (bootElement) bootElement.textContent = `Systems test unavailable — ${message}`;
    const errorElement = document.getElementById('error');
    if (errorElement) errorElement.textContent = message;
    const connectionElement = document.getElementById('connection');
    if (connectionElement) connectionElement.textContent = 'Systems test failed to start';
    document.body.dataset.connection = 'error';
    console.error(error);
    throw error;
  });
}
