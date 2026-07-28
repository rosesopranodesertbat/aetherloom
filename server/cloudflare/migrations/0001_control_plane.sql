PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS match_allocations (
  match_id TEXT PRIMARY KEY,
  dispatch_hash TEXT NOT NULL,
  roster_hash TEXT NOT NULL,
  host_id TEXT NOT NULL,
  region TEXT NOT NULL,
  playlist TEXT NOT NULL,
  input_pool TEXT NOT NULL,
  transport TEXT NOT NULL CHECK (transport IN ('wss', 'quic')),
  native_endpoint TEXT,
  build_hash TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('reserved', 'active', 'aborting', 'draining', 'complete', 'released')),
  player_count INTEGER NOT NULL CHECK (player_count BETWEEN 1 AND 128),
  bot_count INTEGER NOT NULL CHECK (bot_count BETWEEN 0 AND 128),
  matchmaking_shard TEXT,
  lease_id TEXT,
  assignments_json TEXT,
  reservations_json TEXT,
  queue_confirmed_at_ms INTEGER,
  created_at_ms INTEGER NOT NULL,
  expires_at_ms INTEGER NOT NULL,
  released_at_ms INTEGER,
  release_reason TEXT,
  CHECK (
    status NOT IN ('active', 'aborting')
    OR (
      matchmaking_shard IS NOT NULL
      AND lease_id IS NOT NULL
      AND assignments_json IS NOT NULL
      AND reservations_json IS NOT NULL
    )
  )
);
CREATE INDEX IF NOT EXISTS match_allocations_region_status
  ON match_allocations(region, status, created_at_ms DESC);
CREATE INDEX IF NOT EXISTS match_allocations_expiry
  ON match_allocations(status, expires_at_ms);

CREATE TABLE IF NOT EXISTS profile_projection (
  account_id TEXT PRIMARY KEY,
  rating INTEGER NOT NULL DEFAULT 1000,
  profile_version INTEGER NOT NULL,
  island_version INTEGER NOT NULL DEFAULT 0,
  updated_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS profile_projection_rating
  ON profile_projection(rating DESC, account_id);

CREATE TABLE IF NOT EXISTS settlement_projection (
  settlement_id TEXT PRIMARY KEY,
  payload_hash TEXT NOT NULL,
  account_id TEXT NOT NULL,
  reservation_id TEXT NOT NULL UNIQUE,
  match_id TEXT NOT NULL,
  outcome TEXT NOT NULL CHECK (outcome IN ('extracted', 'defeated', 'abandoned')),
  rating_after INTEGER NOT NULL CHECK (rating_after BETWEEN 0 AND 5000),
  applied_at_ms INTEGER NOT NULL,
  archive_object_key TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS settlement_projection_account
  ON settlement_projection(account_id, applied_at_ms DESC);
CREATE INDEX IF NOT EXISTS settlement_projection_match
  ON settlement_projection(match_id, applied_at_ms);

CREATE TABLE IF NOT EXISTS match_history (
  match_id TEXT PRIMARY KEY,
  region TEXT NOT NULL,
  playlist TEXT NOT NULL,
  build_hash TEXT NOT NULL,
  replay_manifest_key TEXT,
  started_at_ms INTEGER NOT NULL,
  completed_at_ms INTEGER,
  settlement_state TEXT NOT NULL DEFAULT 'pending'
    CHECK (settlement_state IN ('pending', 'settling', 'complete', 'failed'))
);
