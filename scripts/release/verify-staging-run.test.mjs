import assert from "node:assert/strict";
import test from "node:test";

import { verifyStagingRun } from "./verify-staging-run.mjs";

const SHA = "1111111111111111111111111111111111111111";
const REPOSITORY = "example/aetherloom";

function mockFetch(overrides = {}) {
  const repository = {
    default_branch: "main",
    ...overrides.repository,
  };
  const workflow = {
    id: 42,
    path: ".github/workflows/staging.yml",
    state: "active",
    ...overrides.workflow,
  };
  const run = {
    workflow_id: 42,
    event: "workflow_dispatch",
    status: "completed",
    conclusion: "success",
    path: ".github/workflows/staging.yml@refs/heads/main",
    head_branch: "main",
    head_sha: SHA,
    repository: { full_name: REPOSITORY },
    head_repository: { full_name: REPOSITORY },
    inputs: { source_ref: SHA },
    ...overrides.run,
  };
  return async (url) => {
    const value = url.endsWith("/actions/runs/123")
      ? run
      : url.endsWith("/actions/workflows/staging.yml")
        ? workflow
        : repository;
    return new Response(JSON.stringify(value), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };
}

test("accepts only the requested SHA from the active protected main workflow", async () => {
  assert.deepEqual(
    await verifyStagingRun({
      runId: "123",
      repository: REPOSITORY,
      token: "test-token",
      expectedSha: SHA,
      fetchImpl: mockFetch(),
    }),
    { runId: "123", expectedSha: SHA, workflowId: 42 },
  );
});

for (const [label, overrides] of [
  ["different release SHA", { run: { head_sha: "2".repeat(40) } }],
  ["non-main branch", { run: { head_branch: "feature" } }],
  ["fork", { run: { head_repository: { full_name: "attacker/aetherloom" } } }],
  ["different workflow id", { run: { workflow_id: 99 } }],
  ["inactive protected workflow", { workflow: { state: "disabled_manually" } }],
  ["untrusted source input", { run: { inputs: { source_ref: "feature" } } }],
]) {
  test(`rejects a staging run with ${label}`, async () => {
    await assert.rejects(
      verifyStagingRun({
        runId: "123",
        repository: REPOSITORY,
        token: "test-token",
        expectedSha: SHA,
        fetchImpl: mockFetch(overrides),
      }),
      /protected main workflow|source_ref/u,
    );
  });
}
