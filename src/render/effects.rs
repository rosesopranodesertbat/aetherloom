//! Carpets, mana orbs, projectiles and the fireball.

use super::*;

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
            let yaw = self.wizards.yaw[wizard];
            self.push_instance(at, [13.0, 1.1, 17.0], [0.72, 0.16, 0.14], yaw, 0.2, Shape::Cuboid);
            self.push_shadow(at[0], at[2], at[1], 9.0, 0.5);
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

    pub(super) fn draw_orbs(&mut self) -> usize {
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
                ProjectileKind::CreatureBolt => {
                    self.push_instance(at, [size, size, size], colour, 0.0, 1.0, Shape::Sphere)
                }
            }
        }
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
