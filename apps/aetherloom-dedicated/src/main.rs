use aetherloom_core::WorldSeed;
use aetherloom_dedicated::{
    DedicatedMatch, MatchBuild, NoopReplaySink, NoopSettlementSink, ProcessConfig,
    SignedTicketVerifier, TicketVerificationError, VerifiedTicket,
};
use aetherloom_server::{DedicatedQuicHost, LoopbackTransport, PeerId};

#[derive(Debug)]
struct RejectAllTickets;

impl SignedTicketVerifier for RejectAllTickets {
    fn verify(
        &self,
        _signed_ticket: &[u8],
        _expected_build: MatchBuild,
        _now_unix_seconds: u64,
    ) -> Result<VerifiedTicket, TicketVerificationError> {
        Err(TicketVerificationError::BackendUnavailable)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let build = MatchBuild::new([1; 16], [2; 16], 1)?;
    let config = ProcessConfig::new(build, WorldSeed::new(1), 16)?;
    let (_local_client, server_transport) =
        LoopbackTransport::pair(PeerId(1), PeerId(2), 32)?;
    let host = DedicatedQuicHost::new(server_transport)?;
    let process = DedicatedMatch::new(
        config,
        host,
        RejectAllTickets,
        NoopReplaySink,
        NoopSettlementSink,
    );
    let health = process.health();
    println!(
        "Aetherloom local loopback self-check: {:?}, {} reserved players.",
        health.state, health.reserved_players
    );
    println!(
        "Production startup is disabled: load an external Quinn TLS config and provide an audited signed-ticket verifier, persistence sink, and settlement sink."
    );
    Ok(())
}
