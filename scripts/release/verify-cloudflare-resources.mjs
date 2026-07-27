#!/usr/bin/env node

import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const REQUIRED_SECRETS = [
  "JOIN_TICKET_SIGNING_KEYS_JSON",
  "PLAYER_TICKET_KEYS_JSON",
  "RESULT_TICKET_KEYS_JSON",
  "SERVICE_TICKET_KEYS_JSON",
];
const HEX_256_RE = /^[0-9a-f]{64}$/u;
const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u;
const QUEUE_ID_RE = /^[0-9a-f]{32}$/u;

function fail(message) {
  throw new Error(message);
}

function canonicalJson(value) {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (value !== null && typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
}

function sha256(value) {
  return createHash("sha256").update(value, "utf8").digest("hex");
}

export function accountFingerprint(accountId) {
  if (!/^[0-9a-f]{32}$/u.test(accountId ?? "")) {
    fail("CLOUDFLARE_ACCOUNT_ID must be a lowercase 32-character account id");
  }
  return sha256(`cloudflare-account:v1:${accountId}`);
}

export function manifestFingerprint(manifest) {
  const { resourceFingerprint: _fingerprint, ...identity } = manifest;
  return sha256(canonicalJson(identity));
}

function requireString(value, label) {
  if (typeof value !== "string" || value.length === 0) fail(`${label} is missing`);
  return value;
}

function assertManifest(manifest, environment, accountId) {
  if (
    manifest?.version !== 1 ||
    manifest.environment !== environment ||
    !HEX_256_RE.test(manifest.accountIdSha256 ?? "") ||
    !HEX_256_RE.test(manifest.resourceFingerprint ?? "") ||
    !UUID_RE.test(manifest.d1DatabaseId ?? "") ||
    !QUEUE_ID_RE.test(manifest.settlementQueueId ?? "") ||
    !QUEUE_ID_RE.test(manifest.settlementDeadLetterQueueId ?? "")
  ) {
    fail("committed Cloudflare resource manifest is incomplete or invalid");
  }
  for (const name of [
    "workerName",
    "d1DatabaseName",
    "r2BucketName",
    "r2Location",
    "settlementQueue",
    "settlementDeadLetterQueue",
  ]) {
    requireString(manifest[name], `manifest ${name}`);
  }
  if (manifest.accountIdSha256 !== accountFingerprint(accountId)) {
    fail("Cloudflare account does not match the committed resource fingerprint");
  }
  if (manifest.resourceFingerprint !== manifestFingerprint(manifest)) {
    fail("committed Cloudflare resource fingerprint is invalid");
  }
}

function assertConfig(manifest, config) {
  const d1 = config?.d1_databases?.find((binding) => binding.binding === "CONTROL_DB");
  const r2 = config?.r2_buckets?.find((binding) => binding.binding === "MATCH_ARCHIVE");
  const producer = config?.queues?.producers?.find(
    (binding) => binding.binding === "SETTLEMENT_QUEUE",
  );
  const consumer = config?.queues?.consumers?.find(
    (binding) => binding.queue === manifest.settlementQueue,
  );
  if (
    config?.name !== manifest.workerName ||
    config?.workers_dev !== manifest.workersDev ||
    config?.vars?.ENVIRONMENT !== manifest.environment ||
    d1?.database_name !== manifest.d1DatabaseName ||
    d1?.database_id !== manifest.d1DatabaseId ||
    r2?.bucket_name !== manifest.r2BucketName ||
    producer?.queue !== manifest.settlementQueue ||
    consumer?.dead_letter_queue !== manifest.settlementDeadLetterQueue
  ) {
    fail("rendered Wrangler config does not match the committed resource manifest");
  }
  const route = config?.routes?.find((candidate) => candidate.custom_domain === true);
  if ((route?.pattern ?? null) !== (manifest.customDomain ?? null)) {
    fail("rendered Worker custom domain does not match the committed resource manifest");
  }
  const services = Object.fromEntries(
    (config?.services ?? []).map((service) => [service.binding, service.service]),
  );
  if (
    (services.MATCH_CAPACITY_API ?? null) !== (manifest.matchCapacityWorker ?? null) ||
    (services.BROWSER_MATCH_ORIGIN ?? null) !== (manifest.browserMatchWorker ?? null)
  ) {
    fail("rendered service bindings do not match the committed resource manifest");
  }
}

function parseQueueInfo(text, label) {
  const name = /^Queue Name:\s*(\S+)\s*$/mu.exec(text)?.[1];
  const id = /^Queue ID:\s*([0-9a-f]{32})\s*$/mu.exec(text)?.[1];
  if (!name || !id) fail(`${label} metadata is malformed`);
  return { name, id };
}

export function verifyResourceIdentity({
  environment,
  accountId,
  manifest,
  config,
  d1Info,
  r2Info,
  settlementQueueInfo,
  settlementDeadLetterQueueInfo,
  secrets,
  deployments,
}) {
  if (!["staging", "production"].includes(environment)) {
    fail("environment must be staging or production");
  }
  assertManifest(manifest, environment, accountId);
  assertConfig(manifest, config);
  if (
    d1Info?.name !== manifest.d1DatabaseName ||
    d1Info?.uuid !== manifest.d1DatabaseId
  ) {
    fail("remote D1 database identity does not match the committed manifest");
  }
  if (
    r2Info?.name !== manifest.r2BucketName ||
    r2Info?.location !== manifest.r2Location
  ) {
    fail("remote R2 bucket identity does not match the committed manifest");
  }
  const queue = parseQueueInfo(settlementQueueInfo, "settlement Queue");
  const dlq = parseQueueInfo(settlementDeadLetterQueueInfo, "settlement dead-letter Queue");
  if (
    queue.name !== manifest.settlementQueue ||
    queue.id !== manifest.settlementQueueId ||
    dlq.name !== manifest.settlementDeadLetterQueue ||
    dlq.id !== manifest.settlementDeadLetterQueueId
  ) {
    fail("remote Queue identity does not match the committed manifest");
  }
  const secretNames = secrets
    .map((secret) => secret?.name)
    .filter((name) => typeof name === "string")
    .sort();
  if (canonicalJson(secretNames) !== canonicalJson(REQUIRED_SECRETS)) {
    fail("remote Worker secret names do not exactly match the required set");
  }
  if (
    !Array.isArray(deployments) ||
    deployments.length < 1 ||
    deployments.some(
      (deployment) =>
        typeof deployment?.id !== "string" ||
        !Array.isArray(deployment?.versions) ||
        deployment.versions.length < 1,
    )
  ) {
    fail("remote Worker has no valid deployment to receive a migration");
  }
  return manifest.resourceFingerprint;
}

function option(name) {
  const index = process.argv.indexOf(name);
  if (index < 0 || index + 1 >= process.argv.length) fail(`missing ${name}`);
  return process.argv[index + 1];
}

async function jsonFile(path, label) {
  try {
    return JSON.parse(await readFile(path, "utf8"));
  } catch {
    fail(`${label} is not valid JSON`);
  }
}

async function main() {
  const command = process.argv[2];
  if (command === "account-fingerprint") {
    process.stdout.write(accountFingerprint(process.env.CLOUDFLARE_ACCOUNT_ID));
    return;
  }
  const manifestPath = option("--manifest");
  const manifest = await jsonFile(manifestPath, "resource manifest");
  if (command === "fingerprint") {
    process.stdout.write(manifestFingerprint(manifest));
    return;
  }
  if (command !== "verify") {
    fail("usage: verify-cloudflare-resources.mjs <account-fingerprint|fingerprint|verify> [options]");
  }
  const fingerprint = verifyResourceIdentity({
    environment: option("--environment"),
    accountId: process.env.CLOUDFLARE_ACCOUNT_ID,
    manifest,
    config: await jsonFile(option("--config"), "Wrangler config"),
    d1Info: await jsonFile(option("--d1-info"), "D1 metadata"),
    r2Info: await jsonFile(option("--r2-info"), "R2 metadata"),
    settlementQueueInfo: await readFile(option("--settlement-queue-info"), "utf8"),
    settlementDeadLetterQueueInfo: await readFile(
      option("--settlement-dlq-info"),
      "utf8",
    ),
    secrets: await jsonFile(option("--secrets"), "Worker secret metadata"),
    deployments: await jsonFile(option("--deployments"), "Worker deployments"),
  });
  console.log(`verified ${option("--environment")} Cloudflare resource identity ${fingerprint}`);
}

const invoked = process.argv[1] === undefined
  ? undefined
  : pathToFileURL(resolve(process.argv[1])).href;
if (invoked === import.meta.url) {
  main().catch((error) => {
    console.error(`Cloudflare resource verification failed: ${error.message}`);
    process.exitCode = 1;
  });
}
