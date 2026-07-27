use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use aetherloom_core::WorldSeed;
use aetherloom_dedicated::{
    pump_quic_connection_events, DedicatedMatch, InboundRejection, MatchBuild,
    NoDirectTicketVerifier, NoopReplaySink, NoopSettlementSink, ProcessConfig,
    QuicConnectionPumpReport, SequentialPeerIdAllocator,
    SignedTicketVerifier, TicketConnectionAdmission, TicketVerificationError,
    UnixTimeSource, VerifiedTicket,
};
use aetherloom_protocol::{
    EnvelopeMetadata, InputBatch, Message, MessageEnvelope, PlayerCommand,
    PlayerId, TeamId,
};
use aetherloom_quic::{NativeQuicTransport, QuicTransportConfig};
use aetherloom_server::{
    DedicatedQuicHost, DeliveryKind, InboundMessage, PeerId,
};
use quinn::{ClientConfig, Endpoint, ServerConfig};
use rcgen::generate_simple_self_signed;
use rustls::pki_types::PrivatePkcs8KeyDer;
use rustls::RootCertStore;

const NOW: u64 = 10_000;
const TEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy)]
struct FixedClock;

impl UnixTimeSource for FixedClock {
    fn now_unix_seconds(&self) -> Option<u64> {
        Some(NOW)
    }
}

struct CountingVerifier {
    calls: Arc<AtomicUsize>,
    ticket: VerifiedTicket,
}

impl SignedTicketVerifier for CountingVerifier {
    fn verify(
        &self,
        signed_ticket: &[u8],
        expected_build: MatchBuild,
        _now_unix_seconds: u64,
    ) -> Result<VerifiedTicket, TicketVerificationError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if signed_ticket != b"signed-by-control-plane"
            || self.ticket.match_id != expected_build.match_id()
        {
            return Err(TicketVerificationError::BadSignature);
        }
        Ok(self.ticket)
    }
}

fn tls_configs() -> (ServerConfig, ClientConfig) {
    let certified =
        generate_simple_self_signed(vec!["localhost".to_owned()]).expect("test certificate");
    let certificate = certified.cert.der().clone();
    let private_key = PrivatePkcs8KeyDer::from(certified.key_pair.serialize_der());
    let server =
        ServerConfig::with_single_cert(vec![certificate.clone()], private_key.into())
            .expect("server TLS config");
    let mut roots = RootCertStore::empty();
    roots.add(certificate).expect("test trust anchor");
    let client =
        ClientConfig::with_root_certificates(Arc::new(roots)).expect("client TLS config");
    (server, client)
}

async fn connect(
    address: std::net::SocketAddr,
    client_config: ClientConfig,
) -> (Endpoint, quinn::Connection) {
    let mut endpoint =
        Endpoint::client("127.0.0.1:0".parse().expect("client address"))
            .expect("client endpoint");
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

async fn wait_for_report(
    process: &mut DedicatedMatch<
        DedicatedQuicHost<NativeQuicTransport>,
        NoDirectTicketVerifier,
        NoopReplaySink,
        NoopSettlementSink,
    >,
    predicate: impl Fn(&QuicConnectionPumpReport) -> bool,
) -> QuicConnectionPumpReport {
    tokio::time::timeout(TEST_TIMEOUT, async {
        loop {
            let report =
                pump_quic_connection_events(process).expect("connection event pump");
            if predicate(&report) {
                return report;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("event pump timeout")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verified_ticket_claims_enter_the_match_once_and_own_identity() {
    let build = MatchBuild::new([7; 16], [9; 16], 42).expect("build");
    let ticket = VerifiedTicket {
        match_id: build.match_id(),
        content_build_hash: build.content_build_hash(),
        match_epoch: build.match_epoch(),
        account_id: [3; 16],
        player_id: PlayerId::new(0).expect("player"),
        team_id: TeamId::new(4).expect("team"),
        expires_at_unix_seconds: NOW + 60,
    };
    let verification_calls = Arc::new(AtomicUsize::new(0));
    let policy = TicketConnectionAdmission::new(
        CountingVerifier {
            calls: Arc::clone(&verification_calls),
            ticket,
        },
        build,
        FixedClock,
        SequentialPeerIdAllocator::new(500),
    );

    let (server_tls, client_tls) = tls_configs();
    let transport = NativeQuicTransport::bind(
        server_tls,
        QuicTransportConfig {
            bind_address: "127.0.0.1:0".parse().expect("server address"),
            idle_timeout: Duration::from_secs(2),
            keepalive_interval: Duration::from_millis(200),
            admission_timeout: Duration::from_secs(1),
            ..QuicTransportConfig::default()
        },
        Arc::new(policy),
    )
    .expect("QUIC bind");
    let address = transport.local_address();
    let host = DedicatedQuicHost::new(transport).expect("dedicated host");
    let config =
        ProcessConfig::new(build, WorldSeed::new(0x5eed), 2).expect("process config");
    let mut process = DedicatedMatch::new(
        config,
        host,
        NoDirectTicketVerifier,
        NoopReplaySink,
        NoopSettlementSink,
    );

    let (_client_endpoint, client) = connect(address, client_tls).await;
    let mut ticket_stream = client.open_uni().await.expect("ticket stream");
    let signed_ticket = b"signed-by-control-plane";
    ticket_stream
        .write_all(&(signed_ticket.len() as u32).to_be_bytes())
        .await
        .expect("ticket length");
    ticket_stream
        .write_all(signed_ticket)
        .await
        .expect("signed ticket");
    ticket_stream.finish().expect("finish ticket stream");

    let report = wait_for_report(&mut process, |report| {
        report.authenticated_events == 1
    })
    .await;
    assert_eq!(report.admitted_players, 1);
    assert!(report.rejected.is_empty());
    assert_eq!(verification_calls.load(Ordering::SeqCst), 1);
    assert_eq!(process.health().connected_players, 1);
    assert_eq!(
        process
            .state()
            .player(PlayerId::new(0).expect("player"))
            .expect("admitted player")
            .team,
        Some(TeamId::new(4).expect("team"))
    );

    let empty = pump_quic_connection_events(&mut process).expect("empty event pump");
    assert_eq!(empty, QuicConnectionPumpReport::default());
    assert_eq!(verification_calls.load(Ordering::SeqCst), 1);

    // The input payload has no account/player identity field. The transport's
    // authenticated PeerId binding supplies the player when commands enter the
    // match.
    let command =
        PlayerCommand::new(0, 1, 0, 0, 0, 0, 0, None).expect("command");
    let batch = InputBatch::new(vec![command]).expect("batch");
    let envelope = MessageEnvelope::new(
        EnvelopeMetadata::new(
            build.content_build_hash(),
            build.match_epoch(),
            1,
            0,
            0,
        ),
        Message::InputBatch(batch),
    );
    let payload = envelope.encode_datagram().expect("input datagram");
    let unbound_peer = PeerId(999_999);
    assert_eq!(
        process.ingest_message(InboundMessage {
            peer: unbound_peer,
            delivery: DeliveryKind::Datagram,
            payload: payload.clone(),
        }),
        Err(InboundRejection::UnknownPeer(unbound_peer))
    );
    assert_eq!(
        process.ingest_message(InboundMessage {
            peer: PeerId(500),
            delivery: DeliveryKind::Datagram,
            payload,
        }),
        Ok(1)
    );

    client.close(0_u32.into(), b"test complete");
    let report = wait_for_report(&mut process, |report| {
        report.disconnected_events == 1
    })
    .await;
    assert_eq!(report.released_players, 1);
    assert_eq!(verification_calls.load(Ordering::SeqCst), 1);
    assert_eq!(process.health().connected_players, 0);
}
