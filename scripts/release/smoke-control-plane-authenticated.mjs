#!/usr/bin/env node

import {
  constants as fsConstants,
  lstat,
  open,
} from "node:fs/promises";
import {
  createHash,
  createHmac,
  randomBytes,
} from "node:crypto";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const IDENTIFIER_RE = /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$/u;
const HEX_128_RE = /^[0-9a-f]{32}$/u;
const BASE64URL_RE = /^[A-Za-z0-9_-]+$/u;
const MAX_RESPONSE_BYTES = 64 * 1024;
const DEFAULT_TIMEOUT_MS = 10_000;
const DEFAULT_TICKET_LIFETIME_SECONDS = 120;

function fail(message) {
  throw new Error(message);
}

function requireIdentifier(value, label) {
  if (typeof value !== "string" || !IDENTIFIER_RE.test(value)) {
    fail(`${label} must be a 1-128 character Aetherloom identifier`);
  }
  return value;
}

function requireBuildHash(value) {
  if (!HEX_128_RE.test(value ?? "") || /^0{32}$/u.test(value)) {
    fail("expected build must be a nonzero 128-bit lowercase hexadecimal content id");
  }
  return value;
}

function canonicalJson(value) {
  return JSON.stringify(sortJson(value));
}

function sortJson(value) {
  if (Array.isArray(value)) return value.map(sortJson);
  if (typeof value === "object" && value !== null) {
    return Object.fromEntries(
      Object.keys(value)
        .sort()
        .map((key) => [key, sortJson(value[key])]),
    );
  }
  return value;
}

function base64Url(bytes) {
  return Buffer.from(bytes).toString("base64url");
}

function randomHex128() {
  return randomBytes(16).toString("hex");
}

function parseKeySet(encoded, label) {
  if (typeof encoded !== "string") fail(`${label} is missing from the secret bundle`);
  let parsed;
  try {
    parsed = JSON.parse(encoded);
  } catch {
    fail(`${label} is not valid JSON`);
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    fail(`${label} must be a JSON object`);
  }
  const entries = Object.entries(parsed);
  if (entries.length < 1 || entries.length > 8) {
    fail(`${label} must contain between one and eight keys`);
  }
  const keys = {};
  for (const [keyId, encodedKey] of entries) {
    requireIdentifier(keyId, `${label} key id`);
    if (
      typeof encodedKey !== "string" ||
      !BASE64URL_RE.test(encodedKey) ||
      Buffer.from(encodedKey, "base64url").toString("base64url") !== encodedKey
    ) {
      fail(`${label} contains an invalid base64url key`);
    }
    const key = Buffer.from(encodedKey, "base64url");
    if (key.byteLength < 32) fail(`${label} keys must contain at least 32 random bytes`);
    keys[keyId] = key;
  }
  return keys;
}

function selectKey(keys, requestedKeyId, label) {
  if (requestedKeyId !== undefined) {
    requireIdentifier(requestedKeyId, `${label} key id`);
    const key = keys[requestedKeyId];
    if (key === undefined) fail(`${label} key id is not present in the secret bundle`);
    return { keyId: requestedKeyId, key };
  }
  const entries = Object.entries(keys);
  if (entries.length !== 1) {
    fail(`${label} has multiple keys; specify its key id explicitly`);
  }
  const [keyId, key] = entries[0];
  return { keyId, key };
}

export async function readWorkerSecretBundle(filePath, keyIds = {}) {
  if (typeof filePath !== "string" || filePath.length === 0) {
    fail("a worker secret bundle path is required");
  }
  const absolutePath = resolve(filePath);
  const pathMetadata = await lstat(absolutePath);
  if (pathMetadata.isSymbolicLink() || !pathMetadata.isFile()) {
    fail("worker secret bundle must be a regular file, not a link");
  }

  const noFollow = fsConstants.O_NOFOLLOW ?? 0;
  const handle = await open(absolutePath, fsConstants.O_RDONLY | noFollow);
  let raw;
  try {
    const metadata = await handle.stat();
    if (!metadata.isFile()) fail("worker secret bundle must be a regular file");
    if ((metadata.mode & 0o7777) !== 0o600) {
      fail("worker secret bundle must have exact mode 0600");
    }
    if (
      typeof process.getuid === "function" &&
      metadata.uid !== process.getuid()
    ) {
      fail("worker secret bundle must be owned by the current operator");
    }
    if (metadata.size < 2 || metadata.size > MAX_RESPONSE_BYTES) {
      fail("worker secret bundle has an invalid size");
    }
    raw = await handle.readFile("utf8");
  } finally {
    await handle.close();
  }

  let bundle;
  try {
    bundle = JSON.parse(raw);
  } catch {
    fail("worker secret bundle is not valid JSON");
  }
  if (typeof bundle !== "object" || bundle === null || Array.isArray(bundle)) {
    fail("worker secret bundle must be a JSON object");
  }

  const player = selectKey(
    parseKeySet(bundle.PLAYER_TICKET_KEYS_JSON, "PLAYER_TICKET_KEYS_JSON"),
    keyIds.player,
    "player ticket key set",
  );
  const service = selectKey(
    parseKeySet(bundle.SERVICE_TICKET_KEYS_JSON, "SERVICE_TICKET_KEYS_JSON"),
    keyIds.service,
    "service ticket key set",
  );
  return { player, service };
}

export function mintHmacTicket(claims, signingKey) {
  const header = {
    alg: "HS256",
    kid: requireIdentifier(signingKey.keyId, "ticket key id"),
    typ: "AETHERLOOM-TICKET",
    v: 1,
  };
  const headerPart = base64Url(Buffer.from(canonicalJson(header), "utf8"));
  const claimsPart = base64Url(Buffer.from(canonicalJson(claims), "utf8"));
  const signingInput = `${headerPart}.${claimsPart}`;
  const signature = createHmac("sha256", signingKey.key)
    .update(signingInput, "utf8")
    .digest();
  return `${signingInput}.${base64Url(signature)}`;
}

function ticketClaims({
  audience,
  issuer,
  purpose,
  subject,
  nowSeconds,
  scopes,
  platform,
}) {
  return {
    v: 1,
    iss: issuer,
    aud: audience,
    purpose,
    sub: subject,
    iat: nowSeconds,
    nbf: nowSeconds - 2,
    exp: nowSeconds + DEFAULT_TICKET_LIFETIME_SECONDS,
    nonce: randomHex128(),
    ...(scopes === undefined ? {} : { scopes }),
    ...(platform === undefined ? {} : { platform }),
  };
}

function validatedBaseUrl(value) {
  const url = new URL(value);
  const local = url.hostname === "localhost" || url.hostname === "127.0.0.1";
  if (url.protocol !== "https:" && !(local && url.protocol === "http:")) {
    fail("control-plane target must use HTTPS, except for localhost tests");
  }
  if (url.username !== "" || url.password !== "" || url.search !== "" || url.hash !== "") {
    fail("control-plane target must not contain credentials, a query, or a fragment");
  }
  url.pathname = url.pathname.replace(/\/+$/u, "");
  return url;
}

function validatedOrigin(value) {
  const url = new URL(value);
  if (
    url.protocol !== "https:" ||
    url.origin !== value ||
    url.username !== "" ||
    url.password !== ""
  ) {
    fail("allowed origin must be an exact HTTPS origin without a trailing slash");
  }
  return value;
}

function endpoint(baseUrl, path) {
  const url = new URL(baseUrl);
  url.pathname = `${url.pathname.replace(/\/+$/u, "")}${path}`;
  return url;
}

function commonHeaders(origin) {
  return {
    accept: "application/json",
    "cache-control": "no-store",
    "user-agent": "aetherloom-operator-smoke/1",
    ...(origin === undefined ? {} : { origin }),
  };
}

async function responseJson(response, label) {
  const body = await response.arrayBuffer();
  if (body.byteLength > MAX_RESPONSE_BYTES) fail(`${label} response is too large`);
  const type = response.headers.get("content-type")?.toLowerCase() ?? "";
  if (!type.includes("application/json")) fail(`${label} did not return JSON`);
  try {
    return JSON.parse(Buffer.from(body).toString("utf8"));
  } catch {
    fail(`${label} returned malformed JSON`);
  }
}

async function apiRequest(baseUrl, path, {
  method = "GET",
  token,
  origin,
  body,
  headers = {},
  timeoutMs,
  fetchImpl = fetch,
}) {
  const response = await fetchImpl(endpoint(baseUrl, path), {
    method,
    redirect: "error",
    signal: AbortSignal.timeout(timeoutMs),
    headers: {
      ...commonHeaders(origin),
      ...(token === undefined ? {} : { authorization: `Bearer ${token}` }),
      ...(body === undefined ? {} : { "content-type": "application/json" }),
      ...headers,
    },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
  return response;
}

function expectStatus(response, expected, label) {
  const statuses = Array.isArray(expected) ? expected : [expected];
  if (!statuses.includes(response.status)) {
    fail(`${label} returned HTTP ${response.status}; expected ${statuses.join(" or ")}`);
  }
}

function expectCors(response, origin, label) {
  if (response.headers.get("access-control-allow-origin") !== origin) {
    fail(`${label} did not return the exact allowed CORS origin`);
  }
  const vary = response.headers.get("vary")?.toLowerCase().split(",").map((part) => part.trim()) ?? [];
  if (!vary.includes("origin")) fail(`${label} did not vary on Origin`);
}

function expectSecurityHeaders(response, label) {
  if (response.headers.get("x-content-type-options") !== "nosniff") {
    fail(`${label} is missing X-Content-Type-Options`);
  }
  if (response.headers.get("referrer-policy") !== "no-referrer") {
    fail(`${label} is missing Referrer-Policy`);
  }
  if (!response.headers.get("x-aetherloom-request-id")) {
    fail(`${label} is missing its request id`);
  }
}

async function expectApiError(response, status, code, label, origin) {
  expectStatus(response, status, label);
  if (origin !== undefined) expectCors(response, origin, label);
  expectSecurityHeaders(response, label);
  const payload = await responseJson(response, label);
  if (payload?.error?.code !== code) {
    fail(`${label} returned the wrong API error code`);
  }
}

function tamperSignature(ticket) {
  const parts = ticket.split(".");
  const signature = parts[2];
  const first = signature[0] === "A" ? "B" : "A";
  parts[2] = `${first}${signature.slice(1)}`;
  return parts.join(".");
}

function stableSmokeAccount(baseUrl) {
  const digest = createHash("sha256")
    .update(new URL(baseUrl).origin, "utf8")
    .digest("hex")
    .slice(0, 20);
  return `staging-smoke-${digest}`;
}

function fnv1a(value) {
  let hash = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash >>> 0;
}

function matchmakingShard(region, rating, partyId, shardCount, skillBucketWidth) {
  const skillBucket = Math.floor(Math.max(0, rating) / skillBucketWidth);
  const shardIndex = fnv1a(partyId) % shardCount;
  return `${region}:casual-extraction:browser:mmr-${skillBucket}:shard-${shardIndex}`;
}

async function cleanupIsolatedState({
  baseUrl,
  token,
  origin,
  timeoutMs,
  shard,
  queueTicketId,
  reservationId,
  fetchImpl,
}) {
  const failures = [];
  const request = (path, init) =>
    apiRequest(baseUrl, path, { timeoutMs, fetchImpl, ...init });
  if (shard !== undefined && queueTicketId !== undefined) {
    try {
      const response = await request("/v1/matchmaking/cancel", {
        method: "POST",
        token,
        origin,
        body: { shard, queueTicketId },
      });
      expectStatus(response, 200, "matchmaking cleanup");
      const payload = await responseJson(response, "matchmaking cleanup");
      if (payload?.status !== "cancelled") fail("matchmaking cleanup did not cancel the queue ticket");
    } catch (error) {
      failures.push(`matchmaking cleanup failed: ${error.message}`);
    }
  }
  if (reservationId !== undefined) {
    try {
      const response = await request("/v1/profile/reservations/cancel", {
        method: "POST",
        token,
        origin,
        body: { reservationId },
      });
      expectStatus(response, 200, "reservation cleanup");
      const payload = await responseJson(response, "reservation cleanup");
      if (payload?.status !== "cancelled") fail("reservation cleanup did not cancel the reservation");
    } catch (error) {
      failures.push(`reservation cleanup failed: ${error.message}`);
    }
  }
  return failures;
}

export async function runAuthenticatedSmoke(options, report = (message) => console.log(message)) {
  if (options.confirmIsolatedCapacityProbe !== true) {
    fail("the isolated capacity probe requires --confirm-isolated-capacity-probe");
  }
  const baseUrl = validatedBaseUrl(options.url);
  const expectedBuild = requireBuildHash(options.expectedBuild);
  const origin = validatedOrigin(options.origin);
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  if (!Number.isInteger(timeoutMs) || timeoutMs < 1_000 || timeoutMs > 60_000) {
    fail("timeout must be an integer between 1000 and 60000 milliseconds");
  }
  const issuer = requireIdentifier(
    options.issuer ?? "aetherloom-control-plane",
    "ticket issuer",
  );
  const playerAudience = requireIdentifier(
    options.playerAudience ?? "aetherloom-player",
    "player ticket audience",
  );
  const serviceAudience = requireIdentifier(
    options.serviceAudience ?? "aetherloom-services",
    "service ticket audience",
  );
  const region = requireIdentifier(options.region ?? "weur", "region");
  const shardCount = options.shardCount ?? 16;
  const skillBucketWidth = options.skillBucketWidth ?? 200;
  if (!Number.isInteger(shardCount) || shardCount < 1 || shardCount > 256) {
    fail("matchmaking shard count must be an integer between 1 and 256");
  }
  if (
    !Number.isInteger(skillBucketWidth) ||
    skillBucketWidth < 25 ||
    skillBucketWidth > 1_000
  ) {
    fail("matchmaking skill bucket width must be an integer between 25 and 1000");
  }
  const accountId = requireIdentifier(
    options.accountId ?? stableSmokeAccount(baseUrl),
    "smoke account id",
  );
  const secrets = await readWorkerSecretBundle(options.secretsFile, {
    player: options.playerKeyId,
    service: options.serviceKeyId,
  });

  const mintPlayerToken = () => mintHmacTicket(
    ticketClaims({
      audience: playerAudience,
      issuer,
      purpose: "session",
      subject: accountId,
      nowSeconds: Math.floor(Date.now() / 1_000),
      platform: "web",
    }),
    secrets.player,
  );
  const playerToken = mintPlayerToken();
  const nowSeconds = Math.floor(Date.now() / 1_000);
  const serviceToken = mintHmacTicket(
    ticketClaims({
      audience: serviceAudience,
      issuer,
      purpose: "service",
      subject: "staging-smoke-operator",
      nowSeconds,
      scopes: ["match:dispatch"],
    }),
    secrets.service,
  );
  const fetchImpl = options.fetchImpl ?? fetch;
  if (typeof fetchImpl !== "function") fail("fetch implementation must be a function");
  const request = (path, init = {}) =>
    apiRequest(baseUrl, path, { timeoutMs, fetchImpl, ...init });

  let reservationId;
  let queueTicketId;
  let shard;
  let primaryError;
  try {
    const healthResponse = await request("/healthz");
    expectStatus(healthResponse, 200, "health check");
    expectSecurityHeaders(healthResponse, "health check");
    const health = await responseJson(healthResponse, "health check");
    if (
      health?.status !== "ok" ||
      health?.service !== "aetherloom-control-plane" ||
      health?.environment !== "staging" ||
      health?.authoritativeHz !== 128 ||
      health?.buildHash !== expectedBuild
    ) {
      fail("health payload does not match the staging environment, 128 Hz, and expected build");
    }
    report("PASS environment and content build");

    const preflight = await request("/v1/profile", {
      method: "OPTIONS",
      origin,
      headers: {
        "access-control-request-method": "GET",
        "access-control-request-headers": "authorization",
      },
    });
    expectStatus(preflight, 204, "CORS preflight");
    expectCors(preflight, origin, "CORS preflight");
    expectSecurityHeaders(preflight, "CORS preflight");
    if (
      !preflight.headers
        .get("access-control-allow-headers")
        ?.toLowerCase()
        .split(",")
        .map((part) => part.trim())
        .includes("authorization")
    ) {
      fail("CORS preflight does not allow Authorization");
    }

    const blockedOrigin = "https://aetherloom-smoke.invalid";
    const rejectedCors = await request("/healthz", {
      origin: blockedOrigin,
    });
    await expectApiError(rejectedCors, 403, "origin_forbidden", "disallowed CORS origin");
    if (rejectedCors.headers.has("access-control-allow-origin")) {
      fail("disallowed CORS origin was reflected");
    }
    report("PASS CORS allowlist and rejection");

    const missingAuth = await request("/v1/profile", {
      origin,
    });
    await expectApiError(missingAuth, 401, "missing_ticket", "missing authentication", origin);

    const tamperedAuth = await request("/v1/profile", {
      token: tamperSignature(playerToken),
      origin,
    });
    await expectApiError(
      tamperedAuth,
      401,
      "invalid_ticket_signature",
      "tampered authentication",
      origin,
    );

    const wrongPurpose = await request(
      "/internal/v1/matchmaking/dispatch",
      {
        method: "POST",
        token: playerToken,
        body: {},
      },
    );
    await expectApiError(
      wrongPurpose,
      401,
      "unknown_ticket_key",
      "player ticket on service route",
    );

    const profileResponse = await request("/v1/profile", {
      token: playerToken,
      origin,
    });
    expectStatus(profileResponse, 200, "authenticated profile");
    expectCors(profileResponse, origin, "authenticated profile");
    expectSecurityHeaders(profileResponse, "authenticated profile");
    const profile = await responseJson(profileResponse, "authenticated profile");
    if (
      profile?.accountId !== accountId ||
      !Array.isArray(profile?.inventory) ||
      !Number.isInteger(profile?.rating) ||
      !Number.isInteger(profile?.version)
    ) {
      fail("authenticated profile response is not the isolated smoke profile");
    }
    report("PASS player authentication and key-domain isolation");

    reservationId = `smoke-res-${randomHex128()}`;
    const reservationResponse = await request(
      "/v1/profile/reservations",
      {
        method: "POST",
        token: playerToken,
        origin,
        headers: { "idempotency-key": reservationId },
        body: { loadout: [] },
      },
    );
    expectStatus(reservationResponse, [200, 201], "isolated empty reservation");
    const reservation = await responseJson(
      reservationResponse,
      "isolated empty reservation",
    );
    if (
      reservation?.reservationId !== reservationId ||
      reservation?.status !== "reserved" ||
      !Array.isArray(reservation?.loadout) ||
      reservation.loadout.length !== 0
    ) {
      fail("isolated reservation response is invalid");
    }

    queueTicketId = `smoke-queue-${randomHex128()}`;
    shard = matchmakingShard(
      region,
      profile.rating,
      `solo-${accountId}`,
      shardCount,
      skillBucketWidth,
    );
    const enqueueResponse = await request(
      "/v1/matchmaking/enqueue",
      {
        method: "POST",
        token: playerToken,
        origin,
        headers: { "idempotency-key": queueTicketId },
        body: {
          region,
          playlist: "casual-extraction",
          inputPool: "browser",
          reservationIds: { [accountId]: reservationId },
        },
      },
    );
    expectStatus(enqueueResponse, [200, 201], "isolated matchmaking enqueue");
    const enqueue = await responseJson(enqueueResponse, "isolated matchmaking enqueue");
    if (
      enqueue?.queueTicketId !== queueTicketId ||
      enqueue?.status !== "waiting" ||
      typeof enqueue?.shard !== "string"
    ) {
      fail("isolated matchmaking response is invalid");
    }
    const expectedShard = shard;
    shard = requireIdentifier(enqueue.shard, "matchmaking shard");
    if (shard !== expectedShard) {
      fail("matchmaking shard does not match the deployed staging configuration");
    }

    const matchId = randomHex128();
    const dispatchResponse = await request(
      "/internal/v1/matchmaking/dispatch",
      {
        method: "POST",
        token: serviceToken,
        body: {
          shard,
          matchId,
          matchEpoch: Date.now(),
          region,
          playlist: "casual-extraction",
          inputPool: "browser",
          buildHash: expectedBuild,
          targetPlayers: 1,
          allowBots: false,
        },
      },
    );
    await expectApiError(
      dispatchResponse,
      503,
      "match_capacity_unavailable",
      "unconfigured capacity dispatch",
    );
    report("PASS match capacity fails closed");
  } catch (error) {
    primaryError = error;
  }

  const cleanupFailures = await cleanupIsolatedState({
    baseUrl,
    token: mintPlayerToken(),
    origin,
    timeoutMs,
    shard,
    queueTicketId,
    reservationId,
    fetchImpl,
  });
  if (cleanupFailures.length === 0 && reservationId !== undefined) {
    report("PASS isolated smoke state cleaned up");
  }
  if (primaryError !== undefined || cleanupFailures.length > 0) {
    const failures = [
      ...(primaryError === undefined ? [] : [primaryError.message]),
      ...cleanupFailures,
    ];
    fail(failures.join("; "));
  }
  report("Authenticated staging smoke passed.");
}

function optionValue(argumentsList, name) {
  const index = argumentsList.indexOf(name);
  if (index < 0) return undefined;
  const value = argumentsList[index + 1];
  if (value === undefined || value.startsWith("--")) fail(`${name} requires a value`);
  return value;
}

export function parseArguments(argumentsList) {
  const knownFlags = new Set([
    "--url",
    "--secrets-file",
    "--expected-build",
    "--origin",
    "--issuer",
    "--player-audience",
    "--service-audience",
    "--player-kid",
    "--service-kid",
    "--account-id",
    "--region",
    "--shard-count",
    "--skill-bucket-width",
    "--timeout-ms",
    "--confirm-isolated-capacity-probe",
  ]);
  for (let index = 0; index < argumentsList.length; index += 1) {
    const argument = argumentsList[index];
    if (!knownFlags.has(argument)) fail(`unknown argument: ${argument}`);
    if (argument !== "--confirm-isolated-capacity-probe") index += 1;
  }
  const timeoutText = optionValue(argumentsList, "--timeout-ms");
  const shardCountText = optionValue(argumentsList, "--shard-count");
  const skillBucketWidthText = optionValue(argumentsList, "--skill-bucket-width");
  return {
    url: optionValue(argumentsList, "--url"),
    secretsFile: optionValue(argumentsList, "--secrets-file"),
    expectedBuild: optionValue(argumentsList, "--expected-build"),
    origin: optionValue(argumentsList, "--origin"),
    issuer: optionValue(argumentsList, "--issuer"),
    playerAudience: optionValue(argumentsList, "--player-audience"),
    serviceAudience: optionValue(argumentsList, "--service-audience"),
    playerKeyId: optionValue(argumentsList, "--player-kid"),
    serviceKeyId: optionValue(argumentsList, "--service-kid"),
    accountId: optionValue(argumentsList, "--account-id"),
    region: optionValue(argumentsList, "--region"),
    shardCount: shardCountText === undefined ? undefined : Number(shardCountText),
    skillBucketWidth:
      skillBucketWidthText === undefined ? undefined : Number(skillBucketWidthText),
    timeoutMs: timeoutText === undefined ? undefined : Number(timeoutText),
    confirmIsolatedCapacityProbe: argumentsList.includes(
      "--confirm-isolated-capacity-probe",
    ),
  };
}

function usage() {
  return [
    "usage: smoke-control-plane-authenticated.mjs",
    "  --url <staging-worker-url>",
    "  --secrets-file <mode-0600-worker-secret-json>",
    "  --expected-build <32-lowercase-hex>",
    "  --origin <allowed-staging-https-origin>",
    "  --confirm-isolated-capacity-probe",
    "  [--player-kid <id>] [--service-kid <id>] [--region <id>]",
  ].join("\n");
}

async function main() {
  const options = parseArguments(process.argv.slice(2));
  if (
    options.url === undefined ||
    options.secretsFile === undefined ||
    options.expectedBuild === undefined ||
    options.origin === undefined
  ) {
    fail(usage());
  }
  await runAuthenticatedSmoke(options);
}

const invokedPath = process.argv[1] === undefined
  ? undefined
  : pathToFileURL(resolve(process.argv[1])).href;
if (invokedPath === import.meta.url) {
  main().catch((error) => {
    console.error(`Authenticated staging smoke failed: ${error.message}`);
    process.exitCode = 1;
  });
}
