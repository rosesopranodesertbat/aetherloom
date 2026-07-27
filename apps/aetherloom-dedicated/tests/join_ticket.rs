use aetherloom_dedicated::{
    Ed25519JoinTicketVerifier, MatchAdmissionScope, MatchBuild, SignedTicketVerifier,
    TicketVerificationError, TicketVerifierConfigError,
};
use aetherloom_protocol::{InputPool, PlayerId, RegionId, TeamId};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ring::signature::Ed25519KeyPair;
use serde::Deserialize;

const FIXTURE_JSON: &str = include_str!("../../../server/cloudflare/fixtures/join-ticket-v1.json");
const NOW: u64 = 2_000_000_001;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    private_key_pkcs8_base64_url: String,
    public_key_raw_base64_url: String,
    token: String,
}

fn fixture() -> Fixture {
    serde_json::from_str(FIXTURE_JSON).expect("fixture")
}

fn build() -> MatchBuild {
    MatchBuild::new([0x11; 16], [0x22; 16], 42).expect("build")
}

fn scope() -> MatchAdmissionScope {
    MatchAdmissionScope::new(
        RegionId::new("weur").expect("region"),
        InputPool::Controller,
    )
}

fn verifier() -> Ed25519JoinTicketVerifier {
    let fixture = fixture();
    Ed25519JoinTicketVerifier::from_public_key_set_json(
        "aetherloom-control-plane",
        "aetherloom-match",
        &format!(
            r#"{{"test-ed25519-v1":"{}"}}"#,
            fixture.public_key_raw_base64_url
        ),
    )
    .expect("verifier")
}

fn resign_claims(mutator: impl FnOnce(&mut serde_json::Value)) -> String {
    let fixture = fixture();
    let parts: Vec<_> = fixture.token.split('.').collect();
    assert_eq!(parts.len(), 3);
    let claims_bytes = URL_SAFE_NO_PAD.decode(parts[1]).expect("claims");
    let mut claims: serde_json::Value = serde_json::from_slice(&claims_bytes).expect("claims json");
    mutator(&mut claims);
    let claims_part =
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).expect("canonical claims"));
    let signing_input = format!("{}.{}", parts[0], claims_part);
    let private_key = URL_SAFE_NO_PAD
        .decode(fixture.private_key_pkcs8_base64_url)
        .expect("private key");
    let key_pair =
        Ed25519KeyPair::from_pkcs8_maybe_unchecked(&private_key).expect("test signing key");
    let signature = URL_SAFE_NO_PAD.encode(key_pair.sign(signing_input.as_bytes()).as_ref());
    format!("{signing_input}.{signature}")
}

#[test]
fn cloudflare_fixture_verifies_to_exact_rust_claims() {
    let fixture = fixture();
    let ticket = verifier()
        .verify(fixture.token.as_bytes(), build(), scope(), NOW)
        .expect("valid ticket");
    assert_eq!(ticket.match_id, [0x11; 16]);
    assert_eq!(ticket.content_build_hash, [0x22; 16]);
    assert_eq!(ticket.match_epoch, 42);
    assert_eq!(ticket.region.as_str(), "weur");
    assert_eq!(ticket.input_pool, InputPool::Controller);
    assert_eq!(ticket.nonce, [0x44; 16]);
    assert_eq!(ticket.account_id, [0x33; 16]);
    assert_eq!(ticket.player_id, PlayerId::new(7).expect("player"));
    assert_eq!(ticket.team_id, TeamId::new(3).expect("team"));
    assert_eq!(ticket.expires_at_unix_seconds, 2_000_000_120);
}

#[test]
fn verifier_binds_match_build_epoch_and_time() {
    let fixture = fixture();
    let verifier = verifier();
    assert_eq!(
        verifier.verify(
            fixture.token.as_bytes(),
            MatchBuild::new([0x55; 16], [0x22; 16], 42).expect("build"),
            scope(),
            NOW,
        ),
        Err(TicketVerificationError::WrongMatch),
    );
    assert_eq!(
        verifier.verify(
            fixture.token.as_bytes(),
            MatchBuild::new([0x11; 16], [0x55; 16], 42).expect("build"),
            scope(),
            NOW,
        ),
        Err(TicketVerificationError::WrongBuild),
    );
    assert_eq!(
        verifier.verify(
            fixture.token.as_bytes(),
            MatchBuild::new([0x11; 16], [0x22; 16], 43).expect("build"),
            scope(),
            NOW,
        ),
        Err(TicketVerificationError::WrongEpoch),
    );
    assert_eq!(
        verifier.verify(fixture.token.as_bytes(), build(), scope(), 2_000_000_120),
        Err(TicketVerificationError::Expired),
    );
    assert_eq!(
        verifier.verify(fixture.token.as_bytes(), build(), scope(), 1_999_999_990),
        Err(TicketVerificationError::NotYetValid),
    );
}

#[test]
fn verifier_binds_region_and_input_pool() {
    let fixture = fixture();
    let verifier = verifier();
    assert_eq!(
        verifier.verify(
            fixture.token.as_bytes(),
            build(),
            MatchAdmissionScope::new(
                RegionId::new("eeur").expect("region"),
                InputPool::Controller,
            ),
            NOW,
        ),
        Err(TicketVerificationError::WrongRegion)
    );
    assert_eq!(
        verifier.verify(
            fixture.token.as_bytes(),
            build(),
            MatchAdmissionScope::new(
                RegionId::new("weur").expect("region"),
                InputPool::MouseKeyboard,
            ),
            NOW,
        ),
        Err(TicketVerificationError::WrongInputPool)
    );
}

#[test]
fn malformed_signature_unknown_fields_and_noncanonical_ids_are_rejected() {
    let fixture = fixture();
    let mut tampered = fixture.token.into_bytes();
    let last = tampered.last_mut().expect("signature");
    *last = if *last == b'A' { b'B' } else { b'A' };
    assert_eq!(
        verifier().verify(&tampered, build(), scope(), NOW),
        Err(TicketVerificationError::BadSignature),
    );

    let extension = resign_claims(|claims| {
        claims
            .as_object_mut()
            .expect("object")
            .insert("admin".to_owned(), serde_json::Value::Bool(true));
    });
    assert_eq!(
        verifier().verify(extension.as_bytes(), build(), scope(), NOW),
        Err(TicketVerificationError::Malformed),
    );

    let uppercase = resign_claims(|claims| {
        claims["match_id"] =
            serde_json::Value::String("1111111111111111111111111111111A".to_owned());
    });
    assert_eq!(
        verifier().verify(uppercase.as_bytes(), build(), scope(), NOW),
        Err(TicketVerificationError::Malformed),
    );
}

#[test]
fn verifier_configuration_accepts_only_canonical_public_key_sets() {
    assert_eq!(
        Ed25519JoinTicketVerifier::from_public_key_set_json(
            "aetherloom-control-plane",
            "aetherloom-match",
            "{}",
        )
        .expect_err("empty key set"),
        TicketVerifierConfigError::EmptyKeySet,
    );
    assert_eq!(
        Ed25519JoinTicketVerifier::from_public_key_set_json(
            "aetherloom-control-plane",
            "aetherloom-match",
            r#"{"bad key":"AA"}"#,
        )
        .expect_err("bad key id"),
        TicketVerifierConfigError::InvalidKeyId,
    );
    assert_eq!(verifier().key_count(), 1);
}
