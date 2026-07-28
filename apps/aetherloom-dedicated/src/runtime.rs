use std::collections::{BTreeMap, VecDeque};

use aetherloom_core::{
    CommandSet, Controller, CoreError, Entity, MatchState, PlayerCommand, PlayerId, PlayerOutcome,
    TeamId, TickEvent, TickEventKind, TickEvents,
};
use aetherloom_protocol::{
    ChunkRevision, EntityId, EntityState, EnvelopeMetadata, EventBatch, EventKind, GameEvent,
    InputBatch, LootEntry, MatchOutcome, MatchResult, Message, MessageEnvelope, PlayerMatchResult,
    ProtocolError, SnapshotDelta, SnapshotId, SnapshotKeyframe, TerrainChunkState, TerrainDelta,
    TerrainOp, TerrainOpKind, MAX_DATAGRAM_BYTES, MAX_EVENTS_PER_BATCH,
};
use aetherloom_server::{
    can_reconnect, disconnect_action, DeliveryKind, DisconnectAction, HostError, HostKind,
    HostState, InboundMessage, MatchHost, MonotonicClock, PeerId, SchedulerError, TickId,
    TickSample, TickScheduler,
};

use crate::{
    AdmissionError, DeterministicSpatialGrid, EgressClass, OutboundPacket, PersistenceKind,
    PriorityEgress, ProcessConfig, ReplayCheckpointSink, ReplayChunk, SettlementSink,
    SignedTicketVerifier, SinkError, VerifiedTicket,
};

pub const MAX_COMMAND_FUTURE_TICKS: u64 = 8;
pub const MAX_SIMULATION_HEADROOM_NS: u64 = 4_000_000;
pub const MAX_TICK_JITTER_HEADROOM_NS: u64 = 500_000;
const MAX_INGRESS_PER_TICK: usize = 4_096;
const MAX_INGRESS_PER_PEER_PER_TICK: usize = 32;
const MAX_RECENT_CRITICAL_EVENTS: usize = 512;
/// Stable protocol item id used for the core's aggregate banked-resource balance.
///
/// Itemized expedition loot will replace this compatibility representation, but
/// the mapping is deliberately server-owned so settlement callers cannot choose
/// item ids or quantities.
const BANKED_RESOURCE_LOOT_ITEM_ID: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplicationProfile {
    NativeCompetitive128Hz,
    BrowserCasual32Hz,
}

impl ReplicationProfile {
    fn snapshot_due(self, tick: u64) -> bool {
        match self {
            Self::NativeCompetitive128Hz => true,
            Self::BrowserCasual32Hz => tick % 4 == 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapacitySignal {
    pub healthy: bool,
    pub content_build_hash: [u8; 16],
    pub available_match_slot: bool,
    pub p99_simulation_ns: u64,
    pub p99_tick_start_jitter_ns: u64,
}

impl CapacitySignal {
    pub const fn ready(content_build_hash: [u8; 16]) -> Self {
        Self {
            healthy: true,
            content_build_hash,
            available_match_slot: true,
            p99_simulation_ns: 0,
            p99_tick_start_jitter_ns: 0,
        }
    }

    pub fn has_headroom(self, expected_build: [u8; 16]) -> bool {
        self.healthy
            && self.available_match_slot
            && self.content_build_hash == expected_build
            && self.p99_simulation_ns < MAX_SIMULATION_HEADROOM_NS
            && self.p99_tick_start_jitter_ns < MAX_TICK_JITTER_HEADROOM_NS
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HealthReport {
    pub state: HostState,
    pub content_build_hash: [u8; 16],
    pub authoritative_tick: u64,
    pub connected_players: usize,
    pub reserved_players: usize,
    pub max_players: u16,
    pub admission_headroom: bool,
    pub egress_packets: usize,
    pub egress_bytes: usize,
    pub persistence_chunks: usize,
    pub persistence_bytes: usize,
    pub dropped_replay_tick_records: u64,
    pub dropped_replay_checkpoints: u64,
    pub replay_evidence_complete: bool,
    pub settlement_pending: bool,
    pub settlement_finalized: bool,
    pub host_receive_failures: u64,
    pub ingress_quota_drops: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InboundRejection {
    Protocol(ProtocolError),
    UnknownPeer(PeerId),
    WrongBuild,
    WrongEpoch,
    ReplayedEnvelope {
        previous: u32,
        received: u32,
    },
    StaleCommand {
        current: u64,
        received: u64,
    },
    FutureCommand {
        current: u64,
        received: u64,
        maximum: u64,
    },
    DuplicateTickCommand {
        player: PlayerId,
        tick: u64,
    },
    ObsoletePlayerSequence {
        player: PlayerId,
        previous: u32,
        received: u32,
    },
    NonMonotonicPendingSequence {
        player: PlayerId,
        earlier_tick: u64,
        earlier_sequence: u32,
        later_tick: u64,
        later_sequence: u32,
    },
    UnexpectedClientMessage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TickReport {
    pub tick: u64,
    pub state_hash: u64,
    pub applied_commands: usize,
    pub ingress_rejections: usize,
    pub ingress_quota_drops: u64,
    pub packets_sent: usize,
    pub packets_queued: usize,
    pub distant_updates_shed: u64,
    pub medium_updates_shed: u64,
    pub critical_updates_deferred: u64,
    pub persistence_chunks_queued: usize,
    pub scheduler_sample: TickSample,
}

#[derive(Debug)]
pub enum ProcessError {
    Core(CoreError),
    Host(HostError),
    Scheduler(SchedulerError),
    Protocol(ProtocolError),
    PermitMismatch {
        expected: u64,
        actual: u64,
    },
    InvalidResult,
    MatchNotTerminal(PlayerId),
    MissingAuthoritativePlayer(PlayerId),
    ReservationTeamMismatch(PlayerId),
    AuthoritativeResultMismatch,
    SettlementResultConflict,
    SettlementAlreadyFinalized,
    IncoherentTickFailure {
        expected_tick: u64,
        actual_tick: u64,
    },
}

impl core::fmt::Display for ProcessError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for ProcessError {}

impl From<CoreError> for ProcessError {
    fn from(value: CoreError) -> Self {
        Self::Core(value)
    }
}

impl From<HostError> for ProcessError {
    fn from(value: HostError) -> Self {
        Self::Host(value)
    }
}

impl From<SchedulerError> for ProcessError {
    fn from(value: SchedulerError) -> Self {
        Self::Scheduler(value)
    }
}

impl From<ProtocolError> for ProcessError {
    fn from(value: ProtocolError) -> Self {
        Self::Protocol(value)
    }
}

#[derive(Clone, Debug)]
struct PlayerSession {
    account_id: [u8; 16],
    player_id: PlayerId,
    reserved_team_id: TeamId,
    peer: Option<PeerId>,
    disconnected_at: Option<TickId>,
    expired: bool,
    terminal_settlement: Option<PlayerMatchResult>,
    last_envelope_sequence: Option<u32>,
    last_client_ack_tick: u64,
    replication_profile: ReplicationProfile,
    replication: ReplicationState,
}

#[derive(Clone, Debug)]
struct ReplicationState {
    baseline: SnapshotId,
    next_snapshot_id: u32,
    next_envelope_sequence: u32,
    known_entities: BTreeMap<EntityId, EntityState>,
    last_sent_ticks: BTreeMap<EntityId, u64>,
    last_sent_spell_cooldown_ticks: [u16; aetherloom_protocol::SPELL_COOLDOWN_SLOTS],
    needs_keyframe: bool,
    last_keyframe_tick: u64,
}

impl ReplicationState {
    fn new() -> Self {
        Self {
            baseline: SnapshotId::NONE,
            next_snapshot_id: 1,
            next_envelope_sequence: 1,
            known_entities: BTreeMap::new(),
            last_sent_ticks: BTreeMap::new(),
            last_sent_spell_cooldown_ticks: [0; aetherloom_protocol::SPELL_COOLDOWN_SLOTS],
            needs_keyframe: true,
            last_keyframe_tick: 0,
        }
    }

    fn allocate_snapshot_id(&self) -> SnapshotId {
        SnapshotId::new(self.next_snapshot_id)
    }

    fn advance_cursors(&mut self) {
        self.next_snapshot_id = self.next_snapshot_id.wrapping_add(1);
        if self.next_snapshot_id == 0 {
            self.next_snapshot_id = 1;
        }
        self.advance_envelope_cursor();
    }

    fn advance_envelope_cursor(&mut self) {
        self.next_envelope_sequence = self.next_envelope_sequence.wrapping_add(1);
        if self.next_envelope_sequence == 0 {
            self.next_envelope_sequence = 1;
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WireInterest {
    Critical,
    Medium,
    Distant,
}

impl WireInterest {
    fn due(self, tick: u64) -> bool {
        match self {
            Self::Critical => true,
            Self::Medium => tick % 4 == 0,
            Self::Distant => tick % 16 == 0,
        }
    }

    fn egress_class(self) -> EgressClass {
        match self {
            Self::Critical => EgressClass::CombatCritical,
            Self::Medium => EgressClass::Medium,
            Self::Distant => EgressClass::Distant,
        }
    }
}

#[derive(Clone, Debug)]
struct ReplicationCandidate {
    interest: WireInterest,
    is_viewer: bool,
    last_sent_tick: Option<u64>,
    distance_squared: i128,
    state: EntityState,
}

#[derive(Clone, Debug)]
struct ReplicationProposal {
    packet: OutboundPacket,
    replication: ReplicationState,
    distant_shed: u64,
    medium_shed: u64,
    critical_deferred: u64,
}

#[derive(Clone, Debug)]
struct PackedDelta {
    payload: Vec<u8>,
    included: Vec<EntityState>,
    highest_interest: WireInterest,
    distant_deferred: u64,
    medium_deferred: u64,
    critical_deferred: u64,
}

#[derive(Clone, Debug)]
struct TickOutputTemplate {
    message: Message,
    delivery: DeliveryKind,
    class: EgressClass,
    resync_if_dropped: bool,
    superseded_by_same_tick_keyframe: bool,
}

#[derive(Debug)]
struct PersistenceQueue {
    byte_capacity: usize,
    bytes: usize,
    chunks: VecDeque<ReplayChunk>,
    dropped_tick_hashes: u64,
    dropped_checkpoints: u64,
}

impl PersistenceQueue {
    fn new(byte_capacity: usize) -> Self {
        Self {
            byte_capacity,
            bytes: 0,
            chunks: VecDeque::new(),
            dropped_tick_hashes: 0,
            dropped_checkpoints: 0,
        }
    }

    fn enqueue(&mut self, chunk: ReplayChunk) {
        let length = chunk.bytes.len();
        if length > self.byte_capacity {
            self.record_drop(chunk.kind);
            return;
        }
        if chunk.kind == PersistenceKind::Checkpoint {
            while self.bytes.saturating_add(length) > self.byte_capacity {
                let Some(index) = self
                    .chunks
                    .iter()
                    .position(|queued| queued.kind == PersistenceKind::TickRecord)
                else {
                    break;
                };
                let removed = self
                    .chunks
                    .remove(index)
                    .expect("position points to an existing chunk");
                self.bytes -= removed.bytes.len();
                self.dropped_tick_hashes = self.dropped_tick_hashes.saturating_add(1);
            }
        }
        if self.bytes.saturating_add(length) > self.byte_capacity {
            self.record_drop(chunk.kind);
            return;
        }
        self.bytes += length;
        self.chunks.push_back(chunk);
    }

    fn flush<R: ReplayCheckpointSink>(&mut self, sink: &mut R) {
        while let Some(chunk) = self.chunks.front() {
            match sink.try_store(chunk) {
                Ok(()) => {
                    let stored = self.chunks.pop_front().expect("front chunk exists");
                    self.bytes -= stored.bytes.len();
                }
                Err(SinkError::Backpressure | SinkError::Unavailable) => return,
                Err(SinkError::Rejected) => {
                    let rejected = self.chunks.pop_front().expect("front chunk exists");
                    self.bytes -= rejected.bytes.len();
                    self.record_drop(rejected.kind);
                }
            }
        }
    }

    fn record_drop(&mut self, kind: PersistenceKind) {
        match kind {
            PersistenceKind::TickRecord => {
                self.dropped_tick_hashes = self.dropped_tick_hashes.saturating_add(1);
            }
            PersistenceKind::Checkpoint => {
                self.dropped_checkpoints = self.dropped_checkpoints.saturating_add(1);
            }
        }
    }
}

/// One authoritative match pinned to one simulation/content build.
pub struct DedicatedMatch<H, V, R, S> {
    config: ProcessConfig,
    host: H,
    verifier: V,
    replay_sink: R,
    settlement_sink: S,
    state: MatchState,
    spatial_grid: DeterministicSpatialGrid,
    sessions: BTreeMap<PlayerId, PlayerSession>,
    used_ticket_nonces: BTreeMap<[u8; 16], u64>,
    peer_bindings: BTreeMap<PeerId, PlayerId>,
    pending_commands: BTreeMap<u64, BTreeMap<PlayerId, PlayerCommand>>,
    egress: PriorityEgress,
    persistence: PersistenceQueue,
    pending_settlement: Option<MatchResult>,
    settled_result: Option<MatchResult>,
    capacity: CapacitySignal,
    started: bool,
    ingress_rejections: VecDeque<InboundRejection>,
    distant_updates_shed: u64,
    medium_updates_shed: u64,
    critical_updates_deferred: u64,
    host_receive_failures: u64,
    ingress_quota_drops: u64,
    recent_critical_events: VecDeque<GameEvent>,
}

impl<H, V, R, S> DedicatedMatch<H, V, R, S>
where
    H: MatchHost,
    V: SignedTicketVerifier,
    R: ReplayCheckpointSink,
    S: SettlementSink,
{
    pub fn new(
        config: ProcessConfig,
        host: H,
        verifier: V,
        replay_sink: R,
        settlement_sink: S,
    ) -> Self {
        let build_hash = config.build().content_build_hash();
        Self {
            state: MatchState::new(config.core_match_config(), config.seed()),
            spatial_grid: DeterministicSpatialGrid::default(),
            sessions: BTreeMap::new(),
            used_ticket_nonces: BTreeMap::new(),
            peer_bindings: BTreeMap::new(),
            pending_commands: BTreeMap::new(),
            egress: PriorityEgress::new(
                config.egress_packet_capacity(),
                config.egress_byte_capacity(),
            ),
            persistence: PersistenceQueue::new(config.persistence_byte_capacity()),
            pending_settlement: None,
            settled_result: None,
            capacity: CapacitySignal::ready(build_hash),
            started: false,
            ingress_rejections: VecDeque::with_capacity(256),
            distant_updates_shed: 0,
            medium_updates_shed: 0,
            critical_updates_deferred: 0,
            host_receive_failures: 0,
            ingress_quota_drops: 0,
            recent_critical_events: VecDeque::with_capacity(MAX_RECENT_CRITICAL_EVENTS),
            config,
            host,
            verifier,
            replay_sink,
            settlement_sink,
        }
    }

    pub const fn config(&self) -> ProcessConfig {
        self.config
    }

    pub fn state(&self) -> &MatchState {
        &self.state
    }

    pub fn host(&self) -> &H {
        &self.host
    }

    pub fn host_mut(&mut self) -> &mut H {
        &mut self.host
    }

    pub fn egress(&self) -> &PriorityEgress {
        &self.egress
    }

    pub fn spatial_grid(&self) -> &DeterministicSpatialGrid {
        &self.spatial_grid
    }

    pub fn update_capacity_signal(&mut self, signal: CapacitySignal) {
        self.capacity = signal;
    }

    pub fn health(&self) -> HealthReport {
        HealthReport {
            state: self.host.state(),
            content_build_hash: self.config.build().content_build_hash(),
            authoritative_tick: self.state.tick(),
            connected_players: self
                .sessions
                .values()
                .filter(|session| session.peer.is_some())
                .count(),
            reserved_players: self
                .state
                .players()
                .iter()
                .filter(|player| player.controller != Controller::Empty)
                .count(),
            max_players: self.config.max_players(),
            admission_headroom: self
                .capacity
                .has_headroom(self.config.build().content_build_hash())
                && self.host.accepts_new_players()
                && !self.started,
            egress_packets: self.egress.len(),
            egress_bytes: self.egress.queued_bytes(),
            persistence_chunks: self.persistence.chunks.len(),
            persistence_bytes: self.persistence.bytes,
            dropped_replay_tick_records: self.persistence.dropped_tick_hashes,
            dropped_replay_checkpoints: self.persistence.dropped_checkpoints,
            replay_evidence_complete: self.persistence.chunks.is_empty()
                && self.persistence.dropped_tick_hashes == 0
                && self.persistence.dropped_checkpoints == 0,
            settlement_pending: self.pending_settlement.is_some(),
            settlement_finalized: self.settled_result.is_some(),
            host_receive_failures: self.host_receive_failures,
            ingress_quota_drops: self.ingress_quota_drops,
        }
    }

    /// Advances non-blocking replay and settlement handoffs without advancing
    /// authoritative simulation time.
    ///
    /// Deployment loops use this while draining so an acknowledged background
    /// spool can retire the final queued chunks before process exit.
    pub fn poll_external_handoffs(&mut self) {
        self.persistence.flush(&mut self.replay_sink);
        self.flush_settlement();
    }

    pub fn admit(
        &mut self,
        peer: PeerId,
        signed_ticket: &[u8],
        now_unix_seconds: u64,
    ) -> Result<PlayerId, AdmissionError> {
        self.preflight_admission(peer)?;
        let ticket = self.verifier.verify(
            signed_ticket,
            self.config.build(),
            self.config.admission_scope(),
            now_unix_seconds,
        )?;
        self.bind_verified_admission(peer, ticket, now_unix_seconds)
    }

    /// Admits claims already authenticated by the connection transport.
    ///
    /// Native QUIC uses this entry point so a signed connection ticket is
    /// verified exactly once during the TLS-established admission flow. This
    /// method still pins the claims to this match, build, epoch, expiry, and
    /// capacity before creating the authoritative peer binding.
    pub fn admit_verified(
        &mut self,
        peer: PeerId,
        ticket: VerifiedTicket,
        verified_at_unix_seconds: u64,
    ) -> Result<PlayerId, AdmissionError> {
        self.preflight_admission(peer)?;
        self.bind_verified_admission(peer, ticket, verified_at_unix_seconds)
    }

    fn preflight_admission(&self, peer: PeerId) -> Result<(), AdmissionError> {
        if !self.host.accepts_new_players() {
            return Err(AdmissionError::Draining);
        }
        if !self
            .capacity
            .has_headroom(self.config.build().content_build_hash())
        {
            return Err(AdmissionError::NoHeadroom);
        }
        if self.peer_bindings.contains_key(&peer) {
            return Err(AdmissionError::PeerAlreadyBound);
        }
        Ok(())
    }

    fn bind_verified_admission(
        &mut self,
        peer: PeerId,
        ticket: VerifiedTicket,
        now_unix_seconds: u64,
    ) -> Result<PlayerId, AdmissionError> {
        self.validate_ticket(ticket, now_unix_seconds)?;
        self.used_ticket_nonces
            .retain(|_, expires_at| *expires_at > now_unix_seconds);
        if self.used_ticket_nonces.contains_key(&ticket.nonce) {
            return Err(AdmissionError::TicketReplayed);
        }

        if let Some(existing) = self.sessions.get_mut(&ticket.player_id) {
            if existing.account_id != ticket.account_id {
                return Err(AdmissionError::AccountMismatch);
            }
            if existing.reserved_team_id != ticket.team_id {
                return Err(AdmissionError::TeamMismatch);
            }
            if existing.peer.is_some() {
                return Err(AdmissionError::PlayerAlreadyConnected);
            }
            let disconnected_at = existing
                .disconnected_at
                .ok_or(AdmissionError::PlayerAlreadyConnected)?;
            if existing.expired || !can_reconnect(disconnected_at, TickId(self.state.tick())) {
                return Err(AdmissionError::ReconnectExpired);
            }
            self.state
                .set_controller(ticket.player_id, Controller::Human)
                .map_err(|_| AdmissionError::SimulationRejected)?;
            existing.peer = Some(peer);
            existing.disconnected_at = None;
            existing.last_envelope_sequence = None;
            existing.replication.needs_keyframe = true;
            self.peer_bindings.insert(peer, ticket.player_id);
            self.used_ticket_nonces
                .insert(ticket.nonce, ticket.expires_at_unix_seconds);
            return Ok(ticket.player_id);
        }

        if self.started {
            return Err(AdmissionError::MatchAlreadyStarted);
        }
        if self.sessions.len() >= self.config.max_players() as usize {
            return Err(AdmissionError::PlayerOutOfRange(ticket.player_id));
        }
        self.state
            .add_player(ticket.player_id, ticket.team_id, Controller::Human)
            .map_err(|_| AdmissionError::SimulationRejected)?;
        self.sessions.insert(
            ticket.player_id,
            PlayerSession {
                account_id: ticket.account_id,
                player_id: ticket.player_id,
                reserved_team_id: ticket.team_id,
                peer: Some(peer),
                disconnected_at: None,
                expired: false,
                terminal_settlement: None,
                last_envelope_sequence: None,
                last_client_ack_tick: 0,
                replication_profile: match self.host.kind() {
                    HostKind::DedicatedQuic => ReplicationProfile::NativeCompetitive128Hz,
                    HostKind::CloudflareWebSocket => ReplicationProfile::BrowserCasual32Hz,
                },
                replication: ReplicationState::new(),
            },
        );
        self.peer_bindings.insert(peer, ticket.player_id);
        self.used_ticket_nonces
            .insert(ticket.nonce, ticket.expires_at_unix_seconds);
        Ok(ticket.player_id)
    }

    fn validate_ticket(
        &self,
        ticket: VerifiedTicket,
        now_unix_seconds: u64,
    ) -> Result<(), AdmissionError> {
        let build = self.config.build();
        if ticket.expires_at_unix_seconds <= now_unix_seconds {
            return Err(AdmissionError::TicketExpired);
        }
        if ticket.match_id != build.match_id() {
            return Err(AdmissionError::WrongMatch);
        }
        if ticket.content_build_hash != build.content_build_hash() {
            return Err(AdmissionError::WrongBuild);
        }
        if ticket.match_epoch != build.match_epoch() {
            return Err(AdmissionError::WrongEpoch);
        }
        if ticket.region != self.config.admission_scope().region() {
            return Err(AdmissionError::WrongRegion);
        }
        if ticket.input_pool != self.config.admission_scope().input_pool() {
            return Err(AdmissionError::WrongInputPool);
        }
        if ticket.nonce.iter().all(|byte| *byte == 0) {
            return Err(AdmissionError::Ticket(
                crate::TicketVerificationError::Malformed,
            ));
        }
        if ticket.player_id.get() >= self.config.max_players() {
            return Err(AdmissionError::PlayerOutOfRange(ticket.player_id));
        }
        Ok(())
    }

    pub fn begin_drain(&mut self) {
        self.host.begin_drain();
    }

    pub const fn has_started(&self) -> bool {
        self.started
    }

    pub fn admitted_player_count(&self) -> usize {
        self.sessions.len()
    }

    /// True once every profile-backed participant has reached an authoritative
    /// terminal outcome. Bots are not profile reservations and are ignored.
    pub fn all_admitted_players_terminal(&self) -> bool {
        !self.sessions.is_empty()
            && self.sessions.iter().all(|(player_id, session)| {
                session.terminal_settlement.is_some()
                    || self
                        .state
                        .player(*player_id)
                        .is_some_and(|player| player.outcome != PlayerOutcome::Active)
            })
    }

    /// Converts still-active admitted participants to authoritative
    /// abandonment during an orchestrated shutdown.
    ///
    /// This is deliberately process-owned: callers cannot supply outcomes,
    /// score, loot, teams, or player membership. Unbanked and banked loot are
    /// forfeited, while the informational score remains the server-observed
    /// banked-resource count.
    pub fn abandon_active_players(&mut self) -> Result<usize, ProcessError> {
        let active: Vec<(PlayerId, TeamId, u32)> = self
            .sessions
            .iter()
            .filter_map(|(player_id, session)| {
                if session.terminal_settlement.is_some() {
                    return None;
                }
                let player = self.state.player(*player_id)?;
                (player.outcome == PlayerOutcome::Active).then_some((
                    *player_id,
                    session.reserved_team_id,
                    player.inventory.banked_resources,
                ))
            })
            .collect();

        for (player_id, team_id, score) in &active {
            self.state.set_controller(*player_id, Controller::Empty)?;
            let session = self
                .sessions
                .get_mut(player_id)
                .expect("active players come from sessions");
            if let Some(peer) = session.peer.take() {
                self.peer_bindings.remove(&peer);
                self.egress.purge_peer(peer);
            }
            session.expired = true;
            session.terminal_settlement = Some(PlayerMatchResult {
                player_id: *player_id,
                team_id: *team_id,
                outcome: MatchOutcome::Abandoned,
                rating_delta: 0,
                score: *score,
                banked_loot: Vec::new(),
            });
            self.clear_pending_for(*player_id);
        }
        Ok(active.len())
    }

    pub fn disconnect_peer(&mut self, peer: PeerId) -> bool {
        self.egress.purge_peer(peer);
        let Some(player_id) = self.peer_bindings.remove(&peer) else {
            return false;
        };
        let Some(session) = self.sessions.get_mut(&player_id) else {
            return false;
        };
        session.peer = None;
        session.disconnected_at = Some(TickId(self.state.tick()));
        self.clear_pending_for(player_id);
        true
    }

    fn clear_pending_for(&mut self, player_id: PlayerId) {
        for commands in self.pending_commands.values_mut() {
            commands.remove(&player_id);
        }
        self.pending_commands
            .retain(|_, commands| !commands.is_empty());
    }

    pub fn request_resync(&mut self, peer: PeerId) -> Result<(), InboundRejection> {
        let player_id = self
            .peer_bindings
            .get(&peer)
            .copied()
            .ok_or(InboundRejection::UnknownPeer(peer))?;
        self.sessions
            .get_mut(&player_id)
            .expect("peer bindings only reference sessions")
            .replication
            .needs_keyframe = true;
        Ok(())
    }

    pub fn ingest_message(&mut self, inbound: InboundMessage) -> Result<usize, InboundRejection> {
        let envelope = match (inbound.delivery, self.host.kind()) {
            (DeliveryKind::Datagram, _) => MessageEnvelope::decode_datagram(&inbound.payload),
            (DeliveryKind::Reliable, HostKind::CloudflareWebSocket) => {
                // WebSocket preserves this logical datagram frame reliably;
                // the envelope still carries gameplay-datagram semantics.
                MessageEnvelope::decode_datagram(&inbound.payload)
            }
            (DeliveryKind::Reliable, HostKind::DedicatedQuic) => {
                MessageEnvelope::decode_reliable(&inbound.payload)
            }
        }
        .map_err(InboundRejection::Protocol)?;

        let build = self.config.build();
        if envelope.metadata.content_build_hash != build.content_build_hash() {
            return Err(InboundRejection::WrongBuild);
        }
        if envelope.metadata.match_epoch != build.match_epoch() {
            return Err(InboundRejection::WrongEpoch);
        }
        let bound = self
            .peer_bindings
            .get(&inbound.peer)
            .copied()
            .ok_or(InboundRejection::UnknownPeer(inbound.peer))?;
        let Message::InputBatch(batch) = envelope.message else {
            return Err(InboundRejection::UnexpectedClientMessage);
        };

        let current = self.state.tick();
        let newest_tick = batch.newest().target_tick();
        if newest_tick < current {
            return Err(InboundRejection::StaleCommand {
                current,
                received: newest_tick,
            });
        }
        let maximum = current.saturating_add(MAX_COMMAND_FUTURE_TICKS);
        if newest_tick > maximum {
            return Err(InboundRejection::FutureCommand {
                current,
                received: newest_tick,
                maximum,
            });
        }
        if batch
            .commands
            .iter()
            .any(|command| command.target_tick() > maximum)
        {
            let received = batch
                .commands
                .iter()
                .map(PlayerCommand::target_tick)
                .max()
                .unwrap_or(newest_tick);
            return Err(InboundRejection::FutureCommand {
                current,
                received,
                maximum,
            });
        }

        let session = self
            .sessions
            .get(&bound)
            .expect("peer bindings only reference sessions");
        if let Some(previous) = session.last_envelope_sequence {
            if !sequence_is_newer(envelope.metadata.sequence, previous) {
                return Err(InboundRejection::ReplayedEnvelope {
                    previous,
                    received: envelope.metadata.sequence,
                });
            }
        }

        let inserted = self.insert_batch(bound, &batch, current)?;
        let session = self
            .sessions
            .get_mut(&bound)
            .expect("peer bindings only reference sessions");
        session.last_envelope_sequence = Some(envelope.metadata.sequence);
        session.last_client_ack_tick = envelope.metadata.acknowledgement_tick.min(current);
        Ok(inserted)
    }

    fn insert_batch(
        &mut self,
        player_id: PlayerId,
        batch: &InputBatch,
        current: u64,
    ) -> Result<usize, InboundRejection> {
        let mut staged = Vec::new();
        for command in &batch.commands {
            let tick = command.target_tick();
            if tick < current {
                continue;
            }
            match self
                .pending_commands
                .get(&tick)
                .and_then(|commands| commands.get(&player_id))
            {
                Some(existing) if existing == command => continue,
                Some(_) => {
                    return Err(InboundRejection::DuplicateTickCommand {
                        player: player_id,
                        tick,
                    });
                }
                None => staged.push(*command),
            }
        }

        let last_accepted = self
            .state
            .player(player_id)
            .and_then(|player| player.last_accepted_sequence());
        let mut timeline: BTreeMap<u64, u32> = self
            .pending_commands
            .iter()
            .filter_map(|(tick, commands)| {
                (*tick >= current)
                    .then(|| {
                        commands
                            .get(&player_id)
                            .map(|command| (*tick, command.sequence()))
                    })
                    .flatten()
            })
            .collect();
        for command in &staged {
            timeline.insert(command.target_tick(), command.sequence());
        }

        let mut previous = last_accepted.map(|sequence| (current.saturating_sub(1), sequence));
        for (tick, sequence) in timeline {
            if let Some((previous_tick, previous_sequence)) = previous {
                if !sequence_is_newer(sequence, previous_sequence) {
                    if previous_tick < current {
                        return Err(InboundRejection::ObsoletePlayerSequence {
                            player: player_id,
                            previous: previous_sequence,
                            received: sequence,
                        });
                    }
                    return Err(InboundRejection::NonMonotonicPendingSequence {
                        player: player_id,
                        earlier_tick: previous_tick,
                        earlier_sequence: previous_sequence,
                        later_tick: tick,
                        later_sequence: sequence,
                    });
                }
            }
            previous = Some((tick, sequence));
        }

        let inserted = staged.len();
        for command in staged {
            self.pending_commands
                .entry(command.target_tick())
                .or_default()
                .insert(player_id, command);
        }
        Ok(inserted)
    }

    pub fn pending_command_count(&self) -> usize {
        self.pending_commands.values().map(BTreeMap::len).sum()
    }

    pub fn replication_profile(&self, peer: PeerId) -> Option<ReplicationProfile> {
        let player_id = self.peer_bindings.get(&peer)?;
        self.sessions
            .get(player_id)
            .map(|session| session.replication_profile)
    }

    pub fn recent_rejections(&self) -> impl ExactSizeIterator<Item = &InboundRejection> {
        self.ingress_rejections.iter()
    }

    pub fn run_scheduled_tick<C: MonotonicClock>(
        &mut self,
        scheduler: &mut TickScheduler<C>,
    ) -> Result<TickReport, ProcessError> {
        let permit = scheduler.wait_for_tick()?;
        let expected_tick = permit.tick().0;
        let hash_before = self.state.authoritative_hash();
        let tick_result = self.advance_for_permit(permit.tick());
        let mut report = match tick_result {
            Ok(report) => {
                if self.state.tick() != expected_tick.saturating_add(1) {
                    scheduler.cancel_tick(permit)?;
                    return Err(ProcessError::IncoherentTickFailure {
                        expected_tick,
                        actual_tick: self.state.tick(),
                    });
                }
                report
            }
            Err(error) => {
                if self.state.tick() == expected_tick.saturating_add(1) {
                    let _ = scheduler.complete_tick(permit)?;
                    return Err(error);
                }
                scheduler.cancel_tick(permit)?;
                if self.state.tick() == expected_tick
                    && self.state.authoritative_hash() == hash_before
                {
                    return Err(error);
                }
                return Err(ProcessError::IncoherentTickFailure {
                    expected_tick,
                    actual_tick: self.state.tick(),
                });
            }
        };
        let scheduler_sample = scheduler.complete_tick(permit)?;
        report.scheduler_sample = scheduler_sample;
        if let Some(metrics) = scheduler.metrics().snapshot() {
            self.capacity.p99_simulation_ns = metrics.p99_duration_ns;
            self.capacity.p99_tick_start_jitter_ns = metrics.p99_start_jitter_ns.unsigned_abs();
        }
        Ok(report)
    }

    fn advance_for_permit(&mut self, permit_tick: TickId) -> Result<TickReport, ProcessError> {
        if permit_tick.0 != self.state.tick() {
            return Err(ProcessError::PermitMismatch {
                expected: self.state.tick(),
                actual: permit_tick.0,
            });
        }
        let rejected_before = self.ingress_rejections.len();
        let quota_drops_before = self.ingress_quota_drops;
        self.poll_ingress();
        if !self.started {
            self.fill_empty_with_bots()?;
        }
        self.update_disconnect_policy()?;

        let tick = self.state.tick();
        let mut commands = CommandSet::new(tick);
        if let Some(pending) = self.pending_commands.get(&tick) {
            for (player_id, command) in pending {
                commands
                    .insert(*player_id, *command)
                    .map_err(|error| ProcessError::Protocol(error.into()))?;
            }
        }
        let replay_record = encode_tick_record(&commands);
        let events = self.state.advance_tick(commands)?;
        self.pending_commands.remove(&tick);
        self.pending_commands.retain(|target, _| *target > tick);
        self.started = true;
        self.spatial_grid.rebuild(self.state.entities());

        self.enqueue_persistence(events.tick, replay_record);
        self.enqueue_replication()?;
        self.enqueue_tick_outputs(&events)?;
        let packets_sent = self.egress.flush(&mut self.host)?;
        self.persistence.flush(&mut self.replay_sink);
        self.flush_settlement();

        Ok(TickReport {
            tick: events.tick,
            state_hash: self.state.authoritative_hash(),
            applied_commands: events.applied_commands.len(),
            ingress_rejections: self.ingress_rejections.len() - rejected_before,
            ingress_quota_drops: self.ingress_quota_drops.saturating_sub(quota_drops_before),
            packets_sent,
            packets_queued: self.egress.len(),
            distant_updates_shed: self.distant_updates_shed,
            medium_updates_shed: self.medium_updates_shed,
            critical_updates_deferred: self.critical_updates_deferred,
            persistence_chunks_queued: self.persistence.chunks.len(),
            scheduler_sample: TickSample {
                tick: permit_tick,
                scheduled_start_ns: 0,
                actual_start_ns: 0,
                start_jitter_ns: 0,
                duration_ns: 0,
            },
        })
    }

    /// Adds one director-selected bot before insertion begins.
    pub fn add_bot(&mut self, player_id: PlayerId, team_id: TeamId) -> Result<(), AdmissionError> {
        if self.started {
            return Err(AdmissionError::MatchAlreadyStarted);
        }
        if player_id.get() >= self.config.max_players() {
            return Err(AdmissionError::PlayerOutOfRange(player_id));
        }
        if self
            .state
            .player(player_id)
            .is_some_and(|player| player.controller != Controller::Empty)
        {
            return Err(AdmissionError::BotSlotOccupied(player_id));
        }
        self.state
            .add_player(player_id, team_id, Controller::Bot)
            .map_err(|_| AdmissionError::SimulationRejected)?;
        Ok(())
    }

    /// Locks insertion and fills every unreserved combatant slot with a bot.
    ///
    /// Bots are ordinary core controllers and therefore generate ordinary
    /// `PlayerCommand` values inside the deterministic command path.
    pub fn fill_empty_with_bots(&mut self) -> Result<usize, ProcessError> {
        let mut added = 0;
        for raw in 0..self.config.max_players() {
            let player_id = PlayerId::new(raw).expect("ProcessConfig caps players at 128");
            if self
                .state
                .player(player_id)
                .is_some_and(|player| player.controller != Controller::Empty)
            {
                continue;
            }
            let team_id = TeamId::new(raw).expect("player and team limits are identical");
            self.state.add_player(player_id, team_id, Controller::Bot)?;
            added += 1;
        }
        self.started = true;
        Ok(added)
    }

    pub fn fill_vacancies_with_bots(&mut self) -> Result<usize, ProcessError> {
        self.fill_empty_with_bots()
    }

    fn poll_ingress(&mut self) {
        let mut accepted_per_peer = BTreeMap::<PeerId, usize>::new();
        for _ in 0..MAX_INGRESS_PER_TICK {
            let inbound = match self.host.poll_receive() {
                Ok(Some(inbound)) => inbound,
                Ok(None) => return,
                Err(_) => {
                    self.host_receive_failures = self.host_receive_failures.saturating_add(1);
                    self.capacity.healthy = false;
                    return;
                }
            };
            let accepted = accepted_per_peer.entry(inbound.peer).or_default();
            if *accepted == MAX_INGRESS_PER_PEER_PER_TICK {
                // Input batches are redundant and superseding. Bounding work
                // per peer protects the fixed tick budget; the transport's
                // fair dequeue still lets other admitted peers make progress.
                self.ingress_quota_drops = self.ingress_quota_drops.saturating_add(1);
                continue;
            }
            *accepted += 1;
            if let Err(rejection) = self.ingest_message(inbound) {
                self.push_rejection(rejection);
            }
        }
    }

    fn push_rejection(&mut self, rejection: InboundRejection) {
        if self.ingress_rejections.len() == 256 {
            self.ingress_rejections.pop_front();
        }
        self.ingress_rejections.push_back(rejection);
    }

    fn update_disconnect_policy(&mut self) -> Result<(), ProcessError> {
        let now = TickId(self.state.tick());
        let disconnected: Vec<PlayerId> = self
            .sessions
            .values()
            .filter(|session| session.peer.is_none() && !session.expired)
            .map(|session| session.player_id)
            .collect();
        for player_id in disconnected {
            let disconnected_at = self.sessions[&player_id]
                .disconnected_at
                .expect("disconnected sessions have a timestamp");
            match disconnect_action(disconnected_at, now) {
                DisconnectAction::AwaitReconnect => {}
                DisconnectAction::BotControl => {
                    if self.state.player(player_id).is_some_and(|player| {
                        player.controller == Controller::Human
                            && player.outcome == PlayerOutcome::Active
                    }) {
                        self.state.set_controller(player_id, Controller::Bot)?;
                    }
                }
                DisconnectAction::Defeat => {
                    let reserved_team_id = self.sessions[&player_id].reserved_team_id;
                    let terminal_settlement =
                        self.authoritative_player_result(player_id, reserved_team_id, true)?;
                    self.state.set_controller(player_id, Controller::Empty)?;
                    let session = self.sessions.get_mut(&player_id).expect("session exists");
                    session.expired = true;
                    session.terminal_settlement = Some(terminal_settlement);
                    self.clear_pending_for(player_id);
                }
            }
        }
        Ok(())
    }

    fn enqueue_persistence(&mut self, completed_tick: u64, mut replay_record: Vec<u8>) {
        let build = self.config.build();
        replay_record.extend_from_slice(&self.state.authoritative_hash().to_le_bytes());
        self.persistence.enqueue(ReplayChunk {
            match_id: build.match_id(),
            content_build_hash: build.content_build_hash(),
            first_tick: completed_tick,
            last_tick: completed_tick,
            kind: PersistenceKind::TickRecord,
            bytes: replay_record,
        });
        let interval = self.config.checkpoint_interval_ticks();
        if interval != 0 && self.state.tick() % interval == 0 {
            self.persistence.enqueue(ReplayChunk {
                match_id: build.match_id(),
                content_build_hash: build.content_build_hash(),
                first_tick: 0,
                last_tick: self.state.tick(),
                kind: PersistenceKind::Checkpoint,
                bytes: self.state.checkpoint().encode(),
            });
        }
    }

    fn enqueue_replication(&mut self) -> Result<(), ProcessError> {
        // Materialize the post-tick wire view once. Rebuilding the same map
        // and spatial merge for every viewer made replication work scale with
        // viewers times entities before any actual interest filtering.
        let current = self.current_wire_entities();
        let ordered_entity_ids = self.spatial_grid.all_entity_ids();
        let viewers: Vec<PlayerId> = self
            .sessions
            .values()
            .filter(|session| session.peer.is_some() && !session.expired)
            .map(|session| session.player_id)
            .collect();
        let mut resync_peers = Vec::new();
        for viewer in viewers {
            let Some(proposal) =
                self.replication_proposal(viewer, &current, &ordered_entity_ids)?
            else {
                continue;
            };
            let outcome = self.egress.enqueue(proposal.packet);
            resync_peers.extend(outcome.evicted_resync_peers);
            if outcome.accepted {
                let session = self
                    .sessions
                    .get_mut(&viewer)
                    .expect("viewer session exists");
                session.replication = proposal.replication;
                self.distant_updates_shed = self
                    .distant_updates_shed
                    .saturating_add(proposal.distant_shed);
                self.medium_updates_shed = self
                    .medium_updates_shed
                    .saturating_add(proposal.medium_shed);
                self.critical_updates_deferred = self
                    .critical_updates_deferred
                    .saturating_add(proposal.critical_deferred);
            } else {
                self.sessions
                    .get_mut(&viewer)
                    .expect("viewer session exists")
                    .replication
                    .needs_keyframe = true;
            }
        }
        for peer in resync_peers {
            if let Some(player_id) = self.peer_bindings.get(&peer).copied() {
                self.sessions
                    .get_mut(&player_id)
                    .expect("peer binding references a session")
                    .replication
                    .needs_keyframe = true;
            }
        }
        Ok(())
    }

    fn enqueue_tick_outputs(&mut self, events: &TickEvents) -> Result<(), ProcessError> {
        self.recent_critical_events
            .retain(|event| event.tick.saturating_add(1) >= events.tick);
        let redundant_critical: Vec<GameEvent> =
            self.recent_critical_events.iter().cloned().collect();
        let templates = tick_output_templates(events, &redundant_critical);
        for event in events
            .events
            .iter()
            .filter(|event| {
                event_route(event.kind) == (DeliveryKind::Datagram, EgressClass::CombatCritical)
            })
            .copied()
            .map(wire_event)
        {
            if self.recent_critical_events.len() == MAX_RECENT_CRITICAL_EVENTS {
                self.recent_critical_events.pop_front();
            }
            self.recent_critical_events.push_back(event);
        }
        if templates.is_empty() {
            return Ok(());
        }
        let viewers: Vec<PlayerId> = self
            .sessions
            .values()
            .filter(|session| session.peer.is_some() && !session.expired)
            .map(|session| session.player_id)
            .collect();
        let mut resync_peers = Vec::new();

        for viewer in viewers {
            let session = self
                .sessions
                .get(&viewer)
                .expect("tick-output viewers have sessions");
            let peer = session.peer.expect("tick-output viewers are connected");
            let acknowledgement_tick = session.last_client_ack_tick;
            let mut replication = session.replication.clone();
            let keyframed_this_tick = replication.last_keyframe_tick == self.state.tick();

            for template in &templates {
                if template.superseded_by_same_tick_keyframe && keyframed_this_tick {
                    continue;
                }
                let envelope = MessageEnvelope::new(
                    self.envelope_metadata(
                        replication.next_envelope_sequence,
                        acknowledgement_tick,
                    ),
                    template.message.clone(),
                );
                let payload = match template.delivery {
                    DeliveryKind::Datagram => envelope.encode_datagram()?,
                    DeliveryKind::Reliable => envelope.encode_reliable()?,
                };
                debug_assert!(
                    template.delivery != DeliveryKind::Datagram
                        || payload.len() <= MAX_DATAGRAM_BYTES
                );
                let outcome = self.egress.enqueue(OutboundPacket {
                    peer,
                    delivery: template.delivery,
                    class: template.class,
                    server_tick: self.state.tick(),
                    payload,
                    resync_if_dropped: template.resync_if_dropped,
                });
                resync_peers.extend(outcome.evicted_resync_peers);
                if outcome.accepted {
                    replication.advance_envelope_cursor();
                } else if template.resync_if_dropped {
                    replication.needs_keyframe = true;
                    self.critical_updates_deferred =
                        self.critical_updates_deferred.saturating_add(1);
                }
            }

            self.sessions
                .get_mut(&viewer)
                .expect("tick-output viewer remains connected")
                .replication = replication;
        }

        for peer in resync_peers {
            if let Some(player_id) = self.peer_bindings.get(&peer).copied() {
                self.sessions
                    .get_mut(&player_id)
                    .expect("peer binding references a session")
                    .replication
                    .needs_keyframe = true;
            }
        }
        Ok(())
    }

    fn replication_proposal(
        &self,
        viewer: PlayerId,
        current: &BTreeMap<EntityId, EntityState>,
        ordered_entity_ids: &[EntityId],
    ) -> Result<Option<ReplicationProposal>, ProcessError> {
        let session = self
            .sessions
            .get(&viewer)
            .expect("replication viewers have sessions");
        let peer = session.peer.expect("replication viewers are connected");
        let mut replication = session.replication.clone();
        let tick = self.state.tick();
        let keyframe_due = replication.needs_keyframe
            || replication.baseline.is_none()
            || tick.saturating_sub(replication.last_keyframe_tick)
                >= self.config.keyframe_interval_ticks();
        if keyframe_due {
            return Ok(Some(self.keyframe_proposal(
                peer,
                viewer,
                session,
                replication,
                current,
            )?));
        }
        if !session.replication_profile.snapshot_due(tick) {
            return Ok(None);
        }

        let viewer_position = self
            .state
            .player(viewer)
            .map(|player| player.position_cm)
            .unwrap_or([0; 3]);
        let mut candidates = Vec::new();
        for entity_id in ordered_entity_ids {
            let Some(entity) = current.get(entity_id) else {
                continue;
            };
            let distance_squared = spatial_distance_squared(viewer_position, entity.position_cm);
            let interest = classify_interest(
                self.state
                    .player(viewer)
                    .and_then(|player| player.entity_id),
                *entity_id,
                distance_squared,
            );
            if interest.due(tick) && replication.known_entities.get(entity_id) != Some(entity) {
                candidates.push(ReplicationCandidate {
                    interest,
                    is_viewer: self
                        .state
                        .player(viewer)
                        .and_then(|player| player.entity_id)
                        == Some(*entity_id),
                    last_sent_tick: replication.last_sent_ticks.get(entity_id).copied(),
                    distance_squared,
                    state: entity.clone(),
                });
            }
        }
        candidates.sort_by_key(|candidate| {
            (
                interest_rank(candidate.interest),
                !candidate.is_viewer,
                candidate.last_sent_tick.unwrap_or(0),
                candidate.distance_squared,
                candidate.state.entity_id,
            )
        });

        let removed: Vec<EntityId> = replication
            .known_entities
            .keys()
            .filter(|entity_id| !current.contains_key(entity_id))
            .copied()
            .collect();
        let spell_cooldown_ticks = self
            .state
            .player(viewer)
            .map(|player| player.spell_cooldown_ticks)
            .unwrap_or([0; aetherloom_protocol::SPELL_COOLDOWN_SLOTS]);
        let cooldowns_changed = spell_cooldown_ticks != replication.last_sent_spell_cooldown_ticks;
        if removed.len() > 64 {
            replication.needs_keyframe = true;
            return Ok(Some(self.keyframe_proposal(
                peer,
                viewer,
                session,
                replication,
                current,
            )?));
        }
        if candidates.is_empty() && removed.is_empty() && !cooldowns_changed {
            return Ok(None);
        }

        let snapshot_id = replication.allocate_snapshot_id();
        let packed = pack_delta(
            self.envelope_metadata(
                replication.next_envelope_sequence,
                session.last_client_ack_tick,
            ),
            snapshot_id,
            replication.baseline,
            viewer,
            self.state
                .player(viewer)
                .and_then(|player| player.last_accepted_sequence())
                .unwrap_or(0),
            spell_cooldown_ticks,
            &candidates,
            &removed,
        )?;
        if packed.included.is_empty() && removed.is_empty() && !cooldowns_changed {
            return Ok(None);
        }
        for entity_id in removed {
            replication.known_entities.remove(&entity_id);
            replication.last_sent_ticks.remove(&entity_id);
        }
        for entity in packed.included {
            replication
                .last_sent_ticks
                .insert(entity.entity_id, self.state.tick());
            replication.known_entities.insert(entity.entity_id, entity);
        }
        replication.last_sent_spell_cooldown_ticks = spell_cooldown_ticks;
        replication.baseline = snapshot_id;
        replication.advance_cursors();
        Ok(Some(ReplicationProposal {
            packet: OutboundPacket {
                peer,
                delivery: DeliveryKind::Datagram,
                class: if cooldowns_changed {
                    EgressClass::CombatCritical
                } else {
                    packed.highest_interest.egress_class()
                },
                server_tick: tick,
                payload: packed.payload,
                resync_if_dropped: true,
            },
            replication,
            distant_shed: packed.distant_deferred,
            medium_shed: packed.medium_deferred,
            critical_deferred: packed.critical_deferred,
        }))
    }

    fn keyframe_proposal(
        &self,
        peer: PeerId,
        viewer: PlayerId,
        session: &PlayerSession,
        mut replication: ReplicationState,
        current: &BTreeMap<EntityId, EntityState>,
    ) -> Result<ReplicationProposal, ProcessError> {
        let snapshot_id = replication.allocate_snapshot_id();
        let entities = current.values().cloned().collect();
        let terrain_revisions = self
            .state
            .terrain()
            .iter()
            .map(|chunk| ChunkRevision {
                x: chunk.coord.x,
                y: chunk.coord.y,
                revision: chunk.revision,
            })
            .collect();
        let terrain_chunks = self
            .state
            .terrain()
            .iter()
            .map(|chunk| TerrainChunkState {
                x: chunk.coord.x,
                y: chunk.coord.y,
                revision: chunk.revision,
                heights_cm: chunk.heights_cm.clone(),
            })
            .collect();
        let envelope = MessageEnvelope::new(
            self.envelope_metadata(
                replication.next_envelope_sequence,
                session.last_client_ack_tick,
            ),
            Message::SnapshotKeyframe(SnapshotKeyframe {
                snapshot_id,
                viewer,
                acknowledged_input_sequence: self
                    .state
                    .player(viewer)
                    .and_then(|player| player.last_accepted_sequence())
                    .unwrap_or(0),
                spell_cooldown_ticks: self
                    .state
                    .player(viewer)
                    .map(|player| player.spell_cooldown_ticks)
                    .unwrap_or([0; aetherloom_protocol::SPELL_COOLDOWN_SLOTS]),
                entities,
                terrain_revisions,
                terrain_chunks,
            }),
        );
        let payload = envelope.encode_reliable()?;
        replication.baseline = snapshot_id;
        replication.known_entities = current.clone();
        replication.last_sent_ticks = replication
            .known_entities
            .keys()
            .map(|entity_id| (*entity_id, self.state.tick()))
            .collect();
        replication.last_sent_spell_cooldown_ticks = self
            .state
            .player(viewer)
            .map(|player| player.spell_cooldown_ticks)
            .unwrap_or([0; aetherloom_protocol::SPELL_COOLDOWN_SLOTS]);
        replication.needs_keyframe = false;
        replication.last_keyframe_tick = self.state.tick();
        replication.advance_cursors();
        Ok(ReplicationProposal {
            packet: OutboundPacket {
                peer,
                delivery: DeliveryKind::Reliable,
                class: EgressClass::ReliableControl,
                server_tick: self.state.tick(),
                payload,
                resync_if_dropped: true,
            },
            replication,
            distant_shed: 0,
            medium_shed: 0,
            critical_deferred: 0,
        })
    }

    fn current_wire_entities(&self) -> BTreeMap<EntityId, EntityState> {
        self.state
            .entities()
            .iter()
            .map(|entity| (entity.id, wire_entity(entity)))
            .collect()
    }

    fn envelope_metadata(&self, sequence: u32, acknowledgement_tick: u64) -> EnvelopeMetadata {
        EnvelopeMetadata::new(
            self.config.build().content_build_hash(),
            self.config.build().match_epoch(),
            sequence,
            self.state.tick(),
            acknowledgement_tick,
        )
    }

    /// Builds the only match result this process is authorized to settle.
    ///
    /// The caller supplies only the globally unique idempotency key. Player
    /// membership and teams come from verified admission reservations, while
    /// outcomes and banked resources come from the authoritative `MatchState`.
    /// Bots are deliberately excluded because they have no profile reservation.
    pub fn authoritative_match_result(
        &self,
        result_id: [u8; 16],
    ) -> Result<MatchResult, ProcessError> {
        if result_id.iter().all(|byte| *byte == 0) || self.sessions.is_empty() {
            return Err(ProcessError::InvalidResult);
        }

        let mut players = Vec::with_capacity(self.sessions.len());
        for (player_id, session) in &self.sessions {
            if session.player_id != *player_id {
                return Err(ProcessError::MissingAuthoritativePlayer(*player_id));
            }
            let result = match &session.terminal_settlement {
                Some(result) => result.clone(),
                None => {
                    self.authoritative_player_result(*player_id, session.reserved_team_id, false)?
                }
            };
            if result.player_id != *player_id || result.team_id != session.reserved_team_id {
                return Err(ProcessError::ReservationTeamMismatch(*player_id));
            }
            players.push(result);
        }

        let result = MatchResult {
            result_id,
            match_id: self.config.build().match_id(),
            completed_tick: self.state.tick(),
            players,
        };
        result.validate().map_err(|_| ProcessError::InvalidResult)?;
        Ok(result)
    }

    /// Seals and submits an authoritative result without accepting economic
    /// content from the caller.
    ///
    /// Repeating the same id retries a backpressured sink. Once a different
    /// result has been sealed, this match can never be settled under another
    /// id.
    pub fn seal_match_result(&mut self, result_id: [u8; 16]) -> Result<MatchResult, ProcessError> {
        if let Some(existing) = self.sealed_result() {
            if existing.result_id != result_id {
                return Err(ProcessError::SettlementAlreadyFinalized);
            }
            let result = existing.clone();
            self.flush_settlement();
            return Ok(result);
        }

        let result = self.authoritative_match_result(result_id)?;
        self.pending_settlement = Some(result.clone());
        self.flush_settlement();
        Ok(result)
    }

    /// Strict compatibility boundary for callers that already construct a
    /// `MatchResult`.
    ///
    /// The proposed value is never trusted: every field must byte-for-byte
    /// equal the result derived from admission reservations and `MatchState`.
    /// Prefer [`Self::seal_match_result`] for new integrations.
    pub fn submit_match_result(&mut self, result: MatchResult) -> Result<(), ProcessError> {
        result.validate().map_err(|_| ProcessError::InvalidResult)?;

        if let Some(existing) = self.sealed_result() {
            if *existing != result {
                return if existing.result_id == result.result_id {
                    Err(ProcessError::SettlementResultConflict)
                } else {
                    Err(ProcessError::SettlementAlreadyFinalized)
                };
            }
            self.flush_settlement();
            return Ok(());
        }

        let authoritative = self.authoritative_match_result(result.result_id)?;
        if authoritative != result {
            return Err(ProcessError::AuthoritativeResultMismatch);
        }
        self.pending_settlement = Some(authoritative);
        self.flush_settlement();
        Ok(())
    }

    /// Returns the immutable pending or finalized result, if one exists.
    pub fn sealed_match_result(&self) -> Option<&MatchResult> {
        self.sealed_result()
    }

    fn sealed_result(&self) -> Option<&MatchResult> {
        self.settled_result
            .as_ref()
            .or(self.pending_settlement.as_ref())
    }

    fn authoritative_player_result(
        &self,
        player_id: PlayerId,
        reserved_team_id: TeamId,
        defeat_if_active: bool,
    ) -> Result<PlayerMatchResult, ProcessError> {
        let player = self
            .state
            .player(player_id)
            .ok_or(ProcessError::MissingAuthoritativePlayer(player_id))?;
        if player.controller == Controller::Empty {
            return Err(ProcessError::MissingAuthoritativePlayer(player_id));
        }
        if player.team != Some(reserved_team_id) {
            return Err(ProcessError::ReservationTeamMismatch(player_id));
        }

        let outcome = match player.outcome {
            PlayerOutcome::Active if defeat_if_active => MatchOutcome::Defeated,
            PlayerOutcome::Active => {
                return Err(ProcessError::MatchNotTerminal(player_id));
            }
            PlayerOutcome::Extracted => MatchOutcome::Extracted,
            PlayerOutcome::Defeated => MatchOutcome::Defeated,
            PlayerOutcome::Disconnected => MatchOutcome::Disconnected,
            PlayerOutcome::Abandoned => MatchOutcome::Abandoned,
        };
        let banked_resources = if outcome == MatchOutcome::Extracted {
            player.inventory.banked_resources
        } else {
            0
        };
        let banked_loot = if banked_resources == 0 {
            Vec::new()
        } else {
            vec![LootEntry {
                item_id: BANKED_RESOURCE_LOOT_ITEM_ID,
                quantity: banked_resources,
            }]
        };

        Ok(PlayerMatchResult {
            player_id,
            team_id: reserved_team_id,
            outcome,
            // Rating projection remains control-plane-owned until a versioned
            // in-match rating policy is introduced. The match process can only
            // authorize the neutral delta, never caller-selected rating changes.
            rating_delta: 0,
            score: banked_resources,
            banked_loot,
        })
    }

    fn flush_settlement(&mut self) {
        let Some(result) = self.pending_settlement.as_ref() else {
            return;
        };
        match self.settlement_sink.try_settle(result) {
            Ok(()) => {
                self.settled_result = self.pending_settlement.take();
            }
            Err(SinkError::Backpressure | SinkError::Unavailable) => {}
            Err(SinkError::Rejected) => {
                // Keep the immutable result for operational retry/evidence.
            }
        }
    }
}

fn tick_output_templates(
    events: &TickEvents,
    redundant_critical: &[GameEvent],
) -> Vec<TickOutputTemplate> {
    let mut templates = Vec::with_capacity(events.terrain.len().saturating_add(3));
    for deformation in &events.terrain {
        templates.push(TickOutputTemplate {
            message: Message::TerrainDelta(TerrainDelta {
                chunk_x: deformation.chunk.x,
                chunk_y: deformation.chunk.y,
                base_revision: deformation.base_revision,
                new_revision: deformation.new_revision,
                operations: vec![TerrainOp {
                    kind: TerrainOpKind::HeightDelta,
                    cell_x: deformation.cell_x as u16,
                    cell_y: deformation.cell_y as u16,
                    height_delta_cm: deformation.height_delta_cm,
                    material: 0,
                    flags: 0,
                }],
            }),
            delivery: DeliveryKind::Reliable,
            class: EgressClass::ReliableControl,
            resync_if_dropped: true,
            superseded_by_same_tick_keyframe: true,
        });
    }

    let mut reliable = Vec::new();
    let mut critical = redundant_critical.to_vec();
    let mut cosmetic = Vec::new();
    for event in &events.events {
        let wire = wire_event(*event);
        match event_route(event.kind) {
            (DeliveryKind::Reliable, _) => reliable.push(wire),
            (DeliveryKind::Datagram, EgressClass::CombatCritical) => critical.push(wire),
            (DeliveryKind::Datagram, EgressClass::Cosmetic) => cosmetic.push(wire),
            _ => unreachable!("event routes are exhaustive"),
        }
    }
    append_event_templates(
        &mut templates,
        reliable,
        DeliveryKind::Reliable,
        EgressClass::ReliableControl,
        true,
    );
    append_event_templates(
        &mut templates,
        critical,
        DeliveryKind::Datagram,
        EgressClass::CombatCritical,
        false,
    );
    append_event_templates(
        &mut templates,
        cosmetic,
        DeliveryKind::Datagram,
        EgressClass::Cosmetic,
        false,
    );
    templates
}

fn append_event_templates(
    templates: &mut Vec<TickOutputTemplate>,
    events: Vec<GameEvent>,
    delivery: DeliveryKind,
    class: EgressClass,
    resync_if_dropped: bool,
) {
    for batch in events.chunks(MAX_EVENTS_PER_BATCH) {
        templates.push(TickOutputTemplate {
            message: Message::EventBatch(EventBatch {
                events: batch.to_vec(),
            }),
            delivery,
            class,
            resync_if_dropped,
            superseded_by_same_tick_keyframe: false,
        });
    }
}

fn event_route(kind: TickEventKind) -> (DeliveryKind, EgressClass) {
    match kind {
        TickEventKind::PlayerJoined
        | TickEventKind::PlayerLeft
        | TickEventKind::Defeat
        | TickEventKind::Extraction => (DeliveryKind::Reliable, EgressClass::ReliableControl),
        TickEventKind::Cast | TickEventKind::Damage | TickEventKind::ProjectileImpact => {
            (DeliveryKind::Datagram, EgressClass::CombatCritical)
        }
        TickEventKind::EntitySpawned
        | TickEventKind::EntityDespawned
        | TickEventKind::TerrainDeformed => (DeliveryKind::Datagram, EgressClass::Cosmetic),
    }
}

fn wire_event(event: TickEvent) -> GameEvent {
    let kind = match event.kind {
        TickEventKind::PlayerJoined => EventKind::PLAYER_JOINED,
        TickEventKind::PlayerLeft => EventKind::PLAYER_LEFT,
        TickEventKind::Cast => EventKind::CAST,
        TickEventKind::Damage => EventKind::DAMAGE,
        TickEventKind::Defeat => EventKind::DEATH,
        TickEventKind::Extraction => EventKind::EXTRACTION,
        TickEventKind::EntitySpawned => EventKind::SPAWN,
        TickEventKind::EntityDespawned => EventKind::DESPAWN,
        TickEventKind::TerrainDeformed => EventKind::TERRAIN_DEFORMED,
        TickEventKind::ProjectileImpact => EventKind::HIT,
    };
    GameEvent {
        event_id: event.event_id,
        tick: event.tick,
        kind,
        actor: event.actor,
        target: event.target,
        data: event.data,
    }
}

#[allow(clippy::too_many_arguments)]
fn pack_delta(
    metadata: EnvelopeMetadata,
    snapshot_id: SnapshotId,
    baseline_id: SnapshotId,
    viewer: PlayerId,
    acknowledged_input_sequence: u32,
    spell_cooldown_ticks: [u16; aetherloom_protocol::SPELL_COOLDOWN_SLOTS],
    candidates: &[ReplicationCandidate],
    removed_entities: &[EntityId],
) -> Result<PackedDelta, ProtocolError> {
    let mut included = Vec::new();
    let mut highest_interest = WireInterest::Distant;
    let mut distant_deferred = 0_u64;
    let mut medium_deferred = 0_u64;
    let mut critical_deferred = 0_u64;

    for candidate in candidates {
        if included.len() == 32 {
            record_deferred(
                candidate.interest,
                &mut critical_deferred,
                &mut distant_deferred,
                &mut medium_deferred,
            );
            continue;
        }
        let mut trial = included.clone();
        trial.push(candidate.state.clone());
        let envelope = MessageEnvelope::new(
            metadata,
            Message::SnapshotDelta(SnapshotDelta {
                snapshot_id,
                baseline_id,
                viewer,
                acknowledged_input_sequence,
                spell_cooldown_ticks,
                entities: trial.clone(),
                removed_entities: removed_entities.to_vec(),
            }),
        );
        match envelope.encode_datagram() {
            Ok(payload) if payload.len() <= MAX_DATAGRAM_BYTES => {
                included = trial;
                if interest_rank(candidate.interest) < interest_rank(highest_interest) {
                    highest_interest = candidate.interest;
                }
            }
            _ => record_deferred(
                candidate.interest,
                &mut critical_deferred,
                &mut distant_deferred,
                &mut medium_deferred,
            ),
        }
    }

    let envelope = MessageEnvelope::new(
        metadata,
        Message::SnapshotDelta(SnapshotDelta {
            snapshot_id,
            baseline_id,
            viewer,
            acknowledged_input_sequence,
            spell_cooldown_ticks,
            entities: included.clone(),
            removed_entities: removed_entities.to_vec(),
        }),
    );
    let payload = envelope.encode_datagram()?;
    debug_assert!(payload.len() <= MAX_DATAGRAM_BYTES);
    Ok(PackedDelta {
        payload,
        included,
        highest_interest,
        distant_deferred,
        medium_deferred,
        critical_deferred,
    })
}

fn wire_entity(entity: &Entity) -> EntityState {
    EntityState {
        entity_id: entity.id,
        archetype: entity.kind as u16,
        owner: entity.owner,
        team: entity.team,
        position_cm: entity.position_cm,
        velocity_cm_per_tick: entity.velocity_cm_per_tick,
        yaw: entity.yaw,
        pitch: entity.pitch,
        health: entity.health,
        flags: entity.flags,
    }
}

fn encode_tick_record(commands: &CommandSet) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(18 + commands.len() * 27);
    bytes.extend_from_slice(b"ALTR");
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&commands.tick().to_le_bytes());
    bytes.extend_from_slice(&(commands.len() as u16).to_le_bytes());
    for (player_id, command) in commands.iter() {
        bytes.extend_from_slice(&player_id.get().to_le_bytes());
        bytes.extend_from_slice(&command.target_tick().to_le_bytes());
        bytes.extend_from_slice(&command.sequence().to_le_bytes());
        bytes.extend_from_slice(&command.move_x().to_le_bytes());
        bytes.extend_from_slice(&command.move_y().to_le_bytes());
        bytes.extend_from_slice(&command.move_vertical().to_le_bytes());
        bytes.extend_from_slice(&command.look_yaw().to_le_bytes());
        bytes.extend_from_slice(&command.look_pitch().to_le_bytes());
        bytes.extend_from_slice(&command.action_flags().to_le_bytes());
        bytes.push(command.requested_spell().unwrap_or(u8::MAX));
    }
    bytes
}

fn classify_interest(
    viewer_entity: Option<EntityId>,
    entity_id: EntityId,
    distance_squared: i128,
) -> WireInterest {
    if viewer_entity == Some(entity_id) || distance_squared <= 2_000_i128.pow(2) {
        WireInterest::Critical
    } else if distance_squared <= 8_000_i128.pow(2) {
        WireInterest::Medium
    } else {
        WireInterest::Distant
    }
}

fn interest_rank(interest: WireInterest) -> u8 {
    match interest {
        WireInterest::Critical => 0,
        WireInterest::Medium => 1,
        WireInterest::Distant => 2,
    }
}

fn record_deferred(
    interest: WireInterest,
    critical: &mut u64,
    distant: &mut u64,
    medium: &mut u64,
) {
    match interest {
        WireInterest::Critical => *critical = critical.saturating_add(1),
        WireInterest::Medium => *medium = medium.saturating_add(1),
        WireInterest::Distant => *distant = distant.saturating_add(1),
    }
}

fn spatial_distance_squared(first: [i32; 3], second: [i32; 3]) -> i128 {
    let dx = i128::from(first[0]) - i128::from(second[0]);
    let dy = i128::from(first[1]) - i128::from(second[1]);
    let dz = i128::from(first[2]) - i128::from(second[2]);
    dx * dx + dy * dy + dz * dz
}

fn sequence_is_newer(candidate: u32, previous: u32) -> bool {
    let delta = candidate.wrapping_sub(previous);
    delta != 0 && delta < (1_u32 << 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aetherloom_core::WorldSeed;
    use aetherloom_server::{DisconnectReason, HostKind};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Debug)]
    struct TestHost;

    impl MatchHost for TestHost {
        fn kind(&self) -> HostKind {
            HostKind::DedicatedQuic
        }

        fn state(&self) -> HostState {
            HostState::AcceptingPlayers
        }

        fn begin_drain(&mut self) {}

        fn stop(&mut self) {}

        fn poll_receive(&mut self) -> Result<Option<InboundMessage>, HostError> {
            Ok(None)
        }

        fn try_send_gameplay(&mut self, _peer: PeerId, _payload: &[u8]) -> Result<(), HostError> {
            Ok(())
        }

        fn try_send_reliable(&mut self, _peer: PeerId, _payload: &[u8]) -> Result<(), HostError> {
            Ok(())
        }

        fn disconnect(
            &mut self,
            _peer: PeerId,
            _reason: DisconnectReason,
        ) -> Result<(), HostError> {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct NeverVerifier;

    impl SignedTicketVerifier for NeverVerifier {
        fn verify(
            &self,
            _signed_ticket: &[u8],
            _expected_build: crate::MatchBuild,
            _expected_scope: crate::MatchAdmissionScope,
            _now_unix_seconds: u64,
        ) -> Result<VerifiedTicket, crate::TicketVerificationError> {
            Err(crate::TicketVerificationError::BadSignature)
        }
    }

    #[derive(Debug)]
    struct BackpressuredReplaySink;

    impl ReplayCheckpointSink for BackpressuredReplaySink {
        fn try_store(&mut self, _chunk: &ReplayChunk) -> Result<(), SinkError> {
            Err(SinkError::Backpressure)
        }
    }

    #[derive(Debug)]
    struct ScriptedSettlementSink {
        attempts: Rc<RefCell<Vec<MatchResult>>>,
        responses: VecDeque<Result<(), SinkError>>,
    }

    impl SettlementSink for ScriptedSettlementSink {
        fn try_settle(&mut self, result: &MatchResult) -> Result<(), SinkError> {
            self.attempts.borrow_mut().push(result.clone());
            self.responses.pop_front().unwrap_or(Ok(()))
        }
    }

    type SettlementRuntime =
        DedicatedMatch<TestHost, NeverVerifier, crate::NoopReplaySink, ScriptedSettlementSink>;

    #[derive(Debug, Default)]
    struct ManualClock {
        now: u64,
    }

    impl MonotonicClock for ManualClock {
        fn now_ns(&self) -> u64 {
            self.now
        }

        fn sleep_until_ns(&mut self, deadline_ns: u64) {
            self.now = self.now.max(deadline_ns);
        }
    }

    fn test_command(tick: u64, sequence: u32) -> PlayerCommand {
        PlayerCommand::new(tick, sequence, 0, 0, 0, 0, 0, 0, None).expect("valid command")
    }

    fn test_admission_scope() -> crate::MatchAdmissionScope {
        crate::MatchAdmissionScope::new(
            aetherloom_protocol::RegionId::new("local").expect("region"),
            aetherloom_protocol::InputPool::Mixed,
        )
    }

    fn settlement_runtime(
        responses: impl IntoIterator<Item = Result<(), SinkError>>,
    ) -> (SettlementRuntime, Rc<RefCell<Vec<MatchResult>>>) {
        let build = crate::MatchBuild::new([1; 16], [2; 16], 1).expect("build");
        let config = ProcessConfig::new(build, test_admission_scope(), WorldSeed::new(7), 2)
            .expect("config");
        let attempts = Rc::new(RefCell::new(Vec::new()));
        let sink = ScriptedSettlementSink {
            attempts: Rc::clone(&attempts),
            responses: responses.into_iter().collect(),
        };
        (
            DedicatedMatch::new(config, TestHost, NeverVerifier, crate::NoopReplaySink, sink),
            attempts,
        )
    }

    fn admit_settlement_players(runtime: &mut SettlementRuntime) {
        for raw in 0..2_u16 {
            let player_id = PlayerId::new(raw).expect("player");
            let team_id = TeamId::new(raw + 10).expect("team");
            let mut account_id = [0_u8; 16];
            account_id[0] = raw as u8 + 1;
            runtime
                .admit_verified(
                    PeerId(u64::from(raw) + 1),
                    VerifiedTicket {
                        match_id: runtime.config.build().match_id(),
                        content_build_hash: runtime.config.build().content_build_hash(),
                        match_epoch: runtime.config.build().match_epoch(),
                        region: runtime.config.admission_scope().region(),
                        input_pool: runtime.config.admission_scope().input_pool(),
                        nonce: [raw as u8 + 1; 16],
                        account_id,
                        player_id,
                        team_id,
                        expires_at_unix_seconds: 100,
                    },
                    1,
                )
                .expect("verified reservation");
        }
    }

    fn extract_settlement_players(runtime: &mut SettlementRuntime) {
        for raw in 0..2_u16 {
            let player_id = PlayerId::new(raw).expect("player");
            runtime.pending_commands.entry(0).or_default().insert(
                player_id,
                PlayerCommand::new(
                    0,
                    1,
                    0,
                    0,
                    0,
                    0,
                    0,
                    aetherloom_protocol::ACTION_EXTRACT,
                    None,
                )
                    .expect("extract command"),
            );
        }
        let mut scheduler = TickScheduler::new(ManualClock::default(), 8);
        runtime
            .run_scheduled_tick(&mut scheduler)
            .expect("terminal authoritative tick");
    }

    #[test]
    fn settlement_rejects_untrusted_outcomes_and_economics() {
        let (mut runtime, attempts) = settlement_runtime([]);
        admit_settlement_players(&mut runtime);
        let result_id = [3; 16];
        assert!(matches!(
            runtime.seal_match_result(result_id),
            Err(ProcessError::MatchNotTerminal(player))
                if player == PlayerId::new(0).expect("player")
        ));

        extract_settlement_players(&mut runtime);
        let authoritative = runtime
            .authoritative_match_result(result_id)
            .expect("terminal result");
        assert_eq!(authoritative.completed_tick, 1);
        assert_eq!(authoritative.players.len(), 2);
        assert_eq!(
            authoritative
                .players
                .iter()
                .map(|player| (player.player_id.get(), player.team_id.get()))
                .collect::<Vec<_>>(),
            vec![(0, 10), (1, 11)]
        );
        assert!(authoritative.players.iter().all(|player| {
            player.outcome == MatchOutcome::Extracted
                && player.rating_delta == 0
                && player.score == 0
                && player.banked_loot.is_empty()
        }));

        let mut forged_outcome = authoritative.clone();
        forged_outcome.players[0].outcome = MatchOutcome::Defeated;
        assert!(matches!(
            runtime.submit_match_result(forged_outcome),
            Err(ProcessError::AuthoritativeResultMismatch)
        ));

        let mut forged_economy = authoritative.clone();
        forged_economy.players[0].rating_delta = 32_000;
        forged_economy.players[0].score = u32::MAX;
        forged_economy.players[0].banked_loot.push(LootEntry {
            item_id: 999,
            quantity: u32::MAX,
        });
        assert!(matches!(
            runtime.submit_match_result(forged_economy),
            Err(ProcessError::AuthoritativeResultMismatch)
        ));

        let mut forged_roster = authoritative;
        forged_roster.players.pop();
        assert!(matches!(
            runtime.submit_match_result(forged_roster),
            Err(ProcessError::AuthoritativeResultMismatch)
        ));
        assert!(attempts.borrow().is_empty());
    }

    #[test]
    fn exact_authoritative_settlement_is_immutable_and_idempotent() {
        let (mut runtime, attempts) = settlement_runtime([Ok(())]);
        admit_settlement_players(&mut runtime);
        extract_settlement_players(&mut runtime);
        let expected = runtime
            .authoritative_match_result([4; 16])
            .expect("authoritative result");

        runtime
            .submit_match_result(expected.clone())
            .expect("exact result settles");
        assert!(runtime.health().settlement_finalized);
        assert_eq!(runtime.sealed_match_result(), Some(&expected));
        assert_eq!(attempts.borrow().as_slice(), &[expected.clone()]);

        runtime
            .submit_match_result(expected.clone())
            .expect("exact retry is idempotent");
        assert_eq!(attempts.borrow().len(), 1);

        let mut conflicting_content = expected.clone();
        conflicting_content.players[0].score = 1;
        assert!(matches!(
            runtime.submit_match_result(conflicting_content),
            Err(ProcessError::SettlementResultConflict)
        ));
        assert!(matches!(
            runtime.seal_match_result([5; 16]),
            Err(ProcessError::SettlementAlreadyFinalized)
        ));
        assert_eq!(runtime.sealed_match_result(), Some(&expected));
    }

    #[test]
    fn backpressured_settlement_retries_only_the_exact_seal() {
        let (mut runtime, attempts) = settlement_runtime([Err(SinkError::Backpressure), Ok(())]);
        admit_settlement_players(&mut runtime);
        extract_settlement_players(&mut runtime);

        let expected = runtime
            .seal_match_result([6; 16])
            .expect("authoritative seal");
        assert!(runtime.health().settlement_pending);
        assert!(!runtime.health().settlement_finalized);
        assert_eq!(attempts.borrow().as_slice(), &[expected.clone()]);

        let mut conflict = expected.clone();
        conflict.players[1].outcome = MatchOutcome::Defeated;
        assert!(matches!(
            runtime.submit_match_result(conflict),
            Err(ProcessError::SettlementResultConflict)
        ));
        assert!(matches!(
            runtime.seal_match_result([7; 16]),
            Err(ProcessError::SettlementAlreadyFinalized)
        ));
        assert_eq!(attempts.borrow().len(), 1);

        runtime
            .submit_match_result(expected.clone())
            .expect("exact retry succeeds");
        assert!(!runtime.health().settlement_pending);
        assert!(runtime.health().settlement_finalized);
        assert_eq!(attempts.borrow().as_slice(), &[expected.clone(), expected]);
    }

    #[test]
    fn reconnect_cannot_change_the_admitted_team_reservation() {
        let (mut runtime, _attempts) = settlement_runtime([]);
        admit_settlement_players(&mut runtime);
        assert!(runtime.disconnect_peer(PeerId(1)));

        let mut account_id = [0_u8; 16];
        account_id[0] = 1;
        assert_eq!(
            runtime.admit_verified(
                PeerId(99),
                VerifiedTicket {
                    match_id: runtime.config.build().match_id(),
                    content_build_hash: runtime.config.build().content_build_hash(),
                    match_epoch: runtime.config.build().match_epoch(),
                    region: runtime.config.admission_scope().region(),
                    input_pool: runtime.config.admission_scope().input_pool(),
                    nonce: [99; 16],
                    account_id,
                    player_id: PlayerId::new(0).expect("player"),
                    team_id: TeamId::new(12).expect("different team"),
                    expires_at_unix_seconds: 100,
                },
                1,
            ),
            Err(AdmissionError::TeamMismatch)
        );
    }

    #[test]
    fn verified_admission_rechecks_scope_and_consumes_nonce_once() {
        let (mut runtime, _attempts) = settlement_runtime([]);
        let build = runtime.config.build();
        let scope = runtime.config.admission_scope();
        let base = VerifiedTicket {
            match_id: build.match_id(),
            content_build_hash: build.content_build_hash(),
            match_epoch: build.match_epoch(),
            region: scope.region(),
            input_pool: scope.input_pool(),
            nonce: [12; 16],
            account_id: [7; 16],
            player_id: PlayerId::new(0).expect("player"),
            team_id: TeamId::new(0).expect("team"),
            expires_at_unix_seconds: 100,
        };

        assert_eq!(
            runtime.admit_verified(
                PeerId(1),
                VerifiedTicket {
                    region: aetherloom_protocol::RegionId::new("eeur").expect("region"),
                    ..base
                },
                1,
            ),
            Err(AdmissionError::WrongRegion)
        );
        assert_eq!(
            runtime.admit_verified(
                PeerId(1),
                VerifiedTicket {
                    input_pool: aetherloom_protocol::InputPool::Controller,
                    ..base
                },
                1,
            ),
            Err(AdmissionError::WrongInputPool)
        );
        assert_eq!(
            runtime.admit_verified(
                PeerId(1),
                VerifiedTicket {
                    nonce: [0; 16],
                    ..base
                },
                1,
            ),
            Err(AdmissionError::Ticket(
                crate::TicketVerificationError::Malformed
            ))
        );
        assert_eq!(
            runtime.admit_verified(PeerId(1), base, 1),
            Ok(base.player_id)
        );
        assert!(runtime.disconnect_peer(PeerId(1)));
        assert_eq!(
            runtime.admit_verified(PeerId(2), base, 1),
            Err(AdmissionError::TicketReplayed)
        );
        assert_eq!(
            runtime.admit_verified(
                PeerId(2),
                VerifiedTicket {
                    nonce: [13; 16],
                    ..base
                },
                1,
            ),
            Ok(base.player_id)
        );
    }

    #[test]
    fn core_rejection_cancels_permit_and_preserves_pending_tick_for_retry() {
        let build = crate::MatchBuild::new([1; 16], [2; 16], 1).expect("build");
        let config = ProcessConfig::new(build, test_admission_scope(), WorldSeed::new(7), 1)
            .expect("config");
        let mut runtime = DedicatedMatch::new(
            config,
            TestHost,
            NeverVerifier,
            crate::NoopReplaySink,
            crate::NoopSettlementSink,
        );
        let player = PlayerId::new(0).expect("player");
        runtime
            .state
            .add_player(player, TeamId::new(0).expect("team"), Controller::Human)
            .expect("player");
        runtime
            .pending_commands
            .entry(0)
            .or_default()
            .insert(player, test_command(0, 1));
        let mut scheduler = TickScheduler::new(ManualClock::default(), 8);
        runtime
            .run_scheduled_tick(&mut scheduler)
            .expect("first tick");

        runtime
            .pending_commands
            .entry(1)
            .or_default()
            .insert(player, test_command(1, 1));
        assert!(matches!(
            runtime.run_scheduled_tick(&mut scheduler),
            Err(ProcessError::Core(CoreError::Command(_)))
        ));
        assert_eq!(runtime.state.tick(), 1);
        assert_eq!(scheduler.next_tick(), TickId(1));
        assert_eq!(runtime.pending_command_count(), 1);

        runtime
            .pending_commands
            .get_mut(&1)
            .expect("pending tick")
            .insert(player, test_command(1, 2));
        runtime
            .run_scheduled_tick(&mut scheduler)
            .expect("retry succeeds");
        assert_eq!(runtime.state.tick(), 2);
        assert_eq!(scheduler.next_tick(), TickId(2));
    }

    #[test]
    fn replay_evidence_loss_is_visible_in_health() {
        let build = crate::MatchBuild::new([1; 16], [2; 16], 1).expect("build");
        let config = ProcessConfig::new(build, test_admission_scope(), WorldSeed::new(7), 1)
            .expect("config")
            .with_queue_limits(8, 8_192, 1)
            .expect("queue limits");
        let mut runtime = DedicatedMatch::new(
            config,
            TestHost,
            NeverVerifier,
            crate::NoopReplaySink,
            crate::NoopSettlementSink,
        );
        let mut scheduler = TickScheduler::new(ManualClock::default(), 8);
        runtime
            .run_scheduled_tick(&mut scheduler)
            .expect("authoritative tick");

        let health = runtime.health();
        assert_eq!(health.dropped_replay_tick_records, 1);
        assert_eq!(health.dropped_replay_checkpoints, 0);
        assert!(!health.replay_evidence_complete);
        assert!(!health.settlement_pending);
        assert!(!health.settlement_finalized);
    }

    #[test]
    fn queued_replay_evidence_is_not_reported_as_complete() {
        let build = crate::MatchBuild::new([1; 16], [2; 16], 1).expect("build");
        let config = ProcessConfig::new(build, test_admission_scope(), WorldSeed::new(7), 1)
            .expect("config");
        let mut runtime = DedicatedMatch::new(
            config,
            TestHost,
            NeverVerifier,
            BackpressuredReplaySink,
            crate::NoopSettlementSink,
        );
        let mut scheduler = TickScheduler::new(ManualClock::default(), 8);
        runtime
            .run_scheduled_tick(&mut scheduler)
            .expect("authoritative tick");

        let health = runtime.health();
        assert_eq!(health.dropped_replay_tick_records, 0);
        assert_eq!(health.persistence_chunks, 1);
        assert!(!health.replay_evidence_complete);
    }

    #[test]
    fn orchestrated_shutdown_seals_active_humans_as_abandoned() {
        let (mut runtime, attempts) = settlement_runtime([Ok(())]);
        admit_settlement_players(&mut runtime);
        assert!(!runtime.all_admitted_players_terminal());
        assert_eq!(runtime.abandon_active_players().expect("abandon"), 2);
        assert!(runtime.all_admitted_players_terminal());
        let result = runtime
            .seal_match_result([0x55; 16])
            .expect("authoritative shutdown result");
        assert!(result
            .players
            .iter()
            .all(|player| player.outcome == MatchOutcome::Abandoned));
        assert!(result
            .players
            .iter()
            .all(|player| player.banked_loot.is_empty()));
        assert_eq!(attempts.borrow().as_slice(), &[result]);
    }

    #[test]
    fn full_width_positions_cannot_overflow_replication_distance() {
        let distance =
            spatial_distance_squared(
                [i32::MIN, i32::MIN, i32::MIN],
                [i32::MAX, i32::MAX, i32::MAX],
            );
        assert_eq!(
            classify_interest(None, EntityId::new(0, 1).expect("entity"), distance),
            WireInterest::Distant
        );
    }
}
