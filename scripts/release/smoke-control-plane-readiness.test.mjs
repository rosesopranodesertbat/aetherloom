import assert from "node:assert/strict";
import {
  createHmac,
  timingSafeEqual,
} from "node:crypto";
import {
  chmod,
  mkdtemp,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  parseArguments,
  runReadinessSmoke,
} from "./smoke-control-plane-readiness.mjs";

const BUILD = "11111111111111111111111111111111";
const PLAYER = Buffer.alloc(32, 0x11);
const SERVICE = Buffer.alloc(32, 0x22);

async function fixture(directory) {
  const path = join(directory, "readiness.json");
  await writeFile(
    path,
    JSON.stringify({
      PLAYER_TICKET_KEYS_JSON: JSON.stringify({
        "player-v1": PLAYER.toString("base64url"),
      }),
      SERVICE_TICKET_KEYS_JSON: JSON.stringify({
        "service-v1": SERVICE.toString("base64url"),
      }),
    }),
    { mode: 0o600 },
  );
  await chmod(path, 0o600);
  return path;
}

function verifiedTicket(request) {
  const compact = request.headers.get("authorization")?.slice("Bearer ".length);
  if (!compact) return "missing_ticket";
  const parts = compact.split(".");
  if (parts.length !== 3) return "invalid_ticket_signature";
  const expected = createHmac("sha256", SERVICE)
    .update(`${parts[0]}.${parts[1]}`, "utf8")
    .digest();
  const actual = Buffer.from(parts[2], "base64url");
  return expected.byteLength === actual.byteLength && timingSafeEqual(expected, actual)
    ? "valid"
    : "invalid_ticket_signature";
}

function response(status, body) {
  return new Response(JSON.stringify(body), {
    status,
    headers: {
      "content-type": "application/json",
      "x-content-type-options": "nosniff",
      "referrer-policy": "no-referrer",
      "x-aetherloom-request-id": "readiness-test",
    },
  });
}

test("readiness smoke proves rejection and authenticated cryptographic success", async (context) => {
  const directory = await mkdtemp(join(tmpdir(), "aetherloom-readiness-"));
  context.after(() => rm(directory, { recursive: true, force: true }));
  const secretsFile = await fixture(directory);
  const reports = [];
  await runReadinessSmoke(
    {
      url: "http://127.0.0.1:8787",
      environment: "staging",
      expectedBuild: BUILD,
      secretsFile,
      fetchImpl: async (input, init) => {
        const request = new Request(input, init);
        const verified = verifiedTicket(request);
        if (verified !== "valid") {
          return response(401, { error: { code: verified } });
        }
        return response(200, {
          status: "ready",
          environment: "staging",
          buildHash: BUILD,
          authentication: "verified",
          cryptography: "verified",
        });
      },
    },
    (message) => reports.push(message),
  );
  assert.deepEqual(reports, ["PASS authenticated staging cryptographic readiness"]);
  assert.equal(reports.join("\n").includes(SERVICE.toString("base64url")), false);
});

test("readiness CLI parser keeps credentials file-based", () => {
  assert.deepEqual(
    parseArguments([
      "--url", "https://example.workers.dev",
      "--environment", "production",
      "--expected-build", BUILD,
      "--secrets-file", "/tmp/readiness.json",
    ]),
    {
      url: "https://example.workers.dev",
      environment: "production",
      expectedBuild: BUILD,
      secretsFile: "/tmp/readiness.json",
      serviceKeyId: undefined,
      issuer: undefined,
      serviceAudience: undefined,
      timeoutMs: undefined,
    },
  );
});
