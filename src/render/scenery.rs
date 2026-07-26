//! Palms, boulders and everything else scattered across the terrain.
//!
//! Placement is clumped (see `terrain::scatter_scenery`); this file is only
//! concerned with turning one scenery slot into geometry.
//!
//! Nothing in here is allowed to come out the same twice. Every proportion,
//! angle, count and tint is hashed off the item's own slot index, because a
//! grove where every tree carries the same seven fronds at the same seven
//! bearings reads as wallpaper however well one tree is built.

use super::*;

/// One scenery slot resolved into everything the model routines need.
///
/// The slot index is the important field: it is the seed every proportion is
/// hashed off. It has to be the slot rather than an RNG draw, because a draw
/// would reshuffle the whole island sixty times a second, and rather than the
/// item's stored spin because that is one number shared by everything a model
/// wants to vary independently.
pub(super) struct SceneryItem {
    slot: usize,
    x: f32,
    ground: f32,
    z: f32,
    scale: f32,
    /// The item's stored heading, used only as a starting bearing now.
    spin: f32,
    /// Seconds of fire left; zero for anything not alight.
    burn: f32,
    detail: SceneryDetail,
}

impl SceneryItem {
    /// Stable pseudo-random in -1..1 for this item and this `salt`, from the
    /// same integer mixer `CreatureBody::vary` uses.
    ///
    /// Not a `sin`-fract: at f32 precision `fract(sin(x) * 43758.5)` lands on
    /// a couple of hundred distinct values and correlates strongly between
    /// neighbouring slots, and neighbouring slots are exactly what a clump of
    /// palms is made of. That is most of why a grove used to look like one
    /// tree stamped eight times.
    fn vary(&self, salt: f32) -> f32 {
        let mut hash = (self.slot as u32)
            .wrapping_mul(747_796_405)
            .wrapping_add((salt * 97.0) as i32 as u32);
        hash ^= hash >> 15;
        hash = hash.wrapping_mul(2_246_822_519);
        hash ^= hash >> 13;
        hash = hash.wrapping_mul(3_266_489_917);
        hash ^= hash >> 16;
        (hash & 0xffff) as f32 / 32768.0 - 1.0
    }

    /// Stable pseudo-random in 0..1.
    fn vary_unit(&self, salt: f32) -> f32 {
        self.vary(salt) * 0.5 + 0.5
    }

    /// True for roughly `odds` of items, fixed per item. For features that are
    /// there or not there rather than scaled: nuts, a skirt of dead fronds, a
    /// fallen log beside the stump.
    fn quirk(&self, salt: f32, odds: f32) -> bool {
        self.vary_unit(salt) < odds
    }

    /// `base` nudged in value and in hue, so a stand of palms is not one green
    /// and a scree slope is not one grey.
    fn tint(&self, base: [f32; 3], salt: f32, amount: f32) -> [f32; 3] {
        let shift = self.vary(salt) * amount;
        let warm = self.vary(salt + 1.7) * amount * 0.7;
        [
            clamp(base[0] + shift + warm, 0.0, 1.0),
            clamp(base[1] + shift, 0.0, 1.0),
            clamp(base[2] + shift - warm, 0.0, 1.0),
        ]
    }

    /// True at the near tier, the only one where trim is worth an instance.
    fn near(&self) -> bool {
        self.detail == SceneryDetail::Full
    }
}

impl World {
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
            let item = SceneryItem {
                slot: index,
                x,
                ground,
                z,
                scale: self.scenery.scale[index],
                spin: self.scenery.rotation[index],
                burn: self.scenery.burn_remaining[index],
                detail,
            };
            // only the near tier is shadowed: past that the blob is a couple
            // of pixels and costs an instance each
            if detail == SceneryDetail::Full && self.scenery.standing[index] {
                // the blob tracks how wide this particular item grew, or a
                // spindly palm sits on the same disc as a giant
                let radius = match self.scenery.kind[index] {
                    SceneryKind::Palm => (5.0 + item.vary_unit(200.0) * 3.6) * item.scale,
                    SceneryKind::Rock => (3.1 + item.vary_unit(201.0) * 2.6) * item.scale,
                };
                self.push_shadow(x, z, ground, radius, 0.34);
            }
            match self.scenery.kind[index] {
                SceneryKind::Palm => {
                    if self.scenery.standing[index] {
                        self.draw_palm(&item);
                    } else {
                        self.draw_burnt_stump(&item);
                    }
                }
                SceneryKind::Rock => self.draw_boulder(&item),
            }
        }
        true
    }

    /// A curving, tapering trunk under a crown where no two fronds match.
    ///
    /// Height, girth, which way the shaft bends, how many fronds there are,
    /// where each one sits around the crown, how far it lifts or hangs, and
    /// whether the tree carries nuts or a skirt of dead fronds at all are all
    /// hashed off the slot. Nothing is shared with the tree beside it but the
    /// species.
    pub(super) fn draw_palm(&mut self, item: &SceneryItem) {
        /// Trunk height and base girth before this tree's own build is applied.
        const TRUNK_HEIGHT: f32 = 17.0;
        const TRUNK_GIRTH: f32 = 2.1;
        /// Share of the base girth lost by the time the shaft reaches the crown.
        const TRUNK_TAPER: f32 = 0.44;

        let scale = item.scale;
        // Overall build. Tall palms are relatively slimmer, which is why the
        // girth is divided through by the stature rather than drawn free.
        let stature = 0.70 + item.vary_unit(1.0) * 0.72;
        let height = TRUNK_HEIGHT * scale * stature;
        let girth = TRUNK_GIRTH * scale * (0.80 + item.vary_unit(2.0) * 0.52)
            / (0.86 + stature * 0.18);
        let char_mix = clamp(item.burn * 0.16, 0.0, 0.85);
        let bark = item.tint(
            [
                0.47 - char_mix * 0.38,
                0.36 - char_mix * 0.31,
                0.22 - char_mix * 0.18,
            ],
            3.0,
            0.055,
        );
        let leaf = item.tint(
            [
                0.24 - char_mix * 0.18,
                0.47 - char_mix * 0.40,
                0.19 - char_mix * 0.15,
            ],
            5.5,
            0.065,
        );

        // The shaft sweeps out one way and bows across it, so it is a curve in
        // space rather than a stick tipped at a fixed angle. Both components
        // vanish at the roots, so the tree still meets the ground upright.
        let sweep_bearing = item.spin;
        let sweep = (0.6 + item.vary_unit(7.0) * 4.2) * scale;
        let bow_bearing = sweep_bearing + 1.3 + item.vary(8.0) * 0.9;
        let bow = item.vary(9.0) * 2.7 * scale;
        let trunk_at = |along: f32| -> [f32; 3] {
            let out = sweep * along * along;
            let across = bow * sin(along * core::f32::consts::PI);
            [
                item.x + sin(sweep_bearing) * out + sin(bow_bearing) * across,
                item.ground + height * along,
                item.z + cos(sweep_bearing) * out + cos(bow_bearing) * across,
            ]
        };
        // Orientation of the shaft where it crosses `along`, read off the curve
        // itself. Without it every segment stands to attention inside a trunk
        // that is supposed to be leaning, which is the whole seam problem.
        let trunk_lean = |along: f32| -> Rotation {
            const STEP: f32 = 0.09;
            let below = trunk_at(max(along - STEP, 0.0));
            let above = trunk_at(min(along + STEP, 1.0));
            let run_x = above[0] - below[0];
            let run_z = above[2] - below[2];
            Rotation::new(
                atan2(run_x, run_z),
                atan2(sqrt(length_sq2(run_x, run_z)), above[1] - below[1]),
                0.0,
            )
        };

        let crown = trunk_at(1.0);
        if item.detail == SceneryDetail::Distant {
            // one post and one canopy: enough to read as a palm on the horizon
            self.push_oriented(
                trunk_at(0.5),
                [girth * 0.86, height * 1.04, girth * 0.86],
                bark,
                trunk_lean(0.5),
                0.0,
                Shape::Cylinder,
            );
            let span = (8.4 + item.vary_unit(10.0) * 4.4) * scale;
            self.push_instance(
                [crown[0], crown[1] + 1.0 * scale, crown[2]],
                [span, (2.6 + item.vary_unit(11.0) * 1.8) * scale, span * 0.92],
                leaf,
                item.spin,
                0.0,
                Shape::Cone,
            );
            return;
        }

        // Root flare. A shaft that meets the ground on a hard rim is the tell
        // of a stack of cylinders; palms stand on a splay of root.
        if item.near() {
            let flare = (2.2 + item.vary_unit(12.0) * 1.9) * scale;
            self.push_oriented(
                [item.x, item.ground + flare * 0.24, item.z],
                [girth * 2.4, flare, girth * 2.2],
                [bark[0] * 0.80, bark[1] * 0.78, bark[2] * 0.76],
                Rotation::new(
                    item.spin * 1.9,
                    item.vary(13.0) * 0.11,
                    item.vary(14.0) * 0.11,
                ),
                0.0,
                Shape::Frustum,
            );
        }

        // Segments run along the curve, overlap their neighbours by a third,
        // and swell and pinch as they climb rather than tapering cleanly.
        let segments = if item.near() { 5 } else { 3 };
        for segment in 0..segments {
            let along = (segment as f32 + 0.5) / segments as f32;
            let salt = 20.0 + segment as f32 * 3.0;
            let width = girth * (1.0 - TRUNK_TAPER * along) * (1.0 + item.vary(salt) * 0.12);
            let shade = item.vary(salt + 1.0) * 0.04;
            self.push_oriented(
                trunk_at(along),
                [
                    width,
                    height / segments as f32 * 1.38,
                    // never perfectly round: an oval shaft catches the sun
                    // along one flank the way real bark does
                    width * (0.90 + item.vary_unit(salt + 2.0) * 0.18),
                ],
                [bark[0] + shade, bark[1] + shade, bark[2] + shade * 0.6],
                trunk_lean(along),
                0.0,
                Shape::Cylinder,
            );
        }

        if item.near() {
            // Scars where old fronds tore away. They girdle the shaft, which
            // both is what a palm looks like and stops the trunk reading as one
            // smooth extrusion.
            for ring in 0..3 {
                let salt = 36.0 + ring as f32 * 2.0;
                let along = 0.22 + ring as f32 * 0.24 + item.vary(salt) * 0.07;
                let width = girth
                    * (1.0 - TRUNK_TAPER * along)
                    * (1.12 + item.vary_unit(salt + 1.0) * 0.14);
                self.push_oriented(
                    trunk_at(along),
                    [
                        width,
                        (0.36 + item.vary_unit(salt + 0.8) * 0.44) * scale,
                        width * 0.96,
                    ],
                    [bark[0] * 0.70, bark[1] * 0.70, bark[2] * 0.68],
                    trunk_lean(along),
                    0.0,
                    Shape::Cylinder,
                );
            }
        }

        let top_girth = girth * (1.0 - TRUNK_TAPER);
        // The crown has a lean of its own, and not always the trunk's: a palm
        // grows toward the light and then gets pushed about by the wind.
        let crown_bearing = sweep_bearing + item.vary(44.0) * 3.2;
        let crown_tilt = 0.08 + item.vary_unit(45.0) * 0.36;
        let boss = [
            crown[0] + sin(crown_bearing) * crown_tilt * top_girth,
            crown[1] + top_girth * 0.18,
            crown[2] + cos(crown_bearing) * crown_tilt * top_girth,
        ];
        // The fibrous knot the fronds spring from. A dozen roots meeting at a
        // bare point is the other half of why a crown looks assembled.
        self.push_oriented(
            boss,
            [top_girth * 2.1, top_girth * 1.7, top_girth * 1.9],
            [
                (bark[0] + leaf[0]) * 0.5,
                (bark[1] + leaf[1]) * 0.5,
                (bark[2] + leaf[2]) * 0.5,
            ],
            Rotation::new(crown_bearing, crown_tilt, item.vary(46.0) * 0.35),
            0.0,
            Shape::Boulder,
        );

        // Even spacing is what reads as a rosette, so each bearing is jittered
        // and one or two gaps are opened. The fronds then bunch on one side of
        // the crown and show sky on the other, which is what a palm does.
        let fronds = if item.near() {
            PALM_FRONDS - 2 + (item.vary_unit(47.0) * 4.99) as i32
        } else {
            5
        };
        let spacing = core::f32::consts::TAU / fronds as f32;
        let first_gap = (item.vary_unit(48.0) * fronds as f32) as i32;
        let second_gap = (item.vary_unit(49.0) * fronds as f32) as i32;
        let first_width = 0.16 + item.vary_unit(50.0) * 0.30;
        let second_width = 0.12 + item.vary_unit(51.0) * 0.26;
        for blade in 0..fronds {
            let salt = 60.0 + blade as f32 * 6.0;
            let mut bearing = item.spin + spacing * (blade as f32 + item.vary(salt) * 0.30);
            if blade >= first_gap {
                bearing += spacing * first_width;
            }
            if blade >= second_gap {
                bearing += spacing * second_width;
            }
            // Some fronds are held out above the horizontal and some hang
            // nearly back down the trunk; the crown's own lean rides on top,
            // pressing the downhill side lower and twisting the flanks.
            let offset = bearing - crown_bearing;
            // a squat tree hangs its fronds less steeply, or the tips of the
            // longest ones sweep the sand
            let pitch = -0.40
                + item.vary_unit(salt + 1.0) * (0.78 + stature * 0.32)
                + crown_tilt * cos(offset);
            let roll = item.vary(salt + 2.0) * 0.55 + crown_tilt * sin(offset) * 0.8;
            let reach = (7.0 + item.vary_unit(salt + 3.0) * 5.2) * scale * (0.76 + stature * 0.28);
            // the mesh carries the droop; this only says how heavy it is
            let droop = (PALM_FROND_DROOP + item.vary(salt + 4.0) * 1.5) * scale * 1.45;
            let width = (1.7 + item.vary_unit(salt + 5.0) * 1.2) * scale;
            let age = item.vary_unit(salt + 2.5);
            self.push_oriented(
                frond_centre(boss, bearing, pitch, reach, droop),
                [width, droop, reach],
                [leaf[0] + age * 0.15, leaf[1] + age * 0.04, leaf[2] - age * 0.04],
                Rotation::new(bearing, pitch, roll),
                0.0,
                Shape::Frond,
            );
        }

        if item.near() {
            // The spear: next year's frond, still rolled up and standing out of
            // the middle. It is the one thing that breaks the crown's dome.
            let spear = (3.2 + item.vary_unit(52.0) * 3.6) * scale;
            self.push_oriented(
                [
                    boss[0] + sin(crown_bearing) * spear * 0.2,
                    boss[1] + spear * 0.4,
                    boss[2] + cos(crown_bearing) * spear * 0.2,
                ],
                [top_girth * 0.72, spear, top_girth * 0.6],
                [leaf[0] + 0.10, leaf[1] + 0.13, leaf[2] + 0.03],
                Rotation::new(crown_bearing, crown_tilt * 1.7, 0.0),
                0.0,
                Shape::Cone,
            );
        }

        // A skirt of dead fronds, on most trees but not all: brown, hanging far
        // steeper than the live ones, and half of them snapped off short.
        if item.near() && item.quirk(53.0, 0.74) {
            let dead = if item.quirk(54.0, 0.45) { 2 } else { 1 };
            for skirt in 0..dead {
                let salt = 120.0 + skirt as f32 * 7.0;
                let bearing = item.spin + item.vary(salt) * 3.14;
                let pitch = 0.86 + item.vary_unit(salt + 1.0) * 0.68;
                let reach = (5.4 + item.vary_unit(salt + 2.0) * 3.6)
                    * scale
                    * if item.quirk(salt + 3.0, 0.4) { 0.46 } else { 1.0 };
                let droop = (1.9 + item.vary_unit(salt + 4.0) * 2.2) * scale;
                let dry = item.tint([0.33, 0.25, 0.13], salt + 5.0, 0.05);
                self.push_oriented(
                    frond_centre(
                        [boss[0], boss[1] - top_girth * 0.5, boss[2]],
                        bearing,
                        pitch,
                        reach,
                        droop,
                    ),
                    [droop * 0.5, droop, reach],
                    dry,
                    Rotation::new(bearing, pitch, item.vary(salt + 6.0) * 0.9),
                    0.0,
                    Shape::Frond,
                );
            }
        }

        // Nuts, on about half of the trees and never spread evenly: they hang
        // in one cluster tucked under the crown.
        if item.near() && item.burn <= 0.0 && item.quirk(55.0, 0.55) {
            let nuts = 2 + (item.vary_unit(56.0) * 2.0) as i32;
            let cluster = item.spin + item.vary(57.0) * 3.14;
            for nut in 0..nuts {
                let salt = 140.0 + nut as f32 * 4.0;
                let angle = cluster + item.vary(salt) * 0.85;
                let out = (0.7 + item.vary_unit(salt + 1.0) * 1.1) * scale;
                let size = (1.05 + item.vary_unit(salt + 2.0) * 0.7) * scale;
                self.push_oriented(
                    [
                        boss[0] + sin(angle) * out,
                        boss[1] - top_girth * (0.7 + item.vary_unit(salt + 3.0) * 0.7),
                        boss[2] + cos(angle) * out,
                    ],
                    [size, size * (0.82 + item.vary_unit(salt + 2.5) * 0.3), size * 0.92],
                    item.tint([0.30, 0.22, 0.12], salt + 1.5, 0.05),
                    Rotation::new(angle * 1.7, item.vary(salt + 3.5) * 1.2, item.vary(salt) * 1.2),
                    0.0,
                    Shape::Boulder,
                );
            }
        }

        if item.burn > 0.0 {
            // flame bodies sit in the crown; the smoke comes from update_fires
            let flare = 0.7 + sin(self.session.elapsed * 9.0 + item.spin * 4.0) * 0.3;
            let fire = (5.6 + item.vary_unit(160.0) * 2.6) * scale * flare;
            self.push_instance(
                [boss[0], boss[1] + 3.0 * scale, boss[2]],
                [fire, fire * 1.3, fire],
                [1.0, 0.55, 0.16],
                item.spin + self.session.elapsed * 2.0,
                1.0,
                Shape::Cone,
            );
            if item.near() {
                self.push_instance(
                    [boss[0], boss[1] + 7.0 * scale, boss[2]],
                    [fire * 0.5, fire * 0.72, fire * 0.5],
                    [1.0, 0.86, 0.42],
                    item.spin - self.session.elapsed * 2.6,
                    1.0,
                    Shape::Cone,
                );
            }
        }
    }

    /// What a palm leaves behind once the fire has finished with it: a charred
    /// shaft snapped off short and leaning, the splinters of the break still
    /// standing on it, and ash heaped round the roots.
    pub(super) fn draw_burnt_stump(&mut self, item: &SceneryItem) {
        let scale = item.scale;
        let char_colour = item.tint([0.15, 0.12, 0.10], 80.0, 0.045);
        let height = (2.6 + item.vary_unit(81.0) * 3.8) * scale;
        let girth = (1.7 + item.vary_unit(82.0) * 1.1) * scale;
        // burnt through on one flank, so it went over that way
        let bearing = item.spin + item.vary(83.0) * 0.9;
        let lean = 0.10 + item.vary_unit(84.0) * 0.36;
        let axis = [
            sin(bearing) * sin(lean),
            cos(lean),
            cos(bearing) * sin(lean),
        ];
        let base = [item.x, item.ground - 0.4 * scale, item.z];
        self.push_oriented(
            [
                base[0] + axis[0] * height * 0.5,
                base[1] + axis[1] * height * 0.5,
                base[2] + axis[2] * height * 0.5,
            ],
            [girth, height, girth * 0.88],
            char_colour,
            Rotation::new(bearing, lean, 0.0),
            0.0,
            Shape::Frustum,
        );
        // ash and root swell, which also hides where the shaft enters the soil
        self.push_oriented(
            [item.x, item.ground + girth * 0.1, item.z],
            [girth * 2.5, girth * 0.85, girth * 2.2],
            [char_colour[0] * 1.5, char_colour[1] * 1.45, char_colour[2] * 1.4],
            Rotation::new(item.spin * 2.1, item.vary(85.0) * 0.14, item.vary(86.0) * 0.14),
            0.0,
            Shape::Boulder,
        );
        if !item.near() {
            return;
        }

        // The break is a crown of shards. A stump cut off flat is a fence post.
        let top = [
            base[0] + axis[0] * height,
            base[1] + axis[1] * height * 0.94,
            base[2] + axis[2] * height,
        ];
        let shards = 2 + (item.vary_unit(87.0) * 2.0) as i32;
        for shard in 0..shards {
            let salt = 88.0 + shard as f32 * 3.0;
            let angle = bearing + shard as f32 * 2.3 + item.vary(salt) * 1.1;
            let tilt = 0.16 + item.vary_unit(salt + 1.0) * 0.55;
            let length = (1.3 + item.vary_unit(salt + 2.0) * 2.8) * scale;
            let out = [sin(angle) * sin(tilt), cos(tilt), cos(angle) * sin(tilt)];
            self.push_oriented(
                [
                    top[0] + out[0] * length * 0.32,
                    top[1] + out[1] * length * 0.32,
                    top[2] + out[2] * length * 0.32,
                ],
                [girth * 0.34, length, girth * 0.3],
                [char_colour[0] * 0.8, char_colour[1] * 0.8, char_colour[2] * 0.8],
                Rotation::new(angle, tilt, 0.0),
                0.0,
                Shape::Cone,
            );
        }
        // and, on some, the length of trunk that came down with it
        if item.quirk(98.0, 0.55) {
            let log = (3.6 + item.vary_unit(99.0) * 4.0) * scale;
            let fell = bearing + 1.1 + item.vary(100.0) * 1.4;
            let away = (1.6 + item.vary_unit(101.0) * 2.2) * scale;
            self.push_oriented(
                [
                    item.x + sin(fell) * away,
                    item.ground + girth * 0.34,
                    item.z + cos(fell) * away,
                ],
                [girth * 0.86, log, girth * 0.8],
                [char_colour[0] * 1.2, char_colour[1] * 1.15, char_colour[2] * 1.1],
                // laid over onto its side, tipped a little out of true
                Rotation::new(
                    fell,
                    core::f32::consts::FRAC_PI_2 + item.vary(102.0) * 0.16,
                    item.vary(103.0) * 0.4,
                ),
                0.0,
                Shape::Cylinder,
            );
        }
    }

    /// Weathered stone: irregular lumps at real angles with chips shed at the
    /// foot, and three builds rather than one egg — a slab lying half buried, a
    /// tall rock split by a fissure, or a heap of rubble.
    pub(super) fn draw_boulder(&mut self, item: &SceneryItem) {
        let scale = item.scale;
        // value and hue move on separate hashes, so a scree slope is not one
        // grey and the warm rocks and the cold ones sit side by side
        let stone = item.tint([0.42, 0.40, 0.37], 60.0, 0.155);
        // `spread` sets how far the lumps sit apart, `squat` how flat they are,
        // `lift` how much of one stands proud of the ground, `cant` how hard
        // they are tipped out of plumb.
        let form = item.vary_unit(62.0);
        let (mut lumps, spread, squat, lift, cant) = if form < 0.30 {
            (2, 1.25, 0.34, 0.14, 0.34) // a slab, half buried and canted over
        } else if form < 0.60 {
            (2, 0.85, 1.34, 0.42, 0.30) // a tall rock, split up the middle
        } else {
            (ROCK_LUMPS + 1, 1.15, 0.74, 0.30, 0.12) // a heap of rubble
        };
        lumps = match item.detail {
            SceneryDetail::Full => lumps,
            SceneryDetail::Reduced => {
                if lumps > 3 {
                    3
                } else {
                    lumps
                }
            }
            SceneryDetail::Distant => 1,
        };
        let main_width = (3.4 + item.vary(63.0) * 1.3) * scale;

        for lump in 0..lumps {
            let salt = 64.0 + lump as f32 * 8.0;
            let shrink = if lump == 0 {
                1.0
            } else {
                0.46 + item.vary_unit(salt) * 0.44
            };
            let width = main_width * shrink * (0.9 + item.vary_unit(salt + 1.0) * 0.32);
            let tall = main_width * squat * shrink * (0.74 + item.vary_unit(salt + 3.0) * 0.56);
            let bearing = item.spin + lump as f32 * 2.3 + item.vary(salt + 4.0) * 1.2;
            // lumps overlap by roughly a quarter, so the mass has no seams
            let out = if lump == 0 {
                0.0
            } else {
                (width + main_width) * 0.5 * spread * (0.52 + item.vary_unit(salt + 5.0) * 0.42)
            };
            let shade = item.vary(salt + 6.0) * 0.06;
            self.push_oriented(
                [
                    item.x + sin(bearing) * out,
                    // sunk, so it sits in the ground rather than on it
                    item.ground + tall * lift,
                    item.z + cos(bearing) * out,
                ],
                [
                    width,
                    tall,
                    width * (0.7 + item.vary_unit(salt + 2.0) * 0.55),
                ],
                [stone[0] + shade, stone[1] + shade, stone[2] + shade * 0.8],
                // Pitch and roll are the whole point: flat-shaded facets only
                // read as stone if neighbouring lumps present different ones to
                // the sun. The pitch leans each lump out along its own bearing,
                // which is what opens the fissure on the split rocks.
                Rotation::new(
                    bearing * 1.6,
                    item.vary(salt + 7.0) * 0.3 + cant * if lump == 0 { 0.4 } else { 1.0 },
                    item.vary(salt + 0.5) * (0.24 + cant),
                ),
                0.0,
                Shape::Boulder,
            );
        }

        if item.near() {
            // Splinters shed at the foot. Nothing in nature stops on a clean
            // line where the rock meets the ground.
            for chip in 0..2 {
                let salt = 90.0 + chip as f32 * 5.0;
                let bearing = item.spin * 1.4 + chip as f32 * 2.9 + item.vary(salt) * 1.7;
                let out = main_width * (0.66 + item.vary_unit(salt + 1.0) * 0.7);
                let size = main_width * (0.16 + item.vary_unit(salt + 2.0) * 0.2);
                let shade = item.vary(salt + 3.0) * 0.05 - 0.02;
                self.push_oriented(
                    [
                        item.x + sin(bearing) * out,
                        item.ground + size * 0.22,
                        item.z + cos(bearing) * out,
                    ],
                    [size * 1.5, size * 0.8, size * 1.2],
                    [stone[0] + shade, stone[1] + shade, stone[2] + shade],
                    Rotation::new(
                        bearing * 2.2,
                        item.vary(salt + 4.0) * 0.5,
                        item.vary(salt + 2.5) * 0.6,
                    ),
                    0.0,
                    // one rounded, one a hard flake, so the debris is not all
                    // the same pebble
                    if chip == 0 { Shape::Boulder } else { Shape::Wedge },
                );
            }
        }
    }
}

/// Where to place a frond so its root lands on `root` and its tip falls away
/// along `bearing`.
///
/// The mesh runs root at -Z to tip at +Z with the root sitting a little above
/// its own centre line, and the instance is positioned by its centre, so both
/// have to be walked back out. The root is then pushed back into the crown,
/// because a frond that starts exactly where the boss ends shows the join.
fn frond_centre(root: [f32; 3], bearing: f32, pitch: f32, reach: f32, droop: f32) -> [f32; 3] {
    /// How far the mesh's root sits above the instance centre, as a share of
    /// the instance's height.
    const ROOT_RISE: f32 = 0.22;
    /// How far of its own length the frond is buried in the crown.
    const ROOT_INSET: f32 = 0.12;
    let along = reach * 0.5 * (1.0 - ROOT_INSET);
    let flat = cos(pitch);
    [
        root[0] + sin(bearing) * flat * along,
        root[1] - droop * ROOT_RISE * flat - sin(pitch) * along,
        root[2] + cos(bearing) * flat * along,
    ]
}
