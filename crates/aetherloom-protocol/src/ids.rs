use crate::ValidationError;

/// Stable player slot identifier for a match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlayerId(u16);

impl PlayerId {
    pub const MAX_RAW: u16 = 127;

    pub fn new(raw: u16) -> Result<Self, ValidationError> {
        if raw <= Self::MAX_RAW {
            Ok(Self(raw))
        } else {
            Err(ValidationError::InvalidPlayerId(raw))
        }
    }

    pub const fn get(self) -> u16 {
        self.0
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl TryFrom<u16> for PlayerId {
    type Error = ValidationError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<PlayerId> for u16 {
    fn from(value: PlayerId) -> Self {
        value.get()
    }
}

/// Stable team identifier. A match may use one team per player.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TeamId(u16);

impl TeamId {
    pub const MAX_RAW: u16 = 127;

    pub fn new(raw: u16) -> Result<Self, ValidationError> {
        if raw <= Self::MAX_RAW {
            Ok(Self(raw))
        } else {
            Err(ValidationError::InvalidTeamId(raw))
        }
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

impl TryFrom<u16> for TeamId {
    type Error = ValidationError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<TeamId> for u16 {
    fn from(value: TeamId) -> Self {
        value.get()
    }
}

/// Stable network identity for an entity.
///
/// Dense storage slots must not be sent over the wire. The low 32 bits retain
/// the storage index while the high 32 bits carry a non-zero generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityId(u64);

impl EntityId {
    pub fn new(index: u32, generation: u32) -> Result<Self, ValidationError> {
        if generation == 0 {
            return Err(ValidationError::InvalidEntityId(index as u64));
        }
        Ok(Self(((generation as u64) << 32) | index as u64))
    }

    pub fn from_raw(raw: u64) -> Result<Self, ValidationError> {
        if raw == 0 || (raw >> 32) == 0 {
            Err(ValidationError::InvalidEntityId(raw))
        } else {
            Ok(Self(raw))
        }
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub const fn index(self) -> u32 {
        self.0 as u32
    }

    pub const fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }
}

impl TryFrom<u64> for EntityId {
    type Error = ValidationError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::from_raw(value)
    }
}

impl From<EntityId> for u64 {
    fn from(value: EntityId) -> Self {
        value.get()
    }
}

/// Monotonic identity of an authoritative replication snapshot.
///
/// Zero is reserved for "no baseline".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SnapshotId(u32);

impl SnapshotId {
    pub const NONE: Self = Self(0);

    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u32 {
        self.0
    }

    pub const fn is_none(self) -> bool {
        self.0 == 0
    }
}

impl From<u32> for SnapshotId {
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

impl From<SnapshotId> for u32 {
    fn from(value: SnapshotId) -> Self {
        value.get()
    }
}

/// Source currently controlling a player slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Controller {
    Human = 1,
    Bot = 2,
    Empty = 3,
}

impl Controller {
    pub fn from_wire(raw: u8) -> Result<Self, ValidationError> {
        match raw {
            1 => Ok(Self::Human),
            2 => Ok(Self::Bot),
            3 => Ok(Self::Empty),
            _ => Err(ValidationError::InvalidController(raw)),
        }
    }
}

impl TryFrom<u8> for Controller {
    type Error = ValidationError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::from_wire(value)
    }
}

impl From<Controller> for u8 {
    fn from(value: Controller) -> Self {
        value as u8
    }
}
