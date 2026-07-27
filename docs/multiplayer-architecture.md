# Aetherloom multiplayer implementation

This document records the executable foundation for the 128 Hz multiplayer
architecture. It distinguishes invariants already enforced by code from
production integrations that still need platform credentials, SDKs, or
capacity.

## Runtime topology

```text
native / console                           browser
shared Rust client boundary                compatibility / future Rust client
platform QUIC adapter                      binary WebSocket adapter
             \                             /
              aetherloom-server host boundary
                 regional match process
               apps/aetherloom-dedicated
              aetherloom-core @ 128 Hz
                         |
              replay evidence + signed results
                         |
Cloudflare Worker -> Durable Objects -> Queue -> D1 / R2
sessions/tickets      profiles/queues      projections/evidence
```

Cloudflare is the control plane. A Durable Object never carries the
high-frequency competitive data plane. `aetherloom-quic` is the concrete
Quinn/rustls server transport; `DedicatedQuicHost` and `CloudflareWssHost`
share the same `MatchHost` boundary, so gameplay does not know which transport
admitted a peer.

## Enforced invariants

- `AUTHORITATIVE_HZ` is 128 and one tick is 7,812,500 ns.
- A match has at most 128 addressable player slots.
- Commands contain only a target tick, sequence, quantized movement/look,
  action flags, and an optional requested spell.
- Command sets resolve in ascending `PlayerId` order.
- Entity identities contain a slot and a non-zero generation. Reusing a dense
  slot cannot make an old `EntityId` valid.
- Protocol values use explicit little-endian encoding. Raw Rust memory is not a
  wire or checkpoint format.
- Gameplay datagrams are rejected above 1,200 bytes.
- Input datagrams contain commands only. Account, player, and team identity
  come exclusively from the authenticated connection binding.
- Generation, AI, combat, environment, and cosmetic random streams are
  independent.
- Authoritative checkpoint bytes are versioned and validated before restore.
- Terrain is represented by revisioned chunks and explicit deformation events
  in the online core.
- Transport and worker queues are bounded and expose backpressure.
- The server scheduler uses absolute deadlines. Lateness causes catch-up, never
  skipped ticks or time dilation.
- Reliable terrain deltas and complete terrain chunks in keyframes let a
  replica detect a missing revision and replace stale terrain.
- Authoritative gameplay events use stable IDs so redundant delivery can be
  de-duplicated without replaying effects.
- Disconnect immediately clears transient input, changes to bot control at
  three seconds, and expires reconnection after 90 seconds.

The browser game in `src/` remains a compatibility adapter while presentation
moves behind the platform interfaces. Its JavaScript loop now calls
`advanceTick()` at 128 Hz and `extractFrame()` once per rendered frame. The
legacy `step(dt)` export ignores caller-provided time and advances one fixed
tick.

## Crates and services

| Path | Responsibility |
| --- | --- |
| `crates/aetherloom-protocol` | IDs, commands, envelopes, snapshots, events, terrain messages, results, strict codec |
| `crates/aetherloom-core` | Owned `MatchState`, bots, stable entity pool, chunked terrain, replay hash, checkpoints, replication and client replica |
| `crates/aetherloom-client` | 128 Hz prediction orchestration, 128/64 Hz input batching, adaptive interpolation, presentation frames, lifecycle, transport and vendor-console boundaries |
| `crates/aetherloom-server` | Host/transport interfaces, loopback transport, bounded worker channels, fixed-rate scheduling and disconnect policy |
| `crates/aetherloom-quic` | Quinn/rustls datagrams and streams, bounded async socket queues, authenticated connection lifecycle and draining |
| `apps/aetherloom-dedicated` | One build-pinned authoritative match, admission, replication, replay/checkpoint and settlement boundaries |
| `server/cloudflare` | Tickets, account/profile Durable Objects, sharded matchmaking, capacity direction, R2/D1/Queue settlement |
| `src` and `site` | Existing offline/browser compatibility game and WebGPU presentation |

The intended host loop is:

```rust
let permit = scheduler.wait_for_tick()?;
let commands = ingress.commands_for(permit.tick());
let events = match_state.advance_tick(commands);
let snapshots = replication.build(match_state, events);
egress.try_publish(snapshots)?;
scheduler.complete_tick(permit)?;
```

Network ingress must authenticate the peer-to-player binding before inserting
a command. It rejects replays, out-of-window ticks, invalid quantized fields,
and commands for any player other than the authenticated one.

## Replication policy

The core labels interest tiers; the host schedules them as follows:

| Data | Maximum frequency |
| --- | ---: |
| Local reconciliation and combat-critical nearby state | 128 Hz |
| Medium-distance entities | 32 Hz |
| Distant entities and objectives | 8 Hz |
| Browser input batches | 64 Hz |
| Browser snapshots | 32 Hz |

Snapshot datagrams are replaceable. A receiver discards obsolete snapshot
sequences, requests a reliable keyframe when its baseline is absent, and
retains only bounded history. The shared client measures arrival jitter with an
integer EWMA and adapts interpolation between 2-6 ticks (15.6-46.9 ms). It
retains a bounded remote-snapshot ring for actual position interpolation while
local prediction stays independent.

Before either replica mutates, the client pins every incoming envelope to the
active content build and match epoch. Snapshot datagrams, event datagrams, and
the reliable stream use separate replay windows so priority delivery of a
newer combat event cannot make an older-but-required snapshot baseline appear
obsolete.

Terrain deformations are reliable and revision checked. A revision gap requests
resynchronization; the next keyframe replaces the bounded client terrain cache
with complete chunk contents. Gameplay event IDs are retained in a bounded
de-duplication window so critical events can be repeated without applying
their presentation effects twice.

Instant targeting can query the bounded authoritative pose history. The ring
covers at least 256 ms, but validation refuses a rewind beyond 200 ms.
Projectiles and terrain deformation are never rewound.

## Persistence and match lifecycle

1. Each account Durable Object reserves its loadout with an idempotency key.
2. Matchmaking places the party in a shard derived from region, playlist,
   input pool, skill bucket, and shard number.
3. The match director reserves a healthy host pinned to the requested content
   build and commits every reservation.
4. The host accepts only short-lived signed join tickets for that match,
   region, input pool, and build.
5. Loot remains match-local until extraction.
6. The host derives an immutable result seal from its authoritative roster,
   outcomes, scores, and extracted loot. A trusted settlement sink signs that
   exact result and archives replay evidence.
7. Cloudflare Queue may redeliver the result; the profile object applies its
   economic effect exactly once and rejects conflicting reuse.
8. D1 and R2 projections are repaired idempotently after partial failure.

Browser parties are forced into `casual-extraction` with the browser input
pool. Competitive capacity returns a native QUIC endpoint; casual browser
capacity returns a WSS allocation forwarded through a fixed service binding.

## Platform boundary

Gameplay has no graphics, audio, account, storage, or socket dependency.
Presentation extraction emits typed player/entity/terrain data through
`RendererBackend`. `PlatformServices` owns identity, entitlement, storage,
achievements, suspend/resume, and device-loss reporting.

The web build can implement `RendererBackend` with WebGPU. Desktop can use
`wgpu` over Metal, D3D12, or Vulkan. Console builds implement the same boundary
through a vendor-owned static library/C ABI; no assumption is made that a
console SDK exposes `wgpu`.

Offline play uses `LoopbackTransport`. Its save namespace must remain distinct
from online profile inventory and ratings.

## Verification

Fast checks:

```bash
cargo test --workspace --all-targets
npm test
npm test --prefix server/cloudflare
```

Before valuable rewards or competitive queueing, run the production host
adapter against the full impairment matrix and the 60-minute, 128-combatant
performance gate. A passing gate requires:

- exactly 128 simulation ticks per second;
- p99 simulation duration below 4 ms;
- p99 tick-start jitter below 0.5 ms;
- no dropped ticks, time dilation, or unbounded queues;
- median egress below 96 KiB/s/client and p95 below 192 KiB/s/client.

The repository contains the production gate and a short deterministic CI
profile. A passing CI profile proves the threshold logic, not production host
capacity or Valorant-like latency. The real 60-minute gate still has to run on
the chosen regional hardware with the real match workload and network sinks.

## Remaining implementation and production integration

The repository deliberately does not invent production credentials or licensed
SDKs, and it does not label scaffolding as a shipped service. The following
work remains:

- production TLS configuration, certificate/key rotation, the audited ticket
  verifier, and wiring them into the dedicated executable;
- a native client socket adapter, production browser WSS match origin,
  reconnect-ticket refresh, and cross-region failover;
- regional capacity provisioning, a separate snapshot-generation worker, and
  production-class 16/32/64/128-player soak measurements;
- real Cloudflare resource IDs, secrets, migrations, service bindings, and
  deployment, plus live Durable Object/D1/R2/Queue fault-injection tests;
- session issuance, platform-account linking, and the currently unbound
  home-island Wasm ABI;
- migration of the legacy game’s complete spell, creature, economy and terrain
  rules into `MatchState`;
- the full Rust `wgpu` visual port and proprietary console backends;
- native/web cross-backend visual goldens and production telemetry export for
  RTT, loss, snapshot age, reconciliation, CPU, memory and bandwidth by region;
- platform identity, entitlement, moderation, voice/chat, and certification;
- licensed console SDK integration and certification.

Matches are build-pinned. Deployments must mark old hosts draining and allow
their active matches to finish rather than replacing code underneath a match.
