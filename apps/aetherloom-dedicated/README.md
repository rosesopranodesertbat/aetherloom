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
   process `MatchBuild`.
3. `SequentialPeerIdAllocator` assigns an opaque server-side connection id.
4. `NativeQuicTransport` emits the exact verified claims in one authenticated
   lifecycle event before accepting gameplay input.
5. `pump_quic_connection_events` calls `DedicatedMatch::admit_verified` once,
   pinning those claims to match, build, epoch, expiry, capacity, account,
   player, and team.
6. Gameplay is authorized by the resulting `PeerId -> PlayerId` binding.
   Protocol version 2 input batches contain commands only—no unsigned account,
   player, or team identity.

The native process should use `NoDirectTicketVerifier`, which disables the
second/raw-ticket admission route. Its loop must call
`pump_quic_connection_events` before polling gameplay ingress each tick.

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

The Quinn/rustls backend is implemented in `aetherloom-quic`, but this crate
still does **not** invent a certificate, private key, certificate policy,
ticket-signing key, Cloudflare credential, replay store, or settlement client.
The executable remains a loopback configuration check. Production startup must
load its Quinn `ServerConfig` externally and supply the audited verifier and
sinks:

```rust,no_run
use std::sync::Arc;
use aetherloom_dedicated::{
    NoDirectTicketVerifier, SequentialPeerIdAllocator, SystemUnixTime,
    TicketConnectionAdmission,
};
use aetherloom_quic::{NativeQuicTransport, QuicTransportConfig};

# fn wire<V>(
#   server_tls: quinn::ServerConfig,
#   verifier: V,
#   build: aetherloom_dedicated::MatchBuild,
# ) -> Result<NativeQuicTransport, Box<dyn std::error::Error>>
# where V: aetherloom_dedicated::SignedTicketVerifier + Send + Sync + 'static {
let admission = TicketConnectionAdmission::new(
    verifier,
    build,
    SystemUnixTime,
    SequentialPeerIdAllocator::new(1),
);
let transport = NativeQuicTransport::bind(
    server_tls,
    QuicTransportConfig::default(),
    Arc::new(admission),
)?;
# let _verified_only = NoDirectTicketVerifier;
# Ok(transport)
# }
```

Run the isolated crate tests with:

```sh
cargo test --manifest-path apps/aetherloom-dedicated/Cargo.toml
```

The `quic_admission` integration test uses generated localhost certificates
only inside the test and proves that signature verification occurs once, the
same claims reach the match, an unbound connection cannot submit the
identity-free gameplay payload, and the disconnect event releases the binding.
