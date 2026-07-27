const IDENTIFIER_RE = /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$/;
const IDEMPOTENCY_RE = /^[A-Za-z0-9][A-Za-z0-9_-]{15,127}$/;

export class ApiError extends Error {
  readonly status: number;
  readonly code: string;
  readonly details?: unknown;

  constructor(status: number, code: string, message: string, details?: unknown) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
    this.details = details;
  }
}

export function jsonResponse(value: unknown, status = 200, headers?: HeadersInit): Response {
  const out = new Headers(headers);
  out.set("content-type", "application/json; charset=utf-8");
  out.set("cache-control", "no-store");
  return new Response(JSON.stringify(value), { status, headers: out });
}

export function errorResponse(error: unknown, requestId: string): Response {
  if (error instanceof ApiError) {
    return jsonResponse(
      {
        error: {
          code: error.code,
          message: error.message,
          requestId,
          ...(error.details === undefined ? {} : { details: error.details }),
        },
      },
      error.status,
    );
  }

  console.error("unhandled request error", { requestId, error });
  return jsonResponse(
    {
      error: {
        code: "internal_error",
        message: "The request could not be completed.",
        requestId,
      },
    },
    500,
  );
}

export async function readJson<T>(request: Request | Response, maxBytes = 65_536): Promise<T> {
  const declaredLength = Number(request.headers.get("content-length") ?? "0");
  if (Number.isFinite(declaredLength) && declaredLength > maxBytes) {
    throw new ApiError(413, "body_too_large", `Request body exceeds ${maxBytes} bytes.`);
  }
  const body = await request.arrayBuffer();
  if (body.byteLength === 0) {
    throw new ApiError(400, "missing_body", "A JSON request body is required.");
  }
  if (body.byteLength > maxBytes) {
    throw new ApiError(413, "body_too_large", `Request body exceeds ${maxBytes} bytes.`);
  }
  try {
    return JSON.parse(new TextDecoder().decode(body)) as T;
  } catch {
    throw new ApiError(400, "invalid_json", "The request body is not valid JSON.");
  }
}

export function assertObject(value: unknown, label: string): asserts value is Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new ApiError(400, "invalid_request", `${label} must be an object.`);
  }
}

export function requireString(value: unknown, label: string, maxLength = 128): string {
  if (typeof value !== "string" || value.length === 0 || value.length > maxLength) {
    throw new ApiError(400, "invalid_request", `${label} must be a non-empty string.`);
  }
  return value;
}

export function requireIdentifier(value: unknown, label: string): string {
  const parsed = requireString(value, label);
  if (!IDENTIFIER_RE.test(parsed)) {
    throw new ApiError(400, "invalid_request", `${label} contains unsupported characters.`);
  }
  return parsed;
}

export function requireIdempotencyKey(value: unknown, label = "Idempotency-Key"): string {
  const parsed = requireString(value, label);
  if (!IDEMPOTENCY_RE.test(parsed)) {
    throw new ApiError(
      400,
      "invalid_idempotency_key",
      `${label} must be 16-128 characters using letters, digits, '_' or '-'.`,
    );
  }
  return parsed;
}

export function requireInteger(
  value: unknown,
  label: string,
  minimum: number,
  maximum: number,
): number {
  if (!Number.isSafeInteger(value) || (value as number) < minimum || (value as number) > maximum) {
    throw new ApiError(
      400,
      "invalid_request",
      `${label} must be an integer between ${minimum} and ${maximum}.`,
    );
  }
  return value as number;
}

export function parsePositiveConfigInt(
  value: string,
  label: string,
  minimum: number,
  maximum: number,
): number {
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < minimum || parsed > maximum) {
    throw new Error(`${label} must be an integer between ${minimum} and ${maximum}`);
  }
  return parsed;
}

export function canonicalJson(value: unknown): string {
  return JSON.stringify(sortJson(value));
}

function sortJson(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(sortJson);
  }
  if (typeof value === "object" && value !== null) {
    const record = value as Record<string, unknown>;
    return Object.fromEntries(
      Object.keys(record)
        .sort()
        .map((key) => [key, sortJson(record[key])]),
    );
  }
  return value;
}

export function bytesToBase64Url(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/u, "");
}

export function base64UrlToBytes(value: string): Uint8Array {
  if (!/^[A-Za-z0-9_-]+$/u.test(value)) {
    throw new ApiError(401, "invalid_ticket", "Ticket contains invalid base64url data.");
  }
  const padded = value.replaceAll("-", "+").replaceAll("_", "/") + "=".repeat((4 - (value.length % 4)) % 4);
  try {
    const binary = atob(padded);
    return Uint8Array.from(binary, (character) => character.charCodeAt(0));
  } catch {
    throw new ApiError(401, "invalid_ticket", "Ticket contains invalid base64url data.");
  }
}

export async function sha256Base64Url(value: string | ArrayBuffer): Promise<string> {
  const input = typeof value === "string" ? new TextEncoder().encode(value) : value;
  return bytesToBase64Url(new Uint8Array(await crypto.subtle.digest("SHA-256", input)));
}

export function randomToken(prefix: string): string {
  const bytes = crypto.getRandomValues(new Uint8Array(18));
  return `${prefix}_${bytesToBase64Url(bytes)}`;
}

export function inventoryStacks(value: unknown, label: string, maximumStacks = 64): Array<{ itemId: string; quantity: number }> {
  if (!Array.isArray(value) || value.length > maximumStacks) {
    throw new ApiError(400, "invalid_request", `${label} must contain at most ${maximumStacks} stacks.`);
  }
  const combined = new Map<string, number>();
  for (const [index, raw] of value.entries()) {
    assertObject(raw, `${label}[${index}]`);
    const itemId = requireIdentifier(raw.itemId, `${label}[${index}].itemId`);
    const quantity = requireInteger(raw.quantity, `${label}[${index}].quantity`, 1, 1_000_000);
    const next = (combined.get(itemId) ?? 0) + quantity;
    if (!Number.isSafeInteger(next) || next > 1_000_000) {
      throw new ApiError(400, "invalid_request", `${label} contains too many units of ${itemId}.`);
    }
    combined.set(itemId, next);
  }
  return [...combined.entries()]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([itemId, quantity]) => ({ itemId, quantity }));
}

export function requestId(request: Request): string {
  return request.headers.get("cf-ray") ?? crypto.randomUUID();
}

export async function mapInBatches<T, R>(
  values: readonly T[],
  batchSize: number,
  mapper: (value: T) => Promise<R>,
): Promise<R[]> {
  const output: R[] = [];
  for (let index = 0; index < values.length; index += batchSize) {
    const batch = values.slice(index, index + batchSize);
    output.push(...(await Promise.all(batch.map(mapper))));
  }
  return output;
}
