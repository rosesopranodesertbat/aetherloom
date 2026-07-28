import { readFile, writeFile } from "node:fs/promises";

function fail(message) {
  throw new Error(message);
}

function option(name) {
  const index = process.argv.indexOf(name);
  if (index === -1 || index + 1 >= process.argv.length) {
    fail(`missing ${name}`);
  }
  return process.argv[index + 1];
}

function validateCommit(value) {
  if (!/^[0-9a-f]{40}$/u.test(value)) {
    fail("commit must be a full lowercase 40-character Git SHA");
  }
  return value;
}

function validateBuildHash(value) {
  if (!/^[0-9a-f]{32}$/u.test(value) || /^0{32}$/u.test(value)) {
    fail("build hash must be a nonzero 128-bit lowercase hexadecimal content id");
  }
  return value;
}

function validateService(value) {
  if (!["control-plane"].includes(value)) {
    fail("unsupported deployment proof service");
  }
  return value;
}

function validateUrl(value) {
  const url = new URL(value);
  if (url.protocol !== "https:") {
    fail("deployment proof URL must use HTTPS");
  }
  return url.toString().replace(/\/$/u, "");
}

async function createProof() {
  const proof = {
    version: 1,
    environment: option("--environment"),
    service: validateService(option("--service")),
    commit: validateCommit(option("--commit")),
    buildHash: validateBuildHash(option("--build-hash")),
    url: validateUrl(option("--url")),
    checkedAt: new Date().toISOString(),
  };
  if (!["staging", "production"].includes(proof.environment)) {
    fail("deployment proof environment must be staging or production");
  }
  await writeFile(option("--output"), `${JSON.stringify(proof, null, 2)}\n`, { flag: "w" });
  console.log(`recorded ${proof.environment} ${proof.service} smoke proof`);
}

async function verifyProof() {
  const proof = JSON.parse(await readFile(option("--proof"), "utf8"));
  const expectedEnvironment = option("--environment");
  const expectedService = validateService(option("--service"));
  const expectedCommit = validateCommit(option("--commit"));
  const expectedBuildHash = validateBuildHash(option("--build-hash"));
  if (
    proof?.version !== 1 ||
    proof.environment !== expectedEnvironment ||
    proof.service !== expectedService ||
    proof.commit !== expectedCommit ||
    proof.buildHash !== expectedBuildHash ||
    typeof proof.checkedAt !== "string" ||
    !Number.isFinite(Date.parse(proof.checkedAt))
  ) {
    fail("deployment proof does not match the requested promotion");
  }
  validateUrl(proof.url);
  console.log(`verified ${expectedEnvironment} ${expectedService} smoke proof for ${expectedCommit}`);
}

const command = process.argv[2];
if (command === "create") {
  await createProof();
} else if (command === "verify") {
  await verifyProof();
} else {
  fail("usage: deployment-proof.mjs <create|verify> [options]");
}
