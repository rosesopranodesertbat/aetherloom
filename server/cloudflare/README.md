# Aetherloom Cloudflare control plane

This directory contains the deployable control plane and a separate,
staging-only browser match Worker. The control-plane Worker is not a match
server; competitive 128 Hz simulation remains behind replaceable service
bindings:

- `MATCH_CAPACITY_API` reserves and releases regional WSS or QUIC match hosts.
- `BROWSER_MATCH_ORIGIN` receives authenticated browser WebSocket upgrades.

For the playable browser systems test, `src/browser-match-worker.ts` binds
`BROWSER_MATCH_ORIGIN` to a private, content-build-versioned Worker with one
`BrowserMatchRoom` Durable Object per room. The new Worker is deployed before
the gateway binding changes, leaving the previous build available if rollout
fails and allowing old sessions to drain. It runs the real Rust `MatchState` through
`generated/aetherloom-worker-match.wasm`, caps rooms at eight players, accepts
64 Hz binary input, and emits 32 Hz snapshots. The host advances fixed 128 Hz
simulation ticks in bounded wall-clock batches; Durable Objects do not provide
the continuous 7.8125 ms pacing or jitter guarantee required for competitive
play. This host is therefore explicitly casual staging, with progression and
settlement disabled. New public claims are source-limited, resume tokens rotate,
and a systems-test session expires after 15 minutes. A host-memory restart
closes its sockets and starts a fresh round on reconnect; this slice does not
claim mid-round checkpoint continuity.

The Worker owns short-lived authentication, profile and island serialization,
matchmaking coordination, loadout escrow, settlement delivery, and durable
projections. Competitive native/console traffic connects directly to a regional
QUIC host after receiving a signed join ticket.

No credentials are committed. Public staging resource identifiers and
fingerprints are committed so deployment can fail closed if an account or
resource binding drifts; production identifiers remain operator-supplied.
Nothing in this scaffold deploys automatically.

## Components

| Component | Responsibility |
| --- | --- |
| Gateway Worker | Origin checks, signed tickets, public API, internal service API, WSS handoff |
| `ProfileIslandObject` | One SQLite-backed object per account; inventory, reservations, settlement deduplication, island head |
| Headless island boundary | Command-driven 128 Hz tick batching, journal replay, idle checkpointing, dormant timestamp settlement |
| `MatchmakingShardObject` | Region/playlist/input/MMR/shard queue, bounded leases, per-member private assignment |
| Match director | Replaceable capacity service contract and D1 allocation record |
| `BrowserMatchRoom` | Private staging systems-test room, signed admission, authoritative Rust/Wasm duel, reconnect and bot takeover |
| D1 | Searchable projections, allocation state, match history |
| R2 | Private island checkpoints and immutable settlement receipts/replay references |
| Queue | At-least-once settlement delivery with a dead-letter queue |

Durable Objects use the current declarative `exports` lifecycle configuration
with SQLite storage. D1's ordered schema changes are in `migrations/`.

## Trust boundaries

- Public player requests require a compact HMAC-SHA256 session ticket.
- Join admission is signed with Ed25519. Only the control plane receives the
  PKCS#8 private key; WSS gateways and dedicated match hosts receive raw public
  keys and therefore cannot mint admission.
- Browser WebSockets carry `aetherloom.v2` and
  `aetherloom.auth.<join-ticket>` in `Sec-WebSocket-Protocol`; the gateway strips
  the auth pseudo-protocol before forwarding.
- Internal requests require a short-lived service ticket and an operation
  scope.
- A settlement additionally includes a signed `result` ticket from a dedicated
  result-key domain. Its `result_hash`, `settlement_id`, and `match_id` must
  match the canonical body; its subject and verified key id must map to the
  `host_id` recorded for that match allocation.
- Competitive settlements must also name
  `match.result.<matchId>.<settlementId>.<sha256>.json`. Before enqueueing, the
  control plane reads that R2 object, hashes its bytes, and verifies metadata
  binding the match, settlement, signed result hash, host, build, and evidence
  hash. Casual results may omit evidence; supplied evidence gets the same
  checks.
- Clients never choose rating/MMR. Matchmaking reads each party member's
  serialized profile.
- Parties and their required input pool come from signed session claims. Web
  parties are restricted to `casual-extraction` and the browser pool.

Symmetric session, service and result tickets have the shape documented by
`schemas/ticket-claims.schema.json`. Their key-set secrets are JSON maps from a
rotation key id to at least 32 random base64url bytes. Join tickets use the
strict, extension-free `schemas/join-ticket-claims.schema.json` contract and
an `EdDSA` / `AETHERLOOM-JOIN` compact header. `JOIN_TICKET_SIGNING_KEYS_JSON`
maps key ids to base64url Ed25519 PKCS#8 private DER and stays in the control
plane. `JOIN_TICKET_PUBLIC_KEYS_JSON` maps the same ids to base64url raw
32-byte public keys; this is the only join-key material provisioned to match
hosts. The public map and active join/service key ids are ordinary Wrangler
variables; only private signing and HMAC maps are secrets. Player, join,
service, and result key domains are deliberately
independent. `RESULT_TICKET_HOSTS_JSON` binds
each result key id to exactly one capacity `host_id`; a result signer sets that
host id as the ticket subject. The control plane keeps the complete verifier
map, while each match host receives only its own result-signing secret. Never
distribute the full result key set to a host. A result key can never
authenticate an internal service request.

Join `match_id`, `build_hash`, account `sub`, and `nonce` values are nonzero
128-bit values encoded as exactly 32 lowercase hexadecimal characters. The
release pipeline derives `build_hash` as the first 16 bytes of SHA-256 over its
documented canonical release identity and supplies that exact value to both
Cloudflare and the dedicated `MatchBuild`; hand-authored build labels are not
valid. The
`match_epoch` is a nonzero JavaScript-safe integer and is pinned in the
dispatch hash, capacity request, D1 allocation, ticket, Rust `MatchBuild`, and
every gameplay envelope. Join tickets live for at most 120 seconds and contain
exactly the documented claims; unknown claims, alternate encodings, uppercase
hex, stale epochs, and noncanonical JSON/base64url are rejected.

Content addressing detects mismatched evidence at acceptance; it is not an R2
object-lock feature. Limit evidence-prefix writes to the match-host ingestion
path and apply retention/versioning controls appropriate to the audit window.

## Exactly-once economic effect

Cloudflare Queues deliver at least once. Reward safety therefore comes from
idempotency rather than delivery assumptions:

1. The match result body is hashed and verified against its result ticket.
2. Competitive result evidence is content-addressed and verified before the
   economic command reaches the Queue.
3. `settlementId` and `reservationId` are unique inside the account Durable
   Object.
4. A replay with the same IDs and hash returns the stored receipt.
5. A replay with different content is rejected.
6. R2 receipts and D1 projections use the same settlement id and hash.
7. If R2 or D1 fails after the profile mutation, Queue retry receives the
   cached profile receipt and resumes projection.

Reservation semantics are explicit: extraction returns the reserved loadout and
banks loot; defeat or abandonment consumes the loadout and banks no loot.
Queue reservations expire unless match dispatch commits them. A failed
dispatch before journaling releases its capacity and queue lease. Once
journaled, the exact roster stays pinned and retries resume it. Commits run
sequentially, and rollback waits for every attempted commit (including one
whose acknowledgement may have been lost). If its signed assignments expire,
the queue roster is atomically cancelled before all journaled reservations are
compensated and capacity is released.

## API outline

Public routes:

- `GET /healthz`
- `GET /v1/profile`
- `POST /v1/profile/reservations`
- `POST /v1/profile/reservations/cancel`
- `GET /v1/profile/island`
- `GET /v1/profile/island/checkpoint`
- `GET /v1/profile/island/runtime`
- `POST /v1/profile/island/commands`
- `POST /v1/profile/island/deactivate`
- `POST /v1/matchmaking/enqueue`
- `POST /v1/matchmaking/status`
- `POST /v1/matchmaking/cancel`
- `GET /v1/ws/casual/:matchId` (WebSocket upgrade)
- `POST /v1/demo/join` (staging only; ephemeral account and empty loadout)
- `GET /v1/demo/ws/:matchId` (staging-only WebSocket upgrade)

Internal routes:

- `POST /internal/v1/demo/smoke/join`, scope `deployment:verify`; creates an
  isolated random room for the two-client deployment proof
- `POST /internal/v1/matchmaking/dispatch`, scope `match:dispatch`
- `POST /internal/v1/settlements`, scope `result:enqueue`, plus a signed result
- `PUT /internal/v1/islands/checkpoints/:checkpointId`, scope
  `island:checkpoint`
- `POST /internal/v1/readiness`, scope `deployment:verify`; returns only
  aggregate readiness after HMAC and Ed25519 round trips across all private key
  domains

Mutation requests use 16-128 character `Idempotency-Key` values. The internal
island upload is intentionally unavailable to player session tickets: allowing
a client to author an online checkpoint would let an offline or modified save
enter shared progression. Its trusted service caller also supplies:

- `X-Aetherloom-Account-Id`
- `X-Aetherloom-Expected-Version`
- `X-Aetherloom-Logical-Time-Ms`
- `X-Aetherloom-Content-Build`

The R2 object key includes the account id and the checkpoint bytes' SHA-256,
not the retry/idempotency id. The object is written before the Durable Object
advances the island head, so a failed compare-and-swap can leave an
unreferenced object but can never replace the bytes referenced by a live head.
Production operations should periodically reconcile the private
`island.*.bin` prefix against current heads before deleting old objects; never
apply an unconditional age-only lifecycle rule to that prefix.

### Dispatch recovery

`match_allocations` binds each `match_id` to hashes of one immutable dispatch
request and one exact roster. Before queue publication, D1 journals that
roster's lease, reservations, and signed assignments. Matchmaking then stores
the same assignments as a prepared hold; only after that hold exists does D1
mark the allocation active, followed by an idempotent queue confirmation and a
D1 confirmation timestamp.

A retry with the same dispatch resumes this journal and cannot claim or sign
for another roster. A retry after a lost confirmation response compares the
matched queue receipt and succeeds idempotently. Prepared leases that reach
their short-lived ticket expiry remain pinned instead of silently returning
committed players to matchmaking. The next retry moves D1 into an aborting
state, atomically cancels the prepared queue roster (or observes that
confirmation already won), compensates the journaled reservations, and then
releases capacity. Each compensation also writes a per-reservation,
per-match abort tombstone, so a delayed or concurrent commit for the aborted
match cannot recreate escrow after rollback.

### Home-island runtime lifecycle

`src/island-runtime.ts` is the replaceable boundary between the control plane
and the platform-neutral Rust simulation. Its Wasm ABI is deliberately
headless: fixed input and output buffers, restore, batched tick advance, events,
and checkpoint exports. Every accepted command batch:

1. derives due ticks from elapsed wall time using an integer remainder at
   exactly 128 Hz;
2. advances at most 32 ticks in one delivery and durably carries the remaining
   tick debt into later batches instead of discarding or time-dilating it;
3. journals the command bytes and resulting tick range in the account Durable
   Object before replying;
4. deduplicates retries by monotonic sequence and payload hash.

The backlog has a fail-closed storage bound of 1,000,000 ticks. Crossing that
bound rejects the delivery without moving the durable wall-clock anchor or
mutating the simulation; it never truncates authoritative time. Normal clients
drain the debt by delivering subsequent batches.

Wasm necessarily mutates before its journal transaction can commit. If that
transaction fails or its result is ambiguous, the object marks the in-memory
simulation invalid. Before the next new command or checkpoint, it restores the
last R2 checkpoint and replays only committed journal rows. This prevents an
uncommitted batch from being applied twice after a storage fault.

The object checkpoints and clears its replay journal after 256 batches, when
the client explicitly deactivates, or from a one-shot 30-second idle alarm.
After eviction it restores the R2 checkpoint and replays the remaining durable
journal before accepting another command.

Dormant islands do **not** receive simulation or crafting alarms. On profile
activation, command delivery, or the unrelated reservation/idle alarm, pending
jobs whose stored `completes_at_ms` is in the past atomically become ready. This
keeps offline economy based on timestamps rather than continuous physics.

The repository's current Rust core does not yet export this ABI.
`src/island-module.ts` therefore exports `undefined`; runtime status honestly
reports `adapter: "unbound"` and command requests return
`501 island_runtime_not_bound`. Once the core supplies the documented exports,
replace that seam with a static `.wasm` import (Wrangler uploads it as a
`CompiledWasm` module). The lifecycle and persistence APIs do not change.

## Capacity service contract

The match director sends a scoped service ticket and:

```json
{
  "matchId": "11111111111111111111111111111111",
  "matchEpoch": 42,
  "region": "weur",
  "playlist": "squad-extraction",
  "inputPool": "controller",
  "buildHash": "22222222222222222222222222222222",
  "playerCount": 96,
  "botCount": 32,
  "authoritativeHz": 128
}
```

The capacity service returns a trusted allocation:

```json
{
  "hostId": "weur-host-42",
  "transport": "quic",
  "nativeEndpoint": "match.example.net:4433",
  "expiresAtMs": 1785140000000
}
```

For a browser allocation, `transport` is `wss` and `nativeEndpoint` is omitted.
The gateway forwards the upgrade to the `BROWSER_MATCH_ORIGIN` service binding
using a fixed internal URL, avoiding an SSRF-capable host URL in D1.

## Local preparation and validation

From this directory:

```bash
npm install
npm run build:browser-match
npm run validate
npm test
cp .dev.vars.example .dev.vars
```

Replace every `.dev.vars` placeholder with independently generated local keys.
The key ids in `RESULT_TICKET_KEYS_JSON` must exactly match
`RESULT_TICKET_HOSTS_JSON`, and each mapped host id must match the capacity
service's allocation record.
One way to generate a base64url key is:

```bash
openssl rand -base64 32 | tr '+/' '-_' | tr -d '='
```

Apply the D1 migration to local state before exercising routes:

```bash
npx wrangler d1 migrations apply CONTROL_DB --local
npm run dev
```

`wrangler.jsonc` uses local/automatic resource provisioning and does not include
the external service bindings, so dispatch and WSS handoff intentionally return
503 until those services are configured. The production example is a template.
Staging additionally renders
`wrangler.generated.browser-match.staging.jsonc`, deploys that private Worker
before the public control plane, and runs a two-client authoritative damage
smoke test through the public join and WebSocket routes. The private Worker has
no public route of its own.

Copy it to the tracked `deploy/production.resources.json`, replace every
`REPLACE_WITH_...` value, create or bind the named resources, and commit its
canonical resource fingerprint. The protected workflow compares the generated
config, account fingerprint, live D1 UUID, R2 location, Queue IDs, Worker
deployment, and exact secret names before it can migrate. It then captures and
uploads a fresh D1 Time Travel recovery bookmark before applying any
production migration.

## Required production tests before rollout

- Ticket expiry, key rotation, wrong audience/purpose/scope, and malformed-token
  fuzzing.
- Durable Object eviction/restart tests for reservation and matchmaking alarms.
- Duplicate and conflicting reservation/settlement delivery.
- Queue retry after each boundary: profile, R2, and D1.
- Dispatch compensation after partial party commits and capacity failure.
- WSS protocol stripping, build pinning, origin rejection, and service-binding
  outage.
- Island checkpoint compare-and-swap, R2 loss, and orphan reconciliation.
- 16/32/64/128-player host admission tests. This control plane does not claim
  the 128 Hz performance gate; that gate belongs to the dedicated match-server
  process.
