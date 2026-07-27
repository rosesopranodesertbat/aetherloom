use aetherloom_client::{
    ClientAuthoritativeSession, ClientSessionError, ClientSessionOutcome,
    IncomingServerMessage,
};
use aetherloom_protocol::{
    Delivery, EntityId, EntityState, EnvelopeMetadata, EventBatch, EventKind,
    GameEvent, Message, MessageEnvelope, PlayerId, SnapshotDelta, SnapshotId,
    SnapshotKeyframe,
};

const BUILD: [u8; 16] = [7; 16];
const EPOCH: u64 = 44;

fn viewer() -> PlayerId {
    PlayerId::new(0).expect("viewer")
}

fn envelope(sequence: u32, message: Message) -> MessageEnvelope {
    MessageEnvelope::new(
        EnvelopeMetadata::new(BUILD, EPOCH, sequence, u64::from(sequence), 0),
        message,
    )
}

fn entity(x: i32) -> EntityState {
    EntityState {
        entity_id: EntityId::new(0, 1).expect("entity"),
        archetype: 1,
        owner: Some(viewer()),
        team: None,
        position_cm: [x, 0, -10],
        velocity_cm_per_tick: [0; 3],
        yaw: 0,
        pitch: 0,
        health: 100,
        flags: 0,
    }
}

fn keyframe(sequence: u32) -> IncomingServerMessage {
    IncomingServerMessage {
        delivery: Delivery::Reliable,
        envelope: envelope(
            sequence,
            Message::SnapshotKeyframe(SnapshotKeyframe {
                snapshot_id: SnapshotId::new(sequence),
                viewer: viewer(),
                acknowledged_input_sequence: 0,
                entities: vec![entity(25)],
                terrain_revisions: Vec::new(),
                terrain_chunks: Vec::new(),
            }),
        ),
    }
}

fn delta(sequence: u32) -> IncomingServerMessage {
    IncomingServerMessage {
        delivery: Delivery::Datagram,
        envelope: envelope(
            sequence,
            Message::SnapshotDelta(SnapshotDelta {
                snapshot_id: SnapshotId::new(2),
                baseline_id: SnapshotId::new(1),
                viewer: viewer(),
                acknowledged_input_sequence: 0,
                entities: vec![entity(50)],
                removed_entities: Vec::new(),
            }),
        ),
    }
}

fn damage(sequence: u32) -> IncomingServerMessage {
    IncomingServerMessage {
        delivery: Delivery::Datagram,
        envelope: envelope(
            sequence,
            Message::EventBatch(EventBatch {
                events: vec![GameEvent {
                    event_id: u64::from(sequence) + 1,
                    tick: u64::from(sequence),
                    kind: EventKind::DAMAGE,
                    actor: None,
                    target: None,
                    data: [5, 95, 0, 0],
                }],
            }),
        ),
    }
}

#[test]
fn session_pins_build_and_epoch_before_either_replica_mutates() {
    let mut session = ClientAuthoritativeSession::new(viewer(), BUILD, EPOCH);
    let mut prior_match = damage(500);
    prior_match.envelope.metadata.match_epoch = EPOCH - 1;
    assert_eq!(
        session.apply(&prior_match),
        Err(ClientSessionError::WrongEpoch {
            expected: EPOCH,
            received: EPOCH - 1,
        })
    );
    assert_eq!(session.feed_replica().pending_event_count(), 0);
    assert!(session.entity_replica().entities().is_empty());

    let mut wrong_build = keyframe(1);
    wrong_build.envelope.metadata.content_build_hash = [8; 16];
    assert_eq!(
        session.apply(&wrong_build),
        Err(ClientSessionError::WrongBuild {
            expected: BUILD,
            received: [8; 16],
        })
    );
    assert!(session.entity_replica().entities().is_empty());

    assert_eq!(
        session.apply(&keyframe(1)),
        Ok(ClientSessionOutcome::KeyframeApplied)
    );
    assert_eq!(session.entity_replica().entities().len(), 1);
}

#[test]
fn datagram_replay_window_is_bounded_separately_from_reliable_delivery() {
    let mut session = ClientAuthoritativeSession::new(viewer(), BUILD, EPOCH);
    assert_eq!(
        session.apply(&keyframe(100)),
        Ok(ClientSessionOutcome::KeyframeApplied)
    );
    assert_eq!(
        session.apply(&damage(2)),
        Ok(ClientSessionOutcome::Feed(
            aetherloom_client::AuthoritativeFeedOutcome::EventsQueued(1)
        ))
    );
    assert_eq!(
        session.apply(&damage(2)),
        Ok(ClientSessionOutcome::ObsoleteEnvelope)
    );
    assert_eq!(session.feed_replica().pending_event_count(), 1);
}

#[test]
fn wrong_delivery_is_rejected_without_consuming_its_sequence() {
    let mut session = ClientAuthoritativeSession::new(viewer(), BUILD, EPOCH);
    let mut wrong = keyframe(10);
    wrong.delivery = Delivery::Datagram;
    assert_eq!(
        session.apply(&wrong),
        Err(ClientSessionError::WrongDelivery(Delivery::Datagram))
    );

    assert!(matches!(
        session.apply(&damage(9)),
        Ok(ClientSessionOutcome::Feed(_))
    ));
}

#[test]
fn prioritized_event_cannot_obsolete_an_older_snapshot_lane() {
    let mut session = ClientAuthoritativeSession::new(viewer(), BUILD, EPOCH);
    assert_eq!(
        session.apply(&keyframe(1)),
        Ok(ClientSessionOutcome::KeyframeApplied)
    );

    // Dedicated egress may send the newer combat-critical event before the
    // already-queued medium/distant snapshot.
    assert!(matches!(
        session.apply(&damage(11)),
        Ok(ClientSessionOutcome::Feed(_))
    ));
    assert_eq!(
        session.apply(&delta(10)),
        Ok(ClientSessionOutcome::EntitySnapshotApplied)
    );
    assert_eq!(
        session.entity_replica().entities()[0].position_cm,
        [50, 0, -10]
    );
}
