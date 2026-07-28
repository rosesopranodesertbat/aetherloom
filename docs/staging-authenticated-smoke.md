# Authenticated staging smoke check

The authenticated smoke check is an operator tool for the bootstrap staging
control plane. It does not add an HTTP route, bypass authentication, or upload
credentials. It reads the same local Worker secret bundle used for deployment,
mints two-minute HMAC tickets in memory, and sends them only to the requested
HTTPS Worker origin.

The check proves:

- `/healthz` reports `staging`, 128 Hz, and the exact expected content build;
- the configured browser origin passes preflight while an unlisted origin is
  rejected without being reflected;
- missing and tampered player tickets are rejected;
- a valid player ticket can read a dedicated smoke profile;
- a player-key ticket is rejected by an internal service route before its
  claims are trusted;
- an isolated empty-loadout queue dispatch returns
  `match_capacity_unavailable` while staging has no capacity binding; and
- the isolated queue entry and reservation are cancelled before exit.

The release workflow also runs the smaller, non-mutating readiness check after
every control-plane deploy. It authenticates with a service ticket, proves
missing and tampered tickets fail, and asks the Worker to round-trip its player,
service, result, and Ed25519 join key domains without returning key ids or
private material:

```bash
node scripts/release/smoke-control-plane-readiness.mjs \
  --url https://REPLACE_WITH_STAGING_WORKER_URL \
  --environment staging \
  --expected-build abdc743597a10be51ad5344f01c89462 \
  --secrets-file /secure/path/aetherloom-readiness-keys.json \
  --service-kid staging-service-2026-07
```

The readiness file needs only `PLAYER_TICKET_KEYS_JSON` and
`SERVICE_TICKET_KEYS_JSON`, with values identical to the deployed Worker. It
has the same ownership, regular-file, and exact mode-0600 requirements as the
full operator bundle.

The smoke profile is derived from the Worker origin, contains no player
identity, carries no items, and is reused by later checks. The capacity probe
does create a short-lived empty reservation and queue entry. The command
requires an explicit confirmation flag for that reason.

Player and service tickets use independent HMAC key sets. Supplying a
player-key ticket to a service route must therefore fail with
`401 unknown_ticket_key`; the Worker rejects the signing domain before it
parses or trusts the ticket's purpose claim.

## Run

Keep the generated secret bundle outside the repository and make its mode
exactly `0600`. The script also rejects symbolic links, non-regular files, files
owned by another user, malformed key sets, and keys shorter than 32 bytes.

```bash
node scripts/release/smoke-control-plane-authenticated.mjs \
  --url https://REPLACE_WITH_STAGING_WORKER_URL \
  --secrets-file /secure/path/aetherloom-staging-worker-secrets.json \
  --expected-build abdc743597a10be51ad5344f01c89462 \
  --origin https://aetherloom-staging.pages.dev \
  --confirm-isolated-capacity-probe
```

Run this capacity expectation only against bootstrap staging while
`MATCH_CAPACITY_API` is deliberately absent. If capacity is added, replace this
probe with a separately designed end-to-end allocation test; an unexpected
successful allocation makes this check fail.

When a key set contains more than one rotation key, select the intended keys:

```bash
node scripts/release/smoke-control-plane-authenticated.mjs \
  --url https://REPLACE_WITH_STAGING_WORKER_URL \
  --secrets-file /secure/path/aetherloom-staging-worker-secrets.json \
  --expected-build abdc743597a10be51ad5344f01c89462 \
  --origin https://aetherloom-staging.pages.dev \
  --player-kid staging-player-2026-07 \
  --service-kid staging-service-2026-07 \
  --confirm-isolated-capacity-probe
```

The tool reports only named pass/fail checks. It never prints a ticket, private
key, secret-bundle value, response body, or player data. A failure after
isolated state is created includes cleanup in the same run and reports cleanup
failure separately.

The optional `--shard-count` and `--skill-bucket-width` values must match the
deployed matchmaking configuration if staging changes from its defaults of
`16` and `200`. They let the tool derive the cleanup shard before enqueueing,
so it can still cancel state if the enqueue response is lost.

## Test

The test suite uses an in-memory Worker-shaped endpoint and does not open a
network socket:

```bash
node --test scripts/release/smoke-control-plane-authenticated.test.mjs \
  scripts/release/smoke-control-plane-readiness.test.mjs
```

It covers canonical ticket encoding, signature verification, file permission
and symlink rejection, authentication failures, CORS behavior, the
fail-closed capacity response, cleanup, and secret-free reporting.
