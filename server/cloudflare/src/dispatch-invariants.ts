/**
 * Small, runtime-neutral helpers for the dispatch saga.
 *
 * This module deliberately has no Cloudflare imports so its ordering and
 * idempotency contracts can be executed directly by the validator.
 */

export interface DispatchIdentityInput {
  shard: string;
  matchId: string;
  matchEpoch: number;
  region: string;
  playlist: string;
  inputPool: string;
  buildHash: string;
  targetPlayers: number;
  allowBots: boolean;
}

export interface RosterIdentityPlayer {
  accountId: string;
  reservationId: string;
  playerSlot: number;
  teamId: number;
}

export interface ClaimedRosterEntry {
  accountIds: string[];
  reservationIds: Record<string, string>;
}

export interface DispatchPlayer extends RosterIdentityPlayer {
  partyTeam: number;
}

export type DispatchRecoveryPhase =
  | "staged"
  | "active-pending"
  | "active-confirmed"
  | "aborting";

export function checkpointObjectKey(accountId: string, contentSha256: string): string {
  return `island.checkpoint.${accountId}.${contentSha256}.bin`;
}

export function dispatchIdentityValue(dispatch: DispatchIdentityInput): string {
  return JSON.stringify([
    dispatch.shard,
    dispatch.matchId,
    dispatch.matchEpoch,
    dispatch.region,
    dispatch.playlist,
    dispatch.inputPool,
    dispatch.buildHash,
    dispatch.targetPlayers,
    dispatch.allowBots,
  ]);
}

export function rosterIdentityValue(players: readonly RosterIdentityPlayer[]): string {
  return JSON.stringify(
    players.map((player) => [
      player.accountId,
      player.reservationId,
      player.playerSlot,
      player.teamId,
    ]),
  );
}

export function materializeRoster(
  entries: readonly ClaimedRosterEntry[],
  reportedPlayerCount: number,
  playlist: "casual-extraction" | "solo-extraction" | "squad-extraction",
): DispatchPlayer[] {
  if (!Array.isArray(entries) || !Number.isSafeInteger(reportedPlayerCount)) {
    throw new Error("Matchmaking claim has an invalid roster.");
  }
  const accounts = new Set<string>();
  const players: Array<Omit<DispatchPlayer, "playerSlot" | "teamId">> = [];
  for (const [partyTeam, entry] of entries.entries()) {
    if (
      typeof entry !== "object" ||
      entry === null ||
      !Array.isArray(entry.accountIds) ||
      entry.accountIds.length < 1 ||
      entry.accountIds.length > 4 ||
      typeof entry.reservationIds !== "object" ||
      entry.reservationIds === null ||
      Array.isArray(entry.reservationIds)
    ) {
      throw new Error("Matchmaking claim contains an invalid party.");
    }
    const reservationKeys = Object.keys(entry.reservationIds).sort();
    const sortedAccounts = [...entry.accountIds].sort();
    if (
      reservationKeys.length !== sortedAccounts.length ||
      reservationKeys.some((key, index) => key !== sortedAccounts[index])
    ) {
      throw new Error("Matchmaking claim must provide exactly one reservation per account.");
    }
    for (const accountId of entry.accountIds) {
      const reservationId = entry.reservationIds[accountId];
      if (
        typeof accountId !== "string" ||
        accountId.length === 0 ||
        typeof reservationId !== "string" ||
        reservationId.length === 0 ||
        accounts.has(accountId)
      ) {
        throw new Error("Matchmaking claim contains a duplicate or invalid account reservation.");
      }
      accounts.add(accountId);
      players.push({ accountId, reservationId, partyTeam });
    }
  }
  if (
    players.length < 1 ||
    players.length > 128 ||
    reportedPlayerCount !== players.length
  ) {
    throw new Error("Matchmaking claim player count does not match its roster.");
  }
  return players.map((player, playerSlot) => ({
    ...player,
    playerSlot,
    teamId: playlist === "solo-extraction" ? playerSlot : player.partyTeam,
  }));
}

export function dispatchRecoveryAction(
  phase: DispatchRecoveryPhase,
  allocationExpiresAtMs: number,
  assignmentExpiriesAtMs: readonly number[],
  nowMs: number,
): "done" | "resume" | "abort" {
  if (phase === "active-confirmed") return "done";
  if (phase === "aborting") return "abort";
  const assignmentExpiresAtMs = Math.min(...assignmentExpiriesAtMs);
  if (
    !Number.isSafeInteger(assignmentExpiresAtMs) ||
    assignmentExpiresAtMs <= nowMs + 5_000 ||
    allocationExpiresAtMs <= nowMs + 5_000
  ) {
    return "abort";
  }
  return "resume";
}

/**
 * Commits in order and records a value before starting its request. A rejected
 * request may still have committed remotely if its acknowledgement was lost,
 * so every attempted value belongs in the idempotent rollback set.
 */
export async function commitSequentially<T>(
  values: readonly T[],
  attempted: T[],
  commit: (value: T) => Promise<void>,
): Promise<void> {
  for (const value of values) {
    attempted.push(value);
    await commit(value);
  }
}

/**
 * Waits for every compensation attempt and returns all failures for logging.
 */
export async function compensateAll<T>(
  values: readonly T[],
  compensate: (value: T) => Promise<void>,
): Promise<unknown[]> {
  const outcomes = await Promise.allSettled(
    values.map((value) => Promise.resolve().then(() => compensate(value))),
  );
  return outcomes.flatMap((outcome) =>
    outcome.status === "rejected" ? [outcome.reason] : [],
  );
}

/**
 * Queue confirmation is intentionally last: after it succeeds the caller must
 * perform no further fallible state transition.
 */
export async function activateBeforeConfirm(
  activate: () => Promise<void>,
  confirm: () => Promise<void>,
): Promise<void> {
  await activate();
  await confirm();
}
