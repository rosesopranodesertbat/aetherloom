import { BrowserMatchRoom } from "./browser-match-room.ts";
import type {
  BrowserMatchClaimRequest,
  BrowserMatchEnv,
} from "./browser-match-types.ts";
import {
  BROWSER_MATCH_INPUT_HZ,
  BROWSER_MATCH_MAX_PLAYERS,
  BROWSER_MATCH_SNAPSHOT_HZ,
  BROWSER_MATCH_TICK_HZ,
} from "./browser-match-types.ts";
import {
  ApiError,
  assertObject,
  errorResponse,
  jsonResponse,
  readJson,
  requireHex128,
} from "./util.ts";

export { BrowserMatchRoom };

async function handleRequest(request: Request, env: BrowserMatchEnv): Promise<Response> {
  const url = new URL(request.url);
  if (request.method === "GET" && url.pathname === "/healthz") {
    return jsonResponse({
      status: "ok",
      service: "aetherloom-browser-match",
      environment: env.ENVIRONMENT,
      buildHash: env.CONTENT_BUILD_HASH,
      authoritativeHz: BROWSER_MATCH_TICK_HZ,
      inputHz: BROWSER_MATCH_INPUT_HZ,
      snapshotHz: BROWSER_MATCH_SNAPSHOT_HZ,
      maxPlayers: BROWSER_MATCH_MAX_PLAYERS,
      competitive: false,
    });
  }
  if (request.method === "POST" && url.pathname === "/v1/demo/claims") {
    const body = await readJson<BrowserMatchClaimRequest>(request, 2_048);
    assertObject(body, "browser match claim");
    const matchId = requireHex128(body.matchId, "matchId");
    return room(env, matchId).fetch(
      new Request("https://browser-match-room.internal/claim", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
      }),
    );
  }
  const websocketMatch = /^\/v1\/demo\/matches\/([0-9a-f]{32})\/ws$/u.exec(url.pathname);
  if (request.method === "GET" && websocketMatch?.[1] !== undefined) {
    const matchId = requireHex128(websocketMatch[1], "matchId");
    return room(env, matchId).fetch(
      new Request("https://browser-match-room.internal/ws", {
        method: "GET",
        headers: request.headers,
      }),
    );
  }
  throw new ApiError(404, "not_found", "Browser match route not found.");
}

function room(env: BrowserMatchEnv, matchId: string): DurableObjectStub {
  return env.BROWSER_MATCH_ROOMS.get(env.BROWSER_MATCH_ROOMS.idFromName(matchId));
}

export default {
  async fetch(request: Request, env: BrowserMatchEnv): Promise<Response> {
    try {
      return await handleRequest(request, env);
    } catch (error) {
      return errorResponse(error, request.headers.get("cf-ray") ?? crypto.randomUUID());
    }
  },
} satisfies ExportedHandler<BrowserMatchEnv>;
