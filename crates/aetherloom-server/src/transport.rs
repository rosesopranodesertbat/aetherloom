//! Transport-neutral message types and an in-process loopback implementation.

use crate::queue::{
    bounded_channel, BoundedReceiver, BoundedSender, QueueConfigError, QueueReceiveError,
    QueueSendError,
};
use std::error::Error;
use std::fmt;

/// Conservative payload ceiling that avoids Internet-path fragmentation.
pub const MAX_GAMEPLAY_DATAGRAM_BYTES: usize = aetherloom_protocol::MAX_DATAGRAM_BYTES;

/// Stable connection identity assigned by the hosting adapter.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PeerId(pub u64);

/// Delivery semantics requested from the underlying transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryKind {
    /// Loss, duplication, and reordering are permitted.
    Datagram,
    /// Ordered/reliable delivery supplied by a stream or WebSocket.
    Reliable,
}

/// A received binary message. `peer` is always the source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InboundMessage {
    pub peer: PeerId,
    pub delivery: DeliveryKind,
    pub payload: Vec<u8>,
}

/// Transport features used to validate a host adapter at startup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransportCapabilities {
    pub reliable_messages: bool,
    pub unreliable_datagrams: bool,
    pub max_datagram_payload: usize,
}

impl TransportCapabilities {
    pub const fn quic_compatible(max_datagram_payload: usize) -> Self {
        Self {
            reliable_messages: true,
            unreliable_datagrams: true,
            max_datagram_payload,
        }
    }

    pub const fn websocket() -> Self {
        Self {
            reliable_messages: true,
            unreliable_datagrams: false,
            max_datagram_payload: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisconnectReason {
    ClientClosed,
    ReconnectExpired,
    ProtocolViolation,
    ServerDraining,
    ServerShutdown,
}

#[derive(Debug, Eq, PartialEq)]
pub enum TransportError {
    Backpressure,
    Closed,
    InvalidPeer(PeerId),
    UnsupportedDelivery(DeliveryKind),
    DatagramTooLarge { actual: usize, maximum: usize },
    InvalidConfiguration(QueueConfigError),
    Backend(String),
}

impl fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backpressure => formatter.write_str("transport queue is full"),
            Self::Closed => formatter.write_str("transport is closed"),
            Self::InvalidPeer(peer) => write!(formatter, "unknown peer {}", peer.0),
            Self::UnsupportedDelivery(delivery) => {
                write!(formatter, "transport does not support {delivery:?}")
            }
            Self::DatagramTooLarge { actual, maximum } => {
                write!(formatter, "datagram is {actual} bytes; maximum is {maximum}")
            }
            Self::InvalidConfiguration(error) => error.fmt(formatter),
            Self::Backend(message) => formatter.write_str(message),
        }
    }
}

impl Error for TransportError {}

impl From<QueueConfigError> for TransportError {
    fn from(error: QueueConfigError) -> Self {
        Self::InvalidConfiguration(error)
    }
}

/// Adapter boundary for QUIC, WebSocket, console networking, and local play.
///
/// Production implementations own their TLS/authentication setup outside this
/// trait. The server runtime receives an already-authenticated transport and
/// never attempts to invent certificates or cryptographic policy.
pub trait NetworkTransport {
    fn capabilities(&self) -> TransportCapabilities;

    /// Returns immediately with a message, no message, or a transport failure.
    fn poll_receive(&mut self) -> Result<Option<InboundMessage>, TransportError>;

    /// Enqueues a message without blocking the simulation thread.
    fn try_send(
        &mut self,
        peer: PeerId,
        delivery: DeliveryKind,
        payload: &[u8],
    ) -> Result<(), TransportError>;

    fn disconnect(
        &mut self,
        peer: PeerId,
        reason: DisconnectReason,
    ) -> Result<(), TransportError>;
}

/// One endpoint of a bounded in-process transport pair.
///
/// This is the transport used by the offline campaign, deterministic tests, and
/// server/client integration tests. It deliberately models backpressure.
#[derive(Debug)]
pub struct LoopbackTransport {
    local_peer: PeerId,
    remote_peer: PeerId,
    outbound: Option<BoundedSender<InboundMessage>>,
    inbound: BoundedReceiver<InboundMessage>,
}

impl LoopbackTransport {
    pub fn pair(
        first: PeerId,
        second: PeerId,
        capacity: usize,
    ) -> Result<(Self, Self), TransportError> {
        let (first_to_second_tx, first_to_second_rx) = bounded_channel(capacity)?;
        let (second_to_first_tx, second_to_first_rx) = bounded_channel(capacity)?;

        Ok((
            Self {
                local_peer: first,
                remote_peer: second,
                outbound: Some(first_to_second_tx),
                inbound: second_to_first_rx,
            },
            Self {
                local_peer: second,
                remote_peer: first,
                outbound: Some(second_to_first_tx),
                inbound: first_to_second_rx,
            },
        ))
    }

    pub const fn local_peer(&self) -> PeerId {
        self.local_peer
    }

    pub const fn remote_peer(&self) -> PeerId {
        self.remote_peer
    }

    pub fn close(&mut self) {
        self.outbound = None;
    }
}

impl NetworkTransport for LoopbackTransport {
    fn capabilities(&self) -> TransportCapabilities {
        TransportCapabilities::quic_compatible(MAX_GAMEPLAY_DATAGRAM_BYTES)
    }

    fn poll_receive(&mut self) -> Result<Option<InboundMessage>, TransportError> {
        match self.inbound.try_receive() {
            Ok(message) => Ok(Some(message)),
            Err(QueueReceiveError::Empty) => Ok(None),
            Err(QueueReceiveError::Disconnected) => Err(TransportError::Closed),
        }
    }

    fn try_send(
        &mut self,
        peer: PeerId,
        delivery: DeliveryKind,
        payload: &[u8],
    ) -> Result<(), TransportError> {
        if peer != self.remote_peer {
            return Err(TransportError::InvalidPeer(peer));
        }
        if delivery == DeliveryKind::Datagram
            && payload.len() > MAX_GAMEPLAY_DATAGRAM_BYTES
        {
            return Err(TransportError::DatagramTooLarge {
                actual: payload.len(),
                maximum: MAX_GAMEPLAY_DATAGRAM_BYTES,
            });
        }

        let sender = self.outbound.as_ref().ok_or(TransportError::Closed)?;
        let message = InboundMessage {
            peer: self.local_peer,
            delivery,
            payload: payload.to_vec(),
        };
        sender.try_send(message).map_err(|error| match error {
            QueueSendError::Full(_) => TransportError::Backpressure,
            QueueSendError::Disconnected(_) => TransportError::Closed,
        })
    }

    fn disconnect(
        &mut self,
        peer: PeerId,
        _reason: DisconnectReason,
    ) -> Result<(), TransportError> {
        if peer != self.remote_peer {
            return Err(TransportError::InvalidPeer(peer));
        }
        self.close();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_preserves_source_and_delivery_kind() {
        let (mut client, mut server) =
            LoopbackTransport::pair(PeerId(1), PeerId(2), 2).expect("loopback pair");

        client
            .try_send(PeerId(2), DeliveryKind::Datagram, b"input")
            .expect("send");
        assert_eq!(
            server.poll_receive().expect("receive"),
            Some(InboundMessage {
                peer: PeerId(1),
                delivery: DeliveryKind::Datagram,
                payload: b"input".to_vec(),
            })
        );
    }

    #[test]
    fn loopback_enforces_mtu_and_backpressure() {
        let (mut first, _second) =
            LoopbackTransport::pair(PeerId(1), PeerId(2), 1).expect("loopback pair");

        let oversized = vec![0; MAX_GAMEPLAY_DATAGRAM_BYTES + 1];
        assert_eq!(
            first.try_send(PeerId(2), DeliveryKind::Datagram, &oversized),
            Err(TransportError::DatagramTooLarge {
                actual: oversized.len(),
                maximum: MAX_GAMEPLAY_DATAGRAM_BYTES,
            })
        );

        first
            .try_send(PeerId(2), DeliveryKind::Reliable, b"first")
            .expect("first item fits");
        assert_eq!(
            first.try_send(PeerId(2), DeliveryKind::Reliable, b"second"),
            Err(TransportError::Backpressure)
        );
    }
}
