ALTER TABLE match_allocations
  ADD COLUMN match_epoch INTEGER NOT NULL DEFAULT 1
  CHECK (match_epoch > 0);
