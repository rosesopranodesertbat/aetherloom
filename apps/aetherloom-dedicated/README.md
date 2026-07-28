# Aetherloom dedicated match process

This crate is the application layer for exactly one authoritative match. It
pins a match/content build, authenticates `PeerId` to `PlayerId` admission,
validates input envelopes, advances `aetherloom-core` once for every 128 Hz
`TickScheduler` permit, and produces bounded viewer-specific replication,
replay/checkpoint, and settlement work.

Implemented boundaries:

- signed admission-ticket verification;
- native QUIC ticket-preface verification with server-owned `PeerId`
  allocation and ordered authenticated/disconnected lifecycle events;
- health/headroom-gated admission for at most 128 players;
- replay, stale/future input, and peer-spoof rejection;
- deterministic spatial-grid queries and entity-order merges;
- reliable keyframes and explicit resynchronization;
- complete terrain chunks in keyframes plus reliable, revision-checked
  deformation deltas;
- versioned gameplay-event batches, with two-tick datagram redundancy for
  latency-critical cast/damage effects and reliable defeat/extraction events;
- per-session replication policy: native critical snapshots may run at 128 Hz,
  while browser/WSS snapshots are gated to 32 Hz;
- prioritized bounded egress that sheds cosmetics and distant replication
  before combat-critical work;
- per-flush egress fairness that skips a backpressured peer while continuing
  healthy peers, with a bounded per-peer service quantum;
- deterministic round-robin QUIC ingress, reserved per-peer queue shares, and
  a 32-message per-peer decode ceiling in each 128 Hz tick;
- one post-tick wire-entity materialization shared by every viewer, avoiding a
  full entity-map rebuild per client;
- immediate transient-input clearing, bot takeover after three seconds, and a
  90-second reconnect expiry;
- draining without changing the simulation rate;
- bounded, versioned input/state-hash replay records plus checkpoints and
  authoritative, idempotent result-settlement boundaries.

Health snapshots expose scheduler timing and bounded-queue pressure together
with ingress quota drops, replay/checkpoint drops, and whether settlement is
pending or finalized.
The QUIC adapter additionally exposes per-peer RTT, loss, congestion, MTU, and
traffic counters. Production exporters and regional aggregation remain
deployment-owned.

Native admission is deliberately split into two trust domains:

1. `TicketConnectionAdmission` reads one capped, length-prefixed signed ticket
   from the first client unidirectional stream.
2. The caller-owned `SignedTicketVerifier` verifies it exactly once against the
   process `MatchBuild` and immutable region/input-pool admission scope.
3. `SequentialPeerIdAllocator` assigns an opaque server-side connection id.
4. `NativeQuicTransport` emits the exact verified claims in one authenticated
   lifecycle event before accepting gameplay input.
5. `pump_quic_connection_events` calls `DedicatedMatch::admit_verified` once,
   pinning those claims to match, build, epoch, region, input pool, expiry,
   one-use nonce, capacity, account, player, and team.
6. Gameplay is authorized by the resulting `PeerId -> PlayerId` binding.
   Protocol version 4 input batches contain commands only—no unsigned account,
   player, or team identity.

The native process should use `NoDirectTicketVerifier`, which disables the
second/raw-ticket admission route. Its loop must call
`pump_quic_connection_events` before polling gameplay ingress each tick.

`Ed25519JoinTicketVerifier` is the production verify-only implementation. Its
JSON key set maps rotation key ids to unpadded base64url raw 32-byte Ed25519
public keys. It accepts the strict Cloudflare join-ticket v1 contract, verifies
the signature before parsing claims, then binds issuer, audience, nonzero
128-bit lowercase-hex match/build/account/nonce ids, nonzero epoch, validity
window, exact match region and input pool, player slot and team. It exposes no
signing API and never receives the Cloudflare PKCS#8 private key:

```rust
use aetherloom_dedicated::Ed25519JoinTicketVerifier;

let verifier = Ed25519JoinTicketVerifier::from_public_key_set_json(
    "aetherloom-control-plane",
    "aetherloom-match",
    r#"{"join-2026-07":"BASE64URL_RAW_32_BYTE_PUBLIC_KEY"}"#,
)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Authoritative settlement

`seal_match_result(result_id)` is the preferred finalization API. The caller
provides only a non-zero idempotency key. The process constructs the complete
`MatchResult` in stable `PlayerId` order:

- the player roster and team assignments come from verified admission
  reservations, so director-selected bots are never profile-settled;
- every admitted player must have a terminal authoritative outcome;
- reconnect-grace expiry captures a defeat result before the core slot is
  cleared;
- banked loot and score come from the authoritative banked-resource balance;
- non-extracted players receive no expedition loot; and
- rating delta is fixed to zero until a versioned authoritative rating policy
  is added. Rating projection may happen in the control plane, but callers
  cannot inject a delta into the signed match result.

The aggregate core resource balance is represented by stable loot item id `1`
until gameplay has an itemized authoritative loot ledger. `submit_match_result`
remains as a strict compatibility API: it recomputes the authoritative result
and accepts only an exact match. A pending or finalized result retains its full
content, so retries with the same id but altered outcome, roster, score, rating,
or loot are rejected. `SettlementSink` receives this immutable seal; evidence
storage/signing and Cloudflare delivery occur after that trust boundary and
must not rewrite it.

## Staging executable

The binary is a real one-match QUIC process. It loads a PEM certificate and
private key, an Ed25519 join-ticket public-key set, and immutable match/build
configuration from command-line options or `AETHERLOOM_*` environment
variables. It never generates a credential or receives a ticket-signing private
key. Run `cargo run -p aetherloom-dedicated -- --help` for the complete
configuration contract.

The lobby accepts verified reservations until `minimum-humans` is reached or
the lobby deadline expires. If no human reservation ever arrived by the
deadline, the process cancels without creating a simulation tick or a bot-only
match. Once at least one reservation has arrived, the deadline may start the
match and fill vacant combatant slots with bots. New humans cannot enter after
that insertion barrier.

A successfully admitted ticket nonce is one-use for the lifetime of the
ticket. Reconnect retains the existing 90-second reservation semantics, but the
control plane must issue a fresh ticket with a fresh nonce for every new
connection attempt.

The process exposes:

- `GET /livez`, `GET /readyz`, and `GET /healthz` on the configured health
  listener;
- an atomic JSON health file for a same-host capacity agent;
- `POST /drain` only when a 32-byte-or-longer bearer token is mounted through
  `AETHERLOOM_ADMIN_TOKEN_FILE`; and
- a drain-marker file suitable for a container `preStop` hook.

During drain, new QUIC handshakes stop while existing clients continue for the
configured grace period. Remaining active reservations become authoritative
abandonments, the immutable result is sealed, and shutdown waits for durable
handoff acknowledgements.

`FileReplaySpool` performs fsynced WAL writes on a dedicated worker. Its
filename and versioned header bind match id, content build hash, and nonzero
match epoch; reopening a mismatched identity fails closed. The simulation
retains each bounded replay chunk until that worker acknowledges durability,
so disk latency never blocks a 128 Hz tick. `FileSettlementSpool`
uses an atomic no-replace ready file keyed by `result_id`; identical delivery is
idempotent and conflicting content is rejected. A host agent must upload these
ready artifacts to R2/Queues and delete them only after the remote service
acknowledges them.

The deployment supervisor must call the authenticated drain endpoint (or create
the drain marker) before sending its final process-termination signal. Direct
OS-signal interception is intentionally not hidden inside the gameplay binary,
and abruptly killing the process bypasses settlement finalization. Match
restart/restore and the remote spool uploader remain host-agent responsibilities.

Run the isolated crate tests with:

```sh
cargo test --manifest-path apps/aetherloom-dedicated/Cargo.toml
```

The `quic_admission` integration test uses generated localhost certificates
only inside the test and proves that signature verification occurs once, the
same claims reach the match, an unbound connection cannot submit the
identity-free gameplay payload, and the disconnect event releases the binding.
