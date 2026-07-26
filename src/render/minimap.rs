//! The minimap raster.

use crate::math::*;
use crate::world::*;

// ---- minimap ---------------------------------------------------------------
const MINIMAP_MAX_SIDE: usize = 160;

pub struct Minimap {
    pub pixels: [u8; MINIMAP_MAX_SIDE * MINIMAP_MAX_SIDE * 4],
}

impl Minimap {
    pub const fn new() -> Minimap {
        Minimap {
            pixels: [0; MINIMAP_MAX_SIDE * MINIMAP_MAX_SIDE * 4],
        }
    }
}

impl World {
    /// Rasterises the heightmap into an RGBA image of `side` by `side` pixels.
    pub fn rasterise_minimap(&mut self, side: usize) {
        let side = side.clamp(1, MINIMAP_MAX_SIDE);
        let step = GRID_WIDTH as f32 / side as f32;
        for row in 0..side {
            for column in 0..side {
                let height = self.height_at_cell(
                    (column as f32 * step) as i32,
                    (row as f32 * step) as i32,
                );
                let colour = minimap_colour(height);
                let base = (row * side + column) * 4;
                self.minimap.pixels[base] = colour[0];
                self.minimap.pixels[base + 1] = colour[1];
                self.minimap.pixels[base + 2] = colour[2];
                self.minimap.pixels[base + 3] = 255;
            }
        }
    }
}

/// Sea shades by depth, then beach, scrub, rock and snow by altitude.
fn minimap_colour(height: f32) -> [u8; 3] {
    const DEEPEST: f32 = 30.0;
    const BEACH_TOP: f32 = 8.0;
    const SCRUB_TOP: f32 = 52.0;
    const ROCK_TOP: f32 = 118.0;
    if height < -1.0 {
        let depth = min(1.0, -height / DEEPEST);
        let shallowness = 1.0 - depth;
        [
            (14.0 + shallowness * 26.0) as u8,
            (34.0 + shallowness * 50.0) as u8,
            (62.0 + shallowness * 44.0) as u8,
        ]
    } else if height < BEACH_TOP {
        [178, 152, 98]
    } else if height < SCRUB_TOP {
        [(74.0 + height * 0.7) as u8, (96.0 + height * 0.5) as u8, 50]
    } else if height < ROCK_TOP {
        [106, 96, 84]
    } else {
        [210, 205, 193]
    }
}
