import assert from "node:assert/strict";
import {
  createHmac,
  timingSafeEqual,
} from "node:crypto";
import {
  chmod,
  mkdtemp,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  mintHmacTicket,
  parseArguments,
  readWorkerSecretBundle,
  runAuthenticatedSmoke,
} from "./smoke-control-plane-authenticated.mjs";

const BUILD_HASH = "abdc743597a10be51ad5344f01c89462";
const ALLOWED_ORIGIN = "https://aetherloom-staging.pages.dev";
const PLAYER_KID = "staging-player-test";
const SERVICE_KID = "staging-service-test";
const PLAYER_KEY = Buffer.alloc(32, 0x11);
const SERVICE_KEY = Buffer.alloc(32, 0x22);

async function secretFixture(directory) {
  const path = join(directory, "worker-secrets.json");
  await writeFile(
    path,
    JSON.stringify({
      PLAYER_TICKET_KEYS_JSON: JSON.stringify({
        [PLAYER_KID]: PLAYER_KEY.toString("base64url"),
      }),
      SERVICE_TICKET_KEYS_JSON: JSON.stringify({
        [SERVICE_KID]: SERVICE_KEY.toString("base64url"),
      }),
      JOIN_TICKET_SIGNING_KEYS_JSON: "{}",
      RESULT_TICKET_KEYS_JSON: "{}",
    }),
    { mode: 0o600 },
  );
  await chmod(path, 0o600);
  return path;
}

function ticket(request, keyDomain) {
  const authorization = request.headers.get("authorization");
  if (!authorization?.startsWith("Bearer ")) return { error: "missing_ticket" };
  const compact = authorization.slice("Bearer ".length);
  const parts = compact.split(".");
  if (parts.length !== 3) return { error: "invalid_ticket" };
  let header;
  let claims;
  try {
    header = JSON.parse(Buffer.from(parts[0], "base64url").toString("utf8"));
    claims = JSON.parse(Buffer.from(parts[1], "base64url").toString("utf8"));
  } catch {
    return { error: "invalid_ticket" };
  }
  const key = keyDomain === "player" && header.kid === PLAYER_KID
    ? PLAYER_KEY
    : keyDomain === "service" && header.kid === SERVICE_KID
      ? SERVICE_KEY
      : undefined;
  if (key === undefined) return { error: "unknown_ticket_key" };
  const expected = createHmac("sha256", key)
    .update(`${parts[0]}.${parts[1]}`, "utf8")
    .digest();
  const received = Buffer.from(parts[2], "base64url");
  if (
    expected.byteLength !== received.byteLength ||
    !timingSafeEqual(expected, received)
  ) {
    return { error: "invalid_ticket_signature" };
  }
  return { claims };
}

function commonHeaders(request, extra = {}) {
  const headers = new Headers({
    "x-content-type-options": "nosniff",
    "referrer-policy": "no-referrer",
    "x-aetherloom-request-id": "local-smoke-test",
    ...extra,
  });
  if (request.headers.get("origin") === ALLOWED_ORIGIN) {
    headers.set("access-control-allow-origin", ALLOWED_ORIGIN);
    headers.set(
      "access-control-allow-headers",
      "Authorization, Content-Type, Idempotency-Key",
    );
    headers.set("access-control-allow-methods", "GET, POST, PUT, OPTIONS");
    headers.set("vary", "Origin");
  }
  return headers;
}

function json(request, status, body) {
  const headers = commonHeaders(request, {
    "content-type": "application/json; charset=utf-8",
    "cache-control": "no-store",
  });
  return new Response(JSON.stringify(body), { status, headers });
}

function apiError(request, status, code) {
  return json(request, status, {
    error: { code, message: "test error", requestId: "local-smoke-test" },
  });
}

async function requestBody(request) {
  return request.json();
}

function stagingShard(accountId) {
  const partyId = `solo-${accountId}`;
  let hash = 0x811c9dc5;
  for (let index = 0; index < partyId.length; index += 1) {
    hash ^= partyId.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return `weur:casual-extraction:browser:mmr-5:shard-${(hash >>> 0) % 16}`;
}

function mockControlPlane() {
  const state = {
    accountId: undefined,
    reservationId: undefined,
    queueTicketId: undefined,
    shard: undefined,
    dispatches: 0,
    keyDomainRejections: 0,
    queueCancelled: false,
    reservationCancelled: false,
  };
  const fetchImpl = async (input, init) => {
    const request = new Request(input, init);
    try {
      const url = new URL(request.url);
      if (
        request.headers.has("origin") &&
        request.headers.get("origin") !== ALLOWED_ORIGIN
      ) {
        return apiError(request, 403, "origin_forbidden");
      }
      if (request.method === "OPTIONS") {
        return new Response(null, { status: 204, headers: commonHeaders(request) });
      }
      if (request.method === "GET" && url.pathname === "/healthz") {
        return json(request, 200, {
          status: "ok",
          service: "aetherloom-control-plane",
          environment: "staging",
          authoritativeHz: 128,
          buildHash: BUILD_HASH,
        });
      }

      const internal = url.pathname.startsWith("/internal/");
      const verified = ticket(request, internal ? "service" : "player");
      if (verified.error !== undefined) {
        if (internal && verified.error === "unknown_ticket_key") {
          state.keyDomainRejections += 1;
        }
        return apiError(
          request,
          verified.error === "missing_ticket" ? 401 : 401,
          verified.error,
        );
      }
      const claims = verified.claims;
      if (internal && claims.purpose !== "service") {
        return apiError(request, 403, "wrong_ticket_purpose");
      }
      if (!internal && claims.purpose !== "session") {
        return apiError(request, 403, "wrong_ticket_purpose");
      }

      if (request.method === "GET" && url.pathname === "/v1/profile") {
        state.accountId = claims.sub;
        state.shard = stagingShard(claims.sub);
        return json(request, 200, {
          accountId: claims.sub,
          rating: 1000,
          version: 1,
          inventory: [],
          island: null,
        });
      }
      if (
        request.method === "POST" &&
        url.pathname === "/v1/profile/reservations"
      ) {
        const body = await requestBody(request);
        assert.deepEqual(body, { loadout: [] });
        state.reservationId = request.headers.get("idempotency-key");
        return json(request, 201, {
          reservationId: state.reservationId,
          status: "reserved",
          loadout: [],
          expiresAtMs: Date.now() + 60_000,
        });
      }
      if (
        request.method === "POST" &&
        url.pathname === "/v1/matchmaking/enqueue"
      ) {
        const body = await requestBody(request);
        assert.equal(body.reservationIds[state.accountId], state.reservationId);
        state.queueTicketId = request.headers.get("idempotency-key");
        return json(request, 201, {
          queueTicketId: state.queueTicketId,
          status: "waiting",
          playerCount: 1,
          shard: state.shard,
        });
      }
      if (
        request.method === "POST" &&
        url.pathname === "/internal/v1/matchmaking/dispatch"
      ) {
        const body = await requestBody(request);
        assert.equal(claims.scopes.includes("match:dispatch"), true);
        assert.equal(body.shard, state.shard);
        assert.equal(body.buildHash, BUILD_HASH);
        state.dispatches += 1;
        return apiError(request, 503, "match_capacity_unavailable");
      }
      if (
        request.method === "POST" &&
        url.pathname === "/v1/matchmaking/cancel"
      ) {
        const body = await requestBody(request);
        assert.equal(body.shard, state.shard);
        assert.equal(body.queueTicketId, state.queueTicketId);
        state.queueCancelled = true;
        return json(request, 200, {
          queueTicketId: state.queueTicketId,
          status: "cancelled",
        });
      }
      if (
        request.method === "POST" &&
        url.pathname === "/v1/profile/reservations/cancel"
      ) {
        const body = await requestBody(request);
        assert.equal(body.reservationId, state.reservationId);
        state.reservationCancelled = true;
        return json(request, 200, {
          reservationId: state.reservationId,
          status: "cancelled",
        });
      }
      return apiError(request, 404, "not_found");
    } catch (error) {
      return new Response(JSON.stringify({ testFailure: error.message }), {
        status: 500,
        headers: { "content-type": "application/json" },
      });
    }
  };
  return {
    url: "http://127.0.0.1:8787",
    state,
    fetchImpl,
  };
}

test("ticket encoding is canonical and HMAC authenticated", () => {
  const ticketValue = mintHmacTicket(
    {
      v: 1,
      sub: "smoke",
      purpose: "session",
      nonce: "11111111111111111111111111111111",
      nbf: 99,
      iss: "issuer",
      iat: 100,
      exp: 200,
      aud: "audience",
    },
    { keyId: PLAYER_KID, key: PLAYER_KEY },
  );
  const [headerPart, claimsPart, signaturePart] = ticketValue.split(".");
  assert.equal(
    Buffer.from(headerPart, "base64url").toString("utf8"),
    '{"alg":"HS256","kid":"staging-player-test","typ":"AETHERLOOM-TICKET","v":1}',
  );
  assert.equal(
    Buffer.from(claimsPart, "base64url").toString("utf8"),
    '{"aud":"audience","exp":200,"iat":100,"iss":"issuer","nbf":99,"nonce":"11111111111111111111111111111111","purpose":"session","sub":"smoke","v":1}',
  );
  const expected = createHmac("sha256", PLAYER_KEY)
    .update(`${headerPart}.${claimsPart}`, "utf8")
    .digest("base64url");
  assert.equal(signaturePart, expected);
});

test("secret bundle requires an owned regular mode-0600 file", async (context) => {
  const directory = await mkdtemp(join(tmpdir(), "aetherloom-auth-smoke-"));
  context.after(() => rm(directory, { recursive: true, force: true }));
  const secretPath = await secretFixture(directory);
  const bundle = await readWorkerSecretBundle(secretPath);
  assert.equal(bundle.player.keyId, PLAYER_KID);
  assert.equal(bundle.service.keyId, SERVICE_KID);

  await chmod(secretPath, 0o640);
  await assert.rejects(
    readWorkerSecretBundle(secretPath),
    /exact mode 0600/u,
  );
  await chmod(secretPath, 0o600);

  const linkPath = join(directory, "secret-link.json");
  await symlink(secretPath, linkPath);
  await assert.rejects(
    readWorkerSecretBundle(linkPath),
    /regular file, not a link/u,
  );
});

test("authenticated smoke covers auth, CORS, fail-closed capacity, and cleanup", async (context) => {
  const directory = await mkdtemp(join(tmpdir(), "aetherloom-auth-smoke-"));
  context.after(() => rm(directory, { recursive: true, force: true }));
  const secretPath = await secretFixture(directory);
  const controlPlane = mockControlPlane();
  const reports = [];

  await runAuthenticatedSmoke(
    {
      url: controlPlane.url,
      secretsFile: secretPath,
      expectedBuild: BUILD_HASH,
      origin: ALLOWED_ORIGIN,
      confirmIsolatedCapacityProbe: true,
      fetchImpl: controlPlane.fetchImpl,
    },
    (message) => reports.push(message),
  );

  assert.equal(controlPlane.state.dispatches, 1);
  assert.equal(controlPlane.state.keyDomainRejections, 1);
  assert.equal(controlPlane.state.queueCancelled, true);
  assert.equal(controlPlane.state.reservationCancelled, true);
  assert.deepEqual(reports, [
    "PASS environment and content build",
    "PASS CORS allowlist and rejection",
    "PASS player authentication and key-domain isolation",
    "PASS match capacity fails closed",
    "PASS isolated smoke state cleaned up",
    "Authenticated staging smoke passed.",
  ]);
  const reportText = reports.join("\n");
  assert.equal(reportText.includes(PLAYER_KEY.toString("base64url")), false);
  assert.equal(reportText.includes(SERVICE_KEY.toString("base64url")), false);
});

test("CLI parser requires explicit capacity probe confirmation", () => {
  const parsed = parseArguments([
    "--url",
    "https://example.workers.dev",
    "--secrets-file",
    "/tmp/operator.json",
    "--expected-build",
    BUILD_HASH,
    "--origin",
    ALLOWED_ORIGIN,
    "--confirm-isolated-capacity-probe",
  ]);
  assert.equal(parsed.confirmIsolatedCapacityProbe, true);
  assert.equal(parsed.expectedBuild, BUILD_HASH);
});
