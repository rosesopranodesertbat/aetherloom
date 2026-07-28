use std::error::Error;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use aetherloom_quic::{
    AdmissionFuture, AdmissionRejection, AuthenticatedConnection, Connection, ConnectionAdmission,
    ConnectionEvent, NativeQuicTransport, VerifiedConnectionClaims,
};
use aetherloom_server::{
    DedicatedQuicHost, DisconnectReason, NetworkTransport, PeerId, TransportError,
};

use crate::{
    AdmissionError, DedicatedMatch, MatchAdmissionScope, MatchBuild, ReplayCheckpointSink,
    SettlementSink, SignedTicketVerifier, TicketVerificationError, VerifiedTicket,
};

/// Hard cap for the signed ticket sent as the first client-initiated
/// unidirectional stream.
pub const MAX_SIGNED_CONNECTION_TICKET_BYTES: usize = 4 * 1024;

/// Supplies wall-clock time to ticket verification.
pub trait UnixTimeSource: Send + Sync + 'static {
    fn now_unix_seconds(&self) -> Option<u64>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemUnixTime;

impl UnixTimeSource for SystemUnixTime {
    fn now_unix_seconds(&self) -> Option<u64> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_secs())
    }
}

/// Allocates an opaque connection identity on the server.
pub trait ServerPeerIdAllocator: Send + Sync + 'static {
    fn allocate(&self) -> Option<PeerId>;
}

#[derive(Debug)]
pub struct SequentialPeerIdAllocator {
    next: AtomicU64,
}

impl SequentialPeerIdAllocator {
    pub const fn new(first: u64) -> Self {
        Self {
            next: AtomicU64::new(first),
        }
    }
}

impl ServerPeerIdAllocator for SequentialPeerIdAllocator {
    fn allocate(&self) -> Option<PeerId> {
        self.next
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_add(1)
            })
            .ok()
            .map(PeerId)
    }
}

/// QUIC admission policy that verifies the signed ticket before the connection
/// receives a server-owned `PeerId`.
pub struct TicketConnectionAdmission<V, C, A> {
    verifier: V,
    expected_build: MatchBuild,
    expected_scope: MatchAdmissionScope,
    clock: C,
    peer_ids: A,
}

impl<V, C, A> TicketConnectionAdmission<V, C, A> {
    pub const fn new(
        verifier: V,
        expected_build: MatchBuild,
        expected_scope: MatchAdmissionScope,
        clock: C,
        peer_ids: A,
    ) -> Self {
        Self {
            verifier,
            expected_build,
            expected_scope,
            clock,
            peer_ids,
        }
    }
}

impl<V, C, A> ConnectionAdmission for TicketConnectionAdmission<V, C, A>
where
    V: SignedTicketVerifier + Send + Sync + 'static,
    C: UnixTimeSource,
    A: ServerPeerIdAllocator,
{
    fn admit<'a>(&'a self, connection: &'a Connection) -> AdmissionFuture<'a> {
        Box::pin(async move {
            let now_unix_seconds = self
                .clock
                .now_unix_seconds()
                .ok_or_else(|| AdmissionRejection::new("system clock unavailable"))?;
            let signed_ticket = read_signed_ticket(connection).await?;
            let ticket = self
                .verifier
                .verify(
                    &signed_ticket,
                    self.expected_build,
                    self.expected_scope,
                    now_unix_seconds,
                )
                .map_err(|_| AdmissionRejection::new("ticket verification failed"))?;
            let peer = self
                .peer_ids
                .allocate()
                .ok_or_else(|| AdmissionRejection::new("peer id space exhausted"))?;
            Ok(AuthenticatedConnection {
                peer,
                verified_at_unix_seconds: now_unix_seconds,
                claims: ticket.into(),
            })
        })
    }
}

async fn read_signed_ticket(connection: &Connection) -> Result<Vec<u8>, AdmissionRejection> {
    let mut stream = connection
        .accept_uni()
        .await
        .map_err(|_| AdmissionRejection::new("ticket stream unavailable"))?;
    let mut encoded_length = [0_u8; 4];
    stream
        .read_exact(&mut encoded_length)
        .await
        .map_err(|_| AdmissionRejection::new("truncated ticket length"))?;
    let length = u32::from_be_bytes(encoded_length) as usize;
    if length == 0 || length > MAX_SIGNED_CONNECTION_TICKET_BYTES {
        return Err(AdmissionRejection::new("invalid ticket length"));
    }
    let mut signed_ticket = vec![0_u8; length];
    stream
        .read_exact(&mut signed_ticket)
        .await
        .map_err(|_| AdmissionRejection::new("truncated signed ticket"))?;

    let mut trailing = [0_u8; 1];
    match stream.read(&mut trailing).await {
        Ok(None) => Ok(signed_ticket),
        Ok(Some(_)) => Err(AdmissionRejection::new("trailing ticket data")),
        Err(_) => Err(AdmissionRejection::new("ticket stream failed")),
    }
}

impl From<VerifiedTicket> for VerifiedConnectionClaims {
    fn from(ticket: VerifiedTicket) -> Self {
        Self {
            match_id: ticket.match_id,
            content_build_hash: ticket.content_build_hash,
            match_epoch: ticket.match_epoch,
            region: ticket.region,
            input_pool: ticket.input_pool,
            nonce: ticket.nonce,
            account_id: ticket.account_id,
            player_id: ticket.player_id,
            team_id: ticket.team_id,
            expires_at_unix_seconds: ticket.expires_at_unix_seconds,
        }
    }
}

impl From<VerifiedConnectionClaims> for VerifiedTicket {
    fn from(claims: VerifiedConnectionClaims) -> Self {
        Self {
            match_id: claims.match_id,
            content_build_hash: claims.content_build_hash,
            match_epoch: claims.match_epoch,
            region: claims.region,
            input_pool: claims.input_pool,
            nonce: claims.nonce,
            account_id: claims.account_id,
            player_id: claims.player_id,
            team_id: claims.team_id,
            expires_at_unix_seconds: claims.expires_at_unix_seconds,
        }
    }
}

/// Verifier used by a native process whose only admission path is the
/// transport-authenticated event pump.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoDirectTicketVerifier;

impl SignedTicketVerifier for NoDirectTicketVerifier {
    fn verify(
        &self,
        _signed_ticket: &[u8],
        _expected_build: MatchBuild,
        _expected_scope: MatchAdmissionScope,
        _now_unix_seconds: u64,
    ) -> Result<VerifiedTicket, TicketVerificationError> {
        Err(TicketVerificationError::BackendUnavailable)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RejectedQuicAdmission {
    pub peer: PeerId,
    pub error: AdmissionError,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct QuicConnectionPumpReport {
    pub authenticated_events: usize,
    pub admitted_players: usize,
    pub disconnected_events: usize,
    pub released_players: usize,
    pub rejected: Vec<RejectedQuicAdmission>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct QuicConnectionPumpError(pub TransportError);

impl fmt::Display for QuicConnectionPumpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "QUIC connection event transport failed: {}",
            self.0
        )
    }
}

impl Error for QuicConnectionPumpError {}

/// Applies every pending connection event before gameplay ingress.
///
/// Each authenticated event carries the exact claims returned by the one
/// signature-verification call. Rejected claims close the connection and never
/// establish a gameplay peer binding.
pub fn pump_quic_connection_events<V, R, S>(
    process: &mut DedicatedMatch<DedicatedQuicHost<NativeQuicTransport>, V, R, S>,
) -> Result<QuicConnectionPumpReport, QuicConnectionPumpError>
where
    V: SignedTicketVerifier,
    R: ReplayCheckpointSink,
    S: SettlementSink,
{
    let mut report = QuicConnectionPumpReport::default();
    loop {
        let event = {
            process
                .host_mut()
                .transport_mut()
                .poll_connection_event()
                .map_err(QuicConnectionPumpError)?
        };
        let Some(event) = event else {
            return Ok(report);
        };
        match event {
            ConnectionEvent::Authenticated(authenticated) => {
                report.authenticated_events += 1;
                let result = process.admit_verified(
                    authenticated.peer,
                    authenticated.claims.into(),
                    authenticated.verified_at_unix_seconds,
                );
                match result {
                    Ok(_) => report.admitted_players += 1,
                    Err(error) => {
                        report.rejected.push(RejectedQuicAdmission {
                            peer: authenticated.peer,
                            error,
                        });
                        let reason = if error == AdmissionError::Draining {
                            DisconnectReason::ServerDraining
                        } else {
                            DisconnectReason::ProtocolViolation
                        };
                        match process
                            .host_mut()
                            .transport_mut()
                            .disconnect(authenticated.peer, reason)
                        {
                            Ok(()) | Err(TransportError::InvalidPeer(_)) => {}
                            Err(error) => {
                                return Err(QuicConnectionPumpError(error));
                            }
                        }
                    }
                }
            }
            ConnectionEvent::Disconnected { peer } => {
                report.disconnected_events += 1;
                if process.disconnect_peer(peer) {
                    report.released_players += 1;
                }
            }
        }
    }
}
