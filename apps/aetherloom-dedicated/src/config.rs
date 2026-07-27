use aetherloom_core::{MatchConfig, WorldSeed, MAX_PLAYERS};
use aetherloom_protocol::{InputPool, RegionId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MatchAdmissionScope {
    region: RegionId,
    input_pool: InputPool,
}

impl MatchAdmissionScope {
    pub const fn new(region: RegionId, input_pool: InputPool) -> Self {
        Self { region, input_pool }
    }

    pub const fn region(self) -> RegionId {
        self.region
    }

    pub const fn input_pool(self) -> InputPool {
        self.input_pool
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MatchBuild {
    match_id: [u8; 16],
    content_build_hash: [u8; 16],
    match_epoch: u64,
}

impl MatchBuild {
    pub fn new(
        match_id: [u8; 16],
        content_build_hash: [u8; 16],
        match_epoch: u64,
    ) -> Result<Self, ConfigError> {
        if match_id.iter().all(|byte| *byte == 0) {
            return Err(ConfigError::ZeroMatchId);
        }
        if content_build_hash.iter().all(|byte| *byte == 0) {
            return Err(ConfigError::ZeroContentBuildHash);
        }
        if match_epoch == 0 {
            return Err(ConfigError::ZeroMatchEpoch);
        }
        Ok(Self {
            match_id,
            content_build_hash,
            match_epoch,
        })
    }

    pub const fn match_id(&self) -> [u8; 16] {
        self.match_id
    }

    pub const fn content_build_hash(&self) -> [u8; 16] {
        self.content_build_hash
    }

    pub const fn match_epoch(&self) -> u64 {
        self.match_epoch
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessConfig {
    build: MatchBuild,
    admission_scope: MatchAdmissionScope,
    seed: WorldSeed,
    max_players: u16,
    egress_packet_capacity: usize,
    egress_byte_capacity: usize,
    persistence_byte_capacity: usize,
    keyframe_interval_ticks: u64,
    checkpoint_interval_ticks: u64,
}

impl ProcessConfig {
    pub fn new(
        build: MatchBuild,
        admission_scope: MatchAdmissionScope,
        seed: WorldSeed,
        max_players: u16,
    ) -> Result<Self, ConfigError> {
        if max_players == 0 || max_players as usize > MAX_PLAYERS {
            return Err(ConfigError::InvalidPlayerCapacity(max_players));
        }
        Ok(Self {
            build,
            admission_scope,
            seed,
            max_players,
            egress_packet_capacity: 512,
            egress_byte_capacity: 8 * 1024 * 1024,
            persistence_byte_capacity: 4 * 1024 * 1024,
            keyframe_interval_ticks: 2 * aetherloom_protocol::AUTHORITATIVE_HZ as u64,
            checkpoint_interval_ticks: 10 * aetherloom_protocol::AUTHORITATIVE_HZ as u64,
        })
    }

    pub fn with_queue_limits(
        mut self,
        egress_packets: usize,
        egress_bytes: usize,
        persistence_bytes: usize,
    ) -> Result<Self, ConfigError> {
        if egress_packets == 0 || egress_bytes == 0 || persistence_bytes == 0 {
            return Err(ConfigError::ZeroQueueCapacity);
        }
        self.egress_packet_capacity = egress_packets;
        self.egress_byte_capacity = egress_bytes;
        self.persistence_byte_capacity = persistence_bytes;
        Ok(self)
    }

    pub fn with_intervals(
        mut self,
        keyframe_interval_ticks: u64,
        checkpoint_interval_ticks: u64,
    ) -> Result<Self, ConfigError> {
        if keyframe_interval_ticks == 0 {
            return Err(ConfigError::ZeroKeyframeInterval);
        }
        self.keyframe_interval_ticks = keyframe_interval_ticks;
        self.checkpoint_interval_ticks = checkpoint_interval_ticks;
        Ok(self)
    }

    pub const fn build(&self) -> MatchBuild {
        self.build
    }

    pub const fn admission_scope(&self) -> MatchAdmissionScope {
        self.admission_scope
    }

    pub const fn seed(&self) -> WorldSeed {
        self.seed
    }

    pub const fn max_players(&self) -> u16 {
        self.max_players
    }

    pub const fn egress_packet_capacity(&self) -> usize {
        self.egress_packet_capacity
    }

    pub const fn egress_byte_capacity(&self) -> usize {
        self.egress_byte_capacity
    }

    pub const fn persistence_byte_capacity(&self) -> usize {
        self.persistence_byte_capacity
    }

    pub const fn keyframe_interval_ticks(&self) -> u64 {
        self.keyframe_interval_ticks
    }

    pub const fn checkpoint_interval_ticks(&self) -> u64 {
        self.checkpoint_interval_ticks
    }

    pub fn core_match_config(&self) -> MatchConfig {
        MatchConfig::new(self.max_players, 8_192, 4_096)
            .expect("ProcessConfig validates the player count")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigError {
    ZeroMatchId,
    ZeroContentBuildHash,
    ZeroMatchEpoch,
    InvalidPlayerCapacity(u16),
    ZeroQueueCapacity,
    ZeroKeyframeInterval,
}

impl core::fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for ConfigError {}
