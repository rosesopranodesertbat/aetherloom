use std::collections::{BTreeMap, BTreeSet, VecDeque};

use aetherloom_server::{
    DeliveryKind, HostError, MatchHost, PeerId, TransportError,
};

/// A single simulation tick must not spend an unbounded amount of work
/// servicing one peer even when that peer's transport remains writable.
///
/// Normal replication emits only a handful of frames per peer per tick. This
/// deliberately generous ceiling protects fairness without constraining the
/// intended 128 Hz traffic profile.
const MAX_PACKETS_PER_PEER_PER_FLUSH: usize = 32;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum EgressClass {
    ReliableControl,
    CombatCritical,
    Medium,
    Distant,
    Cosmetic,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutboundPacket {
    pub peer: PeerId,
    pub delivery: DeliveryKind,
    pub class: EgressClass,
    pub server_tick: u64,
    pub payload: Vec<u8>,
    pub resync_if_dropped: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EnqueueOutcome {
    pub accepted: bool,
    pub reliable_backpressure: bool,
    pub evicted_resync_peers: Vec<PeerId>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EgressStats {
    pub enqueued: u64,
    pub sent: u64,
    pub dropped_cosmetic: u64,
    pub dropped_distant: u64,
    pub dropped_medium: u64,
    pub coalesced_critical: u64,
    pub reliable_backpressure: u64,
    pub transport_backpressure: u64,
    pub purged_peer_packets: u64,
    pub terminal_peer_drops: u64,
}

#[derive(Debug)]
pub struct PriorityEgress {
    packet_capacity: usize,
    byte_capacity: usize,
    reliable_packet_reserve: usize,
    reliable_byte_reserve: usize,
    queued_bytes: usize,
    reliable: VecDeque<OutboundPacket>,
    critical: VecDeque<OutboundPacket>,
    medium: VecDeque<OutboundPacket>,
    distant: VecDeque<OutboundPacket>,
    cosmetic: VecDeque<OutboundPacket>,
    stats: EgressStats,
}

impl PriorityEgress {
    pub fn new(packet_capacity: usize, byte_capacity: usize) -> Self {
        assert!(packet_capacity > 0, "egress packet capacity must be positive");
        assert!(byte_capacity > 0, "egress byte capacity must be positive");
        Self {
            packet_capacity,
            byte_capacity,
            reliable_packet_reserve: (packet_capacity / 8).max(1),
            reliable_byte_reserve: (byte_capacity / 8).max(1),
            queued_bytes: 0,
            reliable: VecDeque::new(),
            critical: VecDeque::new(),
            medium: VecDeque::new(),
            distant: VecDeque::new(),
            cosmetic: VecDeque::new(),
            stats: EgressStats::default(),
        }
    }

    pub fn enqueue(&mut self, packet: OutboundPacket) -> EnqueueOutcome {
        let mut outcome = EnqueueOutcome::default();
        if packet.payload.len() > self.byte_capacity {
            if packet.class == EgressClass::ReliableControl {
                outcome.reliable_backpressure = true;
                self.stats.reliable_backpressure =
                    self.stats.reliable_backpressure.saturating_add(1);
            } else {
                self.record_drop(packet.class);
            }
            if packet.resync_if_dropped {
                outcome.evicted_resync_peers.push(packet.peer);
            }
            return outcome;
        }

        while !self.fits(&packet) {
            let evicted = match packet.class {
                EgressClass::ReliableControl => self
                    .pop_lowest()
                    .or_else(|| self.pop_from(EgressClass::CombatCritical)),
                EgressClass::CombatCritical => self
                    .pop_lowest()
                    .or_else(|| {
                        self.coalesce_same_peer(
                            packet.peer,
                            packet.class,
                            packet.resync_if_dropped,
                        )
                    }),
                EgressClass::Medium => self
                    .pop_from(EgressClass::Cosmetic)
                    .or_else(|| self.pop_from(EgressClass::Distant)),
                EgressClass::Distant => self.pop_from(EgressClass::Cosmetic),
                EgressClass::Cosmetic => None,
            };
            let Some(evicted) = evicted else {
                if packet.class == EgressClass::ReliableControl {
                    outcome.reliable_backpressure = true;
                    self.stats.reliable_backpressure =
                        self.stats.reliable_backpressure.saturating_add(1);
                } else {
                    self.record_drop(packet.class);
                }
                if packet.resync_if_dropped {
                    outcome.evicted_resync_peers.push(packet.peer);
                }
                return outcome;
            };
            self.record_drop(evicted.class);
            if evicted.resync_if_dropped {
                outcome.evicted_resync_peers.push(evicted.peer);
            }
        }

        self.queued_bytes += packet.payload.len();
        self.queue_mut(packet.class).push_back(packet);
        self.stats.enqueued = self.stats.enqueued.saturating_add(1);
        outcome.accepted = true;
        outcome.evicted_resync_peers.sort_unstable();
        outcome.evicted_resync_peers.dedup();
        outcome
    }

    pub fn flush<H: MatchHost>(&mut self, host: &mut H) -> Result<usize, HostError> {
        let mut sent = 0;
        let mut attempts_remaining = self.len();
        let mut unavailable_peers = BTreeSet::new();
        let mut sent_per_peer = BTreeMap::<PeerId, usize>::new();

        while attempts_remaining > 0 {
            let Some((class, position)) =
                self.next_eligible_packet(&unavailable_peers)
            else {
                return Ok(sent);
            };
            let packet = self
                .queue_mut(class)
                .remove(position)
                .expect("eligible position points to a queued packet");
            let packet_peer = packet.peer;
            let result = match packet.delivery {
                DeliveryKind::Datagram => {
                    host.try_send_gameplay(packet.peer, &packet.payload)
                }
                DeliveryKind::Reliable => {
                    host.try_send_reliable(packet.peer, &packet.payload)
                }
            };
            attempts_remaining -= 1;
            match result {
                Ok(()) => {
                    self.queued_bytes -= packet.payload.len();
                    self.stats.sent = self.stats.sent.saturating_add(1);
                    sent += 1;
                    let peer_sent = sent_per_peer.entry(packet_peer).or_default();
                    *peer_sent += 1;
                    if *peer_sent == MAX_PACKETS_PER_PEER_PER_FLUSH {
                        unavailable_peers.insert(packet_peer);
                    }
                }
                Err(HostError::Transport(TransportError::Backpressure)) => {
                    self.queue_mut(class).insert(position, packet);
                    self.stats.transport_backpressure =
                        self.stats.transport_backpressure.saturating_add(1);
                    unavailable_peers.insert(packet_peer);
                }
                Err(HostError::Transport(
                    TransportError::InvalidPeer(_) | TransportError::Closed,
                )) => {
                    // The selected packet has already left its queue. Account
                    // for it explicitly, then purge every other frame for the
                    // terminal connection identity.
                    self.queued_bytes -= packet.payload.len();
                    self.stats.purged_peer_packets =
                        self.stats.purged_peer_packets.saturating_add(1);
                    let removed = self.purge_peer(packet_peer);
                    self.stats.terminal_peer_drops = self
                        .stats
                        .terminal_peer_drops
                        .saturating_add((removed + 1) as u64);
                }
                Err(error) => {
                    self.queue_mut(class).insert(position, packet);
                    return Err(error);
                }
            }
        }
        Ok(sent)
    }

    /// Removes all pending output for a disconnected connection identity.
    ///
    /// This must run before a `PeerId` can be reused so an old connection's
    /// keyframe or datagram can never be delivered to the new occupant.
    pub fn purge_peer(&mut self, peer: PeerId) -> usize {
        let mut removed_packets = 0;
        let mut removed_bytes = 0;
        for queue in [
            &mut self.reliable,
            &mut self.critical,
            &mut self.medium,
            &mut self.distant,
            &mut self.cosmetic,
        ] {
            let before = queue.len();
            queue.retain(|packet| {
                if packet.peer == peer {
                    removed_bytes += packet.payload.len();
                    false
                } else {
                    true
                }
            });
            removed_packets += before - queue.len();
        }
        self.queued_bytes = self.queued_bytes.saturating_sub(removed_bytes);
        self.stats.purged_peer_packets = self
            .stats
            .purged_peer_packets
            .saturating_add(removed_packets as u64);
        removed_packets
    }

    pub fn len(&self) -> usize {
        self.reliable.len()
            + self.critical.len()
            + self.medium.len()
            + self.distant.len()
            + self.cosmetic.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub const fn queued_bytes(&self) -> usize {
        self.queued_bytes
    }

    pub const fn packet_capacity(&self) -> usize {
        self.packet_capacity
    }

    pub const fn byte_capacity(&self) -> usize {
        self.byte_capacity
    }

    pub const fn stats(&self) -> EgressStats {
        self.stats
    }

    fn fits(&self, packet: &OutboundPacket) -> bool {
        if packet.class == EgressClass::ReliableControl {
            self.len() < self.packet_capacity
                && self
                    .queued_bytes
                    .saturating_add(packet.payload.len())
                    <= self.byte_capacity
        } else {
            let non_reliable_packets = self.len().saturating_sub(self.reliable.len());
            let non_reliable_bytes = self
                .queued_bytes
                .saturating_sub(self.reliable.iter().map(|item| item.payload.len()).sum());
            non_reliable_packets
                < self
                    .packet_capacity
                    .saturating_sub(self.reliable_packet_reserve)
                && non_reliable_bytes.saturating_add(packet.payload.len())
                    <= self
                        .byte_capacity
                        .saturating_sub(self.reliable_byte_reserve)
                && self.len() < self.packet_capacity
                && self
                    .queued_bytes
                    .saturating_add(packet.payload.len())
                    <= self.byte_capacity
        }
    }

    fn pop_lowest(&mut self) -> Option<OutboundPacket> {
        self.pop_from(EgressClass::Cosmetic)
            .or_else(|| self.pop_from(EgressClass::Distant))
            .or_else(|| self.pop_from(EgressClass::Medium))
    }

    fn coalesce_same_peer(
        &mut self,
        peer: PeerId,
        incoming: EgressClass,
        resync_if_dropped: bool,
    ) -> Option<OutboundPacket> {
        // Only superseding state snapshots may coalesce. Ephemeral combat
        // events share the latency-critical queue but must never evict a
        // snapshot or one another merely because they target the same peer.
        if !resync_if_dropped {
            return None;
        }
        let queue = match incoming {
            EgressClass::ReliableControl => &mut self.reliable,
            EgressClass::CombatCritical => &mut self.critical,
            _ => return None,
        };
        let position = queue
            .iter()
            .position(|packet| packet.peer == peer && packet.resync_if_dropped)?;
        let removed = queue.remove(position)?;
        self.queued_bytes -= removed.payload.len();
        Some(removed)
    }

    fn pop_from(&mut self, class: EgressClass) -> Option<OutboundPacket> {
        let removed = self.queue_mut(class).pop_front()?;
        self.queued_bytes -= removed.payload.len();
        Some(removed)
    }

    fn record_drop(&mut self, class: EgressClass) {
        match class {
            EgressClass::Cosmetic => {
                self.stats.dropped_cosmetic =
                    self.stats.dropped_cosmetic.saturating_add(1);
            }
            EgressClass::Distant => {
                self.stats.dropped_distant =
                    self.stats.dropped_distant.saturating_add(1);
            }
            EgressClass::Medium => {
                self.stats.dropped_medium =
                    self.stats.dropped_medium.saturating_add(1);
            }
            EgressClass::CombatCritical => {
                self.stats.coalesced_critical =
                    self.stats.coalesced_critical.saturating_add(1);
            }
            EgressClass::ReliableControl => {
                debug_assert!(
                    false,
                    "reliable control is backpressured, never silently dropped"
                );
            }
        }
    }

    fn next_eligible_packet(
        &self,
        unavailable_peers: &BTreeSet<PeerId>,
    ) -> Option<(EgressClass, usize)> {
        for class in [
            EgressClass::ReliableControl,
            EgressClass::CombatCritical,
            EgressClass::Medium,
            EgressClass::Distant,
            EgressClass::Cosmetic,
        ] {
            if let Some(position) = self
                .queue(class)
                .iter()
                .position(|packet| !unavailable_peers.contains(&packet.peer))
            {
                return Some((class, position));
            }
        }
        None
    }

    fn queue(&self, class: EgressClass) -> &VecDeque<OutboundPacket> {
        match class {
            EgressClass::ReliableControl => &self.reliable,
            EgressClass::CombatCritical => &self.critical,
            EgressClass::Medium => &self.medium,
            EgressClass::Distant => &self.distant,
            EgressClass::Cosmetic => &self.cosmetic,
        }
    }

    fn queue_mut(&mut self, class: EgressClass) -> &mut VecDeque<OutboundPacket> {
        match class {
            EgressClass::ReliableControl => &mut self.reliable,
            EgressClass::CombatCritical => &mut self.critical,
            EgressClass::Medium => &mut self.medium,
            EgressClass::Distant => &mut self.distant,
            EgressClass::Cosmetic => &mut self.cosmetic,
        }
    }
}
