use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use aetherloom_protocol::{InputPool, PlayerId, RegionId, TeamId, MAX_RELIABLE_FRAME_BYTES};
use aetherloom_quic::{
    AdmissionFuture, AdmissionRejection, AuthenticatedConnection, ConnectionAdmission,
    ConnectionEvent, NativeQuicTransport, QuicTransportConfig, VerifiedConnectionClaims,
};
use aetherloom_server::{
    DeliveryKind, NetworkTransport, PeerId, TransportError, MAX_GAMEPLAY_DATAGRAM_BYTES,
};
use quinn::{ClientConfig, Connection, Endpoint, ServerConfig};
use rcgen::generate_simple_self_signed;
use rustls::pki_types::PrivatePkcs8KeyDer;
use rustls::RootCertStore;

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

struct IncrementingAdmission {
    next: AtomicU64,
}

impl IncrementingAdmission {
    fn new(first: u64) -> Self {
        Self {
            next: AtomicU64::new(first),
        }
    }
}

impl ConnectionAdmission for IncrementingAdmission {
    fn admit<'a>(&'a self, _connection: &'a Connection) -> AdmissionFuture<'a> {
        let peer = PeerId(self.next.fetch_add(1, Ordering::Relaxed));
        Box::pin(async move { Ok(authenticated(peer)) })
    }
}

fn authenticated(peer: PeerId) -> AuthenticatedConnection {
    AuthenticatedConnection {
        peer,
        verified_at_unix_seconds: 100,
        claims: VerifiedConnectionClaims {
            match_id: [1; 16],
            content_build_hash: [2; 16],
            match_epoch: 3,
            region: RegionId::new("local").expect("region"),
            input_pool: InputPool::Mixed,
            nonce: [3; 16],
            account_id: [4; 16],
            player_id: PlayerId::new(0).expect("player"),
            team_id: TeamId::new(0).expect("team"),
            expires_at_unix_seconds: 200,
        },
    }
}

struct RejectAdmission;

impl ConnectionAdmission for RejectAdmission {
    fn admit<'a>(&'a self, _connection: &'a Connection) -> AdmissionFuture<'a> {
        Box::pin(async { Err(AdmissionRejection::new("test rejection")) })
    }
}

fn tls_configs() -> (ServerConfig, ClientConfig) {
    let certified =
        generate_simple_self_signed(vec!["localhost".to_owned()]).expect("test certificate");
    let certificate = certified.cert.der().clone();
    let private_key = PrivatePkcs8KeyDer::from(certified.key_pair.serialize_der());

    let server = ServerConfig::with_single_cert(vec![certificate.clone()], private_key.into())
        .expect("server TLS config");
    let mut roots = RootCertStore::empty();
    roots.add(certificate).expect("test trust anchor");
    let client = ClientConfig::with_root_certificates(Arc::new(roots)).expect("client TLS config");
    (server, client)
}

fn test_config() -> QuicTransportConfig {
    QuicTransportConfig {
        bind_address: "127.0.0.1:0".parse().expect("loopback address"),
        idle_timeout: Duration::from_secs(2),
        keepalive_interval: Duration::from_millis(200),
        admission_timeout: Duration::from_secs(1),
        ..QuicTransportConfig::default()
    }
}

async fn connect(
    address: std::net::SocketAddr,
    client_config: ClientConfig,
) -> (Endpoint, Connection) {
    let mut endpoint =
        Endpoint::client("127.0.0.1:0".parse().expect("client address")).expect("client endpoint");
    endpoint.set_default_client_config(client_config);
    let connection = tokio::time::timeout(
        TEST_TIMEOUT,
        endpoint
            .connect(address, "localhost")
            .expect("connect parameters"),
    )
    .await
    .expect("connect timeout")
    .expect("QUIC connection");
    (endpoint, connection)
}

async fn eventually<T>(mut operation: impl FnMut() -> Option<T>) -> T {
    tokio::time::timeout(TEST_TIMEOUT, async {
        loop {
            if let Some(value) = operation() {
                return value;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("condition timeout")
}

async fn receive_server(transport: &mut NativeQuicTransport) -> aetherloom_server::InboundMessage {
    eventually(|| transport.poll_receive().expect("server receive")).await
}

async fn wait_for_peer(transport: &NativeQuicTransport) -> PeerId {
    eventually(|| transport.connected_peers().first().copied()).await
}

async fn write_frame(stream: &mut quinn::SendStream, payload: &[u8]) {
    stream
        .write_all(&(payload.len() as u32).to_be_bytes())
        .await
        .expect("frame length");
    stream.write_all(payload).await.expect("frame payload");
}

async fn read_frame(stream: &mut quinn::RecvStream) -> Vec<u8> {
    let mut length = [0_u8; 4];
    stream.read_exact(&mut length).await.expect("frame length");
    let mut payload = vec![0_u8; u32::from_be_bytes(length) as usize];
    stream
        .read_exact(&mut payload)
        .await
        .expect("frame payload");
    payload
}

async fn wait_for(mut predicate: impl FnMut() -> bool) {
    eventually(|| predicate().then_some(())).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn localhost_datagram_and_reliable_roundtrip() {
    let (server_tls, client_tls) = tls_configs();
    let mut transport = NativeQuicTransport::bind(
        server_tls,
        test_config(),
        Arc::new(IncrementingAdmission::new(41)),
    )
    .expect("server bind");
    let (_client_endpoint, client) = connect(transport.local_address(), client_tls).await;
    let peer = wait_for_peer(&transport).await;
    assert_eq!(peer, PeerId(41));
    let event = eventually(|| transport.poll_connection_event().expect("connection event")).await;
    assert_eq!(
        event,
        ConnectionEvent::Authenticated(authenticated(PeerId(41)))
    );
    assert_eq!(
        transport
            .poll_connection_event()
            .expect("event queue remains open"),
        None
    );

    client
        .send_datagram(b"client input".as_slice().into())
        .expect("client datagram");
    let message = receive_server(&mut transport).await;
    assert_eq!(message.peer, peer);
    assert_eq!(message.delivery, DeliveryKind::Datagram);
    assert_eq!(message.payload, b"client input");

    transport
        .try_send(peer, DeliveryKind::Datagram, b"server snapshot")
        .expect("server datagram");
    let payload = tokio::time::timeout(TEST_TIMEOUT, client.read_datagram())
        .await
        .expect("datagram timeout")
        .expect("server datagram");
    assert_eq!(payload.as_ref(), b"server snapshot");

    let mut client_stream = client.open_uni().await.expect("client reliable stream");
    write_frame(&mut client_stream, b"authenticated command").await;
    client_stream.finish().expect("finish reliable stream");
    let message = receive_server(&mut transport).await;
    assert_eq!(message.peer, peer);
    assert_eq!(message.delivery, DeliveryKind::Reliable);
    assert_eq!(message.payload, b"authenticated command");

    transport
        .try_send(peer, DeliveryKind::Reliable, b"reliable keyframe")
        .expect("server reliable");
    let mut server_stream = tokio::time::timeout(TEST_TIMEOUT, client.accept_uni())
        .await
        .expect("stream timeout")
        .expect("server reliable stream");
    assert_eq!(read_frame(&mut server_stream).await, b"reliable keyframe");

    let stats = transport.stats();
    assert_eq!(stats.active_connections, 1);
    assert_eq!(stats.accepted_connections, 1);
    assert_eq!(stats.inbound_datagrams, 1);
    assert_eq!(stats.inbound_reliable_frames, 1);
    assert_eq!(
        stats.inbound_payload_bytes,
        (b"client input".len() + b"authenticated command".len()) as u64
    );
    assert_eq!(stats.outbound_datagrams, 1);
    assert_eq!(stats.outbound_reliable_frames, 1);
    assert_eq!(
        stats.outbound_payload_bytes,
        (b"server snapshot".len() + b"reliable keyframe".len()) as u64
    );
    assert_eq!(stats.queued_ingress_bytes, 0);
    assert_eq!(stats.queued_egress_bytes, 0);
    assert_eq!(
        transport.peer_telemetry()[0].remote_address,
        None,
        "remote addresses must be redacted by default"
    );

    client.close(0_u32.into(), b"test complete");
    wait_for(|| transport.stats().active_connections == 0).await;
    let event = eventually(|| transport.poll_connection_event().expect("disconnect event")).await;
    assert_eq!(event, ConnectionEvent::Disconnected { peer });
    transport.shutdown_gracefully(Instant::now() + TEST_TIMEOUT);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn flooding_peer_cannot_starve_another_peer_ingress() {
    let (server_tls, client_tls) = tls_configs();
    let mut config = test_config();
    config.max_connections = 2;
    config.inbound_queue_capacity = 8;
    config.inbound_queue_byte_capacity = 8 * MAX_GAMEPLAY_DATAGRAM_BYTES;
    let mut transport =
        NativeQuicTransport::bind(server_tls, config, Arc::new(IncrementingAdmission::new(50)))
            .expect("server bind");

    let (first_endpoint, first) = connect(transport.local_address(), client_tls.clone()).await;
    wait_for(|| transport.connected_peers().len() == 1).await;
    let (second_endpoint, second) = connect(transport.local_address(), client_tls).await;
    wait_for(|| transport.connected_peers().len() == 2).await;

    for marker in 0..8_u8 {
        first
            .send_datagram(vec![marker; 16].into())
            .expect("flood datagram enters Quinn");
    }
    second
        .send_datagram(vec![0xa5; 16].into())
        .expect("healthy datagram enters Quinn");
    wait_for(|| {
        let stats = transport.stats();
        stats.inbound_datagrams >= 5 && stats.dropped_datagrams >= 1
    })
    .await;

    let first_message = receive_server(&mut transport).await;
    let second_message = receive_server(&mut transport).await;
    assert_eq!(first_message.peer, PeerId(50));
    assert_eq!(
        second_message.peer,
        PeerId(51),
        "stable round-robin must service the healthy peer before another flood frame"
    );
    assert_eq!(second_message.payload, vec![0xa5; 16]);

    first.close(0_u32.into(), b"test complete");
    second.close(0_u32.into(), b"test complete");
    wait_for(|| transport.stats().active_connections == 0).await;
    transport.shutdown_gracefully(Instant::now() + TEST_TIMEOUT);
    drop((first_endpoint, second_endpoint));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn peer_telemetry_reports_quinn_path_and_removes_closed_peer() {
    let (server_tls, client_tls) = tls_configs();
    let mut config = test_config();
    config.expose_remote_address_in_telemetry = true;
    let transport =
        NativeQuicTransport::bind(server_tls, config, Arc::new(IncrementingAdmission::new(91)))
            .expect("server bind");
    let (_endpoint, client) = connect(transport.local_address(), client_tls).await;
    let peer = wait_for_peer(&transport).await;

    let telemetry = eventually(|| {
        let snapshots = transport.peer_telemetry();
        snapshots
            .first()
            .copied()
            .filter(|snapshot| snapshot.path_sent_packets > 0)
    })
    .await;
    assert_eq!(telemetry.peer, peer);
    assert!(telemetry.rtt > Duration::ZERO);
    let remote_address = telemetry
        .remote_address
        .expect("remote address was explicitly enabled");
    assert!(remote_address.ip().is_loopback());
    assert_ne!(remote_address.port(), 0);
    assert_eq!(telemetry.queued_egress_bytes, 0);
    assert!(telemetry.udp_tx_datagrams > 0);
    assert!(telemetry.udp_tx_bytes > 0);
    assert!(telemetry.udp_rx_datagrams > 0);
    assert!(telemetry.udp_rx_bytes > 0);
    assert!(telemetry.congestion_window_bytes > 0);
    assert!(telemetry.current_mtu >= MAX_GAMEPLAY_DATAGRAM_BYTES as u16);

    client.close(0_u32.into(), b"telemetry removal");
    wait_for(|| transport.peer_telemetry().is_empty()).await;
    assert!(transport.connected_peers().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversize_send_is_rejected_before_queueing() {
    let (server_tls, client_tls) = tls_configs();
    let mut transport = NativeQuicTransport::bind(
        server_tls,
        test_config(),
        Arc::new(IncrementingAdmission::new(1)),
    )
    .expect("server bind");
    let (_endpoint, _client) = connect(transport.local_address(), client_tls).await;
    let peer = wait_for_peer(&transport).await;

    let oversized_datagram = vec![0_u8; MAX_GAMEPLAY_DATAGRAM_BYTES + 1];
    assert_eq!(
        transport.try_send(peer, DeliveryKind::Datagram, &oversized_datagram),
        Err(TransportError::DatagramTooLarge {
            actual: MAX_GAMEPLAY_DATAGRAM_BYTES + 1,
            maximum: MAX_GAMEPLAY_DATAGRAM_BYTES,
        })
    );

    let oversized_reliable = vec![0_u8; MAX_RELIABLE_FRAME_BYTES + 1];
    assert!(matches!(
        transport.try_send(peer, DeliveryKind::Reliable, &oversized_reliable),
        Err(TransportError::Backend(_))
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_reliable_length_closes_the_peer() {
    let (server_tls, client_tls) = tls_configs();
    let transport = NativeQuicTransport::bind(
        server_tls,
        test_config(),
        Arc::new(IncrementingAdmission::new(5)),
    )
    .expect("server bind");
    let (_endpoint, client) = connect(transport.local_address(), client_tls).await;
    assert_eq!(wait_for_peer(&transport).await, PeerId(5));

    let mut stream = client.open_uni().await.expect("client reliable stream");
    stream
        .write_all(&((MAX_RELIABLE_FRAME_BYTES as u32) + 1).to_be_bytes())
        .await
        .expect("malformed length");
    stream.finish().expect("finish malformed stream");

    wait_for(|| transport.stats().protocol_violations == 1).await;
    tokio::time::timeout(TEST_TIMEOUT, client.closed())
        .await
        .expect("close timeout");
    wait_for(|| transport.stats().active_connections == 0).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bounded_reliable_ingress_applies_backpressure() {
    let (server_tls, client_tls) = tls_configs();
    let mut config = test_config();
    config.max_connections = 1;
    config.inbound_queue_capacity = 1;
    let transport =
        NativeQuicTransport::bind(server_tls, config, Arc::new(IncrementingAdmission::new(8)))
            .expect("server bind");
    let (_endpoint, client) = connect(transport.local_address(), client_tls).await;
    assert_eq!(wait_for_peer(&transport).await, PeerId(8));

    let mut stream = client.open_uni().await.expect("client reliable stream");
    write_frame(&mut stream, b"fills bounded ingress").await;
    write_frame(&mut stream, b"must not be silently dropped").await;
    stream.finish().expect("finish stream");

    wait_for(|| transport.stats().queue_backpressure >= 1).await;
    tokio::time::timeout(TEST_TIMEOUT, client.closed())
        .await
        .expect("backpressure close timeout");
    assert_eq!(transport.stats().inbound_reliable_frames, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bounded_reliable_egress_reports_backpressure_without_blocking() {
    let (server_tls, client_tls) = tls_configs();
    let mut config = test_config();
    config.outbound_queue_capacity_per_peer = 1;
    config.outbound_queue_byte_capacity_per_peer = MAX_RELIABLE_FRAME_BYTES;
    let mut transport =
        NativeQuicTransport::bind(server_tls, config, Arc::new(IncrementingAdmission::new(12)))
            .expect("server bind");
    let (_endpoint, client) = connect(transport.local_address(), client_tls).await;
    let peer = wait_for_peer(&transport).await;

    // The client deliberately does not accept/read the server's reliable
    // stream. Flow control keeps the first maximum-size frame in flight, so the
    // per-peer byte budget rejects a later frame immediately.
    let payload = vec![0_u8; MAX_RELIABLE_FRAME_BYTES];
    let mut observed_backpressure = false;
    for _ in 0..16 {
        match transport.try_send(peer, DeliveryKind::Reliable, &payload) {
            Ok(()) => {}
            Err(TransportError::Backpressure) => {
                observed_backpressure = true;
                break;
            }
            Err(error) => panic!("unexpected send failure: {error}"),
        }
    }
    assert!(observed_backpressure);
    assert!(transport.stats().queue_backpressure >= 1);
    assert!(transport.stats().queued_egress_bytes <= MAX_RELIABLE_FRAME_BYTES);

    client.close(0_u32.into(), b"test complete");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admission_identity_is_server_owned_and_drain_rejects_new_connections() {
    let (server_tls, client_tls) = tls_configs();
    let mut config = test_config();
    config.max_connections = 1;
    let mut transport =
        NativeQuicTransport::bind(server_tls, config, Arc::new(IncrementingAdmission::new(77)))
            .expect("server bind");
    let (first_endpoint, first) = connect(transport.local_address(), client_tls.clone()).await;
    assert_eq!(wait_for_peer(&transport).await, PeerId(77));

    let mut second_endpoint =
        Endpoint::client("127.0.0.1:0".parse().expect("client address")).expect("client endpoint");
    second_endpoint.set_default_client_config(client_tls.clone());
    let second = tokio::time::timeout(
        TEST_TIMEOUT,
        second_endpoint
            .connect(transport.local_address(), "localhost")
            .expect("connect parameters"),
    )
    .await
    .expect("connection-limit timeout");
    assert!(second.is_err(), "connection cap must refuse a second peer");
    wait_for(|| transport.stats().rejected_connections >= 1).await;
    assert_eq!(transport.connected_peers(), vec![PeerId(77)]);

    transport.begin_draining();
    assert!(transport.is_draining());
    let mut third_endpoint =
        Endpoint::client("127.0.0.1:0".parse().expect("client address")).expect("client endpoint");
    third_endpoint.set_default_client_config(client_tls);
    let third = third_endpoint
        .connect(transport.local_address(), "localhost")
        .expect("connect parameters")
        .await;
    assert!(third.is_err(), "draining server must refuse new handshakes");

    first.close(0_u32.into(), b"leave drained server");
    wait_for(|| transport.stats().active_connections == 0).await;
    transport.shutdown_gracefully(Instant::now() + TEST_TIMEOUT);
    drop((first_endpoint, second_endpoint, third_endpoint));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_admission_rejection_never_registers_a_peer() {
    let (server_tls, client_tls) = tls_configs();
    let transport = NativeQuicTransport::bind(server_tls, test_config(), Arc::new(RejectAdmission))
        .expect("server bind");
    let (_endpoint, client) = connect(transport.local_address(), client_tls).await;
    tokio::time::timeout(TEST_TIMEOUT, client.closed())
        .await
        .expect("rejection close timeout");
    wait_for(|| transport.stats().rejected_connections == 1).await;
    assert!(transport.connected_peers().is_empty());
    assert_eq!(transport.stats().accepted_connections, 0);
}

// Compile-time assertion that admission hooks can be genuinely asynchronous.
#[allow(dead_code)]
fn admission_future_is_send(
    future: AdmissionFuture<'_>,
) -> Pin<Box<dyn Future<Output = Result<AuthenticatedConnection, AdmissionRejection>> + Send + '_>>
{
    future
}
