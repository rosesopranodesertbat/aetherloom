import { readFile, writeFile } from "node:fs/promises";
import { basename, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function fail(message) {
  throw new Error(message);
}

function option(name) {
  const index = process.argv.indexOf(name);
  if (index < 0 || index + 1 >= process.argv.length) fail(`missing ${name}`);
  return process.argv[index + 1];
}

function identifier(value, label) {
  if (
    typeof value !== "string" ||
    !/^[A-Za-z0-9][A-Za-z0-9_.-]{1,127}$/u.test(value) ||
    value.includes("REPLACE_")
  ) {
    fail(`${label} is not a valid concrete identifier`);
  }
  return value;
}

const environment = option("--environment");
if (environment !== "staging") {
  fail("the browser multiplayer systems-test Worker is staging-only");
}
const buildHash = option("--build-hash");
if (!/^[0-9a-f]{32}$/u.test(buildHash) || /^0{32}$/u.test(buildHash)) {
  fail("--build-hash must be a nonzero 128-bit lowercase hexadecimal content id");
}
const output = resolve(option("--output"));
if (
  dirname(output) !== root ||
  basename(output) !== "wrangler.generated.browser-match.staging.jsonc"
) {
  fail("output must be server/cloudflare/wrangler.generated.browser-match.staging.jsonc");
}
const resources = JSON.parse(await readFile(resolve(option("--resources")), "utf8"));
if (resources.version !== 1 || resources.environment !== environment) {
  fail("deployment resource manifest must be version 1 for staging");
}
const workerPrefix = identifier(resources.browserMatchWorker, "browser-match Worker prefix");
const workerName = identifier(
  `${workerPrefix}-${buildHash.slice(0, 12)}`,
  "versioned browser-match Worker",
);
let joinTicketPublicKeys = resources.joinTicketPublicKeys;
if (process.env.JOIN_TICKET_PUBLIC_KEYS_JSON) {
  try {
    joinTicketPublicKeys = JSON.parse(process.env.JOIN_TICKET_PUBLIC_KEYS_JSON);
  } catch {
    fail("JOIN_TICKET_PUBLIC_KEYS_JSON must contain valid JSON");
  }
}
if (
  joinTicketPublicKeys === null ||
  typeof joinTicketPublicKeys !== "object" ||
  Array.isArray(joinTicketPublicKeys) ||
  Object.keys(joinTicketPublicKeys).length < 1
) {
  fail("deployment manifest needs at least one join-ticket public key");
}
const activeJoinTicketKid =
  process.env.ACTIVE_JOIN_TICKET_KID || resources.activeJoinTicketKid;
if (
  typeof activeJoinTicketKid !== "string" ||
  !Object.hasOwn(joinTicketPublicKeys, activeJoinTicketKid)
) {
  fail("ACTIVE_JOIN_TICKET_KID is absent from the browser-match verifier set");
}

const config = {
  $schema: "./node_modules/wrangler/config-schema.json",
  name: workerName,
  main: "src/browser-match-worker.ts",
  compatibility_date: "2026-07-27",
  workers_dev: false,
  minify: true,
  observability: { enabled: true },
  vars: {
    ENVIRONMENT: "staging",
    TICKET_ISSUER: "aetherloom-control-plane",
    JOIN_TICKET_AUDIENCE: "aetherloom-match",
    JOIN_TICKET_PUBLIC_KEYS_JSON: JSON.stringify(joinTicketPublicKeys),
    CONTENT_BUILD_HASH: buildHash,
  },
  durable_objects: {
    bindings: [
      {
        name: "BROWSER_MATCH_ROOMS",
        class_name: "BrowserMatchRoom",
      },
    ],
  },
  exports: {
    BrowserMatchRoom: {
      type: "durable-object",
      storage: "sqlite",
    },
  },
  rules: [
    {
      type: "CompiledWasm",
      globs: ["**/*.wasm"],
      fallthrough: true,
    },
  ],
};

await writeFile(output, `${JSON.stringify(config, null, 2)}\n`, {
  flag: "w",
  mode: 0o600,
});
console.log(`rendered staging browser-match config for build ${buildHash}`);
