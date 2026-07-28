export const BROWSER_MATCH_MAX_PLAYERS = 8;
export const BROWSER_MATCH_TICK_HZ = 128;
export const BROWSER_MATCH_INPUT_HZ = 64;
export const BROWSER_MATCH_SNAPSHOT_HZ = 32;
export const BROWSER_MATCH_MAX_FRAME_BYTES = 1_200;
export const BROWSER_MATCH_RECONNECT_MS = 90_000;
export const BROWSER_MATCH_BOT_TAKEOVER_MS = 3_000;

export interface BrowserMatchEnv {
  BROWSER_MATCH_ROOMS: DurableObjectNamespace;
  ENVIRONMENT: string;
  TICKET_ISSUER: string;
  JOIN_TICKET_AUDIENCE: string;
  JOIN_TICKET_PUBLIC_KEYS_JSON: string;
  CONTENT_BUILD_HASH: string;
}

export interface BrowserMatchClaimRequest {
  room: string;
  matchId: string;
  buildHash: string;
  resumeToken?: string;
}

export interface BrowserMatchClaim {
  matchId: string;
  matchEpoch: number;
  accountId: string;
  playerSlot: number;
  teamId: number;
  resumeToken: string;
  playerCount: number;
}

export interface PendingInputSample {
  moveX: number;
  moveY: number;
  yaw: number;
  cast: boolean;
}

export interface SocketAttachment {
  accountId: string;
  slot: number;
  teamId: number;
  lastSequence: number;
  clientClock: number;
  lastAckedSnapshotSequence: number;
  lastSentSnapshotSequence: number;
  rateWindowStartedAtMs: number;
  rateWindowMessages: number;
  lastInputAtMs: number;
  sessionExpiresAtMs: number;
  pendingInputs: PendingInputSample[];
}

export interface CorePlayerSnapshot {
  playerId: number;
  controller: number;
  teamId: number;
  entityKey: string;
  xCm: number;
  yCm: number;
  zCm: number;
  velocityXCmPerTick: number;
  velocityYCmPerTick: number;
  velocityZCmPerTick: number;
  yaw: number;
  health: number;
  cooldownTicks: number;
  outcome: number;
}

export interface CoreProjectileSnapshot {
  entityKey: string;
  ownerPlayerId: number;
  teamId: number;
  xCm: number;
  yCm: number;
  zCm: number;
  velocityXCmPerTick: number;
  velocityYCmPerTick: number;
  velocityZCmPerTick: number;
  yaw: number;
  pitch: number;
  lifetimeTicks: number;
  flags: number;
}

export interface CoreEventSnapshot {
  eventId: number;
  tick: number;
  kind: number;
  actorEntityKey: string | null;
  targetEntityKey: string | null;
  data: readonly [number, number, number, number];
}

export interface CoreMatchSnapshot {
  tick: number;
  players: CorePlayerSnapshot[];
  projectiles: CoreProjectileSnapshot[];
  events: CoreEventSnapshot[];
}
