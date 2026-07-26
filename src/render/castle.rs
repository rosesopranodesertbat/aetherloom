//! The keeps: plinth, towers, walls and the ruin left when one falls.

use super::*;

impl World {
    /// Lowest ground under the castle footprint. A keep placed at the height of
    /// its own centre hangs in the air on the downhill side of any slope.
    fn lowest_ground_under_castle(&self, x: f32, z: f32) -> f32 {
        let mut lowest = self.height_at(x, z);
        for step in 0..8 {
            let angle = step as f32 * core::f32::consts::FRAC_PI_4;
            let sample = self.height_at(
                x + cos(angle) * CASTLE_FOOTPRINT,
                z + sin(angle) * CASTLE_FOOTPRINT,
            );
            lowest = min(lowest, sample);
        }
        lowest
    }

    pub(super) fn draw_castle(&mut self, wizard: usize) {
        let centre_x = self.castles.pos_x[wizard];
        let centre_z = self.castles.pos_z[wizard];
        let ground = self.lowest_ground_under_castle(centre_x, centre_z);
        if self.castles.health[wizard] <= 0.0 {
            self.draw_castle_ruin(wizard, ground);
            return;
        }
        let tier = self.castles.tier[wizard];
        let livery = if wizard == PLAYER {
            [0.24, 0.34, 0.72]
        } else {
            [0.70, 0.20, 0.16]
        };
        let base = self.castles.pos_y[wizard];
        let deck = base + CASTLE_DECK_HEIGHT;
        // sunk so it reads as cut into the hill, but never a skyscraper
        let plinth_bottom = max(ground - CASTLE_PLINTH_SINK, base - CASTLE_PLINTH_MAX_DROP);

        self.push_instance(
            [centre_x, (deck + plinth_bottom) * 0.5, centre_z],
            [CASTLE_FOOTPRINT * 2.0, deck - plinth_bottom, CASTLE_FOOTPRINT * 2.0],
            [
                livery[0] * 0.5 + 0.14,
                livery[1] * 0.5 + 0.12,
                livery[2] * 0.5 + 0.10,
            ],
            0.0,
            0.0,
            Shape::Cuboid,
        );
        // a narrower course under the deck reads as a stepped foundation
        self.push_instance(
            [centre_x, deck - 1.6, centre_z],
            [
                CASTLE_FOOTPRINT * 2.0 - 7.0,
                5.0,
                CASTLE_FOOTPRINT * 2.0 - 7.0,
            ],
            [
                livery[0] * 0.34 + 0.10,
                livery[1] * 0.34 + 0.09,
                livery[2] * 0.34 + 0.08,
            ],
            0.0,
            0.0,
            Shape::Cuboid,
        );

        let keep_height = CASTLE_KEEP_BASE_HEIGHT + tier as f32 * CASTLE_KEEP_HEIGHT_PER_TIER;
        self.push_instance(
            [centre_x, deck + keep_height * 0.5, centre_z],
            [19.0, keep_height, 19.0],
            livery,
            0.0,
            0.0,
            Shape::Cuboid,
        );
        self.push_instance(
            [centre_x, deck + 2.0 + keep_height, centre_z],
            [15.0, 13.0, 15.0],
            [livery[0] * 1.25, livery[1] * 1.25, livery[2] * 1.25],
            0.785,
            0.15,
            Shape::Cone,
        );

        let tower_count = 2 + tier;
        for tower in 0..tower_count {
            let angle = tower as f32 / tower_count as f32 * core::f32::consts::TAU + 0.4;
            let at_x = centre_x + cos(angle) * CASTLE_TOWER_RING;
            let at_z = centre_z + sin(angle) * CASTLE_TOWER_RING;
            // staggered heights so the ring is not a perfect palisade
            let height = 10.0 + tier as f32 * 3.4 + sin(tower as f32 * 2.1) * 3.0;
            self.push_instance(
                [at_x, deck - 2.0 + height * 0.5, at_z],
                [7.0, height, 7.0],
                [livery[0] * 0.9, livery[1] * 0.9, livery[2] * 0.9],
                angle,
                0.0,
                Shape::Cuboid,
            );
            self.push_instance(
                [at_x, deck - 1.0 + height, at_z],
                [6.5, 8.0, 6.5],
                [livery[0] * 1.4, livery[1] * 1.4, livery[2] * 1.4],
                angle,
                0.25,
                Shape::Cone,
            );
        }

        let banner_height = 6.0 + tier as f32 * 2.0;
        self.push_instance(
            [centre_x, deck + 12.0 + keep_height, centre_z],
            [3.0, banner_height, 3.0],
            [0.55, 0.85, 1.0],
            0.0,
            1.0,
            Shape::Sphere,
        );

        let health_fraction = self.castles.health[wizard] / self.castles.max_health[wizard];
        if health_fraction < CASTLE_SMOKE_THRESHOLD && self.rng.chance(0.4) {
            let offset_x = self.rng.range(-16.0, 16.0);
            let offset_z = self.rng.range(-16.0, 16.0);
            let rise = self.rng.range(6.0, keep_height);
            let drift_x = self.rng.range(-3.0, 3.0);
            let drift_y = self.rng.range(6.0, 16.0);
            let drift_z = self.rng.range(-3.0, 3.0);
            self.spawn_particle(
                [centre_x + offset_x, base + rise, centre_z + offset_z],
                [drift_x, drift_y, drift_z],
                1.4,
                5.0,
                [0.35, 0.33, 0.3],
                2.0,
                0.9,
            );
        }
        self.push_blip(
            centre_x,
            centre_z,
            if wizard == PLAYER {
                MapBlip::PlayerCastle
            } else {
                MapBlip::RivalCastle
            },
            1.0,
        );
    }

    pub(super) fn draw_castle_ruin(&mut self, wizard: usize, ground: f32) {
        let top = self.castles.pos_y[wizard] + 3.5;
        let bottom = ground - 5.0;
        self.push_instance(
            [
                self.castles.pos_x[wizard],
                (top + bottom) * 0.5,
                self.castles.pos_z[wizard],
            ],
            [30.0, top - bottom, 30.0],
            [0.22, 0.19, 0.17],
            0.0,
            0.0,
            Shape::Cuboid,
        );
    }

    // ---- creatures ---------------------------------------------------------
}
