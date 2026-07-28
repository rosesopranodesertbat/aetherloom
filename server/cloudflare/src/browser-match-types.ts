export const BROWSER_MATCH_MAX_PLAYERS = 8;
export const BROWSER_MATCH_TICK_HZ = 128;
export const BROWSER_MATCH_INPUT_HZ = 64;
export const BROWSER_MATCH_SNAPSHOT_HZ = 32;
export const BROWSER_MATCH_MAX_FRAME_BYTES = 1_200;
export const BROWSER_MATCH_RECONNECT_MS = 90_000;
export const BROWSER_MATCH_BOT_TAKEOVER_MS = 3_000;
export const BROWSER_MATCH_INPUT_TOKEN_RATE = 128;
export const BROWSER_MATCH_INPUT_TOKEN_CAPACITY = 256;
export const BROWSER_MATCH_INPUT_REJECTION_LIMIT = 256;

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
  sequence: number;
  moveX: number;
  moveY: number;
  moveVertical: number;
  yaw: number;
  pitch: number;
  cast: boolean;
}

export interface SocketAttachment {
  accountId: string;
  slot: number;
  teamId: number;
  lastSequence: number;
  lastAppliedSequence: number;
  clientClock: number;
  lastAckedSnapshotSequence: number;
  lastSentSnapshotSequence: number;
  inputRateTokens: number;
  inputRateUpdatedAtMs: number;
  inputRateRejectedMessages: number;
  lastInputAtMs: number;
  sessionExpiresAtMs: number;
  pendingInputs: PendingInputSample[];
  heldInput: PendingInputSample | null;
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
  pitch: number;
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

export interface InputRateBudget {
  tokens: number;
  updatedAtMs: number;
  rejectedMessages: number;
}

export interface InputRateDecision extends InputRateBudget {
  accepted: boolean;
  shouldClose: boolean;
}

export interface BrowserTickSchedule {
  tickCredit: number;
  lastPulseAtMs: number;
}

export interface BrowserTickBatch extends BrowserTickSchedule {
  ticks: number;
}

export function advanceBrowserTickSchedule(
  schedule: BrowserTickSchedule,
  nowMs: number,
): BrowserTickBatch {
  if (
    !Number.isFinite(schedule.tickCredit) ||
    schedule.tickCredit < 0 ||
    !Number.isFinite(schedule.lastPulseAtMs) ||
    schedule.lastPulseAtMs < 0 ||
    !Number.isFinite(nowMs) ||
    nowMs < 0
  ) {
    throw new Error("Browser match tick schedule is invalid.");
  }
  const elapsed = Math.max(0, Math.min(nowMs - schedule.lastPulseAtMs, 250));
  const available = Math.min(
    32,
    schedule.tickCredit + (elapsed * BROWSER_MATCH_TICK_HZ) / 1_000,
  );
  // Browser inputs are held for exactly two authoritative ticks. Never expose
  // a snapshot halfway through that cycle; retain an odd tick as credit for
  // the next pulse.
  const wholeTicks = Math.min(16, Math.floor(available));
  const ticks = wholeTicks - (wholeTicks % 2);
  return {
    ticks,
    tickCredit: available - ticks,
    lastPulseAtMs: nowMs,
  };
}

export function shouldConsumeBrowserInput(authoritativeTick: number): boolean {
  if (!Number.isSafeInteger(authoritativeTick) || authoritativeTick < 0) {
    throw new Error("Authoritative browser match tick is invalid.");
  }
  return authoritativeTick % 2 === 0;
}

export function isBrowserInputComponent(value: number): boolean {
  return Number.isInteger(value) && value >= -127 && value <= 127;
}

export function consumeBrowserInputToken(
  budget: InputRateBudget,
  nowMs: number,
): InputRateDecision {
  const elapsed = Math.max(0, Math.min(nowMs - budget.updatedAtMs, 60_000));
  let tokens = Math.min(
    BROWSER_MATCH_INPUT_TOKEN_CAPACITY,
    budget.tokens + (elapsed * BROWSER_MATCH_INPUT_TOKEN_RATE) / 1_000,
  );
  let rejectedMessages =
    tokens >= BROWSER_MATCH_INPUT_TOKEN_CAPACITY / 2
      ? 0
      : Math.max(0, budget.rejectedMessages);
  if (tokens < 1) {
    rejectedMessages += 1;
    return {
      accepted: false,
      shouldClose: rejectedMessages >= BROWSER_MATCH_INPUT_REJECTION_LIMIT,
      tokens,
      updatedAtMs: nowMs,
      rejectedMessages,
    };
  }
  tokens -= 1;
  return {
    accepted: true,
    shouldClose: false,
    tokens,
    updatedAtMs: nowMs,
    rejectedMessages,
  };
}
