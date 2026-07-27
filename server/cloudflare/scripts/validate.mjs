import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

function stripJsonComments(source) {
  let output = "";
  let inString = false;
  let escaped = false;
  for (let index = 0; index < source.length; index += 1) {
    const character = source[index];
    const next = source[index + 1];
    if (inString) {
      output += character;
      if (escaped) escaped = false;
      else if (character === "\\") escaped = true;
      else if (character === '"') inString = false;
      continue;
    }
    if (character === '"') {
      inString = true;
      output += character;
      continue;
    }
    if (character === "/" && next === "/") {
      while (index < source.length && source[index] !== "\n") index += 1;
      output += "\n";
      continue;
    }
    if (character === "/" && next === "*") {
      index += 2;
      while (index < source.length && !(source[index] === "*" && source[index + 1] === "/")) {
        index += 1;
      }
      index += 1;
      continue;
    }
    output += character;
  }
  return output.replace(/,\s*([}\]])/gu, "$1");
}

async function json(relativePath, jsonc = false) {
  const source = await readFile(join(root, relativePath), "utf8");
  return JSON.parse(jsonc ? stripJsonComments(source) : source);
}

const requiredFiles = [
  "README.md",
  "wrangler.jsonc",
  "wrangler.production.example.jsonc",
  "migrations/0001_control_plane.sql",
  "migrations/0002_match_epoch.sql",
  "schemas/ticket-claims.schema.json",
  "schemas/join-ticket-claims.schema.json",
  "schemas/settlement-message.schema.json",
  "schemas/matchmaking-enqueue.schema.json",
  "src/index.ts",
  "src/gateway.ts",
  "src/dispatch-invariants.ts",
  "src/profile-island.ts",
  "src/matchmaking.ts",
  "src/match-director.ts",
  "src/island-runtime.ts",
  "src/island-module.ts",
  "src/join-tickets.ts",
  "src/readiness.ts",
  "src/settlements.ts",
  "src/ticket-invariants.ts",
  "src/tickets.ts",
];
await Promise.all(requiredFiles.map((file) => readFile(join(root, file), "utf8")));

const config = await json("wrangler.jsonc", true);
assert.equal(config.compatibility_date, "2026-07-27");
assert.equal(config.main, "src/index.ts");
assert.equal(config.vars.ENVIRONMENT, "local");
assert.match(config.vars.CONTENT_BUILD_HASH, /^[0-9a-f]{32}$/u);
assert.equal(config.vars.RESULT_TICKET_AUDIENCE, "aetherloom-results");
assert.notEqual(config.vars.RESULT_TICKET_AUDIENCE, config.vars.SERVICE_TICKET_AUDIENCE);
assert.deepEqual(JSON.parse(config.vars.RESULT_TICKET_HOSTS_JSON), {
  "local-result-host-v1": "local-match-host",
});
assert.equal(config.vars.ACTIVE_JOIN_TICKET_KID, "local-join-v1");
assert.equal(config.vars.ACTIVE_SERVICE_TICKET_KID, "local-service-v1");
assert.equal(
  Object.keys(JSON.parse(config.vars.JOIN_TICKET_PUBLIC_KEYS_JSON))[0],
  config.vars.ACTIVE_JOIN_TICKET_KID,
);
assert.deepEqual(
  config.durable_objects.bindings.map((binding) => binding.name).sort(),
  ["MATCHMAKING_SHARD", "PROFILE_ISLAND"],
);
assert.equal(config.exports.ProfileIslandObject.storage, "sqlite");
assert.equal(config.exports.MatchmakingShardObject.storage, "sqlite");
assert.deepEqual(config.d1_databases.map((binding) => binding.binding), ["CONTROL_DB"]);
assert.deepEqual(config.r2_buckets.map((binding) => binding.binding), ["MATCH_ARCHIVE"]);
assert.deepEqual(config.queues.producers.map((binding) => binding.binding), ["SETTLEMENT_QUEUE"]);
assert.ok(config.queues.consumers[0].dead_letter_queue);
assert.deepEqual(config.secrets.required.sort(), [
  "JOIN_TICKET_SIGNING_KEYS_JSON",
  "PLAYER_TICKET_KEYS_JSON",
  "RESULT_TICKET_KEYS_JSON",
  "SERVICE_TICKET_KEYS_JSON",
]);

const production = await json("wrangler.production.example.jsonc", true);
assert.equal(production.workers_dev, false);
assert.ok(production.d1_databases[0].database_id.startsWith("REPLACE_WITH_"));
assert.ok(production.vars.CONTENT_BUILD_HASH.startsWith("REPLACE_WITH_"));
assert.equal(production.vars.RESULT_TICKET_AUDIENCE, "aetherloom-results");
assert.notEqual(production.vars.RESULT_TICKET_AUDIENCE, production.vars.SERVICE_TICKET_AUDIENCE);
assert.ok(Object.keys(JSON.parse(production.vars.RESULT_TICKET_HOSTS_JSON))[0].startsWith("REPLACE_WITH_"));
assert.deepEqual(
  production.services.map((service) => service.binding).sort(),
  ["BROWSER_MATCH_ORIGIN", "MATCH_CAPACITY_API"],
);

const stagingResources = await json("deploy/staging.resources.json");
assert.equal(stagingResources.version, 1);
assert.equal(stagingResources.environment, "staging");
assert.match(stagingResources.accountIdSha256, /^[0-9a-f]{64}$/u);
assert.match(stagingResources.resourceFingerprint, /^[0-9a-f]{64}$/u);
assert.match(stagingResources.settlementQueueId, /^[0-9a-f]{32}$/u);
assert.match(stagingResources.settlementDeadLetterQueueId, /^[0-9a-f]{32}$/u);

for (const schemaFile of (await readdir(join(root, "schemas"))).filter((file) => file.endsWith(".json"))) {
  const schema = await json(join("schemas", schemaFile));
  assert.equal(schema.$schema, "https://json-schema.org/draft/2020-12/schema");
  assert.equal(schema.type, "object");
}

const migration = await readFile(join(root, "migrations/0001_control_plane.sql"), "utf8");
const matchEpochMigration = await readFile(join(root, "migrations/0002_match_epoch.sql"), "utf8");
assert.match(matchEpochMigration, /ADD COLUMN match_epoch INTEGER NOT NULL/u);
for (const table of [
  "match_allocations",
  "profile_projection",
  "settlement_projection",
  "match_history",
]) {
  assert.match(migration, new RegExp(`CREATE TABLE IF NOT EXISTS ${table}\\b`, "u"));
}
for (const column of [
  "dispatch_hash TEXT NOT NULL",
  "roster_hash TEXT NOT NULL",
  "matchmaking_shard TEXT",
  "lease_id TEXT",
  "assignments_json TEXT",
  "reservations_json TEXT",
  "queue_confirmed_at_ms INTEGER",
]) {
  assert.match(migration, new RegExp(column.replaceAll(" ", "\\s+"), "u"));
}

const source = await Promise.all(
  (await readdir(join(root, "src")))
    .filter((file) => file.endsWith(".ts"))
    .map((file) => readFile(join(root, "src", file), "utf8")),
).then((files) => files.join("\n"));
assert.match(source, /AUTHORITATIVE_HZ = 128/u);
assert.match(source, /class ProfileIslandObject/u);
assert.match(source, /class MatchmakingShardObject/u);
assert.match(source, /class ServiceBindingMatchDirector/u);
assert.match(source, /ISLAND_TICK_HZ = 128/u);
assert.match(source, /class HeadlessWasmIslandSimulation/u);
assert.match(source, /island_command_journal/u);
assert.match(source, /dormantEconomy: "timestamp-derived"/u);
assert.match(source, /aetherloom\.auth\./u);
assert.match(source, /transactionSync/u);
assert.match(source, /payloadHash/u);
assert.doesNotMatch(
  source,
  /pathname\.startsWith\("\/v1\/profile\/island\/checkpoints\/"\)/u,
  "players must never upload authoritative online-island checkpoints",
);
assert.match(source, /serviceRequest\(request, env, \["island:checkpoint"\]\)/u);
const ticketsSource = await readFile(join(root, "src/tickets.ts"), "utf8");
const joinTicketsSource = await readFile(join(root, "src/join-tickets.ts"), "utf8");
const readinessSource = await readFile(join(root, "src/readiness.ts"), "utf8");
const settlementsSource = await readFile(join(root, "src/settlements.ts"), "utf8");
const gatewaySource = await readFile(join(root, "src/gateway.ts"), "utf8");
const directorSource = await readFile(join(root, "src/match-director.ts"), "utf8");
const matchmakingSource = await readFile(join(root, "src/matchmaking.ts"), "utf8");
const profileSource = await readFile(join(root, "src/profile-island.ts"), "utf8");
const ticketInvariantsSource = await readFile(join(root, "src/ticket-invariants.ts"), "utf8");
assert.match(gatewaySource, /checkpointObjectKey\(accountId, sha256\)/u);
assert.doesNotMatch(
  gatewaySource,
  /sha256Base64Url\(`\$\{accountId\}:\$\{checkpointId\}`\)/u,
);
assert.match(profileSource, /objectKey !== checkpointObjectKey\(accountId, sha256\)/u);
assert.match(
  profileSource,
  /reservation\.status === "expired" \|\| reservation\.status === "cancelled"/u,
);
assert.match(profileSource, /CREATE TABLE IF NOT EXISTS reservation_match_aborts/u);
assert.match(profileSource, /reservation_match_aborted/u);
assert.match(gatewaySource, /materializeRoster\(claim\.entries, claimedPlayerCount, dispatch\.playlist\)/u);
assert.match(gatewaySource, /commitSequentially\([\s\S]*dispatch\.queueConfirmation\.reservations/u);
assert.match(gatewaySource, /stageQueueConfirmation\(/u);
assert.match(gatewaySource, /finishStagedDispatch\(/u);
assert.ok(
  gatewaySource.indexOf("await director.stageQueueConfirmation(") <
    gatewaySource.indexOf("await commitSequentially("),
  "the exact recovery journal must exist before any reservation commit",
);
assert.ok(
  gatewaySource.indexOf('"/prepare"') < gatewaySource.indexOf("await commitSequentially("),
  "the queue roster must be pinned before any reservation commit",
);
assert.match(directorSource, /existing\.dispatch_hash !== dispatchHash/u);
assert.match(directorSource, /existing\.roster_hash !== request\.rosterHash/u);
assert.match(directorSource, /async resumeDispatch\(/u);
assert.match(directorSource, /async stageQueueConfirmation\(/u);
assert.match(directorSource, /async beginAbort\(/u);
assert.match(directorSource, /status = 'aborting'/u);
assert.match(directorSource, /async recordExpiredConfirmedAbort\(/u);
assert.match(directorSource, /const activationCutoffMs = Date\.now\(\) \+ 5_000/u);
assert.match(directorSource, /SET status = 'active'/u);
assert.match(directorSource, /queue_confirmed_at_ms/u);
assert.match(matchmakingSource, /url\.pathname === "\/prepare"/u);
assert.match(matchmakingSource, /status IN \('leased', 'matched'\) AND lease_id = \?/u);
assert.match(matchmakingSource, /assignment_roster_mismatch/u);
assert.match(matchmakingSource, /Prepared assignments have expired/u);
assert.match(matchmakingSource, /assignment_json IS NULL/u);
assert.match(matchmakingSource, /url\.pathname === "\/abort-prepared"/u);
assert.match(matchmakingSource, /SET status = 'cancelled'/u);
assert.match(gatewaySource, /const postCommitAction = dispatchRecoveryAction\(/u);
assert.match(
  gatewaySource,
  /Math\.floor\(Date\.now\(\) \/ 1_000\) \+ 120/u,
  "join tickets must remain short-lived even for long capacity reservations",
);
assert.match(gatewaySource, /signJoinTicket\(/u);
assert.match(gatewaySource, /match_epoch: dispatch\.matchEpoch/u);
assert.match(joinTicketsSource, /alg: "EdDSA"/u);
assert.match(joinTicketsSource, /exactObjectKeys/u);
assert.match(joinTicketsSource, /JOIN_TICKET_LIFETIME_SECONDS = 120/u);
assert.match(joinTicketsSource, /publicKeySetJson/u);
assert.doesNotMatch(joinTicketsSource, /HMAC/u);
assert.match(gatewaySource, /"\/internal\/v1\/readiness"/u);
assert.match(gatewaySource, /serviceRequest\(request, env, \["deployment:verify"\]\)/u);
assert.match(readinessSource, /assertCryptographicReadiness/u);
for (const keyDomain of [
  "PLAYER_TICKET_KEYS_JSON",
  "JOIN_TICKET_SIGNING_KEYS_JSON",
  "SERVICE_TICKET_KEYS_JSON",
  "RESULT_TICKET_KEYS_JSON",
]) {
  assert.match(readinessSource, new RegExp(keyDomain, "u"));
}
assert.match(ticketsSource, /Join tickets must use the asymmetric Ed25519 signer/u);
assert.match(ticketsSource, /export async function verifyTicketWithKeyId/u);
assert.match(ticketsSource, /return \{ claims, keyId: header\.kid \}/u);
assert.match(settlementsSource, /verifyTicketWithKeyId\(resultTicket, env\.RESULT_TICKET_KEYS_JSON/u);
assert.match(settlementsSource, /audience: env\.RESULT_TICKET_AUDIENCE/u);
assert.match(settlementsSource, /authorizedResultSigner\(/u);
assert.match(ticketInvariantsSource, /hostId !== undefined && hostId === subject/u);
assert.match(
  settlementsSource,
  /status IN \('active', 'draining', 'complete'\)[\s\S]*queue_confirmed_at_ms IS NOT NULL/u,
);
assert.match(settlementsSource, /allocation\.host_id !== signerHostId/u);
assert.match(settlementsSource, /async function verifyResultEvidence/u);
assert.match(settlementsSource, /allocation\.playlist !== "casual-extraction"/u);
assert.match(settlementsSource, /await object\.arrayBuffer\(\)/u);
assert.match(settlementsSource, /sha256Base64Url\(bytes\)/u);
assert.match(settlementsSource, /result_evidence_metadata_mismatch/u);
assert.doesNotMatch(
  settlementsSource,
  /verifyTicket(?:WithKeyId)?\(resultTicket, env\.SERVICE_TICKET_KEYS_JSON/u,
);
const settlementSchema = await json("schemas/settlement-message.schema.json");
assert.deepEqual(settlementSchema.dependentRequired, {
  resultObjectKey: ["resultObjectSha256"],
  resultObjectSha256: ["resultObjectKey"],
});

const devVars = await readFile(join(root, ".dev.vars.example"), "utf8");
for (const line of devVars.split("\n").filter((line) => /^[A-Z_]+=/u.test(line))) {
  if (line.includes("_KEYS_JSON=")) assert.match(line, /REPLACE_WITH_/u);
}
assert.match(devVars, /^RESULT_TICKET_KEYS_JSON='\{"local-result-host-v1":/mu);
assert.doesNotMatch(
  `${await readFile(join(root, "wrangler.jsonc"), "utf8")}\n${devVars}`,
  /\b(?:CF_API_TOKEN|CLOUDFLARE_API_TOKEN|ACCOUNT_ID)=["']?[A-Za-z0-9_-]{8,}/u,
);

console.log("cloudflare scaffold: configuration, schemas, bindings, migrations and safety contracts valid");
