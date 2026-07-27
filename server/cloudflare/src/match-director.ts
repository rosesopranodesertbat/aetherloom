import type {
  Env,
  InputPool,
  MatchAllocation,
  MatchTransport,
  Playlist,
  TicketClaims,
} from "./types";
import { signTicket } from "./tickets";
import {
  ApiError,
  assertObject,
  canonicalJson,
  randomToken,
  readJson,
  requireIdentifier,
  requireInteger,
  requireString,
} from "./util";

export interface ReserveMatchRequest {
  matchId: string;
  dispatchHash: string;
  rosterHash: string;
  region: string;
  playlist: Playlist;
  inputPool: InputPool;
  buildHash: string;
  playerCount: number;
  botCount: number;
  authoritativeHz: 128;
}

export interface ResumableDispatch {
  allocation: MatchAllocation;
  phase: "staged" | "active-pending" | "active-confirmed" | "aborting";
  rosterHash: string;
  playerCount: number;
  botCount: number;
  queueConfirmation: {
    shard: string;
    leaseId: string;
    assignments: Record<string, MatchAssignment>;
    reservations: Array<{ accountId: string; reservationId: string }>;
    confirmed: boolean;
  };
}

export interface MatchAssignment {
  matchId: string;
  joinTicket: string;
  transport: MatchTransport;
  endpoint: string;
  expiresAtMs: number;
}

export interface DispatchConfirmation {
  shard: string;
  leaseId: string;
  assignments: Record<string, MatchAssignment>;
  reservations: Array<{ accountId: string; reservationId: string }>;
}

export interface MatchDirector {
  resumeDispatch(matchId: string, dispatchHash: string): Promise<ResumableDispatch | null>;
  reserve(request: ReserveMatchRequest): Promise<MatchAllocation>;
  stageQueueConfirmation(
    matchId: string,
    hostId: string,
    dispatchHash: string,
    rosterHash: string,
    confirmation: DispatchConfirmation,
  ): Promise<ResumableDispatch>;
  activate(
    matchId: string,
    hostId: string,
    dispatchHash: string,
    rosterHash: string,
  ): Promise<void>;
  markQueueConfirmed(
    matchId: string,
    hostId: string,
    dispatchHash: string,
    leaseId: string,
  ): Promise<void>;
  beginAbort(
    matchId: string,
    hostId: string,
    dispatchHash: string,
    rosterHash: string,
  ): Promise<boolean>;
  restoreActiveAfterConfirmedAbort(
    matchId: string,
    hostId: string,
    dispatchHash: string,
    rosterHash: string,
  ): Promise<void>;
  recordExpiredConfirmedAbort(
    matchId: string,
    hostId: string,
    dispatchHash: string,
    rosterHash: string,
    leaseId: string,
  ): Promise<void>;
  release(matchId: string, hostId: string, reason: string): Promise<void>;
  browserAllocation(matchId: string): Promise<MatchAllocation>;
}

interface CapacityResponse {
  hostId: string;
  transport: MatchTransport;
  nativeEndpoint?: string;
  expiresAtMs: number;
}

interface AllocationRow {
  match_id: string;
  dispatch_hash: string;
  roster_hash: string;
  host_id: string;
  region: string;
  playlist: Playlist;
  input_pool: InputPool;
  transport: MatchTransport;
  native_endpoint: string | null;
  build_hash: string;
  expires_at_ms: number;
  status: "reserved" | "active" | "aborting" | "draining" | "complete" | "released";
  player_count: number;
  bot_count: number;
  matchmaking_shard: string | null;
  lease_id: string | null;
  assignments_json: string | null;
  reservations_json: string | null;
  queue_confirmed_at_ms: number | null;
}

export class ServiceBindingMatchDirector implements MatchDirector {
  constructor(private readonly env: Env) {}

  async resumeDispatch(
    matchIdRaw: string,
    dispatchHashRaw: string,
  ): Promise<ResumableDispatch | null> {
    const matchId = requireIdentifier(matchIdRaw, "matchId");
    const dispatchHash = requireIdentifier(dispatchHashRaw, "dispatchHash");
    const existing = await this.allocationRow(matchId);
    if (existing === null) return null;
    if (existing.dispatch_hash !== dispatchHash) {
      throw new ApiError(
        409,
        "match_id_conflict",
        "Match id was already used for a different dispatch request.",
      );
    }
    if (["reserved", "active", "aborting"].includes(existing.status)) {
      if (
        existing.matchmaking_shard === null ||
        existing.lease_id === null ||
        existing.assignments_json === null ||
        existing.reservations_json === null
      ) {
        if (existing.status === "reserved") {
          if (existing.expires_at_ms > Date.now()) {
            throw new ApiError(409, "dispatch_in_progress", "Match dispatch is already in progress.");
          }
          throw new ApiError(
            409,
            "match_id_conflict",
            "Expired unstaged match id cannot be recovered or rebound.",
          );
        }
        throw new Error(`Active match ${matchId} is missing its queue confirmation journal.`);
      }
      if (
        existing.status === "active" &&
        existing.queue_confirmed_at_ms !== null &&
        existing.expires_at_ms <= Date.now()
      ) {
        throw new ApiError(409, "match_id_conflict", "Active match allocation has expired.");
      }
      let assignments: Record<string, MatchAssignment>;
      let reservations: Array<{ accountId: string; reservationId: string }>;
      try {
        assignments = JSON.parse(existing.assignments_json) as Record<string, MatchAssignment>;
        reservations = JSON.parse(existing.reservations_json) as Array<{
          accountId: string;
          reservationId: string;
        }>;
      } catch {
        throw new Error(`Active match ${matchId} has an invalid queue confirmation journal.`);
      }
      return {
        allocation: this.presentAllocation(existing),
        phase:
          existing.status === "aborting"
            ? "aborting"
            : existing.status === "reserved"
            ? "staged"
            : existing.queue_confirmed_at_ms === null
              ? "active-pending"
              : "active-confirmed",
        rosterHash: existing.roster_hash,
        playerCount: existing.player_count,
        botCount: existing.bot_count,
        queueConfirmation: {
          shard: existing.matchmaking_shard,
          leaseId: existing.lease_id,
          assignments,
          reservations,
          confirmed: existing.queue_confirmed_at_ms !== null,
        },
      };
    }
    throw new ApiError(
      409,
      "match_id_conflict",
      "Match id already belongs to an inactive allocation and cannot be rebound.",
    );
  }

  async reserve(request: ReserveMatchRequest): Promise<MatchAllocation> {
    const existing = await this.allocationRow(request.matchId);
    if (existing !== null) {
      if (
        !["reserved", "active"].includes(existing.status) ||
        existing.expires_at_ms <= Date.now() ||
        existing.dispatch_hash !== request.dispatchHash ||
        existing.roster_hash !== request.rosterHash ||
        existing.region !== request.region ||
        existing.playlist !== request.playlist ||
        existing.input_pool !== request.inputPool ||
        existing.build_hash !== request.buildHash ||
        existing.player_count !== request.playerCount ||
        existing.bot_count !== request.botCount
      ) {
        throw new ApiError(
          409,
          "match_id_conflict",
          "Match id already belongs to a different or inactive allocation.",
        );
      }
      return this.presentAllocation(existing);
    }
    if (this.env.MATCH_CAPACITY_API === undefined) {
      throw new ApiError(503, "match_capacity_unavailable", "Match capacity service is not configured.");
    }
    const serviceTicket = await this.serviceTicket("match-director", ["capacity:reserve"], {
      match_id: request.matchId,
      build_hash: request.buildHash,
      region: request.region,
    });
    const response = await this.env.MATCH_CAPACITY_API.fetch(
      new Request("https://match-capacity.internal/v1/reservations", {
        method: "POST",
        headers: {
          authorization: `Bearer ${serviceTicket}`,
          "content-type": "application/json",
        },
        body: JSON.stringify(request),
      }),
    );
    if (!response.ok) {
      throw new ApiError(
        response.status >= 500 ? 503 : 409,
        "capacity_reservation_failed",
        "No healthy match host accepted the reservation.",
      );
    }
    const capacity = this.capacityResponse(await readJson<unknown>(response), request);
    const allocation: MatchAllocation = {
      matchId: request.matchId,
      hostId: capacity.hostId,
      region: request.region,
      playlist: request.playlist,
      inputPool: request.inputPool,
      transport: capacity.transport,
      buildHash: request.buildHash,
      expiresAtMs: capacity.expiresAtMs,
      ...(capacity.nativeEndpoint === undefined ? {} : { nativeEndpoint: capacity.nativeEndpoint }),
    };

    try {
      await this.env.CONTROL_DB.prepare(
        `INSERT INTO match_allocations(
           match_id, dispatch_hash, roster_hash, host_id, region, playlist, input_pool, transport,
           native_endpoint, build_hash, status, player_count, bot_count,
           created_at_ms, expires_at_ms
         ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'reserved', ?, ?, ?, ?)`,
      )
        .bind(
          allocation.matchId,
          request.dispatchHash,
          request.rosterHash,
          allocation.hostId,
          allocation.region,
          allocation.playlist,
          allocation.inputPool,
          allocation.transport,
          allocation.nativeEndpoint ?? null,
          allocation.buildHash,
          request.playerCount,
          request.botCount,
          Date.now(),
          allocation.expiresAtMs,
        )
        .run();
    } catch (error) {
      let winner: AllocationRow | null | undefined;
      try {
        winner = await this.allocationRow(allocation.matchId);
      } catch (lookupError) {
        console.error("allocation persistence outcome is unknown; capacity left to expire", {
          lookupError,
        });
      }
      if (winner !== undefined && winner?.host_id !== allocation.hostId) {
        await this.releaseCapacityOnly(
          allocation.matchId,
          allocation.hostId,
          "control-plane-persistence-failed",
        ).catch((releaseError) => console.error("capacity release failed", { releaseError }));
      }
      throw error;
    }
    return allocation;
  }

  async stageQueueConfirmation(
    matchIdRaw: string,
    hostIdRaw: string,
    dispatchHashRaw: string,
    rosterHashRaw: string,
    confirmation: DispatchConfirmation,
  ): Promise<ResumableDispatch> {
    const matchId = requireIdentifier(matchIdRaw, "matchId");
    const hostId = requireIdentifier(hostIdRaw, "hostId");
    const dispatchHash = requireIdentifier(dispatchHashRaw, "dispatchHash");
    const rosterHash = requireIdentifier(rosterHashRaw, "rosterHash");
    const shard = requireIdentifier(confirmation.shard, "matchmaking shard");
    const leaseId = requireIdentifier(confirmation.leaseId, "leaseId");
    const assignmentsJson = canonicalJson(confirmation.assignments);
    const reservationsJson = canonicalJson(confirmation.reservations);
    if (assignmentsJson.length + reservationsJson.length > 256 * 1024) {
      throw new ApiError(400, "assignments_too_large", "Match recovery journal exceeds its limit.");
    }
    await this.env.CONTROL_DB.prepare(
      `UPDATE match_allocations
       SET matchmaking_shard = ?, lease_id = ?, assignments_json = ?, reservations_json = ?
       WHERE match_id = ? AND host_id = ? AND dispatch_hash = ? AND roster_hash = ?
         AND status = 'reserved' AND matchmaking_shard IS NULL AND lease_id IS NULL
         AND assignments_json IS NULL AND reservations_json IS NULL AND expires_at_ms > ?`,
    )
      .bind(
        shard,
        leaseId,
        assignmentsJson,
        reservationsJson,
        matchId,
        hostId,
        dispatchHash,
        rosterHash,
        Date.now(),
      )
      .run();
    const staged = await this.allocationRow(matchId);
    const journalConflict =
      staged === null ||
      !["reserved", "active", "aborting"].includes(staged.status) ||
      staged.host_id !== hostId ||
      staged.dispatch_hash !== dispatchHash ||
      staged.roster_hash !== rosterHash ||
      staged.matchmaking_shard !== shard ||
      staged.lease_id !== leaseId ||
      staged.assignments_json !== assignmentsJson ||
      staged.reservations_json !== reservationsJson;
    if (journalConflict) {
      if (
        staged === null ||
        (staged.matchmaking_shard === null &&
          staged.lease_id === null &&
          staged.assignments_json === null &&
          staged.reservations_json === null)
      ) {
        throw new ApiError(
          409,
          "allocation_not_stageable",
          "Match allocation expired or changed before its recovery journal was stored.",
        );
      }
      throw new ApiError(
        409,
        "dispatch_journal_conflict",
        "Match recovery journal conflicts with the allocation.",
      );
    }
    const resumed = await this.resumeDispatch(matchId, dispatchHash);
    if (resumed === null) throw new Error(`Staged match ${matchId} disappeared.`);
    return resumed;
  }

  async activate(
    matchIdRaw: string,
    hostIdRaw: string,
    dispatchHashRaw: string,
    rosterHashRaw: string,
  ): Promise<void> {
    const matchId = requireIdentifier(matchIdRaw, "matchId");
    const hostId = requireIdentifier(hostIdRaw, "hostId");
    const dispatchHash = requireIdentifier(dispatchHashRaw, "dispatchHash");
    const rosterHash = requireIdentifier(rosterHashRaw, "rosterHash");
    const staged = await this.allocationRow(matchId);
    if (
      staged === null ||
      staged.status !== "reserved" ||
      staged.host_id !== hostId ||
      staged.dispatch_hash !== dispatchHash ||
      staged.roster_hash !== rosterHash ||
      staged.assignments_json === null
    ) {
      throw new ApiError(
        409,
        "allocation_activation_conflict",
        "Match allocation is not staged for this dispatch roster.",
      );
    }
    let assignmentExpiresAtMs: number;
    try {
      const assignments = JSON.parse(staged.assignments_json) as Record<string, MatchAssignment>;
      assignmentExpiresAtMs = Math.min(
        ...Object.values(assignments).map((assignment) => assignment.expiresAtMs),
      );
    } catch {
      throw new Error(`Staged match ${matchId} has an invalid assignment journal.`);
    }
    const activationCutoffMs = Date.now() + 5_000;
    if (
      !Number.isSafeInteger(assignmentExpiresAtMs) ||
      assignmentExpiresAtMs <= activationCutoffMs
    ) {
      throw new ApiError(
        409,
        "assignment_expired",
        "Match assignments expire too soon to activate this allocation.",
      );
    }
    await this.env.CONTROL_DB.prepare(
      `UPDATE match_allocations
       SET status = 'active'
       WHERE match_id = ? AND host_id = ? AND dispatch_hash = ? AND roster_hash = ?
         AND status = 'reserved' AND matchmaking_shard IS NOT NULL AND lease_id IS NOT NULL
         AND assignments_json IS NOT NULL AND reservations_json IS NOT NULL
         AND expires_at_ms > ?`,
    )
      .bind(matchId, hostId, dispatchHash, rosterHash, activationCutoffMs)
      .run();
    const active = await this.allocationRow(matchId);
    if (
      active === null ||
      active.status !== "active" ||
      active.host_id !== hostId ||
      active.dispatch_hash !== dispatchHash ||
      active.roster_hash !== rosterHash ||
      active.matchmaking_shard === null ||
      active.lease_id === null ||
      active.assignments_json === null ||
      active.reservations_json === null ||
      active.expires_at_ms <= activationCutoffMs
    ) {
      throw new ApiError(
        409,
        "allocation_activation_conflict",
        "Match allocation could not be activated for this dispatch roster.",
      );
    }
  }

  async markQueueConfirmed(
    matchIdRaw: string,
    hostIdRaw: string,
    dispatchHashRaw: string,
    leaseIdRaw: string,
  ): Promise<void> {
    const matchId = requireIdentifier(matchIdRaw, "matchId");
    const hostId = requireIdentifier(hostIdRaw, "hostId");
    const dispatchHash = requireIdentifier(dispatchHashRaw, "dispatchHash");
    const leaseId = requireIdentifier(leaseIdRaw, "leaseId");
    await this.env.CONTROL_DB.prepare(
      `UPDATE match_allocations
       SET queue_confirmed_at_ms = coalesce(queue_confirmed_at_ms, ?)
       WHERE match_id = ? AND host_id = ? AND dispatch_hash = ? AND lease_id = ?
         AND status = 'active'`,
    )
      .bind(Date.now(), matchId, hostId, dispatchHash, leaseId)
      .run();
    const active = await this.allocationRow(matchId);
    if (
      active === null ||
      active.status !== "active" ||
      active.host_id !== hostId ||
      active.dispatch_hash !== dispatchHash ||
      active.lease_id !== leaseId ||
      active.queue_confirmed_at_ms === null
    ) {
      throw new ApiError(
        409,
        "queue_confirmation_conflict",
        "Queue confirmation could not be recorded for this dispatch.",
      );
    }
  }

  async beginAbort(
    matchIdRaw: string,
    hostIdRaw: string,
    dispatchHashRaw: string,
    rosterHashRaw: string,
  ): Promise<boolean> {
    const matchId = requireIdentifier(matchIdRaw, "matchId");
    const hostId = requireIdentifier(hostIdRaw, "hostId");
    const dispatchHash = requireIdentifier(dispatchHashRaw, "dispatchHash");
    const rosterHash = requireIdentifier(rosterHashRaw, "rosterHash");
    await this.env.CONTROL_DB.prepare(
      `UPDATE match_allocations
       SET status = 'aborting'
       WHERE match_id = ? AND host_id = ? AND dispatch_hash = ? AND roster_hash = ?
         AND status IN ('reserved', 'active') AND queue_confirmed_at_ms IS NULL
         AND matchmaking_shard IS NOT NULL AND lease_id IS NOT NULL
         AND assignments_json IS NOT NULL AND reservations_json IS NOT NULL`,
    )
      .bind(matchId, hostId, dispatchHash, rosterHash)
      .run();
    const row = await this.allocationRow(matchId);
    if (
      row?.status === "aborting" &&
      row.host_id === hostId &&
      row.dispatch_hash === dispatchHash &&
      row.roster_hash === rosterHash
    ) {
      return true;
    }
    if (
      row?.status === "active" &&
      row.host_id === hostId &&
      row.dispatch_hash === dispatchHash &&
      row.roster_hash === rosterHash &&
      row.queue_confirmed_at_ms !== null
    ) {
      return false;
    }
    throw new ApiError(
      409,
      "dispatch_abort_conflict",
      "Dispatch could not enter its compensating state.",
    );
  }

  async restoreActiveAfterConfirmedAbort(
    matchIdRaw: string,
    hostIdRaw: string,
    dispatchHashRaw: string,
    rosterHashRaw: string,
  ): Promise<void> {
    const matchId = requireIdentifier(matchIdRaw, "matchId");
    const hostId = requireIdentifier(hostIdRaw, "hostId");
    const dispatchHash = requireIdentifier(dispatchHashRaw, "dispatchHash");
    const rosterHash = requireIdentifier(rosterHashRaw, "rosterHash");
    await this.env.CONTROL_DB.prepare(
      `UPDATE match_allocations
       SET status = 'active'
       WHERE match_id = ? AND host_id = ? AND dispatch_hash = ? AND roster_hash = ?
         AND status = 'aborting'`,
    )
      .bind(matchId, hostId, dispatchHash, rosterHash)
      .run();
    const row = await this.allocationRow(matchId);
    if (
      row === null ||
      row.status !== "active" ||
      row.host_id !== hostId ||
      row.dispatch_hash !== dispatchHash ||
      row.roster_hash !== rosterHash
    ) {
      throw new ApiError(
        409,
        "dispatch_restore_conflict",
        "Confirmed queue receipt could not restore the active allocation.",
      );
    }
  }

  async recordExpiredConfirmedAbort(
    matchIdRaw: string,
    hostIdRaw: string,
    dispatchHashRaw: string,
    rosterHashRaw: string,
    leaseIdRaw: string,
  ): Promise<void> {
    const matchId = requireIdentifier(matchIdRaw, "matchId");
    const hostId = requireIdentifier(hostIdRaw, "hostId");
    const dispatchHash = requireIdentifier(dispatchHashRaw, "dispatchHash");
    const rosterHash = requireIdentifier(rosterHashRaw, "rosterHash");
    const leaseId = requireIdentifier(leaseIdRaw, "leaseId");
    const now = Date.now();
    const allocationCutoff = now + 5_000;
    await this.env.CONTROL_DB.prepare(
      `UPDATE match_allocations
       SET status = CASE WHEN expires_at_ms <= ? THEN 'complete' ELSE 'draining' END,
           queue_confirmed_at_ms = coalesce(queue_confirmed_at_ms, ?),
           released_at_ms = CASE
             WHEN expires_at_ms <= ? THEN coalesce(released_at_ms, ?)
             ELSE released_at_ms
           END,
           release_reason = coalesce(release_reason, 'confirmed-observed-after-ticket-expiry')
       WHERE match_id = ? AND host_id = ? AND dispatch_hash = ? AND roster_hash = ?
         AND lease_id = ? AND status = 'aborting'`,
    )
      .bind(
        allocationCutoff,
        now,
        allocationCutoff,
        now,
        matchId,
        hostId,
        dispatchHash,
        rosterHash,
        leaseId,
      )
      .run();
    const row = await this.allocationRow(matchId);
    if (
      row === null ||
      !["draining", "complete"].includes(row.status) ||
      row.host_id !== hostId ||
      row.dispatch_hash !== dispatchHash ||
      row.roster_hash !== rosterHash ||
      row.lease_id !== leaseId ||
      row.queue_confirmed_at_ms === null
    ) {
      throw new ApiError(
        409,
        "expired_confirmation_record_conflict",
        "Expired confirmed allocation could not be recorded safely.",
      );
    }
  }

  async release(matchIdRaw: string, hostIdRaw: string, reasonRaw: string): Promise<void> {
    const matchId = requireIdentifier(matchIdRaw, "matchId");
    const hostId = requireIdentifier(hostIdRaw, "hostId");
    const reason = requireIdentifier(reasonRaw, "reason");
    await this.releaseCapacityOnly(matchId, hostId, reason);
    await this.env.CONTROL_DB.prepare(
      `UPDATE match_allocations
       SET status = 'released', released_at_ms = ?, release_reason = ?
       WHERE match_id = ? AND host_id = ? AND status IN ('reserved', 'active', 'aborting')`,
    )
      .bind(Date.now(), reason, matchId, hostId)
      .run();
  }

  async browserAllocation(matchIdRaw: string): Promise<MatchAllocation> {
    const matchId = requireIdentifier(matchIdRaw, "matchId");
    const row = await this.env.CONTROL_DB.prepare(
      `SELECT match_id, dispatch_hash, roster_hash, host_id, region, playlist, input_pool, transport,
              native_endpoint, build_hash, expires_at_ms, status, player_count, bot_count,
              matchmaking_shard, lease_id, assignments_json, reservations_json,
              queue_confirmed_at_ms
       FROM match_allocations
       WHERE match_id = ? AND status = 'active' AND queue_confirmed_at_ms IS NOT NULL
         AND expires_at_ms > ?`,
    )
      .bind(matchId, Date.now())
      .first<AllocationRow>();
    if (row === null) throw new ApiError(404, "match_not_found", "Match allocation was not found.");
    if (row.transport !== "wss") {
      throw new ApiError(409, "wrong_match_transport", "Match does not accept browser WebSockets.");
    }
    return this.presentAllocation(row);
  }

  private capacityResponse(value: unknown, request: ReserveMatchRequest): CapacityResponse {
    assertObject(value, "capacity response");
    const transport = value.transport;
    if (transport !== "wss" && transport !== "quic") {
      throw new ApiError(502, "invalid_capacity_response", "Capacity service returned an invalid transport.");
    }
    const expectedTransport: MatchTransport =
      request.inputPool === "browser" || request.playlist === "casual-extraction" ? "wss" : "quic";
    if (transport !== expectedTransport) {
      throw new ApiError(502, "invalid_capacity_response", "Capacity service returned the wrong transport.");
    }
    const expiresAtMs = requireInteger(
      value.expiresAtMs,
      "capacity expiresAtMs",
      Date.now() + 30_000,
      Date.now() + 2 * 60 * 60 * 1_000,
    );
    const response: CapacityResponse = {
      hostId: requireIdentifier(value.hostId, "capacity hostId"),
      transport,
      expiresAtMs,
    };
    if (transport === "quic") {
      const endpoint = requireString(value.nativeEndpoint, "capacity nativeEndpoint", 255);
      if (!/^[A-Za-z0-9.-]+:[1-9][0-9]{0,4}$/u.test(endpoint)) {
        throw new ApiError(502, "invalid_capacity_response", "QUIC endpoint must be host:port.");
      }
      response.nativeEndpoint = endpoint;
    }
    return response;
  }

  private async releaseCapacityOnly(matchId: string, hostId: string, reason: string): Promise<void> {
    if (this.env.MATCH_CAPACITY_API === undefined) return;
    const serviceTicket = await this.serviceTicket("match-director", ["capacity:release"], {
      match_id: matchId,
    });
    const response = await this.env.MATCH_CAPACITY_API.fetch(
      new Request(`https://match-capacity.internal/v1/reservations/${encodeURIComponent(matchId)}`, {
        method: "DELETE",
        headers: {
          authorization: `Bearer ${serviceTicket}`,
          "content-type": "application/json",
        },
        body: JSON.stringify({ hostId, reason }),
      }),
    );
    if (!response.ok && response.status !== 404) {
      throw new Error(`Capacity release failed with status ${response.status}.`);
    }
  }

  private presentAllocation(row: AllocationRow): MatchAllocation {
    return {
      matchId: row.match_id,
      hostId: row.host_id,
      region: row.region,
      playlist: row.playlist,
      inputPool: row.input_pool,
      transport: row.transport,
      buildHash: row.build_hash,
      expiresAtMs: row.expires_at_ms,
      ...(row.native_endpoint === null ? {} : { nativeEndpoint: row.native_endpoint }),
    };
  }

  private allocationRow(matchId: string): Promise<AllocationRow | null> {
    return this.env.CONTROL_DB.prepare(
      `SELECT match_id, dispatch_hash, roster_hash, host_id, region, playlist, input_pool,
              transport, native_endpoint, build_hash, expires_at_ms, status,
              player_count, bot_count, matchmaking_shard, lease_id, assignments_json,
              reservations_json, queue_confirmed_at_ms
       FROM match_allocations WHERE match_id = ?`,
    )
      .bind(matchId)
      .first<AllocationRow>();
  }

  private async serviceTicket(
    subject: string,
    scopes: string[],
    additional: Partial<TicketClaims>,
  ): Promise<string> {
    const now = Math.floor(Date.now() / 1_000);
    return signTicket(
      {
        v: 1,
        iss: this.env.TICKET_ISSUER,
        aud: this.env.SERVICE_TICKET_AUDIENCE,
        purpose: "service",
        sub: subject,
        iat: now,
        nbf: now - 2,
        exp: now + 60,
        nonce: randomToken("svc"),
        scopes,
        ...additional,
      },
      this.env.SERVICE_TICKET_KEYS_JSON,
      this.env.ACTIVE_SERVICE_TICKET_KID,
    );
  }
}
