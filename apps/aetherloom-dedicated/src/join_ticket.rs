use std::collections::BTreeMap;

use aetherloom_protocol::{InputPool, PlayerId, RegionId, TeamId};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ring::signature::{UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};

use crate::{
    MatchAdmissionScope, MatchBuild, SignedTicketVerifier, TicketVerificationError, VerifiedTicket,
};

const MAX_SIGNED_TICKET_BYTES: usize = 4_096;
const MAX_KEYS: usize = 8;
const MAX_JOIN_TICKET_LIFETIME_SECONDS: u64 = 120;
const CLOCK_SKEW_SECONDS: u64 = 5;
const MAX_JAVASCRIPT_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug)]
pub struct Ed25519JoinTicketVerifier {
    expected_issuer: String,
    expected_audience: String,
    public_keys: BTreeMap<String, [u8; 32]>,
}

impl Ed25519JoinTicketVerifier {
    /// Builds a verify-only admission boundary from raw Ed25519 public keys.
    ///
    /// `public_key_set_json` is a JSON object from rotation key id to an
    /// unpadded base64url encoding of the raw 32-byte public key. It contains
    /// no signing material and is safe to provision to mutually untrusted
    /// match hosts.
    pub fn from_public_key_set_json(
        expected_issuer: &str,
        expected_audience: &str,
        public_key_set_json: &str,
    ) -> Result<Self, TicketVerifierConfigError> {
        if !is_identifier(expected_issuer) {
            return Err(TicketVerifierConfigError::InvalidIssuer);
        }
        if !is_identifier(expected_audience) {
            return Err(TicketVerifierConfigError::InvalidAudience);
        }
        let encoded_keys: BTreeMap<String, String> = serde_json::from_str(public_key_set_json)
            .map_err(|_| TicketVerifierConfigError::InvalidJson)?;
        if encoded_keys.is_empty() {
            return Err(TicketVerifierConfigError::EmptyKeySet);
        }
        if encoded_keys.len() > MAX_KEYS {
            return Err(TicketVerifierConfigError::TooManyKeys);
        }

        let mut public_keys = BTreeMap::new();
        for (key_id, encoded_key) in encoded_keys {
            if !is_identifier(&key_id) {
                return Err(TicketVerifierConfigError::InvalidKeyId);
            }
            let decoded = decode_canonical_base64url(&encoded_key)
                .ok_or(TicketVerifierConfigError::InvalidPublicKey)?;
            let key: [u8; 32] = decoded
                .try_into()
                .map_err(|_| TicketVerifierConfigError::InvalidPublicKey)?;
            public_keys.insert(key_id, key);
        }

        Ok(Self {
            expected_issuer: expected_issuer.to_owned(),
            expected_audience: expected_audience.to_owned(),
            public_keys,
        })
    }

    pub fn key_count(&self) -> usize {
        self.public_keys.len()
    }
}

impl SignedTicketVerifier for Ed25519JoinTicketVerifier {
    fn verify(
        &self,
        signed_ticket: &[u8],
        expected_build: MatchBuild,
        expected_scope: MatchAdmissionScope,
        now_unix_seconds: u64,
    ) -> Result<VerifiedTicket, TicketVerificationError> {
        if signed_ticket.is_empty() || signed_ticket.len() > MAX_SIGNED_TICKET_BYTES {
            return Err(TicketVerificationError::Malformed);
        }
        let token =
            std::str::from_utf8(signed_ticket).map_err(|_| TicketVerificationError::Malformed)?;
        let mut parts = token.split('.');
        let header_part = parts.next().ok_or(TicketVerificationError::Malformed)?;
        let claims_part = parts.next().ok_or(TicketVerificationError::Malformed)?;
        let signature_part = parts.next().ok_or(TicketVerificationError::Malformed)?;
        if parts.next().is_some() {
            return Err(TicketVerificationError::Malformed);
        }

        let header_bytes =
            decode_canonical_base64url(header_part).ok_or(TicketVerificationError::Malformed)?;
        require_canonical_json(&header_bytes)?;
        let header: JoinTicketHeader = serde_json::from_slice(&header_bytes)
            .map_err(|_| TicketVerificationError::Malformed)?;
        if header.v != 1 || header.alg != "EdDSA" || header.typ != "AETHERLOOM-JOIN" {
            return Err(TicketVerificationError::Malformed);
        }
        if !is_identifier(&header.kid) {
            return Err(TicketVerificationError::Malformed);
        }
        let public_key = self
            .public_keys
            .get(&header.kid)
            .ok_or(TicketVerificationError::UnknownKey)?;
        let signature =
            decode_canonical_base64url(signature_part).ok_or(TicketVerificationError::Malformed)?;
        if signature.len() != 64 {
            return Err(TicketVerificationError::Malformed);
        }
        let mut signing_input = String::with_capacity(header_part.len() + claims_part.len() + 1);
        signing_input.push_str(header_part);
        signing_input.push('.');
        signing_input.push_str(claims_part);
        UnparsedPublicKey::new(&ED25519, public_key)
            .verify(signing_input.as_bytes(), &signature)
            .map_err(|_| TicketVerificationError::BadSignature)?;

        let claims_bytes =
            decode_canonical_base64url(claims_part).ok_or(TicketVerificationError::Malformed)?;
        require_canonical_json(&claims_bytes)?;
        let claims: JoinTicketClaims = serde_json::from_slice(&claims_bytes)
            .map_err(|_| TicketVerificationError::Malformed)?;
        claims.validate(self, expected_build, expected_scope, now_unix_seconds)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum TicketVerifierConfigError {
    InvalidJson,
    InvalidIssuer,
    InvalidAudience,
    EmptyKeySet,
    TooManyKeys,
    InvalidKeyId,
    InvalidPublicKey,
}

impl std::fmt::Display for TicketVerifierConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for TicketVerifierConfigError {}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct JoinTicketHeader {
    alg: String,
    kid: String,
    typ: String,
    v: u8,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct JoinTicketClaims {
    aud: String,
    build_hash: String,
    exp: u64,
    iat: u64,
    input_pool: String,
    iss: String,
    match_epoch: u64,
    match_id: String,
    nbf: u64,
    nonce: String,
    player_slot: u16,
    purpose: String,
    region: String,
    sub: String,
    team_id: u16,
    v: u8,
}

impl JoinTicketClaims {
    fn validate(
        self,
        verifier: &Ed25519JoinTicketVerifier,
        expected_build: MatchBuild,
        expected_scope: MatchAdmissionScope,
        now_unix_seconds: u64,
    ) -> Result<VerifiedTicket, TicketVerificationError> {
        if self.v != 1 || self.purpose != "join" {
            return Err(TicketVerificationError::WrongPurpose);
        }
        if self.iss != verifier.expected_issuer {
            return Err(TicketVerificationError::WrongIssuer);
        }
        if self.aud != verifier.expected_audience {
            return Err(TicketVerificationError::WrongAudience);
        }
        if !is_identifier(&self.iss) || !is_identifier(&self.aud) {
            return Err(TicketVerificationError::Malformed);
        }
        let region = RegionId::new(&self.region).map_err(|_| TicketVerificationError::Malformed)?;
        let input_pool = InputPool::from_name(&self.input_pool)
            .map_err(|_| TicketVerificationError::Malformed)?;
        if self.iat > MAX_JAVASCRIPT_SAFE_INTEGER
            || self.nbf > MAX_JAVASCRIPT_SAFE_INTEGER
            || self.exp > MAX_JAVASCRIPT_SAFE_INTEGER
            || self.match_epoch > MAX_JAVASCRIPT_SAFE_INTEGER
            || self.match_epoch == 0
            || self.exp <= self.iat
            || self.nbf >= self.exp
            || self.exp.saturating_sub(self.iat) > MAX_JOIN_TICKET_LIFETIME_SECONDS
        {
            return Err(TicketVerificationError::Malformed);
        }
        if self.iat > now_unix_seconds.saturating_add(CLOCK_SKEW_SECONDS)
            || self.nbf > now_unix_seconds.saturating_add(CLOCK_SKEW_SECONDS)
        {
            return Err(TicketVerificationError::NotYetValid);
        }
        if self.exp <= now_unix_seconds {
            return Err(TicketVerificationError::Expired);
        }

        let match_id = decode_hex_128(&self.match_id)?;
        let content_build_hash = decode_hex_128(&self.build_hash)?;
        let account_id = decode_hex_128(&self.sub)?;
        let nonce = decode_hex_128(&self.nonce)?;
        if match_id != expected_build.match_id() {
            return Err(TicketVerificationError::WrongMatch);
        }
        if content_build_hash != expected_build.content_build_hash() {
            return Err(TicketVerificationError::WrongBuild);
        }
        if self.match_epoch != expected_build.match_epoch() {
            return Err(TicketVerificationError::WrongEpoch);
        }
        if region != expected_scope.region() {
            return Err(TicketVerificationError::WrongRegion);
        }
        if input_pool != expected_scope.input_pool() {
            return Err(TicketVerificationError::WrongInputPool);
        }

        let player_id =
            PlayerId::new(self.player_slot).map_err(|_| TicketVerificationError::Malformed)?;
        let team_id = TeamId::new(self.team_id).map_err(|_| TicketVerificationError::Malformed)?;
        Ok(VerifiedTicket {
            match_id,
            content_build_hash,
            match_epoch: self.match_epoch,
            region,
            input_pool,
            nonce,
            account_id,
            player_id,
            team_id,
            expires_at_unix_seconds: self.exp,
        })
    }
}

fn require_canonical_json(bytes: &[u8]) -> Result<(), TicketVerificationError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| TicketVerificationError::Malformed)?;
    let canonical = serde_json::to_vec(&value).map_err(|_| TicketVerificationError::Malformed)?;
    if canonical != bytes {
        return Err(TicketVerificationError::Malformed);
    }
    Ok(())
}

fn decode_canonical_base64url(value: &str) -> Option<Vec<u8>> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return None;
    }
    let decoded = URL_SAFE_NO_PAD.decode(value).ok()?;
    if URL_SAFE_NO_PAD.encode(&decoded) != value {
        return None;
    }
    Some(decoded)
}

fn decode_hex_128(value: &str) -> Result<[u8; 16], TicketVerificationError> {
    if value.len() != 32 {
        return Err(TicketVerificationError::Malformed);
    }
    let mut bytes = [0_u8; 16];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        bytes[index] = (hex_nibble(pair[0]).ok_or(TicketVerificationError::Malformed)? << 4)
            | hex_nibble(pair[1]).ok_or(TicketVerificationError::Malformed)?;
    }
    if bytes.iter().all(|byte| *byte == 0) {
        return Err(TicketVerificationError::Malformed);
    }
    Ok(bytes)
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn is_identifier(value: &str) -> bool {
    if value.is_empty() || value.len() > 128 || !value.is_ascii() {
        return false;
    }
    value.bytes().enumerate().all(|(index, byte)| {
        byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'_' | b'.' | b':' | b'-'))
    })
}
