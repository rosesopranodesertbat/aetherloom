import type {
  Env,
  QueuedSettlement,
  SettlementCommand,
  SignedSettlementEnvelope,
} from "./types";
import { verifyTicketWithKeyId } from "./tickets";
import {
  authorizedResultSigner,
  resultEvidenceObjectKey,
} from "./ticket-invariants";
import {
  ApiError,
  assertObject,
  canonicalJson,
  inventoryStacks,
  jsonResponse,
  readJson,
  requireIdentifier,
  requireInteger,
  requireString,
  sha256Base64Url,
} from "./util";

interface SettlementReceipt {
  settlementId: string;
  reservationId: string;
  matchId: string;
  outcome: string;
  lootBanked: Array<{ itemId: string; quantity: number }>;
  rating: number;
  profileVersion: number;
  appliedAtMs: number;
}

interface AllocationSignerRow {
  host_id: string;
  playlist: string;
  build_hash: string;
}

export async function acceptSignedSettlement(request: Request, env: Env): Promise<Response> {
  const envelope = await readJson<SignedSettlementEnvelope>(request, 128 * 1024);
  assertObject(envelope, "settlement envelope");
  const result = validateSettlement(envelope.result, true);
  const resultTicket = requireString(envelope.resultTicket, "resultTicket", 4_096);
  const resultHash = await sha256Base64Url(canonicalJson(result));
  const verified = await verifyTicketWithKeyId(resultTicket, env.RESULT_TICKET_KEYS_JSON, {
    issuer: env.TICKET_ISSUER,
    audience: env.RESULT_TICKET_AUDIENCE,
    purpose: "result",
    requiredScopes: ["result:submit"],
  });
  const { claims, keyId } = verified;
  const signerHostId = authorizedResultSigner(
    env.RESULT_TICKET_HOSTS_JSON,
    keyId,
    claims.sub,
  );
  if (signerHostId === null) {
    throw new ApiError(403, "result_signer_mismatch", "Result signer is not authorized for this key.");
  }
  if (
    claims.result_hash !== resultHash ||
    claims.settlement_id !== result.settlementId ||
    claims.match_id !== result.matchId
  ) {
    throw new ApiError(403, "result_signature_mismatch", "Signed result does not match its envelope.");
  }
  const allocation = await env.CONTROL_DB.prepare(
    `SELECT host_id, playlist, build_hash FROM match_allocations
     WHERE match_id = ? AND status IN ('active', 'draining', 'complete')
       AND queue_confirmed_at_ms IS NOT NULL`,
  )
    .bind(result.matchId)
    .first<AllocationSignerRow>();
  if (allocation === null || allocation.host_id !== signerHostId) {
    throw new ApiError(403, "result_signer_mismatch", "Result signer does not own this match allocation.");
  }
  await verifyResultEvidence(env, result, resultHash, signerHostId, allocation);
  const queued: QueuedSettlement = { ...result, payloadHash: resultHash };
  await env.SETTLEMENT_QUEUE.send(queued);
  return jsonResponse(
    {
      settlementId: result.settlementId,
      payloadHash: resultHash,
      status: "queued",
    },
    202,
  );
}

async function verifyResultEvidence(
  env: Env,
  result: SettlementCommand,
  resultHash: string,
  signerHostId: string,
  allocation: AllocationSignerRow,
): Promise<void> {
  const competitive = allocation.playlist !== "casual-extraction";
  if (result.resultObjectKey === undefined || result.resultObjectSha256 === undefined) {
    if (competitive) {
      throw new ApiError(
        400,
        "result_evidence_required",
        "Competitive settlements require content-addressed result evidence.",
      );
    }
    return;
  }
  const expectedKey = resultEvidenceObjectKey(
    result.matchId,
    result.settlementId,
    result.resultObjectSha256,
  );
  if (result.resultObjectKey !== expectedKey) {
    throw new ApiError(
      400,
      "result_evidence_key_mismatch",
      "Result evidence key is not content-addressed for this settlement.",
    );
  }
  const object = await env.MATCH_ARCHIVE.get(result.resultObjectKey);
  if (object === null) {
    throw new ApiError(409, "result_evidence_missing", "Signed result evidence was not found.");
  }
  if (object.size < 1 || object.size > 8 * 1024 * 1024) {
    throw new ApiError(400, "result_evidence_size", "Result evidence must be between 1 byte and 8 MiB.");
  }
  const bytes = await object.arrayBuffer();
  if ((await sha256Base64Url(bytes)) !== result.resultObjectSha256) {
    throw new ApiError(403, "result_evidence_hash_mismatch", "Result evidence bytes do not match the signed hash.");
  }
  const metadata = object.customMetadata ?? {};
  for (const [field, expected] of Object.entries({
    matchId: result.matchId,
    settlementId: result.settlementId,
    resultHash,
    hostId: signerHostId,
    buildHash: allocation.build_hash,
    sha256: result.resultObjectSha256,
  })) {
    if (metadata[field] !== expected) {
      throw new ApiError(
        403,
        "result_evidence_metadata_mismatch",
        `Result evidence metadata does not bind ${field}.`,
      );
    }
  }
}

export async function consumeSettlements(
  batch: MessageBatch<QueuedSettlement>,
  env: Env,
): Promise<void> {
  for (const message of batch.messages) {
    try {
      await applyQueuedSettlement(message.body, env);
      message.ack();
    } catch (error) {
      console.error("settlement delivery failed", {
        messageId: message.id,
        attempts: message.attempts,
        error,
      });
      message.retry();
    }
  }
}

async function applyQueuedSettlement(raw: QueuedSettlement, env: Env): Promise<void> {
  const result = validateSettlement(raw, false);
  const expectedHash = await sha256Base64Url(canonicalJson(result));
  if (raw.payloadHash !== expectedHash) {
    throw new Error(`Settlement ${result.settlementId} payload hash does not match its body.`);
  }

  const profileId = env.PROFILE_ISLAND.idFromName(result.accountId);
  const profile = env.PROFILE_ISLAND.get(profileId);
  const response = await profile.fetch(
    new Request("https://profile.internal/settlements", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ ...result, payloadHash: expectedHash }),
    }),
  );
  if (!response.ok) {
    const detail = await response.text();
    throw new Error(`Profile settlement failed (${response.status}): ${detail.slice(0, 512)}`);
  }
  const receipt = await readJson<SettlementReceipt>(response);

  const archiveObjectKey = `settlement.${result.matchId}.${result.settlementId}.json`;
  const archiveBody = canonicalJson({
    v: 1,
    payloadHash: expectedHash,
    result,
    receipt,
  });
  await env.MATCH_ARCHIVE.put(archiveObjectKey, archiveBody, {
    httpMetadata: { contentType: "application/json" },
    customMetadata: {
      matchId: result.matchId,
      settlementId: result.settlementId,
      payloadHash: expectedHash,
    },
  });

  const existing = await env.CONTROL_DB.prepare(
    "SELECT payload_hash FROM settlement_projection WHERE settlement_id = ?",
  )
    .bind(result.settlementId)
    .first<{ payload_hash: string }>();
  if (existing !== null && existing.payload_hash !== expectedHash) {
    throw new Error(`D1 settlement ${result.settlementId} has a conflicting payload hash.`);
  }
  await env.CONTROL_DB.batch([
    env.CONTROL_DB.prepare(
      `INSERT INTO settlement_projection(
         settlement_id, payload_hash, account_id, reservation_id, match_id,
         outcome, rating_after, applied_at_ms, archive_object_key
       ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?)
       ON CONFLICT(settlement_id) DO NOTHING`,
    ).bind(
      result.settlementId,
      expectedHash,
      result.accountId,
      result.reservationId,
      result.matchId,
      result.outcome,
      receipt.rating,
      receipt.appliedAtMs,
      archiveObjectKey,
    ),
    env.CONTROL_DB.prepare(
      `INSERT INTO profile_projection(account_id, rating, profile_version, island_version, updated_at_ms)
       VALUES(?, ?, ?, 0, ?)
       ON CONFLICT(account_id) DO UPDATE SET
         rating = excluded.rating,
         profile_version = max(profile_version, excluded.profile_version),
         updated_at_ms = max(updated_at_ms, excluded.updated_at_ms)`,
    ).bind(result.accountId, receipt.rating, receipt.profileVersion, receipt.appliedAtMs),
  ]);
}

function validateSettlement(value: unknown, enforceFreshness: boolean): SettlementCommand {
  assertObject(value, "settlement");
  if (value.v !== 1) throw new ApiError(400, "invalid_settlement", "Settlement version is unsupported.");
  const outcome = value.outcome;
  if (!["extracted", "defeated", "abandoned"].includes(String(outcome))) {
    throw new ApiError(400, "invalid_settlement", "Settlement outcome is unsupported.");
  }
  const issuedAtMs = requireInteger(
    value.issuedAtMs,
    "issuedAtMs",
    enforceFreshness ? Date.now() - 15 * 60 * 1_000 : 0,
    enforceFreshness ? Date.now() + 60_000 : 9_007_199_254_740_991,
  );
  const command: SettlementCommand = {
    v: 1,
    settlementId: requireIdentifier(value.settlementId, "settlementId"),
    accountId: requireIdentifier(value.accountId, "accountId"),
    reservationId: requireIdentifier(value.reservationId, "reservationId"),
    matchId: requireIdentifier(value.matchId, "matchId"),
    outcome: outcome as SettlementCommand["outcome"],
    loot: inventoryStacks(value.loot, "loot", 128),
    ratingDelta: requireInteger(value.ratingDelta, "ratingDelta", -500, 500),
    issuedAtMs,
  };
  if (
    (value.resultObjectKey === undefined) !==
    (value.resultObjectSha256 === undefined)
  ) {
    throw new ApiError(
      400,
      "invalid_result_evidence",
      "resultObjectKey and resultObjectSha256 must be supplied together.",
    );
  }
  if (value.resultObjectKey !== undefined) {
    command.resultObjectKey = requireString(value.resultObjectKey, "resultObjectKey", 512);
    const evidenceSha256 = requireString(
      value.resultObjectSha256,
      "resultObjectSha256",
      43,
    );
    if (!/^[A-Za-z0-9_-]{43}$/u.test(evidenceSha256)) {
      throw new ApiError(
        400,
        "invalid_result_evidence",
        "resultObjectSha256 must be a base64url SHA-256 digest.",
      );
    }
    command.resultObjectSha256 = evidenceSha256;
  }
  return command;
}
