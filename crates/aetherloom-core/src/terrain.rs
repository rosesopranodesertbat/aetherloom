use alloc::vec;
use alloc::vec::Vec;

pub const TERRAIN_CHUNK_SIDE: usize = aetherloom_protocol::TERRAIN_CHUNK_SIDE as usize;
pub const TERRAIN_CELLS: usize = TERRAIN_CHUNK_SIDE * TERRAIN_CHUNK_SIDE;
pub const CELL_WORLD_CM: i32 = 200;
const CHUNK_WORLD_CM: i32 = TERRAIN_CHUNK_SIDE as i32 * CELL_WORLD_CM;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ChunkCoord {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerrainChunk {
    pub coord: ChunkCoord,
    pub revision: u32,
    pub heights_cm: Vec<i16>,
}

impl TerrainChunk {
    pub(crate) fn flat(coord: ChunkCoord) -> Self {
        Self {
            coord,
            revision: 0,
            heights_cm: vec![0; TERRAIN_CELLS],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerrainDeformationEvent {
    pub tick: u64,
    pub chunk: ChunkCoord,
    pub base_revision: u32,
    pub new_revision: u32,
    pub cell_x: u8,
    pub cell_y: u8,
    pub height_delta_cm: i16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainError {
    Capacity,
    InvalidCell { x: u8, y: u8 },
    InvalidCellCount,
    DuplicateChunk,
}

pub(crate) fn world_to_chunk_cell(position_cm: [i32; 3]) -> (ChunkCoord, u8, u8) {
    let chunk_x = position_cm[0].div_euclid(CHUNK_WORLD_CM);
    let chunk_y = position_cm[2].div_euclid(CHUNK_WORLD_CM);
    let local_x = position_cm[0].rem_euclid(CHUNK_WORLD_CM) / CELL_WORLD_CM;
    let local_y = position_cm[2].rem_euclid(CHUNK_WORLD_CM) / CELL_WORLD_CM;
    (
        ChunkCoord {
            x: chunk_x,
            y: chunk_y,
        },
        local_x as u8,
        local_y as u8,
    )
}

pub(crate) fn deform(
    chunks: &mut Vec<TerrainChunk>,
    max_chunks: u32,
    tick: u64,
    coord: ChunkCoord,
    cell_x: u8,
    cell_y: u8,
    height_delta_cm: i16,
) -> Result<TerrainDeformationEvent, TerrainError> {
    if cell_x as usize >= TERRAIN_CHUNK_SIDE || cell_y as usize >= TERRAIN_CHUNK_SIDE {
        return Err(TerrainError::InvalidCell {
            x: cell_x,
            y: cell_y,
        });
    }
    let index = match chunks.binary_search_by_key(&coord, |chunk| chunk.coord) {
        Ok(index) => index,
        Err(index) => {
            if chunks.len() >= max_chunks as usize {
                return Err(TerrainError::Capacity);
            }
            chunks.insert(index, TerrainChunk::flat(coord));
            index
        }
    };
    let chunk = &mut chunks[index];
    let base_revision = chunk.revision;
    chunk.revision = chunk.revision.wrapping_add(1);
    if chunk.revision == 0 {
        chunk.revision = 1;
    }
    let cell_index = cell_y as usize * TERRAIN_CHUNK_SIDE + cell_x as usize;
    chunk.heights_cm[cell_index] =
        chunk.heights_cm[cell_index].saturating_add(height_delta_cm);
    Ok(TerrainDeformationEvent {
        tick,
        chunk: coord,
        base_revision,
        new_revision: chunk.revision,
        cell_x,
        cell_y,
        height_delta_cm,
    })
}

pub(crate) fn validate_chunks(chunks: &[TerrainChunk]) -> Result<(), TerrainError> {
    let mut previous: Option<ChunkCoord> = None;
    for chunk in chunks {
        if chunk.heights_cm.len() != TERRAIN_CELLS {
            return Err(TerrainError::InvalidCellCount);
        }
        if previous.is_some_and(|coord| coord >= chunk.coord) {
            return Err(TerrainError::DuplicateChunk);
        }
        previous = Some(chunk.coord);
    }
    Ok(())
}
