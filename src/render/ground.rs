//! Creatures that walk: the sand worm, the troll, and a keep's villagers
//! and soldiers. Nests live here too — they are rooted to the ground.

use super::*;

impl World {
    pub(super) fn draw_sand_worm(&mut self, body: &CreatureBody) {
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

    pub(super) fn draw_troll(&mut self, body: &CreatureBody) {
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

    pub(super) fn draw_nest(&mut self, body: &CreatureBody) {
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

    /// Townsfolk: a blocky little figure with a walk bob. Deliberately small
    /// and low-contrast next to the soldiers so the two read apart at range.
    pub(super) fn draw_villager(&mut self, body: &CreatureBody) {
        let stride = sin(body.phase * 3.0);
        let bob = abs(stride) * 0.5;
        let smock = if body.faction.is_player() {
            [0.52, 0.44, 0.68]
        } else {
            [0.62, 0.46, 0.32]
        };
        let flash = body.hurt * 0.4;
        // legs, swinging out of phase with each other
        for (side, swing) in [(1.0f32, stride), (-1.0, -stride)] {
            self.push_instance(
                body.ahead(swing * 0.9, 1.4 + bob, side * 0.9),
                [1.3, 3.0, 1.4],
                [0.30, 0.26, 0.22],
                body.facing,
                0.0,
                Shape::Cuboid,
            );
        }
        self.push_instance(
            [body.x, body.y + 4.4 + bob, body.z],
            [3.2, 3.6, 2.4],
            [smock[0] + flash, smock[1], smock[2]],
            body.facing,
            body.hurt * 0.4,
            Shape::Cuboid,
        );
        // arms
        for side in [1.0f32, -1.0] {
            self.push_instance(
                body.ahead(-side * stride * 0.8, 4.4 + bob, side * 2.1),
                [1.0, 2.8, 1.0],
                [smock[0] * 0.86, smock[1] * 0.86, smock[2] * 0.86],
                body.facing,
                0.0,
                Shape::Cuboid,
            );
        }
        self.push_instance(
            [body.x, body.y + 7.2 + bob, body.z],
            [2.2, 2.2, 2.2],
            [0.86, 0.68, 0.52],
            body.facing,
            0.0,
            Shape::Cuboid,
        );
        // hair, offset back so the face reads as facing forwards
        self.push_instance(
            body.ahead(-0.5, 8.2 + bob, 0.0),
            [2.4, 1.2, 2.0],
            [0.28, 0.20, 0.12],
            body.facing,
            0.0,
            Shape::Cuboid,
        );
    }

    /// Garrison troops: taller, armoured, and carrying a spear and shield so
    /// the silhouette is unmistakable from the air.
    pub(super) fn draw_soldier(&mut self, body: &CreatureBody) {
        let stride = sin(body.phase * 2.6);
        let bob = abs(stride) * 0.6;
        let livery = if body.faction.is_player() {
            [0.26, 0.40, 0.74]
        } else {
            [0.66, 0.22, 0.20]
        };
        let steel = [0.62, 0.64, 0.68];
        for (side, swing) in [(1.0f32, stride), (-1.0, -stride)] {
            self.push_instance(
                body.ahead(swing * 1.1, 1.8 + bob, side * 1.1),
                [1.6, 3.8, 1.7],
                [0.28, 0.26, 0.26],
                body.facing,
                0.0,
                Shape::Cuboid,
            );
        }
        // cuirass over a surcoat
        self.push_instance(
            [body.x, body.y + 5.6 + bob, body.z],
            [4.0, 4.4, 2.9],
            [
                livery[0] + body.hurt * 0.4,
                livery[1] + body.hurt * 0.2,
                livery[2],
            ],
            body.facing,
            body.hurt * 0.4,
            Shape::Cuboid,
        );
        self.push_instance(
            [body.x, body.y + 6.6 + bob, body.z],
            [4.2, 1.8, 3.1],
            steel,
            body.facing,
            0.05,
            Shape::Cuboid,
        );
        // head under a helm with a crest
        self.push_instance(
            [body.x, body.y + 8.8 + bob, body.z],
            [2.4, 2.4, 2.4],
            [0.84, 0.66, 0.50],
            body.facing,
            0.0,
            Shape::Cuboid,
        );
        self.push_instance(
            [body.x, body.y + 9.6 + bob, body.z],
            [2.7, 1.8, 2.7],
            steel,
            body.facing,
            0.05,
            Shape::Cuboid,
        );
        self.push_instance(
            [body.x, body.y + 11.0 + bob, body.z],
            [0.8, 1.6, 3.0],
            [livery[0] * 1.2, livery[1] * 1.2, livery[2] * 1.2],
            body.facing,
            0.0,
            Shape::Cone,
        );
        // spear in the right hand, shield on the left
        self.push_instance(
            body.ahead(0.6, 7.0 + bob, 2.6),
            [0.7, 11.0, 0.7],
            [0.36, 0.26, 0.16],
            body.facing,
            0.0,
            Shape::Cuboid,
        );
        self.push_instance(
            body.ahead(0.6, 12.6 + bob, 2.6),
            [0.9, 2.4, 0.9],
            steel,
            body.facing,
            0.1,
            Shape::Cone,
        );
        // Held clear of the body, and rimmed in steel: pressed against the
        // surcoat it just widened the torso into a slab.
        self.push_instance(
            body.ahead(1.0, 5.4 + bob, -3.6),
            [0.8, 4.8, 4.0],
            steel,
            body.facing,
            0.05,
            Shape::Cuboid,
        );
        self.push_instance(
            body.ahead(1.5, 5.4 + bob, -3.6),
            [0.5, 3.2, 2.6],
            [livery[0] * 0.9, livery[1] * 0.9, livery[2] * 0.9],
            body.facing,
            0.0,
            Shape::Cuboid,
        );
    }

}
