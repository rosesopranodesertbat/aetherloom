#!/usr/bin/env node

import { readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

import { manifestFingerprint } from "./verify-cloudflare-resources.mjs";

const BOOKMARK_RE = /^[0-9a-f]{8}-[0-9a-f]{8}-[0-9a-f]{8}-[0-9a-f]{32}$/u;
const SHA_RE = /^[0-9a-f]{40}$/u;

function fail(message) {
  throw new Error(message);
}

function validateInputs(bookmarkResponse, manifest, releaseSha) {
  if (!BOOKMARK_RE.test(bookmarkResponse?.bookmark ?? "")) {
    fail("D1 Time Travel did not return a valid recovery bookmark");
  }
  if (!SHA_RE.test(releaseSha ?? "")) {
    fail("release SHA must be a full lowercase 40-character Git SHA");
  }
  if (
    manifest?.version !== 1 ||
    manifest.environment !== "production" ||
    manifest.resourceFingerprint !== manifestFingerprint(manifest)
  ) {
    fail("production resource manifest fingerprint is invalid");
  }
}

export function buildRecoveryProof({
  bookmarkResponse,
  manifest,
  releaseSha,
  now = new Date(),
}) {
  validateInputs(bookmarkResponse, manifest, releaseSha);
  return {
    version: 1,
    environment: "production",
    databaseName: manifest.d1DatabaseName,
    databaseId: manifest.d1DatabaseId,
    resourceFingerprint: manifest.resourceFingerprint,
    releaseSha,
    bookmark: bookmarkResponse.bookmark,
    capturedAt: now.toISOString(),
    restoreCommand:
      `wrangler d1 time-travel restore ${manifest.d1DatabaseName} --bookmark ${bookmarkResponse.bookmark}`,
  };
}

export function verifyRecoveryProof({
  proof,
  manifest,
  releaseSha,
  now = new Date(),
  maxAgeSeconds = 900,
}) {
  validateInputs({ bookmark: proof?.bookmark }, manifest, releaseSha);
  if (!Number.isInteger(maxAgeSeconds) || maxAgeSeconds < 1 || maxAgeSeconds > 3_600) {
    fail("D1 recovery proof maximum age must be from 1 through 3600 seconds");
  }
  const ageMs = now.getTime() - Date.parse(proof?.capturedAt);
  const expectedRestoreCommand =
    `wrangler d1 time-travel restore ${manifest.d1DatabaseName} --bookmark ${proof?.bookmark}`;
  if (
    proof?.version !== 1 ||
    proof.environment !== "production" ||
    proof.databaseName !== manifest.d1DatabaseName ||
    proof.databaseId !== manifest.d1DatabaseId ||
    proof.resourceFingerprint !== manifest.resourceFingerprint ||
    proof.releaseSha !== releaseSha ||
    proof.restoreCommand !== expectedRestoreCommand ||
    !Number.isFinite(ageMs) ||
    ageMs < -60_000 ||
    ageMs > maxAgeSeconds * 1_000
  ) {
    fail("D1 recovery proof is stale or does not match this production migration");
  }
  return proof.bookmark;
}

function option(name) {
  const index = process.argv.indexOf(name);
  if (index < 0 || index + 1 >= process.argv.length) fail(`missing ${name}`);
  return process.argv[index + 1];
}

async function json(path, label) {
  try {
    return JSON.parse(await readFile(path, "utf8"));
  } catch {
    fail(`${label} is not valid JSON`);
  }
}

async function main() {
  const command = process.argv[2];
  const manifest = await json(option("--manifest"), "production resource manifest");
  const releaseSha = option("--release-sha");
  if (command === "create") {
    const proof = buildRecoveryProof({
      bookmarkResponse: await json(option("--bookmark-json"), "D1 bookmark response"),
      manifest,
      releaseSha,
    });
    await writeFile(option("--output"), `${JSON.stringify(proof, null, 2)}\n`, {
      flag: "wx",
      mode: 0o600,
    });
    console.log("captured production D1 Time Travel recovery point");
    return;
  }
  if (command === "verify") {
    verifyRecoveryProof({
      proof: await json(option("--proof"), "D1 recovery proof"),
      manifest,
      releaseSha,
      maxAgeSeconds: Number(option("--max-age-seconds")),
    });
    console.log("verified fresh production D1 recovery point");
    return;
  }
  fail("usage: d1-recovery-proof.mjs <create|verify> [options]");
}

const invoked = process.argv[1] === undefined
  ? undefined
  : pathToFileURL(resolve(process.argv[1])).href;
if (invoked === import.meta.url) {
  main().catch((error) => {
    console.error(`D1 recovery gate failed: ${error.message}`);
    process.exitCode = 1;
  });
}
