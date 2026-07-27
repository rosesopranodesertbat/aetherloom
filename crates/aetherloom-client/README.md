# Aetherloom portable client boundary

This crate establishes the shared Rust client layer that sits between the
authoritative core and a browser, desktop, or proprietary console host.

Implemented here:

- variable-rate frame orchestration over fixed 128 Hz prediction;
- protocol-ready input mapping with controller deadzones and controller-only UI
  navigation;
- tick-rate controller look integration, so turn rate is independent of render
  cadence;
- explicit prediction-backlog resynchronization instead of client time
  dilation or unbounded catch-up;
- native/loopback 128 Hz and browser 64 Hz input scheduling with current-plus-
  two-previous command redundancy;
- an integer snapshot-jitter estimator that drives a bounded two-to-six-tick
  interpolation delay;
- typed terrain, mesh, instance, particle, camera, and HUD presentation frames;
- a graphics-API-independent `RendererBackend`;
- deterministic capability tiers and fallback selection for HDR, bloom,
  texture formats, shader/storage limits, and memory budgets;
- safe-area layout;
- QUIC, WSS, and loopback transport policies and adapter traits;
- a wire-snapshot bridge that maps validated protocol archetypes, baselines,
  acknowledgements, and authoritative server ticks into `ClientReplica`;
- a session-pinned authoritative entry point that rejects the wrong content
  build or match epoch before mutation and keeps independent replay windows for
  snapshot datagrams, event datagrams, and the reliable stream;
- bounded remote snapshot history and interpolation independent of local
  128 Hz prediction;
- a bounded authoritative feed cache that applies complete terrain keyframes,
  detects reliable terrain revision gaps, requests resynchronization, and
  de-duplicates redundant gameplay-event IDs;
- identity, entitlement, storage, achievement, suspend/resume, and device-loss
  lifecycle boundaries;
- account-scoped online saves that cannot alias the offline campaign namespace;
- a versioned `repr(C)` vendor data boundary and matching
  `include/aetherloom_client.h` for console/static-library adapters.

This is an integration boundary, not a fake platform implementation. It does
not contain a QUIC stack, console SDK, renderer, audio engine, or platform
identity implementation.

The current terrain core stores heights only. A received `SetMaterial`
operation is therefore rejected before mutation and requests a keyframe;
dedicated servers emit only height-deformation operations until material state
is added to authoritative chunks and keyframes.

The existing game still renders through JavaScript/WebGPU. It has **not** yet
been fully ported to Rust/wgpu, and this crate does not claim visual parity.
The next client migration step is to extract the legacy presentation data into
`ClientRenderFrame`, implement a real wgpu backend, and run the existing model
gallery as cross-backend visual goldens.

Run the crate independently:

```sh
cargo test --manifest-path crates/aetherloom-client/Cargo.toml
cargo check --manifest-path crates/aetherloom-client/Cargo.toml \
  --target wasm32-unknown-unknown
```
