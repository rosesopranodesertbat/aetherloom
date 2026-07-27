//! Carpets, mana orbs, projectiles and the fireball.

use super::*;

/// A carpet is woven in this many tiles along each axis. Each tile carries its
/// own slice of the travelling ripple, which is what stops it reading as the
/// flat plank it used to be.
const CARPET_TILES_ACROSS: i32 = 3;
const CARPET_TILES_ALONG: i32 = 4;
const CARPET_WIDTH: f32 = 16.0;
const CARPET_LENGTH: f32 = 20.0;
/// Amplitude and rate of the ripple running from the nose to the tail.
const CARPET_RIPPLE: f32 = 1.05;
const CARPET_RIPPLE_RATE: f32 = 3.4;
/// Tassels per fringed edge.
const CARPET_TASSELS: i32 = 5;
/// Orbs closer than this get their facets and motes; past it, one blob.
const ORB_DETAIL_RANGE: f32 = 320.0;
/// Motes circling a nearby orb.
const ORB_MOTES: i32 = 3;

impl World {
    /// Isolated visual-QA models for carpets, orbs, and projectiles.
    ///
    /// `kind` is local to the preview ABI: player/rival carpet, the three orb
    /// allegiances, then firebolt, meteor, creature bolt, and dragon fire.
    pub(super) fn draw_preview_effect(
        &mut self,
        kind: i32,
        variant: i32,
        origin: [f32; 3],
    ) {
        let angle = variant as f32 * core::f32::consts::TAU / PREVIEW_VARIANT_COUNT as f32;
        match kind {
            0 | 1 => {
                let wizard = if kind == 0 { PLAYER } else { 1 };
                let old_pose = (
                    self.wizards.yaw[wizard],
                    self.wizards.pitch[wizard],
                    self.wizards.roll[wizard],
                    self.session.elapsed,
                );
                self.wizards.yaw[wizard] = angle;
                self.wizards.pitch[wizard] = sin(angle) * 0.18;
                self.wizards.roll[wizard] = cos(angle) * 0.16;
                self.session.elapsed = variant as f32 * 0.12 + 0.3;
                if kind == 0 {
                    self.draw_carpet(
                        wizard,
                        origin,
                        [0.15, 0.21, 0.54],
                        [0.86, 0.63, 0.22],
                        [0.70, 0.19, 0.16],
                    );
                    self.draw_rider(
                        wizard,
                        origin,
                        [0.20, 0.26, 0.58],
                        [0.90, 0.78, 0.34],
                    );
                } else {
                    self.draw_carpet(
                        wizard,
                        origin,
                        [0.62, 0.13, 0.12],
                        [0.20, 0.05, 0.06],
                        [0.86, 0.66, 0.22],
                    );
                    self.draw_rider(
                        wizard,
                        origin,
                        [0.48, 0.09, 0.10],
                        [0.86, 0.72, 0.30],
                    );
                }
                self.wizards.yaw[wizard] = old_pose.0;
                self.wizards.pitch[wizard] = old_pose.1;
                self.wizards.roll[wizard] = old_pose.2;
                self.session.elapsed = old_pose.3;
            }
            2..=4 => {
                let tint = match kind {
                    2 => [1.00, 0.78, 0.16],
                    3 => [0.72, 0.92, 1.00],
                    _ => [1.00, 0.36, 0.30],
                };
                self.draw_orb_model(
                    origin,
                    4.8 + variant as f32 * 0.16,
                    variant as f32 * 0.67 + 0.2,
                    tint,
                    true,
                );
            }
            5..=8 => {
                let projectile = match kind {
                    5 => ProjectileKind::Firebolt,
                    6 => ProjectileKind::Meteor,
                    7 => ProjectileKind::CreatureBolt,
                    _ => ProjectileKind::DragonFire,
                };
                let old_elapsed = self.session.elapsed;
                self.session.elapsed = variant as f32 * 0.04 + 0.1;
                let velocity = [sin(angle) * 70.0, 12.0, cos(angle) * 70.0];
                let (colour, size) = projectile.head_style();
                if projectile == ProjectileKind::CreatureBolt {
                    self.draw_bolt(origin, velocity, size, colour, variant as usize);
                } else {
                    self.draw_fireball(origin, velocity, size, colour, variant as usize);
                }
                self.session.elapsed = old_elapsed;
            }
            _ => {}
        }
    }

    // ---- carpets, orbs, projectiles ----------------------------------------
    pub(super) fn draw_rival_carpets(&mut self) {
        for wizard in 1..=self.session.rival_count {
            if !self.wizards.alive[wizard] {
                continue;
            }
            let at = [
                self.wizards.pos_x[wizard],
                self.wizards.pos_y[wizard],
                self.wizards.pos_z[wizard],
            ];
            self.push_shadow(at[0], at[2], at[1], 9.0, 0.5);
            // deep red field, black border, brass medallion
            self.draw_carpet(
                wizard,
                at,
                [0.62, 0.13, 0.12],
                [0.20, 0.05, 0.06],
                [0.86, 0.66, 0.22],
            );
            self.draw_rider(wizard, at, [0.48, 0.09, 0.10], [0.86, 0.72, 0.30]);
            if self.wizards.ward_remaining[wizard] > 0.0 {
                self.push_instance(
                    [at[0], at[1] + 2.0, at[2]],
                    [20.0, 20.0, 20.0],
                    [1.0, 0.5, 0.4],
                    0.0,
                    0.5,
                    Shape::Sphere,
                );
            }
            self.push_blip(at[0], at[2], MapBlip::RivalWizard, 1.4);
        }
    }

    /// Emitted as one contiguous run so the renderer can drop the whole thing
    /// in first person — see `carpet_first`/`carpet_last`.
    pub(super) fn draw_player_carpet(&mut self) {
        let at = [
            self.wizards.pos_x[PLAYER],
            self.wizards.pos_y[PLAYER],
            self.wizards.pos_z[PLAYER],
        ];
        // Pushed before the skipped range: in first person the carpet itself is
        // dropped, but its shadow is the player's only read on their own height
        // above the ground, so it has to survive.
        self.push_shadow(at[0], at[2], at[1], 10.0, 0.55);
        self.render.carpet_first = self.render.instance_count as i32;
        // indigo field, saffron border, crimson medallion
        self.draw_carpet(
            PLAYER,
            at,
            [0.15, 0.21, 0.54],
            [0.86, 0.63, 0.22],
            [0.70, 0.19, 0.16],
        );
        // Inside the skipped range, so first person never sees it and chase
        // view gets a wizard rather than an unmanned rug.
        self.draw_rider(PLAYER, at, [0.20, 0.26, 0.58], [0.90, 0.78, 0.34]);
        if self.wizards.ward_remaining[PLAYER] > 0.0 {
            self.push_instance(
                at,
                [24.0, 24.0, 24.0],
                [0.42, 0.72, 1.0],
                0.0,
                0.45,
                Shape::Sphere,
            );
        }
        self.render.carpet_last = self.render.instance_count as i32;
    }

    /// The carpet itself: a woven field of tiles that ripples along its length,
    /// a border of contrasting bands, a medallion, corner motifs and a fringe.
    /// It banks with the wizard's roll — a rug that stays dead level through a
    /// hard turn is the giveaway that it is a board and not a textile.
    fn draw_carpet(
        &mut self,
        wizard: usize,
        at: [f32; 3],
        field: [f32; 3],
        border: [f32; 3],
        medallion: [f32; 3],
    ) {
        let yaw = self.wizards.yaw[wizard];
        let bank = self.wizards.roll[wizard];
        let nose = self.wizards.pitch[wizard] * 0.4;
        let clock = self.session.elapsed * CARPET_RIPPLE_RATE + wizard as f32 * 2.0;
        let deck = at[1] - 2.4;

        // Wave height and slope at a point along the carpet, in its own space.
        let ripple = |along: f32| -> (f32, f32) {
            let angle = clock + along * 3.1;
            (
                sin(angle) * CARPET_RIPPLE,
                cos(angle) * CARPET_RIPPLE * 0.16,
            )
        };
        // Carpet space to world: `side` is to the wizard's right, `along` is
        // forward, both in -1..1.
        let place = |side: f32, along: f32, lift: f32| -> [f32; 3] {
            let right = side * CARPET_WIDTH * 0.5;
            let forward = along * CARPET_LENGTH * 0.5;
            [
                at[0] + sin(yaw) * forward + cos(yaw) * right,
                deck + lift + bank * right * 0.35 - nose * forward * 0.3,
                at[2] + cos(yaw) * forward - sin(yaw) * right,
            ]
        };

        let tile_width = CARPET_WIDTH / CARPET_TILES_ACROSS as f32;
        let tile_length = CARPET_LENGTH / CARPET_TILES_ALONG as f32;
        for row in 0..CARPET_TILES_ALONG {
            let along = (row as f32 + 0.5) / CARPET_TILES_ALONG as f32 * 2.0 - 1.0;
            let (lift, slope) = ripple(along);
            for column in 0..CARPET_TILES_ACROSS {
                let side = (column as f32 + 0.5) / CARPET_TILES_ACROSS as f32 * 2.0 - 1.0;
                // the weave alternates two shades, checkerboard fashion
                let shade = if (row + column) % 2 == 0 { 1.0 } else { 0.84 };
                self.push_oriented(
                    place(side, along, lift),
                    // tiles overlap their neighbours, so no seam shows through
                    [tile_width * 1.3, 0.5, tile_length * 1.35],
                    [field[0] * shade, field[1] * shade, field[2] * shade],
                    Rotation::new(yaw, slope, bank * 0.5),
                    0.03,
                    Shape::Cuboid,
                );
            }
        }

        // Border rails frame all four sides. Colouring the outer tiles instead
        // gives stripes down the length, not a border.
        for rail in 0..2 {
            let side = if rail == 0 { -1.0 } else { 1.0 };
            let (lift, slope) = ripple(0.0);
            self.push_oriented(
                place(side, 0.0, lift + 0.15),
                [1.9, 0.55, CARPET_LENGTH * 1.02],
                border,
                Rotation::new(yaw, slope, bank * 0.5),
                0.04,
                Shape::Cuboid,
            );
        }
        for rail in 0..2 {
            let along = if rail == 0 { -1.0 } else { 1.0 };
            let (lift, slope) = ripple(along);
            self.push_oriented(
                place(0.0, along, lift + 0.15),
                [CARPET_WIDTH * 1.02, 0.55, 1.9],
                border,
                Rotation::new(yaw, slope, bank * 0.5),
                0.04,
                Shape::Cuboid,
            );
        }

        // medallion and the four corner motifs
        let (centre_lift, centre_slope) = ripple(0.0);
        self.push_oriented(
            place(0.0, 0.0, centre_lift + 0.45),
            [5.6, 0.4, 7.4],
            medallion,
            Rotation::new(yaw + core::f32::consts::FRAC_PI_4, centre_slope, bank * 0.5),
            0.06,
            Shape::Cuboid,
        );
        for corner in 0..4 {
            let side = if corner % 2 == 0 { -0.6 } else { 0.6 };
            let along = if corner < 2 { -0.62 } else { 0.62 };
            let (lift, slope) = ripple(along);
            self.push_oriented(
                place(side, along, lift + 0.4),
                [2.4, 0.35, 2.4],
                medallion,
                Rotation::new(yaw + core::f32::consts::FRAC_PI_4, slope, bank * 0.5),
                0.05,
                Shape::Cuboid,
            );
        }

        // fringe: threads hang off the ends and swing out of step with each other
        for tassel in 0..CARPET_TASSELS {
            let side = (tassel as f32 + 0.5) / CARPET_TASSELS as f32 * 2.0 - 1.0;
            for (along, offset) in [(-1.1f32, 0.0f32), (1.1, 2.0)] {
                let (lift, slope) = ripple(along);
                let sway = sin(clock * 1.7 + tassel as f32 + offset) * 0.3;
                self.push_oriented(
                    place(side * 0.9, along, lift - 1.2),
                    [0.55, 2.8, 0.55],
                    [border[0] * 1.3, border[1] * 1.25, border[2] * 1.15],
                    Rotation::new(yaw, slope, bank * 0.5 + sway),
                    0.0,
                    Shape::Cuboid,
                );
            }
        }
    }

    /// A rival wizard, kneeling on their carpet. Their old body was one sphere,
    /// which at duelling range is the thing you are aiming at.
    fn draw_rider(&mut self, wizard: usize, at: [f32; 3], robe: [f32; 3], trim: [f32; 3]) {
        let yaw = self.wizards.yaw[wizard];
        let bank = self.wizards.roll[wizard];
        let sway = sin(self.session.elapsed * 1.9 + wizard as f32) * 0.06;
        let base = at[1] - 1.6;
        let forward = |ahead: f32, up: f32, side: f32| -> [f32; 3] {
            [
                at[0] + sin(yaw) * ahead + cos(yaw) * side,
                base + up,
                at[2] + cos(yaw) * ahead - sin(yaw) * side,
            ]
        };
        // robe: a frustum stood on its head, so it flares at the hem
        self.push_oriented(
            forward(-0.5, 3.6, 0.0),
            [5.8, 8.2, 5.2],
            robe,
            Rotation::new(yaw + core::f32::consts::PI, core::f32::consts::PI, bank * 0.4),
            0.0,
            Shape::Frustum,
        );
        self.push_oriented(
            forward(-0.4, 7.6, 0.0),
            [4.4, 2.2, 4.0],
            [robe[0] * 1.2, robe[1] * 1.2, robe[2] * 1.2],
            Rotation::new(yaw, sway, bank * 0.4),
            0.0,
            Shape::Sphere,
        );
        // cowl, with a dark hollow where the face should be
        self.push_oriented(
            forward(-0.2, 9.6, 0.0),
            [3.8, 4.2, 4.0],
            robe,
            Rotation::new(yaw, -0.25 + sway, bank * 0.4),
            0.0,
            Shape::Cone,
        );
        self.push_oriented(
            forward(1.4, 8.9, 0.0),
            [2.0, 1.9, 1.1],
            [0.05, 0.03, 0.05],
            Rotation::new(yaw, sway, 0.0),
            0.0,
            Shape::Cuboid,
        );
        for (side, roll) in [(1.0f32, 0.5f32), (-1.0, -0.35)] {
            self.push_oriented(
                forward(0.9, 6.6, side * 2.2),
                [1.3, 4.0, 1.3],
                robe,
                Rotation::new(yaw, 0.35, roll + bank * 0.4),
                0.0,
                Shape::Cylinder,
            );
        }
        // staff, canted across the body, with a lit head
        self.push_oriented(
            forward(2.0, 6.4, 3.1),
            [0.7, 11.0, 0.7],
            [0.34, 0.24, 0.15],
            Rotation::new(yaw, 0.3, 0.42),
            0.0,
            Shape::Cylinder,
        );
        self.push_instance(
            forward(2.9, 11.4, 5.2),
            [2.0, 2.0, 2.0],
            trim,
            yaw,
            0.55,
            Shape::Boulder,
        );
    }

    pub(super) fn draw_orbs(&mut self) -> usize {
        let mut count = 0;
        let eye_x = self.wizards.pos_x[PLAYER];
        let eye_y = self.wizards.pos_y[PLAYER];
        let eye_z = self.wizards.pos_z[PLAYER];
        for index in 0..MAX_ORBS {
            if !self.orbs.alive[index] {
                continue;
            }
            count += 1;
            let size = 2.0 + self.orbs.amount[index] * 0.18;
            let phase = self.orbs.bob_phase[index];
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
            self.push_blip(at[0], at[2], blip, 0.7);

            let near = length_sq3(at[0] - eye_x, at[1] - eye_y, at[2] - eye_z)
                < ORB_DETAIL_RANGE * ORB_DETAIL_RANGE;
            self.draw_orb_model(at, size, phase, tint, near);
        }
        count
    }

    fn draw_orb_model(
        &mut self,
        at: [f32; 3],
        size: f32,
        phase: f32,
        tint: [f32; 3],
        near: bool,
    ) {
        let pulse = 0.75 + sin(phase * 2.0) * 0.25;
        if !near {
            self.push_instance(
                at,
                [size, size, size],
                [tint[0] * pulse, tint[1] * pulse, tint[2] * pulse],
                0.0,
                1.0,
                Shape::Sphere,
            );
            return;
        }

        // A faceted core inside a soft shell reads as a crystal; one smooth
        // emissive ball reads as a bulb, which is what it was.
        let spin = phase * 0.6;
        self.push_oriented(
            at,
            [size * 0.8, size * 0.9, size * 0.8],
            [tint[0], tint[1], tint[2]],
            Rotation::new(spin, spin * 0.7, spin * 0.4),
            1.0,
            Shape::Boulder,
        );
        self.push_instance(
            at,
            [size * 1.5 * pulse, size * 1.5 * pulse, size * 1.5 * pulse],
            [tint[0] * 0.5, tint[1] * 0.5, tint[2] * 0.5],
            0.0,
            0.5,
            Shape::Sphere,
        );
        for mote in 0..ORB_MOTES {
            let angle = phase * 1.4 + mote as f32 * core::f32::consts::TAU / ORB_MOTES as f32;
            let orbit = size * 1.5;
            let mote_size = size * 0.22;
            self.push_instance(
                [
                    at[0] + cos(angle) * orbit,
                    at[1] + sin(angle * 1.7) * size * 0.5,
                    at[2] + sin(angle) * orbit,
                ],
                [mote_size, mote_size, mote_size],
                tint,
                0.0,
                1.0,
                Shape::Sphere,
            );
        }
    }

    pub(super) fn draw_projectiles(&mut self) {
        for index in 0..MAX_PROJECTILES {
            if !self.projectiles.alive[index] {
                continue;
            }
            let at = [
                self.projectiles.pos_x[index],
                self.projectiles.pos_y[index],
                self.projectiles.pos_z[index],
            ];
            let velocity = [
                self.projectiles.vel_x[index],
                self.projectiles.vel_y[index],
                self.projectiles.vel_z[index],
            ];
            let (colour, size) = self.projectiles.kind[index].head_style();
            match self.projectiles.kind[index] {
                ProjectileKind::Firebolt
                | ProjectileKind::Meteor
                | ProjectileKind::DragonFire => {
                    self.draw_fireball(at, velocity, size, colour, index)
                }
                ProjectileKind::CreatureBolt => self.draw_bolt(at, velocity, size, colour, index),
            }
        }
    }

    /// A creature's spat bolt: a spinning faceted shard with a stretched
    /// husk around it, so it reads as a thrown thing with a direction rather
    /// than as a bead sitting in the air.
    fn draw_bolt(
        &mut self,
        at: [f32; 3],
        velocity: [f32; 3],
        size: f32,
        colour: [f32; 3],
        index: usize,
    ) {
        let speed = length3(velocity[0], velocity[1], velocity[2]);
        let along = if speed > 0.001 {
            [velocity[0] / speed, velocity[1] / speed, velocity[2] / speed]
        } else {
            [0.0, 1.0, 0.0]
        };
        let spin = self.session.elapsed * 9.0 + index as f32 * 1.3;
        // husk: stretched along flight, dim enough that the core still reads
        self.push_oriented(
            at,
            [size * 1.3, size * 1.3, size * 2.9],
            [colour[0] * 0.45, colour[1] * 0.5, colour[2] * 0.4],
            Rotation::new(atan2(along[0], along[2]), -along[1], 0.0),
            0.4,
            Shape::Sphere,
        );
        self.push_oriented(
            at,
            [size, size, size],
            colour,
            Rotation::new(spin, spin * 0.8, spin * 0.5),
            1.0,
            Shape::Boulder,
        );
        // a shard trailing behind, catching up
        self.push_oriented(
            [
                at[0] - along[0] * size * 1.9,
                at[1] - along[1] * size * 1.9,
                at[2] - along[2] * size * 1.9,
            ],
            [size * 0.5, size * 0.5, size * 0.5],
            colour,
            Rotation::new(-spin, spin * 0.6, 0.0),
            0.85,
            Shape::Boulder,
        );
    }

    /// A directional flame volume rather than a glowing marble: nested head
    /// layers around a white-hot core, licks pulled back by motion, a tapered
    /// helical wake, detached embers, then a few cool smoke lobes.
    ///
    /// Every colour is derived from `warm`, so the same assembly stays orange
    /// for Firebolt, heavy amber for Meteor, and magenta for DragonFire. Only
    /// the core is fully emissive; the outer layers retain enough shading for
    /// their overlapping silhouettes to survive bloom.
    pub(super) fn draw_fireball(
        &mut self,
        at: [f32; 3],
        velocity: [f32; 3],
        size: f32,
        warm: [f32; 3],
        index: usize,
    ) {
        const WAKE_KNOTS: i32 = 6;
        const TONGUES: i32 = 4;
        const EMBERS: i32 = 4;
        const SMOKE_LOBES: i32 = 3;

        let speed = length3(velocity[0], velocity[1], velocity[2]);
        // A stalled projectile still needs an axis to build the wake along.
        let along = if speed > 0.001 {
            [
                velocity[0] / speed,
                velocity[1] / speed,
                velocity[2] / speed,
            ]
        } else {
            [0.0, 1.0, 0.0]
        };
        // Two stable directions across the flight axis. The horizontal
        // construction avoids a basis that rolls unpredictably as the shot
        // pitches; the fallback covers a perfectly vertical meteor.
        let across = {
            let raw = [along[2], 0.0, -along[0]];
            let len = length3(raw[0], raw[1], raw[2]);
            if len > 0.001 {
                [raw[0] / len, raw[1] / len, raw[2] / len]
            } else {
                [1.0, 0.0, 0.0]
            }
        };
        let up = [
            along[1] * across[2] - along[2] * across[1],
            along[2] * across[0] - along[0] * across[2],
            along[0] * across[1] - along[1] * across[0],
        ];

        // Exact orientation of a local +Z axis along the velocity. Rolling an
        // ellipsoid around that axis changes its outline without changing its
        // direction, which gives the head a live, asymmetric edge.
        let horizontal = length3(along[0], 0.0, along[2]);
        let flight_yaw = atan2(along[0], along[2]);
        let flight_pitch = -atan2(along[1], horizontal);
        let clock = self.session.elapsed * 11.0 + index as f32 * 2.399_963;
        let flare = 0.90 + sin(clock) * 0.10;
        let cross_flare = 0.90 + cos(clock * 1.31) * 0.10;
        // Faster shots pull a longer flame, but the bounded factor keeps
        // meteors from consuming half the screen and stalled previews useful.
        let motion_stretch = clamp(speed / 180.0, 0.78, 1.25);
        // Size is also a visual vocabulary input: the nine-unit meteor should
        // remain a heavy, faceted mass when the gallery normalises its scale,
        // while the three-unit bolts stay narrow and fluid.
        let mass = clamp((size - 4.0) / 5.0, 0.0, 1.0);

        let hot = [
            1.0,
            clamp(0.72 + warm[1] * 0.28, 0.0, 1.0),
            clamp(0.46 + warm[2] * 0.45, 0.0, 1.0),
        ];
        let inner = [
            clamp(warm[0] * 0.42 + hot[0] * 0.58, 0.0, 1.0),
            clamp(warm[1] * 0.48 + hot[1] * 0.52, 0.0, 1.0),
            clamp(warm[2] * 0.54 + hot[2] * 0.46, 0.0, 1.0),
        ];
        let outer = [warm[0] * 0.78, warm[1] * 0.70, warm[2] * 0.72];
        let smoke = [
            0.14 + warm[0] * 0.18,
            0.13 + warm[1] * 0.14,
            0.14 + warm[2] * 0.12,
        ];

        // The shaded sheath is broad and slightly aft; two hotter volumes sit
        // progressively farther forward, ending in a small leading cap.
        let head_wobble = sin(clock * 0.73) * size * 0.08;
        self.push_oriented(
            [
                at[0] - along[0] * size * 0.16 + across[0] * head_wobble,
                at[1] - along[1] * size * 0.16 + across[1] * head_wobble,
                at[2] - along[2] * size * 0.16 + across[2] * head_wobble,
            ],
            [
                size * (1.72 + mass * 0.34) * flare,
                size * (1.46 + mass * 0.30) * cross_flare,
                size * (2.52 - mass * 0.32) * motion_stretch,
            ],
            outer,
            Rotation::new(flight_yaw, flight_pitch, clock * 0.18),
            0.36,
            Shape::Sphere,
        );
        self.push_oriented(
            [
                at[0] + along[0] * size * 0.20 - up[0] * head_wobble * 0.45,
                at[1] + along[1] * size * 0.20 - up[1] * head_wobble * 0.45,
                at[2] + along[2] * size * 0.20 - up[2] * head_wobble * 0.45,
            ],
            [
                size * (1.16 + mass * 0.22) * cross_flare,
                size * (1.02 + mass * 0.18) * flare,
                size * (1.70 - mass * 0.08) * motion_stretch,
            ],
            inner,
            Rotation::new(flight_yaw, flight_pitch, -clock * 0.14),
            0.68,
            if mass > 0.5 {
                Shape::Boulder
            } else {
                Shape::Sphere
            },
        );
        self.push_oriented(
            [
                at[0] + along[0] * size * 0.43,
                at[1] + along[1] * size * 0.43,
                at[2] + along[2] * size * 0.43,
            ],
            [
                size * (0.78 + mass * 0.30),
                size * (0.72 + mass * 0.28),
                size * (1.02 + mass * 0.15),
            ],
            hot,
            Rotation::new(clock * 0.31, clock * 0.23, clock * 0.17),
            0.94,
            Shape::Boulder,
        );
        self.push_oriented(
            [
                at[0] + along[0] * size * 0.78,
                at[1] + along[1] * size * 0.78,
                at[2] + along[2] * size * 0.78,
            ],
            [size * 0.42, size * 0.42, size * 0.62],
            [1.0, 0.97, clamp(0.82 + warm[2] * 0.12, 0.0, 1.0)],
            Rotation::new(flight_yaw, flight_pitch, 0.0),
            1.0,
            Shape::Sphere,
        );

        // Overlapping wake knots narrow and cool as they recede. Their centres
        // follow a loose helix rather than a ruler-straight row, so the tail
        // has a different silhouette from either side.
        for step in 0..WAKE_KNOTS {
            let progress = (step as f32 + 1.0) / WAKE_KNOTS as f32;
            let spiral = clock * 0.24 + step as f32 * 2.15;
            let lateral = size * (0.12 + progress * 0.42) * (1.0 - progress * 0.35);
            let spiral_cos = cos(spiral);
            let spiral_sin = sin(spiral);
            let radial_x = across[0] * spiral_cos + up[0] * spiral_sin;
            let radial_y = across[1] * spiral_cos + up[1] * spiral_sin;
            let radial_z = across[2] * spiral_cos + up[2] * spiral_sin;
            let back = size * (0.72 + progress * 5.0) * motion_stretch;
            let flicker = 0.91 + sin(clock * 1.07 + step as f32 * 1.91) * 0.09;
            let radius = size * (0.82 * (1.0 - progress) + 0.24) * flicker;
            let length = size
                * (1.55 * (1.0 - progress) + 0.58)
                * motion_stretch;
            let heat = 1.0 - progress;
            self.push_oriented(
                [
                    at[0] - along[0] * back + radial_x * lateral,
                    at[1] - along[1] * back + radial_y * lateral,
                    at[2] - along[2] * back + radial_z * lateral,
                ],
                [radius * 1.08, radius * 0.86, length],
                [
                    warm[0] * (0.38 + heat * 0.57),
                    warm[1] * (0.12 + heat * 0.76),
                    warm[2] * (0.08 + heat * 0.72),
                ],
                Rotation::new(flight_yaw, flight_pitch, spiral * 0.35),
                0.08 + heat * heat * 0.46,
                Shape::Sphere,
            );
        }

        // Conical licks root at the head and point aft/outward. Unlike the old
        // upright cones, their local +Y axes are aligned to the actual flame
        // direction, so pitching a projectile pitches its whole silhouette.
        for tongue in 0..TONGUES {
            let phase = tongue as f32 * 2.399_963 + clock * 0.33;
            let pulse = 0.5 + sin(clock * 1.37 + tongue as f32 * 2.11) * 0.5;
            let phase_cos = cos(phase);
            let phase_sin = sin(phase);
            let radial = [
                across[0] * phase_cos + up[0] * phase_sin,
                across[1] * phase_cos + up[1] * phase_sin,
                across[2] * phase_cos + up[2] * phase_sin,
            ];
            let splay = 0.24 + pulse * 0.18;
            let raw = [
                -along[0] + radial[0] * splay,
                -along[1] + radial[1] * splay,
                -along[2] + radial[2] * splay,
            ];
            let raw_length = length3(raw[0], raw[1], raw[2]);
            let direction = [
                raw[0] / raw_length,
                raw[1] / raw_length,
                raw[2] / raw_length,
            ];
            let length = size * (1.15 + pulse * 0.75);
            let base = [
                at[0] - along[0] * size * 0.10
                    + radial[0] * size * (0.25 + pulse * 0.12),
                at[1] - along[1] * size * 0.10
                    + radial[1] * size * (0.25 + pulse * 0.12),
                at[2] - along[2] * size * 0.10
                    + radial[2] * size * (0.25 + pulse * 0.12),
            ];
            let tongue_horizontal = length3(direction[0], 0.0, direction[2]);
            self.push_oriented(
                [
                    base[0] + direction[0] * length * 0.45,
                    base[1] + direction[1] * length * 0.45,
                    base[2] + direction[2] * length * 0.45,
                ],
                [
                    size * (0.32 + pulse * 0.16),
                    length,
                    size * (0.32 + pulse * 0.16),
                ],
                [
                    warm[0] * 0.54 + hot[0] * 0.46,
                    warm[1] * 0.58 + hot[1] * 0.42,
                    warm[2] * 0.62 + hot[2] * 0.38,
                ],
                Rotation::new(
                    atan2(direction[0], direction[2]),
                    atan2(tongue_horizontal, direction[1]),
                    0.0,
                ),
                0.48 + pulse * 0.20,
                Shape::Cone,
            );
        }

        // Detached sparks escape the wake on different deterministic phases.
        // Boulder prototypes catch one bright facet instead of reading as a
        // second row of smooth bubbles.
        for ember in 0..EMBERS {
            let progress = (ember as f32 + 1.0) / (EMBERS + 1) as f32;
            let phase = clock * 0.61 + ember as f32 * 2.73;
            let phase_cos = cos(phase);
            let phase_sin = sin(phase);
            let radial = [
                across[0] * phase_cos + up[0] * phase_sin,
                across[1] * phase_cos + up[1] * phase_sin,
                across[2] * phase_cos + up[2] * phase_sin,
            ];
            let back = size * (1.8 + progress * 4.8) * motion_stretch;
            let out = size
                * (0.52 + progress * 0.78)
                * (0.82 + sin(clock * 0.47 + ember as f32) * 0.18);
            let ember_size = size
                * (0.10 + (1.0 - progress) * 0.10)
                * (0.88 + cos(phase * 1.7) * 0.12);
            self.push_oriented(
                [
                    at[0] - along[0] * back + radial[0] * out,
                    at[1] - along[1] * back + radial[1] * out,
                    at[2] - along[2] * back + radial[2] * out,
                ],
                [ember_size * 0.72, ember_size * 1.45, ember_size * 0.72],
                [
                    warm[0] * 0.72 + hot[0] * 0.28,
                    warm[1] * 0.72 + hot[1] * 0.28,
                    warm[2] * 0.72 + hot[2] * 0.28,
                ],
                Rotation::new(phase, phase * 0.73, phase * 0.41),
                0.72 - progress * 0.28,
                Shape::Boulder,
            );
        }

        // Opaque rendering cannot support a long translucent plume, so three
        // small, dim lobes are enough to cool the tail without turning it into
        // a chain of dark rocks.
        for puff in 0..SMOKE_LOBES {
            let progress = (puff as f32 + 1.0) / SMOKE_LOBES as f32;
            let phase = clock * 0.12 + puff as f32 * 2.17;
            let phase_cos = cos(phase);
            let phase_sin = sin(phase);
            let radial = [
                across[0] * phase_cos + up[0] * phase_sin,
                across[1] * phase_cos + up[1] * phase_sin,
                across[2] * phase_cos + up[2] * phase_sin,
            ];
            let back = size * (4.70 + progress * 1.55) * motion_stretch;
            let drift = size * (0.22 + progress * 0.42);
            let puff_size = size
                * (0.50 + progress * 0.38)
                * (0.92 + sin(clock * 0.31 + puff as f32 * 1.7) * 0.08);
            self.push_oriented(
                [
                    at[0] - along[0] * back + radial[0] * drift,
                    at[1] - along[1] * back + radial[1] * drift,
                    at[2] - along[2] * back + radial[2] * drift,
                ],
                [puff_size * 1.08, puff_size * 0.76, puff_size * 1.35],
                [
                    smoke[0] * (1.0 - progress * 0.14),
                    smoke[1] * (1.0 - progress * 0.14),
                    smoke[2] * (1.0 - progress * 0.12),
                ],
                Rotation::new(flight_yaw, flight_pitch, phase),
                0.07 + (1.0 - progress) * 0.07,
                Shape::Sphere,
            );
        }
    }

}
