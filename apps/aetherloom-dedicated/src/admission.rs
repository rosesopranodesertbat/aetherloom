use aetherloom_protocol::{InputPool, PlayerId, RegionId, TeamId};

use crate::{MatchAdmissionScope, MatchBuild};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifiedTicket {
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

/// Production implementations verify a signed, short-lived connection ticket.
///
/// The dedicated process never parses unsigned claims or owns signing keys.
pub trait SignedTicketVerifier {
    fn verify(
        &self,
        signed_ticket: &[u8],
        expected_build: MatchBuild,
        expected_scope: MatchAdmissionScope,
        now_unix_seconds: u64,
    ) -> Result<VerifiedTicket, TicketVerificationError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TicketVerificationError {
    Malformed,
    BadSignature,
    Expired,
    NotYetValid,
    UnknownKey,
    WrongIssuer,
    WrongAudience,
    WrongPurpose,
    WrongMatch,
    WrongBuild,
    WrongEpoch,
    WrongRegion,
    WrongInputPool,
    BackendUnavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionError {
    Draining,
    MatchAlreadyStarted,
    NoHeadroom,
    Ticket(TicketVerificationError),
    TicketExpired,
    WrongMatch,
    WrongBuild,
    WrongEpoch,
    WrongRegion,
    WrongInputPool,
    TicketReplayed,
    PlayerOutOfRange(PlayerId),
    PeerAlreadyBound,
    PlayerAlreadyConnected,
    AccountMismatch,
    TeamMismatch,
    ReconnectExpired,
    BotSlotOccupied(PlayerId),
    SimulationRejected,
}

impl From<TicketVerificationError> for AdmissionError {
    fn from(value: TicketVerificationError) -> Self {
        Self::Ticket(value)
    }
}
