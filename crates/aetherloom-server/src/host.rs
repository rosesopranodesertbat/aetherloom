//! Match-host adapters for native QUIC and Cloudflare WebSocket deployments.

use crate::transport::{
    DeliveryKind, DisconnectReason, InboundMessage, NetworkTransport, PeerId,
    TransportError, MAX_GAMEPLAY_DATAGRAM_BYTES,
};
use std::error::Error;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostKind {
    DedicatedQuic,
    CloudflareWebSocket,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostState {
    AcceptingPlayers,
    Draining,
    Stopped,
}

#[derive(Debug, Eq, PartialEq)]
pub enum HostError {
    MissingReliableMessages,
    MissingUnreliableDatagrams,
    DatagramMtuTooSmall { actual: usize, required: usize },
    Stopped,
    Transport(TransportError),
}

impl fmt::Display for HostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingReliableMessages => {
                formatter.write_str("host transport must support reliable messages")
            }
            Self::MissingUnreliableDatagrams => {
                formatter.write_str("competitive host transport must support datagrams")
            }
            Self::DatagramMtuTooSmall { actual, required } => write!(
                formatter,
                "transport datagram payload is {actual} bytes; {required} required"
            ),
            Self::Stopped => formatter.write_str("match host is stopped"),
            Self::Transport(error) => error.fmt(formatter),
        }
    }
}

impl Error for HostError {}

impl From<TransportError> for HostError {
    fn from(error: TransportError) -> Self {
        Self::Transport(error)
    }
}

/// Hosting boundary consumed by the authoritative match runtime.
///
/// Gameplay messages map to QUIC datagrams on competitive hosts and reliable
/// WebSocket messages on Cloudflare browser hosts. Control messages are always
/// reliable.
pub trait MatchHost {
    fn kind(&self) -> HostKind;
    fn state(&self) -> HostState;

    fn accepts_new_players(&self) -> bool {
        self.state() == HostState::AcceptingPlayers
    }

    fn begin_drain(&mut self);
    fn stop(&mut self);
    fn poll_receive(&mut self) -> Result<Option<InboundMessage>, HostError>;
    fn try_send_gameplay(&mut self, peer: PeerId, payload: &[u8])
        -> Result<(), HostError>;
    fn try_send_reliable(&mut self, peer: PeerId, payload: &[u8])
        -> Result<(), HostError>;
    fn disconnect(
        &mut self,
        peer: PeerId,
        reason: DisconnectReason,
    ) -> Result<(), HostError>;
}

/// Competitive host boundary.
///
/// A platform crate supplies a real `NetworkTransport` backed by a QUIC library
/// and production TLS configuration. This crate only validates capabilities and
/// chooses datagram versus reliable delivery; it contains no placeholder crypto.
#[derive(Debug)]
pub struct DedicatedQuicHost<T> {
    transport: T,
    state: HostState,
}

impl<T: NetworkTransport> DedicatedQuicHost<T> {
    pub fn new(transport: T) -> Result<Self, HostError> {
        let capabilities = transport.capabilities();
        if !capabilities.reliable_messages {
            return Err(HostError::MissingReliableMessages);
        }
        if !capabilities.unreliable_datagrams {
            return Err(HostError::MissingUnreliableDatagrams);
        }
        if capabilities.max_datagram_payload < MAX_GAMEPLAY_DATAGRAM_BYTES {
            return Err(HostError::DatagramMtuTooSmall {
                actual: capabilities.max_datagram_payload,
                required: MAX_GAMEPLAY_DATAGRAM_BYTES,
            });
        }

        Ok(Self {
            transport,
            state: HostState::AcceptingPlayers,
        })
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    pub fn into_transport(self) -> T {
        self.transport
    }

    fn ensure_running(&self) -> Result<(), HostError> {
        if self.state == HostState::Stopped {
            Err(HostError::Stopped)
        } else {
            Ok(())
        }
    }
}

impl<T: NetworkTransport> MatchHost for DedicatedQuicHost<T> {
    fn kind(&self) -> HostKind {
        HostKind::DedicatedQuic
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
        self.ensure_running()?;
        self.transport.poll_receive().map_err(Into::into)
    }

    fn try_send_gameplay(
        &mut self,
        peer: PeerId,
        payload: &[u8],
    ) -> Result<(), HostError> {
        self.ensure_running()?;
        self.transport
            .try_send(peer, DeliveryKind::Datagram, payload)
            .map_err(Into::into)
    }

    fn try_send_reliable(
        &mut self,
        peer: PeerId,
        payload: &[u8],
    ) -> Result<(), HostError> {
        self.ensure_running()?;
        self.transport
            .try_send(peer, DeliveryKind::Reliable, payload)
            .map_err(Into::into)
    }

    fn disconnect(
        &mut self,
        peer: PeerId,
        reason: DisconnectReason,
    ) -> Result<(), HostError> {
        self.ensure_running()?;
        self.transport.disconnect(peer, reason).map_err(Into::into)
    }
}

/// Cloudflare browser-host boundary.
///
/// The Worker/Durable Object adapter implements `NetworkTransport` for binary
/// WebSocket sessions. Both gameplay and control traffic intentionally map to
/// reliable messages because browser WebSockets have no datagram mode.
#[derive(Debug)]
pub struct CloudflareWssHost<T> {
    transport: T,
    state: HostState,
}

impl<T: NetworkTransport> CloudflareWssHost<T> {
    pub fn new(transport: T) -> Result<Self, HostError> {
        if !transport.capabilities().reliable_messages {
            return Err(HostError::MissingReliableMessages);
        }
        Ok(Self {
            transport,
            state: HostState::AcceptingPlayers,
        })
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    pub fn into_transport(self) -> T {
        self.transport
    }

    fn ensure_running(&self) -> Result<(), HostError> {
        if self.state == HostState::Stopped {
            Err(HostError::Stopped)
        } else {
            Ok(())
        }
    }
}

impl<T: NetworkTransport> MatchHost for CloudflareWssHost<T> {
    fn kind(&self) -> HostKind {
        HostKind::CloudflareWebSocket
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
        self.ensure_running()?;
        self.transport.poll_receive().map_err(Into::into)
    }

    fn try_send_gameplay(
        &mut self,
        peer: PeerId,
        payload: &[u8],
    ) -> Result<(), HostError> {
        self.ensure_running()?;
        self.transport
            .try_send(peer, DeliveryKind::Reliable, payload)
            .map_err(Into::into)
    }

    fn try_send_reliable(
        &mut self,
        peer: PeerId,
        payload: &[u8],
    ) -> Result<(), HostError> {
        self.ensure_running()?;
        self.transport
            .try_send(peer, DeliveryKind::Reliable, payload)
            .map_err(Into::into)
    }

    fn disconnect(
        &mut self,
        peer: PeerId,
        reason: DisconnectReason,
    ) -> Result<(), HostError> {
        self.ensure_running()?;
        self.transport.disconnect(peer, reason).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{LoopbackTransport, TransportCapabilities};

    #[test]
    fn dedicated_host_maps_gameplay_to_datagrams() {
        let (server, mut client) =
            LoopbackTransport::pair(PeerId(1), PeerId(2), 4).expect("loopback pair");
        let mut host = DedicatedQuicHost::new(server).expect("QUIC capabilities");

        host.try_send_gameplay(PeerId(2), b"input")
            .expect("gameplay send");
        host.try_send_reliable(PeerId(2), b"keyframe")
            .expect("reliable send");

        assert_eq!(
            client.poll_receive().expect("first receive").unwrap().delivery,
            DeliveryKind::Datagram
        );
        assert_eq!(
            client.poll_receive().expect("second receive").unwrap().delivery,
            DeliveryKind::Reliable
        );
    }

    #[test]
    fn websocket_host_maps_all_messages_to_reliable_delivery() {
        let (server, mut client) =
            LoopbackTransport::pair(PeerId(1), PeerId(2), 2).expect("loopback pair");
        let mut host = CloudflareWssHost::new(server).expect("WebSocket capabilities");

        host.try_send_gameplay(PeerId(2), b"batched input")
            .expect("gameplay send");
        assert_eq!(
            client.poll_receive().expect("receive").unwrap().delivery,
            DeliveryKind::Reliable
        );
    }

    #[test]
    fn draining_rejects_admission_but_keeps_active_match_connected() {
        let (server, _client) =
            LoopbackTransport::pair(PeerId(1), PeerId(2), 1).expect("loopback pair");
        let mut host = DedicatedQuicHost::new(server).expect("host");

        host.begin_drain();
        assert_eq!(host.state(), HostState::Draining);
        assert!(!host.accepts_new_players());
        host.try_send_gameplay(PeerId(2), b"existing player")
            .expect("active connections continue while draining");

        host.stop();
        assert_eq!(
            host.try_send_gameplay(PeerId(2), b"stopped"),
            Err(HostError::Stopped)
        );
    }

    #[derive(Debug)]
    struct WebSocketOnly;

    impl NetworkTransport for WebSocketOnly {
        fn capabilities(&self) -> TransportCapabilities {
            TransportCapabilities::websocket()
        }

        fn poll_receive(&mut self) -> Result<Option<InboundMessage>, TransportError> {
            Ok(None)
        }

        fn try_send(
            &mut self,
            _peer: PeerId,
            delivery: DeliveryKind,
            _payload: &[u8],
        ) -> Result<(), TransportError> {
            if delivery == DeliveryKind::Reliable {
                Ok(())
            } else {
                Err(TransportError::UnsupportedDelivery(delivery))
            }
        }

        fn disconnect(
            &mut self,
            _peer: PeerId,
            _reason: DisconnectReason,
        ) -> Result<(), TransportError> {
            Ok(())
        }
    }

    #[test]
    fn dedicated_host_fails_fast_without_datagrams() {
        assert_eq!(
            DedicatedQuicHost::new(WebSocketOnly).unwrap_err(),
            HostError::MissingUnreliableDatagrams
        );
        CloudflareWssHost::new(WebSocketOnly).expect("valid WebSocket host");
    }
}
