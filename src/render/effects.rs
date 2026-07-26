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
            let pulse = 0.75 + sin(phase * 2.0) * 0.25;
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
            if !near {
                self.push_instance(
                    at,
                    [size, size, size],
                    [tint[0] * pulse, tint[1] * pulse, tint[2] * pulse],
                    0.0,
                    1.0,
                    Shape::Sphere,
                );
                continue;
            }

            // A faceted core inside a soft shell reads as a crystal; one
            // smooth emissive ball reads as a bulb, which is what it was.
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
        count
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

    /// A fireball, not a glowing marble: a white-hot core inside a swollen
    /// orange body, a tapering wake of cooling blobs behind it, and tongues
    /// that lick outward and shift frame to frame.
    ///
    /// Only the core is fully emissive. Everything else keeps enough ordinary
    /// shading to show its own form — push the glow up and the bloom fuses the
    /// whole thing back into the white ball this was written to replace.
    pub(super) fn draw_fireball(
        &mut self,
        at: [f32; 3],
        velocity: [f32; 3],
        size: f32,
        warm: [f32; 3],
        index: usize,
    ) {
        /// Blobs in the wake behind the head.
        const WAKE: i32 = 5;
        /// Licking tongues around the head.
        const TONGUES: i32 = 4;

        let speed = length3(velocity[0], velocity[1], velocity[2]);
        // a stalled projectile still needs an axis to build the wake along
        let along = if speed > 0.001 {
            [
                velocity[0] / speed,
                velocity[1] / speed,
                velocity[2] / speed,
            ]
        } else {
            [0.0, 1.0, 0.0]
        };
        // any two directions across the flight axis, for the tongues
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
        // every projectile flickers on its own clock
        let clock = self.session.elapsed * 14.0 + index as f32 * 1.7;
        let flare = 0.86 + sin(clock) * 0.14;

        // wake: blobs trailing back, growing then shrinking, cooling to smoke
        for step in 0..WAKE {
            let along_wake = (step as f32 + 1.0) / WAKE as f32;
            let back = size * (0.9 + along_wake * 3.6);
            let swell = sin(along_wake * 3.14159) * 0.5 + 0.55;
            let blob = size * swell * (1.0 - along_wake * 0.35) * flare;
            let wobble = sin(clock * 0.7 + step as f32 * 2.1) * size * 0.28;
            self.push_instance(
                [
                    at[0] - along[0] * back + across[0] * wobble,
                    at[1] - along[1] * back + across[1] * wobble,
                    at[2] - along[2] * back + across[2] * wobble,
                ],
                [blob, blob, blob],
                [
                    warm[0] * (1.0 - along_wake * 0.55),
                    warm[1] * (1.0 - along_wake * 0.72),
                    warm[2] * (1.0 - along_wake * 0.82),
                ],
                0.0,
                0.42 * (1.0 - along_wake * 0.85),
                Shape::Sphere,
            );
        }

        // outer body: the bulk of the flame, a little ahead of the wake
        let body = size * 1.5 * flare;
        self.push_instance(
            at,
            [body, body, body],
            [warm[0] * 0.92, warm[1] * 0.62, warm[2] * 0.36],
            0.0,
            0.46,
            Shape::Sphere,
        );

        // tongues, splayed around the axis and dragged backwards
        for tongue in 0..TONGUES {
            let angle = tongue as f32 * core::f32::consts::TAU / TONGUES as f32 + clock * 0.35;
            let reach = size * (0.9 + sin(clock * 1.3 + tongue as f32 * 2.3) * 0.45);
            let out_x = across[0] * cos(angle) + up[0] * sin(angle);
            let out_y = across[1] * cos(angle) + up[1] * sin(angle);
            let out_z = across[2] * cos(angle) + up[2] * sin(angle);
            let lick = size * 1.1;
            self.push_instance(
                [
                    at[0] + out_x * reach - along[0] * size * 0.5,
                    at[1] + out_y * reach - along[1] * size * 0.5,
                    at[2] + out_z * reach - along[2] * size * 0.5,
                ],
                [lick * 0.72, lick * 1.7, lick * 0.72],
                [0.98, 0.44, 0.10],
                angle,
                0.62,
                Shape::Cone,
            );
        }

        // white-hot core, slightly ahead so the leading edge is brightest
        let core = size * 0.62 * flare;
        self.push_instance(
            [
                at[0] + along[0] * size * 0.35,
                at[1] + along[1] * size * 0.35,
                at[2] + along[2] * size * 0.35,
            ],
            [core, core, core],
            [1.0, 0.95, 0.74],
            0.0,
            1.0,
            Shape::Sphere,
        );
    }

}
