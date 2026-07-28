# Aetherloom protocol

This crate defines Aetherloom's platform-neutral multiplayer wire contract. It
has no third-party dependencies, forbids unsafe code, and supports `no_std`
targets with an allocator.

## Guarantees

- Authoritative gameplay runs at `AUTHORITATIVE_HZ` (128 Hz).
- IDs are typed and validated; entity IDs include a non-zero generation.
- Player input has bounded planar and continuous vertical axes, quantized and
  tagged with a tick and sequence.
- Input datagrams contain no player or account identity. The authenticated
  transport connection supplies the authoritative `PeerId -> PlayerId`
  binding.
- A `CommandSet` holds at most one command for each of 128 player slots and
  iterates in deterministic player-ID order.
- All integers use explicit little-endian encoding.
- Every frame carries protocol version, content build hash, match epoch,
  sequence, authoritative tick, acknowledgement tick, message kind, delivery
  class, and payload length.
- Datagram encoding and decoding reject frames larger than 1,200 bytes.
- Collection counts are bounded before allocation, and decoded messages are
  semantically validated before being returned.
- Reliable keyframes contain both terrain revision metadata and every
  authoritative cell height, so resynchronization can replace—not merely
  identify—stale chunks.
- Gameplay events carry stable IDs suitable for bounded de-duplication when a
  latency-critical datagram is repeated.

## Delivery classes

| Message | Datagram | Reliable |
| --- | ---: | ---: |
| `InputBatch` | yes | no |
| `SnapshotDelta` | yes | no |
| `SnapshotKeyframe` | no | yes |
| `EventBatch` | yes | yes |
| `TerrainDelta` | no | yes |
| `MatchResult` | no | yes |

An `InputBatch` contains one to three commands in newest-first order and no
identity claim. Normal steady-state senders include the current command and the
preceding two.

## Integrating into the workspace

Add `crates/aetherloom-protocol` as a workspace member, then depend on it by
path from the simulation, client, and server crates:

```toml
[dependencies]
aetherloom-protocol = { path = "../aetherloom-protocol" }
```

Construct `MessageEnvelope` values with the active content build hash and match
epoch. Use `encode_datagram`/`decode_datagram` for QUIC datagrams or bounded WSS
datagram-equivalent frames, and `encode_reliable`/`decode_reliable` for reliable
streams. `aetherloom-quic` owns the native server socket framing and
authentication handoff; browser and platform client adapters implement the
same transport policy outside this crate. Replay windows remain a session-host
responsibility. This crate intentionally owns only canonical protocol data.
