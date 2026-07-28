function fail(message) {
  throw new Error(message);
}

function baseUrl(value) {
  const url = new URL(value);
  const local = url.hostname === "localhost" || url.hostname === "127.0.0.1";
  if (url.protocol !== "https:" && !(local && url.protocol === "http:")) {
    fail("web smoke target must use HTTPS, except for localhost");
  }
  url.pathname = url.pathname.endsWith("/") ? url.pathname : `${url.pathname}/`;
  url.search = "";
  url.hash = "";
  return url;
}

async function fetchChecked(url) {
  const response = await fetch(url, {
    redirect: "follow",
    signal: AbortSignal.timeout(10_000),
    headers: { "user-agent": "aetherloom-release-smoke/1" },
  });
  if (!response.ok) {
    fail(`${url} returned HTTP ${response.status}`);
  }
  return response;
}

async function smoke(target) {
  const root = baseUrl(target);
  const index = await (await fetchChecked(new URL(".", root))).text();
  if (!index.includes("<title>Aetherloom") || !index.includes("sim.wasm")) {
    fail("deployed index does not look like the Aetherloom web client");
  }

  const gallery = await (await fetchChecked(new URL("models.html", root))).text();
  if (!gallery.includes("<title>Aetherloom Model Gallery</title>")) {
    fail("deployed model gallery is missing or unexpected");
  }

  const game = await (await fetchChecked(new URL("game.js", root))).text();
  if (!game.includes("sim.wasm")) {
    fail("deployed game bundle does not reference the simulation Wasm");
  }

  const wasm = new Uint8Array(await (await fetchChecked(new URL("sim.wasm", root))).arrayBuffer());
  if (
    wasm.byteLength < 8 ||
    wasm[0] !== 0x00 ||
    wasm[1] !== 0x61 ||
    wasm[2] !== 0x73 ||
    wasm[3] !== 0x6d
  ) {
    fail("deployed sim.wasm does not have a valid WebAssembly header");
  }
}

const target = process.argv[2];
if (!target) {
  fail("usage: smoke-web.mjs <base-url>");
}

let lastError;
for (let attempt = 1; attempt <= 12; attempt += 1) {
  try {
    await smoke(target);
    console.log(`web smoke check passed: ${target}`);
    process.exit(0);
  } catch (error) {
    lastError = error;
    if (attempt < 12) {
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 5_000));
    }
  }
}
throw lastError;
