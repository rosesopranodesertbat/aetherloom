//! Everything the renderer reads: instance and particle buffers, minimap
//! blips, the state block, the three prototype meshes and the minimap raster.
//!
//! Each creature draws itself in its own function. Instances are pushed with
//! named vectors rather than twelve bare floats, which is what the buffer
//! layout actually is.

use crate::math::*;
use crate::types::*;
use crate::world::*;

/// Scenery beyond this is not submitted at all; the haze hides the pop-in.
const SCENERY_CULL_RANGE: f32 = 1050.0;
/// Plinth half-width. The tower ring must fit inside it.
const CASTLE_FOOTPRINT: f32 = 27.0;
/// Height of the terrace above the levelled pad.
const CASTLE_DECK_HEIGHT: f32 = 6.0;
/// How far the plinth is sunk below the lowest ground it covers.
const CASTLE_PLINTH_SINK: f32 = 7.0;
/// Cap on that sinking, for ground later blown out from under it.
const CASTLE_PLINTH_MAX_DROP: f32 = 26.0;
const CASTLE_TOWER_RING: f32 = 19.0;
const CASTLE_KEEP_BASE_HEIGHT: f32 = 14.0;
const CASTLE_KEEP_HEIGHT_PER_TIER: f32 = 7.0;
/// Below this share of health the keep starts smoking.
const CASTLE_SMOKE_THRESHOLD: f32 = 0.6;

impl World {
    // ---- buffer writers ----------------------------------------------------
    fn push_instance(
        &mut self,
        position: [f32; 3],
        size: [f32; 3],
        colour: [f32; 3],
        yaw: f32,
        glow: f32,
        shape: Shape,
    ) {
        let r = &mut self.render;
        if r.instance_count >= MAX_INSTANCES {
            return;
        }
        let base = r.instance_count * INSTANCE_STRIDE;
        r.instances[base] = position[0];
        r.instances[base + 1] = position[1];
        r.instances[base + 2] = position[2];
        r.instances[base + 3] = size[0];
        r.instances[base + 4] = size[1];
        r.instances[base + 5] = size[2];
        r.instances[base + 6] = colour[0];
        r.instances[base + 7] = colour[1];
        r.instances[base + 8] = colour[2];
        r.instances[base + 9] = yaw;
        r.instances[base + 10] = glow;
        r.instances[base + 11] = shape.as_f32();
        r.instance_count += 1;
    }

    fn push_blip(&mut self, x: f32, z: f32, blip: MapBlip, scale: f32) {
        let r = &mut self.render;
        if r.map_blip_count >= MAX_MAP_BLIPS {
            return;
        }
        let base = r.map_blip_count * MAP_BLIP_STRIDE;
        r.map_blips[base] = x;
        r.map_blips[base + 1] = z;
        r.map_blips[base + 2] = blip.as_f32();
        r.map_blips[base + 3] = scale;
        r.map_blip_count += 1;
    }

    // ---- frame -------------------------------------------------------------
    pub fn build_frame(&mut self) {
        self.render.instance_count = 0;
        self.render.particle_count = 0;
        self.render.map_blip_count = 0;
        self.draw_scenery();
        for wizard in 0..=self.session.rival_count {
            self.draw_castle(wizard);
        }
        for index in 0..MAX_CREATURES {
            if self.creatures.alive[index] {
                self.draw_creature(index);
            }
        }
        self.draw_rival_carpets();
        self.draw_player_carpet();
        let orb_count = self.draw_orbs();
        self.draw_projectiles();
        self.pack_particles();
        self.push_blip(
            self.wizards.pos_x[PLAYER],
            self.wizards.pos_z[PLAYER],
            MapBlip::Player,
            1.6,
        );
        self.write_state_block(orb_count);
    }

    fn draw_scenery(&mut self) {
        for index in 0..self.scenery.count {
            let x = self.scenery.pos_x[index];
            let z = self.scenery.pos_z[index];
            if length_sq2(x - self.wizards.pos_x[PLAYER], z - self.wizards.pos_z[PLAYER])
                > SCENERY_CULL_RANGE * SCENERY_CULL_RANGE
            {
                continue;
            }
            let ground = self.height_at(x, z);
            if ground < 1.0 {
                continue; // sank under water when the ground was reshaped
            }
            let scale = self.scenery.scale[index];
            let spin = self.scenery.rotation[index];
            // Both kinds are sunk past their own ground sample: height is taken
            // at one point, so anything wide lifts off the downhill side.
            match self.scenery.kind[index] {
                SceneryKind::Palm => {
                    self.push_instance(
                        [x, ground + 5.0 * scale, z],
                        [1.5 * scale, 14.0 * scale, 1.5 * scale],
                        [0.36, 0.27, 0.16],
                        spin,
                        0.0,
                        Shape::Cuboid,
                    );
                    self.push_instance(
                        [x, ground + 14.0 * scale, z],
                        [9.0 * scale, 5.0 * scale, 9.0 * scale],
                        [0.22, 0.44, 0.20],
                        spin,
                        0.0,
                        Shape::Cone,
                    );
                }
                SceneryKind::Rock => self.push_instance(
                    [x, ground + 0.4 * scale, z],
                    [5.0 * scale, 6.0 * scale, 4.4 * scale],
                    [0.44, 0.41, 0.38],
                    spin,
                    0.0,
                    Shape::Cuboid,
                ),
            }
        }
    }

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

    fn draw_castle(&mut self, wizard: usize) {
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

    fn draw_castle_ruin(&mut self, wizard: usize, ground: f32) {
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
    fn draw_creature(&mut self, index: usize) {
        let kind = self.creatures.kind[index];
        let body = CreatureBody::of(self, index);
        match kind {
            CreatureKind::SandWorm => self.draw_sand_worm(&body),
            CreatureKind::Wasp => self.draw_wasp(&body),
            CreatureKind::Troll => self.draw_troll(&body),
            CreatureKind::Griffin => self.draw_griffin(&body),
            CreatureKind::Nest => self.draw_nest(&body),
            CreatureKind::Wraith => self.draw_wraith(&body),
            CreatureKind::Balloon => self.draw_balloon(&body),
            CreatureKind::Dragon => self.draw_dragon(&body),
        }
        let blip = match (kind, self.creatures.faction[index]) {
            (CreatureKind::Balloon, _) => MapBlip::Balloon,
            (CreatureKind::Nest, _) => MapBlip::Nest,
            (_, faction) if faction.is_player() => MapBlip::AlliedCreature,
            (_, Faction(2)) => MapBlip::RivalOrb,
            _ => MapBlip::WildCreature,
        };
        let scale = match kind {
            CreatureKind::Nest => 1.6,
            CreatureKind::Dragon => 1.5,
            CreatureKind::Balloon => 0.8,
            _ => 1.0,
        };
        self.push_blip(body.x, body.z, blip, scale);
    }

    fn draw_sand_worm(&mut self, body: &CreatureBody) {
        const SEGMENTS: i32 = 6;
        const SEGMENT_SPACING: f32 = 5.4;
        for segment in 0..SEGMENTS {
            let back = segment as f32 * SEGMENT_SPACING;
            let at_x = body.x - body.facing_sin * back;
            let at_z = body.z - body.facing_cos * back;
            // each segment lags the one ahead, giving the undulation
            let bob = sin(body.phase - segment as f32 * 0.8) * 3.2;
            let girth = 6.8 - segment as f32 * 0.62;
            self.push_instance(
                [at_x, body.y + bob + 1.0, at_z],
                [girth, girth * 0.85, girth],
                [
                    0.62 + body.hurt * 0.35,
                    0.52 - body.hurt * 0.2,
                    0.28,
                ],
                body.facing,
                body.hurt * 0.5,
                Shape::Sphere,
            );
            if segment < SEGMENTS - 1 {
                self.push_instance(
                    [at_x, body.y + bob + 1.0 + girth * 0.42, at_z],
                    [girth * 0.5, girth * 0.36, girth * 0.8],
                    [0.40, 0.30, 0.15],
                    body.facing,
                    0.0,
                    Shape::Cuboid,
                );
            }
        }
        let head_bob = sin(body.phase) * 3.2;
        self.push_instance(
            body.ahead(3.4, head_bob + 0.2, 0.0),
            [3.6, 1.8, 2.2],
            [0.28, 0.09, 0.09],
            body.facing,
            0.0,
            Shape::Cuboid,
        );
        for side in [1.0f32, -1.0] {
            self.push_instance(
                body.ahead(2.4, head_bob + 2.4, side * 1.7),
                [1.5, 1.5, 1.5],
                [0.04, 0.03, 0.04],
                body.facing,
                0.0,
                Shape::Sphere,
            );
        }
    }

    fn draw_wasp(&mut self, body: &CreatureBody) {
        self.push_instance(
            [body.x, body.y, body.z],
            [3.4, 3.0, 5.0],
            [0.85 + body.hurt * 0.15, 0.72, 0.18],
            body.facing,
            body.hurt * 0.6,
            Shape::Sphere,
        );
        for (back, width, height) in [(1.4f32, 3.2f32, 2.8f32), (2.8, 2.5, 2.2)] {
            self.push_instance(
                body.ahead(-back, 0.0, 0.0),
                [width, height, 1.2],
                [0.11, 0.08, 0.05],
                body.facing,
                0.0,
                Shape::Cuboid,
            );
        }
        self.push_instance(
            body.ahead(-4.4, 0.0, 0.0),
            [1.4, 2.8, 1.4],
            [0.16, 0.12, 0.10],
            body.facing,
            0.0,
            Shape::Cone,
        );
        self.push_instance(
            body.ahead(2.9, 0.3, 0.0),
            [2.7, 2.5, 2.5],
            [0.26, 0.19, 0.07],
            body.facing,
            0.0,
            Shape::Sphere,
        );
        for side in [1.0f32, -1.0] {
            self.push_instance(
                body.ahead(3.7, 0.7, side * 0.9),
                [1.2, 1.2, 1.2],
                [0.03, 0.03, 0.03],
                body.facing,
                0.0,
                Shape::Sphere,
            );
        }
        // the wings are one blurred plate, rocked by the phase
        let beat = sin(body.phase) * 0.9;
        self.push_instance(
            [body.x, body.y + 2.2, body.z],
            [7.5, 0.4, 2.2],
            [0.9, 0.9, 0.95],
            body.facing + beat,
            0.1,
            Shape::Cuboid,
        );
    }

    fn draw_troll(&mut self, body: &CreatureBody) {
        let skin = 0.35 + body.hurt * 0.5;
        self.push_instance(
            [body.x, body.y + 5.8, body.z],
            [8.0, 10.0, 6.5],
            [skin, 0.42, 0.32],
            body.facing,
            body.hurt * 0.5,
            Shape::Cuboid,
        );
        self.push_instance(
            [body.x, body.y + 12.6, body.z],
            [5.4, 4.6, 5.0],
            [skin + 0.07, 0.48, 0.36],
            body.facing,
            body.hurt * 0.5,
            Shape::Cuboid,
        );
        for side in [1.0f32, -1.0] {
            self.push_instance(
                body.ahead(2.5, 13.4, side * 1.3),
                [1.2, 1.2, 0.7],
                [1.0, 0.80, 0.22],
                body.facing,
                0.7,
                Shape::Cuboid,
            );
            self.push_instance(
                body.beside(side * 2.3, 15.8),
                [1.6, 3.6, 1.6],
                [0.88, 0.84, 0.72],
                body.facing,
                0.0,
                Shape::Cone,
            );
        }
        let swing = sin(body.phase) * 1.6;
        for (side, lift) in [(-1.0f32, swing), (1.0, -swing)] {
            self.push_instance(
                body.beside(side * 5.5, 6.0 + lift),
                [2.6, 8.0, 2.6],
                [0.30, 0.36, 0.28],
                body.facing,
                0.0,
                Shape::Cuboid,
            );
            self.push_instance(
                body.beside(side * 2.2, 1.5),
                [2.9, 4.4, 3.2],
                [0.26, 0.31, 0.24],
                body.facing,
                0.0,
                Shape::Cuboid,
            );
        }
    }

    fn draw_griffin(&mut self, body: &CreatureBody) {
        self.push_instance(
            [body.x, body.y, body.z],
            [4.6, 4.2, 8.0],
            [0.80 + body.hurt * 0.2, 0.68, 0.42],
            body.facing,
            body.hurt * 0.5,
            Shape::Sphere,
        );
        self.push_instance(
            body.ahead(4.4, 1.6, 0.0),
            [3.4, 3.2, 3.4],
            [0.94, 0.90, 0.78],
            body.facing,
            0.0,
            Shape::Sphere,
        );
        self.push_instance(
            body.ahead(6.1, 1.3, 0.0),
            [1.7, 1.9, 2.6],
            [0.95, 0.70, 0.14],
            body.facing,
            0.0,
            Shape::Cone,
        );
        for side in [1.0f32, -1.0] {
            self.push_instance(
                body.ahead(5.0, 2.4, side * 1.2),
                [1.0, 1.0, 1.0],
                [0.04, 0.03, 0.03],
                body.facing,
                0.0,
                Shape::Sphere,
            );
        }
        self.push_instance(
            body.ahead(-5.4, -0.4, 0.0),
            [2.2, 2.2, 5.2],
            [0.68, 0.55, 0.32],
            body.facing,
            0.0,
            Shape::Cone,
        );
        let beat = sin(body.phase * 2.0) * 0.55;
        for (side, lift) in [(-1.0f32, beat), (1.0, -beat)] {
            self.push_instance(
                body.beside(side * 7.0, 1.5 + lift * 4.0),
                [12.0, 1.2, 6.0],
                [0.92, 0.86, 0.7],
                body.facing,
                0.05,
                Shape::Cone,
            );
        }
    }

    fn draw_nest(&mut self, body: &CreatureBody) {
        // a slow swell, as if something inside is breathing
        let swell = 1.0 + sin(body.phase * 0.7) * 0.06;
        self.push_instance(
            [body.x, body.y - 5.0, body.z],
            [19.0, 17.0, 19.0],
            [0.24, 0.16, 0.26],
            0.0,
            0.0,
            Shape::Cuboid,
        );
        self.push_instance(
            [body.x, body.y + 9.0, body.z],
            [15.0 * swell, 17.0 * swell, 15.0 * swell],
            [0.40, 0.18, 0.44],
            body.phase * 0.1,
            0.25,
            Shape::Cone,
        );
        for spike in 0..4 {
            let angle = spike as f32 * core::f32::consts::FRAC_PI_2 + core::f32::consts::FRAC_PI_4;
            self.push_instance(
                [
                    body.x + cos(angle) * 7.6,
                    body.y + 4.2,
                    body.z + sin(angle) * 7.6,
                ],
                [2.2, 7.4, 2.2],
                [0.28, 0.11, 0.32],
                angle,
                0.0,
                Shape::Cone,
            );
        }
        self.push_instance(
            [body.x, body.y + 20.0, body.z],
            [4.0, 4.0, 4.0],
            [0.85, 0.35, 1.0],
            0.0,
            1.0,
            Shape::Sphere,
        );
    }

    fn draw_wraith(&mut self, body: &CreatureBody) {
        self.push_instance(
            [body.x, body.y, body.z],
            [5.0, 6.5, 5.0],
            [0.55, 0.42, 1.0],
            body.facing,
            0.75,
            Shape::Sphere,
        );
        self.push_instance(
            [body.x, body.y + 3.7, body.z],
            [5.8, 5.2, 5.8],
            [0.28, 0.20, 0.64],
            body.facing,
            0.2,
            Shape::Cone,
        );
        for side in [1.0f32, -1.0] {
            self.push_instance(
                body.ahead(1.9, 1.1, side * 1.2),
                [1.1, 1.1, 1.1],
                [1.0, 0.95, 0.55],
                body.facing,
                1.0,
                Shape::Sphere,
            );
        }
        self.push_instance(
            [body.x, body.y - 5.5, body.z],
            [6.0, 7.0, 6.0],
            [0.36, 0.28, 0.85],
            body.facing,
            0.5,
            Shape::Cone,
        );
        if self.rng.chance(0.4) {
            let offset = [
                self.rng.range(-4.0, 4.0),
                self.rng.range(-4.0, 4.0),
                self.rng.range(-4.0, 4.0),
            ];
            let rise = self.rng.range(2.0, 8.0);
            self.spawn_particle(
                [body.x + offset[0], body.y + offset[1], body.z + offset[2]],
                [0.0, rise, 0.0],
                0.6,
                2.4,
                [0.5, 0.4, 1.0],
                0.0,
                1.0,
            );
        }
    }

    fn draw_balloon(&mut self, body: &CreatureBody) {
        let envelope = if body.faction.is_player() {
            [0.35, 0.6, 1.0]
        } else {
            [0.85, 0.3, 0.28]
        };
        self.push_instance(
            [body.x, body.y, body.z],
            [9.0, 11.0, 9.0],
            envelope,
            body.facing,
            0.15,
            Shape::Sphere,
        );
        self.push_instance(
            [body.x, body.y + 5.6, body.z],
            [4.6, 3.4, 4.6],
            [0.92, 0.88, 0.72],
            body.facing,
            0.1,
            Shape::Cone,
        );
        self.push_instance(
            [body.x, body.y - 2.0, body.z],
            [9.4, 1.4, 9.4],
            [0.20, 0.16, 0.12],
            body.facing,
            0.0,
            Shape::Cuboid,
        );
        self.push_instance(
            [body.x, body.y - 6.0, body.z],
            [0.9, 6.0, 0.9],
            [0.28, 0.22, 0.16],
            body.facing,
            0.0,
            Shape::Cuboid,
        );
        self.push_instance(
            [body.x, body.y - 9.0, body.z],
            [4.0, 3.4, 4.0],
            [0.42, 0.32, 0.2],
            body.facing,
            0.0,
            Shape::Cuboid,
        );
    }

    fn draw_dragon(&mut self, body: &CreatureBody) {
        const SEGMENTS: i32 = 4;
        for segment in 0..SEGMENTS {
            let back = segment as f32 * 7.0;
            let at_x = body.x - body.facing_sin * back;
            let at_z = body.z - body.facing_cos * back;
            let girth = 8.0 - segment as f32 * 1.3;
            let at_y = body.y + sin(body.phase - segment as f32) * 1.4;
            self.push_instance(
                [at_x, at_y, at_z],
                [girth, girth * 0.9, girth * 1.2],
                [0.55 + body.hurt * 0.4, 0.16, 0.18],
                body.facing,
                body.hurt * 0.5,
                Shape::Sphere,
            );
            if segment > 0 {
                self.push_instance(
                    [at_x, at_y + girth * 0.52, at_z],
                    [girth * 0.26, girth * 0.55, girth * 0.72],
                    [0.28, 0.07, 0.09],
                    body.facing,
                    0.0,
                    Shape::Cone,
                );
            }
        }
        let beat = sin(body.phase * 1.6) * 0.6;
        for (side, lift) in [(-1.0f32, beat), (1.0, -beat)] {
            self.push_instance(
                body.beside(side * 11.0, 2.0 + lift * 6.0),
                [20.0, 1.6, 9.0],
                [0.35, 0.10, 0.14],
                body.facing,
                0.05,
                Shape::Cone,
            );
        }
        self.push_instance(
            body.ahead(8.0, 1.0, 0.0),
            [5.0, 5.0, 7.0],
            [0.9, 0.4, 0.2],
            body.facing,
            0.4,
            Shape::Cone,
        );
        for side in [1.0f32, -1.0] {
            self.push_instance(
                body.ahead(6.8, 3.6, side * 1.8),
                [1.4, 3.2, 1.4],
                [0.86, 0.80, 0.66],
                body.facing,
                0.0,
                Shape::Cone,
            );
            self.push_instance(
                body.ahead(9.6, 1.7, side * 1.5),
                [1.3, 1.3, 1.3],
                [1.0, 0.86, 0.20],
                body.facing,
                1.0,
                Shape::Sphere,
            );
        }
        self.push_instance(
            body.ahead(-25.0, 0.0, 0.0),
            [2.4, 5.2, 2.4],
            [0.38, 0.09, 0.11],
            body.facing,
            0.0,
            Shape::Cone,
        );
    }

    // ---- carpets, orbs, projectiles ----------------------------------------
    fn draw_rival_carpets(&mut self) {
        for wizard in 1..=self.session.rival_count {
            if !self.wizards.alive[wizard] {
                continue;
            }
            let at = [
                self.wizards.pos_x[wizard],
                self.wizards.pos_y[wizard],
                self.wizards.pos_z[wizard],
            ];
            let yaw = self.wizards.yaw[wizard];
            self.push_instance(at, [13.0, 1.1, 17.0], [0.72, 0.16, 0.14], yaw, 0.2, Shape::Cuboid);
            self.push_instance(
                [at[0], at[1] + 4.0, at[2]],
                [4.0, 6.0, 4.0],
                [0.9, 0.8, 0.6],
                yaw,
                0.1,
                Shape::Sphere,
            );
            if self.wizards.ward_remaining[wizard] > 0.0 {
                self.push_instance(
                    [at[0], at[1] + 2.0, at[2]],
                    [20.0, 20.0, 20.0],
                    [1.0, 0.5, 0.4],
                    0.0,
                    0.9,
                    Shape::Sphere,
                );
            }
            self.push_blip(at[0], at[2], MapBlip::RivalWizard, 1.4);
        }
    }

    /// Emitted as one contiguous run so the renderer can drop the whole thing
    /// in first person — see `carpet_first`/`carpet_last`.
    fn draw_player_carpet(&mut self) {
        self.render.carpet_first = self.render.instance_count as i32;
        let at = [
            self.wizards.pos_x[PLAYER],
            self.wizards.pos_y[PLAYER],
            self.wizards.pos_z[PLAYER],
        ];
        let yaw = self.wizards.yaw[PLAYER];
        let bob = sin(self.session.elapsed * 2.3) * 0.35;
        self.push_instance(
            [at[0], at[1] - 2.7 + bob, at[2]],
            [16.6, 0.5, 20.6],
            [0.88, 0.65, 0.24],
            yaw,
            0.05,
            Shape::Cuboid,
        );
        self.push_instance(
            [at[0], at[1] - 2.1 + bob, at[2]],
            [15.0, 0.9, 19.0],
            [0.13, 0.20, 0.56],
            yaw,
            0.02,
            Shape::Cuboid,
        );
        self.push_instance(
            [at[0], at[1] - 1.6 + bob, at[2]],
            [5.2, 0.5, 6.6],
            [0.72, 0.20, 0.16],
            yaw,
            0.04,
            Shape::Cuboid,
        );
        if self.wizards.ward_remaining[PLAYER] > 0.0 {
            self.push_instance(
                at,
                [24.0, 24.0, 24.0],
                [0.42, 0.72, 1.0],
                0.0,
                0.8,
                Shape::Sphere,
            );
        }
        self.render.carpet_last = self.render.instance_count as i32;
    }

    fn draw_orbs(&mut self) -> usize {
        let mut count = 0;
        for index in 0..MAX_ORBS {
            if !self.orbs.alive[index] {
                continue;
            }
            count += 1;
            let size = 2.0 + self.orbs.amount[index] * 0.18;
            let pulse = 0.75 + sin(self.orbs.bob_phase[index] * 2.0) * 0.25;
            // gold is nobody's yet, white is yours and inbound, red is theirs
            let (tint, blip) = if self.orbs.claimed_by[index].is_player() {
                ([0.72, 0.92, 1.00], MapBlip::ClaimedOrb)
            } else if self.orbs.claimed_by[index].is_wild() {
                ([1.00, 0.78, 0.16], MapBlip::UnclaimedOrb)
            } else {
                ([1.00, 0.36, 0.30], MapBlip::RivalOrb)
            };
            let at = [
                self.orbs.pos_x[index],
                self.orbs.pos_y[index],
                self.orbs.pos_z[index],
            ];
            self.push_instance(
                at,
                [size, size, size],
                [tint[0] * pulse, tint[1] * pulse, tint[2] * pulse],
                0.0,
                1.0,
                Shape::Sphere,
            );
            self.push_blip(at[0], at[2], blip, 0.7);
            if self.rng.chance(0.10) {
                let drift = [
                    self.rng.range(-3.0, 3.0),
                    self.rng.range(3.0, 9.0),
                    self.rng.range(-3.0, 3.0),
                ];
                self.spawn_particle(at, drift, 0.8, 1.8, [0.45, 0.8, 1.0], 0.0, 1.2);
            }
        }
        count
    }

    fn draw_projectiles(&mut self) {
        for index in 0..MAX_PROJECTILES {
            if !self.projectiles.alive[index] {
                continue;
            }
            let (colour, size) = self.projectiles.kind[index].head_style();
            self.push_instance(
                [
                    self.projectiles.pos_x[index],
                    self.projectiles.pos_y[index],
                    self.projectiles.pos_z[index],
                ],
                [size, size, size],
                colour,
                0.0,
                1.0,
                Shape::Sphere,
            );
        }
    }

    fn pack_particles(&mut self) {
        for index in 0..MAX_PARTICLES {
            if self.particles.life[index] <= 0.0 {
                continue;
            }
            // fades and shrinks together as it ages
            let remaining = self.particles.life[index] / self.particles.initial_life[index];
            let base = self.render.particle_count * PARTICLE_STRIDE;
            self.render.particles[base] = self.particles.pos_x[index];
            self.render.particles[base + 1] = self.particles.pos_y[index];
            self.render.particles[base + 2] = self.particles.pos_z[index];
            self.render.particles[base + 3] =
                self.particles.size[index] * (0.35 + remaining * 0.9);
            self.render.particles[base + 4] = self.particles.red[index];
            self.render.particles[base + 5] = self.particles.green[index];
            self.render.particles[base + 6] = self.particles.blue[index];
            self.render.particles[base + 7] = remaining * remaining;
            self.render.particle_count += 1;
        }
    }

    // ---- the state block ---------------------------------------------------
    /// Layout is fixed: `game.js` indexes it directly. Slots 40..52 are spell
    /// cooldowns, 60..72 unlocks, 80..92 affordability, 100..112 prices.
    fn write_state_block(&mut self, orb_count: usize) {
        let ground = self.height_at(self.wizards.pos_x[PLAYER], self.wizards.pos_z[PLAYER]);
        let target = self.realm_target();
        let mut best_rival_store = 0.0f32;
        for wizard in 1..=self.session.rival_count {
            best_rival_store = max(best_rival_store, self.castles.stored_mana[wizard]);
        }
        let state = &mut self.render.state;
        state[0] = self.wizards.pos_x[PLAYER];
        state[1] = self.wizards.pos_y[PLAYER];
        state[2] = self.wizards.pos_z[PLAYER];
        state[3] = self.wizards.yaw[PLAYER];
        state[4] = self.wizards.pitch[PLAYER];
        state[5] = self.wizards.roll[PLAYER];
        state[6] = length3(
            self.wizards.vel_x[PLAYER],
            self.wizards.vel_y[PLAYER],
            self.wizards.vel_z[PLAYER],
        );
        state[7] = self.wizards.health[PLAYER];
        state[8] = WIZARD_MAX_HEALTH;
        state[9] = self.wizards.mana[PLAYER];
        state[10] = BASE_MANA_CAP
            + self.castles.tier[PLAYER] as f32 * MANA_CAP_PER_TIER
            + self.wizards.mana_cap_bonus[PLAYER];
        state[11] = self.spells.selected as f32;
        state[12] = SPELL_COUNT as f32;
        state[13] = self.castles.stored_mana[PLAYER] / target;
        state[14] = best_rival_store / target;
        state[15] = 1.0;
        state[16] = self.castles.tier[PLAYER] as f32;
        state[17] = self.session.realm as f32;
        state[18] = self.session.outcome.as_f32();
        state[19] = self.session.screen_shake;
        state[20] = self.wizards.ward_remaining[PLAYER];
        state[21] = self.wizards.haste_remaining[PLAYER];
        state[22] = ground;
        state[23] = self.wizards.pos_y[PLAYER] - max(ground, SEA_LEVEL);
        state[24] = self.session.kills;
        state[25] = self.castles.stored_mana[PLAYER];
        state[26] = 0.0;
        state[27] = self.castles.health[PLAYER] / self.castles.max_health[PLAYER];
        state[28] = self.wizards.health[1] / WIZARD_MAX_HEALTH;
        state[29] = if self.wizards.alive[1] {
            sqrt(length_sq2(
                self.wizards.pos_x[1] - self.wizards.pos_x[PLAYER],
                self.wizards.pos_z[1] - self.wizards.pos_z[PLAYER],
            ))
        } else {
            -1.0
        };
        state[30] = orb_count as f32;
        state[31] = 0.0;
        state[32] = self.session.elapsed;
        state[33] = self.castles.pos_x[PLAYER];
        state[34] = self.castles.pos_z[PLAYER];
        state[35] = self.castles.pos_y[PLAYER];
        state[36] = self.session.rival_count as f32;
        state[37] = GRID_WIDTH as f32;
        state[38] = CELL_SIZE;
        // fortress progress: fill, what this tier can hold, and the target
        state[53] = CASTLE_CAPACITY_PER_TIER * self.castles.tier[PLAYER] as f32 / target;
        state[54] = self.castles.stored_mana[PLAYER];
        state[55] = CASTLE_CAPACITY_PER_TIER * self.castles.tier[PLAYER] as f32;
        state[56] = target;
        state[57] = MANA_REGEN_CEILING;

        for slot in 0..SPELL_COUNT {
            let spell = Spell::ALL[slot];
            let price = match spell {
                Spell::Fortress => {
                    34.0 + self.castles.tier[PLAYER] as f32 * 26.0
                }
                other => other.base_cost(),
            };
            let cooldown = spell.cooldown();
            self.render.state[40 + slot] = if cooldown > 0.0 {
                self.spells.cooldown_remaining[slot] / cooldown
            } else {
                0.0
            };
            self.render.state[60 + slot] = if self.spells.unlocked[slot] { 1.0 } else { 0.0 };
            self.render.state[80 + slot] = if self.wizards.mana[PLAYER] >= price {
                1.0
            } else {
                0.0
            };
            self.render.state[100 + slot] = price;
        }
    }

    // ---- camera ------------------------------------------------------------
    /// Builds the view-projection matrix and its inverse for the renderer.
    #[allow(clippy::too_many_arguments)]
    pub fn update_camera(
        &mut self,
        fov_y: f32,
        aspect: f32,
        near: f32,
        far: f32,
        eye: [f32; 3],
        centre: [f32; 3],
        up: [f32; 3],
    ) {
        let projection = perspective(fov_y, aspect, near, far);
        let view = look_at(eye, centre, up);
        let view_projection = multiply(&projection, &view);
        let inverse = invert(&view_projection);
        self.camera_matrices[..16].copy_from_slice(&view_projection);
        self.camera_matrices[16..].copy_from_slice(&inverse);
    }
}

/// The values every creature-drawing routine needs, resolved once.
struct CreatureBody {
    x: f32,
    y: f32,
    z: f32,
    facing: f32,
    facing_sin: f32,
    facing_cos: f32,
    phase: f32,
    /// 1.0 while the hit flash is showing, 0.0 otherwise.
    hurt: f32,
    faction: Faction,
}

impl CreatureBody {
    fn of(world: &World, index: usize) -> CreatureBody {
        let facing = world.creatures.facing[index];
        CreatureBody {
            x: world.creatures.pos_x[index],
            y: world.creatures.pos_y[index],
            z: world.creatures.pos_z[index],
            facing,
            facing_sin: sin(facing),
            facing_cos: cos(facing),
            phase: world.creatures.phase[index],
            hurt: if world.creatures.hurt_flash[index] > 0.0 {
                1.0
            } else {
                0.0
            },
            faction: world.creatures.faction[index],
        }
    }

    /// A point `forward` along the facing, `up` above, and `side` to the
    /// creature's own right.
    fn ahead(&self, forward: f32, up: f32, side: f32) -> [f32; 3] {
        [
            self.x + self.facing_sin * forward + self.facing_cos * side,
            self.y + up,
            self.z + self.facing_cos * forward - self.facing_sin * side,
        ]
    }

    /// A point purely to one side, at a given height.
    fn beside(&self, side: f32, up: f32) -> [f32; 3] {
        [
            self.x + self.facing_cos * side,
            self.y + up,
            self.z - self.facing_sin * side,
        ]
    }
}

// ---- prototype meshes ------------------------------------------------------
/// All three live in a -1..1 box; the vertex shader scales by half the instance
/// size, so a scale of s spans exactly s world units. Vertices are interleaved
/// position and normal.
const MESH_VERTEX_CAPACITY: usize = 1024 * 6;
const MESH_INDEX_CAPACITY: usize = 1024;

pub struct MeshLibrary {
    pub vertices: [[f32; MESH_VERTEX_CAPACITY]; Shape::COUNT],
    pub indices: [[u16; MESH_INDEX_CAPACITY]; Shape::COUNT],
    pub vertex_floats: [usize; Shape::COUNT],
    pub index_counts: [usize; Shape::COUNT],
}

impl MeshLibrary {
    pub const fn new() -> MeshLibrary {
        MeshLibrary {
            vertices: [[0.0; MESH_VERTEX_CAPACITY]; Shape::COUNT],
            indices: [[0; MESH_INDEX_CAPACITY]; Shape::COUNT],
            vertex_floats: [0; Shape::COUNT],
            index_counts: [0; Shape::COUNT],
        }
    }

    fn reset(&mut self, shape: usize) {
        self.vertex_floats[shape] = 0;
        self.index_counts[shape] = 0;
    }

    fn vertex_count(&self, shape: usize) -> usize {
        self.vertex_floats[shape] / 6
    }

    fn add_vertex(&mut self, shape: usize, position: [f32; 3], normal: [f32; 3]) {
        let base = self.vertex_floats[shape];
        if base + 6 > MESH_VERTEX_CAPACITY {
            return;
        }
        self.vertices[shape][base] = position[0];
        self.vertices[shape][base + 1] = position[1];
        self.vertices[shape][base + 2] = position[2];
        self.vertices[shape][base + 3] = normal[0];
        self.vertices[shape][base + 4] = normal[1];
        self.vertices[shape][base + 5] = normal[2];
        self.vertex_floats[shape] = base + 6;
    }

    fn add_triangle(&mut self, shape: usize, a: usize, b: usize, c: usize) {
        let base = self.index_counts[shape];
        if base + 3 > MESH_INDEX_CAPACITY {
            return;
        }
        self.indices[shape][base] = a as u16;
        self.indices[shape][base + 1] = b as u16;
        self.indices[shape][base + 2] = c as u16;
        self.index_counts[shape] = base + 3;
    }

    /// Builds all three. Every face is wound counter-clockwise when seen from
    /// outside; getting that backwards renders the shape inside out under
    /// back-face culling.
    pub fn build(&mut self) {
        for shape in 0..Shape::COUNT {
            self.reset(shape);
        }
        self.build_cuboid();
        self.build_sphere();
        self.build_cone();
    }

    fn build_cuboid(&mut self) {
        const SHAPE: usize = 0;
        const FACES: [([f32; 3], [[f32; 3]; 4]); 6] = [
            ([1.0, 0.0, 0.0], [[1.0, -1.0, -1.0], [1.0, 1.0, -1.0], [1.0, 1.0, 1.0], [1.0, -1.0, 1.0]]),
            ([-1.0, 0.0, 0.0], [[-1.0, -1.0, 1.0], [-1.0, 1.0, 1.0], [-1.0, 1.0, -1.0], [-1.0, -1.0, -1.0]]),
            ([0.0, 1.0, 0.0], [[-1.0, 1.0, -1.0], [-1.0, 1.0, 1.0], [1.0, 1.0, 1.0], [1.0, 1.0, -1.0]]),
            ([0.0, -1.0, 0.0], [[-1.0, -1.0, 1.0], [-1.0, -1.0, -1.0], [1.0, -1.0, -1.0], [1.0, -1.0, 1.0]]),
            ([0.0, 0.0, 1.0], [[-1.0, -1.0, 1.0], [1.0, -1.0, 1.0], [1.0, 1.0, 1.0], [-1.0, 1.0, 1.0]]),
            ([0.0, 0.0, -1.0], [[-1.0, 1.0, -1.0], [1.0, 1.0, -1.0], [1.0, -1.0, -1.0], [-1.0, -1.0, -1.0]]),
        ];
        for (normal, quad) in FACES.iter() {
            let first = self.vertex_count(SHAPE);
            for corner in quad.iter() {
                self.add_vertex(SHAPE, *corner, *normal);
            }
            self.add_triangle(SHAPE, first, first + 1, first + 2);
            self.add_triangle(SHAPE, first, first + 2, first + 3);
        }
    }

    fn build_sphere(&mut self) {
        const SHAPE: usize = 1;
        const SEGMENTS: usize = 14;
        const RINGS: usize = 9;
        for ring in 0..=RINGS {
            let polar = ring as f32 / RINGS as f32 * core::f32::consts::PI;
            for segment in 0..=SEGMENTS {
                let azimuth = segment as f32 / SEGMENTS as f32 * core::f32::consts::TAU;
                let point = [
                    sin(polar) * cos(azimuth),
                    cos(polar),
                    sin(polar) * sin(azimuth),
                ];
                // on a unit sphere the position is also the normal
                self.add_vertex(SHAPE, point, point);
            }
        }
        for ring in 0..RINGS {
            for segment in 0..SEGMENTS {
                let upper = ring * (SEGMENTS + 1) + segment;
                let lower = upper + SEGMENTS + 1;
                self.add_triangle(SHAPE, upper, upper + 1, lower);
                self.add_triangle(SHAPE, upper + 1, lower + 1, lower);
            }
        }
    }

    fn build_cone(&mut self) {
        const SHAPE: usize = 2;
        const SEGMENTS: usize = 14;
        /// Tilts the side normals up so the cone lights like a cone rather
        /// than a cylinder.
        const SIDE_NORMAL_LIFT: f32 = 0.45;
        for segment in 0..SEGMENTS {
            let start = segment as f32 / SEGMENTS as f32 * core::f32::consts::TAU;
            let end = (segment + 1) as f32 / SEGMENTS as f32 * core::f32::consts::TAU;
            let mid = (start + end) * 0.5;
            let normal_length = sqrt(
                cos(mid) * cos(mid) + SIDE_NORMAL_LIFT * SIDE_NORMAL_LIFT + sin(mid) * sin(mid),
            );
            let side_normal = [
                cos(mid) / normal_length,
                SIDE_NORMAL_LIFT / normal_length,
                sin(mid) / normal_length,
            ];
            let rim_start = [cos(start), -1.0, sin(start)];
            let rim_end = [cos(end), -1.0, sin(end)];

            let first = self.vertex_count(SHAPE);
            self.add_vertex(SHAPE, rim_start, side_normal);
            self.add_vertex(SHAPE, rim_end, side_normal);
            self.add_vertex(SHAPE, [0.0, 1.0, 0.0], side_normal);
            self.add_triangle(SHAPE, first, first + 2, first + 1);

            let cap = self.vertex_count(SHAPE);
            let down = [0.0, -1.0, 0.0];
            self.add_vertex(SHAPE, [0.0, -1.0, 0.0], down);
            self.add_vertex(SHAPE, rim_end, down);
            self.add_vertex(SHAPE, rim_start, down);
            self.add_triangle(SHAPE, cap, cap + 2, cap + 1);
        }
    }
}

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
