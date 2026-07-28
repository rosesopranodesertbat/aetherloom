import { DurableObject } from "cloudflare:workers";

import { browserMatchModule } from "./browser-match-module.ts";
import { BrowserMatchSimulation } from "./browser-match-runtime.ts";
import {
  BROWSER_MATCH_BOT_TAKEOVER_MS,
  BROWSER_MATCH_INPUT_TOKEN_CAPACITY,
  BROWSER_MATCH_MAX_FRAME_BYTES,
  BROWSER_MATCH_MAX_PLAYERS,
  BROWSER_MATCH_RECONNECT_MS,
  BROWSER_MATCH_SNAPSHOT_HZ,
  BROWSER_MATCH_TICK_HZ,
  advanceBrowserTickSchedule,
  consumeBrowserInputToken,
  isBrowserInputComponent,
  shouldConsumeBrowserInput,
  type BrowserMatchClaim,
  type BrowserMatchClaimRequest,
  type BrowserMatchEnv,
  type CoreEventSnapshot,
  type CoreMatchSnapshot,
  type CorePlayerSnapshot,
  type SocketAttachment,
} from "./browser-match-types.ts";
import { verifyJoinTicket } from "./join-tickets.ts";
import { publicWebSocketProtocol, ticketFromRequest } from "./tickets.ts";
import {
  ApiError,
  assertObject,
  errorResponse,
  jsonResponse,
  randomHex128,
  randomToken,
  readJson,
  requireHex128,
  requireString,
  sha256Base64Url,
} from "./util.ts";

interface RoomRow extends Record<string, SqlStorageValue> {
  match_id: string;
  room_code: string;
  build_hash: string;
  match_epoch: number;
  seed_low: number;
  seed_high: number;
}

interface PlayerRow extends Record<string, SqlStorageValue> {
  slot: number;
  account_id: string;
  team_id: number;
  resume_hash: string;
  claimed_at_ms: number;
  last_seen_at_ms: number;
  connected_at_ms: number | null;
  disconnected_at_ms: number | null;
  score: number;
}

const INPUT_FRAME_BYTES = 22;
const INPUT_FRAME_VERSION = 2;
const INPUT_FRAME_TYPE = 1;
const SNAPSHOT_FRAME_VERSION = 2;
const SNAPSHOT_FRAME_TYPE = 2;
const SNAPSHOT_HEADER_BYTES = 24;
const SNAPSHOT_PLAYER_BYTES = 30;
const SNAPSHOT_PROJECTILE_BYTES = 24;
const SNAPSHOT_EVENT_BYTES = 20;
const MAX_SNAPSHOT_PROJECTILES = 32;
const MAX_SNAPSHOT_EVENTS = 8;
const MAX_PENDING_INPUTS_PER_PLAYER = 4;
const MAX_UNACKED_SNAPSHOTS = 8;
const UNCONNECTED_CLAIM_MS = 15_000;
const PLAYER_SESSION_MAX_MS = 15 * 60 * 1_000;
const CLIENT_INPUT_TIMEOUT_MS = 5_000;
const CLIENT_INPUT_HOLD_MS = 250;
const ACTION_CAST = 1;
const CORE_DAMAGE_EVENT = 4;
const CORE_DEFEAT_EVENT = 5;
const ROUND_RESET_DELAY_TICKS = BROWSER_MATCH_TICK_HZ * 3;

export class BrowserMatchRoom extends DurableObject<BrowserMatchEnv> {
  private readonly bindings: BrowserMatchEnv;
  private simulation: BrowserMatchSimulation | undefined;
  private timer: number | undefined;
  private lastPulseAtMs = 0;
  private tickCredit = 0;
  private roundResetTick: number | undefined;
  private readonly scores = new Map<number, number>();

  constructor(ctx: DurableObjectState, env: BrowserMatchEnv) {
    super(ctx, env);
    this.bindings = env;
    this.ctx.storage.transactionSync(() => {
      this.ctx.storage.sql.exec(`
        CREATE TABLE IF NOT EXISTS room (
          singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
          match_id TEXT NOT NULL UNIQUE,
          room_code TEXT NOT NULL,
          build_hash TEXT NOT NULL,
          match_epoch INTEGER NOT NULL CHECK (match_epoch >= 1),
          seed_low INTEGER NOT NULL,
          seed_high INTEGER NOT NULL,
          created_at_ms INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS players (
          slot INTEGER PRIMARY KEY CHECK (slot BETWEEN 0 AND 7),
          account_id TEXT NOT NULL UNIQUE,
          team_id INTEGER NOT NULL CHECK (team_id BETWEEN 0 AND 7),
          resume_hash TEXT NOT NULL UNIQUE,
          claimed_at_ms INTEGER NOT NULL,
          last_seen_at_ms INTEGER NOT NULL,
          connected_at_ms INTEGER,
          disconnected_at_ms INTEGER,
          score INTEGER NOT NULL DEFAULT 0 CHECK (score >= 0)
        );

        CREATE TABLE IF NOT EXISTS consumed_join_nonces (
          nonce TEXT PRIMARY KEY,
          expires_at_seconds INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS consumed_join_nonces_expiry
          ON consumed_join_nonces(expires_at_seconds);
      `);
    });
    // The casual systems-test host deliberately keeps round state in memory.
    // If the object is reconstructed with hibernated sockets, force a clean
    // reconnect so clients get a new stream and never mistake a reset tick or
    // snapshot sequence for continuous authoritative state.
    for (const webSocket of this.ctx.getWebSockets()) {
      webSocket.close(1012, "staging match host restarted");
    }
  }

  override async fetch(request: Request): Promise<Response> {
    try {
      const url = new URL(request.url);
      if (request.method === "POST" && url.pathname === "/claim") {
        return await this.claim(await readJson<BrowserMatchClaimRequest>(request, 2_048));
      }
      if (request.method === "GET" && url.pathname === "/ws") {
        return await this.acceptPlayerWebSocket(request);
      }
      return jsonResponse(
        { error: { code: "not_found", message: "Browser match room route not found." } },
        404,
      );
    } catch (error) {
      return errorResponse(error, "browser-match-room");
    }
  }

  override async webSocketMessage(
    webSocket: WebSocket,
    message: string | ArrayBuffer,
  ): Promise<void> {
    if (typeof message === "string" || message.byteLength !== INPUT_FRAME_BYTES) {
      webSocket.close(1003, "binary 22-byte input frames required");
      return;
    }
    let attachment: SocketAttachment;
    try {
      attachment = socketAttachment(webSocket);
    } catch {
      webSocket.close(1011, "match session state invalid");
      return;
    }
    const now = Date.now();
    const view = new DataView(message);
    if (
      view.getUint8(0) !== INPUT_FRAME_VERSION ||
      view.getUint8(1) !== INPUT_FRAME_TYPE ||
      view.getUint16(2, true) !== INPUT_FRAME_BYTES ||
      view.getUint8(15) !== 0
    ) {
      webSocket.close(1003, "invalid input frame");
      return;
    }
    const actionFlags = view.getUint8(14);
    if ((actionFlags & ~ACTION_CAST) !== 0) {
      webSocket.close(1003, "unsupported input action");
      return;
    }
    const budget = consumeBrowserInputToken(
      {
        tokens: attachment.inputRateTokens,
        updatedAtMs: attachment.inputRateUpdatedAtMs,
        rejectedMessages: attachment.inputRateRejectedMessages,
      },
      now,
    );
    attachment.inputRateTokens = budget.tokens;
    attachment.inputRateUpdatedAtMs = budget.updatedAtMs;
    attachment.inputRateRejectedMessages = budget.rejectedMessages;
    if (!budget.accepted) {
      webSocket.serializeAttachment(attachment);
      if (budget.shouldClose) webSocket.close(1008, "sustained input flood");
      return;
    }
    const sequence = view.getUint32(4, true);
    if (sequence === 0 || !sequenceIsNewer(sequence, attachment.lastSequence)) {
      webSocket.serializeAttachment(attachment);
      return;
    }
    const compactMoveX = view.getInt8(8);
    const compactMoveY = view.getInt8(9);
    const compactMoveVertical = view.getInt8(10);
    const compactPitch = view.getInt8(11);
    if (
      ![
        compactMoveX,
        compactMoveY,
        compactMoveVertical,
        compactPitch,
      ].every(isBrowserInputComponent)
    ) {
      webSocket.close(1003, "input component out of range");
      return;
    }
    const moveX = quantizeAxis(compactMoveX);
    const moveY = quantizeAxis(compactMoveY);
    const moveVertical = quantizeAxis(compactMoveVertical);
    const pitch = quantizePitch(compactPitch);
    const yaw = view.getUint16(12, true);
    const snapshotAck = view.getUint32(18, true);
    if (
      snapshotAck !== 0 &&
      (attachment.lastSentSnapshotSequence === 0 ||
        sequenceIsNewer(snapshotAck, attachment.lastSentSnapshotSequence))
    ) {
      webSocket.close(1003, "invalid snapshot acknowledgement");
      return;
    }
    if (
      snapshotAck !== 0 &&
      (attachment.lastAckedSnapshotSequence === 0 ||
        sequenceIsNewer(snapshotAck, attachment.lastAckedSnapshotSequence))
    ) {
      attachment.lastAckedSnapshotSequence = snapshotAck;
    }
    attachment.lastSequence = sequence;
    attachment.clientClock = view.getUint16(16, true);
    attachment.lastInputAtMs = now;
    attachment.pendingInputs.push({
      sequence,
      moveX,
      moveY,
      moveVertical,
      yaw,
      pitch,
      cast: (actionFlags & ACTION_CAST) !== 0,
    });
    if (attachment.pendingInputs.length > MAX_PENDING_INPUTS_PER_PLAYER) {
      attachment.pendingInputs.shift();
    }
    webSocket.serializeAttachment(attachment);
    this.ensureLoop();
  }

  override webSocketClose(
    webSocket: WebSocket,
    _code: number,
    _reason: string,
    _wasClean: boolean,
  ): void {
    this.disconnect(webSocket);
  }

  override webSocketError(webSocket: WebSocket, _error: unknown): void {
    this.disconnect(webSocket);
  }

  private async claim(raw: BrowserMatchClaimRequest): Promise<Response> {
    assertObject(raw, "browser match claim");
    const roomCode = requireString(raw.room, "room", 64);
    if (
      !/^duel-(?:0[1-9]|[12][0-9]|3[0-2])$/u.test(roomCode) &&
      !/^smoke-[0-9a-f]{32}$/u.test(roomCode)
    ) {
      throw new ApiError(400, "invalid_demo_room", "Room code is invalid.");
    }
    const matchId = requireHex128(raw.matchId, "matchId");
    const buildHash = requireHex128(raw.buildHash, "buildHash");
    if (buildHash !== requireHex128(this.bindings.CONTENT_BUILD_HASH, "CONTENT_BUILD_HASH")) {
      throw new ApiError(409, "match_build_mismatch", "Room belongs to another content build.");
    }
    const resumeToken =
      raw.resumeToken === undefined
        ? undefined
        : requireString(raw.resumeToken, "resumeToken", 128);
    const resumeHash =
      resumeToken === undefined ? undefined : await sha256Base64Url(resumeToken);
    const connectedSlots = this.connectedSlots();
    const now = Date.now();
    this.reconcileDisconnectedRows(connectedSlots, now);
    const nextResumeToken = randomToken("demo_resume");
    const nextResumeHash = await sha256Base64Url(nextResumeToken);
    const accountId = randomHex128();
    const expiredSlots: number[] = [];
    let createdPlayer = false;

    const claim = this.ctx.storage.transactionSync(() => {
      let room = this.roomRow();
      if (room === undefined) {
        const seedLow = Number.parseInt(matchId.slice(0, 8), 16) | 0;
        const seedHigh = Number.parseInt(matchId.slice(8, 16), 16) | 0;
        this.ctx.storage.sql.exec(
          `INSERT INTO room (
             singleton, match_id, room_code, build_hash, match_epoch,
             seed_low, seed_high, created_at_ms
           ) VALUES (1, ?, ?, ?, 1, ?, ?, ?)`,
          matchId,
          roomCode,
          buildHash,
          seedLow,
          seedHigh,
          now,
        );
        room = this.roomRow();
      }
      if (
        room === undefined ||
        room.match_id !== matchId ||
        room.room_code !== roomCode ||
        room.build_hash !== buildHash
      ) {
        throw new ApiError(409, "room_identity_conflict", "Room identity is immutable.");
      }

      if (resumeHash !== undefined) {
        const resumed = this.ctx.storage.sql
          .exec<PlayerRow>("SELECT * FROM players WHERE resume_hash = ?", resumeHash)
          .toArray()[0];
        if (resumed === undefined) {
          throw new ApiError(401, "invalid_resume_token", "Resume token is not valid for this room.");
        }
        if (
          resumed.claimed_at_ms <= now - PLAYER_SESSION_MAX_MS ||
          resumed.disconnected_at_ms !== null &&
          resumed.disconnected_at_ms <
            now -
              (resumed.connected_at_ms === null
                ? UNCONNECTED_CLAIM_MS
                : BROWSER_MATCH_RECONNECT_MS)
        ) {
          throw new ApiError(401, "resume_expired", "The reconnect window has expired.");
        }
        this.ctx.storage.sql.exec(
          "UPDATE players SET resume_hash = ?, last_seen_at_ms = ? WHERE slot = ?",
          nextResumeHash,
          now,
          resumed.slot,
        );
        return this.presentClaim(room, resumed, nextResumeToken);
      }

      const players = this.playerRows();
      for (const player of players) {
        if (
          !connectedSlots.has(player.slot) &&
          (player.claimed_at_ms <= now - PLAYER_SESSION_MAX_MS ||
            (player.disconnected_at_ms !== null &&
              player.disconnected_at_ms <
                now -
                  (player.connected_at_ms === null
                    ? UNCONNECTED_CLAIM_MS
                    : BROWSER_MATCH_RECONNECT_MS)))
        ) {
          this.ctx.storage.sql.exec("DELETE FROM players WHERE slot = ?", player.slot);
          expiredSlots.push(player.slot);
        }
      }
      const remaining = this.playerRows();
      const used = new Set(remaining.map((player) => player.slot));
      const slot = Array.from(
        { length: BROWSER_MATCH_MAX_PLAYERS },
        (_, index) => index,
      ).find((candidate) => !used.has(candidate));
      if (slot === undefined) {
        throw new ApiError(409, "demo_room_full", "This multiplayer test room is full.");
      }
      this.ctx.storage.sql.exec(
        `INSERT INTO players (
           slot, account_id, team_id, resume_hash, claimed_at_ms,
           last_seen_at_ms, connected_at_ms, disconnected_at_ms, score
         ) VALUES (?, ?, ?, ?, ?, ?, NULL, ?, 0)`,
        slot,
        accountId,
        slot,
        nextResumeHash,
        now,
        now,
        now,
      );
      createdPlayer = true;
      const player = this.ctx.storage.sql
        .exec<PlayerRow>("SELECT * FROM players WHERE slot = ?", slot)
        .toArray()[0];
      if (player === undefined) throw new Error("New browser match player was not stored.");
      return this.presentClaim(room, player, nextResumeToken);
    });

    if (this.simulation !== undefined) {
      for (const slot of expiredSlots) this.simulation.removePlayer(slot);
      if (createdPlayer) {
        this.simulation.addHuman(claim.playerSlot, claim.teamId);
        this.scores.set(claim.playerSlot, 0);
      }
    }
    return jsonResponse(claim);
  }

  private async acceptPlayerWebSocket(request: Request): Promise<Response> {
    if (request.headers.get("upgrade")?.toLowerCase() !== "websocket") {
      throw new ApiError(426, "websocket_required", "WebSocket upgrade required.");
    }
    const protocol = publicWebSocketProtocol(request, "aetherloom.v2");
    const room = this.roomRow();
    if (room === undefined) {
      throw new ApiError(404, "match_not_found", "Browser match room was not found.");
    }
    const token = ticketFromRequest(request, true);
    const claims = await verifyJoinTicket(
      token,
      this.bindings.JOIN_TICKET_PUBLIC_KEYS_JSON,
      {
        issuer: this.bindings.TICKET_ISSUER,
        audience: this.bindings.JOIN_TICKET_AUDIENCE,
        matchId: room.match_id,
        matchEpoch: room.match_epoch,
        buildHash: room.build_hash,
        inputPool: "browser",
      },
    );
    const player = this.ctx.storage.sql
      .exec<PlayerRow>(
        "SELECT * FROM players WHERE account_id = ? AND slot = ? AND team_id = ?",
        claims.sub,
        claims.player_slot,
        claims.team_id,
      )
      .toArray()[0];
    if (player === undefined) {
      throw new ApiError(403, "player_claim_mismatch", "Join ticket has no room claim.");
    }
    if (player.claimed_at_ms <= Date.now() - PLAYER_SESSION_MAX_MS) {
      throw new ApiError(
        403,
        "demo_session_expired",
        "This staging session has reached its fifteen-minute limit.",
      );
    }
    const nowSeconds = Math.floor(Date.now() / 1_000);
    this.ctx.storage.transactionSync(() => {
      this.ctx.storage.sql.exec(
        "DELETE FROM consumed_join_nonces WHERE expires_at_seconds <= ?",
        nowSeconds,
      );
      const existing = this.ctx.storage.sql
        .exec<{ nonce: string }>(
          "SELECT nonce FROM consumed_join_nonces WHERE nonce = ?",
          claims.nonce,
        )
        .toArray()[0];
      if (existing !== undefined) {
        throw new ApiError(409, "join_ticket_replayed", "Join ticket was already consumed.");
      }
      this.ctx.storage.sql.exec(
        "INSERT INTO consumed_join_nonces (nonce, expires_at_seconds) VALUES (?, ?)",
        claims.nonce,
        claims.exp,
      );
      this.ctx.storage.sql.exec(
        `UPDATE players
         SET last_seen_at_ms = ?, connected_at_ms = coalesce(connected_at_ms, ?),
             disconnected_at_ms = NULL
         WHERE slot = ?`,
        Date.now(),
        Date.now(),
        player.slot,
      );
    });

    for (const existing of this.ctx.getWebSockets(`slot:${player.slot}`)) {
      existing.close(4001, "session replaced");
    }
    const pair = new WebSocketPair();
    const client = pair[0];
    const server = pair[1];
    server.serializeAttachment({
      accountId: player.account_id,
      slot: player.slot,
      teamId: player.team_id,
      lastSequence: 0,
      lastAppliedSequence: 0,
      clientClock: 0,
      lastAckedSnapshotSequence: 0,
      lastSentSnapshotSequence: 0,
      inputRateTokens: BROWSER_MATCH_INPUT_TOKEN_CAPACITY,
      inputRateUpdatedAtMs: Date.now(),
      inputRateRejectedMessages: 0,
      lastInputAtMs: Date.now(),
      sessionExpiresAtMs: player.claimed_at_ms + PLAYER_SESSION_MAX_MS,
      pendingInputs: [],
      heldInput: null,
    } satisfies SocketAttachment);
    this.ctx.acceptWebSocket(server, [`slot:${player.slot}`]);

    const simulation = this.ensureSimulation();
    simulation.setHuman(player.slot);
    simulation.clearInput(player.slot);
    this.sendSnapshot(server, simulation.snapshot(), []);
    this.ensureLoop();
    return new Response(null, {
      status: 101,
      headers: { "sec-websocket-protocol": protocol },
      webSocket: client,
    });
  }

  private ensureSimulation(): BrowserMatchSimulation {
    if (this.simulation !== undefined) return this.simulation;
    const room = this.roomRow();
    if (room === undefined) {
      throw new ApiError(404, "match_not_found", "Browser match room was not found.");
    }
    const simulation = new BrowserMatchSimulation(
      browserMatchModule,
      room.seed_low >>> 0,
      room.seed_high >>> 0,
    );
    const connected = this.connectedSlots();
    const now = Date.now();
    this.reconcileDisconnectedRows(connected, now);
    for (const player of this.playerRows()) {
      simulation.addHuman(player.slot, player.team_id);
      if (
        !connected.has(player.slot) &&
        player.disconnected_at_ms !== null &&
        player.disconnected_at_ms <= now - BROWSER_MATCH_BOT_TAKEOVER_MS
      ) {
        simulation.setBot(player.slot);
      }
      this.scores.set(player.slot, player.score);
    }
    this.simulation = simulation;
    return simulation;
  }

  private ensureLoop(): void {
    if (this.timer !== undefined || this.ctx.getWebSockets().length === 0) return;
    if (this.lastPulseAtMs === 0) this.lastPulseAtMs = Date.now();
    this.timer = setTimeout(() => {
      this.timer = undefined;
      void this.pulse();
    }, Math.ceil(1_000 / BROWSER_MATCH_SNAPSHOT_HZ)) as unknown as number;
  }

  private async pulse(): Promise<void> {
    try {
      if (this.ctx.getWebSockets().length === 0) {
        this.tickCredit = 0;
        this.lastPulseAtMs = 0;
        return;
      }
      const now = Date.now();
      this.expireSessions(now);
      if (this.ctx.getWebSockets().length === 0) {
        this.tickCredit = 0;
        this.lastPulseAtMs = 0;
        return;
      }
      const batch = advanceBrowserTickSchedule(
        { tickCredit: this.tickCredit, lastPulseAtMs: this.lastPulseAtMs },
        now,
      );
      this.lastPulseAtMs = batch.lastPulseAtMs;
      this.tickCredit = batch.tickCredit;

      const simulation = this.ensureSimulation();
      this.activateDueBots(simulation, now);
      let snapshot = simulation.snapshot();
      const events: CoreEventSnapshot[] = [];
      for (let index = 0; index < batch.ticks; index += 1) {
        if (shouldConsumeBrowserInput(snapshot.tick)) {
          this.consumePendingInputs(simulation, now);
        }
        snapshot = simulation.advanceTick();
        events.push(...snapshot.events);
        this.recordDefeats(snapshot, snapshot.events);
        if (
          this.roundResetTick !== undefined &&
          snapshot.tick >= this.roundResetTick
        ) {
          for (const player of snapshot.players) simulation.resetPlayer(player.playerId);
          this.roundResetTick = undefined;
          snapshot = simulation.snapshot();
        }
      }
      for (const webSocket of this.ctx.getWebSockets()) {
        this.sendSnapshot(webSocket, snapshot, events);
      }
    } catch (error) {
      console.error("browser match loop failed", { error });
      for (const webSocket of this.ctx.getWebSockets()) {
        webSocket.close(1011, "authoritative match failure");
      }
      this.simulation = undefined;
      this.tickCredit = 0;
      this.lastPulseAtMs = 0;
      return;
    }
    this.ensureLoop();
  }

  private consumePendingInputs(simulation: BrowserMatchSimulation, now: number): void {
    for (const webSocket of this.ctx.getWebSockets()) {
      const attachment = socketAttachment(webSocket);
      const pending = attachment.pendingInputs.shift();
      const sample =
        pending ??
        (attachment.heldInput !== null &&
        attachment.lastInputAtMs > now - CLIENT_INPUT_HOLD_MS
          ? attachment.heldInput
          : undefined);
      if (sample === undefined) {
        simulation.clearInput(attachment.slot);
        continue;
      }
      simulation.submitInput(
        attachment.slot,
        sample.moveX,
        sample.moveY,
        sample.moveVertical,
        sample.yaw,
        sample.pitch,
        sample.cast,
      );
      if (pending !== undefined) {
        attachment.lastAppliedSequence = pending.sequence;
        attachment.heldInput = { ...pending, cast: false };
      }
      webSocket.serializeAttachment(attachment);
    }
  }

  private activateDueBots(simulation: BrowserMatchSimulation, now: number): void {
    const connected = this.connectedSlots();
    this.reconcileDisconnectedRows(connected, now);
    for (const player of this.playerRows()) {
      if (
        !connected.has(player.slot) &&
        player.disconnected_at_ms !== null &&
        player.disconnected_at_ms <= now - BROWSER_MATCH_BOT_TAKEOVER_MS
      ) {
        const state = simulation
          .snapshot()
          .players.find((candidate) => candidate.playerId === player.slot);
        if (state?.controller === 1) simulation.setBot(player.slot);
      }
    }
  }

  private expireSessions(now: number): void {
    for (const webSocket of this.ctx.getWebSockets()) {
      const attachment = socketAttachment(webSocket);
      if (attachment.sessionExpiresAtMs <= now) {
        webSocket.close(4003, "staging session expired");
      } else if (attachment.lastInputAtMs <= now - CLIENT_INPUT_TIMEOUT_MS) {
        webSocket.close(4004, "input stream timed out");
      }
    }
  }

  private reconcileDisconnectedRows(connected: Set<number>, now: number): void {
    for (const player of this.playerRows()) {
      if (!connected.has(player.slot) && player.disconnected_at_ms === null) {
        this.ctx.storage.sql.exec(
          `UPDATE players
           SET last_seen_at_ms = ?, disconnected_at_ms = ?
           WHERE slot = ? AND disconnected_at_ms IS NULL`,
          now,
          now,
          player.slot,
        );
      }
    }
  }

  private recordDefeats(
    snapshot: CoreMatchSnapshot,
    events: readonly CoreEventSnapshot[],
  ): void {
    if (this.roundResetTick !== undefined) return;
    const byEntity = new Map(snapshot.players.map((player) => [player.entityKey, player]));
    const defeat = events.find((event) => event.kind === CORE_DEFEAT_EVENT);
    if (defeat === undefined) return;
    const actor =
      defeat.actorEntityKey === null ? undefined : byEntity.get(defeat.actorEntityKey);
    if (actor !== undefined) {
      const score = (this.scores.get(actor.playerId) ?? 0) + 1;
      this.scores.set(actor.playerId, score);
      this.ctx.storage.sql.exec(
        "UPDATE players SET score = ? WHERE slot = ?",
        score,
        actor.playerId,
      );
    }
    this.roundResetTick = snapshot.tick + ROUND_RESET_DELAY_TICKS;
  }

  private sendSnapshot(
    webSocket: WebSocket,
    snapshot: CoreMatchSnapshot,
    rawEvents: readonly CoreEventSnapshot[],
  ): void {
    const attachment = socketAttachment(webSocket);
    const outstanding =
      attachment.lastSentSnapshotSequence === 0
        ? 0
        : (attachment.lastSentSnapshotSequence -
            attachment.lastAckedSnapshotSequence) >>>
          0;
    if (outstanding >= MAX_UNACKED_SNAPSHOTS) {
      // Snapshots are replaceable. Coalesce to the latest authoritative state
      // until the client acknowledges progress instead of growing a send
      // queue with obsolete frames.
      return;
    }
    const players = [...snapshot.players].sort((left, right) => left.playerId - right.playerId);
    const projectiles = snapshot.projectiles.slice(0, MAX_SNAPSHOT_PROJECTILES);
    const events = rawEvents
      .filter((event) => event.kind === CORE_DAMAGE_EVENT || event.kind === CORE_DEFEAT_EVENT)
      .slice(-MAX_SNAPSHOT_EVENTS);
    const byteLength =
      SNAPSHOT_HEADER_BYTES +
      players.length * SNAPSHOT_PLAYER_BYTES +
      projectiles.length * SNAPSHOT_PROJECTILE_BYTES +
      events.length * SNAPSHOT_EVENT_BYTES;
    if (byteLength > BROWSER_MATCH_MAX_FRAME_BYTES) {
      throw new Error("Browser match snapshot exceeded its hard frame bound.");
    }
    const buffer = new ArrayBuffer(byteLength);
    const view = new DataView(buffer);
    let snapshotSequence = (attachment.lastSentSnapshotSequence + 1) >>> 0;
    if (snapshotSequence === 0) snapshotSequence = 1;
    view.setUint8(0, SNAPSHOT_FRAME_VERSION);
    view.setUint8(1, SNAPSHOT_FRAME_TYPE);
    view.setUint16(2, byteLength, true);
    view.setUint32(4, snapshot.tick >>> 0, true);
    view.setUint32(8, snapshotSequence, true);
    view.setUint16(12, attachment.clientClock, true);
    view.setUint8(14, attachment.slot);
    view.setUint8(15, players.length);
    view.setUint8(16, projectiles.length);
    view.setUint8(17, events.length);
    const waiting = players.filter((player) => player.outcome === 1).length < 2;
    const matchFlags =
      (this.roundResetTick === undefined ? 0 : 1) |
      (waiting ? 1 << 1 : 0);
    view.setUint16(18, matchFlags, true);
    view.setUint32(20, attachment.lastAppliedSequence, true);

    const connected = this.connectedSlots();
    const byEntity = new Map(players.map((player) => [player.entityKey, player]));
    let offset = SNAPSHOT_HEADER_BYTES;
    for (const player of players) {
      view.setUint8(offset, player.playerId);
      view.setUint8(offset + 1, Math.max(0, player.teamId));
      view.setUint8(offset + 2, player.outcome);
      const flags =
        (player.outcome === 1 && player.health > 0 ? 1 : 0) |
        (player.controller === 2 ? 1 << 1 : 0) |
        (connected.has(player.playerId) ? 1 << 2 : 0);
      view.setUint8(offset + 3, flags);
      view.setInt32(offset + 4, player.xCm, true);
      view.setInt32(offset + 8, player.yCm, true);
      view.setInt32(offset + 12, player.zCm, true);
      view.setUint16(offset + 16, player.yaw, true);
      view.setInt16(offset + 18, player.pitch, true);
      view.setUint16(offset + 20, Math.max(0, player.health), true);
      view.setUint16(offset + 22, 100, true);
      view.setUint16(offset + 24, this.scores.get(player.playerId) ?? 0, true);
      view.setUint32(offset + 26, entityHandle(player.entityKey), true);
      offset += SNAPSHOT_PLAYER_BYTES;
    }
    for (const projectile of projectiles) {
      view.setUint32(offset, entityHandle(projectile.entityKey), true);
      view.setUint8(offset + 4, Math.max(0, projectile.ownerPlayerId));
      view.setUint8(offset + 5, 0);
      view.setUint16(offset + 6, projectile.lifetimeTicks, true);
      view.setInt32(offset + 8, projectile.xCm, true);
      view.setInt32(offset + 12, projectile.yCm, true);
      view.setInt32(offset + 16, projectile.zCm, true);
      view.setUint16(offset + 20, projectile.yaw, true);
      view.setInt16(offset + 22, projectile.pitch, true);
      offset += SNAPSHOT_PROJECTILE_BYTES;
    }
    for (const event of events) {
      const actor = event.actorEntityKey === null ? undefined : byEntity.get(event.actorEntityKey);
      const target =
        event.targetEntityKey === null ? undefined : byEntity.get(event.targetEntityKey);
      const position = target ?? actor;
      view.setUint32(offset, event.eventId >>> 0, true);
      view.setUint8(offset + 4, event.kind);
      view.setUint8(offset + 5, actor?.playerId ?? 255);
      view.setUint8(offset + 6, target?.playerId ?? 255);
      view.setUint8(offset + 7, 0);
      view.setInt32(offset + 8, position?.xCm ?? 0, true);
      view.setInt32(offset + 12, position?.yCm ?? 0, true);
      view.setInt32(offset + 16, position?.zCm ?? 0, true);
      offset += SNAPSHOT_EVENT_BYTES;
    }
    try {
      webSocket.send(buffer);
      attachment.lastSentSnapshotSequence = snapshotSequence;
      webSocket.serializeAttachment(attachment);
    } catch {
      webSocket.close(1011, "snapshot delivery failed");
    }
  }

  private disconnect(webSocket: WebSocket): void {
    let attachment: SocketAttachment;
    try {
      attachment = socketAttachment(webSocket);
    } catch {
      return;
    }
    const stillConnected = this.ctx
      .getWebSockets(`slot:${attachment.slot}`)
      .some((candidate) => candidate !== webSocket);
    if (stillConnected) return;
    try {
      this.simulation?.clearInput(attachment.slot);
    } catch {
      this.simulation = undefined;
    }
    this.ctx.storage.sql.exec(
      `UPDATE players
       SET last_seen_at_ms = ?, disconnected_at_ms = ?
       WHERE slot = ?`,
      Date.now(),
      Date.now(),
      attachment.slot,
    );
  }

  private presentClaim(
    room: RoomRow,
    player: PlayerRow,
    resumeToken: string,
  ): BrowserMatchClaim {
    return {
      matchId: room.match_id,
      matchEpoch: room.match_epoch,
      accountId: player.account_id,
      playerSlot: player.slot,
      teamId: player.team_id,
      resumeToken,
      playerCount: this.playerRows().length,
    };
  }

  private roomRow(): RoomRow | undefined {
    return this.ctx.storage.sql
      .exec<RoomRow>("SELECT * FROM room WHERE singleton = 1")
      .toArray()[0];
  }

  private playerRows(): PlayerRow[] {
    return this.ctx.storage.sql
      .exec<PlayerRow>("SELECT * FROM players ORDER BY slot")
      .toArray();
  }

  private connectedSlots(): Set<number> {
    const slots = new Set<number>();
    for (const webSocket of this.ctx.getWebSockets()) {
      try {
        slots.add(socketAttachment(webSocket).slot);
      } catch {
        webSocket.close(1011, "match session state invalid");
      }
    }
    return slots;
  }
}

function socketAttachment(webSocket: WebSocket): SocketAttachment {
  const value = webSocket.deserializeAttachment() as unknown;
  if (
    typeof value !== "object" ||
    value === null ||
    Array.isArray(value)
  ) {
    throw new Error("Browser match socket attachment is missing.");
  }
  const attachment = value as Partial<SocketAttachment>;
  if (
    typeof attachment.accountId !== "string" ||
    !Number.isInteger(attachment.slot) ||
    attachment.slot === undefined ||
    attachment.slot < 0 ||
    attachment.slot >= BROWSER_MATCH_MAX_PLAYERS ||
    !Number.isInteger(attachment.teamId) ||
    !Number.isInteger(attachment.lastSequence) ||
    !Number.isInteger(attachment.lastAppliedSequence) ||
    !Number.isInteger(attachment.clientClock) ||
    !Number.isInteger(attachment.lastAckedSnapshotSequence) ||
    !Number.isInteger(attachment.lastSentSnapshotSequence) ||
    !Number.isFinite(attachment.inputRateTokens) ||
    attachment.inputRateTokens === undefined ||
    attachment.inputRateTokens < 0 ||
    attachment.inputRateTokens > BROWSER_MATCH_INPUT_TOKEN_CAPACITY ||
    !Number.isInteger(attachment.inputRateUpdatedAtMs) ||
    !Number.isInteger(attachment.inputRateRejectedMessages) ||
    attachment.inputRateRejectedMessages === undefined ||
    attachment.inputRateRejectedMessages < 0 ||
    !Number.isInteger(attachment.lastInputAtMs) ||
    attachment.lastInputAtMs === undefined ||
    attachment.lastInputAtMs <= 0 ||
    !Number.isInteger(attachment.sessionExpiresAtMs) ||
    attachment.sessionExpiresAtMs === undefined ||
    attachment.sessionExpiresAtMs <= 0 ||
    !Array.isArray(attachment.pendingInputs) ||
    attachment.pendingInputs.length > MAX_PENDING_INPUTS_PER_PLAYER ||
    attachment.pendingInputs.some((sample) => !validPendingInput(sample)) ||
    (attachment.heldInput !== null && !validPendingInput(attachment.heldInput))
  ) {
    throw new Error("Browser match socket attachment is invalid.");
  }
  return attachment as SocketAttachment;
}

function validPendingInput(sample: unknown): sample is SocketAttachment["pendingInputs"][number] {
  if (typeof sample !== "object" || sample === null) return false;
  const input = sample as Partial<SocketAttachment["pendingInputs"][number]>;
  return (
    Number.isInteger(input.sequence) &&
    input.sequence !== undefined &&
    input.sequence > 0 &&
    Number.isInteger(input.moveX) &&
    Number.isInteger(input.moveY) &&
    Number.isInteger(input.moveVertical) &&
    Number.isInteger(input.yaw) &&
    Number.isInteger(input.pitch) &&
    typeof input.cast === "boolean"
  );
}

function quantizeAxis(value: number): number {
  return Math.max(-2_047, Math.min(2_047, Math.round((value * 2_047) / 127)));
}

function quantizePitch(value: number): number {
  return Math.max(-16_384, Math.min(16_384, Math.round((value * 16_384) / 127)));
}

function sequenceIsNewer(candidate: number, previous: number): boolean {
  if (previous === 0) return true;
  const delta = (candidate - previous) >>> 0;
  return delta !== 0 && delta < 0x8000_0000;
}

function entityHandle(entityKey: string): number {
  const [generationRaw, indexRaw] = entityKey.split(":");
  const generation = Number(generationRaw);
  const index = Number(indexRaw);
  if (
    !Number.isSafeInteger(generation) ||
    generation < 1 ||
    generation > 0x01ff_ffff ||
    !Number.isSafeInteger(index) ||
    index < 0 ||
    index > 127
  ) {
    throw new Error("Authoritative entity id is out of range.");
  }
  // The browser slice caps the authoritative entity pool at 128, so its
  // network handle can retain the full slot plus 25 generation bits. This
  // prevents a reused dense slot from masquerading as an older entity while
  // keeping the compact v2 browser record inside the 1,200-byte frame bound.
  return ((generation << 7) | index) >>> 0;
}
