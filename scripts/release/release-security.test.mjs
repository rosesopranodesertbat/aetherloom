import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../../", import.meta.url);
const workflowNames = [
  "ci.yml",
  "control-plane-production.yml",
  "pages.yml",
  "staging.yml",
];

async function workflow(name) {
  return readFile(new URL(`.github/workflows/${name}`, root), "utf8");
}

test("every external GitHub Action is pinned to an immutable commit", async () => {
  for (const name of workflowNames) {
    const source = await workflow(name);
    const uses = [...source.matchAll(/^\s*(?:-\s+)?uses:\s+([^#\s]+)(?:\s+#.*)?$/gmu)]
      .map((match) => match[1]);
    assert.ok(uses.length > 0, `${name} should contain at least one action`);
    for (const action of uses) {
      assert.match(
        action,
        /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+@[0-9a-f]{40}$/u,
        `${name} contains a mutable action reference: ${action}`,
      );
    }
  }
});

test("Cloudflare credentials are never job-scoped", async () => {
  for (const name of ["staging.yml", "control-plane-production.yml"]) {
    const source = await workflow(name);
    assert.doesNotMatch(
      source,
      /^ {6}CLOUDFLARE_(?:API_TOKEN|ACCOUNT_ID):/mu,
      `${name} exposes Cloudflare credentials to an entire job`,
    );
  }
});

test("promotion binds staging evidence to the requested release SHA", async () => {
  for (const name of ["pages.yml", "control-plane-production.yml"]) {
    const source = await workflow(name);
    assert.match(
      source,
      /verify-staging-run\.mjs "\$\{STAGING_RUN_ID\}" "\$\{RELEASE_SHA\}"/u,
    );
  }
});

test("remote identity, secret, and recovery gates precede production migration", async () => {
  const production = await workflow("control-plane-production.yml");
  const migration = production.indexOf("Apply production D1 migrations");
  for (const gate of [
    "Verify remote production resources and secret names",
    "Capture production D1 recovery bookmark",
    "production-d1-recovery-${{ inputs.release_sha }}",
    "Require a fresh recoverable D1 point",
  ]) {
    const location = production.indexOf(gate);
    assert.ok(location >= 0 && location < migration, `${gate} must precede migration`);
  }
});

test("root Wrangler state is ignored", async () => {
  const ignores = await readFile(new URL(".gitignore", root), "utf8");
  assert.match(ignores, /^\.wrangler\/$/mu);
});

test("multiplayer smoke observes both sockets before either upgrade can stall", async () => {
  const source = await readFile(
    new URL("scripts/release/smoke-browser-multiplayer.mjs", root),
    "utf8",
  );
  const openStart = source.indexOf("function open(joined)");
  const observeStart = source.indexOf("const observer = observe(socket);", openStart);
  const openListener = source.indexOf('socket.addEventListener(\n      "open"', openStart);
  assert.ok(openStart >= 0 && observeStart > openStart && observeStart < openListener);
  assert.match(
    source,
    /resolve\(\{ socket, observer \}\);[\s\S]*?Promise\.all\(\[open\(firstJoin\), open\(secondJoin\)\]\)/u,
  );
});
