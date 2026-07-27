import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

function fail(message) {
  throw new Error(message);
}

const STAGING_WORKFLOW_PATH = ".github/workflows/staging.yml";
const PROTECTED_BRANCH = "main";

async function githubJson(url, token, fetchImpl) {
  const response = await fetchImpl(url, {
    signal: AbortSignal.timeout(10_000),
    headers: {
      accept: "application/vnd.github+json",
      authorization: `Bearer ${token}`,
      "user-agent": "aetherloom-release-verifier/1",
      "x-github-api-version": "2022-11-28",
    },
  });
  if (!response.ok) {
    fail(`could not verify staging workflow run: GitHub returned HTTP ${response.status}`);
  }
  return response.json();
}

export async function verifyStagingRun({
  runId,
  repository,
  token,
  expectedSha,
  fetchImpl = fetch,
}) {
  if (!/^[1-9][0-9]*$/u.test(runId ?? "")) {
    fail("staging run id must be a positive integer");
  }
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/u.test(repository ?? "")) {
    fail("GITHUB_REPOSITORY is missing or invalid");
  }
  if (!token) {
    fail("GITHUB_TOKEN is required to verify the staging run");
  }
  if (!/^[0-9a-f]{40}$/u.test(expectedSha ?? "")) {
    fail("expected release SHA must be a full lowercase 40-character Git SHA");
  }
  if (typeof fetchImpl !== "function") {
    fail("a fetch implementation is required");
  }

  const base = `https://api.github.com/repos/${repository}`;
  const [repositoryMetadata, protectedWorkflow, run] = await Promise.all([
    githubJson(base, token, fetchImpl),
    githubJson(
      `${base}/actions/workflows/${encodeURIComponent("staging.yml")}`,
      token,
      fetchImpl,
    ),
    githubJson(`${base}/actions/runs/${runId}`, token, fetchImpl),
  ]);
  const workflowPath = String(run.path ?? "").split("@", 1)[0];
  if (
    repositoryMetadata.default_branch !== PROTECTED_BRANCH ||
    protectedWorkflow.path !== STAGING_WORKFLOW_PATH ||
    protectedWorkflow.state !== "active" ||
    run.workflow_id !== protectedWorkflow.id ||
    run.event !== "workflow_dispatch" ||
    run.status !== "completed" ||
    run.conclusion !== "success" ||
    workflowPath !== STAGING_WORKFLOW_PATH ||
    run.head_branch !== PROTECTED_BRANCH ||
    run.head_sha !== expectedSha ||
    run.repository?.full_name !== repository ||
    run.head_repository?.full_name !== repository
  ) {
    fail(
      "the supplied run is not a successful staging deployment of the requested SHA from the protected main workflow",
    );
  }
  if (
    run.inputs?.source_ref !== undefined &&
    ![PROTECTED_BRANCH, expectedSha].includes(run.inputs.source_ref)
  ) {
    fail("the staging run source_ref does not match the requested release");
  }
  return {
    runId,
    expectedSha,
    workflowId: run.workflow_id,
  };
}

async function main() {
  const runId = process.argv[2];
  const expectedSha = process.argv[3];
  await verifyStagingRun({
    runId,
    expectedSha,
    repository: process.env.GITHUB_REPOSITORY,
    token: process.env.GITHUB_TOKEN,
  });
  console.log(`verified staging deployment run ${runId} for ${expectedSha}`);
}

const invoked = process.argv[1] === undefined
  ? undefined
  : pathToFileURL(resolve(process.argv[1])).href;
if (invoked === import.meta.url) {
  main().catch((error) => {
    console.error(`Staging run verification failed: ${error.message}`);
    process.exitCode = 1;
  });
}
