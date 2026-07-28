//! Native QUIC networking for the Aetherloom dedicated-server host.
//!
//! Async socket I/O runs on an owned Tokio thread. The simulation-facing
//! [`NetworkTransport`] implementation only performs bounded `try_send` and
//! `try_recv` operations, so a slow peer cannot block a 128 Hz simulation tick.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::error::Error;
use std::fmt;
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::ops::Bound::{Excluded, Unbounded};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use aetherloom_protocol::{InputPool, PlayerId, RegionId, TeamId, MAX_RELIABLE_FRAME_BYTES};
use aetherloom_server::{
    DeliveryKind, DisconnectReason, InboundMessage, NetworkTransport, PeerId,
    TransportCapabilities, TransportError, MAX_GAMEPLAY_DATAGRAM_BYTES,
};
use bytes::Bytes;
pub use quinn::Connection;
use quinn::{Endpoint, ServerConfig, VarInt};
use tokio::sync::{mpsc, watch};

const RELIABLE_LENGTH_BYTES: usize = 4;
const CLOSE_PROTOCOL: u32 = 0x100;
const CLOSE_REJECTED: u32 = 0x101;
const CLOSE_BACKPRESSURE: u32 = 0x102;
const CLOSE_DRAINING: u32 = 0x103;
const CLOSE_SHUTDOWN: u32 = 0x104;
const CLOSE_IO: u32 = 0x105;

/// Immutable claims produced by verification of a signed connection ticket.
///
/// These fields are delivered on a connection event, never decoded from
/// gameplay datagrams.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifiedConnectionClaims {
    pub match_id: [u8; 16],
    pub content_build_hash: [u8; 16],
    pub match_epoch: u64,
    pub region: RegionId,
    pub input_pool: InputPool,
    pub nonce: [u8; 16],
    pub account_id: [u8; 16],
    pub player_id: PlayerId,
    pub team_id: TeamId,
    pub expires_at_unix_seconds: u64,
}

/// The authoritative result of the caller-owned admission hook.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthenticatedConnection {
    /// Opaque connection identity allocated by the server, not the client.
    pub peer: PeerId,
    /// Time used for signature and expiry verification.
    pub verified_at_unix_seconds: u64,
    pub claims: VerifiedConnectionClaims,
}

/// Ordered lifecycle events consumed before gameplay ingress each tick.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionEvent {
    Authenticated(AuthenticatedConnection),
    Disconnected { peer: PeerId },
}

/// Future returned by an authenticated connection-admission policy.
pub type AdmissionFuture<'a> =
    Pin<Box<dyn Future<Output = Result<AuthenticatedConnection, AdmissionRejection>> + Send + 'a>>;

/// Authenticates a TLS-established QUIC connection and assigns its server-owned
/// identity.
///
/// The hook may inspect `peer_identity()` or consume an authentication preface
/// from a stream. The returned [`AuthenticatedConnection::peer`] is
/// authoritative; this crate never accepts a peer id supplied in a gameplay
/// packet.
pub trait ConnectionAdmission: Send + Sync + 'static {
    fn admit<'a>(&'a self, connection: &'a Connection) -> AdmissionFuture<'a>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdmissionRejection {
    reason: String,
}

impl AdmissionRejection {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl fmt::Display for AdmissionRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.reason)
    }
}

impl Error for AdmissionRejection {}

/// Runtime and resource limits applied in addition to the caller's TLS config.
#[derive(Clone, Debug)]
pub struct QuicTransportConfig {
    pub bind_address: SocketAddr,
    pub max_connections: usize,
    pub connection_event_queue_capacity: usize,
    pub inbound_queue_capacity: usize,
    pub inbound_queue_byte_capacity: usize,
    pub outbound_queue_capacity_per_peer: usize,
    pub outbound_queue_byte_capacity_per_peer: usize,
    pub max_inbound_uni_streams_per_peer: u32,
    pub idle_timeout: Duration,
    pub keepalive_interval: Duration,
    pub admission_timeout: Duration,
    pub datagram_buffer_bytes: usize,
    /// Includes client socket addresses in explicit per-peer telemetry
    /// snapshots.
    ///
    /// This is disabled by default because an IP address is sensitive
    /// operational data and should not become a high-cardinality metrics
    /// label. Enabling it does not change connection admission or routing.
    pub expose_remote_address_in_telemetry: bool,
}

impl Default for QuicTransportConfig {
    fn default() -> Self {
        Self {
            bind_address: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
            max_connections: 128,
            connection_event_queue_capacity: 256,
            inbound_queue_capacity: 4_096,
            inbound_queue_byte_capacity: 16 * 1024 * 1024,
            outbound_queue_capacity_per_peer: 256,
            outbound_queue_byte_capacity_per_peer: 8 * 1024 * 1024,
            max_inbound_uni_streams_per_peer: 4,
            idle_timeout: Duration::from_secs(10),
            keepalive_interval: Duration::from_secs(2),
            admission_timeout: Duration::from_secs(5),
            datagram_buffer_bytes: 2 * 1024 * 1024,
            expose_remote_address_in_telemetry: false,
        }
    }
}

impl QuicTransportConfig {
    fn validate(&self) -> Result<(), QuicBindError> {
        if self.max_connections == 0 {
            return Err(QuicBindError::InvalidConfig(
                "max_connections must be greater than zero",
            ));
        }
        if self.connection_event_queue_capacity == 0 {
            return Err(QuicBindError::InvalidConfig(
                "connection_event_queue_capacity must be greater than zero",
            ));
        }
        if self.inbound_queue_capacity == 0 {
            return Err(QuicBindError::InvalidConfig(
                "inbound_queue_capacity must be greater than zero",
            ));
        }
        if self.inbound_queue_capacity < self.max_connections {
            return Err(QuicBindError::InvalidConfig(
                "inbound_queue_capacity must reserve at least one packet per connection",
            ));
        }
        if self.inbound_queue_byte_capacity < MAX_GAMEPLAY_DATAGRAM_BYTES {
            return Err(QuicBindError::InvalidConfig(
                "inbound_queue_byte_capacity must hold one maximum datagram",
            ));
        }
        if self.inbound_queue_byte_capacity / self.max_connections < MAX_GAMEPLAY_DATAGRAM_BYTES {
            return Err(QuicBindError::InvalidConfig(
                "inbound_queue_byte_capacity must reserve one maximum datagram per connection",
            ));
        }
        if self.outbound_queue_capacity_per_peer == 0 {
            return Err(QuicBindError::InvalidConfig(
                "outbound_queue_capacity_per_peer must be greater than zero",
            ));
        }
        if self.outbound_queue_byte_capacity_per_peer < MAX_GAMEPLAY_DATAGRAM_BYTES {
            return Err(QuicBindError::InvalidConfig(
                "outbound_queue_byte_capacity_per_peer must hold one maximum datagram",
            ));
        }
        if self.max_inbound_uni_streams_per_peer == 0 {
            return Err(QuicBindError::InvalidConfig(
                "max_inbound_uni_streams_per_peer must be greater than zero",
            ));
        }
        if self.idle_timeout.is_zero()
            || self.keepalive_interval.is_zero()
            || self.admission_timeout.is_zero()
        {
            return Err(QuicBindError::InvalidConfig(
                "timeouts and keepalive interval must be greater than zero",
            ));
        }
        if self.keepalive_interval >= self.idle_timeout {
            return Err(QuicBindError::InvalidConfig(
                "keepalive_interval must be shorter than idle_timeout",
            ));
        }
        if self.datagram_buffer_bytes < MAX_GAMEPLAY_DATAGRAM_BYTES {
            return Err(QuicBindError::InvalidConfig(
                "datagram_buffer_bytes must hold one maximum datagram",
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum QuicBindError {
    InvalidConfig(&'static str),
    TransportConfig(String),
    Runtime(String),
    Bind(String),
    StartupChannelClosed,
}

impl fmt::Display for QuicBindError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(message) => formatter.write_str(message),
            Self::TransportConfig(message) => {
                write!(formatter, "invalid QUIC transport config: {message}")
            }
            Self::Runtime(message) => write!(formatter, "could not start Tokio runtime: {message}"),
            Self::Bind(message) => write!(formatter, "could not bind QUIC endpoint: {message}"),
            Self::StartupChannelClosed => formatter.write_str("QUIC worker exited during startup"),
        }
    }
}

impl Error for QuicBindError {}

/// Aggregate transport telemetry.
///
/// Counters cover the lifetime of this transport instance, including peers
/// that have disconnected. Queue-byte fields are live gauges. Atomics are read
/// independently, so this is an operational snapshot rather than a
/// transactionally consistent gameplay record.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct QuicStats {
    pub accepted_connections: u64,
    pub rejected_connections: u64,
    pub active_connections: usize,
    pub inbound_datagrams: u64,
    pub inbound_reliable_frames: u64,
    /// Successfully admitted application-payload bytes.
    ///
    /// This excludes QUIC, UDP, TLS, and reliable-frame length overhead and
    /// saturates at `u64::MAX`.
    pub inbound_payload_bytes: u64,
    pub outbound_datagrams: u64,
    pub outbound_reliable_frames: u64,
    /// Application-payload bytes successfully handed to Quinn.
    ///
    /// This excludes QUIC, UDP, TLS, and reliable-frame length overhead and
    /// saturates at `u64::MAX`.
    pub outbound_payload_bytes: u64,
    /// Application payload currently buffered on bounded ingress queues.
    pub queued_ingress_bytes: usize,
    /// Application payload currently buffered across bounded per-peer egress
    /// queues.
    pub queued_egress_bytes: usize,
    pub dropped_datagrams: u64,
    pub queue_backpressure: u64,
    pub protocol_violations: u64,
    pub io_errors: u64,
}

/// A point-in-time, read-only view of an authenticated QUIC peer.
///
/// `peer` is the server-owned identity returned by [`ConnectionAdmission`].
/// Quinn's counters include protocol overhead and can therefore be compared
/// with the application-payload counters in [`QuicStats`] to diagnose
/// transport costs. Counters are cumulative for the lifetime of this
/// connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerTelemetrySnapshot {
    pub peer: PeerId,
    /// Present only when
    /// [`QuicTransportConfig::expose_remote_address_in_telemetry`] is enabled.
    /// Never use this address as a gameplay or account identity.
    pub remote_address: Option<SocketAddr>,
    pub rtt: Duration,
    pub queued_egress_bytes: usize,
    pub udp_tx_datagrams: u64,
    pub udp_tx_bytes: u64,
    pub udp_rx_datagrams: u64,
    pub udp_rx_bytes: u64,
    pub congestion_window_bytes: u64,
    pub congestion_events: u64,
    pub path_sent_packets: u64,
    pub path_lost_packets: u64,
    pub path_lost_bytes: u64,
    pub sent_plpmtud_probes: u64,
    pub lost_plpmtud_probes: u64,
    pub black_holes_detected: u64,
    pub current_mtu: u16,
}

#[derive(Default)]
struct AtomicStats {
    accepted_connections: AtomicU64,
    rejected_connections: AtomicU64,
    active_connections: AtomicUsize,
    inbound_datagrams: AtomicU64,
    inbound_reliable_frames: AtomicU64,
    inbound_payload_bytes: AtomicU64,
    outbound_datagrams: AtomicU64,
    outbound_reliable_frames: AtomicU64,
    outbound_payload_bytes: AtomicU64,
    dropped_datagrams: AtomicU64,
    queue_backpressure: AtomicU64,
    protocol_violations: AtomicU64,
    io_errors: AtomicU64,
}

impl AtomicStats {
    fn snapshot(&self) -> QuicStats {
        QuicStats {
            accepted_connections: self.accepted_connections.load(Ordering::Relaxed),
            rejected_connections: self.rejected_connections.load(Ordering::Relaxed),
            active_connections: self.active_connections.load(Ordering::Relaxed),
            inbound_datagrams: self.inbound_datagrams.load(Ordering::Relaxed),
            inbound_reliable_frames: self.inbound_reliable_frames.load(Ordering::Relaxed),
            inbound_payload_bytes: self.inbound_payload_bytes.load(Ordering::Relaxed),
            outbound_datagrams: self.outbound_datagrams.load(Ordering::Relaxed),
            outbound_reliable_frames: self.outbound_reliable_frames.load(Ordering::Relaxed),
            outbound_payload_bytes: self.outbound_payload_bytes.load(Ordering::Relaxed),
            queued_ingress_bytes: 0,
            queued_egress_bytes: 0,
            dropped_datagrams: self.dropped_datagrams.load(Ordering::Relaxed),
            queue_backpressure: self.queue_backpressure.load(Ordering::Relaxed),
            protocol_violations: self.protocol_violations.load(Ordering::Relaxed),
            io_errors: self.io_errors.load(Ordering::Relaxed),
        }
    }
}

enum Outbound {
    Datagram(Vec<u8>),
    Reliable(Vec<u8>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IngressQueueError {
    PeerFull,
    GlobalFull,
    Busy,
    Unavailable,
}

#[derive(Debug, Default)]
struct PeerIngressQueue {
    messages: VecDeque<InboundMessage>,
    bytes: usize,
}

#[derive(Debug, Default)]
struct FairIngressState {
    peers: BTreeMap<PeerId, PeerIngressQueue>,
    packets: usize,
    bytes: usize,
    last_served: Option<PeerId>,
}

/// Bounded per-peer ingress with deterministic round-robin dequeue order.
///
/// Dividing the global budgets by the connection ceiling prevents one
/// admitted connection from consuming capacity reserved for every other
/// admitted peer. The simulation-facing pop rotates in stable `PeerId` order,
/// independent of task wakeup order in the async socket runtime.
#[derive(Debug)]
struct FairIngressQueue {
    packet_capacity: usize,
    byte_capacity: usize,
    packet_capacity_per_peer: usize,
    byte_capacity_per_peer: usize,
    queued_bytes: AtomicUsize,
    state: Mutex<FairIngressState>,
}

impl FairIngressQueue {
    fn new(packet_capacity: usize, byte_capacity: usize, max_connections: usize) -> Self {
        debug_assert!(packet_capacity >= max_connections);
        debug_assert!(byte_capacity / max_connections >= MAX_GAMEPLAY_DATAGRAM_BYTES);
        Self {
            packet_capacity,
            byte_capacity,
            packet_capacity_per_peer: packet_capacity / max_connections,
            byte_capacity_per_peer: byte_capacity / max_connections,
            queued_bytes: AtomicUsize::new(0),
            state: Mutex::new(FairIngressState::default()),
        }
    }

    fn try_push(&self, message: InboundMessage) -> Result<(), IngressQueueError> {
        let length = message.payload.len();
        let mut state = self
            .state
            .lock()
            .map_err(|_| IngressQueueError::Unavailable)?;
        let peer_full = state.peers.get(&message.peer).map_or(
            self.packet_capacity_per_peer == 0 || length > self.byte_capacity_per_peer,
            |queue| {
                queue.messages.len() >= self.packet_capacity_per_peer
                    || queue.bytes.saturating_add(length) > self.byte_capacity_per_peer
            },
        );
        if peer_full {
            return Err(IngressQueueError::PeerFull);
        }
        if state.packets >= self.packet_capacity
            || state.bytes.saturating_add(length) > self.byte_capacity
        {
            return Err(IngressQueueError::GlobalFull);
        }

        let queue = state.peers.entry(message.peer).or_default();
        queue.bytes += length;
        queue.messages.push_back(message);
        state.packets += 1;
        state.bytes += length;
        self.queued_bytes.fetch_add(length, Ordering::Release);
        Ok(())
    }

    fn pop(&self) -> Result<Option<InboundMessage>, IngressQueueError> {
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(IngressQueueError::Busy);
            }
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Err(IngressQueueError::Unavailable);
            }
        };
        let next_peer = state
            .last_served
            .and_then(|last| {
                state
                    .peers
                    .range((Excluded(last), Unbounded))
                    .next()
                    .map(|(&peer, _)| peer)
            })
            .or_else(|| state.peers.keys().next().copied());
        let Some(peer) = next_peer else {
            return Ok(None);
        };

        let (message, remove_queue) = {
            let queue = state
                .peers
                .get_mut(&peer)
                .expect("selected peer has an ingress queue");
            let message = queue
                .messages
                .pop_front()
                .expect("only non-empty peer queues are retained");
            queue.bytes -= message.payload.len();
            (message, queue.messages.is_empty())
        };
        if remove_queue {
            state.peers.remove(&peer);
        }
        state.packets -= 1;
        state.bytes -= message.payload.len();
        state.last_served = Some(peer);
        self.queued_bytes
            .fetch_sub(message.payload.len(), Ordering::AcqRel);
        Ok(Some(message))
    }

    fn purge_peer(&self, peer: PeerId) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if let Some(queue) = state.peers.remove(&peer) {
            state.packets = state.packets.saturating_sub(queue.messages.len());
            state.bytes = state.bytes.saturating_sub(queue.bytes);
            self.queued_bytes.fetch_sub(queue.bytes, Ordering::AcqRel);
        }
    }

    fn queued_bytes(&self) -> usize {
        self.queued_bytes.load(Ordering::Acquire)
    }
}

#[derive(Clone)]
struct PeerHandle {
    connection_id: usize,
    connection: Connection,
    outbound: mpsc::Sender<Outbound>,
    queued_bytes: Arc<AtomicUsize>,
    byte_capacity: usize,
}

type PeerMap = Arc<RwLock<HashMap<PeerId, PeerHandle>>>;

struct ConnectionSlot {
    occupied: Arc<AtomicUsize>,
}

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        self.occupied.fetch_sub(1, Ordering::AcqRel);
    }
}

/// A concrete native QUIC implementation of [`NetworkTransport`].
pub struct NativeQuicTransport {
    local_address: SocketAddr,
    connection_events: mpsc::Receiver<ConnectionEvent>,
    inbound: Arc<FairIngressQueue>,
    peers: PeerMap,
    stats: Arc<AtomicStats>,
    draining: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    expose_remote_address_in_telemetry: bool,
    shutdown: watch::Sender<bool>,
    worker: Option<JoinHandle<()>>,
}

impl NativeQuicTransport {
    /// Starts a native QUIC endpoint.
    ///
    /// `server_config` must contain caller-supplied TLS identity and certificate
    /// policy. This function only applies transport limits and never generates
    /// or persists credentials.
    pub fn bind(
        mut server_config: ServerConfig,
        config: QuicTransportConfig,
        admission: Arc<dyn ConnectionAdmission>,
    ) -> Result<Self, QuicBindError> {
        config.validate()?;
        apply_transport_limits(&mut server_config, &config)?;

        let (connection_event_tx, connection_events) =
            mpsc::channel(config.connection_event_queue_capacity);
        let inbound = Arc::new(FairIngressQueue::new(
            config.inbound_queue_capacity,
            config.inbound_queue_byte_capacity,
            config.max_connections,
        ));
        let peers = Arc::new(RwLock::new(HashMap::new()));
        let stats = Arc::new(AtomicStats::default());
        let draining = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        let (shutdown, shutdown_rx) = watch::channel(false);
        let (startup_tx, startup_rx) = std::sync::mpsc::sync_channel(1);

        let worker_peers = Arc::clone(&peers);
        let worker_stats = Arc::clone(&stats);
        let worker_draining = Arc::clone(&draining);
        let worker_stopped = Arc::clone(&stopped);
        let worker_config = config.clone();
        let worker_inbound = Arc::clone(&inbound);
        let worker = thread::Builder::new()
            .name("aetherloom-quic".to_owned())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = startup_tx.send(Err(QuicBindError::Runtime(error.to_string())));
                        worker_stopped.store(true, Ordering::Release);
                        return;
                    }
                };

                runtime.block_on(async move {
                    let endpoint = match Endpoint::server(server_config, worker_config.bind_address)
                    {
                        Ok(endpoint) => endpoint,
                        Err(error) => {
                            let _ = startup_tx.send(Err(QuicBindError::Bind(error.to_string())));
                            worker_stopped.store(true, Ordering::Release);
                            return;
                        }
                    };
                    let local_address = match endpoint.local_addr() {
                        Ok(address) => address,
                        Err(error) => {
                            let _ = startup_tx.send(Err(QuicBindError::Bind(error.to_string())));
                            worker_stopped.store(true, Ordering::Release);
                            return;
                        }
                    };
                    if startup_tx.send(Ok(local_address)).is_err() {
                        endpoint.close(VarInt::from_u32(CLOSE_SHUTDOWN), b"startup cancelled");
                        worker_stopped.store(true, Ordering::Release);
                        return;
                    }

                    run_endpoint(
                        endpoint,
                        worker_config,
                        admission,
                        connection_event_tx,
                        worker_inbound,
                        worker_peers,
                        worker_stats,
                        worker_draining,
                        shutdown_rx,
                    )
                    .await;
                    worker_stopped.store(true, Ordering::Release);
                });
            })
            .map_err(|error| QuicBindError::Runtime(error.to_string()))?;

        let local_address = startup_rx
            .recv()
            .map_err(|_| QuicBindError::StartupChannelClosed)??;

        Ok(Self {
            local_address,
            connection_events,
            inbound,
            peers,
            stats,
            draining,
            stopped,
            expose_remote_address_in_telemetry: config.expose_remote_address_in_telemetry,
            shutdown,
            worker: Some(worker),
        })
    }

    pub const fn local_address(&self) -> SocketAddr {
        self.local_address
    }

    pub fn stats(&self) -> QuicStats {
        let mut snapshot = self.stats.snapshot();
        snapshot.queued_ingress_bytes = self.inbound.queued_bytes();
        snapshot.queued_egress_bytes = self.aggregate_queued_egress_bytes();
        snapshot
    }

    pub fn connected_peers(&self) -> Vec<PeerId> {
        let mut peers = match self.peers.read() {
            Ok(peers) => peers.keys().copied().collect::<Vec<_>>(),
            Err(_) => Vec::new(),
        };
        peers.sort_unstable();
        peers
    }

    /// Returns telemetry for the currently authenticated peer registry.
    ///
    /// Removed peers are intentionally absent rather than retained as stale
    /// identities. The result is sorted by server-owned [`PeerId`] so metric
    /// collection and tests do not depend on hash-map order.
    pub fn peer_telemetry(&self) -> Vec<PeerTelemetrySnapshot> {
        let mut handles = match self.peers.read() {
            Ok(peers) => peers
                .iter()
                .map(|(&peer, handle)| (peer, handle.clone()))
                .collect::<Vec<_>>(),
            Err(_) => return Vec::new(),
        };
        handles.sort_unstable_by_key(|(peer, _)| *peer);
        handles
            .into_iter()
            .map(|(peer, handle)| {
                let stats = handle.connection.stats();
                PeerTelemetrySnapshot {
                    peer,
                    remote_address: self
                        .expose_remote_address_in_telemetry
                        .then(|| handle.connection.remote_address()),
                    rtt: stats.path.rtt,
                    queued_egress_bytes: handle.queued_bytes.load(Ordering::Acquire),
                    udp_tx_datagrams: stats.udp_tx.datagrams,
                    udp_tx_bytes: stats.udp_tx.bytes,
                    udp_rx_datagrams: stats.udp_rx.datagrams,
                    udp_rx_bytes: stats.udp_rx.bytes,
                    congestion_window_bytes: stats.path.cwnd,
                    congestion_events: stats.path.congestion_events,
                    path_sent_packets: stats.path.sent_packets,
                    path_lost_packets: stats.path.lost_packets,
                    path_lost_bytes: stats.path.lost_bytes,
                    sent_plpmtud_probes: stats.path.sent_plpmtud_probes,
                    lost_plpmtud_probes: stats.path.lost_plpmtud_probes,
                    black_holes_detected: stats.path.black_holes_detected,
                    current_mtu: stats.path.current_mtu,
                }
            })
            .collect()
    }

    /// Polls the next authenticated lifecycle event without blocking.
    ///
    /// Dedicated runtimes consume all available lifecycle events before
    /// gameplay ingress so the peer binding exists before its first command.
    pub fn poll_connection_event(&mut self) -> Result<Option<ConnectionEvent>, TransportError> {
        match self.connection_events.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(mpsc::error::TryRecvError::Empty) => {
                if self.stopped.load(Ordering::Acquire) {
                    Err(TransportError::Closed)
                } else {
                    Ok(None)
                }
            }
            Err(mpsc::error::TryRecvError::Disconnected) => Err(TransportError::Closed),
        }
    }

    /// Rejects new admissions while existing connections continue normally.
    pub fn begin_draining(&self) {
        self.draining.store(true, Ordering::Release);
    }

    pub fn is_draining(&self) -> bool {
        self.draining.load(Ordering::Acquire)
    }

    /// Drains until all peers leave or `deadline` passes, then closes the
    /// endpoint and joins its I/O thread.
    pub fn shutdown_gracefully(&mut self, deadline: Instant) {
        self.begin_draining();
        while self.stats.active_connections.load(Ordering::Acquire) != 0
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(1));
        }
        self.shutdown_now();
    }

    pub fn shutdown_now(&mut self) {
        let _ = self.shutdown.send(true);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    fn peer(&self, peer: PeerId) -> Result<PeerHandle, TransportError> {
        self.peers
            .read()
            .map_err(|_| TransportError::Backend("QUIC peer registry is poisoned".to_owned()))?
            .get(&peer)
            .cloned()
            .ok_or(TransportError::InvalidPeer(peer))
    }

    fn aggregate_queued_egress_bytes(&self) -> usize {
        match self.peers.read() {
            Ok(peers) => peers.values().fold(0_usize, |total, handle| {
                total.saturating_add(handle.queued_bytes.load(Ordering::Acquire))
            }),
            Err(_) => 0,
        }
    }
}

impl NetworkTransport for NativeQuicTransport {
    fn capabilities(&self) -> TransportCapabilities {
        TransportCapabilities::quic_compatible(MAX_GAMEPLAY_DATAGRAM_BYTES)
    }

    fn poll_receive(&mut self) -> Result<Option<InboundMessage>, TransportError> {
        match self.inbound.pop() {
            Ok(Some(message)) => Ok(Some(message)),
            Ok(None) => {
                if self.stopped.load(Ordering::Acquire) {
                    Err(TransportError::Closed)
                } else {
                    Ok(None)
                }
            }
            Err(IngressQueueError::Unavailable) => Err(TransportError::Backend(
                "QUIC ingress queue is poisoned".to_owned(),
            )),
            Err(IngressQueueError::Busy) => Ok(None),
            Err(IngressQueueError::PeerFull | IngressQueueError::GlobalFull) => {
                unreachable!("pop never reports capacity failures")
            }
        }
    }

    fn try_send(
        &mut self,
        peer: PeerId,
        delivery: DeliveryKind,
        payload: &[u8],
    ) -> Result<(), TransportError> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(TransportError::Closed);
        }
        if delivery == DeliveryKind::Datagram && payload.len() > MAX_GAMEPLAY_DATAGRAM_BYTES {
            return Err(TransportError::DatagramTooLarge {
                actual: payload.len(),
                maximum: MAX_GAMEPLAY_DATAGRAM_BYTES,
            });
        }
        if delivery == DeliveryKind::Reliable && payload.len() > MAX_RELIABLE_FRAME_BYTES {
            return Err(TransportError::Backend(format!(
                "reliable frame is {} bytes; maximum is {}",
                payload.len(),
                MAX_RELIABLE_FRAME_BYTES
            )));
        }

        let handle = self.peer(peer)?;
        let permit = handle.outbound.try_reserve().map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => {
                self.stats
                    .queue_backpressure
                    .fetch_add(1, Ordering::Relaxed);
                TransportError::Backpressure
            }
            mpsc::error::TrySendError::Closed(_) => TransportError::Closed,
        })?;
        if !try_reserve_bytes(&handle.queued_bytes, handle.byte_capacity, payload.len()) {
            self.stats
                .queue_backpressure
                .fetch_add(1, Ordering::Relaxed);
            return Err(TransportError::Backpressure);
        }
        let message = match delivery {
            DeliveryKind::Datagram => Outbound::Datagram(payload.to_vec()),
            DeliveryKind::Reliable => Outbound::Reliable(payload.to_vec()),
        };
        permit.send(message);
        Ok(())
    }

    fn disconnect(&mut self, peer: PeerId, reason: DisconnectReason) -> Result<(), TransportError> {
        let handle = self.peer(peer)?;
        handle.connection.close(
            VarInt::from_u32(close_code(reason)),
            disconnect_reason(reason),
        );
        Ok(())
    }
}

impl Drop for NativeQuicTransport {
    fn drop(&mut self) {
        self.shutdown_now();
    }
}

fn apply_transport_limits(
    server_config: &mut ServerConfig,
    config: &QuicTransportConfig,
) -> Result<(), QuicBindError> {
    let idle_timeout = quinn::IdleTimeout::try_from(config.idle_timeout)
        .map_err(|error| QuicBindError::TransportConfig(error.to_string()))?;
    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(idle_timeout));
    transport.keep_alive_interval(Some(config.keepalive_interval));
    transport.max_concurrent_bidi_streams(VarInt::from_u32(0));
    transport.max_concurrent_uni_streams(VarInt::from_u32(config.max_inbound_uni_streams_per_peer));
    transport.datagram_receive_buffer_size(Some(config.datagram_buffer_bytes));
    transport.datagram_send_buffer_size(config.datagram_buffer_bytes);
    server_config.transport_config(Arc::new(transport));
    Ok(())
}

async fn run_endpoint(
    endpoint: Endpoint,
    config: QuicTransportConfig,
    admission: Arc<dyn ConnectionAdmission>,
    connection_events: mpsc::Sender<ConnectionEvent>,
    inbound: Arc<FairIngressQueue>,
    peers: PeerMap,
    stats: Arc<AtomicStats>,
    draining: Arc<AtomicBool>,
    mut shutdown: watch::Receiver<bool>,
) {
    let occupied_slots = Arc::new(AtomicUsize::new(0));
    loop {
        tokio::select! {
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break };
                if draining.load(Ordering::Acquire) {
                    incoming.refuse();
                    stats.rejected_connections.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                if !try_reserve_slots(&occupied_slots, config.max_connections) {
                    incoming.refuse();
                    stats.rejected_connections.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                let connection_slot = ConnectionSlot {
                    occupied: Arc::clone(&occupied_slots),
                };
                let task_config = config.clone();
                let task_admission = Arc::clone(&admission);
                let task_connection_events = connection_events.clone();
                let task_inbound = inbound.clone();
                let task_peers = Arc::clone(&peers);
                let task_stats = Arc::clone(&stats);
                let task_draining = Arc::clone(&draining);
                tokio::spawn(async move {
                    accept_connection(
                        incoming,
                        task_config,
                        task_admission,
                        task_connection_events,
                        task_inbound,
                        task_peers,
                        task_stats,
                        task_draining,
                        connection_slot,
                    ).await;
                });
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
        }
    }

    endpoint.close(VarInt::from_u32(CLOSE_SHUTDOWN), b"server shutdown");
    endpoint.wait_idle().await;
}

async fn accept_connection(
    incoming: quinn::Incoming,
    config: QuicTransportConfig,
    admission: Arc<dyn ConnectionAdmission>,
    connection_events: mpsc::Sender<ConnectionEvent>,
    inbound: Arc<FairIngressQueue>,
    peers: PeerMap,
    stats: Arc<AtomicStats>,
    draining: Arc<AtomicBool>,
    _connection_slot: ConnectionSlot,
) {
    let connection = match incoming.await {
        Ok(connection) => connection,
        Err(_) => {
            stats.rejected_connections.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };
    if draining.load(Ordering::Acquire) {
        connection.close(VarInt::from_u32(CLOSE_DRAINING), b"server draining");
        stats.rejected_connections.fetch_add(1, Ordering::Relaxed);
        return;
    }

    let authenticated =
        match tokio::time::timeout(config.admission_timeout, admission.admit(&connection)).await {
            Ok(Ok(authenticated)) => authenticated,
            Ok(Err(_)) => {
                // Do not reflect potentially sensitive authentication
                // diagnostics to an unauthenticated remote. Policies can record
                // their own detailed rejection telemetry before returning.
                connection.close(VarInt::from_u32(CLOSE_REJECTED), b"admission rejected");
                stats.rejected_connections.fetch_add(1, Ordering::Relaxed);
                return;
            }
            Err(_) => {
                connection.close(VarInt::from_u32(CLOSE_REJECTED), b"admission timeout");
                stats.rejected_connections.fetch_add(1, Ordering::Relaxed);
                return;
            }
        };
    let peer = authenticated.peer;

    let (outbound_tx, outbound_rx) = mpsc::channel(config.outbound_queue_capacity_per_peer);
    let queued_bytes = Arc::new(AtomicUsize::new(0));
    {
        let mut registry = match peers.write() {
            Ok(registry) => registry,
            Err(_) => {
                connection.close(VarInt::from_u32(CLOSE_REJECTED), b"registry unavailable");
                stats.rejected_connections.fetch_add(1, Ordering::Relaxed);
                return;
            }
        };
        if registry.len() >= config.max_connections || registry.contains_key(&peer) {
            connection.close(VarInt::from_u32(CLOSE_REJECTED), b"peer admission conflict");
            stats.rejected_connections.fetch_add(1, Ordering::Relaxed);
            return;
        }
        registry.insert(
            peer,
            PeerHandle {
                connection_id: connection.stable_id(),
                connection: connection.clone(),
                outbound: outbound_tx,
                queued_bytes: Arc::clone(&queued_bytes),
                byte_capacity: config.outbound_queue_byte_capacity_per_peer,
            },
        );
    }

    if let Err(error) = connection_events.try_send(ConnectionEvent::Authenticated(authenticated)) {
        if matches!(error, mpsc::error::TrySendError::Full(_)) {
            stats.queue_backpressure.fetch_add(1, Ordering::Relaxed);
        }
        if let Ok(mut registry) = peers.write() {
            if registry
                .get(&peer)
                .is_some_and(|handle| handle.connection_id == connection.stable_id())
            {
                registry.remove(&peer);
            }
        }
        connection.close(
            VarInt::from_u32(CLOSE_BACKPRESSURE),
            b"connection event backpressure",
        );
        stats.rejected_connections.fetch_add(1, Ordering::Relaxed);
        return;
    }

    stats.accepted_connections.fetch_add(1, Ordering::Relaxed);
    stats.active_connections.fetch_add(1, Ordering::Relaxed);

    tokio::spawn(outbound_writer(
        connection.clone(),
        outbound_rx,
        queued_bytes,
        Arc::clone(&stats),
    ));
    tokio::spawn(datagram_reader(
        peer,
        connection.clone(),
        inbound.clone(),
        Arc::clone(&stats),
    ));
    tokio::spawn(reliable_acceptor(
        peer,
        connection.clone(),
        inbound.clone(),
        Arc::clone(&stats),
    ));

    let connection_id = connection.stable_id();
    connection.closed().await;
    if let Ok(mut registry) = peers.write() {
        if registry
            .get(&peer)
            .is_some_and(|handle| handle.connection_id == connection_id)
        {
            registry.remove(&peer);
            stats.active_connections.fetch_sub(1, Ordering::Relaxed);
        }
    }
    // Inputs are transient. Discard anything not consumed before the
    // disconnect event so stale frames cannot survive identity reuse.
    inbound.purge_peer(peer);
    let _ = connection_events
        .send(ConnectionEvent::Disconnected { peer })
        .await;
}

async fn datagram_reader(
    peer: PeerId,
    connection: Connection,
    inbound: Arc<FairIngressQueue>,
    stats: Arc<AtomicStats>,
) {
    while let Ok(payload) = connection.read_datagram().await {
        if payload.len() > MAX_GAMEPLAY_DATAGRAM_BYTES {
            stats.protocol_violations.fetch_add(1, Ordering::Relaxed);
            connection.close(VarInt::from_u32(CLOSE_PROTOCOL), b"oversized datagram");
            return;
        }
        let message = InboundMessage {
            peer,
            delivery: DeliveryKind::Datagram,
            payload: payload.to_vec(),
        };
        match inbound.try_push(message) {
            Ok(()) => {
                stats.inbound_datagrams.fetch_add(1, Ordering::Relaxed);
                saturating_add_bytes(&stats.inbound_payload_bytes, payload.len());
            }
            Err(IngressQueueError::PeerFull | IngressQueueError::GlobalFull) => {
                stats.dropped_datagrams.fetch_add(1, Ordering::Relaxed);
                stats.queue_backpressure.fetch_add(1, Ordering::Relaxed);
            }
            Err(IngressQueueError::Unavailable) => {
                stats.io_errors.fetch_add(1, Ordering::Relaxed);
                return;
            }
            Err(IngressQueueError::Busy) => {
                unreachable!("push waits for the short ingress critical section")
            }
        }
    }
}

async fn reliable_acceptor(
    peer: PeerId,
    connection: Connection,
    inbound: Arc<FairIngressQueue>,
    stats: Arc<AtomicStats>,
) {
    loop {
        let stream = match connection.accept_uni().await {
            Ok(stream) => stream,
            Err(_) => return,
        };
        let task_connection = connection.clone();
        let task_inbound = inbound.clone();
        let task_stats = Arc::clone(&stats);
        read_reliable_stream(peer, task_connection, stream, task_inbound, task_stats).await;
    }
}

async fn read_reliable_stream(
    peer: PeerId,
    connection: Connection,
    mut stream: quinn::RecvStream,
    inbound: Arc<FairIngressQueue>,
    stats: Arc<AtomicStats>,
) {
    loop {
        let mut length = [0_u8; RELIABLE_LENGTH_BYTES];
        let first = match stream.read(&mut length[..1]).await {
            Ok(Some(read)) => read,
            Ok(None) => return,
            Err(_) => {
                stats.io_errors.fetch_add(1, Ordering::Relaxed);
                return;
            }
        };
        if first != 1 || stream.read_exact(&mut length[1..]).await.is_err() {
            protocol_close(&connection, &stats, b"truncated reliable length");
            return;
        }
        let length = u32::from_be_bytes(length) as usize;
        if length > MAX_RELIABLE_FRAME_BYTES {
            protocol_close(&connection, &stats, b"oversized reliable frame");
            return;
        }
        let mut payload = vec![0_u8; length];
        if stream.read_exact(&mut payload).await.is_err() {
            protocol_close(&connection, &stats, b"truncated reliable frame");
            return;
        }
        let message = InboundMessage {
            peer,
            delivery: DeliveryKind::Reliable,
            payload,
        };
        match inbound.try_push(message) {
            Ok(()) => {
                stats
                    .inbound_reliable_frames
                    .fetch_add(1, Ordering::Relaxed);
                saturating_add_bytes(&stats.inbound_payload_bytes, length);
            }
            Err(IngressQueueError::PeerFull | IngressQueueError::GlobalFull) => {
                stats.queue_backpressure.fetch_add(1, Ordering::Relaxed);
                connection.close(
                    VarInt::from_u32(CLOSE_BACKPRESSURE),
                    b"reliable ingress backpressure",
                );
                return;
            }
            Err(IngressQueueError::Unavailable) => {
                stats.io_errors.fetch_add(1, Ordering::Relaxed);
                return;
            }
            Err(IngressQueueError::Busy) => {
                unreachable!("push waits for the short ingress critical section")
            }
        }
    }
}

async fn outbound_writer(
    connection: Connection,
    mut outbound: mpsc::Receiver<Outbound>,
    queued_bytes: Arc<AtomicUsize>,
    stats: Arc<AtomicStats>,
) {
    let mut reliable_stream = None;
    while let Some(message) = outbound.recv().await {
        match message {
            Outbound::Datagram(payload) => {
                let payload_length = payload.len();
                if connection.send_datagram(Bytes::from(payload)).is_ok() {
                    stats.outbound_datagrams.fetch_add(1, Ordering::Relaxed);
                    saturating_add_bytes(&stats.outbound_payload_bytes, payload_length);
                } else {
                    stats.io_errors.fetch_add(1, Ordering::Relaxed);
                }
                queued_bytes.fetch_sub(payload_length, Ordering::AcqRel);
            }
            Outbound::Reliable(payload) => {
                let payload_length = payload.len();
                if reliable_stream.is_none() {
                    match connection.open_uni().await {
                        Ok(stream) => reliable_stream = Some(stream),
                        Err(_) => {
                            queued_bytes.fetch_sub(payload_length, Ordering::AcqRel);
                            stats.io_errors.fetch_add(1, Ordering::Relaxed);
                            connection
                                .close(VarInt::from_u32(CLOSE_IO), b"reliable stream open failed");
                            return;
                        }
                    }
                }
                let stream = reliable_stream.as_mut().expect("stream was initialized");
                let length = (payload.len() as u32).to_be_bytes();
                if stream.write_all(&length).await.is_err()
                    || stream.write_all(&payload).await.is_err()
                {
                    queued_bytes.fetch_sub(payload_length, Ordering::AcqRel);
                    stats.io_errors.fetch_add(1, Ordering::Relaxed);
                    connection.close(VarInt::from_u32(CLOSE_IO), b"reliable stream write failed");
                    return;
                }
                stats
                    .outbound_reliable_frames
                    .fetch_add(1, Ordering::Relaxed);
                saturating_add_bytes(&stats.outbound_payload_bytes, payload_length);
                queued_bytes.fetch_sub(payload_length, Ordering::AcqRel);
            }
        }
    }
}

fn protocol_close(connection: &Connection, stats: &AtomicStats, reason: &'static [u8]) {
    stats.protocol_violations.fetch_add(1, Ordering::Relaxed);
    connection.close(VarInt::from_u32(CLOSE_PROTOCOL), reason);
}

fn try_reserve_bytes(counter: &AtomicUsize, capacity: usize, amount: usize) -> bool {
    counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current
                .checked_add(amount)
                .filter(|total| *total <= capacity)
        })
        .is_ok()
}

fn saturating_add_bytes(counter: &AtomicU64, amount: usize) {
    let amount = u64::try_from(amount).unwrap_or(u64::MAX);
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        Some(current.saturating_add(amount))
    });
}

fn try_reserve_slots(counter: &AtomicUsize, capacity: usize) -> bool {
    counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            (current < capacity).then_some(current + 1)
        })
        .is_ok()
}

const fn close_code(reason: DisconnectReason) -> u32 {
    match reason {
        DisconnectReason::ProtocolViolation => CLOSE_PROTOCOL,
        DisconnectReason::ServerDraining => CLOSE_DRAINING,
        DisconnectReason::ServerShutdown => CLOSE_SHUTDOWN,
        DisconnectReason::ClientClosed | DisconnectReason::ReconnectExpired => CLOSE_REJECTED,
    }
}

const fn disconnect_reason(reason: DisconnectReason) -> &'static [u8] {
    match reason {
        DisconnectReason::ClientClosed => b"client closed",
        DisconnectReason::ReconnectExpired => b"reconnect expired",
        DisconnectReason::ProtocolViolation => b"protocol violation",
        DisconnectReason::ServerDraining => b"server draining",
        DisconnectReason::ServerShutdown => b"server shutdown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inbound(peer: u64, marker: u8) -> InboundMessage {
        InboundMessage {
            peer: PeerId(peer),
            delivery: DeliveryKind::Datagram,
            payload: vec![marker; 16],
        }
    }

    #[test]
    fn byte_counters_saturate_instead_of_wrapping() {
        let counter = AtomicU64::new(u64::MAX - 1);
        saturating_add_bytes(&counter, 2);
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn ingress_quota_reserves_capacity_and_dequeues_peers_round_robin() {
        let queue = FairIngressQueue::new(8, 9_600, 2);
        for marker in 1..=4 {
            queue
                .try_push(inbound(10, marker))
                .expect("flooding peer remains within its bounded share");
        }
        assert_eq!(
            queue.try_push(inbound(10, 5)),
            Err(IngressQueueError::PeerFull)
        );

        // The flooded peer cannot consume the healthy peer's reserved share.
        queue
            .try_push(inbound(20, 0xa1))
            .expect("healthy peer has reserved ingress capacity");
        queue
            .try_push(inbound(20, 0xa2))
            .expect("healthy peer retains its own bounded share");

        let mut observed = Vec::new();
        while let Some(message) = queue.pop().expect("queue remains available") {
            observed.push((message.peer, message.payload[0]));
        }
        assert_eq!(
            observed,
            vec![
                (PeerId(10), 1),
                (PeerId(20), 0xa1),
                (PeerId(10), 2),
                (PeerId(20), 0xa2),
                (PeerId(10), 3),
                (PeerId(10), 4),
            ]
        );
        assert_eq!(queue.queued_bytes(), 0);
    }
}
