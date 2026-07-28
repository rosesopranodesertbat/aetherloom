use std::collections::{BTreeSet, VecDeque};

use aetherloom_client::{
    AuthoritativeFeedError, AuthoritativeFeedOutcome, AuthoritativeFeedReplica,
};
use aetherloom_core::{Controller, PlayerOutcome, WorldSeed};
use aetherloom_dedicated::{
    AdmissionError, CapacitySignal, DedicatedMatch, DeterministicSpatialGrid, EgressClass,
    MatchAdmissionScope, MatchBuild, NoopReplaySink, NoopSettlementSink, OutboundPacket,
    PriorityEgress, ProcessConfig, ReplicationProfile, SignedTicketVerifier,
    TicketVerificationError, VerifiedTicket,
};
use aetherloom_protocol::{
    EntityId, EnvelopeMetadata, EventKind, InputBatch, InputPool, Message, MessageEnvelope,
    PlayerCommand, PlayerId, RegionId, TeamId, TerrainOpKind, ACTION_CAST, MAX_DATAGRAM_BYTES,
};
use aetherloom_server::{
    DeliveryKind, DisconnectReason, HostError, HostKind, HostState, InboundMessage, MatchHost,
    MonotonicClock, PeerId, TickScheduler, TransportError, BOT_TAKEOVER_AFTER_TICKS,
    RECONNECT_GRACE_TICKS,
};

const NOW: u64 = 10_000;

#[derive(Clone, Debug, Eq, PartialEq)]
struct SentPacket {
    peer: PeerId,
    delivery: DeliveryKind,
    payload: Vec<u8>,
}

#[derive(Debug)]
struct RecordingHost {
    state: HostState,
    kind: HostKind,
    inbound: VecDeque<InboundMessage>,
    sent: Vec<SentPacket>,
    backpressured: bool,
    backpressured_peers: BTreeSet<PeerId>,
    invalid_peers: BTreeSet<PeerId>,
    closed_peers: BTreeSet<PeerId>,
}

impl RecordingHost {
    fn accepting() -> Self {
        Self {
            state: HostState::AcceptingPlayers,
            kind: HostKind::DedicatedQuic,
            inbound: VecDeque::new(),
            sent: Vec::new(),
            backpressured: false,
            backpressured_peers: BTreeSet::new(),
            invalid_peers: BTreeSet::new(),
            closed_peers: BTreeSet::new(),
        }
    }

    fn backpressured() -> Self {
        Self {
            backpressured: true,
            ..Self::accepting()
        }
    }

    fn websocket() -> Self {
        Self {
            kind: HostKind::CloudflareWebSocket,
            ..Self::accepting()
        }
    }
}

impl MatchHost for RecordingHost {
    fn kind(&self) -> HostKind {
        self.kind
    }

    fn state(&self) -> HostState {
        self.state
    }

    fn begin_drain(&mut self) {
        if self.state == HostState::AcceptingPlayers {
            self.state = HostState::Draining;
        }
    }

    fn stop(&mut self) {
        self.state = HostState::Stopped;
    }

    fn poll_receive(&mut self) -> Result<Option<InboundMessage>, HostError> {
        Ok(self.inbound.pop_front())
    }

    fn try_send_gameplay(&mut self, peer: PeerId, payload: &[u8]) -> Result<(), HostError> {
        self.send(peer, DeliveryKind::Datagram, payload)
    }

    fn try_send_reliable(&mut self, peer: PeerId, payload: &[u8]) -> Result<(), HostError> {
        self.send(peer, DeliveryKind::Reliable, payload)
    }

    fn disconnect(&mut self, _peer: PeerId, _reason: DisconnectReason) -> Result<(), HostError> {
        Ok(())
    }
}

impl RecordingHost {
    fn send(
        &mut self,
        peer: PeerId,
        delivery: DeliveryKind,
        payload: &[u8],
    ) -> Result<(), HostError> {
        if self.backpressured || self.backpressured_peers.contains(&peer) {
            return Err(HostError::Transport(TransportError::Backpressure));
        }
        if self.invalid_peers.contains(&peer) {
            return Err(HostError::Transport(TransportError::InvalidPeer(peer)));
        }
        if self.closed_peers.contains(&peer) {
            return Err(HostError::Transport(TransportError::Closed));
        }
        self.sent.push(SentPacket {
            peer,
            delivery,
            payload: payload.to_vec(),
        });
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
struct TestVerifier;

impl SignedTicketVerifier for TestVerifier {
    fn verify(
        &self,
        signed_ticket: &[u8],
        expected_build: MatchBuild,
        expected_scope: MatchAdmissionScope,
        _now_unix_seconds: u64,
    ) -> Result<VerifiedTicket, TicketVerificationError> {
        let (player, account_variant, nonce_variant) = match signed_ticket {
            [player, account_variant] => (*player, *account_variant, 1),
            [player, account_variant, nonce_variant] => (*player, *account_variant, *nonce_variant),
            _ => return Err(TicketVerificationError::Malformed),
        };
        let player_id =
            PlayerId::new(player as u16).map_err(|_| TicketVerificationError::Malformed)?;
        let team_id =
            TeamId::new((player % 32) as u16).map_err(|_| TicketVerificationError::Malformed)?;
        let mut account_id = [0_u8; 16];
        account_id[0] = player;
        account_id[1] = account_variant;
        let mut nonce = [0_u8; 16];
        nonce[0] = player;
        nonce[1] = account_variant;
        nonce[2] = nonce_variant;
        Ok(VerifiedTicket {
            match_id: expected_build.match_id(),
            content_build_hash: if account_variant == 9 {
                [0x44; 16]
            } else {
                expected_build.content_build_hash()
            },
            match_epoch: expected_build.match_epoch(),
            region: expected_scope.region(),
            input_pool: expected_scope.input_pool(),
            nonce,
            account_id,
            player_id,
            team_id,
            expires_at_unix_seconds: NOW + 10_000,
        })
    }
}

#[derive(Debug, Default)]
struct ManualClock {
    now_ns: u64,
}

impl MonotonicClock for ManualClock {
    fn now_ns(&self) -> u64 {
        self.now_ns
    }

    fn sleep_until_ns(&mut self, deadline_ns: u64) {
        self.now_ns = self.now_ns.max(deadline_ns);
    }
}

type TestProcess = DedicatedMatch<RecordingHost, TestVerifier, NoopReplaySink, NoopSettlementSink>;

fn build() -> MatchBuild {
    MatchBuild::new([7; 16], [9; 16], 42).expect("valid build")
}

fn admission_scope() -> MatchAdmissionScope {
    MatchAdmissionScope::new(RegionId::new("local").expect("region"), InputPool::Mixed)
}

fn process(max_players: u16, host: RecordingHost) -> TestProcess {
    process_with_seed(max_players, host, 0x5eed)
}

fn process_with_seed(max_players: u16, host: RecordingHost, seed: u64) -> TestProcess {
    let config = ProcessConfig::new(
        build(),
        admission_scope(),
        WorldSeed::new(seed),
        max_players,
    )
    .expect("valid config")
    .with_intervals(256, 0)
    .expect("valid intervals");
    DedicatedMatch::new(
        config,
        host,
        TestVerifier,
        NoopReplaySink,
        NoopSettlementSink,
    )
}

fn constrained_process(max_players: u16, host: RecordingHost) -> TestProcess {
    let config = ProcessConfig::new(
        build(),
        admission_scope(),
        WorldSeed::new(0x5eed),
        max_players,
    )
    .expect("valid config")
    .with_queue_limits(4, 128 * 1024, 128)
    .expect("valid queues")
    .with_intervals(256, 0)
    .expect("valid intervals");
    DedicatedMatch::new(
        config,
        host,
        TestVerifier,
        NoopReplaySink,
        NoopSettlementSink,
    )
}

fn ticket(player: u8) -> [u8; 2] {
    [player, 1]
}

fn reconnect_ticket(player: u8, nonce_variant: u8) -> [u8; 3] {
    [player, 1, nonce_variant]
}

fn peer(player: u8) -> PeerId {
    PeerId(1_000 + u64::from(player))
}

fn admit_players(process: &mut TestProcess, count: u16) {
    for raw in 0..count {
        let raw = raw as u8;
        assert_eq!(
            process.admit(peer(raw), &ticket(raw), NOW),
            Ok(PlayerId::new(raw as u16).expect("player"))
        );
    }
}

fn run_tick(process: &mut TestProcess, scheduler: &mut TickScheduler<ManualClock>) {
    process
        .run_scheduled_tick(scheduler)
        .expect("authoritative tick");
}

fn input(
    player: u8,
    peer: PeerId,
    target_tick: u64,
    envelope_sequence: u32,
    move_x: i16,
) -> InboundMessage {
    let command = command(target_tick, envelope_sequence, move_x);
    input_batch(player, peer, envelope_sequence, vec![command])
}

fn command(target_tick: u64, sequence: u32, move_x: i16) -> PlayerCommand {
    PlayerCommand::new(target_tick, sequence, move_x, 0, 0, 0, 0, 0, None)
        .expect("valid command")
}

fn cast_command(target_tick: u64, sequence: u32, yaw: u16) -> PlayerCommand {
    PlayerCommand::new(
        target_tick,
        sequence,
        0,
        0,
        0,
        yaw,
        0,
        ACTION_CAST,
        Some(0),
    )
        .expect("valid cast")
}

fn mend_command(target_tick: u64, sequence: u32) -> PlayerCommand {
    PlayerCommand::new(target_tick, sequence, 0, 0, 0, 0, 0, ACTION_CAST, Some(7))
        .expect("valid Mend command")
}

fn terrain_cast_command(target_tick: u64, sequence: u32, yaw: u16) -> PlayerCommand {
    PlayerCommand::new(
        target_tick,
        sequence,
        0,
        0,
        0,
        yaw,
        -aetherloom_protocol::MAX_LOOK_PITCH,
        ACTION_CAST,
        Some(0),
    )
    .expect("valid terrain cast")
}

fn input_batch(
    _player: u8,
    peer: PeerId,
    envelope_sequence: u32,
    commands: Vec<PlayerCommand>,
) -> InboundMessage {
    let batch = InputBatch::new(commands).expect("valid batch");
    let newest_tick = batch.newest().target_tick();
    let envelope = MessageEnvelope::new(
        EnvelopeMetadata::new(
            build().content_build_hash(),
            build().match_epoch(),
            envelope_sequence,
            newest_tick,
            newest_tick.saturating_sub(1),
        ),
        Message::InputBatch(batch),
    );
    InboundMessage {
        peer,
        delivery: DeliveryKind::Datagram,
        payload: envelope.encode_datagram().expect("datagram"),
    }
}

fn decode_sent(packet: &SentPacket) -> MessageEnvelope {
    match packet.delivery {
        DeliveryKind::Datagram => {
            MessageEnvelope::decode_datagram(&packet.payload).expect("valid datagram")
        }
        DeliveryKind::Reliable => {
            MessageEnvelope::decode_reliable(&packet.payload).expect("valid reliable frame")
        }
    }
}

fn assert_replica_terrain_matches(process: &TestProcess, replica: &AuthoritativeFeedReplica) {
    assert_eq!(replica.terrain().len(), process.state().terrain().len());
    for authoritative in process.state().terrain() {
        let replicated = replica
            .terrain_chunk(authoritative.coord.x, authoritative.coord.y)
            .expect("authoritative chunk replicated");
        assert_eq!(replicated.revision, authoritative.revision);
        assert_eq!(replicated.heights_cm, authoritative.heights_cm);
    }
}

#[test]
fn deterministic_matches_cover_16_and_128_slots() {
    for player_count in [16_u16, 128] {
        let mut first = process(player_count, RecordingHost::accepting());
        let mut second = process(player_count, RecordingHost::accepting());
        admit_players(&mut first, player_count);
        admit_players(&mut second, player_count);
        let mut first_clock = TickScheduler::new(ManualClock::default(), 32);
        let mut second_clock = TickScheduler::new(ManualClock::default(), 32);
        for _ in 0..4 {
            run_tick(&mut first, &mut first_clock);
            run_tick(&mut second, &mut second_clock);
        }
        assert_eq!(
            first.state().authoritative_hash(),
            second.state().authoritative_hash()
        );
        assert_eq!(
            first
                .state()
                .players()
                .iter()
                .filter(|player| player.controller != Controller::Empty)
                .count(),
            player_count as usize
        );
    }
}

#[test]
fn first_permit_fills_unreserved_slots_with_command_path_bots() {
    let mut process = process(16, RecordingHost::accepting());
    admit_players(&mut process, 1);
    let mut scheduler = TickScheduler::new(ManualClock::default(), 8);
    run_tick(&mut process, &mut scheduler);

    assert_eq!(process.health().reserved_players, 16);
    assert_eq!(
        process
            .state()
            .players()
            .iter()
            .filter(|player| player.controller == Controller::Bot)
            .count(),
        15
    );
    assert_eq!(
        process.admit(peer(1), &ticket(1), NOW),
        Err(AdmissionError::MatchAlreadyStarted)
    );
}

#[test]
fn director_selected_bot_uses_the_same_command_set_as_humans() {
    let mut process = process(2, RecordingHost::accepting());
    admit_players(&mut process, 1);
    let bot = PlayerId::new(1).expect("bot slot");
    process
        .add_bot(bot, TeamId::new(1).expect("team"))
        .expect("pre-start bot");
    assert_eq!(
        process.add_bot(bot, TeamId::new(1).expect("team")),
        Err(AdmissionError::BotSlotOccupied(bot))
    );

    let mut scheduler = TickScheduler::new(ManualClock::default(), 8);
    let report = process
        .run_scheduled_tick(&mut scheduler)
        .expect("authoritative tick");
    assert_eq!(report.applied_commands, 1);
    assert_eq!(
        process.state().player(bot).expect("bot").controller,
        Controller::Bot
    );
}

#[test]
fn rejects_unbound_peer_replay_future_and_stale_commands() {
    let mut process = process(2, RecordingHost::accepting());
    admit_players(&mut process, 2);

    let unbound_peer = PeerId(999_999);
    let spoof = input(1, unbound_peer, 0, 1, 100);
    assert!(matches!(
        process.ingest_message(spoof),
        Err(aetherloom_dedicated::InboundRejection::UnknownPeer(peer))
            if peer == unbound_peer
    ));

    let accepted = input(0, peer(0), 0, 1, 100);
    assert_eq!(process.ingest_message(accepted.clone()), Ok(1));
    assert!(matches!(
        process.ingest_message(accepted),
        Err(aetherloom_dedicated::InboundRejection::ReplayedEnvelope { .. })
    ));

    let future = input(0, peer(0), 9, 2, 100);
    assert!(matches!(
        process.ingest_message(future),
        Err(aetherloom_dedicated::InboundRejection::FutureCommand { .. })
    ));

    let mut scheduler = TickScheduler::new(ManualClock::default(), 8);
    run_tick(&mut process, &mut scheduler);
    let stale = input(0, peer(0), 0, 2, 100);
    assert!(matches!(
        process.ingest_message(stale),
        Err(aetherloom_dedicated::InboundRejection::StaleCommand { .. })
    ));
}

#[test]
fn per_tick_ingress_quota_bounds_flood_work_without_starving_a_peer() {
    let mut process = process(2, RecordingHost::accepting());
    admit_players(&mut process, 2);

    let flooding_input = input(0, peer(0), 0, 1, 100);
    for _ in 0..64 {
        process.host_mut().inbound.push_back(flooding_input.clone());
    }
    process
        .host_mut()
        .inbound
        .push_back(input(1, peer(1), 0, 1, -100));

    let mut scheduler = TickScheduler::new(ManualClock::default(), 8);
    let report = process
        .run_scheduled_tick(&mut scheduler)
        .expect("flood-resistant authoritative tick");

    assert_eq!(report.applied_commands, 2);
    assert_eq!(report.ingress_quota_drops, 32);
    assert_eq!(process.health().ingress_quota_drops, 32);
    assert!(
        process.host().inbound.is_empty(),
        "the healthy peer was consumed during the same bounded poll"
    );
}

#[test]
fn rejects_obsolete_current_sequence_without_desynchronizing_scheduler() {
    let mut process = process(1, RecordingHost::accepting());
    admit_players(&mut process, 1);
    let mut scheduler = TickScheduler::new(ManualClock::default(), 8);

    process
        .ingest_message(input(0, peer(0), 0, 1, 100))
        .expect("initial input");
    run_tick(&mut process, &mut scheduler);
    assert_eq!(process.state().tick(), 1);
    assert_eq!(scheduler.next_tick().0, 1);

    let obsolete = input_batch(0, peer(0), 2, vec![command(1, 1, 100)]);
    assert!(matches!(
        process.ingest_message(obsolete),
        Err(
            aetherloom_dedicated::InboundRejection::ObsoletePlayerSequence {
                previous: 1,
                received: 1,
                ..
            }
        )
    ));
    assert!(matches!(
        process.ingest_message(input_batch(0, peer(0), 2, vec![command(0, 2, 100)])),
        Err(aetherloom_dedicated::InboundRejection::StaleCommand { .. })
    ));
    assert!(matches!(
        process.ingest_message(input_batch(0, peer(0), 2, vec![command(10, 2, 100)])),
        Err(aetherloom_dedicated::InboundRejection::FutureCommand { .. })
    ));
    assert_eq!(process.pending_command_count(), 0);
    process
        .ingest_message(input_batch(0, peer(0), 2, vec![command(1, 2, 100)]))
        .expect("newer current input remains valid");

    run_tick(&mut process, &mut scheduler);
    assert_eq!(process.state().tick(), 2);
    assert_eq!(scheduler.next_tick().0, 2);
}

#[test]
fn redundant_batch_conflict_is_rejected_atomically() {
    let mut process = process(1, RecordingHost::accepting());
    admit_players(&mut process, 1);
    process
        .ingest_message(input_batch(0, peer(0), 1, vec![command(1, 2, 50)]))
        .expect("original future command");
    assert_eq!(process.pending_command_count(), 1);

    let conflicting = input_batch(
        0,
        peer(0),
        2,
        vec![command(2, 4, 400), command(1, 3, 300), command(0, 2, 200)],
    );
    assert!(matches!(
        process.ingest_message(conflicting),
        Err(aetherloom_dedicated::InboundRejection::DuplicateTickCommand { tick: 1, .. })
    ));
    assert_eq!(
        process.pending_command_count(),
        1,
        "neither the command before nor after the conflict may commit"
    );

    process
        .ingest_message(input_batch(0, peer(0), 3, vec![command(0, 1, 100)]))
        .expect("atomic rejection leaves current tick available");
    let mut scheduler = TickScheduler::new(ManualClock::default(), 8);
    run_tick(&mut process, &mut scheduler);
    run_tick(&mut process, &mut scheduler);
    assert_eq!(process.state().tick(), 2);
    assert_eq!(scheduler.next_tick().0, 2);
}

#[test]
fn reliable_resync_emits_a_keyframe() {
    let mut process = process(1, RecordingHost::accepting());
    admit_players(&mut process, 1);
    let mut scheduler = TickScheduler::new(ManualClock::default(), 8);
    run_tick(&mut process, &mut scheduler);
    process.host_mut().sent.clear();

    process.request_resync(peer(0)).expect("known peer");
    run_tick(&mut process, &mut scheduler);

    let keyframes: Vec<_> = process
        .host()
        .sent
        .iter()
        .filter(|packet| packet.delivery == DeliveryKind::Reliable)
        .map(|packet| MessageEnvelope::decode_reliable(&packet.payload).expect("valid frame"))
        .filter(|envelope| matches!(envelope.message, Message::SnapshotKeyframe(_)))
        .collect();
    assert_eq!(keyframes.len(), 1);
}

#[test]
fn cooldown_only_casts_emit_native_snapshot_deltas() {
    let mut process = process(1, RecordingHost::accepting());
    admit_players(&mut process, 1);
    let mut scheduler = TickScheduler::new(ManualClock::default(), 8);
    run_tick(&mut process, &mut scheduler);
    process.host_mut().sent.clear();

    process
        .ingest_message(input_batch(0, peer(0), 1, vec![mend_command(1, 1)]))
        .expect("Mend input");
    run_tick(&mut process, &mut scheduler);

    let cooldown_delta = process
        .host()
        .sent
        .iter()
        .filter(|packet| packet.delivery == DeliveryKind::Datagram)
        .map(decode_sent)
        .find_map(|envelope| match envelope.message {
            Message::SnapshotDelta(delta) => Some(delta),
            _ => None,
        })
        .expect("cooldown-only snapshot delta");
    assert!(cooldown_delta.entities.is_empty());
    assert_eq!(cooldown_delta.spell_cooldown_ticks[7], 896);
}

#[test]
fn damage_events_leave_as_mtu_safe_redundant_datagrams_and_dedupe_client_side() {
    // This seed places player one on player zero's south-west firing line,
    // close enough for the deterministic projectile to collide.
    let mut process = process_with_seed(2, RecordingHost::accepting(), 455);
    admit_players(&mut process, 2);
    let cast = input_batch(0, peer(0), 1, vec![cast_command(0, 1, 40_960)]);
    process.ingest_message(cast).expect("cast input");
    let mut scheduler = TickScheduler::new(ManualClock::default(), 32);
    run_tick(&mut process, &mut scheduler);
    process.host_mut().sent.clear();

    for _ in 0..16 {
        run_tick(&mut process, &mut scheduler);
    }
    assert!(
        process
            .state()
            .player(PlayerId::new(1).expect("target"))
            .expect("target state")
            .health
            < 100,
        "the deterministic projectile must hit the target"
    );

    let damage_packets: Vec<&SentPacket> = process
        .host()
        .sent
        .iter()
        .filter(|packet| packet.peer == peer(0))
        .filter(|packet| {
            matches!(
                decode_sent(packet).message,
                Message::EventBatch(ref batch)
                    if batch.events.iter().any(|event| event.kind == EventKind::DAMAGE)
            )
        })
        .collect();
    assert_eq!(
        damage_packets.len(),
        2,
        "current plus previous-tick redundancy must survive one lost datagram"
    );
    assert!(damage_packets.iter().all(|packet| {
        packet.delivery == DeliveryKind::Datagram && packet.payload.len() <= MAX_DATAGRAM_BYTES
    }));

    let mut feed = AuthoritativeFeedReplica::new(PlayerId::new(0).expect("viewer"));
    let recovered = decode_sent(damage_packets[1]);
    assert_eq!(
        feed.apply(&recovered.message),
        Ok(AuthoritativeFeedOutcome::EventsQueued(2))
    );
    assert_eq!(
        feed.apply(&recovered.message),
        Ok(AuthoritativeFeedOutcome::Obsolete),
        "duplicate redundancy must not replay the hit effect"
    );
    let damage = feed.pop_event().expect("recovered damage event");
    assert_eq!(damage.kind, EventKind::DAMAGE);
    assert!(damage.target.is_some());
    let impact = feed.pop_event().expect("recovered impact event");
    assert_eq!(impact.kind, EventKind::HIT);
    assert_eq!(impact.target, damage.target);
}

#[test]
fn reliable_terrain_deltas_converge_and_full_keyframe_repairs_a_gap() {
    let mut process = process(1, RecordingHost::accepting());
    admit_players(&mut process, 1);
    let viewer = PlayerId::new(0).expect("viewer");
    let mut normal = AuthoritativeFeedReplica::new(viewer);
    let mut missed = AuthoritativeFeedReplica::new(viewer);
    let mut scheduler = TickScheduler::new(ManualClock::default(), 128);

    process
        .ingest_message(input_batch(
            0,
            peer(0),
            1,
            vec![terrain_cast_command(0, 1, 0)],
        ))
        .expect("first cast");
    run_tick(&mut process, &mut scheduler);
    let initial_keyframe = process
        .host()
        .sent
        .iter()
        .filter(|packet| packet.peer == peer(0))
        .map(decode_sent)
        .find(|envelope| matches!(envelope.message, Message::SnapshotKeyframe(_)))
        .expect("initial keyframe");
    normal
        .apply(&initial_keyframe.message)
        .expect("normal initial keyframe");
    missed
        .apply(&initial_keyframe.message)
        .expect("missed initial keyframe");
    process.host_mut().sent.clear();

    while process
        .state()
        .terrain()
        .first()
        .map_or(true, |chunk| chunk.revision < 1)
    {
        run_tick(&mut process, &mut scheduler);
    }
    let first_delta = process
        .host()
        .sent
        .iter()
        .filter(|packet| packet.peer == peer(0) && packet.delivery == DeliveryKind::Reliable)
        .map(decode_sent)
        .find(|envelope| {
            matches!(
                envelope.message,
                Message::TerrainDelta(ref delta) if delta.new_revision == 1
            )
        })
        .expect("first reliable terrain delta");
    let Message::TerrainDelta(first_wire_delta) = &first_delta.message else {
        unreachable!("selected terrain delta")
    };
    assert!(first_wire_delta
        .operations
        .iter()
        .all(|operation| operation.kind == TerrainOpKind::HeightDelta));
    assert_eq!(
        normal.apply(&first_delta.message),
        Ok(AuthoritativeFeedOutcome::TerrainApplied)
    );
    assert!(
        process
            .host()
            .sent
            .iter()
            .filter(|packet| packet.peer == peer(0))
            .map(decode_sent)
            .any(|envelope| matches!(
                envelope.message,
                Message::EventBatch(ref batch)
                    if batch.events.iter().any(
                        |event| event.kind == EventKind::TERRAIN_DEFORMED
                    )
            )),
        "terrain deformation effects must leave the server too"
    );

    while process
        .state()
        .player(viewer)
        .is_some_and(|player| player.max_cooldown_ticks() != 0)
    {
        run_tick(&mut process, &mut scheduler);
    }
    process.host_mut().sent.clear();
    let second_cast_tick = process.state().tick();
    process
        .ingest_message(input_batch(
            0,
            peer(0),
            2,
            vec![terrain_cast_command(second_cast_tick, 2, 0)],
        ))
        .expect("second cast after cooldown");
    run_tick(&mut process, &mut scheduler);
    process.host_mut().sent.clear();
    while process
        .state()
        .terrain()
        .first()
        .is_some_and(|chunk| chunk.revision < 2)
    {
        run_tick(&mut process, &mut scheduler);
    }
    let second_delta = process
        .host()
        .sent
        .iter()
        .filter(|packet| packet.peer == peer(0) && packet.delivery == DeliveryKind::Reliable)
        .map(decode_sent)
        .find(|envelope| {
            matches!(
                envelope.message,
                Message::TerrainDelta(ref delta) if delta.new_revision == 2
            )
        })
        .expect("second reliable terrain delta");
    assert_eq!(
        normal.apply(&second_delta.message),
        Ok(AuthoritativeFeedOutcome::TerrainApplied)
    );
    assert_replica_terrain_matches(&process, &normal);

    let Message::TerrainDelta(delta) = &second_delta.message else {
        unreachable!("selected terrain delta")
    };
    assert_eq!(
        missed.apply(&second_delta.message),
        Err(AuthoritativeFeedError::TerrainRevisionGap {
            x: delta.chunk_x,
            y: delta.chunk_y,
            current: 0,
            base: 1,
            new: 2,
        })
    );
    assert!(missed.resync_requested());

    process
        .request_resync(peer(0))
        .expect("connected viewer can request resync");
    process.host_mut().sent.clear();
    run_tick(&mut process, &mut scheduler);
    let recovery = process
        .host()
        .sent
        .iter()
        .filter(|packet| packet.peer == peer(0) && packet.delivery == DeliveryKind::Reliable)
        .map(decode_sent)
        .find(|envelope| matches!(envelope.message, Message::SnapshotKeyframe(_)))
        .expect("full recovery keyframe");
    assert_eq!(
        missed.apply(&recovery.message),
        Ok(AuthoritativeFeedOutcome::KeyframeApplied)
    );
    assert!(!missed.resync_requested());
    assert_replica_terrain_matches(&process, &missed);
}

#[test]
fn disconnect_transitions_to_bot_then_reconnects_and_expires() {
    let mut process = process(1, RecordingHost::accepting());
    admit_players(&mut process, 1);
    let player = PlayerId::new(0).expect("player");
    assert!(process.disconnect_peer(peer(0)));
    let mut scheduler = TickScheduler::new(ManualClock::default(), 16);

    for _ in 0..BOT_TAKEOVER_AFTER_TICKS {
        run_tick(&mut process, &mut scheduler);
    }
    assert_eq!(
        process.state().player(player).expect("player").controller,
        Controller::Human
    );
    run_tick(&mut process, &mut scheduler);
    assert_eq!(
        process.state().player(player).expect("player").controller,
        Controller::Bot
    );

    let replacement_peer = PeerId(99_999);
    assert_eq!(
        process.admit(replacement_peer, &ticket(0), NOW),
        Err(AdmissionError::TicketReplayed)
    );
    assert_eq!(
        process.admit(replacement_peer, &reconnect_ticket(0, 2), NOW),
        Ok(player)
    );
    assert_eq!(
        process.state().player(player).expect("player").controller,
        Controller::Human
    );
    assert!(process.disconnect_peer(replacement_peer));
    for _ in 0..=RECONNECT_GRACE_TICKS {
        run_tick(&mut process, &mut scheduler);
    }
    let expired = process.state().player(player).expect("slot remains");
    assert_eq!(expired.controller, Controller::Empty);
    assert_eq!(expired.outcome, PlayerOutcome::Defeated);
    assert_eq!(
        process.admit(PeerId(100_000), &reconnect_ticket(0, 3), NOW),
        Err(AdmissionError::ReconnectExpired)
    );
    let result = process
        .seal_match_result([0xa5; 16])
        .expect("expired reservation seals from its terminal snapshot");
    assert_eq!(result.players.len(), 1);
    assert_eq!(result.players[0].player_id, player);
    assert_eq!(result.players[0].team_id, TeamId::new(0).expect("team"));
    assert_eq!(
        result.players[0].outcome,
        aetherloom_protocol::MatchOutcome::Defeated
    );
    assert!(result.players[0].banked_loot.is_empty());
}

#[test]
fn build_pin_health_and_draining_gate_admission_without_stopping_match() {
    let mut process = process(2, RecordingHost::accepting());
    assert_eq!(
        process.admit(peer(0), &[0, 9], NOW),
        Err(AdmissionError::WrongBuild)
    );
    process.update_capacity_signal(CapacitySignal {
        healthy: true,
        content_build_hash: [88; 16],
        available_match_slot: true,
        p99_simulation_ns: 0,
        p99_tick_start_jitter_ns: 0,
    });
    assert_eq!(
        process.admit(peer(0), &ticket(0), NOW),
        Err(AdmissionError::NoHeadroom)
    );
    process.update_capacity_signal(CapacitySignal::ready(build().content_build_hash()));
    assert_eq!(
        process.admit(peer(0), &ticket(0), NOW),
        Ok(PlayerId::new(0).expect("player"))
    );
    process.begin_drain();
    assert_eq!(
        process.admit(peer(1), &ticket(1), NOW),
        Err(AdmissionError::Draining)
    );
    let mut scheduler = TickScheduler::new(ManualClock::default(), 8);
    run_tick(&mut process, &mut scheduler);
    assert_eq!(process.state().tick(), 1);
    assert_eq!(process.health().state, HostState::Draining);
}

#[test]
fn queue_pressure_never_drops_simulation_ticks() {
    let mut process = constrained_process(16, RecordingHost::backpressured());
    admit_players(&mut process, 16);
    let mut scheduler = TickScheduler::new(ManualClock::default(), 16);
    for _ in 0..128 {
        run_tick(&mut process, &mut scheduler);
    }
    assert_eq!(process.state().tick(), 128);
    assert!(process.egress().len() <= 4);
    let stats = process.egress().stats();
    assert!(
        stats.reliable_backpressure
            + stats.coalesced_critical
            + stats.dropped_medium
            + stats.dropped_distant
            > 0
    );
}

#[test]
fn egress_sheds_cosmetics_and_distant_work_before_critical_work() {
    let mut queue = PriorityEgress::new(3, 1_024);
    for class in [
        EgressClass::Cosmetic,
        EgressClass::Distant,
        EgressClass::Medium,
    ] {
        assert!(
            queue
                .enqueue(OutboundPacket {
                    peer: PeerId(1),
                    delivery: DeliveryKind::Datagram,
                    class,
                    server_tick: 0,
                    payload: vec![1; 16],
                    resync_if_dropped: false,
                })
                .accepted
        );
    }
    assert!(
        queue
            .enqueue(OutboundPacket {
                peer: PeerId(1),
                delivery: DeliveryKind::Datagram,
                class: EgressClass::CombatCritical,
                server_tick: 1,
                payload: vec![2; 16],
                resync_if_dropped: true,
            })
            .accepted
    );
    assert_eq!(queue.stats().dropped_cosmetic, 1);
    assert_eq!(queue.stats().coalesced_critical, 0);
}

#[test]
fn reliable_control_uses_reserved_capacity_and_reports_backpressure() {
    let mut queue = PriorityEgress::new(2, 128);
    let reliable = |peer| OutboundPacket {
        peer,
        delivery: DeliveryKind::Reliable,
        class: EgressClass::ReliableControl,
        server_tick: 0,
        payload: vec![3; 32],
        resync_if_dropped: true,
    };
    assert!(queue.enqueue(reliable(PeerId(1))).accepted);
    assert!(queue.enqueue(reliable(PeerId(2))).accepted);
    let pressure = queue.enqueue(reliable(PeerId(3)));
    assert!(!pressure.accepted);
    assert!(pressure.reliable_backpressure);
    assert_eq!(queue.stats().reliable_backpressure, 1);
    assert_eq!(queue.len(), 2);
}

#[test]
fn terminal_peer_errors_do_not_head_of_line_block_other_peers() {
    for closed in [false, true] {
        let bad = PeerId(10);
        let good = PeerId(11);
        let mut host = RecordingHost::accepting();
        if closed {
            host.closed_peers.insert(bad);
        } else {
            host.invalid_peers.insert(bad);
        }
        let mut queue = PriorityEgress::new(4, 1_024);
        for peer in [bad, good] {
            assert!(
                queue
                    .enqueue(OutboundPacket {
                        peer,
                        delivery: DeliveryKind::Reliable,
                        class: EgressClass::ReliableControl,
                        server_tick: 1,
                        payload: vec![peer.0 as u8; 16],
                        resync_if_dropped: true,
                    })
                    .accepted
            );
        }

        assert_eq!(queue.flush(&mut host), Ok(1));
        assert!(queue.is_empty());
        assert_eq!(host.sent.len(), 1);
        assert_eq!(host.sent[0].peer, good);
        assert_eq!(queue.stats().terminal_peer_drops, 1);
    }
}

#[test]
fn sustained_peer_backpressure_does_not_stall_healthy_peers() {
    let slow = PeerId(20);
    let healthy = PeerId(21);
    let mut host = RecordingHost::accepting();
    host.backpressured_peers.insert(slow);
    let mut queue = PriorityEgress::new(16, 4_096);

    assert!(
        queue
            .enqueue(OutboundPacket {
                peer: slow,
                delivery: DeliveryKind::Reliable,
                class: EgressClass::ReliableControl,
                server_tick: 1,
                payload: vec![0x51; 16],
                resync_if_dropped: true,
            })
            .accepted
    );

    for tick in 1..=3 {
        assert!(
            queue
                .enqueue(OutboundPacket {
                    peer: healthy,
                    delivery: DeliveryKind::Datagram,
                    class: EgressClass::CombatCritical,
                    server_tick: tick,
                    payload: vec![tick as u8; 16],
                    resync_if_dropped: false,
                })
                .accepted
        );
        assert_eq!(queue.flush(&mut host), Ok(1));
        assert_eq!(queue.len(), 1, "only the slow peer remains queued");
    }

    assert_eq!(
        host.sent
            .iter()
            .map(|packet| (packet.peer, packet.payload[0]))
            .collect::<Vec<_>>(),
        vec![(healthy, 1), (healthy, 2), (healthy, 3)]
    );
    assert_eq!(queue.stats().transport_backpressure, 3);
}

#[test]
fn disconnect_purges_queued_frames_before_peer_id_reuse() {
    let mut process = process(1, RecordingHost::backpressured());
    admit_players(&mut process, 1);
    let mut scheduler = TickScheduler::new(ManualClock::default(), 8);
    run_tick(&mut process, &mut scheduler);
    assert!(!process.egress().is_empty());

    assert!(process.disconnect_peer(peer(0)));
    assert!(process.egress().is_empty());
    process.host_mut().backpressured = false;
    process
        .admit(peer(0), &reconnect_ticket(0, 2), NOW)
        .expect("same peer id may be rebound after purge");
    run_tick(&mut process, &mut scheduler);

    assert_eq!(process.host().sent.len(), 1);
    let envelope =
        MessageEnvelope::decode_reliable(&process.host().sent[0].payload).expect("fresh keyframe");
    assert_eq!(envelope.metadata.server_tick, 2);
    assert!(matches!(envelope.message, Message::SnapshotKeyframe(_)));
}

#[test]
fn browser_snapshots_run_at_32_hz_while_native_remains_128_hz() {
    let mut browser = process(1, RecordingHost::websocket());
    admit_players(&mut browser, 1);
    assert_eq!(
        browser.replication_profile(peer(0)),
        Some(ReplicationProfile::BrowserCasual32Hz)
    );
    let mut browser_scheduler = TickScheduler::new(ManualClock::default(), 16);
    for sequence in 1..=12_u32 {
        let mut inbound = input(0, peer(0), browser.state().tick(), sequence, 2_047);
        inbound.delivery = DeliveryKind::Reliable;
        browser
            .ingest_message(inbound)
            .expect("logical datagram over WSS");
        run_tick(&mut browser, &mut browser_scheduler);
    }
    let browser_delta_ticks: Vec<u64> = browser
        .host()
        .sent
        .iter()
        .filter(|packet| packet.delivery == DeliveryKind::Datagram)
        .map(|packet| {
            MessageEnvelope::decode_datagram(&packet.payload)
                .expect("browser logical datagram")
                .metadata
                .server_tick
        })
        .collect();
    assert_eq!(browser_delta_ticks, vec![4, 8, 12]);

    let mut native = process(1, RecordingHost::accepting());
    admit_players(&mut native, 1);
    assert_eq!(
        native.replication_profile(peer(0)),
        Some(ReplicationProfile::NativeCompetitive128Hz)
    );
    let mut native_scheduler = TickScheduler::new(ManualClock::default(), 16);
    for sequence in 1..=5_u32 {
        native
            .ingest_message(input(0, peer(0), native.state().tick(), sequence, 2_047))
            .expect("native datagram");
        run_tick(&mut native, &mut native_scheduler);
    }
    let native_delta_ticks: Vec<u64> = native
        .host()
        .sent
        .iter()
        .filter(|packet| packet.delivery == DeliveryKind::Datagram)
        .map(|packet| {
            MessageEnvelope::decode_datagram(&packet.payload)
                .expect("native datagram")
                .metadata
                .server_tick
        })
        .collect();
    assert_eq!(native_delta_ticks, vec![2, 3, 4, 5]);
}

#[test]
fn spatial_grid_merge_is_independent_of_worker_completion_order() {
    let entries = vec![
        (EntityId::new(9, 1).expect("entity"), [2_100, 0, -2_100]),
        (EntityId::new(2, 1).expect("entity"), [-100, 0, 100]),
        (EntityId::new(7, 1).expect("entity"), [500, 0, 500]),
        (EntityId::new(1, 1).expect("entity"), [-500, 0, -500]),
    ];
    let mut forward = DeterministicSpatialGrid::default();
    forward.rebuild_entries(entries.iter().copied());
    let mut reverse = DeterministicSpatialGrid::default();
    reverse.rebuild_entries(entries.iter().rev().copied());

    assert_eq!(
        forward.cells().collect::<Vec<_>>(),
        reverse.cells().collect::<Vec<_>>()
    );
    assert_eq!(
        forward.query_radius([0, 0, 0], 1_000),
        reverse.query_radius([0, 0, 0], 1_000)
    );
    assert_eq!(
        forward.query_radius([0, 0, 0], 1_000),
        vec![
            EntityId::new(1, 1).expect("entity"),
            EntityId::new(2, 1).expect("entity"),
            EntityId::new(7, 1).expect("entity"),
        ]
    );
}

#[test]
fn all_128_player_gameplay_datagrams_respect_the_path_mtu() {
    let mut process = process(128, RecordingHost::accepting());
    admit_players(&mut process, 128);
    let mut scheduler = TickScheduler::new(ManualClock::default(), 32);
    run_tick(&mut process, &mut scheduler);

    for tick in 1..=16 {
        for raw in 0..128_u16 {
            let raw = raw as u8;
            process
                .ingest_message(input(raw, peer(raw), tick, tick as u32, 2_047))
                .expect("valid player input");
        }
        run_tick(&mut process, &mut scheduler);
    }

    let sent = &process.host().sent;
    assert!(!sent.is_empty());
    let mut reliable_keyframes = 0;
    let mut peer_zero_entities = std::collections::BTreeSet::new();
    for packet in sent {
        if packet.delivery == DeliveryKind::Datagram {
            assert!(packet.payload.len() <= MAX_DATAGRAM_BYTES);
            let envelope =
                MessageEnvelope::decode_datagram(&packet.payload).expect("valid datagram");
            let Message::SnapshotDelta(delta) = envelope.message else {
                panic!("gameplay datagrams must be snapshot deltas");
            };
            assert!(delta.entities.len() <= 32);
            if packet.peer == peer(0) {
                peer_zero_entities
                    .extend(delta.entities.into_iter().map(|entity| entity.entity_id));
            }
        } else {
            let envelope =
                MessageEnvelope::decode_reliable(&packet.payload).expect("valid reliable frame");
            if matches!(envelope.message, Message::SnapshotKeyframe(_)) {
                reliable_keyframes += 1;
            }
        }
    }
    assert_eq!(
        reliable_keyframes, 128,
        "only the initial keyframe per viewer is allowed; clustered/overfull deltas must defer fairly"
    );
    assert!(
        peer_zero_entities.len() > 32,
        "aged deferred entries must rotate through the bounded datagram budget"
    );
}
