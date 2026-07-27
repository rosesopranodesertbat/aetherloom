import { signJoinTicket, verifyJoinTicket } from "./join-tickets.ts";
import { signTicket, verifyTicket } from "./tickets.ts";
import type { Env, TicketClaims, TicketPurpose } from "./types.ts";
import { requireIdentifier } from "./util.ts";

const READINESS_HEX = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

function keyIds(encoded: string, label: string): string[] {
  let value: unknown;
  try {
    value = JSON.parse(encoded);
  } catch {
    throw new Error(`${label} is not valid JSON.`);
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${label} must be a JSON object.`);
  }
  const ids = Object.keys(value as Record<string, unknown>).sort();
  if (ids.length < 1 || ids.length > 8) {
    throw new Error(`${label} must contain between one and eight keys.`);
  }
  return ids.map((keyId) => requireIdentifier(keyId, `${label} key id`));
}

async function verifyHmacDomain(
  env: Env,
  keySet: string,
  keyId: string,
  purpose: Exclude<TicketPurpose, "join">,
  audience: string,
  subject: string,
  scopes?: string[],
): Promise<void> {
  const now = Math.floor(Date.now() / 1_000);
  const claims: TicketClaims = {
    v: 1,
    iss: env.TICKET_ISSUER,
    aud: audience,
    purpose,
    sub: subject,
    iat: now,
    nbf: now - 1,
    exp: now + 30,
    nonce: READINESS_HEX,
    ...(scopes === undefined ? {} : { scopes }),
  };
  const token = await signTicket(claims, keySet, keyId);
  const verified = await verifyTicket(token, keySet, {
    issuer: env.TICKET_ISSUER,
    audience,
    purpose,
    nowSeconds: now,
    ...(scopes === undefined ? {} : { requiredScopes: scopes }),
  });
  if (verified.sub !== subject || verified.nonce !== READINESS_HEX) {
    throw new Error("HMAC readiness round trip changed signed claims.");
  }
}

function resultHost(env: Env, resultKeyId: string): string {
  let value: unknown;
  try {
    value = JSON.parse(env.RESULT_TICKET_HOSTS_JSON);
  } catch {
    throw new Error("RESULT_TICKET_HOSTS_JSON is not valid JSON.");
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("RESULT_TICKET_HOSTS_JSON must be a JSON object.");
  }
  return requireIdentifier(
    (value as Record<string, unknown>)[resultKeyId],
    "readiness result signer host",
  );
}

/**
 * Exercises every remote secret domain in memory. The caller receives only a
 * pass/fail result; private material and key identifiers never enter the
 * response or logs.
 */
export async function assertCryptographicReadiness(env: Env): Promise<void> {
  const playerKeyId = keyIds(
    env.PLAYER_TICKET_KEYS_JSON,
    "PLAYER_TICKET_KEYS_JSON",
  )[0]!;
  const serviceKeyIds = keyIds(
    env.SERVICE_TICKET_KEYS_JSON,
    "SERVICE_TICKET_KEYS_JSON",
  );
  if (!serviceKeyIds.includes(env.ACTIVE_SERVICE_TICKET_KID)) {
    throw new Error("ACTIVE_SERVICE_TICKET_KID is absent from the service secret.");
  }
  const resultKeyId = keyIds(
    env.RESULT_TICKET_KEYS_JSON,
    "RESULT_TICKET_KEYS_JSON",
  )[0]!;

  await verifyHmacDomain(
    env,
    env.PLAYER_TICKET_KEYS_JSON,
    playerKeyId,
    "session",
    env.PLAYER_TICKET_AUDIENCE,
    "deployment-readiness-player",
  );
  await verifyHmacDomain(
    env,
    env.SERVICE_TICKET_KEYS_JSON,
    env.ACTIVE_SERVICE_TICKET_KID,
    "service",
    env.SERVICE_TICKET_AUDIENCE,
    "deployment-readiness-service",
    ["deployment:verify"],
  );
  await verifyHmacDomain(
    env,
    env.RESULT_TICKET_KEYS_JSON,
    resultKeyId,
    "result",
    env.RESULT_TICKET_AUDIENCE,
    resultHost(env, resultKeyId),
  );

  const now = Math.floor(Date.now() / 1_000);
  const joinTicket = await signJoinTicket(
    {
      v: 1,
      iss: env.TICKET_ISSUER,
      aud: env.JOIN_TICKET_AUDIENCE,
      purpose: "join",
      sub: READINESS_HEX,
      iat: now,
      nbf: now - 1,
      exp: now + 30,
      nonce: READINESS_HEX,
      match_id: READINESS_HEX,
      match_epoch: 1,
      region: "readiness",
      build_hash: env.CONTENT_BUILD_HASH,
      input_pool: "controller",
      player_slot: 0,
      team_id: 0,
    },
    env.JOIN_TICKET_SIGNING_KEYS_JSON,
    env.ACTIVE_JOIN_TICKET_KID,
  );
  await verifyJoinTicket(joinTicket, env.JOIN_TICKET_PUBLIC_KEYS_JSON, {
    issuer: env.TICKET_ISSUER,
    audience: env.JOIN_TICKET_AUDIENCE,
    nowSeconds: now,
    matchId: READINESS_HEX,
    matchEpoch: 1,
    buildHash: env.CONTENT_BUILD_HASH,
    inputPool: "controller",
  });
}
