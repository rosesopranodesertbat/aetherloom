export const AUTHORITATIVE_HZ = 128;
export const MAX_COMBATANTS = 128;
export const MAX_PARTY_SIZE = 4;
export const MAX_GAMEPLAY_DATAGRAM_BYTES = 1_200;

export type TicketPurpose = "session" | "join" | "service" | "result";
export type Platform = "web" | "native" | "console";
export type InputPool = "browser" | "mouse-keyboard" | "controller" | "mixed";
export type Playlist = "casual-extraction" | "solo-extraction" | "squad-extraction";
export type MatchTransport = "wss" | "quic";
export type SettlementOutcome = "extracted" | "defeated" | "abandoned";

export interface PartyClaim {
  party_id: string;
  members: string[];
  input_pool: InputPool;
}

export interface TicketClaims {
  v: 1;
  iss: string;
  aud: string;
  purpose: TicketPurpose;
  sub: string;
  iat: number;
  nbf: number;
  exp: number;
  nonce: string;
  scopes?: string[];
  platform?: Platform;
  party?: PartyClaim;
  match_id?: string;
  match_epoch?: number;
  region?: string;
  build_hash?: string;
  input_pool?: InputPool;
  player_slot?: number;
  team_id?: number;
  settlement_id?: string;
  result_hash?: string;
}

/**
 * Exact claims accepted by a match host. Unlike the multipurpose control-plane
 * ticket type, join tickets cannot carry optional or extension claims.
 */
export interface JoinTicketClaims {
  v: 1;
  iss: string;
  aud: string;
  purpose: "join";
  sub: string;
  iat: number;
  nbf: number;
  exp: number;
  nonce: string;
  match_id: string;
  match_epoch: number;
  region: string;
  build_hash: string;
  input_pool: InputPool;
  player_slot: number;
  team_id: number;
}

export interface InventoryStack {
  itemId: string;
  quantity: number;
}

export interface ReservationCommand {
  accountId: string;
  reservationId: string;
  loadout: InventoryStack[];
  expiresAtMs: number;
}

export interface CommitReservationCommand {
  accountId: string;
  reservationId: string;
  matchId: string;
}

export interface RollbackReservationCommand extends CommitReservationCommand {
  expiresAtMs: number;
}

export interface IslandCheckpointCommand {
  accountId: string;
  checkpointId: string;
  expectedVersion: number;
  logicalTimeMs: number;
  contentBuild: string;
  objectKey: string;
  sha256: string;
  sizeBytes: number;
  etag: string;
}

export interface SettlementCommand {
  v: 1;
  settlementId: string;
  accountId: string;
  reservationId: string;
  matchId: string;
  outcome: SettlementOutcome;
  loot: InventoryStack[];
  ratingDelta: number;
  resultObjectKey?: string;
  resultObjectSha256?: string;
  issuedAtMs: number;
}

export interface QueuedSettlement extends SettlementCommand {
  payloadHash: string;
}

export interface SignedSettlementEnvelope {
  result: SettlementCommand;
  resultTicket: string;
}

export interface MatchmakingEnqueueCommand {
  queueTicketId: string;
  partyId: string;
  accountIds: string[];
  reservationIds: Record<string, string>;
  region: string;
  playlist: Playlist;
  inputPool: InputPool;
  skillMmr: number;
  expiresAtMs: number;
  requestHash: string;
}

export interface ClaimedQueueEntry {
  queueTicketId: string;
  partyId: string;
  accountIds: string[];
  reservationIds: Record<string, string>;
  skillMmr: number;
}

export interface DispatchRequest {
  shard: string;
  matchId: string;
  matchEpoch: number;
  region: string;
  playlist: Playlist;
  inputPool: InputPool;
  buildHash: string;
  targetPlayers: number;
  allowBots: boolean;
}

export interface MatchAllocation {
  matchId: string;
  matchEpoch: number;
  hostId: string;
  region: string;
  playlist: Playlist;
  inputPool: InputPool;
  transport: MatchTransport;
  buildHash: string;
  expiresAtMs: number;
  nativeEndpoint?: string;
}

export interface Env {
  PROFILE_ISLAND: DurableObjectNamespace;
  MATCHMAKING_SHARD: DurableObjectNamespace;
  CONTROL_DB: D1Database;
  MATCH_ARCHIVE: R2Bucket;
  SETTLEMENT_QUEUE: Queue<QueuedSettlement>;
  MATCH_CAPACITY_API?: Fetcher;
  BROWSER_MATCH_ORIGIN?: Fetcher;

  ENVIRONMENT: string;
  ENABLE_MULTIPLAYER_DEMO?: string;
  TICKET_ISSUER: string;
  PLAYER_TICKET_AUDIENCE: string;
  JOIN_TICKET_AUDIENCE: string;
  SERVICE_TICKET_AUDIENCE: string;
  RESULT_TICKET_AUDIENCE: string;
  RESULT_TICKET_HOSTS_JSON: string;
  CONTENT_BUILD_HASH: string;
  ALLOWED_ORIGINS_JSON: string;
  MATCHMAKING_SHARD_COUNT: string;
  MATCHMAKING_SKILL_BUCKET_WIDTH: string;
  MAX_ISLAND_CHECKPOINT_BYTES: string;

  PLAYER_TICKET_KEYS_JSON: string;
  JOIN_TICKET_SIGNING_KEYS_JSON: string;
  JOIN_TICKET_PUBLIC_KEYS_JSON: string;
  SERVICE_TICKET_KEYS_JSON: string;
  RESULT_TICKET_KEYS_JSON: string;
  ACTIVE_JOIN_TICKET_KID: string;
  ACTIVE_SERVICE_TICKET_KID: string;
}
