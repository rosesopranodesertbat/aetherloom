const IDENTIFIER_RE = /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$/;

export function resultTicketHosts(encoded: string): Record<string, string> {
  let value: unknown;
  try {
    value = JSON.parse(encoded);
  } catch {
    throw new Error("RESULT_TICKET_HOSTS_JSON is not valid JSON.");
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("RESULT_TICKET_HOSTS_JSON must be a JSON object.");
  }
  const entries = Object.entries(value as Record<string, unknown>);
  if (entries.length === 0 || entries.length > 8) {
    throw new Error("RESULT_TICKET_HOSTS_JSON must bind between one and eight result keys.");
  }
  return Object.fromEntries(
    entries.map(([keyId, hostId]) => {
      if (
        !IDENTIFIER_RE.test(keyId) ||
        typeof hostId !== "string" ||
        !IDENTIFIER_RE.test(hostId)
      ) {
        throw new Error("RESULT_TICKET_HOSTS_JSON contains an invalid key id or host id.");
      }
      return [keyId, hostId];
    }),
  );
}

export function authorizedResultSigner(
  encodedHosts: string,
  keyId: string,
  subject: string,
): string | null {
  const hostId = resultTicketHosts(encodedHosts)[keyId];
  return hostId !== undefined && hostId === subject ? hostId : null;
}

export function resultEvidenceObjectKey(
  matchId: string,
  settlementId: string,
  evidenceSha256: string,
): string {
  return `match.result.${matchId}.${settlementId}.${evidenceSha256}.json`;
}
