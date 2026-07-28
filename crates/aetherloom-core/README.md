# Aetherloom authoritative core

This crate is the platform-neutral, deterministic multiplayer vertical slice.
It owns gameplay state and advances only in integer 128 Hz ticks. It has no
renderer, wall clock, socket, sound, HUD, camera, particle, platform SDK, or
Cloudflare dependency.

Implemented boundaries:

- up to 128 stable player slots with human, bot, and empty controllers;
- ordinary `PlayerCommand` processing for human and bot input, including
  continuous vertical flight and retained aim pitch;
- generation-checked entity IDs backed by dense component storage;
- separately checkpointed generation, AI, combat, and environment RNG streams;
- revisioned terrain chunks and authoritative deformation events;
- explicit little-endian, checksummed, versioned checkpoints;
- viewer-specific keyframes and deltas with 128/32/8 Hz interest metadata;
- bounded baseline history and explicit reliable-resync signaling;
- 33-tick pose history with targeting rewind capped at 25 ticks;
- deterministic fine-angle 3D projectile aim with normalized speed and an
  authoritative first-person muzzle origin;
- client prediction/reconciliation and a 2–6 tick interpolation window;
- presentation, renderer, and platform-service boundaries.
- exact host-selected player spawns and atomic combat-state resets for
  deterministic arenas, without changing the legacy seeded spawn path.
- safe player-slot reuse that removes owned projectiles, plus explicit
  projectile clearing for clean round transitions.

Run this crate independently:

```sh
cargo test --manifest-path crates/aetherloom-core/Cargo.toml
```

The current gameplay systems are intentionally a small extraction-combat
vertical slice. Integrating the legacy game’s full spell, creature, terrain,
and economy rules into `MatchState` remains a separate migration; those rules
must not call presentation APIs or add new shared RNG draws.
