# Aetherloom native QUIC transport

`aetherloom-quic` adapts Quinn to `aetherloom_server::NetworkTransport`.
Socket work runs asynchronously behind bounded channels; the 128 Hz simulation
thread only polls ingress and attempts nonblocking egress.

The caller supplies the Quinn `ServerConfig` and an asynchronous
`ConnectionAdmission` implementation. Production certificate loading,
certificate rotation, and signed connection-ticket validation remain
caller-owned. A successful hook returns immutable verified player, team,
account, match, epoch, build, region, input-pool, nonce, and expiry claims
together with a server-allocated `PeerId`.

The transport publishes one ordered `ConnectionEvent::Authenticated` before it
starts gameplay readers and one `ConnectionEvent::Disconnected` when that QUIC
connection closes. A dedicated runtime must drain lifecycle events before
polling gameplay ingress. This makes the authenticated peer binding the source
of identity authority. Protocol version 4 input datagrams contain no account,
player, or team identity field.

Gameplay datagrams are capped at 1,200 bytes. Reliable messages are encoded on
unidirectional streams as a four-byte big-endian length followed by the payload,
with the protocol reliable-frame cap enforced before allocation.

`begin_draining` rejects new admissions without interrupting live matches.
`shutdown_gracefully` waits for peers until its deadline and then closes the
endpoint. Idle timeouts, keepalives, connection limits, stream limits, and
bounded ingress/per-peer egress are applied by the adapter. Ingress capacity is
partitioned into per-peer packet and byte shares, then drained in stable
`PeerId` round-robin order. Configuration therefore reserves at least one
maximum-size datagram per possible connection; one flooding peer cannot occupy
another admitted peer's queue budget.

## Observability

`NativeQuicTransport::stats` exposes saturating inbound/outbound
application-payload byte totals and live ingress/egress queue-byte gauges in
addition to connection, message, drop, backpressure, protocol, and I/O counts.
The payload totals deliberately exclude QUIC/TLS/UDP and reliable-frame length
overhead. The queue gauges are bounded by the configured global ingress and
per-peer egress capacities.

`NativeQuicTransport::peer_telemetry` returns a read-only snapshot sorted by
the server-owned `PeerId`. Each active peer includes RTT, queued egress bytes,
UDP datagram/byte counts, congestion window/events, packet and byte loss,
PLPMTUD, black-hole, and current-MTU counters sourced from Quinn. Closed peers
are removed rather than retained as stale identities.

Raw remote addresses are omitted by default. Set
`QuicTransportConfig::expose_remote_address_in_telemetry` only for a trusted,
access-controlled diagnostic consumer; do not use addresses as account or
gameplay identity, or as unbounded metrics labels.

Production aggregation remains outside this crate: the host must periodically
export snapshots to its metrics backend and derive rates, percentiles, regional
SLOs, alerting, and retention. Host CPU/memory, scheduler jitter, application
snapshot age, reconciliation size, client-observed RTT, and end-to-end egress
accounting also belong to the dedicated process and observability platform.

This crate is the native reference backend, not a console networking
dependency. A console vendor stack can implement the same
`aetherloom_server::NetworkTransport` trait without Quinn, Tokio, or rustls
crossing the gameplay boundary.

`apps/aetherloom-dedicated::TicketConnectionAdmission` is the compile-tested
connection-ticket adapter. It consumes a capped length-prefixed ticket from the
first client unidirectional stream, calls the application verifier once,
allocates `PeerId` on the server, and passes the exact verified claims through
the lifecycle event. The application integration test exercises this flow over
a real localhost QUIC connection with test-only certificates.
