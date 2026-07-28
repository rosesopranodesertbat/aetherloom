# Deployment runbook

The release path has three independent gates:

1. pull requests and `main` run validation without deployment;
2. the manually dispatched staging workflow builds one immutable artifact,
   deploys it behind the protected `staging` environment, and smoke-tests it;
3. production workflows promote that exact staged artifact or control-plane
   proof after a second manual confirmation and environment approval.

No workflow contains an API token or runtime signing key. Production deployment
does not run on a push.

## One-time operator login

Local setup uses browser-based authentication:

```bash
gh auth login --hostname github.com --git-protocol ssh --web
cd server/cloudflare
npx wrangler login
```

The GitHub workflows do not inherit this local login. Give the `staging` and
`production` GitHub environments separate, least-privilege
`CLOUDFLARE_API_TOKEN` secrets and `CLOUDFLARE_ACCOUNT_ID` variables. Tokens
must be scoped to only the relevant account and environment resources. The
workflows expose those values only to the individual Wrangler steps that read
or mutate Cloudflare; validation, builds, artifacts, and smoke checks do not
inherit them.

## Provisioned staging resources

The committed [staging Wrangler configuration](../server/cloudflare/wrangler.staging.jsonc)
and its generator defaults bind:

| Resource | Staging value |
| --- | --- |
| Pages project | `aetherloom-staging` |
| Worker | `aetherloom-control-plane-staging` |
| Private browser match Worker prefix | `aetherloom-browser-match-staging` |
| D1 database | `aetherloom-control-staging` |
| D1 id | `220c38b2-1286-4a35-8c47-a7a24b83f387` |
| R2 bucket | `aetherloom-match-archive-staging` |
| Settlement Queue | `aetherloom-settlement-staging` |
| Settlement DLQ | `aetherloom-settlement-staging-dlq` |

`server/cloudflare/deploy/staging.resources.json` also commits the one-way
account fingerprint, D1 UUID, Queue IDs, R2 location, and a canonical resource
fingerprint. Before a migration, the workflow reads the live metadata and
requires every identity plus the exact four Worker secret names to match.
Changing a similarly named resource or selecting another account fails before
any write.

The R2 bucket and browser match Workers are private. Every staging content
build gets a versioned Worker name under that prefix. The new host is deployed
first; the control-plane deployment then atomically switches
`BROWSER_MATCH_ORIGIN` to it, so a failed rollout leaves the previous build
reachable. Old hosts can drain before an operator removes them. Staging still
has no `MATCH_CAPACITY_API`; normal matchmaking allocation therefore fails
closed instead of silently falling back. The demo route is separately gated
and cannot write progression, inventory, ratings, or settlement data.
Public sources may open two new players per five-minute window; reconnects are
separately throttled, reuse their room-scoped credential, and cannot extend the
absolute 15-minute systems-test session. Deployment smoke uses an authenticated
random room and therefore neither depends on nor consumes a public duel room.

After a successful proof, retain the bound version and one previous
`aetherloom-browser-match-staging-<build>` Worker. Once its 15-minute session
window has elapsed, remove older exact-prefix Workers after confirming no
service binding references them. Never force-delete the currently bound or
immediately previous host; Durable Object storage follows the retired staging
host's lifecycle.

The concrete staging file pins the currently staged release. Later workflow
runs generate `wrangler.generated.staging.jsonc` with the candidate's content
build id; generated configs are ignored by Git.

## Content build identity

The release builder walks `site/` in lexical order and records each file's
SHA-256 and byte length. It serializes this canonical value:

```text
JSON.stringify({version:1, commit:<full Git SHA>, files:<ordered file map>})
```

The first 16 bytes of its SHA-256, encoded as 32 lowercase hexadecimal
characters, are `CONTENT_BUILD_HASH`. The manifest is verified before staging
and again before production. The exact same id must be supplied to the
dedicated host's `MatchBuild`; a Git SHA, abbreviated SHA, UUID, uppercase hex,
or independently derived identifier is not interchangeable.

## Staging runtime keys

Only public verifier configuration is committed. Generate the four independent
private key domains into a mode-0600 file outside the repository:

```bash
node scripts/release/generate-worker-secrets.mjs \
  --output /secure/path/aetherloom-staging-worker-secrets.json \
  --join-kid staging-join-2026-07 \
  --player-kid staging-player-2026-07 \
  --service-kid staging-service-2026-07 \
  --result-kid staging-result-2026-07
```

The command prints only the public Ed25519 verifier map. Put that public map and
the active join/service key ids in Wrangler vars. Upload the private file
without printing it:

```bash
cd server/cloudflare
npm exec -- wrangler secret bulk \
  /secure/path/aetherloom-staging-worker-secrets.json \
  --config wrangler.staging.jsonc
```

Delete the local bundle after confirming the secrets exist in Cloudflare.
Retain a properly controlled recovery copy only if the operational key policy
requires one.

For the protected readiness check, store secret
`AETHERLOOM_READINESS_KEYS_B64` in each GitHub environment. Its decoded,
mode-0600 JSON contains only the same `PLAYER_TICKET_KEYS_JSON` and
`SERVICE_TICKET_KEYS_JSON` values already installed on that environment's
Worker. The workflow creates the file under the ephemeral runner directory,
never prints it, and removes it after the check. The authenticated
`POST /internal/v1/readiness` call rejects missing or tampered service tickets,
then round-trips the player, service, result, and Ed25519 join key domains in
Worker memory. Its response contains only aggregate pass/fail state.

`JOIN_TICKET_SIGNING_KEYS_JSON` is an Ed25519 PKCS#8 private-key map.
`PLAYER_TICKET_KEYS_JSON`, `SERVICE_TICKET_KEYS_JSON`, and
`RESULT_TICKET_KEYS_JSON` are independent HMAC key maps. The public
`JOIN_TICKET_PUBLIC_KEYS_JSON`, `ACTIVE_JOIN_TICKET_KID`, and
`ACTIVE_SERVICE_TICKET_KID` are ordinary vars, not secrets. A match host gets
only its join public key and its own result-signing key.

## Bootstrap the staging Worker

From the repository root:

```bash
npm ci --prefix server/cloudflare
npm exec --prefix server/cloudflare -- wrangler deploy \
  --config server/cloudflare/wrangler.staging.jsonc \
  --dry-run
npm exec --prefix server/cloudflare -- wrangler d1 migrations apply CONTROL_DB \
  --config server/cloudflare/wrangler.staging.jsonc \
  --remote
npm exec --prefix server/cloudflare -- wrangler deploy \
  --config server/cloudflare/wrangler.staging.jsonc \
  --strict
```

After the first deployment, set the `staging` environment variable
`AETHERLOOM_CONTROL_PLANE_URL` to its HTTPS URL. Smoke it with:

```bash
node scripts/release/smoke-control-plane.mjs \
  https://REPLACE_WITH_STAGING_WORKER_URL \
  staging \
  abdc743597a10be51ad5344f01c89462
```

The final argument is the content build id in the committed bootstrap config,
not the 40-character commit.

## GitHub repository configuration

Create these environments:

- `staging`: environment-scoped Cloudflare token and account id; add reviewers
  if every test deployment needs approval.
- `production`: a distinct production token, account id, all resource vars,
  and required reviewers.
- `github-pages`: required reviewers for the public web release.

Set the staging environment:

- secret `CLOUDFLARE_API_TOKEN`;
- variable `CLOUDFLARE_ACCOUNT_ID`;
- variable `CLOUDFLARE_PAGES_PROJECT=aetherloom-staging`;
- variable `AETHERLOOM_CONTROL_PLANE_URL` after Worker bootstrap.

The known staging resources and public key configuration default from
`server/cloudflare/deploy/staging.resources.json`. Environment variables may
still supply values to the renderer, but the identity gate rejects any value
that differs from the committed manifest. Production has no implicit resource
defaults. Copy `server/cloudflare/deploy/production.resources.example.json` to
the tracked `server/cloudflare/deploy/production.resources.json`, replace every
placeholder (including the account, Queue, D1 and R2 identity fields), and
record its canonical fingerprint:

```bash
CLOUDFLARE_ACCOUNT_ID=REPLACE_WITH_ACCOUNT_ID \
  node scripts/release/verify-cloudflare-resources.mjs account-fingerprint
node scripts/release/verify-cloudflare-resources.mjs fingerprint \
  --manifest server/cloudflare/deploy/production.resources.json
```

Put the first output in `accountIdSha256`, then put the second output in
`resourceFingerprint`. A production promotion intentionally fails until this
reviewed manifest exists and matches the protected account.

Protect `main`, require pull requests, and require the `Validate` job. Restrict
who can dispatch release workflows and who can approve `production` and
`github-pages`. A staging workflow must itself run from `main` and its event
`head_sha` must equal the candidate. Promotion rechecks the workflow id,
repository, branch, source input, and exact requested SHA before downloading
artifacts.

## Release sequence

1. Merge a validated pull request to `main`.
2. Run **Deploy staging** from the current protected `main` head.
3. The workflow deploys and proves the versioned match host and control plane
   before it publishes the web preview. It automatically restores the previous
   control-plane version if the backend proof fails.
4. Verify the staging web URL and recorded control-plane proof. The proof
   requires two independent browser clients to join one isolated room and
   observe the same authoritative damage result.
5. Copy the successful staging workflow run id and its resolved 40-character
   commit SHA.
6. Run **Promote web production**, enter those two values, and type
   `PROMOTE WEB`.
7. Separately run **Promote control-plane production** and type
   `PROMOTE CONTROL PLANE`. It refuses a run without a matching staging
   control-plane smoke proof.
8. Approve the protected environment deployment after reviewing the displayed
   SHA and staging evidence.

The web production workflow downloads the staged artifact, checks every file
against its manifest, rebuilds the same commit, proves the results are
byte-identical, and only then uploads to GitHub Pages. The control-plane
workflow reapplies validation, performs a Wrangler dry run, validates remote
resource and secret identity, and captures a D1 Time Travel bookmark. It binds
that recovery point to the database fingerprint and release SHA, uploads the
proof successfully, and rechecks freshness before migration. Only then does it
deploy in strict mode and require both `/healthz` and authenticated
cryptographic readiness.

D1 Time Travel is Cloudflare's always-on point-in-time recovery mechanism. A
bookmark can restore the exact pre-migration database state; it remains valid
for 30 days on Workers Paid or 7 days on Workers Free. If a migration needs
rollback, stop writes and use the reviewed restore command in the protected
`production-d1-recovery-<sha>` artifact before that window expires. Restoring
overwrites the live database, so it remains a separately approved incident
action. See [Cloudflare's Time Travel documentation](https://developers.cloudflare.com/d1/reference/time-travel/).

## What these deployments prove

A passing deployment proves the static client and Cloudflare control plane are
reachable with the expected build and that two browser clients can join the
private staging host, exchange binary commands/snapshots, and agree on one
authoritative hit. It does not prove continuous 7.8125 ms pacing, the
128-player performance gate, valuable progression, the regional capacity
service, native QUIC fleet, production session issuer, or home-island Wasm
adapter. Keep progression disabled until the end-to-end and 128-player
production gates in [the multiplayer architecture](multiplayer-architecture.md)
pass.
