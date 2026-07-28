CREATE TABLE IF NOT EXISTS demo_join_limits (
  source_key TEXT PRIMARY KEY,
  window_started_at_ms INTEGER NOT NULL CHECK (window_started_at_ms >= 0),
  claim_count INTEGER NOT NULL CHECK (claim_count >= 1),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms >= window_started_at_ms)
);

CREATE INDEX IF NOT EXISTS demo_join_limits_updated
  ON demo_join_limits(updated_at_ms);
