import assert from "node:assert/strict";
import test from "node:test";

import {
  buildRecoveryProof,
  verifyRecoveryProof,
} from "./d1-recovery-proof.mjs";
import { manifestFingerprint } from "./verify-cloudflare-resources.mjs";

const SHA = "1".repeat(40);
const BOOKMARK = "00000002-00000002-000050b5-6687c247e23b95c517d91195c0baab59";

function manifest() {
  const value = {
    version: 1,
    environment: "production",
    accountIdSha256: "2".repeat(64),
    workerName: "aetherloom-control-plane-production",
    workersDev: false,
    customDomain: "api.example.com",
    d1DatabaseName: "aetherloom-control-production",
    d1DatabaseId: "220c38b2-1286-4a35-8c47-a7a24b83f387",
    r2BucketName: "aetherloom-match-archive-production",
    r2Location: "WEUR",
    settlementQueue: "aetherloom-settlement-production",
    settlementQueueId: "3".repeat(32),
    settlementDeadLetterQueue: "aetherloom-settlement-production-dlq",
    settlementDeadLetterQueueId: "4".repeat(32),
  };
  value.resourceFingerprint = manifestFingerprint(value);
  return value;
}

test("binds a fresh Time Travel bookmark to the production release and database", () => {
  const now = new Date("2026-07-27T20:00:00.000Z");
  const resources = manifest();
  const proof = buildRecoveryProof({
    bookmarkResponse: { bookmark: BOOKMARK },
    manifest: resources,
    releaseSha: SHA,
    now,
  });
  assert.equal(
    verifyRecoveryProof({
      proof,
      manifest: resources,
      releaseSha: SHA,
      now: new Date(now.getTime() + 60_000),
    }),
    BOOKMARK,
  );
});

test("rejects stale and cross-release recovery points", () => {
  const now = new Date("2026-07-27T20:00:00.000Z");
  const resources = manifest();
  const proof = buildRecoveryProof({
    bookmarkResponse: { bookmark: BOOKMARK },
    manifest: resources,
    releaseSha: SHA,
    now,
  });
  assert.throws(
    () => verifyRecoveryProof({
      proof,
      manifest: resources,
      releaseSha: "5".repeat(40),
      now,
    }),
    /does not match/u,
  );
  assert.throws(
    () => verifyRecoveryProof({
      proof,
      manifest: resources,
      releaseSha: SHA,
      now: new Date(now.getTime() + 901_000),
    }),
    /stale/u,
  );
  assert.throws(
    () => verifyRecoveryProof({
      proof,
      manifest: resources,
      releaseSha: SHA,
      now,
      maxAgeSeconds: Number.NaN,
    }),
    /maximum age/u,
  );
});
