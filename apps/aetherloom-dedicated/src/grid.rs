use std::collections::BTreeMap;

use aetherloom_core::EntityPool;
use aetherloom_protocol::EntityId;

pub const DEFAULT_CELL_SIZE_CM: i32 = 2_000;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CellCoord {
    pub x: i32,
    pub z: i32,
}

/// Deterministic broad-phase and interest-management index.
///
/// Cells are stored in coordinate order, cell members are sorted by stable
/// `EntityId`, and every multi-cell query performs a final `EntityId` merge.
/// Worker completion order can therefore never affect gameplay or replication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeterministicSpatialGrid {
    cell_size_cm: i32,
    cells: BTreeMap<CellCoord, Vec<EntityId>>,
    positions: BTreeMap<EntityId, [i32; 3]>,
}

impl DeterministicSpatialGrid {
    pub fn new(cell_size_cm: i32) -> Self {
        assert!(cell_size_cm > 0, "spatial grid cell size must be positive");
        Self {
            cell_size_cm,
            cells: BTreeMap::new(),
            positions: BTreeMap::new(),
        }
    }

    pub fn rebuild(&mut self, entities: &EntityPool) {
        self.rebuild_entries(
            entities
                .iter()
                .map(|entity| (entity.id, entity.position_cm)),
        );
    }

    pub fn rebuild_entries(
        &mut self,
        entries: impl IntoIterator<Item = (EntityId, [i32; 3])>,
    ) {
        self.cells.clear();
        self.positions.clear();
        for (entity_id, position_cm) in entries {
            let cell = self.cell_for(position_cm);
            self.cells.entry(cell).or_default().push(entity_id);
            self.positions.insert(entity_id, position_cm);
        }
        for ids in self.cells.values_mut() {
            ids.sort_unstable();
        }
    }

    pub const fn cell_size_cm(&self) -> i32 {
        self.cell_size_cm
    }

    pub fn cell_for(&self, position_cm: [i32; 3]) -> CellCoord {
        CellCoord {
            x: position_cm[0].div_euclid(self.cell_size_cm),
            z: position_cm[2].div_euclid(self.cell_size_cm),
        }
    }

    pub fn query_radius(&self, center_cm: [i32; 3], radius_cm: i32) -> Vec<EntityId> {
        if radius_cm < 0 {
            return Vec::new();
        }
        let minimum = self.cell_for([
            center_cm[0].saturating_sub(radius_cm),
            center_cm[1],
            center_cm[2].saturating_sub(radius_cm),
        ]);
        let maximum = self.cell_for([
            center_cm[0].saturating_add(radius_cm),
            center_cm[1],
            center_cm[2].saturating_add(radius_cm),
        ]);
        let radius_squared = i128::from(radius_cm) * i128::from(radius_cm);
        let mut merged = Vec::new();
        for (cell, ids) in &self.cells {
            if cell.x < minimum.x
                || cell.x > maximum.x
                || cell.z < minimum.z
                || cell.z > maximum.z
            {
                continue;
            }
            merged.extend(ids.iter().copied());
        }
        merged.sort_unstable();
        merged.dedup();

        merged.retain(|entity_id| {
            self.positions.get(entity_id).is_some_and(|position| {
                let dx = i128::from(position[0]) - i128::from(center_cm[0]);
                let dz = i128::from(position[2]) - i128::from(center_cm[2]);
                dx * dx + dz * dz <= radius_squared
            })
        });
        merged
    }

    pub fn all_entity_ids(&self) -> Vec<EntityId> {
        let mut merged: Vec<EntityId> = self
            .cells
            .values()
            .flat_map(|ids| ids.iter().copied())
            .collect();
        merged.sort_unstable();
        merged.dedup();
        merged
    }

    pub fn cells(&self) -> impl ExactSizeIterator<Item = (CellCoord, &[EntityId])> {
        self.cells
            .iter()
            .map(|(coord, ids)| (*coord, ids.as_slice()))
    }
}

impl Default for DeterministicSpatialGrid {
    fn default() -> Self {
        Self::new(DEFAULT_CELL_SIZE_CM)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radius_query_handles_full_i32_coordinate_span() {
        let near = EntityId::new(0, 1).expect("entity");
        let far = EntityId::new(1, 1).expect("entity");
        let mut grid = DeterministicSpatialGrid::default();
        grid.rebuild_entries([
            (near, [i32::MIN, 0, i32::MIN]),
            (far, [i32::MAX, 0, i32::MAX]),
        ]);

        assert_eq!(
            grid.query_radius([i32::MIN, 0, i32::MIN], 1),
            vec![near]
        );
    }
}
