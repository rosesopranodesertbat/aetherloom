use alloc::vec::Vec;

use aetherloom_protocol::{EntityId, PlayerId, TeamId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum EntityKind {
    Player = 1,
    Projectile = 2,
    Resource = 3,
    Objective = 4,
}

impl EntityKind {
    pub(crate) fn from_u8(value: u8) -> Result<Self, EntityPoolError> {
        match value {
            1 => Ok(Self::Player),
            2 => Ok(Self::Projectile),
            3 => Ok(Self::Resource),
            4 => Ok(Self::Objective),
            _ => Err(EntityPoolError::InvalidKind(value)),
        }
    }
}

/// All authoritative components for the vertical slice use integer units.
///
/// Positions are centimetres and velocity is centimetres per 128 Hz tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entity {
    pub id: EntityId,
    pub kind: EntityKind,
    pub owner: Option<PlayerId>,
    pub team: Option<TeamId>,
    pub position_cm: [i32; 3],
    pub velocity_cm_per_tick: [i16; 3],
    pub yaw: u16,
    pub pitch: i16,
    pub health: u16,
    pub flags: u16,
    pub lifetime_ticks: u16,
}

impl Entity {
    pub fn new(kind: EntityKind) -> Self {
        // This placeholder is replaced atomically by EntityPool::spawn.
        let id = EntityId::new(0, 1).expect("generation one is valid");
        Self {
            id,
            kind,
            owner: None,
            team: None,
            position_cm: [0; 3],
            velocity_cm_per_tick: [0; 3],
            yaw: 0,
            pitch: 0,
            health: 1,
            flags: 0,
            lifetime_ticks: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Slot {
    generation: u32,
    dense_index: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityPool {
    slots: Vec<Slot>,
    dense: Vec<Entity>,
    free: Vec<u32>,
    capacity: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityPoolError {
    Capacity,
    InvalidId,
    InvalidKind(u8),
    Corrupt(&'static str),
}

impl EntityPool {
    pub fn new(capacity: u32) -> Self {
        Self {
            slots: Vec::new(),
            dense: Vec::new(),
            free: Vec::new(),
            capacity,
        }
    }

    pub const fn capacity(&self) -> u32 {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.dense.len()
    }

    pub fn is_empty(&self) -> bool {
        self.dense.is_empty()
    }

    pub fn spawn(&mut self, mut entity: Entity) -> Result<EntityId, EntityPoolError> {
        let slot_index = if let Some(index) = self.free.pop() {
            index
        } else {
            if self.slots.len() >= self.capacity as usize {
                return Err(EntityPoolError::Capacity);
            }
            let index = self.slots.len() as u32;
            self.slots.push(Slot {
                generation: 1,
                dense_index: None,
            });
            index
        };

        let slot = &mut self.slots[slot_index as usize];
        if slot.generation == 0 {
            slot.generation = 1;
        }
        let id = EntityId::new(slot_index, slot.generation)
            .map_err(|_| EntityPoolError::InvalidId)?;
        entity.id = id;
        slot.dense_index = Some(self.dense.len() as u32);
        self.dense.push(entity);
        Ok(id)
    }

    pub fn contains(&self, id: EntityId) -> bool {
        self.slot_for(id).is_some()
    }

    pub fn get(&self, id: EntityId) -> Option<&Entity> {
        let slot = self.slot_for(id)?;
        self.dense.get(slot.dense_index? as usize)
    }

    pub fn get_mut(&mut self, id: EntityId) -> Option<&mut Entity> {
        let dense_index = self.slot_for(id)?.dense_index? as usize;
        self.dense.get_mut(dense_index)
    }

    pub fn remove(&mut self, id: EntityId) -> Option<Entity> {
        let slot_index = id.index() as usize;
        let dense_index = self.slot_for(id)?.dense_index? as usize;
        let removed = self.dense.swap_remove(dense_index);

        if dense_index < self.dense.len() {
            let moved = self.dense[dense_index];
            self.slots[moved.id.index() as usize].dense_index = Some(dense_index as u32);
        }

        let slot = &mut self.slots[slot_index];
        slot.dense_index = None;
        slot.generation = slot.generation.wrapping_add(1);
        if slot.generation == 0 {
            slot.generation = 1;
        }
        self.free.push(slot_index as u32);
        Some(removed)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Entity> {
        self.dense.iter()
    }

    pub fn active_ids_sorted(&self) -> Vec<EntityId> {
        let mut ids: Vec<EntityId> = self.dense.iter().map(|entity| entity.id).collect();
        ids.sort_unstable();
        ids
    }

    fn slot_for(&self, id: EntityId) -> Option<&Slot> {
        let slot = self.slots.get(id.index() as usize)?;
        if slot.generation != id.generation() || slot.dense_index.is_none() {
            return None;
        }
        Some(slot)
    }

    pub(crate) fn slot_generations(&self) -> impl Iterator<Item = u32> + '_ {
        self.slots.iter().map(|slot| slot.generation)
    }

    pub(crate) fn dense_entities(&self) -> &[Entity] {
        &self.dense
    }

    pub(crate) fn free_slots(&self) -> &[u32] {
        &self.free
    }

    pub(crate) fn restore_parts(
        capacity: u32,
        generations: Vec<u32>,
        dense: Vec<Entity>,
        free: Vec<u32>,
    ) -> Result<Self, EntityPoolError> {
        if generations.len() > capacity as usize || dense.len() > generations.len() {
            return Err(EntityPoolError::Corrupt("entity counts exceed capacity"));
        }
        let mut slots: Vec<Slot> = generations
            .into_iter()
            .map(|generation| Slot {
                generation,
                dense_index: None,
            })
            .collect();
        for (dense_index, entity) in dense.iter().enumerate() {
            let index = entity.id.index() as usize;
            let Some(slot) = slots.get_mut(index) else {
                return Err(EntityPoolError::Corrupt("entity slot does not exist"));
            };
            if slot.generation == 0 || slot.generation != entity.id.generation() {
                return Err(EntityPoolError::Corrupt("entity generation mismatch"));
            }
            if slot.dense_index.is_some() {
                return Err(EntityPoolError::Corrupt("duplicate entity slot"));
            }
            slot.dense_index = Some(dense_index as u32);
        }
        let mut free_seen = alloc::vec![false; slots.len()];
        for index in &free {
            let index = *index as usize;
            let Some(slot) = slots.get(index) else {
                return Err(EntityPoolError::Corrupt("free slot does not exist"));
            };
            if slot.dense_index.is_some() || free_seen[index] {
                return Err(EntityPoolError::Corrupt("invalid duplicate free slot"));
            }
            free_seen[index] = true;
        }
        for (index, slot) in slots.iter().enumerate() {
            if slot.dense_index.is_none() && !free_seen[index] {
                return Err(EntityPoolError::Corrupt("untracked free slot"));
            }
        }
        Ok(Self {
            slots,
            dense,
            free,
            capacity,
        })
    }
}

