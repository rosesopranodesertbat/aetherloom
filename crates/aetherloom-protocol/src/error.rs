use core::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationError {
    InvalidPlayerId(u16),
    InvalidTeamId(u16),
    InvalidEntityId(u64),
    InvalidController(u8),
    ZeroCommandSequence,
    MoveAxisOutOfRange {
        axis: &'static str,
        value: i16,
    },
    LookPitchOutOfRange(i16),
    UnsupportedActionFlags(u16),
    SpellOutOfRange(u8),
    EmptyCommandBatch,
    TooManyCommands(usize),
    DuplicateCommandSequence(u32),
    DuplicateCommandTick(u64),
    CommandsNotNewestFirst,
    CommandTickMismatch {
        expected: u64,
        actual: u64,
    },
    DuplicatePlayer(u16),
    EmptyCollection(&'static str),
    TooManyItems {
        field: &'static str,
        count: usize,
        max: usize,
    },
    DuplicateEntity(u64),
    DuplicateEvent(u64),
    DuplicateChunk {
        x: i32,
        y: i32,
    },
    DuplicateTerrainCell {
        x: u16,
        y: u16,
    },
    DuplicateResultPlayer(u16),
    DuplicateLootItem(u32),
    InvalidSnapshotId(u32),
    InvalidBaseline {
        snapshot: u32,
        baseline: u32,
    },
    InvalidArchetype(u16),
    InvalidEventId(u64),
    InvalidEventKind(u16),
    InvalidTerrainCell {
        x: u16,
        y: u16,
    },
    InvalidTerrainCellCount {
        count: usize,
        expected: usize,
    },
    InvalidTerrainRevision {
        base: u32,
        new: u32,
    },
    InvalidTerrainOperation(u8),
    TerrainKeyframeMismatch,
    InvalidRegion,
    InvalidInputPool,
    InvalidMatchIdentifier(&'static str),
    InvalidLootItem(u32),
    InvalidLootQuantity(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProtocolError {
    Validation(ValidationError),
    Truncated { needed: usize, remaining: usize },
    TrailingBytes(usize),
    InvalidMagic([u8; 4]),
    UnsupportedVersion(u16),
    InvalidHeaderLength(u16),
    UnknownMessageKind(u8),
    InvalidEnum { field: &'static str, value: u64 },
    NonZeroReservedFlags(u16),
    PayloadLengthMismatch { declared: usize, actual: usize },
    NumericOverflow(&'static str),
    DatagramTooLarge { len: usize, max: usize },
    ReliableFrameTooLarge { len: usize, max: usize },
    WrongDelivery { expected: u8, actual: u8 },
    MessageNotAllowedOnDelivery { kind: u8, delivery: u8 },
}

impl From<ValidationError> for ProtocolError {
    fn from(value: ValidationError) -> Self {
        Self::Validation(value)
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ValidationError {}

#[cfg(feature = "std")]
impl std::error::Error for ProtocolError {}
