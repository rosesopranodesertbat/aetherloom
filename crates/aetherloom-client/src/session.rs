use aetherloom_core::ClientReplica;
use aetherloom_protocol::{
    Delivery, Message, MessageEnvelope, PlayerId, ValidationError,
};

use crate::{
    apply_wire_snapshot, AuthoritativeFeedError, AuthoritativeFeedOutcome,
    AuthoritativeFeedReplica, WireSnapshotError,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncomingServerMessage {
    /// Logical protocol delivery class. Browser WebSocket adapters still mark
    /// snapshot/event frames as datagrams when they decode datagram envelopes,
    /// even though the underlying WebSocket is reliable.
    pub delivery: Delivery,
    pub envelope: MessageEnvelope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientSessionOutcome {
    ObsoleteEnvelope,
    EntitySnapshotApplied,
    EntitySnapshotObsolete,
    KeyframeApplied,
    Feed(AuthoritativeFeedOutcome),
    MatchResultReceived,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientSessionError {
    WrongBuild {
        expected: [u8; 16],
        received: [u8; 16],
    },
    WrongEpoch {
        expected: u64,
        received: u64,
    },
    UnexpectedServerMessage,
    WrongDelivery(Delivery),
    Validation(ValidationError),
    WireSnapshot(WireSnapshotError),
    AuthoritativeFeed(AuthoritativeFeedError),
}

/// Session-pinned entry point for all authoritative server messages.
///
/// The build and match epoch are checked before either entity or terrain/event
/// replica can mutate. Snapshot datagrams, event datagrams, and the reliable
/// stream keep independent replay windows because priority egress can reorder
/// logical datagram lanes and QUIC does not order datagrams against streams.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientAuthoritativeSession {
    expected_content_build_hash: [u8; 16],
    expected_match_epoch: u64,
    latest_snapshot_datagram_sequence: Option<u32>,
    latest_event_datagram_sequence: Option<u32>,
    latest_reliable_sequence: Option<u32>,
    entity_replica: ClientReplica,
    feed_replica: AuthoritativeFeedReplica,
}

impl ClientAuthoritativeSession {
    pub fn new(
        viewer: PlayerId,
        expected_content_build_hash: [u8; 16],
        expected_match_epoch: u64,
    ) -> Self {
        Self {
            expected_content_build_hash,
            expected_match_epoch,
            latest_snapshot_datagram_sequence: None,
            latest_event_datagram_sequence: None,
            latest_reliable_sequence: None,
            entity_replica: ClientReplica::new(viewer),
            feed_replica: AuthoritativeFeedReplica::new(viewer),
        }
    }

    pub fn entity_replica(&self) -> &ClientReplica {
        &self.entity_replica
    }

    pub fn entity_replica_mut(&mut self) -> &mut ClientReplica {
        &mut self.entity_replica
    }

    pub fn feed_replica(&self) -> &AuthoritativeFeedReplica {
        &self.feed_replica
    }

    pub fn feed_replica_mut(&mut self) -> &mut AuthoritativeFeedReplica {
        &mut self.feed_replica
    }

    pub const fn expected_content_build_hash(&self) -> [u8; 16] {
        self.expected_content_build_hash
    }

    pub const fn expected_match_epoch(&self) -> u64 {
        self.expected_match_epoch
    }

    pub fn apply(
        &mut self,
        incoming: &IncomingServerMessage,
    ) -> Result<ClientSessionOutcome, ClientSessionError> {
        let metadata = incoming.envelope.metadata;
        if metadata.content_build_hash != self.expected_content_build_hash {
            return Err(ClientSessionError::WrongBuild {
                expected: self.expected_content_build_hash,
                received: metadata.content_build_hash,
            });
        }
        if metadata.match_epoch != self.expected_match_epoch {
            return Err(ClientSessionError::WrongEpoch {
                expected: self.expected_match_epoch,
                received: metadata.match_epoch,
            });
        }
        incoming
            .envelope
            .message
            .validate()
            .map_err(ClientSessionError::Validation)?;
        validate_server_delivery(incoming.delivery, &incoming.envelope.message)?;

        let latest = match (&incoming.delivery, &incoming.envelope.message) {
            (Delivery::Datagram, Message::SnapshotDelta(_)) => {
                self.latest_snapshot_datagram_sequence
            }
            (Delivery::Datagram, Message::EventBatch(_)) => {
                self.latest_event_datagram_sequence
            }
            (Delivery::Reliable, _) => self.latest_reliable_sequence,
            _ => None,
        };
        if latest.is_some_and(|previous| {
            !serial_is_newer(metadata.sequence, previous)
        }) {
            return Ok(ClientSessionOutcome::ObsoleteEnvelope);
        }

        let outcome = match &incoming.envelope.message {
            Message::SnapshotKeyframe(_) => {
                // A keyframe replaces both entity and terrain state. Stage the
                // bounded replicas so a local capacity policy cannot leave
                // half of the keyframe visible.
                let mut staged_entities = self.entity_replica.clone();
                let mut staged_feed = self.feed_replica.clone();
                let entity_applied =
                    apply_wire_snapshot(&mut staged_entities, &incoming.envelope)
                        .map_err(ClientSessionError::WireSnapshot)?;
                staged_feed
                    .apply(&incoming.envelope.message)
                    .map_err(ClientSessionError::AuthoritativeFeed)?;
                self.entity_replica = staged_entities;
                self.feed_replica = staged_feed;
                if entity_applied {
                    Ok(ClientSessionOutcome::KeyframeApplied)
                } else {
                    Ok(ClientSessionOutcome::EntitySnapshotObsolete)
                }
            }
            Message::SnapshotDelta(_) => {
                if apply_wire_snapshot(&mut self.entity_replica, &incoming.envelope)
                    .map_err(ClientSessionError::WireSnapshot)?
                {
                    Ok(ClientSessionOutcome::EntitySnapshotApplied)
                } else {
                    Ok(ClientSessionOutcome::EntitySnapshotObsolete)
                }
            }
            Message::EventBatch(_) | Message::TerrainDelta(_) => self
                .feed_replica
                .apply(&incoming.envelope.message)
                .map(ClientSessionOutcome::Feed)
                .map_err(ClientSessionError::AuthoritativeFeed),
            Message::MatchResult(_) => Ok(ClientSessionOutcome::MatchResultReceived),
            Message::InputBatch(_) => {
                Err(ClientSessionError::UnexpectedServerMessage)
            }
        }?;
        match (&incoming.delivery, &incoming.envelope.message) {
            (Delivery::Datagram, Message::SnapshotDelta(_)) => {
                self.latest_snapshot_datagram_sequence = Some(metadata.sequence);
            }
            (Delivery::Datagram, Message::EventBatch(_)) => {
                self.latest_event_datagram_sequence = Some(metadata.sequence);
            }
            (Delivery::Reliable, _) => {
                self.latest_reliable_sequence = Some(metadata.sequence);
            }
            _ => {}
        }
        Ok(outcome)
    }
}

fn validate_server_delivery(
    delivery: Delivery,
    message: &Message,
) -> Result<(), ClientSessionError> {
    let valid = match delivery {
        Delivery::Datagram => {
            matches!(message, Message::SnapshotDelta(_) | Message::EventBatch(_))
        }
        Delivery::Reliable => matches!(
            message,
            Message::SnapshotKeyframe(_)
                | Message::EventBatch(_)
                | Message::TerrainDelta(_)
                | Message::MatchResult(_)
        ),
    };
    if matches!(message, Message::InputBatch(_)) {
        Err(ClientSessionError::UnexpectedServerMessage)
    } else if valid {
        Ok(())
    } else {
        Err(ClientSessionError::WrongDelivery(delivery))
    }
}

fn serial_is_newer(received: u32, previous: u32) -> bool {
    let distance = received.wrapping_sub(previous);
    distance != 0 && distance < (1_u32 << 31)
}
