use aetherloom_protocol::MatchResult;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistenceKind {
    TickRecord,
    Checkpoint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayChunk {
    pub match_id: [u8; 16],
    pub content_build_hash: [u8; 16],
    pub first_tick: u64,
    pub last_tick: u64,
    pub kind: PersistenceKind,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SinkError {
    Backpressure,
    Unavailable,
    Rejected,
}

/// Bounded process queues call this non-blocking persistence boundary.
pub trait ReplayCheckpointSink {
    fn try_store(&mut self, chunk: &ReplayChunk) -> Result<(), SinkError>;
}

/// Receives only a result sealed by `DedicatedMatch` from verified admission
/// reservations and authoritative simulation state.
///
/// Implementations must use `MatchResult::result_id` as an idempotency key,
/// retain the exact content associated with that key, and reject conflicting
/// content. Evidence signing/upload and Cloudflare delivery remain on the far
/// side of this boundary; they must never rewrite the sealed result.
pub trait SettlementSink {
    fn try_settle(&mut self, result: &MatchResult) -> Result<(), SinkError>;
}

#[derive(Debug, Default)]
pub struct NoopReplaySink;

impl ReplayCheckpointSink for NoopReplaySink {
    fn try_store(&mut self, _chunk: &ReplayChunk) -> Result<(), SinkError> {
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct NoopSettlementSink;

impl SettlementSink for NoopSettlementSink {
    fn try_settle(&mut self, _result: &MatchResult) -> Result<(), SinkError> {
        Ok(())
    }
}
