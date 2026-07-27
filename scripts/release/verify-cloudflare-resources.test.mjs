import assert from "node:assert/strict";
import test from "node:test";

import {
  accountFingerprint,
  manifestFingerprint,
  verifyResourceIdentity,
} from "./verify-cloudflare-resources.mjs";

const ACCOUNT = "a".repeat(32);

function fixture() {
  const manifest = {
    version: 1,
    environment: "staging",
    accountIdSha256: accountFingerprint(ACCOUNT),
    workerName: "aetherloom-control-plane-staging",
    workersDev: true,
    d1DatabaseName: "aetherloom-control-staging",
    d1DatabaseId: "220c38b2-1286-4a35-8c47-a7a24b83f387",
    r2BucketName: "aetherloom-match-archive-staging",
    r2Location: "WEUR",
    settlementQueue: "aetherloom-settlement-staging",
    settlementQueueId: "1".repeat(32),
    settlementDeadLetterQueue: "aetherloom-settlement-staging-dlq",
    settlementDeadLetterQueueId: "2".repeat(32),
  };
  manifest.resourceFingerprint = manifestFingerprint(manifest);
  return {
    environment: "staging",
    accountId: ACCOUNT,
    manifest,
    config: {
      name: manifest.workerName,
      workers_dev: true,
      vars: { ENVIRONMENT: "staging" },
      d1_databases: [{
        binding: "CONTROL_DB",
        database_name: manifest.d1DatabaseName,
        database_id: manifest.d1DatabaseId,
      }],
      r2_buckets: [{
        binding: "MATCH_ARCHIVE",
        bucket_name: manifest.r2BucketName,
      }],
      queues: {
        producers: [{
          binding: "SETTLEMENT_QUEUE",
          queue: manifest.settlementQueue,
        }],
        consumers: [{
          queue: manifest.settlementQueue,
          dead_letter_queue: manifest.settlementDeadLetterQueue,
        }],
      },
    },
    d1Info: { name: manifest.d1DatabaseName, uuid: manifest.d1DatabaseId },
    r2Info: { name: manifest.r2BucketName, location: manifest.r2Location },
    settlementQueueInfo:
      `Queue Name: ${manifest.settlementQueue}\nQueue ID: ${manifest.settlementQueueId}\n`,
    settlementDeadLetterQueueInfo:
      `Queue Name: ${manifest.settlementDeadLetterQueue}\nQueue ID: ${manifest.settlementDeadLetterQueueId}\n`,
    secrets: [
      { name: "PLAYER_TICKET_KEYS_JSON" },
      { name: "JOIN_TICKET_SIGNING_KEYS_JSON" },
      { name: "SERVICE_TICKET_KEYS_JSON" },
      { name: "RESULT_TICKET_KEYS_JSON" },
    ],
    deployments: [{ id: "deployment", versions: [{ version_id: "version" }] }],
  };
}

test("verifies committed, rendered, and remote resource identities together", () => {
  const input = fixture();
  assert.equal(verifyResourceIdentity(input), input.manifest.resourceFingerprint);
});

for (const [label, mutate] of [
  ["account", (value) => { value.accountId = "b".repeat(32); }],
  ["manifest fingerprint", (value) => { value.manifest.workerName = "other"; }],
  ["D1 id", (value) => { value.d1Info.uuid = "320c38b2-1286-4a35-8c47-a7a24b83f387"; }],
  ["Queue id", (value) => {
    value.settlementQueueInfo =
      `Queue Name: ${value.manifest.settlementQueue}\nQueue ID: ${"3".repeat(32)}\n`;
  }],
  ["secret set", (value) => { value.secrets.pop(); }],
  ["Worker deployment", (value) => { value.deployments = []; }],
]) {
  test(`rejects a mismatched ${label}`, () => {
    const input = fixture();
    mutate(input);
    assert.throws(() => verifyResourceIdentity(input), /Cloudflare|resource|D1|Queue|secret|deployment/u);
  });
}
