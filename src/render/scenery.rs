//! Palms, boulders and everything else scattered across the terrain.
//!
//! Placement is clumped (see `terrain::scatter_scenery`); this file is only
//! concerned with turning one scenery slot into geometry.

use super::*;

impl World {
    /// Cheap deterministic variation from a scenery item's own stored spin
    /// and scale. It has to be stable frame to frame, so it cannot come from
    /// the RNG — every draw would reshuffle the boulders.
    fn scenery_jitter(spin: f32, scale: f32, salt: f32) -> f32 {
        sin(spin * (7.13 + salt * 3.9) + scale * (5.31 + salt * 2.7) + salt)
    }

    /// Three passes, nearest tier first, each stopping at the budget. Far
    /// scenery is what disappears when the frame is full, which is the one
    /// place it will not be noticed.
    pub(super) fn draw_scenery(&mut self) {
        for tier in [
            SceneryDetail::Full,
            SceneryDetail::Reduced,
            SceneryDetail::Distant,
        ] {
            if !self.draw_scenery_tier(tier) {
                return;
            }
        }
    }

    /// Returns false once the budget is spent, so the remaining tiers are
    /// skipped rather than part-drawn.
    pub(super) fn draw_scenery_tier(&mut self, tier: SceneryDetail) -> bool {
        let eye_x = self.wizards.pos_x[PLAYER];
        let eye_z = self.wizards.pos_z[PLAYER];
        for index in 0..self.scenery.count {
            if self.render.instance_count >= SCENERY_INSTANCE_BUDGET {
                return false;
            }
            let x = self.scenery.pos_x[index];
            let z = self.scenery.pos_z[index];
            let range_sq = length_sq2(x - eye_x, z - eye_z);
            let detail = if range_sq < SCENERY_DETAIL_RANGE * SCENERY_DETAIL_RANGE {
                SceneryDetail::Full
            } else if range_sq < SCENERY_MID_RANGE * SCENERY_MID_RANGE {
                SceneryDetail::Reduced
            } else if range_sq < SCENERY_CULL_RANGE * SCENERY_CULL_RANGE {
                SceneryDetail::Distant
            } else {
                continue;
            };
            if detail != tier {
                continue;
            }
            let ground = self.height_at(x, z);
            if ground < 1.0 {
                continue; // sank under water when the ground was reshaped
            }
            let scale = self.scenery.scale[index];
            let spin = self.scenery.rotation[index];
            let burn = self.scenery.burn_remaining[index];
            // only the near tier is shadowed: past that the blob is a couple
            // of pixels and costs an instance each
            if detail == SceneryDetail::Full && self.scenery.standing[index] {
                let radius = match self.scenery.kind[index] {
                    SceneryKind::Palm => 7.0 * scale,
                    SceneryKind::Rock => 4.4 * scale,
                };
                self.push_shadow(x, z, ground, radius, 0.34);
            }
            match self.scenery.kind[index] {
                SceneryKind::Palm => {
                    if self.scenery.standing[index] {
                        self.draw_palm(x, ground, z, scale, spin, burn, detail);
                    } else {
                        self.draw_burnt_stump(x, ground, z, scale, spin);
                    }
                }
                SceneryKind::Rock => self.draw_boulder(x, ground, z, scale, spin, detail),
            }
        }
        true
    }

    /// A leaning segmented trunk under a crown of drooping fronds, with a few
    /// coconuts tucked underneath. Charring and embers ride on `burn`.
    pub(super) fn draw_palm(
        &mut self,
        x: f32,
        ground: f32,
        z: f32,
        scale: f32,
        spin: f32,
        burn: f32,
        detail: SceneryDetail,
    ) {
        let trunk_segments = if detail == SceneryDetail::Full { 5 } else { 3 };
        const TRUNK_HEIGHT: f32 = 17.0;
        // how far the crown ends up from directly above the roots
        let lean = World::scenery_jitter(spin, scale, 0.0) * 3.4 * scale;
        let lean_x = cos(spin) * lean;
        let lean_z = sin(spin) * lean;
        let crown_y = ground + TRUNK_HEIGHT * scale;
        let char_mix = clamp(burn * 0.16, 0.0, 0.85);
        let bark = [
            0.47 - char_mix * 0.38,
            0.36 - char_mix * 0.31,
            0.22 - char_mix * 0.18,
        ];
        let frond = [
            0.24 - char_mix * 0.18,
            0.47 - char_mix * 0.40,
            0.19 - char_mix * 0.15,
        ];

        if detail == SceneryDetail::Distant {
            // one post and one canopy: enough to read as a palm on the horizon
            self.push_instance(
                [x + lean_x * 0.5, ground + TRUNK_HEIGHT * 0.5 * scale, z + lean_z * 0.5],
                [1.9 * scale, TRUNK_HEIGHT * scale, 1.9 * scale],
                bark,
                spin,
                0.0,
                Shape::Cuboid,
            );
            self.push_instance(
                [x + lean_x, crown_y + 1.0 * scale, z + lean_z],
                [11.0 * scale, 3.4 * scale, 11.0 * scale],
                frond,
                spin,
                0.0,
                Shape::Cone,
            );
            return;
        }

        // Trunk: stacked blocks that narrow and step sideways as they rise, so
        // the silhouette is notched rather than a smooth pole.
        for segment in 0..trunk_segments {
            let along = (segment as f32 + 0.5) / trunk_segments as f32;
            let girth = (2.0 - along * 0.8) * scale;
            let notch = World::scenery_jitter(spin, scale, segment as f32) * 0.35 * scale;
            self.push_instance(
                [
                    x + lean_x * along + notch,
                    ground + TRUNK_HEIGHT * scale * along,
                    z + lean_z * along + notch,
                ],
                [girth, TRUNK_HEIGHT * scale / trunk_segments as f32 + 0.6, girth],
                [
                    bark[0] + notch * 0.06,
                    bark[1] + notch * 0.05,
                    bark[2] + notch * 0.03,
                ],
                spin + along * 0.4,
                0.0,
                Shape::Cuboid,
            );
        }

        let crown_x = x + lean_x;
        let crown_z = z + lean_z;
        // A crown of four flat blades reads as a plate at any distance; the
        // droop is what makes it a palm, so it survives into the reduced tier.
        let fronds = if detail == SceneryDetail::Full {
            PALM_FRONDS
        } else {
            5
        };
        for blade in 0..fronds {
            let angle = spin + blade as f32 * core::f32::consts::TAU / fronds as f32;
            let droop = World::scenery_jitter(spin, scale, blade as f32 + 1.5);
            let length = (8.4 + droop * 1.8) * scale;
            let dir_x = sin(angle);
            let dir_z = cos(angle);
            // inner half rides out roughly level
            self.push_instance(
                [
                    crown_x + dir_x * length * 0.5,
                    crown_y + 1.6 * scale,
                    crown_z + dir_z * length * 0.5,
                ],
                [2.1 * scale, 0.7 * scale, length],
                frond,
                angle,
                0.0,
                Shape::Cuboid,
            );
            {
                // outer half hangs, which is what makes it read as a palm
                self.push_instance(
                    [
                        crown_x + dir_x * length * 1.15,
                        crown_y + 1.6 * scale - PALM_FROND_DROOP * scale,
                        crown_z + dir_z * length * 1.15,
                    ],
                    [1.5 * scale, 0.6 * scale, length * 0.8],
                    [frond[0] * 0.82, frond[1] * 0.82, frond[2] * 0.82],
                    angle,
                    0.0,
                    Shape::Cuboid,
                );
            }
        }

        if detail == SceneryDetail::Full {
            for nut in 0..3 {
                let angle = spin * 2.0 + nut as f32 * 2.1;
                self.push_instance(
                    [
                        crown_x + sin(angle) * 1.9 * scale,
                        crown_y + 0.2 * scale,
                        crown_z + cos(angle) * 1.9 * scale,
                    ],
                    [1.5 * scale, 1.5 * scale, 1.5 * scale],
                    [0.30, 0.22, 0.12],
                    angle,
                    0.0,
                    Shape::Sphere,
                );
            }
        }

        if burn > 0.0 {
            // flame bodies sit in the crown; the smoke comes from update_fires
            let flare = 0.7 + sin(self.session.elapsed * 9.0 + spin * 4.0) * 0.3;
            self.push_instance(
                [crown_x, crown_y + 3.0 * scale, crown_z],
                [7.0 * scale * flare, 9.0 * scale * flare, 7.0 * scale * flare],
                [1.0, 0.55, 0.16],
                spin + self.session.elapsed * 2.0,
                1.0,
                Shape::Cone,
            );
            self.push_instance(
                [crown_x, crown_y + 7.0 * scale, crown_z],
                [3.4 * scale * flare, 5.0 * scale * flare, 3.4 * scale * flare],
                [1.0, 0.86, 0.42],
                spin - self.session.elapsed * 2.6,
                1.0,
                Shape::Cone,
            );
        }
    }

    /// What a palm leaves behind once the fire has finished with it.
    pub(super) fn draw_burnt_stump(&mut self, x: f32, ground: f32, z: f32, scale: f32, spin: f32) {
        self.push_instance(
            [x, ground + 1.6 * scale, z],
            [2.4 * scale, 4.0 * scale, 2.4 * scale],
            [0.13, 0.10, 0.09],
            spin,
            0.0,
            Shape::Cuboid,
        );
    }

    /// Overlapping squashed ellipsoids with one angular chip, sizes and tints
    /// driven off the item's own stored spin so no two boulders match.
    pub(super) fn draw_boulder(
        &mut self,
        x: f32,
        ground: f32,
        z: f32,
        scale: f32,
        spin: f32,
        detail: SceneryDetail,
    ) {
        // One jitter shifts the value, a second swings the hue between warm
        // sandstone and cold slate, so a boulder field is not one grey.
        let value = World::scenery_jitter(spin, scale, 2.0);
        let hue = World::scenery_jitter(spin, scale, 6.0);
        let stone = [
            0.40 + value * 0.13 + hue * 0.06,
            0.38 + value * 0.12,
            0.36 + value * 0.10 - hue * 0.05,
        ];
        let lumps = match detail {
            SceneryDetail::Full => ROCK_LUMPS,
            SceneryDetail::Reduced => 2,
            SceneryDetail::Distant => 1,
        };
        for lump in 0..lumps {
            let salt = lump as f32 + 0.5;
            let offset_angle = spin + lump as f32 * 2.4;
            let offset = if lump == 0 {
                0.0
            } else {
                (1.4 + abs(World::scenery_jitter(spin, scale, salt)) * 1.8) * scale
            };
            // each lump squashes on a different axis, so the mass looks worn
            let width = (3.6 + World::scenery_jitter(spin, scale, salt + 0.3) * 1.5) * scale;
            let height = (2.6 + World::scenery_jitter(spin, scale, salt + 0.7) * 1.1) * scale;
            let depth = (3.4 + World::scenery_jitter(spin, scale, salt + 1.1) * 1.4) * scale;
            let shrink = if lump == 0 { 1.0 } else { 0.68 };
            let shade = World::scenery_jitter(spin, scale, salt + 2.5) * 0.05;
            self.push_instance(
                [
                    x + sin(offset_angle) * offset,
                    // sunk, so it sits in the ground rather than on it
                    ground + height * shrink * 0.32,
                    z + cos(offset_angle) * offset,
                ],
                [width * shrink, height * shrink, depth * shrink],
                [stone[0] + shade, stone[1] + shade, stone[2] + shade],
                offset_angle,
                0.0,
                Shape::Sphere,
            );
        }
        if detail == SceneryDetail::Full {
            // one flat slab keeps some hard edges among the round mass
            let slab = World::scenery_jitter(spin, scale, 4.0);
            self.push_instance(
                [x + slab * 1.2 * scale, ground + 0.5 * scale, z - slab * 1.0 * scale],
                [(3.0 + slab) * scale, 1.4 * scale, (2.4 - slab * 0.6) * scale],
                [stone[0] - 0.04, stone[1] - 0.04, stone[2] - 0.03],
                spin * 1.7,
                0.0,
                Shape::Cuboid,
            );
        }
    }

}
