# Aetherloom browser-match Wasm bridge

This crate is the Rust/Wasm boundary for the small authoritative browser duel.
It runs the real `aetherloom-core::MatchState`; JavaScript supplies latest input
state and reads presentation data, but never calculates movement, projectiles,
damage, outcomes, or ticks.

The bridge is deliberately limited to eight slots. Human input may arrive at
64 Hz, while `worker_match_advance_tick` creates a fresh, monotonic
`PlayerCommand` for every active human and advances exactly one 128 Hz tick. A
submitted input sample is held for exactly two ticks, then automatically
becomes neutral unless refreshed; this matches 64 Hz input without allowing a
stalled socket to move forever. Bots enter the ordinary core bot-command path.
A host should still call `worker_match_clear_input`, `worker_match_set_bot`, or
`worker_match_remove_player` immediately when a socket disconnects.

## Exported functions

All mutation functions return `0` on success or a negative status:

| Status | Meaning |
| ---: | --- |
| `0` | success |
| `-1` | bridge not initialized |
| `-2` | player is outside `0..8` |
| `-3` | invalid team |
| `-4` | player slot is already active |
| `-5` | player slot is empty |
| `-6` | input submitted for a bot |
| `-7` | invalid command field |
| `-8` | authoritative core rejected the operation |

```text
worker_match_abi_version() -> i32                 // 3
worker_match_authoritative_hz() -> i32            // 128
worker_match_max_players() -> i32                 // 8

worker_match_init(seed_low: u32, seed_high: u32) -> i32
worker_match_add_human(player: i32, team: i32) -> i32
worker_match_add_bot(player: i32, team: i32) -> i32
worker_match_remove_player(player: i32) -> i32
worker_match_set_human(player: i32) -> i32
worker_match_set_bot(player: i32) -> i32
worker_match_reset_player(player: i32) -> i32   // also clears all projectiles
worker_match_clear_input(player: i32) -> i32

worker_match_submit_input(
  player: i32,
  move_x: i32, move_y: i32,       // planar protocol range -2047..2047
  move_vertical: i32,              // lift protocol range -2047..2047
  look_yaw: i32,                  // 0..65535
  look_pitch: i32,                // protocol range -16384..16384
  action_flags: i32,              // protocol action bitset
  requested_spell: i32            // -1 for none, otherwise 0..31
) -> i32

worker_match_advance_tick() -> i32
worker_match_snapshot_ptr() -> *const i32
worker_match_snapshot_len() -> u32
```

The snapshot pointer is read-only and remains valid until a call rebuilds the
snapshot. Consumers must reacquire the pointer and length after initialization,
player/controller changes, reset, or tick advancement. Input submission and
input clearing do not rebuild or invalidate the current snapshot.

## Snapshot ABI version 3

The buffer is an `i32[worker_match_snapshot_len()]` in Wasm linear memory.
Unsigned halves are carried in signed words; use `word >>> 0` in JavaScript
before reconstructing a `u64`. Entity ID `0:0` means absent.

### Header: 16 words

| Word | Field |
| ---: | --- |
| 0 | magic `0x314d4c41` (`ALM1` in little-endian bytes) |
| 1 | ABI version (`3`) |
| 2 | total buffer words |
| 3–4 | next authoritative tick, low/high `u32` |
| 5 | authoritative Hz (`128`) |
| 6 | active player record count |
| 7 | player stride (`28`) |
| 8 | projectile record count |
| 9 | projectile stride (`15`) |
| 10 | last-tick event record count |
| 11 | event stride (`13`) |
| 12 | player-record word offset |
| 13 | projectile-record word offset |
| 14 | event-record word offset |
| 15 | reserved, zero |

### Player record: 28 words

```text
0 player_id
1 controller             // Human=1, Bot=2
2 team_id                // -1 when absent
3 entity_id_low
4 entity_id_high
5 x_cm
6 y_cm
7 z_cm
8 velocity_x_cm_per_tick
9 velocity_y_cm_per_tick
10 velocity_z_cm_per_tick
11 yaw
12 pitch
13 health
14..26 spell cooldown ticks for slots 0..12
27 outcome               // Active=1, Extracted=2, Defeated=3, ...
```

### Projectile record: 15 words

```text
0 entity_id_low
1 entity_id_high
2 owner_player_id        // -1 when absent
3 team_id                // -1 when absent
4 x_cm
5 y_cm
6 z_cm
7 velocity_x_cm_per_tick
8 velocity_y_cm_per_tick
9 velocity_z_cm_per_tick
10 yaw
11 pitch
12 lifetime_ticks
13 flags
14 health
```

### Last-tick event record: 13 words

```text
0 event_id_low
1 event_id_high
2 event_tick_low
3 event_tick_high
4 TickEventKind
5 actor_entity_id_low    // 0:0 when absent
6 actor_entity_id_high
7 target_entity_id_low   // 0:0 when absent
8 target_entity_id_high
9 data[0]
10 data[1]
11 data[2]
12 data[3]
```

## Validation

```bash
cargo test -p aetherloom-worker-match
RUSTFLAGS="-D warnings" cargo check -p aetherloom-worker-match \
  --target wasm32-unknown-unknown
```
