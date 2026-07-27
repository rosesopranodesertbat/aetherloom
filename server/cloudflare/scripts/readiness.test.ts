import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { assertCryptographicReadiness } from "../src/readiness.ts";
import type { Env } from "../src/types.ts";

const fixture = JSON.parse(
  await readFile(new URL("../fixtures/join-ticket-v1.json", import.meta.url), "utf8"),
) as {
  privateKeyPkcs8Base64Url: string;
  publicKeyRawBase64Url: string;
};
const hmac = Buffer.alloc(32, 0x5a).toString("base64url");

function readinessEnv(): Env {
  return {
    TICKET_ISSUER: "aetherloom-control-plane",
    PLAYER_TICKET_AUDIENCE: "aetherloom-player",
    JOIN_TICKET_AUDIENCE: "aetherloom-match",
    SERVICE_TICKET_AUDIENCE: "aetherloom-services",
    RESULT_TICKET_AUDIENCE: "aetherloom-results",
    PLAYER_TICKET_KEYS_JSON: JSON.stringify({ "player-v1": hmac }),
    SERVICE_TICKET_KEYS_JSON: JSON.stringify({ "service-v1": hmac }),
    RESULT_TICKET_KEYS_JSON: JSON.stringify({ "result-v1": hmac }),
    JOIN_TICKET_SIGNING_KEYS_JSON: JSON.stringify({
      "join-v1": fixture.privateKeyPkcs8Base64Url,
    }),
    JOIN_TICKET_PUBLIC_KEYS_JSON: JSON.stringify({
      "join-v1": fixture.publicKeyRawBase64Url,
    }),
    RESULT_TICKET_HOSTS_JSON: JSON.stringify({
      "result-v1": "readiness-host",
    }),
    ACTIVE_JOIN_TICKET_KID: "join-v1",
    ACTIVE_SERVICE_TICKET_KID: "service-v1",
    CONTENT_BUILD_HASH: "11111111111111111111111111111111",
  } as Env;
}

test("cryptographic readiness round-trips every private key domain", async () => {
  await assert.doesNotReject(assertCryptographicReadiness(readinessEnv()));
});

test("cryptographic readiness rejects a mismatched join verifier", async () => {
  const env = readinessEnv();
  env.JOIN_TICKET_PUBLIC_KEYS_JSON = JSON.stringify({
    "join-v1": Buffer.alloc(32, 0x33).toString("base64url"),
  });
  await assert.rejects(
    assertCryptographicReadiness(env),
    /signature is invalid/u,
  );
});

test("cryptographic readiness rejects an unmapped result signer", async () => {
  const env = readinessEnv();
  env.RESULT_TICKET_HOSTS_JSON = "{}";
  await assert.rejects(
    assertCryptographicReadiness(env),
    /readiness result signer host/u,
  );
});
