// AETHERLOOM — standalone browser multiplayer systems test.
//
// Authoritative gameplay stays on the staging match service. This client owns
// presentation, local input sampling, modest prediction, and remote
// interpolation only. The existing offline game remains entirely separate.
import { Renderer } from './engine.js';

export const DEFAULT_CONTROL_PLANE = 'https://aetherloom-control-plane-staging.calum-maciver.workers.dev';
export const INPUT_FRAME_BYTES = 20;
export const SNAPSHOT_HEADER_BYTES = 24;
export const PLAYER_RECORD_BYTES = 24;
export const PROJECTILE_RECORD_BYTES = 16;
export const EVENT_RECORD_BYTES = 16;
export const DAMAGE_EVENT_TYPE = 4;
export const MAX_SNAPSHOT_FRAME_BYTES = 1_200;

const PROTOCOL_VERSION = 1;
const INPUT_MESSAGE_TYPE = 1;
const SNAPSHOT_MESSAGE_TYPE = 2;
const INSTANCE_STRIDE = 14;
const PARTICLE_STRIDE = 8;
const PREVIEW_Y = 512;
const PLAYER_SCENE = 20;
const RIVAL_SCENE = 21;
const FIREBOLT_SCENE = 25;
const MAX_PLAYERS = 128;
const MAX_PROJECTILES = 255;
const MAX_EVENTS = 255;
const WORLD_UNITS_PER_CM = 0.1;
const CARPET_CLEARANCE = 34;
const PROJECTILE_CLEARANCE = 39;
const PREDICTION_SPEED = 102.4;
const MAX_PREDICTION_SECONDS = 0.12;
const MAX_IMPACTS = 32;
const MAX_SEEN_EVENTS = 512;

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
  yaw = 0,
  cast = false,
  clientClockMs = 0,
  snapshotAck = 0,
}) {
  integerIn(sequence, 1, 0xffff_ffff, 'sequence');
  integerIn(moveX, -127, 127, 'moveX');
  integerIn(moveY, -127, 127, 'moveY');
  integerIn(yaw, 0, 0xffff, 'yaw');
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
  view.setUint16(10, yaw, true);
  view.setUint8(12, cast ? 1 : 0);
  view.setUint8(13, 0);
  view.setUint16(14, clientClockMs, true);
  view.setUint32(16, snapshotAck, true);
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
  protocolAssert(playerCount <= MAX_PLAYERS, 'too_many_players', 'Snapshot player count exceeds the client limit.');
  protocolAssert(projectileCount <= MAX_PROJECTILES, 'too_many_projectiles', 'Snapshot projectile count exceeds the client limit.');
  protocolAssert(eventCount <= MAX_EVENTS, 'too_many_events', 'Snapshot event count exceeds the client limit.');
  const expectedLength = SNAPSHOT_HEADER_BYTES
    + playerCount * PLAYER_RECORD_BYTES
    + projectileCount * PROJECTILE_RECORD_BYTES
    + eventCount * EVENT_RECORD_BYTES;
  protocolAssert(expectedLength === bytes.byteLength, 'record_length_mismatch', 'Snapshot record counts do not match its length.');
  protocolAssert(view.getUint16(20, true) === 0, 'reserved_bits', 'Snapshot reserved field is not zero.');
  // The documented fields occupy 22 bytes; the fixed 24-byte header keeps two
  // extension bytes. They must remain zero until a later protocol version.
  protocolAssert(view.getUint16(22, true) === 0, 'reserved_bits', 'Snapshot extension field is not zero.');

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
    const entity = view.getUint32(offset + 20, true);
    rejectDuplicate(slots, slot, 'player slot');
    rejectDuplicate(playerEntities, entity, 'player entity');
    const health = view.getUint16(offset + 14, true);
    const maxHealth = view.getUint16(offset + 16, true);
    protocolAssert(maxHealth === 0 || health <= maxHealth, 'invalid_health', 'Player health exceeds maximum health.');
    players.push({
      slot,
      team: view.getUint8(offset + 1),
      status: view.getUint8(offset + 2),
      flags: view.getUint8(offset + 3),
      xcm: view.getInt32(offset + 4, true),
      zcm: view.getInt32(offset + 8, true),
      yaw: view.getUint16(offset + 12, true),
      health,
      maxHealth,
      score: view.getUint16(offset + 18, true),
      entity,
    });
  }
  protocolAssert(
    slots.has(view.getUint8(14)),
    'missing_viewer',
    'Snapshot does not contain its viewer player.',
  );

  for (let index = 0; index < projectileCount; index += 1, offset += PROJECTILE_RECORD_BYTES) {
    protocolAssert(view.getUint8(offset + 5) === 0, 'reserved_bits', 'Projectile reserved field is not zero.');
    const entity = view.getUint32(offset, true);
    rejectDuplicate(projectileEntities, entity, 'projectile entity');
    projectiles.push({
      entity,
      owner: view.getUint8(offset + 4),
      ttl: view.getUint16(offset + 6, true),
      xcm: view.getInt32(offset + 8, true),
      zcm: view.getInt32(offset + 12, true),
    });
  }

  for (let index = 0; index < eventCount; index += 1, offset += EVENT_RECORD_BYTES) {
    protocolAssert(view.getUint8(offset + 7) === 0, 'reserved_bits', 'Event reserved field is not zero.');
    const eventId = view.getUint32(offset, true);
    rejectDuplicate(eventIds, eventId, 'event id');
    events.push({
      eventId,
      type: view.getUint8(offset + 4),
      actor: view.getUint8(offset + 5),
      target: view.getUint8(offset + 6),
      xcm: view.getInt32(offset + 8, true),
      zcm: view.getInt32(offset + 12, true),
    });
  }

  return {
    version: view.getUint8(0),
    type: view.getUint8(1),
    byteLength: declaredLength,
    tick: view.getUint32(4, true),
    snapshotSeq: view.getUint32(8, true),
    echoClock: view.getUint16(12, true),
    viewerSlot: view.getUint8(14),
    playerCount,
    projectileCount,
    eventCount,
    matchFlags: view.getUint16(18, true),
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

export function movementToWorld(moveX, moveY, yaw) {
  integerIn(moveX, -127, 127, 'moveX');
  integerIn(moveY, -127, 127, 'moveY');
  integerIn(yaw, 0, 0xffff, 'yaw');
  const radians = wireYawToCoreRadians(yaw);
  const strafe = moveX / 127;
  const forward = moveY / 127;
  const worldX = clamp(Math.round((Math.cos(radians) * forward + Math.sin(radians) * strafe) * 127), -127, 127);
  const worldZ = clamp(Math.round((Math.sin(radians) * forward - Math.cos(radians) * strafe) * 127), -127, 127);
  return {
    x: worldX === 0 ? 0 : worldX,
    y: worldZ === 0 ? 0 : worldZ,
  };
}

function appendTemplate(destination, instanceCount, template, templateOrigin, worldPosition, yaw) {
  const templateCount = template.length / INSTANCE_STRIDE;
  const capacity = destination.length / INSTANCE_STRIDE;
  if (instanceCount + templateCount > capacity) return instanceCount;
  const sine = Math.sin(yaw);
  const cosine = Math.cos(yaw);
  for (let index = 0; index < templateCount; index += 1) {
    const source = index * INSTANCE_STRIDE;
    const target = (instanceCount + index) * INSTANCE_STRIDE;
    const localX = template[source] - templateOrigin[0];
    const localY = template[source + 1] - templateOrigin[1];
    const localZ = template[source + 2] - templateOrigin[2];
    destination[target] = worldPosition[0] + localX * cosine + localZ * sine;
    destination[target + 1] = worldPosition[1] + localY;
    destination[target + 2] = worldPosition[2] - localX * sine + localZ * cosine;
    for (let field = 3; field < INSTANCE_STRIDE; field += 1) {
      destination[target + field] = template[source + field];
    }
    destination[target + 9] = template[source + 9] + yaw;
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
  appendTemplate(result, 0, template, templateOrigin, worldPosition, yaw);
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
  const result = {
    matchId: string(value.matchId, 'match id', 128),
    accountId: string(value.accountId, 'account id', 128),
    slot: number(value.slot, 'slot', 0, 255),
    resumeToken: string(value.resumeToken, 'resume token', 256),
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

function coreRadiansToWireYaw(radians) {
  return (Math.round(radians / (Math.PI * 2) * 0x1_0000) % 0x1_0000 + 0x1_0000) & 0xffff;
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
      player: this.copyPreviewTemplate(PLAYER_SCENE),
      rival: this.copyPreviewTemplate(RIVAL_SCENE),
      firebolt: this.copyPreviewTemplate(FIREBOLT_SCENE),
    };
    this.instances = new Float32Array(sim.instCapacity() * INSTANCE_STRIDE);
    this.particles = new Float32Array(sim.partCapacity() * PARTICLE_STRIDE);
    this.instanceCount = 0;
    this.particleCount = 0;
    this.playerTracks = new Map();
    this.projectileTracks = new Map();
    this.impacts = [];
    this.seenEvents = new Set();
    this.seenEventOrder = [];
    this.keys = new Set();
    this.touchKeys = new Set();
    this.sequence = 1;
    this.yaw = 0;
    this.yawInitialized = false;
    this.castPulse = false;
    this.gamepadCastHeld = false;
    this.latestInput = { x: 0, y: 0 };
    this.touchAim = null;
    this.sentClocks = new Map();
    this.ws = null;
    this.joinGeneration = 0;
    this.reconnectAttempt = 0;
    this.reconnectTimer = null;
    this.inputTimer = null;
    this.joinedRoom = null;
    this.joinSlot = null;
    this.latestSnapshot = null;
    this.latestSnapshotSeq = null;
    this.rtt = null;
    this.protocolFailures = 0;
    this.lastError = null;
    this.interpolationMs = 1_000 / 32;
    this.lastFrameAt = nowMs();
    this.bindControls();
  }

  copyPreviewTemplate(scene) {
    const count = this.sim.previewScene(scene, 0);
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
    this.renderer.uploadHeights(this.heights, 0, this.terrainWidth - 1);
    this.sim.clearDirty?.();
  }

  heightAt(x, z) {
    if (x < 0 || z < 0 || x >= this.worldSize || z >= this.worldSize) return 0;
    const column = clamp(Math.round(x / this.cellSize), 0, this.terrainWidth - 1);
    const row = clamp(Math.round(z / this.cellSize), 0, this.terrainWidth - 1);
    return this.heights[row * this.terrainWidth + column];
  }

  worldPosition(xcm, zcm, clearance) {
    // The legacy render world uses decimetre-scale units: the 20-unit carpet
    // is roughly two metres long. The authoritative service speaks integer cm.
    // Do not clamp: the authoritative core has no arena clamp, so doing so in
    // presentation would make the visible player diverge from the server.
    const x = this.worldCenter + xcm * WORLD_UNITS_PER_CM;
    const z = this.worldCenter + zcm * WORLD_UNITS_PER_CM;
    return { x, y: this.heightAt(x, z) + clearance, z };
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
    return `aetherloom.demo.resume.${new URL(this.controlPlane).host}.${room}`;
  }

  loadResumeToken(room) {
    try { return sessionStorage.getItem(this.sessionKey(room)) || undefined; }
    catch { return undefined; }
  }

  saveResumeToken(room, token) {
    try { sessionStorage.setItem(this.sessionKey(room), token); }
    catch { /* Private browsing may disable storage; the live socket still works. */ }
  }

  clearResumeToken(room) {
    try { sessionStorage.removeItem(this.sessionKey(room)); }
    catch { /* Storage is an optional reconnect optimization. */ }
  }

  async requestJoin(room, resumeToken) {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 12_000);
    try {
      const response = await fetch(`${this.controlPlane}/v1/demo/join`, {
        method: 'POST',
        mode: 'cors',
        credentials: 'omit',
        cache: 'no-store',
        referrerPolicy: 'no-referrer',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ room, ...(resumeToken ? { resumeToken } : {}) }),
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
    }
  }

  async join({ reconnecting = false } = {}) {
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
      this.playerTracks.clear();
      this.projectileTracks.clear();
      this.impacts.length = 0;
      this.seenEvents.clear();
      this.seenEventOrder.length = 0;
      updateRoomUrl(room);
    }

    try {
      const resumeToken = this.loadResumeToken(room);
      let joined;
      try {
        joined = await this.requestJoin(room, resumeToken);
      } catch (error) {
        if (
          resumeToken &&
          error instanceof JoinError &&
          (error.code === 'invalid_resume_token' || error.code === 'resume_expired')
        ) {
          this.clearResumeToken(room);
          joined = await this.requestJoin(room, undefined);
        } else {
          throw error;
        }
      }
      if (generation !== this.joinGeneration) return;
      this.saveResumeToken(room, joined.resumeToken);
      this.joinSlot = joined.slot;
      this.interpolationMs = 1_000 / joined.snapshotHz;
      this.openSocket(joined, generation);
    } catch (error) {
      if (generation !== this.joinGeneration) return;
      this.showError(error);
      this.setConnection('error', reconnecting ? 'Reconnect failed' : 'Could not join staging');
      this.ui.join.disabled = false;
      if (reconnecting) this.scheduleReconnect();
    }
  }

  openSocket(joined, generation) {
    // Snapshot sequence freshness is scoped to a socket stream. A hibernated
    // room may restart its sequence counter while preserving the match.
    this.latestSnapshotSeq = null;
    this.yawInitialized = false;
    this.seenEvents.clear();
    this.seenEventOrder.length = 0;
    this.rtt = null;
    this.sentClocks.clear();
    let socket;
    try {
      socket = new WebSocket(joined.webSocketUrl, [
        'aetherloom.v1',
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
      if (socket.protocol !== 'aetherloom.v1') {
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
      const terminal = new Map([
        [1002, 'The match protocol was rejected. Reload after the client and server builds match.'],
        [1003, 'The match rejected the binary input format.'],
        [1008, 'The match closed this connection for a policy violation.'],
        [4001, 'This room session was opened in another tab. Join here again to take it over.'],
      ]).get(event.code);
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
    this.joinGeneration += 1;
    clearTimeout(this.reconnectTimer);
    this.stopInput();
    if (this.ws) this.ws.close(1000, 'player left');
    this.ws = null;
    this.latestSnapshot = null;
    this.latestSnapshotSeq = null;
    this.playerTracks.clear();
    this.projectileTracks.clear();
    this.impacts.length = 0;
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
    this.sendInput();
    this.inputTimer = setInterval(() => this.sendInput(), 1_000 / rate);
  }

  stopInput() {
    clearInterval(this.inputTimer);
    this.inputTimer = null;
  }

  sampleInput() {
    let horizontal = (this.keys.has('d') || this.keys.has('arrowright') || this.touchKeys.has('d') ? 1 : 0)
      - (this.keys.has('a') || this.keys.has('arrowleft') || this.touchKeys.has('a') ? 1 : 0);
    let vertical = (this.keys.has('w') || this.keys.has('arrowup') || this.touchKeys.has('w') ? 1 : 0)
      - (this.keys.has('s') || this.keys.has('arrowdown') || this.touchKeys.has('s') ? 1 : 0);
    const pads = navigator.getGamepads?.() || [];
    const pad = Array.from(pads).find(Boolean);
    if (pad) {
      const dead = (value) => Math.abs(value) < 0.16 ? 0 : value;
      const padX = dead(pad.axes[0] || 0);
      const padY = -dead(pad.axes[1] || 0);
      if (Math.hypot(padX, padY) > Math.hypot(horizontal, vertical)) {
        horizontal = padX;
        vertical = padY;
      }
      this.yaw = (this.yaw - Math.round(dead(pad.axes[2] || 0) * 780) + 0x1_0000) & 0xffff;
      const held = Boolean(pad.buttons[0]?.pressed || pad.buttons[7]?.pressed);
      if (held && !this.gamepadCastHeld) this.castPulse = true;
      this.gamepadCastHeld = held;
    } else {
      this.gamepadCastHeld = false;
    }
    const length = Math.hypot(horizontal, vertical);
    if (length > 1) {
      horizontal /= length;
      vertical /= length;
    }
    return {
      x: clamp(Math.round(horizontal * 127), -127, 127),
      y: clamp(Math.round(vertical * 127), -127, 127),
    };
  }

  sendInput() {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) return;
    const localMovement = this.sampleInput();
    const movement = movementToWorld(localMovement.x, localMovement.y, this.yaw);
    this.latestInput = movement;
    const cast = this.castPulse;
    this.castPulse = false;
    const sentAt = nowMs();
    const sentClock = Math.floor(sentAt) & 0xffff;
    const frame = encodeInputFrame({
      sequence: this.sequence,
      moveX: movement.x,
      moveY: movement.y,
      yaw: this.yaw,
      cast,
      clientClockMs: sentClock,
      snapshotAck: this.latestSnapshotSeq ?? 0,
    });
    this.sentClocks.set(sentClock, sentAt);
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
    map.set(key, { previous: current, target, from: receivedAt, seenAt: receivedAt });
  }

  handleSnapshot(snapshot) {
    if (snapshot.viewerSlot !== this.joinSlot) {
      throw new ProtocolError('viewer_mismatch', 'Snapshot viewer does not match the authenticated slot.');
    }
    if (!isNewerSequence(snapshot.snapshotSeq, this.latestSnapshotSeq)) return;
    const receivedAt = nowMs();
    this.latestSnapshotSeq = snapshot.snapshotSeq;
    this.latestSnapshot = snapshot;

    for (const player of snapshot.players) {
      const target = this.worldPosition(player.xcm, player.zcm, CARPET_CLEARANCE);
      target.yaw = player.yaw;
      this.updateTrack(this.playerTracks, player.slot, target, receivedAt);
      if (player.slot === snapshot.viewerSlot && !this.yawInitialized) {
        this.yaw = player.yaw;
        this.yawInitialized = true;
      }
    }
    const liveSlots = new Set(snapshot.players.map((player) => player.slot));
    for (const slot of this.playerTracks.keys()) if (!liveSlots.has(slot)) this.playerTracks.delete(slot);

    for (const projectile of snapshot.projectiles) {
      const target = this.worldPosition(projectile.xcm, projectile.zcm, PROJECTILE_CLEARANCE);
      const previous = this.projectileTracks.get(projectile.entity)?.target;
      if (previous && Math.hypot(target.x - previous.x, target.z - previous.z) > 0.001) {
        target.yaw = coreRadiansToWireYaw(Math.atan2(
          target.z - previous.z,
          target.x - previous.x,
        ));
      } else {
        const owner = snapshot.players.find((player) => player.slot === projectile.owner);
        target.yaw = owner ? owner.yaw : 0;
      }
      this.updateTrack(this.projectileTracks, projectile.entity, target, receivedAt);
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
      if (event.type === DAMAGE_EVENT_TYPE) this.spawnImpact(event, receivedAt);
    }

    const echoedAt = snapshot.echoClock === 0 ? undefined : this.sentClocks.get(snapshot.echoClock);
    if (echoedAt !== undefined) {
      this.sentClocks.delete(snapshot.echoClock);
      const roundTrip = receivedAt - echoedAt;
      if (roundTrip >= 0 && roundTrip <= 10_000) {
        this.rtt = this.rtt === null ? roundTrip : this.rtt * 0.76 + roundTrip * 0.24;
      }
    }
    this.updateStats();
  }

  spawnImpact(event, time) {
    const at = this.worldPosition(event.xcm, event.zcm, CARPET_CLEARANCE);
    this.impacts.push({ eventId: event.eventId, at, bornAt: time });
    if (this.impacts.length > MAX_IMPACTS) this.impacts.splice(0, this.impacts.length - MAX_IMPACTS);
  }

  predictedViewer(time) {
    if (!this.latestSnapshot) return null;
    const slot = this.latestSnapshot.viewerSlot;
    const track = this.playerTracks.get(slot);
    const sampled = sampleTrack(track, time, this.interpolationMs);
    if (!sampled) return null;
    const elapsed = clamp((time - track.from) / 1_000, 0, MAX_PREDICTION_SECONDS);
    const movement = this.latestInput || { x: 0, y: 0 };
    let worldX = movement.x / 127;
    let worldZ = movement.y / 127;
    const length = Math.hypot(worldX, worldZ);
    if (length > 1) {
      worldX /= length;
      worldZ /= length;
    }
    sampled.x += worldX * PREDICTION_SPEED * elapsed;
    sampled.z += worldZ * PREDICTION_SPEED * elapsed;
    sampled.y = this.heightAt(sampled.x, sampled.z) + CARPET_CLEARANCE;
    sampled.yaw = this.yaw;
    return sampled;
  }

  composeInstances(time) {
    let count = 0;
    if (this.latestSnapshot) {
      for (const player of this.latestSnapshot.players) {
        const position = player.slot === this.latestSnapshot.viewerSlot
          ? this.predictedViewer(time)
          : sampleTrack(this.playerTracks.get(player.slot), time, this.interpolationMs);
        if (!position) continue;
        count = appendTemplate(
          this.instances,
          count,
          player.slot === this.latestSnapshot.viewerSlot ? this.templates.player : this.templates.rival,
          this.templateOrigin,
          [position.x, position.y, position.z],
          wireYawToRenderRadians(position.yaw),
        );
      }
      for (const projectile of this.latestSnapshot.projectiles) {
        const position = sampleTrack(this.projectileTracks.get(projectile.entity), time, this.interpolationMs);
        if (!position) continue;
        count = appendTemplate(
          this.instances,
          count,
          this.templates.firebolt,
          this.templateOrigin,
          [position.x, position.y, position.z],
          wireYawToRenderRadians(position.yaw),
        );
      }
    }
    this.instanceCount = count;
  }

  composeParticles(time) {
    let count = 0;
    const capacity = this.particles.length / PARTICLE_STRIDE;
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
    const yaw = wireYawToCoreRadians(viewer.yaw);
    const forwardX = Math.cos(yaw);
    const forwardZ = Math.sin(yaw);
    return {
      ex: viewer.x - forwardX * 58,
      ey: viewer.y + 34,
      ez: viewer.z - forwardZ * 58,
      cx: viewer.x + forwardX * 13,
      cy: viewer.y + 2,
      cz: viewer.z + forwardZ * 13,
      ux: 0,
      uy: 1,
      uz: 0,
    };
  }

  render(time) {
    this.composeInstances(time);
    this.composeParticles(time);
    this.renderer.frame(
      this.camera(time),
      this.particles,
      this.particleCount,
      this.instances,
      this.instanceCount,
      {
        fov: 1.05,
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

  bindControls() {
    const movementKeys = new Set(['w', 'a', 's', 'd', 'arrowup', 'arrowdown', 'arrowleft', 'arrowright']);
    addEventListener('keydown', (event) => {
      if (event.target instanceof HTMLInputElement) return;
      const key = event.key.toLowerCase();
      if (movementKeys.has(key)) {
        this.keys.add(key);
        event.preventDefault();
      }
      if ((key === ' ' || key === 'f') && !event.repeat) {
        this.castPulse = true;
        event.preventDefault();
      }
    });
    addEventListener('keyup', (event) => this.keys.delete(event.key.toLowerCase()));
    const clearHeldControls = () => {
      this.keys.clear();
      this.touchKeys.clear();
      this.castPulse = false;
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
        this.yaw = (this.yaw - Math.round(event.movementX * 70) + 0x1_0000) & 0xffff;
      }
    });
    this.ui.canvas.addEventListener('mousedown', (event) => {
      if (event.button !== 0 || document.body.dataset.connection !== 'connected') return;
      this.castPulse = true;
      if (document.pointerLockElement !== this.ui.canvas) {
        try {
          const pending = this.ui.canvas.requestPointerLock?.();
          pending?.catch?.(() => {});
        } catch {
          // Older implementations return void or throw synchronously.
        }
      }
    });
    this.ui.canvas.addEventListener('pointerdown', (event) => {
      if (event.pointerType === 'mouse' || document.body.dataset.connection !== 'connected') return;
      event.preventDefault();
      this.touchAim = { id: event.pointerId, x: event.clientX };
      this.ui.canvas.setPointerCapture?.(event.pointerId);
    });
    this.ui.canvas.addEventListener('pointermove', (event) => {
      if (!this.touchAim || this.touchAim.id !== event.pointerId) return;
      event.preventDefault();
      const delta = event.clientX - this.touchAim.x;
      this.touchAim.x = event.clientX;
      this.yaw = (this.yaw - Math.round(delta * 95) + 0x1_0000) & 0xffff;
    });
    const finishTouchAim = (event) => {
      if (this.touchAim?.id === event.pointerId) this.touchAim = null;
    };
    this.ui.canvas.addEventListener('pointerup', finishTouchAim);
    this.ui.canvas.addEventListener('pointercancel', finishTouchAim);
    this.ui.canvas.addEventListener('lostpointercapture', finishTouchAim);

    for (const button of document.querySelectorAll('.touch-pad button')) {
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
      this.castPulse = true;
      this.ui.touchFire.classList.add('active');
      this.ui.touchFire.setPointerCapture?.(event.pointerId);
    };
    const castUp = (event) => {
      event.preventDefault();
      this.ui.touchFire.classList.remove('active');
    };
    this.ui.touchFire.addEventListener('pointerdown', castDown);
    this.ui.touchFire.addEventListener('pointerup', castUp);
    this.ui.touchFire.addEventListener('pointercancel', castUp);
    this.ui.touchFire.addEventListener('lostpointercapture', castUp);
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
      rttMs: this.rtt === null ? null : Math.round(this.rtt),
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

let app = null;
let bootPromise = null;

const testApi = {
  encodeInputFrame,
  decodeSnapshotFrame,
  composeTemplate,
  isNewerSequence,
  wrappedClockDelta,
  normalizeControlPlane,
  validateRoom,
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
    if (event.key === 'Enter') app.join();
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
