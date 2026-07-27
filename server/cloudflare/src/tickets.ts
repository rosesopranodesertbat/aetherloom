import type { InputPool, Platform, TicketClaims, TicketPurpose } from "./types";
import {
  ApiError,
  assertObject,
  base64UrlToBytes,
  bytesToBase64Url,
  canonicalJson,
  requireIdentifier,
  requireInteger,
  requireString,
} from "./util";

interface TicketHeader {
  alg: "HS256";
  kid: string;
  typ: "AETHERLOOM-TICKET";
  v: 1;
}

export interface TicketExpectations {
  issuer: string;
  audience: string;
  purpose: TicketPurpose;
  nowSeconds?: number;
  clockSkewSeconds?: number;
  requiredScopes?: string[];
}

export interface VerifiedTicket {
  claims: TicketClaims;
  keyId: string;
}

const encoder = new TextEncoder();
const importedKeys = new Map<string, Promise<CryptoKey>>();
type HmacUsage = "sign" | "verify";

function parseKeySet(encoded: string): Record<string, string> {
  let value: unknown;
  try {
    value = JSON.parse(encoded);
  } catch {
    throw new Error("Ticket key set secret is not valid JSON.");
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("Ticket key set secret must be a JSON object.");
  }
  const entries = Object.entries(value as Record<string, unknown>);
  if (entries.length === 0 || entries.length > 8) {
    throw new Error("Ticket key set must contain between one and eight keys.");
  }
  const result: Record<string, string> = {};
  for (const [kid, secret] of entries) {
    requireIdentifier(kid, "ticket key id");
    if (typeof secret !== "string") throw new Error(`Ticket key ${kid} must be base64url text.`);
    const raw = base64UrlToBytes(secret);
    if (raw.byteLength < 32) throw new Error(`Ticket key ${kid} must contain at least 32 random bytes.`);
    result[kid] = secret;
  }
  return result;
}

function keyFor(keySetJson: string, kid: string, usages: HmacUsage[]): Promise<CryptoKey> {
  const keySet = parseKeySet(keySetJson);
  const encoded = keySet[kid];
  if (encoded === undefined) {
    throw new ApiError(401, "unknown_ticket_key", "Ticket key is not recognized.");
  }
  const cacheKey = `${kid}:${encoded}:${usages.join(",")}`;
  let imported = importedKeys.get(cacheKey);
  if (imported === undefined) {
    imported = crypto.subtle.importKey(
      "raw",
      base64UrlToBytes(encoded),
      { name: "HMAC", hash: "SHA-256" },
      false,
      usages,
    );
    importedKeys.set(cacheKey, imported);
  }
  return imported;
}

function decodePart(part: string): unknown {
  const bytes = base64UrlToBytes(part);
  try {
    return JSON.parse(new TextDecoder().decode(bytes)) as unknown;
  } catch {
    throw new ApiError(401, "invalid_ticket", "Ticket JSON is malformed.");
  }
}

function validateHeader(value: unknown): TicketHeader {
  assertObject(value, "ticket header");
  if (value.alg !== "HS256" || value.typ !== "AETHERLOOM-TICKET" || value.v !== 1) {
    throw new ApiError(401, "invalid_ticket", "Ticket header is unsupported.");
  }
  return {
    alg: "HS256",
    typ: "AETHERLOOM-TICKET",
    v: 1,
    kid: requireIdentifier(value.kid, "ticket header kid"),
  };
}

function validateClaims(value: unknown): TicketClaims {
  assertObject(value, "ticket claims");
  const purpose = value.purpose;
  if (!["session", "join", "service", "result"].includes(String(purpose))) {
    throw new ApiError(401, "invalid_ticket", "Ticket purpose is unsupported.");
  }
  const claims: TicketClaims = {
    v: 1,
    iss: requireString(value.iss, "ticket issuer", 128),
    aud: requireString(value.aud, "ticket audience", 128),
    purpose: purpose as TicketPurpose,
    sub: requireIdentifier(value.sub, "ticket subject"),
    iat: requireInteger(value.iat, "ticket issued-at", 0, 9_007_199_254_740_991),
    nbf: requireInteger(value.nbf, "ticket not-before", 0, 9_007_199_254_740_991),
    exp: requireInteger(value.exp, "ticket expiry", 0, 9_007_199_254_740_991),
    nonce: requireIdentifier(value.nonce, "ticket nonce"),
  };

  if (value.scopes !== undefined) {
    if (!Array.isArray(value.scopes) || value.scopes.length > 32) {
      throw new ApiError(401, "invalid_ticket", "Ticket scopes are malformed.");
    }
    claims.scopes = value.scopes.map((scope, index) =>
      requireIdentifier(scope, `ticket scope ${index}`),
    );
  }
  if (value.platform !== undefined) {
    if (!["web", "native", "console"].includes(String(value.platform))) {
      throw new ApiError(401, "invalid_ticket", "Ticket platform is unsupported.");
    }
    claims.platform = value.platform as Platform;
  }
  if (value.party !== undefined) {
    assertObject(value.party, "ticket party");
    if (!Array.isArray(value.party.members) || value.party.members.length < 1 || value.party.members.length > 4) {
      throw new ApiError(401, "invalid_ticket", "Ticket party members are malformed.");
    }
    const members = value.party.members.map((member, index) =>
      requireIdentifier(member, `ticket party member ${index}`),
    );
    if (new Set(members).size !== members.length || !members.includes(claims.sub)) {
      throw new ApiError(401, "invalid_ticket", "Ticket party membership is inconsistent.");
    }
    claims.party = {
      party_id: requireIdentifier(value.party.party_id, "ticket party id"),
      members,
      input_pool: (() => {
        if (!["browser", "mouse-keyboard", "controller", "mixed"].includes(String(value.party.input_pool))) {
          throw new ApiError(401, "invalid_ticket", "Ticket party input pool is unsupported.");
        }
        return value.party.input_pool as NonNullable<TicketClaims["party"]>["input_pool"];
      })(),
    };
  }

  for (const [wireKey, claimKey] of [
    ["match_id", "match_id"],
    ["region", "region"],
    ["build_hash", "build_hash"],
    ["settlement_id", "settlement_id"],
    ["result_hash", "result_hash"],
  ] as const) {
    if (value[wireKey] !== undefined) {
      claims[claimKey] = requireIdentifier(value[wireKey], `ticket ${wireKey}`);
    }
  }
  if (value.input_pool !== undefined) {
    if (!["browser", "mouse-keyboard", "controller", "mixed"].includes(String(value.input_pool))) {
      throw new ApiError(401, "invalid_ticket", "Ticket input pool is unsupported.");
    }
    claims.input_pool = value.input_pool as InputPool;
  }
  if (value.player_slot !== undefined) {
    claims.player_slot = requireInteger(value.player_slot, "ticket player_slot", 0, 127);
  }
  if (value.team_id !== undefined) {
    claims.team_id = requireInteger(value.team_id, "ticket team_id", 0, 127);
  }
  if (
    purpose === "join" &&
    (claims.match_id === undefined ||
      claims.build_hash === undefined ||
      claims.input_pool === undefined ||
      claims.player_slot === undefined ||
      claims.team_id === undefined)
  ) {
    throw new ApiError(
      401,
      "invalid_ticket",
      "Join tickets must bind the match, build, input pool, player slot, and team.",
    );
  }
  return claims;
}

export async function signTicket(
  claims: TicketClaims,
  keySetJson: string,
  kid: string,
): Promise<string> {
  validateClaims(claims);
  const header: TicketHeader = { alg: "HS256", kid, typ: "AETHERLOOM-TICKET", v: 1 };
  const encodedHeader = bytesToBase64Url(encoder.encode(canonicalJson(header)));
  const encodedClaims = bytesToBase64Url(encoder.encode(canonicalJson(claims)));
  const signingInput = `${encodedHeader}.${encodedClaims}`;
  const key = await keyFor(keySetJson, kid, ["sign"]);
  const signature = await crypto.subtle.sign("HMAC", key, encoder.encode(signingInput));
  return `${signingInput}.${bytesToBase64Url(new Uint8Array(signature))}`;
}

export async function verifyTicket(
  token: string,
  keySetJson: string,
  expectations: TicketExpectations,
): Promise<TicketClaims> {
  return (await verifyTicketWithKeyId(token, keySetJson, expectations)).claims;
}

export async function verifyTicketWithKeyId(
  token: string,
  keySetJson: string,
  expectations: TicketExpectations,
): Promise<VerifiedTicket> {
  if (token.length === 0 || token.length > 4_096) {
    throw new ApiError(401, "invalid_ticket", "Ticket length is invalid.");
  }
  const parts = token.split(".");
  if (parts.length !== 3) {
    throw new ApiError(401, "invalid_ticket", "Ticket must contain three compact parts.");
  }
  const [headerPart, claimsPart, signaturePart] = parts as [string, string, string];
  const header = validateHeader(decodePart(headerPart));
  const key = await keyFor(keySetJson, header.kid, ["verify"]);
  const verified = await crypto.subtle.verify(
    "HMAC",
    key,
    base64UrlToBytes(signaturePart),
    encoder.encode(`${headerPart}.${claimsPart}`),
  );
  if (!verified) throw new ApiError(401, "invalid_ticket_signature", "Ticket signature is invalid.");

  const claims = validateClaims(decodePart(claimsPart));
  const now = expectations.nowSeconds ?? Math.floor(Date.now() / 1_000);
  const skew = expectations.clockSkewSeconds ?? 5;
  if (claims.iss !== expectations.issuer || claims.aud !== expectations.audience) {
    throw new ApiError(401, "invalid_ticket_claims", "Ticket issuer or audience is invalid.");
  }
  if (claims.purpose !== expectations.purpose) {
    throw new ApiError(403, "wrong_ticket_purpose", "Ticket is not valid for this operation.");
  }
  if (claims.iat > now + skew || claims.nbf > now + skew || claims.exp < now - skew || claims.exp <= claims.iat) {
    throw new ApiError(401, "expired_ticket", "Ticket is not currently valid.");
  }
  if (claims.exp - claims.iat > 86_400) {
    throw new ApiError(401, "invalid_ticket_lifetime", "Ticket lifetime is too long.");
  }
  for (const required of expectations.requiredScopes ?? []) {
    if (!claims.scopes?.includes(required)) {
      throw new ApiError(403, "missing_scope", `Ticket is missing required scope ${required}.`);
    }
  }
  return { claims, keyId: header.kid };
}

export function ticketFromRequest(request: Request, allowWebSocketProtocol = false): string {
  const authorization = request.headers.get("authorization");
  if (authorization?.startsWith("Bearer ")) {
    return authorization.slice("Bearer ".length).trim();
  }
  if (allowWebSocketProtocol) {
    const protocols = request.headers
      .get("sec-websocket-protocol")
      ?.split(",")
      .map((protocol) => protocol.trim()) ?? [];
    const authProtocol = protocols.find((protocol) => protocol.startsWith("aetherloom.auth."));
    if (authProtocol !== undefined) {
      return authProtocol.slice("aetherloom.auth.".length);
    }
  }
  throw new ApiError(401, "missing_ticket", "A signed bearer ticket is required.");
}

export function publicWebSocketProtocol(request: Request): string {
  const protocols = request.headers
    .get("sec-websocket-protocol")
    ?.split(",")
    .map((protocol) => protocol.trim()) ?? [];
  if (!protocols.includes("aetherloom.v1")) {
    throw new ApiError(400, "missing_websocket_protocol", "WebSocket protocol aetherloom.v1 is required.");
  }
  return "aetherloom.v1";
}
