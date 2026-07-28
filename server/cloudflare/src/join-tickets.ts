import type { InputPool, JoinTicketClaims } from "./types.ts";
import {
  ApiError,
  assertObject,
  base64UrlToBytes,
  bytesToBase64Url,
  canonicalJson,
  requireHex128,
  requireIdentifier,
  requireInteger,
} from "./util.ts";

interface JoinTicketHeader {
  alg: "EdDSA";
  kid: string;
  typ: "AETHERLOOM-JOIN";
  v: 1;
}

export interface JoinTicketExpectations {
  issuer: string;
  audience: string;
  nowSeconds?: number;
  clockSkewSeconds?: number;
  matchId?: string;
  matchEpoch?: number;
  buildHash?: string;
  inputPool?: InputPool;
}

const JOIN_TICKET_LIFETIME_SECONDS = 120;
const MAX_JOIN_TICKET_BYTES = 4_096;
const EXPECTED_HEADER_KEYS = ["alg", "kid", "typ", "v"] as const;
const EXPECTED_CLAIM_KEYS = [
  "aud",
  "build_hash",
  "exp",
  "iat",
  "input_pool",
  "iss",
  "match_epoch",
  "match_id",
  "nbf",
  "nonce",
  "player_slot",
  "purpose",
  "region",
  "sub",
  "team_id",
  "v",
] as const;

const encoder = new TextEncoder();
const signingKeys = new Map<string, Promise<CryptoKey>>();
const verificationKeys = new Map<string, Promise<CryptoKey>>();

function exactObjectKeys(
  value: Record<string, unknown>,
  expected: readonly string[],
  label: string,
): void {
  const actual = Object.keys(value).sort();
  const sortedExpected = [...expected].sort();
  if (
    actual.length !== sortedExpected.length ||
    actual.some((key, index) => key !== sortedExpected[index])
  ) {
    throw new ApiError(401, "invalid_join_ticket", `${label} does not use the exact v1 fields.`);
  }
}

function decodeCanonicalBase64Url(value: string): Uint8Array {
  const bytes = base64UrlToBytes(value);
  if (bytesToBase64Url(bytes) !== value) {
    throw new ApiError(401, "invalid_join_ticket", "Join ticket base64url is not canonical.");
  }
  return bytes;
}

function decodePart(part: string): unknown {
  try {
    const text = new TextDecoder().decode(decodeCanonicalBase64Url(part));
    const value = JSON.parse(text) as unknown;
    if (canonicalJson(value) !== text) {
      throw new ApiError(401, "invalid_join_ticket", "Join ticket JSON is not canonical.");
    }
    return value;
  } catch (error) {
    if (error instanceof ApiError) throw error;
    throw new ApiError(401, "invalid_join_ticket", "Join ticket JSON is malformed.");
  }
}

function validateHeader(value: unknown): JoinTicketHeader {
  assertObject(value, "join ticket header");
  exactObjectKeys(value, EXPECTED_HEADER_KEYS, "Join ticket header");
  if (value.alg !== "EdDSA" || value.typ !== "AETHERLOOM-JOIN" || value.v !== 1) {
    throw new ApiError(401, "invalid_join_ticket", "Join ticket header is unsupported.");
  }
  return {
    alg: "EdDSA",
    kid: requireIdentifier(value.kid, "join ticket key id"),
    typ: "AETHERLOOM-JOIN",
    v: 1,
  };
}

function validateClaims(value: unknown): JoinTicketClaims {
  assertObject(value, "join ticket claims");
  exactObjectKeys(value, EXPECTED_CLAIM_KEYS, "Join ticket claims");
  if (value.v !== 1 || value.purpose !== "join") {
    throw new ApiError(401, "invalid_join_ticket", "Join ticket claims are unsupported.");
  }
  const inputPool = value.input_pool;
  if (!["browser", "mouse-keyboard", "controller", "mixed"].includes(String(inputPool))) {
    throw new ApiError(401, "invalid_join_ticket", "Join ticket input pool is unsupported.");
  }
  return {
    v: 1,
    iss: requireIdentifier(value.iss, "join ticket issuer"),
    aud: requireIdentifier(value.aud, "join ticket audience"),
    purpose: "join",
    sub: requireHex128(value.sub, "join ticket subject"),
    iat: requireInteger(value.iat, "join ticket issued-at", 0, Number.MAX_SAFE_INTEGER),
    nbf: requireInteger(value.nbf, "join ticket not-before", 0, Number.MAX_SAFE_INTEGER),
    exp: requireInteger(value.exp, "join ticket expiry", 0, Number.MAX_SAFE_INTEGER),
    nonce: requireHex128(value.nonce, "join ticket nonce"),
    match_id: requireHex128(value.match_id, "join ticket match_id"),
    match_epoch: requireInteger(
      value.match_epoch,
      "join ticket match_epoch",
      1,
      Number.MAX_SAFE_INTEGER,
    ),
    region: requireIdentifier(value.region, "join ticket region"),
    build_hash: requireHex128(value.build_hash, "join ticket build_hash"),
    input_pool: inputPool as InputPool,
    player_slot: requireInteger(value.player_slot, "join ticket player_slot", 0, 127),
    team_id: requireInteger(value.team_id, "join ticket team_id", 0, 127),
  };
}

function parseKeySet(encoded: string, kind: "private" | "public"): Record<string, string> {
  let value: unknown;
  try {
    value = JSON.parse(encoded);
  } catch {
    throw new Error(`Ed25519 join ${kind} key set is not valid JSON.`);
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`Ed25519 join ${kind} key set must be a JSON object.`);
  }
  const entries = Object.entries(value as Record<string, unknown>);
  if (entries.length < 1 || entries.length > 8) {
    throw new Error(`Ed25519 join ${kind} key set must contain between one and eight keys.`);
  }
  const result: Record<string, string> = {};
  for (const [kid, encodedKey] of entries) {
    requireIdentifier(kid, "Ed25519 join key id");
    if (typeof encodedKey !== "string") {
      throw new Error(`Ed25519 join ${kind} key ${kid} must be base64url text.`);
    }
    const bytes = decodeCanonicalBase64Url(encodedKey);
    if (kind === "public" && bytes.byteLength !== 32) {
      throw new Error(`Ed25519 join public key ${kid} must contain exactly 32 bytes.`);
    }
    if (kind === "private" && (bytes.byteLength < 48 || bytes.byteLength > 512)) {
      throw new Error(`Ed25519 join private key ${kid} must be PKCS#8 DER.`);
    }
    result[kid] = encodedKey;
  }
  return result;
}

function signingKey(keySetJson: string, kid: string): Promise<CryptoKey> {
  const encoded = parseKeySet(keySetJson, "private")[kid];
  if (encoded === undefined) {
    throw new ApiError(401, "unknown_join_ticket_key", "Join ticket signing key is not recognized.");
  }
  const cacheKey = `${kid}:${encoded}`;
  let imported = signingKeys.get(cacheKey);
  if (imported === undefined) {
    imported = crypto.subtle.importKey(
      "pkcs8",
      decodeCanonicalBase64Url(encoded),
      { name: "Ed25519" },
      false,
      ["sign"],
    );
    signingKeys.set(cacheKey, imported);
  }
  return imported;
}

function verificationKey(keySetJson: string, kid: string): Promise<CryptoKey> {
  const encoded = parseKeySet(keySetJson, "public")[kid];
  if (encoded === undefined) {
    throw new ApiError(401, "unknown_join_ticket_key", "Join ticket verification key is not recognized.");
  }
  const cacheKey = `${kid}:${encoded}`;
  let imported = verificationKeys.get(cacheKey);
  if (imported === undefined) {
    imported = crypto.subtle.importKey(
      "raw",
      decodeCanonicalBase64Url(encoded),
      { name: "Ed25519" },
      false,
      ["verify"],
    );
    verificationKeys.set(cacheKey, imported);
  }
  return imported;
}

function validateTemporalClaims(
  claims: JoinTicketClaims,
  now: number,
  clockSkewSeconds: number,
): void {
  if (
    claims.iat > now + clockSkewSeconds ||
    claims.nbf > now + clockSkewSeconds ||
    claims.exp <= now ||
    claims.exp <= claims.iat ||
    claims.nbf >= claims.exp
  ) {
    throw new ApiError(401, "expired_join_ticket", "Join ticket is not currently valid.");
  }
  if (claims.exp - claims.iat > JOIN_TICKET_LIFETIME_SECONDS) {
    throw new ApiError(401, "invalid_join_ticket_lifetime", "Join ticket lifetime is too long.");
  }
}

export async function signJoinTicket(
  claims: JoinTicketClaims,
  privateKeySetJson: string,
  kid: string,
): Promise<string> {
  const validated = validateClaims(claims);
  const header: JoinTicketHeader = {
    alg: "EdDSA",
    kid: requireIdentifier(kid, "join ticket key id"),
    typ: "AETHERLOOM-JOIN",
    v: 1,
  };
  const headerPart = bytesToBase64Url(encoder.encode(canonicalJson(header)));
  const claimsPart = bytesToBase64Url(encoder.encode(canonicalJson(validated)));
  const signingInput = `${headerPart}.${claimsPart}`;
  const signature = await crypto.subtle.sign(
    "Ed25519",
    await signingKey(privateKeySetJson, kid),
    encoder.encode(signingInput),
  );
  return `${signingInput}.${bytesToBase64Url(new Uint8Array(signature))}`;
}

export async function verifyJoinTicket(
  token: string,
  publicKeySetJson: string,
  expectations: JoinTicketExpectations,
): Promise<JoinTicketClaims> {
  if (token.length < 1 || token.length > MAX_JOIN_TICKET_BYTES) {
    throw new ApiError(401, "invalid_join_ticket", "Join ticket length is invalid.");
  }
  const parts = token.split(".");
  if (parts.length !== 3) {
    throw new ApiError(401, "invalid_join_ticket", "Join ticket must contain three compact parts.");
  }
  const [headerPart, claimsPart, signaturePart] = parts as [string, string, string];
  const header = validateHeader(decodePart(headerPart));
  const signature = decodeCanonicalBase64Url(signaturePart);
  if (signature.byteLength !== 64) {
    throw new ApiError(401, "invalid_join_ticket", "Join ticket signature length is invalid.");
  }
  const validSignature = await crypto.subtle.verify(
    "Ed25519",
    await verificationKey(publicKeySetJson, header.kid),
    signature,
    encoder.encode(`${headerPart}.${claimsPart}`),
  );
  if (!validSignature) {
    throw new ApiError(401, "invalid_join_ticket_signature", "Join ticket signature is invalid.");
  }

  const claims = validateClaims(decodePart(claimsPart));
  if (claims.iss !== expectations.issuer || claims.aud !== expectations.audience) {
    throw new ApiError(401, "invalid_join_ticket_claims", "Join ticket issuer or audience is invalid.");
  }
  validateTemporalClaims(
    claims,
    expectations.nowSeconds ?? Math.floor(Date.now() / 1_000),
    expectations.clockSkewSeconds ?? 5,
  );
  if (
    (expectations.matchId !== undefined && claims.match_id !== expectations.matchId) ||
    (expectations.matchEpoch !== undefined && claims.match_epoch !== expectations.matchEpoch) ||
    (expectations.buildHash !== undefined && claims.build_hash !== expectations.buildHash) ||
    (expectations.inputPool !== undefined && claims.input_pool !== expectations.inputPool)
  ) {
    throw new ApiError(403, "join_ticket_mismatch", "Join ticket does not match the target match.");
  }
  return claims;
}
