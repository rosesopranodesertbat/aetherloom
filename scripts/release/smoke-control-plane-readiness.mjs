#!/usr/bin/env node

import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

import {
  mintHmacTicket,
  readWorkerSecretBundle,
} from "./smoke-control-plane-authenticated.mjs";

const HEX_128_RE = /^[0-9a-f]{32}$/u;
const IDENTIFIER_RE = /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$/u;
const MAX_RESPONSE_BYTES = 64 * 1024;

function fail(message) {
  throw new Error(message);
}

function option(argumentsList, name) {
  const index = argumentsList.indexOf(name);
  if (index < 0) return undefined;
  const value = argumentsList[index + 1];
  if (value === undefined || value.startsWith("--")) fail(`${name} requires a value`);
  return value;
}

function baseUrl(input) {
  const url = new URL(input);
  const local = ["localhost", "127.0.0.1"].includes(url.hostname);
  if (url.protocol !== "https:" && !(local && url.protocol === "http:")) {
    fail("readiness target must use HTTPS, except for localhost tests");
  }
  if (url.username || url.password || url.search || url.hash) {
    fail("readiness target must not contain credentials, a query, or a fragment");
  }
  url.pathname = url.pathname.replace(/\/+$/u, "");
  return url;
}

function endpoint(url, path) {
  const target = new URL(url);
  target.pathname = `${target.pathname.replace(/\/+$/u, "")}${path}`;
  return target;
}

function tamperSignature(ticket) {
  const parts = ticket.split(".");
  const first = parts[2][0] === "A" ? "B" : "A";
  parts[2] = `${first}${parts[2].slice(1)}`;
  return parts.join(".");
}

async function json(response, label) {
  const body = await response.arrayBuffer();
  if (body.byteLength > MAX_RESPONSE_BYTES) fail(`${label} response is too large`);
  if (!response.headers.get("content-type")?.toLowerCase().includes("application/json")) {
    fail(`${label} did not return JSON`);
  }
  try {
    return JSON.parse(Buffer.from(body).toString("utf8"));
  } catch {
    fail(`${label} returned malformed JSON`);
  }
}

function securityHeaders(response, label) {
  if (
    response.headers.get("x-content-type-options") !== "nosniff" ||
    response.headers.get("referrer-policy") !== "no-referrer" ||
    !response.headers.get("x-aetherloom-request-id")
  ) {
    fail(`${label} is missing required security headers`);
  }
}

async function expectAuthFailure(response, code, label) {
  if (response.status !== 401) fail(`${label} returned HTTP ${response.status}; expected 401`);
  securityHeaders(response, label);
  const body = await json(response, label);
  if (body?.error?.code !== code) fail(`${label} returned the wrong authentication error`);
}

export async function runReadinessSmoke(options, report = console.log) {
  const url = baseUrl(options.url);
  if (!["staging", "production"].includes(options.environment)) {
    fail("environment must be staging or production");
  }
  if (!HEX_128_RE.test(options.expectedBuild ?? "") || /^0{32}$/u.test(options.expectedBuild)) {
    fail("expected build must be a nonzero 128-bit lowercase hexadecimal content id");
  }
  for (const [value, label] of [
    [options.issuer ?? "aetherloom-control-plane", "issuer"],
    [options.serviceAudience ?? "aetherloom-services", "service audience"],
  ]) {
    if (!IDENTIFIER_RE.test(value)) fail(`${label} is invalid`);
  }
  const secrets = await readWorkerSecretBundle(options.secretsFile, {
    service: options.serviceKeyId,
  });
  const timeoutMs = options.timeoutMs ?? 10_000;
  if (!Number.isInteger(timeoutMs) || timeoutMs < 1_000 || timeoutMs > 60_000) {
    fail("timeout must be an integer between 1000 and 60000 milliseconds");
  }
  const now = Math.floor(Date.now() / 1_000);
  const serviceTicket = mintHmacTicket(
    {
      v: 1,
      iss: options.issuer ?? "aetherloom-control-plane",
      aud: options.serviceAudience ?? "aetherloom-services",
      purpose: "service",
      sub: "deployment-readiness-probe",
      iat: now,
      nbf: now - 2,
      exp: now + 120,
      nonce: "dddddddddddddddddddddddddddddddd",
      scopes: ["deployment:verify"],
    },
    secrets.service,
  );
  const fetchImpl = options.fetchImpl ?? fetch;
  const request = (token) =>
    fetchImpl(endpoint(url, "/internal/v1/readiness"), {
      method: "POST",
      redirect: "error",
      signal: AbortSignal.timeout(timeoutMs),
      headers: {
        accept: "application/json",
        "cache-control": "no-store",
        "user-agent": "aetherloom-readiness-smoke/1",
        ...(token === undefined ? {} : { authorization: `Bearer ${token}` }),
      },
    });

  await expectAuthFailure(await request(), "missing_ticket", "unauthenticated readiness");
  await expectAuthFailure(
    await request(tamperSignature(serviceTicket)),
    "invalid_ticket_signature",
    "tampered readiness authentication",
  );
  const response = await request(serviceTicket);
  if (response.status !== 200) {
    fail(`authenticated readiness returned HTTP ${response.status}; expected 200`);
  }
  securityHeaders(response, "authenticated readiness");
  const body = await json(response, "authenticated readiness");
  if (
    body?.status !== "ready" ||
    body.environment !== options.environment ||
    body.buildHash !== options.expectedBuild ||
    body.authentication !== "verified" ||
    body.cryptography !== "verified"
  ) {
    fail("authenticated readiness payload does not match the promoted environment and build");
  }
  report(`PASS authenticated ${options.environment} cryptographic readiness`);
}

export function parseArguments(argumentsList) {
  const known = new Set([
    "--url",
    "--environment",
    "--expected-build",
    "--secrets-file",
    "--service-kid",
    "--issuer",
    "--service-audience",
    "--timeout-ms",
  ]);
  for (let index = 0; index < argumentsList.length; index += 2) {
    if (!known.has(argumentsList[index])) fail(`unknown argument: ${argumentsList[index]}`);
  }
  const timeout = option(argumentsList, "--timeout-ms");
  return {
    url: option(argumentsList, "--url"),
    environment: option(argumentsList, "--environment"),
    expectedBuild: option(argumentsList, "--expected-build"),
    secretsFile: option(argumentsList, "--secrets-file"),
    serviceKeyId: option(argumentsList, "--service-kid"),
    issuer: option(argumentsList, "--issuer"),
    serviceAudience: option(argumentsList, "--service-audience"),
    timeoutMs: timeout === undefined ? undefined : Number(timeout),
  };
}

async function main() {
  const options = parseArguments(process.argv.slice(2));
  if (
    options.url === undefined ||
    options.environment === undefined ||
    options.expectedBuild === undefined ||
    options.secretsFile === undefined
  ) {
    fail(
      "usage: smoke-control-plane-readiness.mjs --url <https-url> --environment <staging|production> --expected-build <hex128> --secrets-file <mode-0600-json>",
    );
  }
  await runReadinessSmoke(options);
}

const invoked = process.argv[1] === undefined
  ? undefined
  : pathToFileURL(resolve(process.argv[1])).href;
if (invoked === import.meta.url) {
  main().catch((error) => {
    console.error(`Control-plane readiness failed: ${error.message}`);
    process.exitCode = 1;
  });
}
