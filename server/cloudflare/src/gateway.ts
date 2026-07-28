import {
  ServiceBindingMatchDirector,
  type ResumableDispatch,
} from "./match-director";
import { matchmakingShardName } from "./matchmaking";
import { acceptSignedSettlement } from "./settlements";
import { signJoinTicket, verifyJoinTicket } from "./join-tickets";
import { assertCryptographicReadiness } from "./readiness";
import { publicWebSocketProtocol, ticketFromRequest, verifyTicket } from "./tickets";
import {
  activateBeforeConfirm,
  checkpointObjectKey,
  commitSequentially,
  compensateAll,
  dispatchIdentityValue,
  dispatchRecoveryAction,
  materializeRoster,
  rosterIdentityValue,
} from "./dispatch-invariants";
import type {
  ClaimedQueueEntry,
  DispatchRequest,
  Env,
  InputPool,
  InventoryStack,
  IslandCheckpointCommand,
  MatchAllocation,
  MatchmakingEnqueueCommand,
  Playlist,
  TicketClaims,
} from "./types";
import {
  ApiError,
  assertObject,
  bytesToBase64Url,
  canonicalJson,
  errorResponse,
  inventoryStacks,
  jsonResponse,
  mapInBatches,
  parsePositiveConfigInt,
  randomHex128,
  randomToken,
  readJson,
  requestId,
  requireHex128,
  requireIdempotencyKey,
  requireIdentifier,
  requireInteger,
  requireString,
  sha256Base64Url,
  sha256Hex128,
} from "./util";

interface MatchmakingBody {
  region: string;
  playlist: Playlist;
  inputPool: InputPool;
  reservationIds: Record<string, string>;
}

interface ClaimResponse {
  leaseId: string;
  matchId: string;
  playerCount: number;
  entries: ClaimedQueueEntry[];
}

interface ProfileView {
  accountId: string;
  rating: number;
  version: number;
  island: {
    checkpointId: string;
    version: number;
    logicalTimeMs: number;
    contentBuild: string;
    objectKey: string;
    sha256: string;
    sizeBytes: number;
    etag: string;
    updatedAtMs: number;
  } | null;
}

interface DemoJoinBody {
  room: string;
  resumeToken?: string;
  joinNonce?: string;
}

interface DemoClaim {
  matchId: string;
  matchEpoch: number;
  accountId: string;
  playerSlot: number;
  teamId: number;
  resumeToken: string;
  playerCount: number;
  createdPlayer: boolean;
}

interface DemoAdmission {
  sourceKey: string;
  windowStartedAtMs: number;
}

const PUBLIC_DEMO_ROOM_RE = /^duel-(?:0[1-9]|[12][0-9]|3[0-2])$/u;
const SMOKE_DEMO_ROOM_RE = /^smoke-[0-9a-f]{32}$/u;
const DEMO_JOIN_NONCE_RE = /^demo_join_[A-Za-z0-9_-]{24}$/u;
const DEMO_JOIN_WINDOW_MS = 5 * 60 * 1_000;
const DEMO_NEW_CLAIMS_PER_WINDOW = 2;
const DEMO_RESUMES_PER_WINDOW = 20;

export async function handleRequest(request: Request, env: Env): Promise<Response> {
  const id = requestId(request);
  try {
    const url = new URL(request.url);
    const internal = url.pathname.startsWith("/internal/");
    if (!internal) enforceOrigin(request, env);

    if (request.method === "OPTIONS" && !internal) {
      return withCommonHeaders(new Response(null, { status: 204 }), request, env, id);
    }

    let response: Response;
    if (request.method === "GET" && url.pathname === "/healthz") {
      response = jsonResponse({
        status: "ok",
        service: "aetherloom-control-plane",
        environment: env.ENVIRONMENT,
        authoritativeHz: 128,
        buildHash: env.CONTENT_BUILD_HASH,
      });
    } else if (request.method === "GET" && url.pathname === "/v1/profile") {
      const claims = await playerSession(request, env);
      response = await profileFetch(env, claims.sub, "/profile", "GET");
    } else if (request.method === "POST" && url.pathname === "/v1/profile/reservations") {
      response = await reserveLoadout(request, env);
    } else if (request.method === "POST" && url.pathname === "/v1/profile/reservations/cancel") {
      response = await cancelReservation(request, env);
    } else if (request.method === "GET" && url.pathname === "/v1/profile/island") {
      response = await islandMetadata(request, env);
    } else if (request.method === "GET" && url.pathname === "/v1/profile/island/checkpoint") {
      response = await downloadIslandCheckpoint(request, env);
    } else if (request.method === "GET" && url.pathname === "/v1/profile/island/runtime") {
      const claims = await playerSession(request, env);
      response = await profileFetch(env, claims.sub, "/island/runtime", "GET");
    } else if (request.method === "POST" && url.pathname === "/v1/profile/island/commands") {
      response = await advanceIsland(request, env);
    } else if (request.method === "POST" && url.pathname === "/v1/profile/island/deactivate") {
      const claims = await playerSession(request, env);
      response = await profileFetch(env, claims.sub, "/island/runtime/deactivate", "POST");
    } else if (
      request.method === "PUT" &&
      url.pathname.startsWith("/internal/v1/islands/checkpoints/")
    ) {
      response = await uploadIslandCheckpoint(request, env, url.pathname);
    } else if (request.method === "POST" && url.pathname === "/v1/matchmaking/enqueue") {
      response = await enqueueMatchmaking(request, env);
    } else if (request.method === "POST" && url.pathname === "/v1/matchmaking/status") {
      response = await matchmakingStatus(request, env, false);
    } else if (request.method === "POST" && url.pathname === "/v1/matchmaking/cancel") {
      response = await matchmakingStatus(request, env, true);
    } else if (request.method === "POST" && url.pathname === "/v1/demo/join") {
      response = await joinMultiplayerDemo(request, env, false);
    } else if (
      request.method === "GET" &&
      url.pathname.startsWith("/v1/demo/ws/")
    ) {
      return await routeDemoBrowserWebSocket(request, env, url.pathname);
    } else if (
      request.method === "GET" &&
      url.pathname.startsWith("/v1/ws/casual/")
    ) {
      return await routeBrowserWebSocket(request, env, url.pathname);
    } else if (
      request.method === "POST" &&
      url.pathname === "/internal/v1/demo/smoke/join"
    ) {
      await serviceRequest(request, env, ["deployment:verify"]);
      response = await joinMultiplayerDemo(request, env, true);
    } else if (
      request.method === "POST" &&
      url.pathname === "/internal/v1/matchmaking/dispatch"
    ) {
      await serviceRequest(request, env, ["match:dispatch"]);
      response = await dispatchMatch(request, env);
    } else if (
      request.method === "POST" &&
      url.pathname === "/internal/v1/settlements"
    ) {
      await serviceRequest(request, env, ["result:enqueue"]);
      response = await acceptSignedSettlement(request, env);
    } else if (
      request.method === "POST" &&
      url.pathname === "/internal/v1/readiness"
    ) {
      await serviceRequest(request, env, ["deployment:verify"]);
      await assertCryptographicReadiness(env);
      response = jsonResponse({
        status: "ready",
        environment: env.ENVIRONMENT,
        buildHash: env.CONTENT_BUILD_HASH,
        authentication: "verified",
        cryptography: "verified",
      });
    } else {
      response = jsonResponse({ error: { code: "not_found", message: "Route not found.", requestId: id } }, 404);
    }
    return withCommonHeaders(response, request, env, id);
  } catch (error) {
    return withCommonHeaders(errorResponse(error, id), request, env, id);
  }
}

function requireMultiplayerDemo(env: Env): void {
  if (
    env.ENABLE_MULTIPLAYER_DEMO !== "true" ||
    (env.ENVIRONMENT !== "staging" && env.ENVIRONMENT !== "local")
  ) {
    throw new ApiError(
      404,
      "multiplayer_demo_disabled",
      "The multiplayer systems test is not enabled in this environment.",
    );
  }
  if (env.BROWSER_MATCH_ORIGIN === undefined) {
    throw new ApiError(
      503,
      "browser_match_unavailable",
      "The browser match service is not configured.",
    );
  }
}

async function joinMultiplayerDemo(
  request: Request,
  env: Env,
  deploymentSmoke: boolean,
): Promise<Response> {
  requireMultiplayerDemo(env);
  const body = await readJson<DemoJoinBody>(request, 2_048);
  assertObject(body, "multiplayer demo join request");
  const room = requireString(body.room, "room", 64).trim().toLowerCase();
  const validRoom = deploymentSmoke
    ? SMOKE_DEMO_ROOM_RE.test(room)
    : PUBLIC_DEMO_ROOM_RE.test(room);
  if (!validRoom) {
    throw new ApiError(
      400,
      "invalid_demo_room",
      deploymentSmoke
        ? "Deployment smoke rooms must use a random smoke identifier."
        : "The staging systems test provides rooms duel-01 through duel-32.",
    );
  }
  const resumeToken =
    body.resumeToken === undefined
      ? undefined
      : requireString(body.resumeToken, "resumeToken", 128);
  const joinNonce =
    body.joinNonce === undefined
      ? undefined
      : requireString(body.joinNonce, "joinNonce", 64);
  if (
    (resumeToken === undefined && !DEMO_JOIN_NONCE_RE.test(joinNonce ?? "")) ||
    (resumeToken !== undefined && joinNonce !== undefined)
  ) {
    throw new ApiError(
      400,
      "invalid_join_attempt",
      "A fresh join requires one valid join nonce; a resume must omit it.",
    );
  }
  const buildHash = requireHex128(env.CONTENT_BUILD_HASH, "CONTENT_BUILD_HASH");
  const admission = deploymentSmoke
    ? undefined
    : await enforceDemoAdmission(request, env, buildHash, resumeToken !== undefined);
  const matchId = await sha256Hex128(`aetherloom-demo:${buildHash}:${room}`);
  const claimResponse = await env.BROWSER_MATCH_ORIGIN!.fetch(
    new Request("https://browser-match.internal/v1/demo/claims", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        room,
        matchId,
        buildHash,
        ...(resumeToken === undefined ? {} : { resumeToken }),
        ...(joinNonce === undefined ? {} : { joinNonce }),
      }),
    }),
  );
  if (!claimResponse.ok) {
    if (
      admission !== undefined &&
      claimResponse.status >= 400 &&
      claimResponse.status < 500
    ) {
      await refundDemoAdmission(env, admission);
    }
    return claimResponse;
  }
  const rawClaim = await readJson<DemoClaim>(claimResponse, 4_096);
  assertObject(rawClaim, "browser match claim");
  if (typeof rawClaim.createdPlayer !== "boolean") {
    throw new ApiError(
      502,
      "browser_match_claim_invalid",
      "The browser match service returned an invalid admission result.",
    );
  }
  const claim: DemoClaim = {
    matchId: requireHex128(rawClaim.matchId, "claim matchId"),
    matchEpoch: requireInteger(
      rawClaim.matchEpoch,
      "claim matchEpoch",
      1,
      Number.MAX_SAFE_INTEGER,
    ),
    accountId: requireHex128(rawClaim.accountId, "claim accountId"),
    playerSlot: requireInteger(rawClaim.playerSlot, "claim playerSlot", 0, 7),
    teamId: requireInteger(rawClaim.teamId, "claim teamId", 0, 7),
    resumeToken: requireString(rawClaim.resumeToken, "claim resumeToken", 128),
    playerCount: requireInteger(rawClaim.playerCount, "claim playerCount", 1, 8),
    createdPlayer: rawClaim.createdPlayer,
  };
  if (claim.matchId !== matchId) {
    throw new ApiError(
      502,
      "browser_match_claim_mismatch",
      "The browser match service returned a claim for a different match.",
    );
  }
  if (
    admission !== undefined &&
    resumeToken === undefined &&
    !claim.createdPlayer
  ) {
    await refundDemoAdmission(env, admission);
  }

  const now = Math.floor(Date.now() / 1_000);
  const ticket = await signJoinTicket(
    {
      v: 1,
      iss: env.TICKET_ISSUER,
      aud: env.JOIN_TICKET_AUDIENCE,
      purpose: "join",
      sub: claim.accountId,
      iat: now,
      nbf: now - 2,
      exp: now + 120,
      nonce: randomHex128(),
      match_id: matchId,
      match_epoch: claim.matchEpoch,
      region: "staging",
      build_hash: buildHash,
      input_pool: "browser",
      player_slot: claim.playerSlot,
      team_id: claim.teamId,
    },
    env.JOIN_TICKET_SIGNING_KEYS_JSON,
    env.ACTIVE_JOIN_TICKET_KID,
  );
  const requestUrl = new URL(request.url);
  requestUrl.protocol = requestUrl.protocol === "https:" ? "wss:" : "ws:";
  requestUrl.pathname = `/v1/demo/ws/${matchId}`;
  requestUrl.search = "";
  requestUrl.hash = "";
  return jsonResponse({
    matchId,
    matchEpoch: claim.matchEpoch,
    accountId: claim.accountId,
    slot: claim.playerSlot,
    teamId: claim.teamId,
    resumeToken: claim.resumeToken,
    playerCount: claim.playerCount,
    webSocketUrl: requestUrl.toString(),
    ticket,
    tickHz: 128,
    inputHz: 64,
    snapshotHz: 32,
    progression: "disabled",
    competitive: false,
  });
}

async function enforceDemoAdmission(
  request: Request,
  env: Env,
  buildHash: string,
  resume: boolean,
): Promise<DemoAdmission> {
  let source = request.headers.get("cf-connecting-ip");
  if (env.ENVIRONMENT === "local" && source === null) source = "local-development";
  if (source === null || source.length < 2 || source.length > 64) {
    throw new ApiError(
      403,
      "demo_source_unavailable",
      "The multiplayer systems test could not verify the connection source.",
    );
  }
  const now = Date.now();
  const cutoff = now - DEMO_JOIN_WINDOW_MS;
  const sourceKey = await sha256Base64Url(
    `aetherloom-demo-admission:${buildHash}:${resume ? "resume" : "new"}:${source}`,
  );
  const limit = resume ? DEMO_RESUMES_PER_WINDOW : DEMO_NEW_CLAIMS_PER_WINDOW;
  const result = await env.CONTROL_DB.prepare(
    `INSERT INTO demo_join_limits (
       source_key, window_started_at_ms, claim_count, updated_at_ms
     ) VALUES (?1, ?2, 1, ?2)
     ON CONFLICT(source_key) DO UPDATE SET
       window_started_at_ms = CASE
         WHEN demo_join_limits.window_started_at_ms <= ?3 THEN excluded.window_started_at_ms
         ELSE demo_join_limits.window_started_at_ms
       END,
       claim_count = CASE
         WHEN demo_join_limits.window_started_at_ms <= ?3 THEN 1
         ELSE min(demo_join_limits.claim_count + 1, ?4)
       END,
       updated_at_ms = excluded.updated_at_ms
     RETURNING claim_count, window_started_at_ms`,
  )
    .bind(sourceKey, now, cutoff, limit + 1)
    .first<{ claim_count: number; window_started_at_ms: number }>();
  if (
    result === null ||
    !Number.isInteger(result.claim_count) ||
    result.claim_count < 1 ||
    !Number.isInteger(result.window_started_at_ms) ||
    result.window_started_at_ms < 0
  ) {
    throw new Error("Demo admission limiter returned an invalid result.");
  }
  if (result.claim_count > limit) {
    throw new ApiError(
      429,
      "demo_join_rate_limited",
      resume
        ? "This connection is reconnecting too frequently. Wait before trying again."
        : "This connection has already opened two new staging players. Reuse an existing tab or wait five minutes.",
    );
  }
  return {
    sourceKey,
    windowStartedAtMs: result.window_started_at_ms,
  };
}

async function refundDemoAdmission(
  env: Env,
  admission: DemoAdmission,
): Promise<void> {
  try {
    await env.CONTROL_DB.prepare(
      `UPDATE demo_join_limits
       SET claim_count = max(claim_count - 1, 0),
           updated_at_ms = ?3
       WHERE source_key = ?1
         AND window_started_at_ms = ?2
         AND claim_count > 0`,
    )
      .bind(admission.sourceKey, admission.windowStartedAtMs, Date.now())
      .run();
  } catch (error) {
    // A failed claim must remain the primary response. Ambiguous limiter
    // failures are surfaced in Worker logs and never mint extra capacity.
    console.error("Could not refund rejected demo admission.", { error });
  }
}

async function routeDemoBrowserWebSocket(
  request: Request,
  env: Env,
  pathname: string,
): Promise<Response> {
  requireMultiplayerDemo(env);
  if (request.headers.get("upgrade")?.toLowerCase() !== "websocket") {
    throw new ApiError(426, "websocket_required", "This endpoint requires a WebSocket upgrade.");
  }
  const matchId = requireHex128(pathname.slice("/v1/demo/ws/".length), "matchId");
  const token = ticketFromRequest(request, true);
  await verifyJoinTicket(token, env.JOIN_TICKET_PUBLIC_KEYS_JSON, {
    issuer: env.TICKET_ISSUER,
    audience: env.JOIN_TICKET_AUDIENCE,
    matchId,
    buildHash: requireHex128(env.CONTENT_BUILD_HASH, "CONTENT_BUILD_HASH"),
    inputPool: "browser",
  });
  const protocol = publicWebSocketProtocol(request, "aetherloom.v3");
  const headers = new Headers(request.headers);
  headers.set("authorization", `Bearer ${token}`);
  headers.set("sec-websocket-protocol", protocol);
  headers.delete("cookie");
  return env.BROWSER_MATCH_ORIGIN!.fetch(
    new Request(
      `https://browser-match.internal/v1/demo/matches/${encodeURIComponent(matchId)}/ws`,
      {
        method: "GET",
        headers,
      },
    ),
  );
}

async function advanceIsland(request: Request, env: Env): Promise<Response> {
  const claims = await playerSession(request, env);
  const sequence = requireInteger(
    Number(request.headers.get("x-aetherloom-command-sequence")),
    "X-Aetherloom-Command-Sequence",
    1,
    9_007_199_254_740_991,
  );
  const declaredLength = Number(request.headers.get("content-length") ?? "0");
  if (declaredLength > 4_096) {
    throw new ApiError(413, "island_commands_too_large", "Island command batch exceeds 4096 bytes.");
  }
  const commands = await request.arrayBuffer();
  if (commands.byteLength === 0 || commands.byteLength > 4_096) {
    throw new ApiError(413, "island_commands_too_large", "Island command batch must be 1-4096 bytes.");
  }
  return profileFetch(env, claims.sub, "/island/runtime/commands", "POST", {
    accountId: claims.sub,
    sequence,
    commandsBase64: bytesToBase64Url(new Uint8Array(commands)),
  });
}

async function reserveLoadout(request: Request, env: Env): Promise<Response> {
  const claims = await playerSession(request, env);
  const idempotencyKey = requireIdempotencyKey(request.headers.get("idempotency-key"));
  const body = await readJson<{ loadout: InventoryStack[] }>(request);
  assertObject(body, "reservation request");
  const loadout = inventoryStacks(body.loadout, "loadout");
  return profileFetch(env, claims.sub, "/reservations", "POST", {
    accountId: claims.sub,
    reservationId: idempotencyKey,
    loadout,
    expiresAtMs: Date.now() + 10 * 60 * 1_000,
  });
}

async function cancelReservation(request: Request, env: Env): Promise<Response> {
  const claims = await playerSession(request, env);
  const body = await readJson<{ reservationId: string }>(request);
  assertObject(body, "reservation cancellation");
  return profileFetch(env, claims.sub, "/reservations/cancel", "POST", {
    accountId: claims.sub,
    reservationId: requireIdentifier(body.reservationId, "reservationId"),
  });
}

async function islandMetadata(request: Request, env: Env): Promise<Response> {
  const claims = await playerSession(request, env);
  const response = await profileFetch(env, claims.sub, "/island", "GET");
  const view = await readJson<{ accountId: string; checkpoint: ProfileView["island"] }>(response);
  return jsonResponse({
    accountId: view.accountId,
    checkpoint: publicCheckpoint(view.checkpoint),
  });
}

async function downloadIslandCheckpoint(request: Request, env: Env): Promise<Response> {
  const claims = await playerSession(request, env);
  const metadataResponse = await profileFetch(env, claims.sub, "/island", "GET");
  const metadata = await readJson<{ checkpoint: ProfileView["island"] }>(metadataResponse);
  if (metadata.checkpoint === null) {
    throw new ApiError(404, "island_checkpoint_not_found", "No island checkpoint exists.");
  }
  const object = await env.MATCH_ARCHIVE.get(metadata.checkpoint.objectKey);
  if (object === null) {
    throw new ApiError(503, "island_checkpoint_unavailable", "Island checkpoint object is unavailable.");
  }
  const headers = new Headers();
  headers.set("content-type", "application/octet-stream");
  headers.set("cache-control", "private, no-store");
  headers.set("etag", object.httpEtag);
  headers.set("x-aetherloom-checkpoint-id", metadata.checkpoint.checkpointId);
  headers.set("x-aetherloom-checkpoint-version", String(metadata.checkpoint.version));
  headers.set("x-aetherloom-checkpoint-sha256", metadata.checkpoint.sha256);
  headers.set("x-aetherloom-content-build", metadata.checkpoint.contentBuild);
  return new Response(object.body, { headers });
}

async function uploadIslandCheckpoint(
  request: Request,
  env: Env,
  pathname: string,
): Promise<Response> {
  await serviceRequest(request, env, ["island:checkpoint"]);
  const accountId = requireIdentifier(
    request.headers.get("x-aetherloom-account-id"),
    "X-Aetherloom-Account-Id",
  );
  const checkpointId = requireIdempotencyKey(
    pathname.slice("/internal/v1/islands/checkpoints/".length),
    "checkpointId",
  );
  const expectedVersion = requireInteger(
    Number(request.headers.get("x-aetherloom-expected-version")),
    "X-Aetherloom-Expected-Version",
    0,
    2_147_483_647,
  );
  const logicalTimeMs = requireInteger(
    Number(request.headers.get("x-aetherloom-logical-time-ms")),
    "X-Aetherloom-Logical-Time-Ms",
    0,
    9_007_199_254_740_991,
  );
  const contentBuild = requireIdentifier(
    request.headers.get("x-aetherloom-content-build"),
    "X-Aetherloom-Content-Build",
  );
  if (contentBuild !== env.CONTENT_BUILD_HASH) {
    throw new ApiError(409, "content_build_mismatch", "Checkpoint build does not match the online build.");
  }
  const maximumBytes = parsePositiveConfigInt(
    env.MAX_ISLAND_CHECKPOINT_BYTES,
    "MAX_ISLAND_CHECKPOINT_BYTES",
    1,
    64 * 1024 * 1024,
  );
  const declaredLength = Number(request.headers.get("content-length") ?? "0");
  if (declaredLength > maximumBytes) {
    throw new ApiError(413, "checkpoint_too_large", `Checkpoint exceeds ${maximumBytes} bytes.`);
  }
  const bytes = await request.arrayBuffer();
  if (bytes.byteLength === 0 || bytes.byteLength > maximumBytes) {
    throw new ApiError(413, "checkpoint_too_large", `Checkpoint must be 1-${maximumBytes} bytes.`);
  }
  const sha256 = await sha256Base64Url(bytes);
  const objectKey = checkpointObjectKey(accountId, sha256);
  const object = await env.MATCH_ARCHIVE.put(objectKey, bytes, {
    httpMetadata: { contentType: "application/octet-stream" },
    customMetadata: {
      accountId,
      checkpointId,
      contentBuild,
      sha256,
    },
  });
  const command: IslandCheckpointCommand = {
    accountId,
    checkpointId,
    expectedVersion,
    logicalTimeMs,
    contentBuild,
    objectKey,
    sha256,
    sizeBytes: bytes.byteLength,
    etag: object.etag,
  };
  const checkpointResponse = await profileFetch(
    env,
    accountId,
    "/island/checkpoints",
    "PUT",
    command,
  );
  const result = await readJson<{ checkpoint: ProfileView["island"] }>(checkpointResponse);
  if (result.checkpoint === null) throw new Error("Profile object returned an empty checkpoint.");
  await env.CONTROL_DB.prepare(
    `INSERT INTO profile_projection(account_id, rating, profile_version, island_version, updated_at_ms)
     VALUES(?, 1000, 1, ?, ?)
     ON CONFLICT(account_id) DO UPDATE SET
       island_version = max(island_version, excluded.island_version),
       updated_at_ms = max(updated_at_ms, excluded.updated_at_ms)`,
  )
    .bind(accountId, result.checkpoint.version, Date.now())
    .run();
  return jsonResponse({ accountId, checkpoint: publicCheckpoint(result.checkpoint) }, checkpointResponse.status);
}

async function enqueueMatchmaking(request: Request, env: Env): Promise<Response> {
  const claims = await playerSession(request, env);
  const queueTicketId = requireIdempotencyKey(request.headers.get("idempotency-key"));
  const body = validateMatchmakingBody(await readJson<unknown>(request));
  enforcePlatformPool(claims, body);
  const members = [...(claims.party?.members ?? [claims.sub])].sort();
  const partyId = claims.party?.party_id ?? `solo-${claims.sub}`;
  const reservationIds = validateReservationMap(body.reservationIds, members);
  const profiles = await mapInBatches(members, 8, (accountId) => profileView(env, accountId));
  const skillMmr = Math.round(
    profiles.reduce((total, profile) => total + profile.rating, 0) / profiles.length,
  );
  const shardCount = parsePositiveConfigInt(
    env.MATCHMAKING_SHARD_COUNT,
    "MATCHMAKING_SHARD_COUNT",
    1,
    256,
  );
  const skillBucketWidth = parsePositiveConfigInt(
    env.MATCHMAKING_SKILL_BUCKET_WIDTH,
    "MATCHMAKING_SKILL_BUCKET_WIDTH",
    25,
    1_000,
  );
  const shard = matchmakingShardName(
    body.region,
    body.playlist,
    body.inputPool,
    skillMmr,
    partyId,
    shardCount,
    skillBucketWidth,
  );
  const requestHash = await sha256Base64Url(
    canonicalJson({ body, members, partyId, reservationIds, skillMmr }),
  );
  const command: MatchmakingEnqueueCommand = {
    queueTicketId,
    partyId,
    accountIds: members,
    reservationIds,
    region: body.region,
    playlist: body.playlist,
    inputPool: body.inputPool,
    skillMmr,
    expiresAtMs: Date.now() + 5 * 60 * 1_000,
    requestHash,
  };
  const response = await matchmakingFetch(env, shard, "/enqueue", command);
  const result = await readJson<Record<string, unknown>>(response);
  return jsonResponse({ ...result, shard }, response.status);
}

async function matchmakingStatus(request: Request, env: Env, cancel: boolean): Promise<Response> {
  const claims = await playerSession(request, env);
  const body = await readJson<{ shard: string; queueTicketId: string }>(request);
  assertObject(body, "matchmaking ticket request");
  const shard = requireIdentifier(body.shard, "shard");
  const queueTicketId = requireIdentifier(body.queueTicketId, "queueTicketId");
  return matchmakingFetch(env, shard, cancel ? "/cancel" : "/status", {
    shard,
    queueTicketId,
    accountId: claims.sub,
  });
}

async function dispatchMatch(request: Request, env: Env): Promise<Response> {
  const dispatch = validateDispatch(await readJson<unknown>(request));
  if (dispatch.buildHash !== env.CONTENT_BUILD_HASH) {
    throw new ApiError(409, "content_build_mismatch", "Dispatch build does not match this control plane.");
  }
  const dispatchHash = await sha256Base64Url(dispatchIdentityValue(dispatch));
  const director = new ServiceBindingMatchDirector(env);
  const resumed = await director.resumeDispatch(dispatch.matchId, dispatchHash);
  if (resumed !== null) {
    const outcome = await finishStagedDispatch(
      env,
      director,
      dispatch.matchId,
      dispatchHash,
      resumed,
    );
    if (outcome !== "active") {
      throw new ApiError(
        409,
        outcome === "aborted" ? "dispatch_expired" : "confirmed_match_not_joinable",
        outcome === "aborted"
          ? "Staged dispatch expired and was safely compensated."
          : "Queue confirmation succeeded, but its assignment is no longer joinable.",
      );
    }
    return jsonResponse({
      matchId: dispatch.matchId,
      matchEpoch: resumed.allocation.matchEpoch,
      status: "active",
      hostId: resumed.allocation.hostId,
      transport: resumed.allocation.transport,
      playerCount: resumed.playerCount,
      botCount: resumed.botCount,
      idempotentReplay: true,
    });
  }

  const requestedLeaseId = randomToken("lease");
  const claimResponse = await matchmakingFetch(env, dispatch.shard, "/claim", {
    leaseId: requestedLeaseId,
    matchId: dispatch.matchId,
    maxPlayers: dispatch.targetPlayers,
    leaseExpiresAtMs: Date.now() + 30_000,
  });
  const claim = await readJson<ClaimResponse>(claimResponse);
  const leaseId = requireIdentifier(claim.leaseId, "claim leaseId");
  if (leaseId !== requestedLeaseId || claim.matchId !== dispatch.matchId) {
    throw new ApiError(502, "claim_match_mismatch", "Matchmaking returned a claim for another match.");
  }
  const claimedPlayerCount = requireInteger(
    claim.playerCount,
    "claim playerCount",
    0,
    dispatch.targetPlayers,
  );
  if (claimedPlayerCount === 0) {
    return jsonResponse({ matchId: dispatch.matchId, status: "waiting", playerCount: 0 }, 202);
  }
  if (!dispatch.allowBots && claimedPlayerCount < dispatch.targetPlayers) {
    await matchmakingFetch(env, dispatch.shard, "/release", { leaseId });
    return jsonResponse(
      {
        matchId: dispatch.matchId,
        status: "waiting",
        playerCount: claimedPlayerCount,
        targetPlayers: dispatch.targetPlayers,
      },
      202,
    );
  }

  const botCount = dispatch.allowBots ? dispatch.targetPlayers - claimedPlayerCount : 0;
  const players = materializeRoster(claim.entries, claimedPlayerCount, dispatch.playlist);
  const rosterHash = await sha256Base64Url(rosterIdentityValue(players));
  let allocation: MatchAllocation | undefined;
  let preserveStagedRecovery = false;
  try {
    allocation = await director.reserve({
      matchId: dispatch.matchId,
      matchEpoch: dispatch.matchEpoch,
      dispatchHash,
      rosterHash,
      region: dispatch.region,
      playlist: dispatch.playlist,
      inputPool: dispatch.inputPool,
      buildHash: dispatch.buildHash,
      playerCount: claimedPlayerCount,
      botCount,
      authoritativeHz: 128,
    });
    const activeAllocation = allocation;
    const expiresSeconds = Math.min(
      Math.floor(activeAllocation.expiresAtMs / 1_000),
      Math.floor(Date.now() / 1_000) + 120,
    );
    const assignments = Object.fromEntries(
      await Promise.all(
        players.map(async (player) => {
          const now = Math.floor(Date.now() / 1_000);
          const joinTicket = await signJoinTicket(
            {
              v: 1,
              iss: env.TICKET_ISSUER,
              aud: env.JOIN_TICKET_AUDIENCE,
              purpose: "join",
              sub: player.accountId,
              iat: now,
              nbf: now - 2,
              exp: expiresSeconds,
              nonce: randomHex128(),
              match_id: dispatch.matchId,
              match_epoch: dispatch.matchEpoch,
              region: dispatch.region,
              build_hash: dispatch.buildHash,
              input_pool: dispatch.inputPool,
              player_slot: player.playerSlot,
              team_id: player.teamId,
            },
            env.JOIN_TICKET_SIGNING_KEYS_JSON,
            env.ACTIVE_JOIN_TICKET_KID,
          );
          const endpoint =
            activeAllocation.transport === "wss"
              ? `/v1/ws/casual/${dispatch.matchId}`
              : (activeAllocation.nativeEndpoint ?? "");
          return [
            player.accountId,
            {
              matchId: dispatch.matchId,
              matchEpoch: dispatch.matchEpoch,
              joinTicket,
              transport: activeAllocation.transport,
              endpoint,
              expiresAtMs: expiresSeconds * 1_000,
            },
          ];
        }),
      ),
    );
    const reservations = players.map(({ accountId, reservationId }) => ({
      accountId,
      reservationId,
    }));
    const confirmation = {
      shard: dispatch.shard,
      leaseId,
      assignments,
      reservations,
    };
    if (
      canonicalJson(assignments).length + canonicalJson(reservations).length >
      256 * 1024
    ) {
      throw new ApiError(400, "assignments_too_large", "Match recovery journal exceeds its limit.");
    }
    let staged: ResumableDispatch;
    try {
      staged = await director.stageQueueConfirmation(
        dispatch.matchId,
        activeAllocation.hostId,
        dispatchHash,
        rosterHash,
        confirmation,
      );
      preserveStagedRecovery = true;
    } catch (error) {
      preserveStagedRecovery =
        !(error instanceof ApiError) ||
        !["allocation_not_stageable", "assignments_too_large", "invalid_request"].includes(
          error.code,
        );
      throw error;
    }
    const outcome = await finishStagedDispatch(
      env,
      director,
      dispatch.matchId,
      dispatchHash,
      staged,
    );
    if (outcome !== "active") {
      throw new ApiError(
        409,
        outcome === "aborted" ? "dispatch_expired" : "confirmed_match_not_joinable",
        outcome === "aborted"
          ? "Staged dispatch expired and was safely compensated."
          : "Queue confirmation succeeded, but its assignment is no longer joinable.",
      );
    }
    return jsonResponse(
      {
        matchId: dispatch.matchId,
        status: "active",
        hostId: allocation.hostId,
        transport: allocation.transport,
        playerCount: claimedPlayerCount,
        botCount,
      },
      201,
    );
  } catch (error) {
    if (preserveStagedRecovery) throw error;
    if (allocation !== undefined) {
      await director
        .release(dispatch.matchId, allocation.hostId, "dispatch-failed")
        .catch((releaseError) => console.error("match allocation release failed", { releaseError }));
    }
    await matchmakingFetch(env, dispatch.shard, "/release", { leaseId }).catch((releaseError) =>
      console.error("matchmaking lease release failed", { releaseError }),
    );
    throw error;
  }
}

async function finishStagedDispatch(
  env: Env,
  director: ServiceBindingMatchDirector,
  matchId: string,
  dispatchHash: string,
  dispatch: ResumableDispatch,
): Promise<"active" | "aborted" | "confirmed-expired"> {
  const recoveryAction = dispatchRecoveryAction(
    dispatch.phase,
    dispatch.allocation.expiresAtMs,
    Object.values(dispatch.queueConfirmation.assignments).map(
      (assignment) => assignment.expiresAtMs,
    ),
    Date.now(),
  );
  if (recoveryAction === "done") return "active";
  if (recoveryAction === "abort") {
    if (dispatch.phase !== "aborting") {
      const aborting = await director.beginAbort(
        matchId,
        dispatch.allocation.hostId,
        dispatchHash,
        dispatch.rosterHash,
      );
      if (!aborting) return "active";
    }
    return abortStagedDispatch(env, director, matchId, dispatchHash, dispatch);
  }
  const assignmentExpiresAtMs = Math.min(
    ...Object.values(dispatch.queueConfirmation.assignments).map(
      (assignment) => assignment.expiresAtMs,
    ),
  );
  await matchmakingFetch(env, dispatch.queueConfirmation.shard, "/prepare", {
    leaseId: dispatch.queueConfirmation.leaseId,
    matchId,
    assignments: dispatch.queueConfirmation.assignments,
    holdExpiresAtMs: assignmentExpiresAtMs,
  });
  if (dispatch.phase === "staged") {
    const attempted: Array<{ accountId: string; reservationId: string }> = [];
    try {
      await commitSequentially(
        dispatch.queueConfirmation.reservations,
        attempted,
        async (reservation) => {
          await profileFetch(env, reservation.accountId, "/reservations/commit", "POST", {
            accountId: reservation.accountId,
            reservationId: reservation.reservationId,
            matchId,
          });
        },
      );
    } catch {
      const aborting = await director.beginAbort(
        matchId,
        dispatch.allocation.hostId,
        dispatchHash,
        dispatch.rosterHash,
      );
      if (!aborting) return "active";
      return abortStagedDispatch(env, director, matchId, dispatchHash, {
        ...dispatch,
        phase: "aborting",
      });
    }
    const postCommitAction = dispatchRecoveryAction(
      "staged",
      dispatch.allocation.expiresAtMs,
      Object.values(dispatch.queueConfirmation.assignments).map(
        (assignment) => assignment.expiresAtMs,
      ),
      Date.now(),
    );
    if (postCommitAction === "abort") {
      const aborting = await director.beginAbort(
        matchId,
        dispatch.allocation.hostId,
        dispatchHash,
        dispatch.rosterHash,
      );
      if (!aborting) return "active";
      return abortStagedDispatch(env, director, matchId, dispatchHash, {
        ...dispatch,
        phase: "aborting",
      });
    }
  }
  await activateBeforeConfirm(
    async () => {
      if (dispatch.phase === "staged") {
        await director.activate(
          matchId,
          dispatch.allocation.hostId,
          dispatchHash,
          dispatch.rosterHash,
        );
      }
    },
    async () => {
      await matchmakingFetch(env, dispatch.queueConfirmation.shard, "/confirm", {
        leaseId: dispatch.queueConfirmation.leaseId,
        matchId,
        assignments: dispatch.queueConfirmation.assignments,
      });
    },
  );
  await director.markQueueConfirmed(
    matchId,
    dispatch.allocation.hostId,
    dispatchHash,
    dispatch.queueConfirmation.leaseId,
  );
  return "active";
}

async function abortStagedDispatch(
  env: Env,
  director: ServiceBindingMatchDirector,
  matchId: string,
  dispatchHash: string,
  dispatch: ResumableDispatch,
): Promise<"active" | "aborted" | "confirmed-expired"> {
  try {
    await matchmakingFetch(env, dispatch.queueConfirmation.shard, "/abort-prepared", {
      leaseId: dispatch.queueConfirmation.leaseId,
      matchId,
    });
  } catch (error) {
    if (!(error instanceof ApiError) || error.code !== "lease_already_confirmed") throw error;
    const assignmentExpiresAtMs = Math.min(
      ...Object.values(dispatch.queueConfirmation.assignments).map(
        (assignment) => assignment.expiresAtMs,
      ),
    );
    if (
      assignmentExpiresAtMs <= Date.now() + 5_000 ||
      dispatch.allocation.expiresAtMs <= Date.now() + 5_000
    ) {
      await director.recordExpiredConfirmedAbort(
        matchId,
        dispatch.allocation.hostId,
        dispatchHash,
        dispatch.rosterHash,
        dispatch.queueConfirmation.leaseId,
      );
      return "confirmed-expired";
    }
    await director.restoreActiveAfterConfirmedAbort(
      matchId,
      dispatch.allocation.hostId,
      dispatchHash,
      dispatch.rosterHash,
    );
    await director.markQueueConfirmed(
      matchId,
      dispatch.allocation.hostId,
      dispatchHash,
      dispatch.queueConfirmation.leaseId,
    );
    return "active";
  }
  const rollbackErrors = await rollbackReservations(
    env,
    matchId,
    dispatch.queueConfirmation.reservations,
  );
  if (rollbackErrors.length > 0) {
    throw new ApiError(
      503,
      "dispatch_compensation_incomplete",
      "Dispatch remains in recovery until every reservation rollback succeeds.",
    );
  }
  await director.release(matchId, dispatch.allocation.hostId, "dispatch-expired");
  return "aborted";
}

async function rollbackReservations(
  env: Env,
  matchId: string,
  reservations: ReadonlyArray<{ accountId: string; reservationId: string }>,
): Promise<unknown[]> {
  const rollbackErrors = await compensateAll(reservations, async (reservation) => {
    await profileFetch(env, reservation.accountId, "/reservations/rollback", "POST", {
      accountId: reservation.accountId,
      reservationId: reservation.reservationId,
      matchId,
      expiresAtMs: Date.now() + 5 * 60 * 1_000,
    });
  });
  for (const rollbackError of rollbackErrors) {
    console.error("reservation rollback failed", { rollbackError });
  }
  return rollbackErrors;
}

async function routeBrowserWebSocket(
  request: Request,
  env: Env,
  pathname: string,
): Promise<Response> {
  if (request.headers.get("upgrade")?.toLowerCase() !== "websocket") {
    throw new ApiError(426, "websocket_required", "This endpoint requires a WebSocket upgrade.");
  }
  const matchId = requireHex128(pathname.slice("/v1/ws/casual/".length), "matchId");
  const token = ticketFromRequest(request, true);
  const claims = await verifyJoinTicket(token, env.JOIN_TICKET_PUBLIC_KEYS_JSON, {
    issuer: env.TICKET_ISSUER,
    audience: env.JOIN_TICKET_AUDIENCE,
    matchId,
    buildHash: requireHex128(env.CONTENT_BUILD_HASH, "CONTENT_BUILD_HASH"),
    inputPool: "browser",
  });
  const protocol = publicWebSocketProtocol(request);
  const allocation = await new ServiceBindingMatchDirector(env).browserAllocation(matchId);
  if (
    allocation.buildHash !== claims.build_hash ||
    allocation.matchEpoch !== claims.match_epoch
  ) {
    throw new ApiError(409, "match_build_mismatch", "Match is running a different content build.");
  }
  if (env.BROWSER_MATCH_ORIGIN === undefined) {
    throw new ApiError(503, "browser_match_unavailable", "Browser match service is not configured.");
  }
  const headers = new Headers(request.headers);
  headers.set("authorization", `Bearer ${token}`);
  headers.set("sec-websocket-protocol", protocol);
  headers.set("x-aetherloom-account-id", claims.sub);
  headers.set("x-aetherloom-match-id", matchId);
  headers.set("x-aetherloom-host-id", allocation.hostId);
  headers.delete("cookie");
  return env.BROWSER_MATCH_ORIGIN.fetch(
    new Request(`https://browser-match.internal/v1/matches/${encodeURIComponent(matchId)}/ws`, {
      method: "GET",
      headers,
    }),
  );
}

async function playerSession(request: Request, env: Env): Promise<TicketClaims> {
  return verifyTicket(ticketFromRequest(request), env.PLAYER_TICKET_KEYS_JSON, {
    issuer: env.TICKET_ISSUER,
    audience: env.PLAYER_TICKET_AUDIENCE,
    purpose: "session",
  });
}

async function serviceRequest(
  request: Request,
  env: Env,
  scopes: string[],
): Promise<TicketClaims> {
  return verifyTicket(ticketFromRequest(request), env.SERVICE_TICKET_KEYS_JSON, {
    issuer: env.TICKET_ISSUER,
    audience: env.SERVICE_TICKET_AUDIENCE,
    purpose: "service",
    requiredScopes: scopes,
  });
}

async function profileView(env: Env, accountId: string): Promise<ProfileView> {
  const response = await profileFetch(env, accountId, "/profile", "GET");
  if (!response.ok) throw new Error(`Profile lookup failed with ${response.status}.`);
  return readJson<ProfileView>(response);
}

async function profileFetch(
  env: Env,
  accountIdRaw: string,
  path: string,
  method: string,
  body?: unknown,
): Promise<Response> {
  const accountId = requireIdentifier(accountIdRaw, "accountId");
  const stub = env.PROFILE_ISLAND.get(env.PROFILE_ISLAND.idFromName(accountId));
  const response = await stub.fetch(
    new Request(`https://profile.internal${path}`, {
      method,
      headers: {
        "content-type": "application/json",
        "x-aetherloom-account-id": accountId,
      },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    }),
  );
  if (!response.ok) {
    throw await responseAsApiError(response);
  }
  return response;
}

async function matchmakingFetch(
  env: Env,
  shardRaw: string,
  path: string,
  body: unknown,
): Promise<Response> {
  const shard = requireIdentifier(shardRaw, "shard");
  const stub = env.MATCHMAKING_SHARD.get(env.MATCHMAKING_SHARD.idFromName(shard));
  const response = await stub.fetch(
    new Request(`https://matchmaking.internal${path}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    }),
  );
  if (!response.ok) throw await responseAsApiError(response);
  return response;
}

async function responseAsApiError(response: Response): Promise<ApiError> {
  let body: unknown;
  try {
    body = await response.json();
  } catch {
    return new ApiError(response.status, "upstream_error", "An internal state service rejected the request.");
  }
  const record = body as { error?: { code?: unknown; message?: unknown; details?: unknown } };
  return new ApiError(
    response.status,
    typeof record.error?.code === "string" ? record.error.code : "upstream_error",
    typeof record.error?.message === "string"
      ? record.error.message
      : "An internal state service rejected the request.",
    record.error?.details,
  );
}

function validateMatchmakingBody(value: unknown): MatchmakingBody {
  assertObject(value, "matchmaking request");
  const playlist = value.playlist;
  if (!["casual-extraction", "solo-extraction", "squad-extraction"].includes(String(playlist))) {
    throw new ApiError(400, "invalid_playlist", "Playlist is unsupported.");
  }
  const inputPool = value.inputPool;
  if (!["browser", "mouse-keyboard", "controller", "mixed"].includes(String(inputPool))) {
    throw new ApiError(400, "invalid_input_pool", "Input pool is unsupported.");
  }
  assertObject(value.reservationIds, "reservationIds");
  return {
    region: requireIdentifier(value.region, "region"),
    playlist: playlist as Playlist,
    inputPool: inputPool as InputPool,
    reservationIds: value.reservationIds as Record<string, string>,
  };
}

function validateReservationMap(
  value: Record<string, string>,
  members: string[],
): Record<string, string> {
  const keys = Object.keys(value).sort();
  if (keys.length !== members.length || keys.some((key, index) => key !== members[index])) {
    throw new ApiError(400, "invalid_reservations", "Every signed party member needs one reservation.");
  }
  return Object.fromEntries(
    members.map((accountId) => [
      accountId,
      requireIdentifier(value[accountId], `reservationIds.${accountId}`),
    ]),
  );
}

function enforcePlatformPool(claims: TicketClaims, body: MatchmakingBody): void {
  if (claims.platform === undefined) {
    throw new ApiError(403, "platform_claim_required", "Session ticket does not identify its platform.");
  }
  if (claims.party !== undefined && claims.party.input_pool !== body.inputPool) {
    throw new ApiError(403, "party_input_pool_mismatch", "Party ticket requires a different input pool.");
  }
  if (claims.platform === "web" || claims.party?.input_pool === "browser") {
    if (body.playlist !== "casual-extraction" || body.inputPool !== "browser") {
      throw new ApiError(403, "browser_competitive_forbidden", "Browser sessions must use the casual browser pool.");
    }
    return;
  }
  if (body.inputPool === "browser" || body.playlist === "casual-extraction") {
    throw new ApiError(400, "native_pool_mismatch", "Native and console sessions must use a native input pool.");
  }
  const partySize = claims.party?.members.length ?? 1;
  if (body.playlist === "solo-extraction" && partySize !== 1) {
    throw new ApiError(400, "party_playlist_mismatch", "Solo extraction requires a one-player party.");
  }
}

function validateDispatch(value: unknown): DispatchRequest {
  assertObject(value, "dispatch request");
  const playlist = value.playlist;
  if (!["casual-extraction", "solo-extraction", "squad-extraction"].includes(String(playlist))) {
    throw new ApiError(400, "invalid_playlist", "Playlist is unsupported.");
  }
  const inputPool = value.inputPool;
  if (!["browser", "mouse-keyboard", "controller", "mixed"].includes(String(inputPool))) {
    throw new ApiError(400, "invalid_input_pool", "Input pool is unsupported.");
  }
  if (typeof value.allowBots !== "boolean") {
    throw new ApiError(400, "invalid_request", "allowBots must be boolean.");
  }
  const dispatch = {
    shard: requireIdentifier(value.shard, "shard"),
    matchId: requireHex128(value.matchId, "matchId"),
    matchEpoch: requireInteger(
      value.matchEpoch,
      "matchEpoch",
      1,
      Number.MAX_SAFE_INTEGER,
    ),
    region: requireIdentifier(value.region, "region"),
    playlist: playlist as Playlist,
    inputPool: inputPool as InputPool,
    buildHash: requireHex128(value.buildHash, "buildHash"),
    targetPlayers: requireInteger(value.targetPlayers, "targetPlayers", 1, 128),
    allowBots: value.allowBots,
  };
  const shardPrefix = `${dispatch.region}:${dispatch.playlist}:${dispatch.inputPool}:`;
  if (!dispatch.shard.startsWith(shardPrefix)) {
    throw new ApiError(400, "shard_dispatch_mismatch", "Shard does not match dispatch dimensions.");
  }
  return dispatch;
}

function publicCheckpoint(checkpoint: ProfileView["island"]): unknown {
  if (checkpoint === null) return null;
  const { objectKey: _objectKey, ...publicMetadata } = checkpoint;
  return publicMetadata;
}

function allowedOrigins(env: Env): Set<string> {
  let value: unknown;
  try {
    value = JSON.parse(env.ALLOWED_ORIGINS_JSON);
  } catch {
    throw new Error("ALLOWED_ORIGINS_JSON is not valid JSON.");
  }
  if (!Array.isArray(value) || value.some((origin) => typeof origin !== "string")) {
    throw new Error("ALLOWED_ORIGINS_JSON must be a JSON string array.");
  }
  return new Set(value);
}

function enforceOrigin(request: Request, env: Env): void {
  const origin = request.headers.get("origin");
  if (origin !== null && !allowedOrigins(env).has(origin)) {
    throw new ApiError(403, "origin_forbidden", "Request origin is not allowed.");
  }
}

function withCommonHeaders(
  response: Response,
  request: Request,
  env: Env,
  id: string,
): Response {
  if (response.status === 101) return response;
  const headers = new Headers(response.headers);
  headers.set("x-content-type-options", "nosniff");
  headers.set("referrer-policy", "no-referrer");
  headers.set("x-aetherloom-request-id", id);
  const origin = request.headers.get("origin");
  if (origin !== null && allowedOrigins(env).has(origin)) {
    headers.set("access-control-allow-origin", origin);
    headers.set("access-control-allow-methods", "GET, POST, PUT, OPTIONS");
    headers.set(
      "access-control-allow-headers",
      "Authorization, Content-Type, Idempotency-Key, X-Aetherloom-Command-Sequence, X-Aetherloom-Content-Build, X-Aetherloom-Expected-Version, X-Aetherloom-Logical-Time-Ms",
    );
    headers.set("access-control-max-age", "600");
    headers.append("vary", "Origin");
  }
  return new Response(response.body, {
    status: response.status,
    statusText: response.statusText,
    headers,
  });
}
