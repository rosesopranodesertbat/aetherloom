import { DurableObject } from "cloudflare:workers";
import {
  ISLAND_IDLE_AFTER_MS,
  ISLAND_TICK_HZ,
  MAX_ISLAND_COMMAND_BYTES,
  MAX_RETAINED_ISLAND_BACKLOG_TICKS,
  commitAdvancedSimulation,
  createIslandSimulation,
  planIslandTickBatch,
  type HeadlessIslandSimulation,
} from "./island-runtime";
import { islandSimulationModule } from "./island-module";
import { checkpointObjectKey } from "./dispatch-invariants";
import type {
  CommitReservationCommand,
  Env,
  IslandCheckpointCommand,
  InventoryStack,
  QueuedSettlement,
  ReservationCommand,
  RollbackReservationCommand,
} from "./types";
import {
  ApiError,
  base64UrlToBytes,
  bytesToBase64Url,
  canonicalJson,
  errorResponse,
  inventoryStacks,
  jsonResponse,
  readJson,
  requireIdentifier,
  requireInteger,
  requireString,
  sha256Base64Url,
} from "./util";

interface InventoryRow extends Record<string, SqlStorageValue> {
  item_id: string;
  quantity: number;
  reserved: number;
}

interface ReservationRow extends Record<string, SqlStorageValue> {
  reservation_id: string;
  request_hash: string;
  status: "reserved" | "committed" | "expired" | "cancelled" | "settled";
  loadout_json: string;
  response_json: string;
  expires_at_ms: number;
  match_id: string | null;
  settlement_id: string | null;
}

interface SettlementRow extends Record<string, SqlStorageValue> {
  payload_hash: string;
  response_json: string;
}

interface IslandRow extends Record<string, SqlStorageValue> {
  checkpoint_id: string;
  version: number;
  logical_time_ms: number;
  content_build: string;
  object_key: string;
  sha256: string;
  size_bytes: number;
  etag: string;
  updated_at_ms: number;
}

interface IslandRuntimeRow extends Record<string, SqlStorageValue> {
  logical_tick: number;
  last_wall_clock_ms: number;
  tick_remainder: number;
  backlog_ticks: number;
  active_until_ms: number | null;
  last_sequence: number;
  checkpoint_version: number;
  dirty_batches: number;
}

interface IslandRuntimeReceipt extends Record<string, SqlStorageValue> {
  payload_hash: string;
  response_json: string;
}

interface IslandRuntimeCommand {
  accountId: string;
  sequence: number;
  commandsBase64: string;
}

interface IslandJournalRow extends Record<string, SqlStorageValue> {
  ticks: number;
  commands_blob: ArrayBuffer;
}

function firstRow<T>(rows: T[]): T | undefined {
  return rows[0];
}

export class ProfileIslandObject extends DurableObject<Env> {
  private readonly bindings: Env;
  private readonly simulation: HeadlessIslandSimulation;
  private runtimeLoaded = false;
  private runtimeTail: Promise<void> = Promise.resolve();

  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    this.bindings = env;
    this.simulation = createIslandSimulation(islandSimulationModule);
    this.ctx.storage.transactionSync(() => {
      this.ctx.storage.sql.exec(`
        CREATE TABLE IF NOT EXISTS profile (
          singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
          account_id TEXT NOT NULL UNIQUE,
          rating INTEGER NOT NULL DEFAULT 1000 CHECK (rating >= 0 AND rating <= 5000),
          profile_version INTEGER NOT NULL DEFAULT 1 CHECK (profile_version >= 1),
          created_at_ms INTEGER NOT NULL,
          updated_at_ms INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS inventory (
          item_id TEXT PRIMARY KEY,
          quantity INTEGER NOT NULL DEFAULT 0 CHECK (quantity >= 0),
          reserved INTEGER NOT NULL DEFAULT 0 CHECK (reserved >= 0 AND reserved <= quantity),
          updated_at_ms INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS reservations (
          reservation_id TEXT PRIMARY KEY,
          request_hash TEXT NOT NULL,
          status TEXT NOT NULL CHECK (status IN ('reserved', 'committed', 'expired', 'cancelled', 'settled')),
          loadout_json TEXT NOT NULL,
          response_json TEXT NOT NULL,
          expires_at_ms INTEGER NOT NULL,
          match_id TEXT,
          settlement_id TEXT UNIQUE,
          created_at_ms INTEGER NOT NULL,
          updated_at_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS reservations_expiry
          ON reservations(status, expires_at_ms);

        CREATE TABLE IF NOT EXISTS reservation_match_aborts (
          reservation_id TEXT NOT NULL,
          match_id TEXT NOT NULL,
          aborted_at_ms INTEGER NOT NULL,
          PRIMARY KEY (reservation_id, match_id)
        );

        CREATE TABLE IF NOT EXISTS settlements (
          settlement_id TEXT PRIMARY KEY,
          payload_hash TEXT NOT NULL,
          reservation_id TEXT NOT NULL UNIQUE,
          match_id TEXT NOT NULL,
          outcome TEXT NOT NULL CHECK (outcome IN ('extracted', 'defeated', 'abandoned')),
          response_json TEXT NOT NULL,
          applied_at_ms INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS island_checkpoints (
          checkpoint_id TEXT PRIMARY KEY,
          version INTEGER NOT NULL UNIQUE CHECK (version >= 1),
          logical_time_ms INTEGER NOT NULL CHECK (logical_time_ms >= 0),
          content_build TEXT NOT NULL,
          object_key TEXT NOT NULL UNIQUE,
          sha256 TEXT NOT NULL,
          size_bytes INTEGER NOT NULL CHECK (size_bytes > 0),
          etag TEXT NOT NULL,
          updated_at_ms INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS island_head (
          singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
          checkpoint_id TEXT NOT NULL,
          version INTEGER NOT NULL,
          FOREIGN KEY (checkpoint_id) REFERENCES island_checkpoints(checkpoint_id)
        );

        CREATE TABLE IF NOT EXISTS island_runtime (
          singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
          logical_tick INTEGER NOT NULL DEFAULT 0 CHECK (logical_tick >= 0),
          last_wall_clock_ms INTEGER NOT NULL DEFAULT 0,
          tick_remainder INTEGER NOT NULL DEFAULT 0 CHECK (tick_remainder BETWEEN 0 AND 999),
          backlog_ticks INTEGER NOT NULL DEFAULT 0
            CHECK (backlog_ticks BETWEEN 0 AND ${MAX_RETAINED_ISLAND_BACKLOG_TICKS}),
          active_until_ms INTEGER,
          last_sequence INTEGER NOT NULL DEFAULT 0 CHECK (last_sequence >= 0),
          checkpoint_version INTEGER NOT NULL DEFAULT 0 CHECK (checkpoint_version >= 0),
          dirty_batches INTEGER NOT NULL DEFAULT 0 CHECK (dirty_batches >= 0)
        );
        INSERT OR IGNORE INTO island_runtime(singleton) VALUES(1);

        CREATE TABLE IF NOT EXISTS island_command_journal (
          sequence INTEGER PRIMARY KEY,
          tick_start INTEGER NOT NULL,
          tick_end INTEGER NOT NULL,
          ticks INTEGER NOT NULL CHECK (ticks BETWEEN 1 AND 32),
          commands_blob BLOB NOT NULL,
          created_at_ms INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS island_command_receipts (
          sequence INTEGER PRIMARY KEY,
          payload_hash TEXT NOT NULL,
          response_json TEXT NOT NULL,
          processed_at_ms INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS island_jobs (
          job_id TEXT PRIMARY KEY,
          job_kind TEXT NOT NULL,
          state TEXT NOT NULL CHECK (state IN ('pending', 'ready', 'claimed', 'cancelled')),
          payload_json TEXT NOT NULL,
          started_at_ms INTEGER NOT NULL,
          completes_at_ms INTEGER NOT NULL,
          updated_at_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS island_jobs_completion
          ON island_jobs(state, completes_at_ms);
      `);
      const runtimeColumns = this.ctx.storage.sql
        .exec<{ name: string }>("PRAGMA table_info(island_runtime)")
        .toArray();
      if (!runtimeColumns.some((column) => column.name === "backlog_ticks")) {
        this.ctx.storage.sql.exec(
          `ALTER TABLE island_runtime
           ADD COLUMN backlog_ticks INTEGER NOT NULL DEFAULT 0
             CHECK (backlog_ticks BETWEEN 0 AND ${MAX_RETAINED_ISLAND_BACKLOG_TICKS})`,
        );
      }
    });
  }

  override async fetch(request: Request): Promise<Response> {
    try {
      const url = new URL(request.url);
      if (request.method === "GET" && url.pathname === "/profile") {
        const accountId = this.accountFromRequest(request);
        this.ensureAccount(accountId);
        return jsonResponse(this.profileView(accountId));
      }
      if (request.method === "POST" && url.pathname === "/reservations") {
        const command = await readJson<ReservationCommand>(request);
        return await this.reserve(command);
      }
      if (request.method === "POST" && url.pathname === "/reservations/commit") {
        const command = await readJson<CommitReservationCommand>(request);
        return this.commitReservation(command);
      }
      if (request.method === "POST" && url.pathname === "/reservations/rollback") {
        const command = await readJson<RollbackReservationCommand>(request);
        return await this.rollbackReservation(command);
      }
      if (request.method === "POST" && url.pathname === "/reservations/cancel") {
        const command = await readJson<Pick<ReservationCommand, "accountId" | "reservationId">>(request);
        return await this.cancelReservation(command.accountId, command.reservationId);
      }
      if (request.method === "POST" && url.pathname === "/settlements") {
        return this.applySettlement(await readJson<QueuedSettlement>(request));
      }
      if (request.method === "GET" && url.pathname === "/island") {
        const accountId = this.accountFromRequest(request);
        this.ensureAccount(accountId);
        return jsonResponse({ accountId, checkpoint: this.islandHead() });
      }
      if (request.method === "PUT" && url.pathname === "/island/checkpoints") {
        return this.updateIsland(await readJson<IslandCheckpointCommand>(request));
      }
      if (request.method === "GET" && url.pathname === "/island/runtime") {
        const accountId = this.accountFromRequest(request);
        this.ensureAccount(accountId);
        this.settleDormantJobs(Date.now());
        return jsonResponse(this.runtimeStatus(accountId));
      }
      if (request.method === "POST" && url.pathname === "/island/runtime/commands") {
        const command = await readJson<IslandRuntimeCommand>(request, 16 * 1024);
        return await this.withRuntimeLock(() => this.advanceIslandRuntime(command));
      }
      if (request.method === "POST" && url.pathname === "/island/runtime/deactivate") {
        const accountId = this.accountFromRequest(request);
        this.ensureAccount(accountId);
        return await this.withRuntimeLock(async () => {
          await this.checkpointRuntime(true);
          await this.scheduleAlarm();
          return jsonResponse(this.runtimeStatus(accountId));
        });
      }
      return jsonResponse({ error: { code: "not_found", message: "Profile operation not found." } }, 404);
    } catch (error) {
      return errorResponse(error, "profile-island-do");
    }
  }

  override async alarm(): Promise<void> {
    const now = Date.now();
    this.expireReservations(now);
    this.settleDormantJobs(now);
    await this.withRuntimeLock(async () => {
      const runtime = this.runtimeRow();
      if (runtime.active_until_ms !== null && runtime.active_until_ms <= now) {
        await this.checkpointRuntime(true);
      }
    });
    await this.scheduleAlarm();
  }

  private accountFromRequest(request: Request): string {
    return requireIdentifier(request.headers.get("x-aetherloom-account-id"), "account id");
  }

  private ensureAccount(accountId: string): void {
    requireIdentifier(accountId, "accountId");
    const existing = firstRow(
      this.ctx.storage.sql
        .exec<{ account_id: string }>("SELECT account_id FROM profile WHERE singleton = 1")
        .toArray(),
    );
    if (existing === undefined) {
      const now = Date.now();
      this.ctx.storage.sql.exec(
        "INSERT INTO profile(singleton, account_id, created_at_ms, updated_at_ms) VALUES(1, ?, ?, ?)",
        accountId,
        now,
        now,
      );
      return;
    }
    if (existing.account_id !== accountId) {
      throw new ApiError(403, "profile_key_mismatch", "Profile object does not belong to this account.");
    }
  }

  private profileView(accountId: string): unknown {
    const profile = firstRow(
      this.ctx.storage.sql
        .exec<{ rating: number; profile_version: number; created_at_ms: number; updated_at_ms: number }>(
          "SELECT rating, profile_version, created_at_ms, updated_at_ms FROM profile WHERE singleton = 1",
        )
        .toArray(),
    );
    if (profile === undefined) throw new Error("Profile row is missing after initialization.");
    const inventory = this.ctx.storage.sql
      .exec<InventoryRow>(
        "SELECT item_id, quantity, reserved FROM inventory WHERE quantity > 0 ORDER BY item_id",
      )
      .toArray()
      .map((row) => ({
        itemId: row.item_id,
        quantity: row.quantity,
        available: row.quantity - row.reserved,
        reserved: row.reserved,
      }));
    return {
      accountId,
      rating: profile.rating,
      version: profile.profile_version,
      inventory,
      island: this.islandHead(),
      createdAtMs: profile.created_at_ms,
      updatedAtMs: profile.updated_at_ms,
    };
  }

  private async reserve(command: ReservationCommand): Promise<Response> {
    const accountId = requireIdentifier(command.accountId, "accountId");
    const reservationId = requireIdentifier(command.reservationId, "reservationId");
    const loadout = inventoryStacks(command.loadout, "loadout");
    const expiresAtMs = requireInteger(
      command.expiresAtMs,
      "expiresAtMs",
      Date.now() + 30_000,
      Date.now() + 3_600_000,
    );
    const requestHash = await crypto.subtle
      .digest("SHA-256", new TextEncoder().encode(canonicalJson({ accountId, reservationId, loadout })))
      .then((digest) =>
        Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join(""),
      );

    const result = this.ctx.storage.transactionSync(() => {
      this.ensureAccount(accountId);
      const existing = firstRow(
        this.ctx.storage.sql
          .exec<ReservationRow>(
            `SELECT reservation_id, request_hash, status, loadout_json, response_json,
                    expires_at_ms, match_id, settlement_id
             FROM reservations WHERE reservation_id = ?`,
            reservationId,
          )
          .toArray(),
      );
      if (existing !== undefined) {
        if (existing.request_hash !== requestHash) {
          throw new ApiError(
            409,
            "idempotency_conflict",
            "Reservation id was already used with a different loadout.",
          );
        }
        return {
          status: 200,
          body: {
            reservationId: existing.reservation_id,
            status: existing.status,
            loadout: JSON.parse(existing.loadout_json) as unknown,
            expiresAtMs: existing.expires_at_ms,
            matchId: existing.match_id,
            settlementId: existing.settlement_id,
          },
        };
      }

      for (const stack of loadout) {
        const inventory = firstRow(
          this.ctx.storage.sql
            .exec<InventoryRow>(
              "SELECT item_id, quantity, reserved FROM inventory WHERE item_id = ?",
              stack.itemId,
            )
            .toArray(),
        );
        const available = inventory === undefined ? 0 : inventory.quantity - inventory.reserved;
        if (available < stack.quantity) {
          throw new ApiError(409, "insufficient_inventory", `Not enough available ${stack.itemId}.`);
        }
      }

      const now = Date.now();
      for (const stack of loadout) {
        this.ctx.storage.sql.exec(
          "UPDATE inventory SET reserved = reserved + ?, updated_at_ms = ? WHERE item_id = ?",
          stack.quantity,
          now,
          stack.itemId,
        );
      }
      const body = {
        reservationId,
        status: "reserved",
        loadout,
        expiresAtMs,
      };
      this.ctx.storage.sql.exec(
        `INSERT INTO reservations(
           reservation_id, request_hash, status, loadout_json, response_json,
           expires_at_ms, created_at_ms, updated_at_ms
         ) VALUES(?, ?, 'reserved', ?, ?, ?, ?, ?)`,
        reservationId,
        requestHash,
        JSON.stringify(loadout),
        JSON.stringify(body),
        expiresAtMs,
        now,
        now,
      );
      this.bumpProfile(now);
      return { status: 201, body };
    });

    await this.scheduleAlarm();
    return jsonResponse(result.body, result.status);
  }

  private commitReservation(command: CommitReservationCommand): Response {
    const accountId = requireIdentifier(command.accountId, "accountId");
    const reservationId = requireIdentifier(command.reservationId, "reservationId");
    const matchId = requireIdentifier(command.matchId, "matchId");
    const body = this.ctx.storage.transactionSync(() => {
      this.ensureAccount(accountId);
      const aborted = firstRow(
        this.ctx.storage.sql
          .exec<{ aborted: number }>(
            `SELECT 1 AS aborted FROM reservation_match_aborts
             WHERE reservation_id = ? AND match_id = ?`,
            reservationId,
            matchId,
          )
          .toArray(),
      );
      if (aborted !== undefined) {
        throw new ApiError(
          409,
          "reservation_match_aborted",
          "Reservation was terminally compensated for this match.",
        );
      }
      const reservation = this.reservation(reservationId);
      if (reservation.status === "committed" && reservation.match_id === matchId) {
        return { reservationId, matchId, status: "committed" };
      }
      if (reservation.status !== "reserved") {
        throw new ApiError(409, "reservation_not_reservable", "Reservation is no longer available.");
      }
      if (reservation.expires_at_ms <= Date.now()) {
        this.releaseReservationInventory(reservation, "expired", Date.now());
        throw new ApiError(409, "reservation_expired", "Reservation expired before match allocation.");
      }
      const now = Date.now();
      this.ctx.storage.sql.exec(
        `UPDATE reservations
         SET status = 'committed', match_id = ?, updated_at_ms = ?
         WHERE reservation_id = ?`,
        matchId,
        now,
        reservationId,
      );
      this.bumpProfile(now);
      return { reservationId, matchId, status: "committed" };
    });
    return jsonResponse(body);
  }

  private async rollbackReservation(command: RollbackReservationCommand): Promise<Response> {
    const accountId = requireIdentifier(command.accountId, "accountId");
    const reservationId = requireIdentifier(command.reservationId, "reservationId");
    const matchId = requireIdentifier(command.matchId, "matchId");
    const expiresAtMs = requireInteger(
      command.expiresAtMs,
      "expiresAtMs",
      Date.now() + 30_000,
      Date.now() + 600_000,
    );
    const body = this.ctx.storage.transactionSync(() => {
      this.ensureAccount(accountId);
      const reservation = this.reservationOrUndefined(reservationId);
      this.ctx.storage.sql.exec(
        `INSERT INTO reservation_match_aborts(reservation_id, match_id, aborted_at_ms)
         VALUES(?, ?, ?)
         ON CONFLICT(reservation_id, match_id) DO NOTHING`,
        reservationId,
        matchId,
        Date.now(),
      );
      if (reservation === undefined) {
        return { reservationId, status: "absent" };
      }
      if (reservation.status === "reserved") {
        return { reservationId, status: "reserved", expiresAtMs: reservation.expires_at_ms };
      }
      if (reservation.status === "expired" || reservation.status === "cancelled") {
        return { reservationId, status: reservation.status };
      }
      if (reservation.status !== "committed" || reservation.match_id !== matchId) {
        throw new ApiError(409, "reservation_rollback_conflict", "Committed reservation does not match.");
      }
      const now = Date.now();
      this.ctx.storage.sql.exec(
        `UPDATE reservations
         SET status = 'reserved', match_id = NULL, expires_at_ms = ?, updated_at_ms = ?
         WHERE reservation_id = ?`,
        expiresAtMs,
        now,
        reservationId,
      );
      this.bumpProfile(now);
      return { reservationId, status: "reserved", expiresAtMs };
    });
    await this.scheduleAlarm();
    return jsonResponse(body);
  }

  private async cancelReservation(accountIdRaw: string, reservationIdRaw: string): Promise<Response> {
    const accountId = requireIdentifier(accountIdRaw, "accountId");
    const reservationId = requireIdentifier(reservationIdRaw, "reservationId");
    const body = this.ctx.storage.transactionSync(() => {
      this.ensureAccount(accountId);
      const reservation = this.reservation(reservationId);
      if (reservation.status === "cancelled" || reservation.status === "expired") {
        return { reservationId, status: reservation.status };
      }
      if (reservation.status !== "reserved") {
        throw new ApiError(409, "reservation_in_match", "A committed reservation cannot be cancelled.");
      }
      const now = Date.now();
      this.releaseReservationInventory(reservation, "cancelled", now);
      this.bumpProfile(now);
      return { reservationId, status: "cancelled" };
    });
    await this.scheduleAlarm();
    return jsonResponse(body);
  }

  private applySettlement(command: QueuedSettlement): Response {
    const accountId = requireIdentifier(command.accountId, "accountId");
    const settlementId = requireIdentifier(command.settlementId, "settlementId");
    const reservationId = requireIdentifier(command.reservationId, "reservationId");
    const matchId = requireIdentifier(command.matchId, "matchId");
    const payloadHash = requireIdentifier(command.payloadHash, "payloadHash");
    const loot = inventoryStacks(command.loot, "loot", 128);
    const ratingDelta = requireInteger(command.ratingDelta, "ratingDelta", -500, 500);
    if (!["extracted", "defeated", "abandoned"].includes(command.outcome)) {
      throw new ApiError(400, "invalid_settlement", "Settlement outcome is unsupported.");
    }

    const result = this.ctx.storage.transactionSync(() => {
      this.ensureAccount(accountId);
      const existing = firstRow(
        this.ctx.storage.sql
          .exec<SettlementRow>(
            "SELECT payload_hash, response_json FROM settlements WHERE settlement_id = ?",
            settlementId,
          )
          .toArray(),
      );
      if (existing !== undefined) {
        if (existing.payload_hash !== payloadHash) {
          throw new ApiError(
            409,
            "settlement_id_conflict",
            "Settlement id was already applied with different content.",
          );
        }
        return { status: 200, body: JSON.parse(existing.response_json) as unknown };
      }

      const reservation = this.reservation(reservationId);
      if (reservation.status === "settled") {
        throw new ApiError(
          409,
          "reservation_already_settled",
          "Reservation was settled by a different settlement id.",
        );
      }
      if (reservation.status !== "committed" || reservation.match_id !== matchId) {
        throw new ApiError(
          409,
          "reservation_match_conflict",
          "Settlement does not match a committed reservation.",
        );
      }

      const now = Date.now();
      const loadout = inventoryStacks(
        JSON.parse(reservation.loadout_json) as unknown,
        "stored loadout",
      );
      for (const stack of loadout) {
        if (command.outcome === "extracted") {
          this.ctx.storage.sql.exec(
            "UPDATE inventory SET reserved = reserved - ?, updated_at_ms = ? WHERE item_id = ?",
            stack.quantity,
            now,
            stack.itemId,
          );
        } else {
          this.ctx.storage.sql.exec(
            `UPDATE inventory
             SET quantity = quantity - ?, reserved = reserved - ?, updated_at_ms = ?
             WHERE item_id = ?`,
            stack.quantity,
            stack.quantity,
            now,
            stack.itemId,
          );
        }
      }
      if (command.outcome === "extracted") {
        for (const stack of loot) {
          this.ctx.storage.sql.exec(
            `INSERT INTO inventory(item_id, quantity, reserved, updated_at_ms)
             VALUES(?, ?, 0, ?)
             ON CONFLICT(item_id) DO UPDATE SET
               quantity = quantity + excluded.quantity,
               updated_at_ms = excluded.updated_at_ms`,
            stack.itemId,
            stack.quantity,
            now,
          );
        }
      }
      this.ctx.storage.sql.exec(
        `UPDATE profile
         SET rating = max(0, min(5000, rating + ?)),
             profile_version = profile_version + 1,
             updated_at_ms = ?
         WHERE singleton = 1`,
        ratingDelta,
        now,
      );

      const rating = firstRow(
        this.ctx.storage.sql
          .exec<{ rating: number; profile_version: number }>(
            "SELECT rating, profile_version FROM profile WHERE singleton = 1",
          )
          .toArray(),
      );
      if (rating === undefined) throw new Error("Profile row disappeared during settlement.");
      const body = {
        settlementId,
        reservationId,
        matchId,
        outcome: command.outcome,
        lootBanked: command.outcome === "extracted" ? loot : [],
        rating: rating.rating,
        profileVersion: rating.profile_version,
        appliedAtMs: now,
      };
      this.ctx.storage.sql.exec(
        `INSERT INTO settlements(
           settlement_id, payload_hash, reservation_id, match_id, outcome, response_json, applied_at_ms
         ) VALUES(?, ?, ?, ?, ?, ?, ?)`,
        settlementId,
        payloadHash,
        reservationId,
        matchId,
        command.outcome,
        JSON.stringify(body),
        now,
      );
      this.ctx.storage.sql.exec(
        `UPDATE reservations
         SET status = 'settled', settlement_id = ?, updated_at_ms = ?
         WHERE reservation_id = ?`,
        settlementId,
        now,
        reservationId,
      );
      return { status: 201, body };
    });
    return jsonResponse(result.body, result.status);
  }

  private updateIsland(command: IslandCheckpointCommand): Response {
    const accountId = requireIdentifier(command.accountId, "accountId");
    const checkpointId = requireIdentifier(command.checkpointId, "checkpointId");
    const expectedVersion = requireInteger(command.expectedVersion, "expectedVersion", 0, 2_147_483_647);
    const logicalTimeMs = requireInteger(command.logicalTimeMs, "logicalTimeMs", 0, 9_007_199_254_740_991);
    const contentBuild = requireIdentifier(command.contentBuild, "contentBuild");
    const objectKey = requireString(command.objectKey, "objectKey", 512);
    const sha256 = requireIdentifier(command.sha256, "sha256");
    const sizeBytes = requireInteger(command.sizeBytes, "sizeBytes", 1, 64 * 1024 * 1024);
    const etag = requireIdentifier(command.etag, "etag");
    if (objectKey !== checkpointObjectKey(accountId, sha256)) {
      throw new ApiError(
        400,
        "checkpoint_object_key_mismatch",
        "Checkpoint object key is not content-addressed for this account and payload.",
      );
    }

    const result = this.ctx.storage.transactionSync(() => {
      this.ensureAccount(accountId);
      const existing = firstRow(
        this.ctx.storage.sql
          .exec<IslandRow>(
            `SELECT checkpoint_id, version, logical_time_ms, content_build, object_key,
                    sha256, size_bytes, etag, updated_at_ms
             FROM island_checkpoints WHERE checkpoint_id = ?`,
            checkpointId,
          )
          .toArray(),
      );
      if (existing !== undefined) {
        if (
          existing.object_key !== objectKey ||
          existing.sha256 !== sha256 ||
          existing.size_bytes !== sizeBytes ||
          existing.content_build !== contentBuild
        ) {
          throw new ApiError(
            409,
            "checkpoint_id_conflict",
            "Checkpoint id was already used for different data.",
          );
        }
        return { status: 200, checkpoint: this.presentIsland(existing) };
      }
      const currentVersion = this.islandHead()?.version ?? 0;
      const runtime = this.runtimeRow();
      if (
        runtime.dirty_batches > 0 ||
        runtime.backlog_ticks > 0 ||
        (runtime.active_until_ms !== null && runtime.active_until_ms > Date.now())
      ) {
        throw new ApiError(
          409,
          "island_runtime_active",
          "Deactivate and checkpoint the active island before replacing its checkpoint.",
        );
      }
      if (currentVersion !== expectedVersion) {
        throw new ApiError(409, "island_version_conflict", "Island checkpoint version is stale.", {
          expectedVersion,
          currentVersion,
        });
      }
      const version = currentVersion + 1;
      const now = Date.now();
      this.ctx.storage.sql.exec(
        `INSERT INTO island_checkpoints(
           checkpoint_id, version, logical_time_ms, content_build, object_key,
           sha256, size_bytes, etag, updated_at_ms
         ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?)`,
        checkpointId,
        version,
        logicalTimeMs,
        contentBuild,
        objectKey,
        sha256,
        sizeBytes,
        etag,
        now,
      );
      this.ctx.storage.sql.exec(
        `INSERT INTO island_head(singleton, checkpoint_id, version)
         VALUES(1, ?, ?)
         ON CONFLICT(singleton) DO UPDATE SET
           checkpoint_id = excluded.checkpoint_id,
           version = excluded.version`,
        checkpointId,
        version,
      );
      this.bumpProfile(now);
      return {
        status: 201,
        checkpoint: {
          checkpointId,
          version,
          logicalTimeMs,
          contentBuild,
          objectKey,
          sha256,
          sizeBytes,
          etag,
          updatedAtMs: now,
        },
      };
    });
    if (result.status === 201) this.runtimeLoaded = false;
    return jsonResponse({ accountId, checkpoint: result.checkpoint }, result.status);
  }

  private islandHead(): ReturnType<ProfileIslandObject["presentIsland"]> | null {
    const row = firstRow(
      this.ctx.storage.sql
        .exec<IslandRow>(
          `SELECT c.checkpoint_id, c.version, c.logical_time_ms, c.content_build,
                  c.object_key, c.sha256, c.size_bytes, c.etag, c.updated_at_ms
           FROM island_head h
           JOIN island_checkpoints c ON c.checkpoint_id = h.checkpoint_id
           WHERE h.singleton = 1`,
        )
        .toArray(),
    );
    return row === undefined ? null : this.presentIsland(row);
  }

  private presentIsland(row: IslandRow) {
    return {
      checkpointId: row.checkpoint_id,
      version: row.version,
      logicalTimeMs: row.logical_time_ms,
      contentBuild: row.content_build,
      objectKey: row.object_key,
      sha256: row.sha256,
      sizeBytes: row.size_bytes,
      etag: row.etag,
      updatedAtMs: row.updated_at_ms,
    };
  }

  private reservation(reservationId: string): ReservationRow {
    const row = this.reservationOrUndefined(reservationId);
    if (row === undefined) throw new ApiError(404, "reservation_not_found", "Reservation was not found.");
    return row;
  }

  private reservationOrUndefined(reservationId: string): ReservationRow | undefined {
    return firstRow(
      this.ctx.storage.sql
        .exec<ReservationRow>(
          `SELECT reservation_id, request_hash, status, loadout_json, response_json,
                  expires_at_ms, match_id, settlement_id
           FROM reservations WHERE reservation_id = ?`,
          reservationId,
        )
        .toArray(),
    );
  }

  private releaseReservationInventory(
    reservation: ReservationRow,
    status: "expired" | "cancelled",
    now: number,
  ): void {
    const loadout = inventoryStacks(JSON.parse(reservation.loadout_json) as unknown, "stored loadout");
    for (const stack of loadout) {
      this.ctx.storage.sql.exec(
        "UPDATE inventory SET reserved = reserved - ?, updated_at_ms = ? WHERE item_id = ?",
        stack.quantity,
        now,
        stack.itemId,
      );
    }
    this.ctx.storage.sql.exec(
      "UPDATE reservations SET status = ?, updated_at_ms = ? WHERE reservation_id = ?",
      status,
      now,
      reservation.reservation_id,
    );
  }

  private expireReservations(now: number): void {
    this.ctx.storage.transactionSync(() => {
      const expired = this.ctx.storage.sql
        .exec<ReservationRow>(
          `SELECT reservation_id, request_hash, status, loadout_json, response_json,
                  expires_at_ms, match_id, settlement_id
           FROM reservations
           WHERE status = 'reserved' AND expires_at_ms <= ?
           ORDER BY expires_at_ms
           LIMIT 100`,
          now,
        )
        .toArray();
      for (const reservation of expired) {
        this.releaseReservationInventory(reservation, "expired", now);
      }
      if (expired.length > 0) this.bumpProfile(now);
    });
  }

  private async scheduleAlarm(): Promise<void> {
    const next = firstRow(
      this.ctx.storage.sql
        .exec<{ wake_at_ms: number | null }>(
          `SELECT min(wake_at_ms) AS wake_at_ms FROM (
             SELECT expires_at_ms AS wake_at_ms
             FROM reservations
             WHERE status = 'reserved'
             UNION ALL
             SELECT active_until_ms AS wake_at_ms
             FROM island_runtime
             WHERE active_until_ms IS NOT NULL
           )`,
        )
        .toArray(),
    );
    if (next === undefined || next.wake_at_ms === null) {
      await this.ctx.storage.deleteAlarm();
    } else {
      await this.ctx.storage.setAlarm(Math.max(Date.now() + 1_000, next.wake_at_ms));
    }
  }

  private async advanceIslandRuntime(command: IslandRuntimeCommand): Promise<Response> {
    const accountId = requireIdentifier(command.accountId, "accountId");
    const sequence = requireInteger(command.sequence, "sequence", 1, 9_007_199_254_740_991);
    const commands = base64UrlToBytes(requireIdentifier(command.commandsBase64, "commandsBase64"));
    if (commands.byteLength === 0 || commands.byteLength > MAX_ISLAND_COMMAND_BYTES) {
      throw new ApiError(
        413,
        "island_commands_too_large",
        `Island command batch must be 1-${MAX_ISLAND_COMMAND_BYTES} bytes.`,
      );
    }
    this.ensureAccount(accountId);
    this.settleDormantJobs(Date.now());
    if (!this.simulation.available) {
      throw new ApiError(
        501,
        "island_runtime_not_bound",
        "The active-island boundary is installed, but no compatible headless Wasm artifact is bound.",
      );
    }
    const payloadHash = await sha256Base64Url(
      canonicalJson({ accountId, sequence, commandsBase64: command.commandsBase64 }),
    );
    const previous = firstRow(
      this.ctx.storage.sql
        .exec<IslandRuntimeReceipt>(
          "SELECT payload_hash, response_json FROM island_command_receipts WHERE sequence = ?",
          sequence,
        )
        .toArray(),
    );
    if (previous !== undefined) {
      if (previous.payload_hash !== payloadHash) {
        throw new ApiError(
          409,
          "island_sequence_conflict",
          "Island command sequence was already used with different commands.",
        );
      }
      await this.scheduleAlarm();
      return jsonResponse(JSON.parse(previous.response_json) as unknown);
    }

    await this.ensureRuntimeLoaded();
    const runtime = this.runtimeRow();
    if (sequence !== runtime.last_sequence + 1) {
      throw new ApiError(409, "island_sequence_gap", "Island commands must be contiguous.", {
        expectedSequence: runtime.last_sequence + 1,
      });
    }
    const now = Date.now();
    const tickPlan = planIslandTickBatch(
      {
        lastWallClockMs: runtime.last_wall_clock_ms,
        tickRemainder: runtime.tick_remainder,
        backlogTicks: runtime.backlog_ticks,
        activeUntilMs: runtime.active_until_ms,
      },
      now,
    );
    if (tickPlan.kind === "not_due") {
      return jsonResponse(
        {
          sequence,
          status: "tick_not_due",
          retryAfterMs: tickPlan.retryAfterMs,
          logicalTick: runtime.logical_tick,
          backlogTicks: runtime.backlog_ticks,
        },
        202,
      );
    }
    const ticks = tickPlan.ticks;
    const tickStart = runtime.logical_tick;
    let advanced: Awaited<ReturnType<HeadlessIslandSimulation["advance"]>>;
    try {
      advanced = await this.simulation.advance(ticks, commands);
    } catch (error) {
      // A Wasm error is not proof that the module left its memory untouched.
      this.runtimeLoaded = false;
      throw error;
    }
    // From this point until the journal transaction commits, Wasm is ahead of
    // durable state. Leave the replica invalid even if response encoding or
    // another intermediate operation unexpectedly throws.
    this.runtimeLoaded = false;
    const tickEnd = tickStart + ticks;
    const responseBody = {
      sequence,
      status: "advanced",
      tickStart,
      tickEnd,
      ticks,
      backlogTicks: tickPlan.backlogTicks,
      eventsBase64: bytesToBase64Url(advanced.events),
      activeUntilMs: now + ISLAND_IDLE_AFTER_MS,
    };
    const commandBuffer = commands.buffer.slice(
      commands.byteOffset,
      commands.byteOffset + commands.byteLength,
    ) as ArrayBuffer;
    commitAdvancedSimulation(
      () =>
        this.ctx.storage.transactionSync(() => {
          const current = this.runtimeRow();
          if (current.last_sequence !== runtime.last_sequence) {
            throw new ApiError(409, "island_runtime_race", "Island runtime state changed concurrently.");
          }
          this.ctx.storage.sql.exec(
            `INSERT INTO island_command_journal(
               sequence, tick_start, tick_end, ticks, commands_blob, created_at_ms
             ) VALUES(?, ?, ?, ?, ?, ?)`,
            sequence,
            tickStart,
            tickEnd,
            ticks,
            commandBuffer,
            now,
          );
          this.ctx.storage.sql.exec(
            `INSERT INTO island_command_receipts(
               sequence, payload_hash, response_json, processed_at_ms
             ) VALUES(?, ?, ?, ?)`,
            sequence,
            payloadHash,
            JSON.stringify(responseBody),
            now,
          );
          this.ctx.storage.sql.exec(
            `UPDATE island_runtime
             SET logical_tick = ?, last_wall_clock_ms = ?, tick_remainder = ?,
                 backlog_ticks = ?, active_until_ms = ?, last_sequence = ?,
                 dirty_batches = dirty_batches + 1
             WHERE singleton = 1`,
            tickEnd,
            now,
            tickPlan.tickRemainder,
            tickPlan.backlogTicks,
            now + ISLAND_IDLE_AFTER_MS,
            sequence,
          );
        }),
      () => {
        this.runtimeLoaded = false;
      },
    );
    this.runtimeLoaded = true;
    if (runtime.dirty_batches + 1 >= 256) {
      await this.checkpointRuntime(false);
    }
    await this.scheduleAlarm();
    return jsonResponse(responseBody);
  }

  private runtimeStatus(accountId: string): unknown {
    const runtime = this.runtimeRow();
    const readyJobs =
      firstRow(
        this.ctx.storage.sql
          .exec<{ count: number }>(
            "SELECT count(*) AS count FROM island_jobs WHERE state = 'ready'",
          )
          .toArray(),
      )?.count ?? 0;
    return {
      accountId,
      adapter: this.simulation.available ? "headless-wasm" : "unbound",
      authoritativeHz: ISLAND_TICK_HZ,
      logicalTick: runtime.logical_tick,
      lastSequence: runtime.last_sequence,
      backlogTicks: runtime.backlog_ticks,
      active: runtime.active_until_ms !== null && runtime.active_until_ms > Date.now(),
      activeUntilMs: runtime.active_until_ms,
      checkpointVersion: runtime.checkpoint_version,
      dirtyBatches: runtime.dirty_batches,
      readyJobs,
      dormantEconomy: "timestamp-derived",
    };
  }

  private async ensureRuntimeLoaded(): Promise<void> {
    if (this.runtimeLoaded) return;
    const head = this.islandHead();
    let checkpoint: ArrayBuffer | null = null;
    if (head !== null) {
      const object = await this.bindings.MATCH_ARCHIVE.get(head.objectKey);
      if (object === null) {
        throw new ApiError(503, "island_checkpoint_unavailable", "Island checkpoint object is unavailable.");
      }
      checkpoint = await object.arrayBuffer();
      if ((await sha256Base64Url(checkpoint)) !== head.sha256) {
        throw new ApiError(503, "island_checkpoint_corrupt", "Island checkpoint hash verification failed.");
      }
    }
    await this.simulation.restore(checkpoint);
    const journal = this.ctx.storage.sql
      .exec<IslandJournalRow>(
        "SELECT ticks, commands_blob FROM island_command_journal ORDER BY sequence",
      )
      .toArray();
    for (const entry of journal) {
      await this.simulation.advance(entry.ticks, new Uint8Array(entry.commands_blob));
    }
    this.runtimeLoaded = true;
  }

  private async checkpointRuntime(markIdle: boolean): Promise<void> {
    const runtime = this.runtimeRow();
    if (runtime.dirty_batches === 0) {
      if (markIdle && runtime.active_until_ms !== null) {
        this.ctx.storage.sql.exec(
          "UPDATE island_runtime SET active_until_ms = NULL WHERE singleton = 1",
        );
      }
      return;
    }
    if (!this.simulation.available) {
      if (markIdle) {
        this.ctx.storage.sql.exec(
          "UPDATE island_runtime SET active_until_ms = NULL WHERE singleton = 1",
        );
      }
      return;
    }
    await this.ensureRuntimeLoaded();
    const checkpoint = await this.simulation.checkpoint();
    const maximumBytes = Number(this.bindings.MAX_ISLAND_CHECKPOINT_BYTES);
    if (
      !Number.isSafeInteger(maximumBytes) ||
      checkpoint.byteLength === 0 ||
      checkpoint.byteLength > maximumBytes
    ) {
      throw new ApiError(500, "island_checkpoint_invalid", "Headless island checkpoint size is invalid.");
    }
    const sha256 = await sha256Base64Url(checkpoint);
    const checkpointId = `runtime-${runtime.logical_tick}`;
    const objectKey = `island.runtime.${sha256}.bin`;
    const object = await this.bindings.MATCH_ARCHIVE.put(objectKey, checkpoint, {
      httpMetadata: { contentType: "application/octet-stream" },
      customMetadata: {
        checkpointId,
        contentBuild: this.bindings.CONTENT_BUILD_HASH,
        logicalTick: String(runtime.logical_tick),
        sha256,
      },
    });
    this.ctx.storage.transactionSync(() => {
      const current = this.runtimeRow();
      if (current.logical_tick !== runtime.logical_tick || current.dirty_batches !== runtime.dirty_batches) {
        throw new ApiError(409, "island_checkpoint_race", "Island state advanced during checkpoint.");
      }
      const currentVersion = this.islandHead()?.version ?? 0;
      const version = currentVersion + 1;
      const now = Date.now();
      this.ctx.storage.sql.exec(
        `INSERT INTO island_checkpoints(
           checkpoint_id, version, logical_time_ms, content_build, object_key,
           sha256, size_bytes, etag, updated_at_ms
         ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?)`,
        checkpointId,
        version,
        Math.floor((runtime.logical_tick * 1_000) / ISLAND_TICK_HZ),
        this.bindings.CONTENT_BUILD_HASH,
        objectKey,
        sha256,
        checkpoint.byteLength,
        object.etag,
        now,
      );
      this.ctx.storage.sql.exec(
        `INSERT INTO island_head(singleton, checkpoint_id, version)
         VALUES(1, ?, ?)
         ON CONFLICT(singleton) DO UPDATE SET
           checkpoint_id = excluded.checkpoint_id,
           version = excluded.version`,
        checkpointId,
        version,
      );
      this.ctx.storage.sql.exec("DELETE FROM island_command_journal");
      this.ctx.storage.sql.exec(
        `DELETE FROM island_command_receipts
         WHERE sequence <= max(0, ? - 512)`,
        runtime.last_sequence,
      );
      this.ctx.storage.sql.exec(
        `UPDATE island_runtime
         SET checkpoint_version = ?, dirty_batches = 0,
             active_until_ms = ?
         WHERE singleton = 1`,
        version,
        markIdle ? null : runtime.active_until_ms,
      );
      this.bumpProfile(now);
    });
  }

  private settleDormantJobs(now: number): number {
    return this.ctx.storage.sql.exec(
      `UPDATE island_jobs
       SET state = 'ready', updated_at_ms = ?
       WHERE state = 'pending' AND completes_at_ms <= ?`,
      now,
      now,
    ).rowsWritten;
  }

  private runtimeRow(): IslandRuntimeRow {
    const runtime = firstRow(
      this.ctx.storage.sql.exec<IslandRuntimeRow>("SELECT * FROM island_runtime WHERE singleton = 1").toArray(),
    );
    if (runtime === undefined) throw new Error("Island runtime row is missing.");
    return runtime;
  }

  private async withRuntimeLock<T>(operation: () => Promise<T>): Promise<T> {
    const previous = this.runtimeTail;
    let release!: () => void;
    this.runtimeTail = new Promise<void>((resolve) => {
      release = resolve;
    });
    await previous;
    try {
      return await operation();
    } finally {
      release();
    }
  }

  private bumpProfile(now: number): void {
    this.ctx.storage.sql.exec(
      `UPDATE profile
       SET profile_version = profile_version + 1, updated_at_ms = ?
       WHERE singleton = 1`,
      now,
    );
  }
}
