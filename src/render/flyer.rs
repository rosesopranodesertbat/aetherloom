//! Creatures that fly, and the wing they share.

use super::*;

impl World {
    /// A wing built from panels that step outward, sweep back and lift more
    /// the further they are from the shoulder — so the tip whips and the root
    /// barely moves, instead of the whole slab see-sawing as one oval.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_wing(
        &mut self,
        body: &CreatureBody,
        side: f32,
        flap: f32,
        span: f32,
        chord: f32,
        membrane: [f32; 3],
        bone: [f32; 3],
        feathered: bool,
    ) {
        const PANELS: i32 = 4;
        /// Fraction of the chord each panel is dragged backwards, at the tip.
        const TIP_SWEEP: f32 = 0.55;
        /// How much narrower the chord is at the tip than at the shoulder.
        const TIP_TAPER: f32 = 0.48;

        let panel_span = span / PANELS as f32;
        for panel in 0..PANELS {
            let along = (panel as f32 + 1.0) / PANELS as f32;
            // squared, so the outer panels carry the stroke
            let lift = flap * along * along;
            let sweep = -chord * TIP_SWEEP * along;
            let out = span * along;
            let panel_chord = chord * (1.0 - TIP_TAPER * along);
            let twist = side * along * 0.3 + flap * 0.05;
            self.push_instance(
                body.ahead(sweep, lift, side * out),
                [panel_span * 1.12, 0.75, panel_chord],
                [
                    membrane[0] * (1.0 - along * 0.12),
                    membrane[1] * (1.0 - along * 0.12),
                    membrane[2] * (1.0 - along * 0.12),
                ],
                body.facing + twist,
                0.04,
                Shape::Cuboid,
            );
            // leading-edge bone, thicker at the shoulder
            self.push_instance(
                body.ahead(sweep + panel_chord * 0.44, lift + 0.35, side * out),
                [panel_span * 0.95, 1.15 - along * 0.4, panel_chord * 0.2],
                bone,
                body.facing + twist,
                0.0,
                Shape::Cuboid,
            );
        }

        let tip_lift = flap;
        let tip_sweep = -chord * TIP_SWEEP;
        if feathered {
            // primaries fan off the tip and trail behind
            for feather in 0..4 {
                let spread = feather as f32 * 0.16;
                self.push_instance(
                    body.ahead(
                        tip_sweep - chord * (0.35 + spread),
                        tip_lift - spread * 1.2,
                        side * (span * (1.0 + spread * 0.5)),
                    ),
                    [1.5, 0.55, chord * (0.75 - spread * 0.5)],
                    [bone[0] * 1.05, bone[1] * 1.02, bone[2]],
                    body.facing + side * (0.35 + spread),
                    0.0,
                    Shape::Cuboid,
                );
            }
        } else {
            // finger struts run from the shoulder out to the trailing edge,
            // pinching the membrane into scallops the way a bat wing does
            for finger in 0..3 {
                let along = 0.4 + finger as f32 * 0.28;
                self.push_instance(
                    body.ahead(
                        -chord * (0.2 + finger as f32 * 0.22),
                        flap * along * along,
                        side * span * along,
                    ),
                    [span * 0.5, 0.85, 0.9],
                    bone,
                    body.facing + side * (0.5 + finger as f32 * 0.2),
                    0.0,
                    Shape::Cuboid,
                );
            }
            // claw at the leading tip
            self.push_instance(
                body.ahead(tip_sweep + chord * 0.4, tip_lift + 0.4, side * span * 1.06),
                [1.1, 1.1, 2.6],
                [0.9, 0.86, 0.74],
                body.facing + side * 0.4,
                0.0,
                Shape::Cone,
            );
        }
    }

    pub(super) fn draw_griffin(&mut self, body: &CreatureBody) {
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
        // shoulders, so the wings look joined on rather than stuck through
        for side in [1.0f32, -1.0] {
            self.push_instance(
                body.ahead(0.6, 1.8, side * 2.4),
                [2.6, 2.6, 3.4],
                [0.86, 0.80, 0.60],
                body.facing,
                0.0,
                Shape::Sphere,
            );
        }
        let beat = sin(body.phase * 2.0) * 4.2;
        for side in [1.0f32, -1.0] {
            self.draw_wing(
                body,
                side,
                2.0 + beat,
                11.0,
                6.4,
                [0.92, 0.86, 0.70],
                [0.74, 0.64, 0.44],
                true,
            );
        }
        // hind legs, tucked
        for side in [1.0f32, -1.0] {
            self.push_instance(
                body.ahead(-2.6, -2.4, side * 2.0),
                [2.0, 2.4, 3.6],
                [0.70, 0.58, 0.36],
                body.facing,
                0.0,
                Shape::Cuboid,
            );
        }
        // front talons
        for side in [1.0f32, -1.0] {
            self.push_instance(
                body.ahead(2.6, -2.8, side * 1.8),
                [1.6, 2.0, 2.8],
                [0.95, 0.72, 0.18],
                body.facing,
                0.0,
                Shape::Cone,
            );
        }
    }

    pub(super) fn draw_wasp(&mut self, body: &CreatureBody) {
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

    pub(super) fn draw_wraith(&mut self, body: &CreatureBody) {
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

    pub(super) fn draw_balloon(&mut self, body: &CreatureBody) {
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

    pub(super) fn draw_dragon(&mut self, body: &CreatureBody) {
        /// Neck, trunk and tail are one continuous run of segments.
        const SEGMENTS: i32 = 9;
        const SEGMENT_SPACING: f32 = 6.2;
        /// Segment where the shoulders — and so the wings — sit.
        const SHOULDER_SEGMENT: i32 = 2;
        /// Lateral travel of the body wave, and how fast it runs down the body.
        const WAVE_AMPLITUDE: f32 = 5.5;
        const WAVE_LAG: f32 = 0.72;
        const WAVE_RISE: f32 = 2.6;

        let scale_dark = [0.34 + body.hurt * 0.4, 0.09, 0.12];
        let scale_lit = [0.58 + body.hurt * 0.4, 0.17, 0.19];
        let belly = [0.72, 0.46, 0.24];

        for segment in 0..SEGMENTS {
            // segment 0 is the base of the neck; positive goes back down
            // the tail, negative forward into the neck
            let back = (segment - SHOULDER_SEGMENT) as f32 * SEGMENT_SPACING;
            let wave = sin(body.phase - (segment as f32) * WAVE_LAG);
            let lateral = wave * WAVE_AMPLITUDE;
            let rise = cos(body.phase - (segment as f32) * WAVE_LAG) * WAVE_RISE;
            // thickest at the shoulders, tapering both ways
            let from_shoulder = abs((segment - SHOULDER_SEGMENT) as f32);
            let girth = max(8.4 - from_shoulder * 1.15, 1.8);
            let at = body.ahead(-back, rise, lateral);
            let tilt = body.facing + wave * 0.22;
            self.push_instance(
                at,
                [girth, girth * 0.88, girth * 1.25],
                scale_lit,
                tilt,
                body.hurt * 0.5,
                Shape::Sphere,
            );
            // pale underside
            self.push_instance(
                [at[0], at[1] - girth * 0.34, at[2]],
                [girth * 0.62, girth * 0.3, girth * 1.1],
                belly,
                tilt,
                0.0,
                Shape::Sphere,
            );
            // dorsal spine, taller over the shoulders
            if segment > 0 {
                let spine = max(girth * 0.62, 1.4);
                self.push_instance(
                    [at[0], at[1] + girth * 0.5 + spine * 0.3, at[2]],
                    [girth * 0.22, spine, girth * 0.7],
                    scale_dark,
                    tilt,
                    0.0,
                    Shape::Cone,
                );
            }
        }

        // wings, off the shoulder segment so they ride the body wave
        let shoulder_wave = sin(body.phase - SHOULDER_SEGMENT as f32 * WAVE_LAG);
        let shoulder = CreatureBody {
            x: body.x + body.facing_cos * shoulder_wave * WAVE_AMPLITUDE,
            y: body.y + cos(body.phase - SHOULDER_SEGMENT as f32 * WAVE_LAG) * WAVE_RISE,
            z: body.z - body.facing_sin * shoulder_wave * WAVE_AMPLITUDE,
            ..*body
        };
        let beat = sin(body.phase * 1.5) * 7.0;
        for side in [1.0f32, -1.0] {
            self.draw_wing(
                &shoulder,
                side,
                3.0 + beat,
                19.0,
                10.0,
                [0.40, 0.13, 0.16],
                [0.24, 0.07, 0.09],
                false,
            );
        }

        // head, at the front of the neck run
        let head_wave = sin(body.phase + WAVE_LAG * 2.0);
        let head_side = head_wave * WAVE_AMPLITUDE * 0.8;
        let head_rise = cos(body.phase + WAVE_LAG * 2.0) * WAVE_RISE;
        let snout_yaw = body.facing + head_wave * 0.3;
        let head_forward = (SHOULDER_SEGMENT as f32 + 2.4) * SEGMENT_SPACING;
        self.push_instance(
            body.ahead(head_forward, head_rise + 1.0, head_side),
            [5.2, 4.6, 8.0],
            scale_lit,
            snout_yaw,
            body.hurt * 0.5,
            Shape::Sphere,
        );
        // jaw and snout
        self.push_instance(
            body.ahead(head_forward + 4.2, head_rise - 0.6, head_side),
            [3.6, 2.6, 5.4],
            scale_dark,
            snout_yaw,
            0.0,
            Shape::Cuboid,
        );
        for side in [1.0f32, -1.0] {
            // teeth
            self.push_instance(
                body.ahead(head_forward + 6.4, head_rise - 1.2, head_side + side * 1.1),
                [0.9, 1.6, 0.9],
                [0.94, 0.90, 0.78],
                snout_yaw,
                0.0,
                Shape::Cone,
            );
            // swept horns
            self.push_instance(
                body.ahead(head_forward - 2.4, head_rise + 4.0, head_side + side * 2.0),
                [1.4, 4.4, 1.4],
                [0.86, 0.80, 0.66],
                snout_yaw + side * 0.4,
                0.0,
                Shape::Cone,
            );
            // eyes
            self.push_instance(
                body.ahead(head_forward + 2.2, head_rise + 1.8, head_side + side * 1.9),
                [1.3, 1.3, 1.3],
                [1.0, 0.86, 0.20],
                snout_yaw,
                1.0,
                Shape::Sphere,
            );
            // jaw frill
            self.push_instance(
                body.ahead(head_forward - 1.0, head_rise - 0.4, head_side + side * 3.0),
                [2.6, 3.4, 3.0],
                scale_dark,
                snout_yaw + side * 0.5,
                0.0,
                Shape::Cone,
            );
        }

        // tail fin, riding the end of the wave
        let tail_index = SEGMENTS - 1;
        let tail_wave = sin(body.phase - tail_index as f32 * WAVE_LAG);
        let tail_back = (tail_index - SHOULDER_SEGMENT) as f32 * SEGMENT_SPACING + 4.0;
        self.push_instance(
            body.ahead(
                -tail_back,
                cos(body.phase - tail_index as f32 * WAVE_LAG) * WAVE_RISE,
                tail_wave * WAVE_AMPLITUDE,
            ),
            [2.6, 6.0, 5.0],
            scale_dark,
            body.facing + tail_wave * 0.3,
            0.0,
            Shape::Cone,
        );
    }

}
