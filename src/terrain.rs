//! The heightmap: sampling, deformation and realm generation.

use crate::math::*;
use crate::types::DeformKind;
use crate::world::*;

/// Radial island masks are summed, then perturbed by two octaves of value
/// noise. Everything below is a knob on that.
const OCEAN_FLOOR: f32 = -26.0;
const ISLAND_MIN_COUNT: i32 = 3;
const ISLAND_EXTRA_COUNT: i32 = 3;
const ISLAND_MAX: usize = 8;
/// Island centres stay this far in from the world edge, as a fraction.
const ISLAND_INSET: (f32, f32) = (0.18, 0.82);
const ISLAND_RADIUS_RANGE: (f32, f32) = (0.16, 0.30);
const ISLAND_PEAK_RANGE: (f32, f32) = (38.0, 96.0);
/// The mask reaches this far past its nominal radius before it is ignored.
const ISLAND_FALLOFF_LIMIT: f32 = 1.4;
const ISLAND_HEIGHT_GAIN: f32 = 1.6;
const COARSE_NOISE_SCALE: f32 = 0.0055;
const COARSE_NOISE_HEIGHT: f32 = 34.0;
const FINE_NOISE_SCALE: f32 = 0.021;
const FINE_NOISE_HEIGHT: f32 = 9.0;
/// Noise is damped underwater so the seabed stays smooth.
const SEABED_NOISE_DAMPING: f32 = 0.35;
/// Heights in this band get pulled toward sea level to make real beaches.
const BEACH_BAND: (f32, f32) = (-4.0, 8.0);
const BEACH_FLATTENING: f32 = 0.55;

/// Grid cells from a castle centre that get levelled into a shelf.
const CASTLE_PAD_CELLS: i32 = 13;
/// Fraction of the pad that is dead flat before the skirt starts. Must clear
/// the plinth (half-width 27 units) or the terrace hangs over the slope.
const CASTLE_PAD_FLAT_FRACTION: f32 = 0.72;

impl World {
    // ---- sampling ----------------------------------------------------------
    /// Height at a grid cell, clamped to the edge rather than wrapping.
    pub fn height_at_cell(&self, column: i32, row: i32) -> f32 {
        let column = column.clamp(0, GRID_MAX_INDEX);
        let row = row.clamp(0, GRID_MAX_INDEX);
        self.terrain.height[(row * GRID_WIDTH + column) as usize]
    }

    fn set_height_at_cell(&mut self, column: i32, row: i32, value: f32) {
        if column < 0 || column > GRID_MAX_INDEX || row < 0 || row > GRID_MAX_INDEX {
            return;
        }
        self.terrain.height[(row * GRID_WIDTH + column) as usize] = value;
        if row < self.terrain.dirty_first_row {
            self.terrain.dirty_first_row = row;
        }
        if row > self.terrain.dirty_last_row {
            self.terrain.dirty_last_row = row;
        }
    }

    /// Bilinearly filtered height in world space.
    pub fn height_at(&self, x: f32, z: f32) -> f32 {
        let grid_x = x / CELL_SIZE;
        let grid_z = z / CELL_SIZE;
        let column = floor(grid_x) as i32;
        let row = floor(grid_z) as i32;
        let fx = grid_x - column as f32;
        let fz = grid_z - row as f32;
        let near = {
            let a = self.height_at_cell(column, row);
            let b = self.height_at_cell(column + 1, row);
            a + (b - a) * fx
        };
        let far = {
            let a = self.height_at_cell(column, row + 1);
            let b = self.height_at_cell(column + 1, row + 1);
            a + (b - a) * fx
        };
        near + (far - near) * fz
    }

    /// Ground or sea surface, whichever is higher.
    pub fn surface_at(&self, x: f32, z: f32) -> f32 {
        max(self.height_at(x, z), SEA_LEVEL)
    }

    /// Marches a ray until it meets the ground, returning the distance
    /// travelled. Steps proportionally to the gap so open sky is crossed fast.
    pub fn distance_to_ground(&self, origin: [f32; 3], direction: [f32; 3], limit: f32) -> f32 {
        const MIN_STEP: f32 = 5.0;
        const COARSE_GAP: f32 = 12.0;
        const GAP_STEP_FRACTION: f32 = 0.6;
        let mut travelled = 4.0;
        while travelled < limit {
            let point = [
                origin[0] + direction[0] * travelled,
                origin[1] + direction[1] * travelled,
                origin[2] + direction[2] * travelled,
            ];
            let ground = self.height_at(point[0], point[2]);
            if point[1] <= ground {
                return travelled;
            }
            let gap = point[1] - ground;
            travelled += if gap > COARSE_GAP {
                gap * GAP_STEP_FRACTION
            } else {
                MIN_STEP
            };
        }
        limit
    }

    // ---- value noise -------------------------------------------------------
    /// Hash of a lattice point, mixed with the seed so each realm differs.
    /// Every multiply wraps deliberately; without the explicit wrapping ops
    /// Rust would panic in debug and the terrain would differ from release.
    fn lattice_hash(&self, x: i32, y: i32) -> f32 {
        let mut hash = x
            .wrapping_mul(374_761_393)
            .wrapping_add(y.wrapping_mul(668_265_263))
            .wrapping_add(self.rng.state() as i32) as u32;
        hash = (hash ^ (hash >> 13)).wrapping_mul(1_274_126_177);
        hash ^= hash >> 16;
        (hash & 0xffff) as f32 / 32768.0 - 1.0
    }

    fn value_noise(&self, x: f32, y: f32) -> f32 {
        let cell_x = floor(x) as i32;
        let cell_y = floor(y) as i32;
        let fx = smooth_fade(x - cell_x as f32);
        let fy = smooth_fade(y - cell_y as f32);
        let bottom_left = self.lattice_hash(cell_x, cell_y);
        let bottom_right = self.lattice_hash(cell_x + 1, cell_y);
        let top_left = self.lattice_hash(cell_x, cell_y + 1);
        let top_right = self.lattice_hash(cell_x + 1, cell_y + 1);
        let bottom = bottom_left + (bottom_right - bottom_left) * fx;
        let top = top_left + (top_right - top_left) * fx;
        bottom + (top - bottom) * fy
    }

    /// Sum of octaves at halving amplitude. The 2.03 lacunarity is slightly
    /// off two so octaves do not line up into visible grid artefacts.
    fn fractal_noise(&self, x: f32, y: f32, octaves: i32) -> f32 {
        let mut total = 0.0f32;
        let mut amplitude = 0.5f32;
        let mut frequency = 1.0f32;
        for _ in 0..octaves {
            total += amplitude * self.value_noise(x * frequency, y * frequency);
            frequency *= 2.03;
            amplitude *= 0.5;
        }
        total
    }

    // ---- deformation -------------------------------------------------------
    pub fn deform(&mut self, x: f32, z: f32, radius: f32, amount: f32, kind: DeformKind) {
        let centre_col = x / CELL_SIZE;
        let centre_row = z / CELL_SIZE;
        let cell_radius = radius / CELL_SIZE;
        let first_col = floor(centre_col - cell_radius) as i32 - 1;
        let last_col = ceil(centre_col + cell_radius) as i32 + 1;
        let first_row = floor(centre_row - cell_radius) as i32 - 1;
        let last_row = ceil(centre_row + cell_radius) as i32 + 1;
        for row in first_row..=last_row {
            for col in first_col..=last_col {
                let offset_col = col as f32 - centre_col;
                let offset_row = row as f32 - centre_row;
                let normalised =
                    sqrt(offset_col * offset_col + offset_row * offset_row) / cell_radius;
                if normalised > 1.0 {
                    continue;
                }
                let weight = match kind {
                    // a signed wave, so a quake leaves the average alone
                    DeformKind::Ripple => cos(normalised * 3.14159) * (1.0 - normalised),
                    _ => {
                        let bell = 1.0 - normalised * normalised;
                        bell * bell
                    }
                };
                let current = self.height_at_cell(col, row);
                let updated = match kind {
                    DeformKind::Cone => {
                        let cone = amount * (1.0 - normalised);
                        current + cone * 0.55 + amount * 0.45 * weight
                    }
                    _ => current - amount * weight,
                };
                self.set_height_at_cell(col, row, updated);
            }
        }
    }

    /// Cuts a level shelf for a castle: dead flat well past the plinth, then a
    /// smooth skirt down to the original hill. A plain quadratic falloff is
    /// only truly flat at the exact centre, which on a steep peak leaves slope
    /// under the terrace edge and makes the keep look like it is hanging off.
    pub fn level_castle_pad(&mut self, x: f32, z: f32) {
        self.deform(x, z, 30.0, 0.0, DeformKind::Ripple);
        let pad_height = self.height_at(x, z);
        let centre_col = (x / CELL_SIZE) as i32;
        let centre_row = (z / CELL_SIZE) as i32;
        for row in (centre_row - CASTLE_PAD_CELLS)..=(centre_row + CASTLE_PAD_CELLS) {
            for col in (centre_col - CASTLE_PAD_CELLS)..=(centre_col + CASTLE_PAD_CELLS) {
                let dx = (col - centre_col) as f32;
                let dz = (row - centre_row) as f32;
                let normalised = sqrt(dx * dx + dz * dz) / CASTLE_PAD_CELLS as f32;
                if normalised > 1.0 {
                    continue;
                }
                let skirt = clamp(
                    (normalised - CASTLE_PAD_FLAT_FRACTION) / (1.0 - CASTLE_PAD_FLAT_FRACTION),
                    0.0,
                    1.0,
                );
                let blend = 1.0 - smooth_fade(skirt);
                let current = self.height_at_cell(col, row);
                self.set_height_at_cell(col, row, current + (pad_height - current) * blend);
            }
        }
    }

    // ---- generation --------------------------------------------------------
    pub fn generate_terrain(&mut self) {
        let island_count = (ISLAND_MIN_COUNT + self.rng.below(ISLAND_EXTRA_COUNT)) as usize;
        let mut centre_x = [0.0f32; ISLAND_MAX];
        let mut centre_z = [0.0f32; ISLAND_MAX];
        let mut radius = [0.0f32; ISLAND_MAX];
        let mut peak = [0.0f32; ISLAND_MAX];
        for i in 0..island_count {
            centre_x[i] = self.rng.range(ISLAND_INSET.0, ISLAND_INSET.1) * WORLD_SIZE;
            centre_z[i] = self.rng.range(ISLAND_INSET.0, ISLAND_INSET.1) * WORLD_SIZE;
            radius[i] = self.rng.range(ISLAND_RADIUS_RANGE.0, ISLAND_RADIUS_RANGE.1) * WORLD_SIZE;
            peak[i] = self.rng.range(ISLAND_PEAK_RANGE.0, ISLAND_PEAK_RANGE.1);
        }
        for row in 0..GRID_WIDTH {
            for col in 0..GRID_WIDTH {
                let world_x = col as f32 * CELL_SIZE;
                let world_z = row as f32 * CELL_SIZE;
                let mut height = OCEAN_FLOOR;
                for i in 0..island_count {
                    let dx = world_x - centre_x[i];
                    let dz = world_z - centre_z[i];
                    let normalised = sqrt(dx * dx + dz * dz) / radius[i];
                    if normalised >= ISLAND_FALLOFF_LIMIT {
                        continue;
                    }
                    let mask = smooth_fade(max(1.0 - normalised, 0.0));
                    height += peak[i] * mask * ISLAND_HEIGHT_GAIN;
                }
                let detail = self.fractal_noise(
                    world_x * COARSE_NOISE_SCALE,
                    world_z * COARSE_NOISE_SCALE,
                    5,
                ) * COARSE_NOISE_HEIGHT
                    + self.fractal_noise(world_x * FINE_NOISE_SCALE, world_z * FINE_NOISE_SCALE, 3)
                        * FINE_NOISE_HEIGHT;
                height += detail * if height > -10.0 { 1.0 } else { SEABED_NOISE_DAMPING };
                if height > BEACH_BAND.0 && height < BEACH_BAND.1 {
                    height *= BEACH_FLATTENING;
                }
                self.terrain.height[(row * GRID_WIDTH + col) as usize] = height;
            }
        }
        self.terrain.dirty_first_row = 0;
        self.terrain.dirty_last_row = GRID_MAX_INDEX;
    }

    /// A random land cell above `min_height`, as a flat grid index. Falls back
    /// to the map centre if the realm is unusually watery.
    pub fn find_land(&mut self, min_height: f32) -> i32 {
        const ATTEMPTS: i32 = 4000;
        const EDGE_MARGIN: i32 = 12;
        for _ in 0..ATTEMPTS {
            let col = EDGE_MARGIN + self.rng.below(GRID_WIDTH - EDGE_MARGIN * 2);
            let row = EDGE_MARGIN + self.rng.below(GRID_WIDTH - EDGE_MARGIN * 2);
            if self.height_at_cell(col, row) > min_height {
                return row * GRID_WIDTH + col;
            }
        }
        (GRID_WIDTH / 2) * GRID_WIDTH + GRID_WIDTH / 2
    }

    /// World position of a flat grid index.
    pub fn cell_to_world(index: i32) -> (f32, f32) {
        (
            (index % GRID_WIDTH) as f32 * CELL_SIZE,
            (index / GRID_WIDTH) as f32 * CELL_SIZE,
        )
    }

    /// Scenery is placed in clumps rather than uniformly: a scatter of seed
    /// points, each with its own spread, then draws around them. Uniform
    /// noise gives an even grey rash across every hillside; clumps give you
    /// boulder fields and groves with clear ground between them.
    pub fn scatter_scenery(&mut self) {
        /// Below this height it is beach or seabed and stays bare.
        const MIN_PLANTING_HEIGHT: f32 = 2.0;
        /// Palms only below this; above it is rock, with a mixed band between.
        const PALM_LINE: f32 = 16.0;
        const TREE_LINE: f32 = 60.0;
        const CLUMPS: usize = 210;
        const CLUMP_SPREAD: (f32, f32) = (40.0, 190.0);
        /// A minority are placed loose so the clumps do not look laid out.
        const STRAY_FRACTION: f32 = 0.18;

        self.scenery.count = 0;
        let mut clump_x = [0.0f32; CLUMPS];
        let mut clump_z = [0.0f32; CLUMPS];
        let mut clump_spread = [0.0f32; CLUMPS];
        for i in 0..CLUMPS {
            clump_x[i] = self.rng.range(0.0, WORLD_SIZE);
            clump_z[i] = self.rng.range(0.0, WORLD_SIZE);
            clump_spread[i] = self.rng.range(CLUMP_SPREAD.0, CLUMP_SPREAD.1);
        }

        // Most of the map is ocean and most draws land in it, so the loop
        // runs well past the cap and stops on whichever comes first.
        for _ in 0..(MAX_SCENERY * 2) {
            if self.scenery.count >= MAX_SCENERY {
                break;
            }
            let (x, z) = if self.rng.unit() < STRAY_FRACTION {
                (self.rng.range(0.0, WORLD_SIZE), self.rng.range(0.0, WORLD_SIZE))
            } else {
                let clump = self.rng.below(CLUMPS as i32) as usize;
                let spread = clump_spread[clump];
                // two draws per axis biases toward the middle, so a clump is
                // dense at its heart and thins out
                let jitter_x = self.rng.range(-spread, spread) * self.rng.range(0.35, 1.0);
                let jitter_z = self.rng.range(-spread, spread) * self.rng.range(0.35, 1.0);
                (
                    clamp(clump_x[clump] + jitter_x, 0.0, WORLD_SIZE),
                    clamp(clump_z[clump] + jitter_z, 0.0, WORLD_SIZE),
                )
            };
            let height = self.height_at(x, z);
            if height < MIN_PLANTING_HEIGHT {
                continue;
            }
            let kind = if height < PALM_LINE {
                SceneryKind::Palm
            } else if height < TREE_LINE {
                if self.rng.below(3) == 0 {
                    SceneryKind::Palm
                } else {
                    SceneryKind::Rock
                }
            } else {
                SceneryKind::Rock
            };
            // rocks vary far more in size than palms do
            let scale = match kind {
                SceneryKind::Rock => self.rng.range(0.45, 2.3),
                SceneryKind::Palm => self.rng.range(0.75, 1.45),
            };
            let rotation = self.rng.range(0.0, core::f32::consts::TAU);
            let slot = self.scenery.count;
            self.scenery.pos_x[slot] = x;
            self.scenery.pos_z[slot] = z;
            self.scenery.kind[slot] = kind;
            self.scenery.scale[slot] = scale;
            self.scenery.rotation[slot] = rotation;
            self.scenery.standing[slot] = true;
            self.scenery.burn_remaining[slot] = 0.0;
            self.scenery.count += 1;
        }
    }
}

/// Keeps a position inside the playable area.
#[inline]
pub fn clamp_to_world(value: f32) -> f32 {
    clamp(value, WORLD_MARGIN, WORLD_SIZE - WORLD_MARGIN)
}
