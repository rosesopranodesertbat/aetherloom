function fail(message) {
  throw new Error(message);
}

function healthUrl(value) {
  const url = new URL(value);
  const local = url.hostname === "localhost" || url.hostname === "127.0.0.1";
  if (url.protocol !== "https:" && !(local && url.protocol === "http:")) {
    fail("control-plane smoke target must use HTTPS, except for localhost");
  }
  url.pathname = `${url.pathname.replace(/\/+$/u, "")}/healthz`;
  url.search = "";
  url.hash = "";
  return url;
}

async function smoke(target, expectedEnvironment, expectedBuild) {
  const response = await fetch(healthUrl(target), {
    redirect: "error",
    signal: AbortSignal.timeout(10_000),
    headers: { "user-agent": "aetherloom-release-smoke/1" },
  });
  if (!response.ok) {
    fail(`control-plane health check returned HTTP ${response.status}`);
  }
  const type = response.headers.get("content-type") ?? "";
  if (!type.toLowerCase().includes("application/json")) {
    fail("control-plane health check did not return JSON");
  }
  const health = await response.json();
  if (
    health?.status !== "ok" ||
    health?.service !== "aetherloom-control-plane" ||
    health?.environment !== expectedEnvironment ||
    health?.authoritativeHz !== 128 ||
    health?.buildHash !== expectedBuild
  ) {
    fail("control-plane health payload does not match the promoted environment and build");
  }
}

const [target, expectedEnvironment, expectedBuild] = process.argv.slice(2);
if (!target || !["staging", "production"].includes(expectedEnvironment)) {
  fail("usage: smoke-control-plane.mjs <base-url> <staging|production> <build-hash>");
}
if (!/^[0-9a-f]{32}$/u.test(expectedBuild) || /^0{32}$/u.test(expectedBuild)) {
  fail("expected build must be a nonzero 128-bit lowercase hexadecimal content id");
}

let lastError;
for (let attempt = 1; attempt <= 12; attempt += 1) {
  try {
    await smoke(target, expectedEnvironment, expectedBuild);
    console.log(`control-plane smoke check passed: ${target}`);
    process.exit(0);
  } catch (error) {
    lastError = error;
    if (attempt < 12) {
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 5_000));
    }
  }
}
throw lastError;
