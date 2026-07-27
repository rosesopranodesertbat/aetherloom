import { existsSync } from "node:fs";
import { readFile, writeFile } from "node:fs/promises";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function fail(message) {
  throw new Error(message);
}

function option(name, required = true) {
  const index = process.argv.indexOf(name);
  if (index === -1) {
    if (required) fail(`missing ${name}`);
    return undefined;
  }
  if (index + 1 >= process.argv.length) fail(`missing value for ${name}`);
  return process.argv[index + 1];
}

function value(environmentName, resourceName, resources, required = true) {
  const candidate = process.env[environmentName] || resources[resourceName];
  if ((candidate === undefined || candidate === "") && required) {
    fail(`missing deployment value ${environmentName}`);
  }
  return candidate;
}

function identifier(input, label) {
  if (
    typeof input !== "string" ||
    !/^[A-Za-z0-9][A-Za-z0-9_.-]{1,127}$/u.test(input) ||
    input.includes("REPLACE_")
  ) {
    fail(`${label} is not a valid concrete resource identifier`);
  }
  return input;
}

function integer(input, label, minimum, maximum) {
  const parsed = Number(input);
  if (!Number.isInteger(parsed) || parsed < minimum || parsed > maximum) {
    fail(`${label} must be an integer from ${minimum} through ${maximum}`);
  }
  return parsed;
}

function boolean(input, label) {
  if (input === true || input === "true") return true;
  if (input === false || input === "false") return false;
  fail(`${label} must be true or false`);
}

function jsonDeploymentValue(name, resourceName, resources) {
  const source = process.env[name];
  if (source) {
    try {
      return JSON.parse(source);
    } catch {
      fail(`${name} must contain valid JSON`);
    }
  }
  const resourceValue = resources[resourceName];
  if (resourceValue === undefined) fail(`missing deployment value ${name}`);
  return resourceValue;
}

function allowedOrigins(resources) {
  const origins = jsonDeploymentValue(
    "AETHERLOOM_ALLOWED_ORIGINS_JSON",
    "allowedOrigins",
    resources,
  );
  if (!Array.isArray(origins) || origins.length === 0) {
    fail("AETHERLOOM_ALLOWED_ORIGINS_JSON must be a non-empty JSON array");
  }
  return origins.map((origin) => {
    if (typeof origin !== "string") fail("allowed origins must be strings");
    const url = new URL(origin);
    if (url.protocol !== "https:" || url.pathname !== "/" || url.search || url.hash) {
      fail("deployed allowed origins must be HTTPS origins without paths, queries, or fragments");
    }
    return url.origin;
  });
}

function resultHosts(resources) {
  const hosts = jsonDeploymentValue(
    "AETHERLOOM_RESULT_TICKET_HOSTS_JSON",
    "resultTicketHosts",
    resources,
  );
  if (hosts === null || typeof hosts !== "object" || Array.isArray(hosts)) {
    fail("AETHERLOOM_RESULT_TICKET_HOSTS_JSON must be a JSON object");
  }
  const entries = Object.entries(hosts);
  if (entries.length === 0) fail("at least one result signer mapping is required");
  for (const [keyId, hostId] of entries) {
    identifier(keyId, "result signer key id");
    identifier(hostId, "result signer host id");
  }
  return Object.fromEntries(entries);
}

function joinPublicKeys(resources) {
  const keys = jsonDeploymentValue(
    "JOIN_TICKET_PUBLIC_KEYS_JSON",
    "joinTicketPublicKeys",
    resources,
  );
  if (keys === null || typeof keys !== "object" || Array.isArray(keys)) {
    fail("JOIN_TICKET_PUBLIC_KEYS_JSON must be a JSON object");
  }
  const entries = Object.entries(keys);
  if (entries.length === 0) fail("at least one join verifier key is required");
  for (const [keyId, encodedKey] of entries) {
    identifier(keyId, "join verifier key id");
    if (
      typeof encodedKey !== "string" ||
      !/^[A-Za-z0-9_-]{43}$/u.test(encodedKey) ||
      Buffer.from(encodedKey, "base64url").byteLength !== 32
    ) {
      fail(`join verifier ${keyId} must be a raw 32-byte base64url Ed25519 public key`);
    }
  }
  return Object.fromEntries(entries);
}

const environment = option("--environment");
if (!["staging", "production"].includes(environment)) {
  fail("--environment must be staging or production");
}

const buildHash = option("--build-hash");
if (!/^[0-9a-f]{32}$/u.test(buildHash) || /^0{32}$/u.test(buildHash)) {
  fail("--build-hash must be a nonzero 128-bit lowercase hexadecimal content id");
}

const output = resolve(option("--output"));
if (
  dirname(output) !== root ||
  basename(output) !== `wrangler.generated.${environment}.jsonc`
) {
  fail(`output must be server/cloudflare/wrangler.generated.${environment}.jsonc`);
}

const requestedResources = option("--resources", false);
const defaultResources = join(root, "deploy", `${environment}.resources.json`);
const resourcesPath = requestedResources ? resolve(requestedResources) : defaultResources;
let resources = {};
if (requestedResources && !existsSync(resourcesPath)) {
  fail(`requested deployment resource manifest does not exist: ${resourcesPath}`);
}
if (existsSync(resourcesPath)) {
  resources = JSON.parse(await readFile(resourcesPath, "utf8"));
  if (resources.version !== 1 || resources.environment !== environment) {
    fail(`deployment resource manifest must be version 1 for ${environment}`);
  }
}

const workerName = identifier(
  value("CF_WORKER_NAME", "workerName", resources),
  "Worker name",
);
const d1DatabaseName = identifier(
  value("CF_D1_DATABASE_NAME", "d1DatabaseName", resources),
  "D1 database name",
);
const d1DatabaseId = identifier(
  value("CF_D1_DATABASE_ID", "d1DatabaseId", resources),
  "D1 database id",
);
const r2BucketName = identifier(
  value("CF_MATCH_ARCHIVE_BUCKET", "r2BucketName", resources),
  "R2 bucket name",
);
const settlementQueue = identifier(
  value("CF_SETTLEMENT_QUEUE", "settlementQueue", resources),
  "settlement Queue name",
);
const settlementDeadLetterQueue = identifier(
  value("CF_SETTLEMENT_DLQ", "settlementDeadLetterQueue", resources),
  "settlement dead-letter Queue name",
);
const workersDev = boolean(
  value("CF_WORKERS_DEV", "workersDev", resources),
  "workersDev",
);
const customDomain = value("CF_WORKER_CUSTOM_DOMAIN", "customDomain", resources, false);
if (!workersDev && !customDomain) {
  fail("a custom domain is required when workers.dev is disabled");
}
if (customDomain) {
  const url = new URL(`https://${customDomain}`);
  if (url.hostname !== customDomain || url.pathname !== "/") {
    fail("CF_WORKER_CUSTOM_DOMAIN must be a bare hostname");
  }
}

const matchCapacityWorker = value(
  "CF_MATCH_CAPACITY_WORKER",
  "matchCapacityWorker",
  resources,
  environment === "production",
);
const browserMatchWorker = value(
  "CF_BROWSER_MATCH_WORKER",
  "browserMatchWorker",
  resources,
  environment === "production",
);
if (Boolean(matchCapacityWorker) !== Boolean(browserMatchWorker)) {
  fail("capacity and browser-match service bindings must be configured together");
}

const origins = allowedOrigins(resources);
const signerHosts = resultHosts(resources);
const publicJoinKeys = joinPublicKeys(resources);
const activeJoinTicketKid = identifier(
  value("ACTIVE_JOIN_TICKET_KID", "activeJoinTicketKid", resources),
  "active join ticket key id",
);
if (!Object.hasOwn(publicJoinKeys, activeJoinTicketKid)) {
  fail("ACTIVE_JOIN_TICKET_KID is not present in JOIN_TICKET_PUBLIC_KEYS_JSON");
}
const activeServiceTicketKid = identifier(
  value("ACTIVE_SERVICE_TICKET_KID", "activeServiceTicketKid", resources),
  "active service ticket key id",
);
const matchmakingShardCount = integer(
  value("CF_MATCHMAKING_SHARD_COUNT", "matchmakingShardCount", resources),
  "matchmaking shard count",
  1,
  256,
);

const config = {
  $schema: "./node_modules/wrangler/config-schema.json",
  name: workerName,
  main: "src/index.ts",
  compatibility_date: "2026-07-27",
  workers_dev: workersDev,
  minify: true,
  observability: { enabled: true },
  vars: {
    ENVIRONMENT: environment,
    TICKET_ISSUER: "aetherloom-control-plane",
    PLAYER_TICKET_AUDIENCE: "aetherloom-player",
    JOIN_TICKET_AUDIENCE: "aetherloom-match",
    SERVICE_TICKET_AUDIENCE: "aetherloom-services",
    RESULT_TICKET_AUDIENCE: "aetherloom-results",
    RESULT_TICKET_HOSTS_JSON: JSON.stringify(signerHosts),
    JOIN_TICKET_PUBLIC_KEYS_JSON: JSON.stringify(publicJoinKeys),
    ACTIVE_JOIN_TICKET_KID: activeJoinTicketKid,
    ACTIVE_SERVICE_TICKET_KID: activeServiceTicketKid,
    CONTENT_BUILD_HASH: buildHash,
    ALLOWED_ORIGINS_JSON: JSON.stringify(origins),
    MATCHMAKING_SHARD_COUNT: String(matchmakingShardCount),
    MATCHMAKING_SKILL_BUCKET_WIDTH: String(
      integer(process.env.CF_MATCHMAKING_SKILL_BUCKET_WIDTH || 200, "skill bucket width", 1, 10_000),
    ),
    MAX_ISLAND_CHECKPOINT_BYTES: String(
      integer(process.env.CF_MAX_ISLAND_CHECKPOINT_BYTES || 8_388_608, "checkpoint byte limit", 1, 64 * 1024 * 1024),
    ),
  },
  secrets: {
    required: [
      "PLAYER_TICKET_KEYS_JSON",
      "JOIN_TICKET_SIGNING_KEYS_JSON",
      "SERVICE_TICKET_KEYS_JSON",
      "RESULT_TICKET_KEYS_JSON",
    ],
  },
  durable_objects: {
    bindings: [
      { name: "PROFILE_ISLAND", class_name: "ProfileIslandObject" },
      { name: "MATCHMAKING_SHARD", class_name: "MatchmakingShardObject" },
    ],
  },
  exports: {
    ProfileIslandObject: { type: "durable-object", storage: "sqlite" },
    MatchmakingShardObject: { type: "durable-object", storage: "sqlite" },
  },
  d1_databases: [
    {
      binding: "CONTROL_DB",
      database_name: d1DatabaseName,
      database_id: d1DatabaseId,
      migrations_dir: "migrations",
    },
  ],
  r2_buckets: [{ binding: "MATCH_ARCHIVE", bucket_name: r2BucketName }],
  queues: {
    producers: [{ binding: "SETTLEMENT_QUEUE", queue: settlementQueue }],
    consumers: [
      {
        queue: settlementQueue,
        max_batch_size: 10,
        max_batch_timeout: 5,
        max_retries: 10,
        dead_letter_queue: settlementDeadLetterQueue,
        max_concurrency: environment === "production" ? 16 : 4,
      },
    ],
  },
};

if (customDomain) {
  config.routes = [{ pattern: customDomain, custom_domain: true }];
}
if (matchCapacityWorker && browserMatchWorker) {
  config.services = [
    {
      binding: "MATCH_CAPACITY_API",
      service: identifier(matchCapacityWorker, "capacity Worker"),
    },
    {
      binding: "BROWSER_MATCH_ORIGIN",
      service: identifier(browserMatchWorker, "browser-match Worker"),
    },
  ];
}

await writeFile(output, `${JSON.stringify(config, null, 2)}\n`, {
  flag: "w",
  mode: 0o600,
});
console.log(`rendered ${environment} Wrangler config for build ${buildHash}`);
