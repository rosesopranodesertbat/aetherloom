import { DurableObject } from "cloudflare:workers";
import type {
  ClaimedQueueEntry,
  Env,
  InputPool,
  MatchmakingEnqueueCommand,
  Playlist,
} from "./types";
import {
  ApiError,
  assertObject,
  canonicalJson,
  errorResponse,
  jsonResponse,
  readJson,
  requireIdentifier,
  requireInteger,
} from "./util";

interface QueueRow extends Record<string, SqlStorageValue> {
  queue_ticket_id: string;
  party_id: string;
  account_ids_json: string;
  reservation_ids_json: string;
  player_count: number;
  skill_mmr: number;
  request_hash: string;
  status: "waiting" | "leased" | "matched" | "cancelled" | "expired";
  lease_id: string | null;
  lease_expires_at_ms: number | null;
  match_id: string | null;
  assignment_json: string | null;
  created_at_ms: number;
  expires_at_ms: number;
}

interface ClaimCommand {
  leaseId: string;
  matchId: string;
  maxPlayers: number;
  leaseExpiresAtMs: number;
}

interface ConfirmCommand {
  leaseId: string;
  matchId: string;
  assignments: Record<
    string,
    {
      matchId: string;
      matchEpoch: number;
      joinTicket: string;
      transport: "wss" | "quic";
      endpoint: string;
      expiresAtMs: number;
    }
  >;
}

interface PrepareCommand extends ConfirmCommand {
  holdExpiresAtMs: number;
}

interface LeaseCommand {
  leaseId: string;
}

interface AbortLeaseCommand extends LeaseCommand {
  matchId: string;
}

function earliestAssignmentExpiry(
  assignments: ConfirmCommand["assignments"],
  matchId: string,
): number {
  const values = Object.entries(assignments);
  if (values.length === 0) {
    throw new ApiError(400, "assignment_roster_mismatch", "Assignments cannot be empty.");
  }
  let expectedMatchEpoch: number | undefined;
  return Math.min(
    ...values.map(([accountId, assignment]) => {
      assertObject(assignment, `assignment for ${accountId}`);
      if (assignment.matchId !== matchId) {
        throw new ApiError(400, "invalid_assignment", `Invalid match assignment for ${accountId}.`);
      }
      const matchEpoch = requireInteger(
        assignment.matchEpoch,
        `assignment match epoch for ${accountId}`,
        1,
        Number.MAX_SAFE_INTEGER,
      );
      if (expectedMatchEpoch !== undefined && matchEpoch !== expectedMatchEpoch) {
        throw new ApiError(
          400,
          "assignment_epoch_mismatch",
          "Join assignments do not share one match epoch.",
        );
      }
      expectedMatchEpoch = matchEpoch;
      return requireInteger(
        assignment.expiresAtMs,
        `assignment expiry for ${accountId}`,
        0,
        9_007_199_254_740_991,
      );
    }),
  );
}

function fnv1a(value: string): number {
  let hash = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash >>> 0;
}

export function matchmakingShardName(
  region: string,
  playlist: Playlist,
  inputPool: InputPool,
  skillMmr: number,
  partyId: string,
  shardCount: number,
  skillBucketWidth: number,
): string {
  const skillBucket = Math.floor(Math.max(0, skillMmr) / skillBucketWidth);
  const shardIndex = fnv1a(partyId) % shardCount;
  return `${region}:${playlist}:${inputPool}:mmr-${skillBucket}:shard-${shardIndex}`;
}

export class MatchmakingShardObject extends DurableObject<Env> {
  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    this.ctx.storage.transactionSync(() => {
      this.ctx.storage.sql.exec(`
        CREATE TABLE IF NOT EXISTS queue_entries (
          queue_ticket_id TEXT PRIMARY KEY,
          party_id TEXT NOT NULL,
          account_ids_json TEXT NOT NULL,
          reservation_ids_json TEXT NOT NULL,
          player_count INTEGER NOT NULL CHECK (player_count BETWEEN 1 AND 4),
          skill_mmr INTEGER NOT NULL CHECK (skill_mmr BETWEEN 0 AND 10000),
          request_hash TEXT NOT NULL,
          status TEXT NOT NULL CHECK (status IN ('waiting', 'leased', 'matched', 'cancelled', 'expired')),
          lease_id TEXT,
          lease_expires_at_ms INTEGER,
          match_id TEXT,
          assignment_json TEXT,
          created_at_ms INTEGER NOT NULL,
          expires_at_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS queue_waiting_order
          ON queue_entries(status, created_at_ms, queue_ticket_id);
        CREATE INDEX IF NOT EXISTS queue_lease_expiry
          ON queue_entries(status, lease_expires_at_ms);

        CREATE TABLE IF NOT EXISTS active_members (
          account_id TEXT PRIMARY KEY,
          queue_ticket_id TEXT NOT NULL,
          FOREIGN KEY (queue_ticket_id) REFERENCES queue_entries(queue_ticket_id)
        );
      `);
    });
  }

  override async fetch(request: Request): Promise<Response> {
    try {
      const url = new URL(request.url);
      this.reap(Date.now());
      if (request.method === "POST" && url.pathname === "/enqueue") {
        return await this.enqueue(await readJson<MatchmakingEnqueueCommand>(request));
      }
      if (request.method === "POST" && url.pathname === "/status") {
        const body = await readJson<{ queueTicketId: string; accountId: string }>(request);
        return this.status(body.queueTicketId, body.accountId);
      }
      if (request.method === "POST" && url.pathname === "/cancel") {
        const body = await readJson<{ queueTicketId: string; accountId: string }>(request);
        return await this.cancel(body.queueTicketId, body.accountId);
      }
      if (request.method === "POST" && url.pathname === "/claim") {
        return await this.claim(await readJson<ClaimCommand>(request));
      }
      if (request.method === "POST" && url.pathname === "/confirm") {
        return await this.confirm(await readJson<ConfirmCommand>(request, 256 * 1024));
      }
      if (request.method === "POST" && url.pathname === "/prepare") {
        return await this.prepare(await readJson<PrepareCommand>(request, 256 * 1024));
      }
      if (request.method === "POST" && url.pathname === "/abort-prepared") {
        return await this.abortPrepared(await readJson<AbortLeaseCommand>(request));
      }
      if (request.method === "POST" && url.pathname === "/release") {
        return await this.release(await readJson<LeaseCommand>(request));
      }
      return jsonResponse({ error: { code: "not_found", message: "Matchmaking operation not found." } }, 404);
    } catch (error) {
      return errorResponse(error, "matchmaking-shard-do");
    }
  }

  override async alarm(): Promise<void> {
    this.reap(Date.now());
    await this.scheduleAlarm();
  }

  private async enqueue(command: MatchmakingEnqueueCommand): Promise<Response> {
    const queueTicketId = requireIdentifier(command.queueTicketId, "queueTicketId");
    const partyId = requireIdentifier(command.partyId, "partyId");
    const requestHash = requireIdentifier(command.requestHash, "requestHash");
    const accountIds = this.accountIds(command.accountIds);
    const reservationIds = this.reservationIds(command.reservationIds, accountIds);
    const skillMmr = requireInteger(command.skillMmr, "skillMmr", 0, 10_000);
    const expiresAtMs = requireInteger(
      command.expiresAtMs,
      "expiresAtMs",
      Date.now() + 30_000,
      Date.now() + 600_000,
    );

    const result = this.ctx.storage.transactionSync(() => {
      const existing = this.row(queueTicketId);
      if (existing !== undefined) {
        if (existing.request_hash !== requestHash) {
          throw new ApiError(
            409,
            "idempotency_conflict",
            "Queue ticket was already used for different matchmaking parameters.",
          );
        }
        return { status: 200, body: this.present(existing) };
      }

      for (const accountId of accountIds) {
        const active = this.ctx.storage.sql
          .exec<{ queue_ticket_id: string }>(
            "SELECT queue_ticket_id FROM active_members WHERE account_id = ?",
            accountId,
          )
          .toArray()[0];
        if (active !== undefined) {
          throw new ApiError(409, "already_queued", `${accountId} already has an active queue entry.`);
        }
      }
      const now = Date.now();
      this.ctx.storage.sql.exec(
        `INSERT INTO queue_entries(
           queue_ticket_id, party_id, account_ids_json, reservation_ids_json,
           player_count, skill_mmr, request_hash, status, created_at_ms, expires_at_ms
         ) VALUES(?, ?, ?, ?, ?, ?, ?, 'waiting', ?, ?)`,
        queueTicketId,
        partyId,
        JSON.stringify(accountIds),
        JSON.stringify(reservationIds),
        accountIds.length,
        skillMmr,
        requestHash,
        now,
        expiresAtMs,
      );
      for (const accountId of accountIds) {
        this.ctx.storage.sql.exec(
          "INSERT INTO active_members(account_id, queue_ticket_id) VALUES(?, ?)",
          accountId,
          queueTicketId,
        );
      }
      return {
        status: 201,
        body: {
          queueTicketId,
          partyId,
          status: "waiting",
          playerCount: accountIds.length,
          createdAtMs: now,
          expiresAtMs,
        },
      };
    });
    await this.scheduleAlarm();
    return jsonResponse(result.body, result.status);
  }

  private status(queueTicketIdRaw: string, accountIdRaw: string): Response {
    const queueTicketId = requireIdentifier(queueTicketIdRaw, "queueTicketId");
    const accountId = requireIdentifier(accountIdRaw, "accountId");
    const row = this.row(queueTicketId);
    if (row === undefined) throw new ApiError(404, "queue_ticket_not_found", "Queue ticket was not found.");
    const members = JSON.parse(row.account_ids_json) as string[];
    if (!members.includes(accountId)) {
      throw new ApiError(403, "queue_ticket_forbidden", "Queue ticket does not belong to this account.");
    }
    const result = this.present(row) as Record<string, unknown>;
    if (row.status === "matched" && row.assignment_json !== null) {
      const assignments = JSON.parse(row.assignment_json) as Record<string, unknown>;
      result.assignment = assignments[accountId];
    }
    return jsonResponse(result);
  }

  private async cancel(queueTicketIdRaw: string, accountIdRaw: string): Promise<Response> {
    const queueTicketId = requireIdentifier(queueTicketIdRaw, "queueTicketId");
    const accountId = requireIdentifier(accountIdRaw, "accountId");
    const body = this.ctx.storage.transactionSync(() => {
      const row = this.row(queueTicketId);
      if (row === undefined) throw new ApiError(404, "queue_ticket_not_found", "Queue ticket was not found.");
      const members = JSON.parse(row.account_ids_json) as string[];
      if (!members.includes(accountId)) {
        throw new ApiError(403, "queue_ticket_forbidden", "Queue ticket does not belong to this account.");
      }
      if (row.status === "cancelled" || row.status === "expired") return this.present(row);
      if (row.status !== "waiting") {
        throw new ApiError(409, "queue_ticket_locked", "Queue ticket is being assigned or has matched.");
      }
      this.ctx.storage.sql.exec(
        "UPDATE queue_entries SET status = 'cancelled' WHERE queue_ticket_id = ?",
        queueTicketId,
      );
      this.removeMembers(queueTicketId);
      return { ...this.present(row), status: "cancelled" };
    });
    await this.scheduleAlarm();
    return jsonResponse(body);
  }

  private async claim(command: ClaimCommand): Promise<Response> {
    const leaseId = requireIdentifier(command.leaseId, "leaseId");
    const matchId = requireIdentifier(command.matchId, "matchId");
    const maxPlayers = requireInteger(command.maxPlayers, "maxPlayers", 1, 128);
    const leaseExpiresAtMs = requireInteger(
      command.leaseExpiresAtMs,
      "leaseExpiresAtMs",
      Date.now() + 5_000,
      Date.now() + 60_000,
    );

    const entries = this.ctx.storage.transactionSync(() => {
      const candidates = this.ctx.storage.sql
        .exec<QueueRow>(
          `SELECT * FROM queue_entries
           WHERE status = 'waiting' AND expires_at_ms > ?
           ORDER BY created_at_ms, queue_ticket_id
           LIMIT 128`,
          Date.now(),
        )
        .toArray();
      const selected: QueueRow[] = [];
      let players = 0;
      for (const candidate of candidates) {
        if (players + candidate.player_count > maxPlayers) continue;
        selected.push(candidate);
        players += candidate.player_count;
        if (players === maxPlayers) break;
      }
      for (const selectedRow of selected) {
        this.ctx.storage.sql.exec(
          `UPDATE queue_entries
           SET status = 'leased', lease_id = ?, lease_expires_at_ms = ?, match_id = ?
           WHERE queue_ticket_id = ? AND status = 'waiting'`,
          leaseId,
          leaseExpiresAtMs,
          matchId,
          selectedRow.queue_ticket_id,
        );
      }
      return selected.map<ClaimedQueueEntry>((row) => ({
        queueTicketId: row.queue_ticket_id,
        partyId: row.party_id,
        accountIds: JSON.parse(row.account_ids_json) as string[],
        reservationIds: JSON.parse(row.reservation_ids_json) as Record<string, string>,
        skillMmr: row.skill_mmr,
      }));
    });
    await this.scheduleAlarm();
    return jsonResponse({
      leaseId,
      matchId,
      playerCount: entries.reduce((total, entry) => total + entry.accountIds.length, 0),
      entries,
    });
  }

  private async confirm(command: ConfirmCommand): Promise<Response> {
    const leaseId = requireIdentifier(command.leaseId, "leaseId");
    const matchId = requireIdentifier(command.matchId, "matchId");
    assertObject(command.assignments, "assignments");
    const assignmentExpiresAtMs = earliestAssignmentExpiry(command.assignments, matchId);
    const body = this.ctx.storage.transactionSync(() => {
      const rows = this.ctx.storage.sql
        .exec<QueueRow>(
          `SELECT * FROM queue_entries
           WHERE status IN ('leased', 'matched') AND lease_id = ?
           ORDER BY queue_ticket_id`,
          leaseId,
        )
        .toArray();
      if (rows.length === 0) throw new ApiError(409, "lease_not_found", "Matchmaking lease is not active.");
      const statuses = new Set(rows.map((row) => row.status));
      if (statuses.size !== 1) {
        throw new ApiError(409, "lease_state_conflict", "Matchmaking lease has inconsistent entry states.");
      }
      if (rows[0]?.status === "leased" && assignmentExpiresAtMs <= Date.now()) {
        throw new ApiError(409, "assignment_expired", "Prepared assignments have expired.");
      }
      const expectedAccounts = rows
        .flatMap((row) => JSON.parse(row.account_ids_json) as string[])
        .sort();
      const assignmentAccounts = Object.keys(command.assignments).sort();
      if (
        expectedAccounts.length !== assignmentAccounts.length ||
        expectedAccounts.some((accountId, index) => accountId !== assignmentAccounts[index])
      ) {
        throw new ApiError(400, "assignment_roster_mismatch", "Assignments do not exactly match the lease roster.");
      }
      for (const row of rows) {
        if (row.match_id !== matchId) {
          throw new ApiError(409, "lease_match_conflict", "Lease belongs to another match.");
        }
        const members = JSON.parse(row.account_ids_json) as string[];
        const partyAssignments = Object.fromEntries(
          members.map((accountId) => {
            const assignment = command.assignments[accountId];
            if (assignment === undefined) {
              throw new ApiError(400, "missing_assignment", `No match assignment for ${accountId}.`);
            }
            return [accountId, assignment];
          }),
        );
        if (row.status === "matched") {
          if (
            row.assignment_json === null ||
            canonicalJson(JSON.parse(row.assignment_json)) !== canonicalJson(partyAssignments)
          ) {
            throw new ApiError(
              409,
              "assignment_conflict",
              "Lease was already confirmed with different assignments.",
            );
          }
          continue;
        }
        if (
          row.assignment_json === null ||
          canonicalJson(JSON.parse(row.assignment_json)) !== canonicalJson(partyAssignments)
        ) {
          throw new ApiError(
            409,
            "confirmation_not_prepared",
            "Lease was not prepared with these assignments.",
          );
        }
        this.ctx.storage.sql.exec(
          `UPDATE queue_entries
           SET status = 'matched', assignment_json = ?, lease_expires_at_ms = NULL
           WHERE queue_ticket_id = ?`,
          JSON.stringify(partyAssignments),
          row.queue_ticket_id,
        );
        this.removeMembers(row.queue_ticket_id);
      }
      return {
        leaseId,
        matchId,
        status: "matched",
        partyCount: rows.length,
        idempotentReplay: rows[0]?.status === "matched",
      };
    });
    await this.scheduleAlarm();
    return jsonResponse(body);
  }

  private async prepare(command: PrepareCommand): Promise<Response> {
    const leaseId = requireIdentifier(command.leaseId, "leaseId");
    const matchId = requireIdentifier(command.matchId, "matchId");
    assertObject(command.assignments, "assignments");
    const holdExpiresAtMs = requireInteger(
      command.holdExpiresAtMs,
      "holdExpiresAtMs",
      0,
      9_007_199_254_740_991,
    );
    const assignmentExpiresAtMs = earliestAssignmentExpiry(command.assignments, matchId);
    if (holdExpiresAtMs !== assignmentExpiresAtMs) {
      throw new ApiError(
        400,
        "assignment_expiry_mismatch",
        "Prepared hold must end with the earliest join assignment.",
      );
    }
    const body = this.ctx.storage.transactionSync(() => {
      const rows = this.ctx.storage.sql
        .exec<QueueRow>(
          `SELECT * FROM queue_entries
           WHERE status IN ('leased', 'matched') AND lease_id = ?
           ORDER BY queue_ticket_id`,
          leaseId,
        )
        .toArray();
      if (rows.length === 0) {
        throw new ApiError(409, "lease_not_found", "Matchmaking lease is not active.");
      }
      const statuses = new Set(rows.map((row) => row.status));
      if (statuses.size !== 1) {
        throw new ApiError(409, "lease_state_conflict", "Matchmaking lease has inconsistent entry states.");
      }
      const expectedAccounts = rows
        .flatMap((row) => JSON.parse(row.account_ids_json) as string[])
        .sort();
      const assignmentAccounts = Object.keys(command.assignments).sort();
      if (
        expectedAccounts.length !== assignmentAccounts.length ||
        expectedAccounts.some((accountId, index) => accountId !== assignmentAccounts[index])
      ) {
        throw new ApiError(400, "assignment_roster_mismatch", "Assignments do not exactly match the lease roster.");
      }
      if (rows[0]?.status === "leased" && assignmentExpiresAtMs <= Date.now() + 5_000) {
        throw new ApiError(409, "assignment_expired", "Assignments expire too soon to prepare.");
      }
      for (const row of rows) {
        if (row.match_id !== matchId) {
          throw new ApiError(409, "lease_match_conflict", "Lease belongs to another match.");
        }
        const members = JSON.parse(row.account_ids_json) as string[];
        const partyAssignments = Object.fromEntries(
          members.map((accountId) => {
            const assignment = command.assignments[accountId];
            if (assignment === undefined || assignment.matchId !== matchId) {
              throw new ApiError(400, "invalid_assignment", `Invalid match assignment for ${accountId}.`);
            }
            return [accountId, assignment];
          }),
        );
        const assignmentJson = canonicalJson(partyAssignments);
        if (row.assignment_json !== null) {
          if (canonicalJson(JSON.parse(row.assignment_json)) !== assignmentJson) {
            throw new ApiError(
              409,
              "assignment_conflict",
              "Lease was already prepared with different assignments.",
            );
          }
        } else if (row.status === "matched") {
          throw new ApiError(409, "assignment_conflict", "Matched lease has no assignment receipt.");
        }
        if (row.status === "leased") {
          this.ctx.storage.sql.exec(
            `UPDATE queue_entries
             SET assignment_json = ?, lease_expires_at_ms = max(lease_expires_at_ms, ?)
             WHERE queue_ticket_id = ? AND status = 'leased' AND lease_id = ?`,
            assignmentJson,
            holdExpiresAtMs,
            row.queue_ticket_id,
            leaseId,
          );
        }
      }
      return {
        leaseId,
        matchId,
        status: rows[0]?.status === "matched" ? "matched" : "prepared",
        partyCount: rows.length,
        idempotentReplay: rows.every((row) => row.assignment_json !== null),
      };
    });
    await this.scheduleAlarm();
    return jsonResponse(body);
  }

  private async release(command: LeaseCommand): Promise<Response> {
    const leaseId = requireIdentifier(command.leaseId, "leaseId");
    const body = this.ctx.storage.transactionSync(() => {
      const cursor = this.ctx.storage.sql.exec(
        `UPDATE queue_entries
         SET status = 'waiting', lease_id = NULL, lease_expires_at_ms = NULL, match_id = NULL
         WHERE status = 'leased' AND lease_id = ? AND assignment_json IS NULL`,
        leaseId,
      );
      return { leaseId, status: "released", entriesReleased: cursor.rowsWritten };
    });
    await this.scheduleAlarm();
    return jsonResponse(body);
  }

  private async abortPrepared(command: AbortLeaseCommand): Promise<Response> {
    const leaseId = requireIdentifier(command.leaseId, "leaseId");
    const matchId = requireIdentifier(command.matchId, "matchId");
    const body = this.ctx.storage.transactionSync(() => {
      const matched = this.ctx.storage.sql
        .exec<{ queue_ticket_id: string }>(
          `SELECT queue_ticket_id FROM queue_entries
           WHERE status = 'matched' AND lease_id = ? AND match_id = ?
           LIMIT 1`,
          leaseId,
          matchId,
        )
        .toArray()[0];
      if (matched !== undefined) {
        throw new ApiError(409, "lease_already_confirmed", "Prepared lease is already matched.");
      }
      const rows = this.ctx.storage.sql
        .exec<{ queue_ticket_id: string }>(
          `SELECT queue_ticket_id FROM queue_entries
           WHERE status = 'leased' AND lease_id = ? AND match_id = ?`,
          leaseId,
          matchId,
        )
        .toArray();
      for (const row of rows) {
        this.ctx.storage.sql.exec(
          `UPDATE queue_entries
           SET status = 'cancelled', lease_expires_at_ms = NULL
           WHERE queue_ticket_id = ? AND status = 'leased'`,
          row.queue_ticket_id,
        );
        this.removeMembers(row.queue_ticket_id);
      }
      const priorAbortCount = this.ctx.storage.sql
        .exec<{ count: number }>(
          `SELECT count(*) AS count FROM queue_entries
           WHERE status = 'cancelled' AND lease_id = ? AND match_id = ?`,
          leaseId,
          matchId,
        )
        .toArray()[0]?.count ?? 0;
      return {
        leaseId,
        matchId,
        status: "aborted",
        entriesAborted: priorAbortCount,
        idempotentReplay: rows.length === 0 && priorAbortCount > 0,
      };
    });
    await this.scheduleAlarm();
    return jsonResponse(body);
  }

  private reap(now: number): void {
    this.ctx.storage.transactionSync(() => {
      this.ctx.storage.sql.exec(
        `UPDATE queue_entries
         SET lease_expires_at_ms = NULL
         WHERE status = 'leased' AND assignment_json IS NOT NULL
           AND lease_expires_at_ms <= ?`,
        now,
      );
      this.ctx.storage.sql.exec(
        `UPDATE queue_entries
         SET status = 'waiting', lease_id = NULL, lease_expires_at_ms = NULL,
             match_id = NULL, assignment_json = NULL
         WHERE status = 'leased' AND assignment_json IS NULL
           AND lease_expires_at_ms <= ?`,
        now,
      );
      const expired = this.ctx.storage.sql
        .exec<{ queue_ticket_id: string }>(
          `SELECT queue_ticket_id FROM queue_entries
           WHERE status = 'waiting' AND expires_at_ms <= ?`,
          now,
        )
        .toArray();
      for (const row of expired) {
        this.ctx.storage.sql.exec(
          "UPDATE queue_entries SET status = 'expired' WHERE queue_ticket_id = ?",
          row.queue_ticket_id,
        );
        this.removeMembers(row.queue_ticket_id);
      }
    });
  }

  private async scheduleAlarm(): Promise<void> {
    const next = this.ctx.storage.sql
      .exec<{ wake_at_ms: number | null }>(
        `SELECT min(wake_at_ms) AS wake_at_ms FROM (
           SELECT expires_at_ms AS wake_at_ms FROM queue_entries WHERE status = 'waiting'
           UNION ALL
           SELECT lease_expires_at_ms AS wake_at_ms FROM queue_entries WHERE status = 'leased'
         )`,
      )
      .toArray()[0];
    if (next === undefined || next.wake_at_ms === null) {
      await this.ctx.storage.deleteAlarm();
    } else {
      await this.ctx.storage.setAlarm(Math.max(Date.now() + 1_000, next.wake_at_ms));
    }
  }

  private row(queueTicketId: string): QueueRow | undefined {
    return this.ctx.storage.sql
      .exec<QueueRow>("SELECT * FROM queue_entries WHERE queue_ticket_id = ?", queueTicketId)
      .toArray()[0];
  }

  private present(row: QueueRow): Record<string, unknown> {
    return {
      queueTicketId: row.queue_ticket_id,
      partyId: row.party_id,
      status: row.status,
      playerCount: row.player_count,
      skillMmr: row.skill_mmr,
      matchId: row.match_id,
      createdAtMs: row.created_at_ms,
      expiresAtMs: row.expires_at_ms,
    };
  }

  private removeMembers(queueTicketId: string): void {
    this.ctx.storage.sql.exec("DELETE FROM active_members WHERE queue_ticket_id = ?", queueTicketId);
  }

  private accountIds(value: unknown): string[] {
    if (!Array.isArray(value) || value.length < 1 || value.length > 4) {
      throw new ApiError(400, "invalid_party", "Party must contain between one and four accounts.");
    }
    const accounts = value.map((account, index) => requireIdentifier(account, `accountIds[${index}]`));
    if (new Set(accounts).size !== accounts.length) {
      throw new ApiError(400, "invalid_party", "Party contains duplicate accounts.");
    }
    return [...accounts].sort();
  }

  private reservationIds(value: unknown, accountIds: string[]): Record<string, string> {
    assertObject(value, "reservationIds");
    const keys = Object.keys(value).sort();
    if (keys.length !== accountIds.length || keys.some((key, index) => key !== accountIds[index])) {
      throw new ApiError(400, "invalid_reservations", "Every party member needs exactly one reservation.");
    }
    return Object.fromEntries(
      accountIds.map((accountId) => [
        accountId,
        requireIdentifier(value[accountId], `reservationIds.${accountId}`),
      ]),
    );
  }
}
