//! Creatures that walk: the sand worm, the troll, and a keep's villagers
//! and soldiers. Nests live here too — they are rooted to the ground.

use super::*;

/// Body rings on a worm at each detail tier. An annelid reads as a long chain
/// of many close segments, so there have to be plenty of them; the tail is
/// what goes first, since the head and its mouth carry the read at any
/// distance. The rings that survive stretch to cover for the ones that do not.
const WORM_RINGS_FULL: usize = 12;
const WORM_RINGS_REDUCED: usize = 8;
const WORM_RINGS_DISTANT: usize = 4;
/// Centre-to-centre spacing as a fraction of a ring's own girth. Well under
/// the ring's own length, so each one is buried better than a third of the way
/// into the one ahead and no seam is ever on show.
const WORM_RING_PITCH: f32 = 0.68;
/// Radians of twist added per ring, so the ribbing spirals down the body
/// instead of lining up into a row of identical beads. It also walks the
/// dorsal scutes off the spine and back, which is what stops the row of them
/// reading as a zip.
const WORM_TWIST: f32 = 0.38;
/// How far each ring lags the one ahead in the travelling wave. Tuned so a
/// wavelength spans most of the body at full detail, and stated per full-detail
/// ring so the coarser tiers snake through the same shape.
const WORM_WAVE_LAG: f32 = 0.52;
/// How far the mouth teeth cant in towards the throat, and the palps out.
const WORM_TOOTH_CANT: f32 = 0.44;
const WORM_PALP_SPLAY: f32 = 0.58;

/// Knee and knuckle heights before a troll's own stature multiplier. Knuckles
/// hanging below the knee is the whole silhouette, so the two move together.
const TROLL_KNEE: f32 = 5.6;
const TROLL_KNUCKLE: f32 = 4.3;

/// Chitin spines on a nest at each tier. The full figure is a ceiling, not a
/// count: each nest knocks a few off its own total, so no two carry the same
/// number of them.
const NEST_SPINES_FULL: usize = 10;
const NEST_SPINES_REDUCED: usize = 5;
const NEST_SPINES_DISTANT: usize = 2;

/// A frustum stood on its head, so it is wide at the top: shoulders broader
/// than the waist, a thigh thicker at the hip. Pitch still reads the usual way
/// through the flip; roll does not, so it is negated here and callers can go
/// on thinking "positive raises the right side".
fn upended(yaw: f32, pitch: f32, roll: f32) -> Rotation {
    Rotation::new(yaw, core::f32::consts::PI + pitch, -roll)
}

impl World {
    /// An annelid, not three boulders and a mouth: a long chain of small
    /// rings creased by darker collars, dorsal scutes lapping over one another
    /// like roof tiles and wandering off the spine as the body twists, a paler
    /// keeled belly, bristles raked back off the flanks, and a maw that is two
    /// staggered rosettes of teeth around a throat that recedes.
    pub(super) fn draw_sand_worm(&mut self, body: &CreatureBody) {
        /// Overlapping ventral plates, front to back. Three is the fewest that
        /// still reads as a segmented belly rather than one pale smear.
        const BELLY_PLATES: usize = 3;
        /// Rings carrying a raked bristle on each flank. Setae are the cheapest
        /// thing that stops a chain of smooth lumps reading as smooth.
        const SETAE_RINGS: [usize; 3] = [2, 5, 8];
        /// Fewest teeth in the outer rosette; a worm grows up to two more off
        /// its own hash. The inner rosette is fixed — it is half-hidden anyway,
        /// and its job is to stagger the outer one rather than to be counted.
        const TEETH_OUTER: usize = 6;
        const TEETH_INNER: usize = 3;

        let rings = match body.detail {
            BodyDetail::Full => WORM_RINGS_FULL,
            BodyDetail::Reduced => WORM_RINGS_REDUCED,
            BodyDetail::Distant => WORM_RINGS_DISTANT,
        };
        // Fewer rings at range still have to span the same animal, so each one
        // lengthens to cover for the ones that are gone, and every station
        // along the body is quoted in full-detail rings. Without this the worm
        // visibly shortens and stops writhing as the player backs away.
        let stretch = min(WORM_RINGS_FULL as f32 / rings as f32, 3.0);

        // Centre, orientation and length for a Y-axis piece — cone or cylinder
        // — spanning `from` to `to`. Bristles, palps and the tail horn are all
        // described by their two ends, and solving for the ends is what keeps a
        // joint on its socket instead of near it.
        let shaft = |from: [f32; 3], to: [f32; 3]| -> ([f32; 3], Rotation, f32) {
            let delta = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
            let length = max(length3(delta[0], delta[1], delta[2]), 0.001);
            let flat = sqrt(length_sq2(delta[0], delta[2]));
            (
                [
                    (from[0] + to[0]) * 0.5,
                    (from[1] + to[1]) * 0.5,
                    (from[2] + to[2]) * 0.5,
                ],
                Rotation::new(atan2(delta[0], delta[2]), atan2(flat, delta[1]), 0.0),
                length,
            )
        };
        // Two pieces of one creature in one flat colour read as one moulded
        // object; a shade between them reads as two parts.
        let shaded = |base: [f32; 3], amount: f32| -> [f32; 3] {
            [base[0] * amount, base[1] * amount, base[2] * amount]
        };

        let girth = 6.6 * (0.84 + body.vary_unit(1.0) * 0.34);
        // How hard this one narrows behind the shoulder: some are near-parallel
        // pipes, some are all shoulder and then a whip.
        let waisting = 0.30 + body.vary_unit(14.0) * 0.40;
        // Three skins, not two — bleached, rust-dark, and an ashen grey that
        // reads as a different animal again. A burrow that turns up four
        // identical worms is what all of this hashing exists to prevent.
        let skin = body.vary_unit(2.0);
        let hide = if skin < 0.34 {
            body.tint([0.75, 0.67, 0.49], 3.0, 0.08)
        } else if skin < 0.70 {
            body.tint([0.55, 0.42, 0.23], 3.0, 0.10)
        } else {
            body.tint([0.45, 0.39, 0.41], 3.0, 0.09)
        };
        let chitin = [hide[0] * 0.46, hide[1] * 0.39, hide[2] * 0.36];
        let crease = shaded(chitin, 0.58);
        let bone = [
            hide[0] * 0.5 + 0.46,
            hide[1] * 0.5 + 0.44,
            hide[2] * 0.4 + 0.38,
        ];
        let belly = [
            min(hide[0] + 0.20, 1.0),
            min(hide[1] + 0.19, 1.0),
            min(hide[2] + 0.26, 1.0),
        ];
        let sway = 3.2 + body.vary_unit(5.0) * 3.2;
        let heave = 2.0 + body.vary_unit(6.0) * 2.6;
        let twist = body.vary(7.0) * 1.4;
        // Most of them have had a scute prised off by something bigger, and a
        // few have lost the pair behind it as well.
        let lost = if body.quirk(8.0, 0.62) {
            2 + (body.vary_unit(9.0) * 4.99) as usize
        } else {
            rings
        };
        let lost_pair = if body.quirk(15.0, 0.30) { lost + 2 } else { rings };
        let scar_ring = if body.quirk(10.0, 0.40) {
            1 + (body.vary_unit(11.0) * 4.99) as usize
        } else {
            rings
        };
        let wave = |index: f32| -> (f32, f32) {
            let travel = body.phase - index * WORM_WAVE_LAG;
            // the tail whips through a wider arc than the shoulders do, and
            // the two axes run at different rates so the body writhes rather
            // than swinging as one rigid arc
            let arc = 0.38 + index / WORM_RINGS_FULL as f32 * 1.30;
            (
                sin(travel) * sway * arc,
                sin(travel * 0.63 + 1.2) * heave * arc,
            )
        };

        let mut back = 0.0;
        // where the last ring ended up, so the tail horn hinges on it rather
        // than on wherever the accumulated step happened to stop
        let mut tail_back = 0.0;
        let mut tail_width = girth * 0.13;
        for ring in 0..rings {
            let along = ring as f32 / (rings - 1) as f32;
            let index = ring as f32 * stretch;
            // fattest a third of the way down, then narrowing at this worm's
            // own rate; the floor stops the last ring vanishing outright
            let profile = max(
                sin((0.34 + along * 0.62) * core::f32::consts::PI) * (1.0 - along * waisting),
                0.13,
            );
            let width = girth * profile * (1.0 + body.vary(40.0 + ring as f32) * 0.10);
            let step = width * WORM_RING_PITCH * stretch;
            let (side_now, lift_now) = wave(index);
            let (side_next, lift_next) = wave(index + stretch);
            // each ring aims along the body rather than along the heading, or
            // the chain reads as beads threaded on a straight wire
            let aim = Rotation::new(
                body.facing + atan2(side_now - side_next, step),
                -atan2(lift_now - lift_next, step),
                index * WORM_TWIST + twist,
            );
            let rise = lift_now + width * 0.34;
            let shade = 1.0 - along * 0.16 + body.vary(90.0 + ring as f32) * 0.06;
            self.push_oriented(
                body.ahead(-back, rise, side_now),
                [width, width * 0.95, width * 1.25 * stretch],
                [
                    clamp(hide[0] * shade, 0.0, 1.0),
                    clamp(hide[1] * shade, 0.0, 1.0),
                    clamp(hide[2] * shade, 0.0, 1.0),
                ],
                aim,
                0.0,
                Shape::Boulder,
            );
            // A dark collar sunk into the crease behind the ring. One instance
            // apiece, and it is the whole difference between a segmented body
            // and a row of boulders.
            if body.detail.at_least(BodyDetail::Full) && ring % 2 == 1 && ring + 1 < rings {
                let (gap_side, gap_lift) = wave(index + stretch * 0.5);
                self.push_oriented(
                    body.ahead(-back - step * 0.5, gap_lift + width * 0.32, gap_side),
                    [width * 1.01, step * 0.40, width * 1.04],
                    crease,
                    Rotation::new(aim.yaw, aim.pitch + core::f32::consts::FRAC_PI_2, 0.0),
                    0.0,
                    Shape::Cylinder,
                );
            }
            let plated = match body.detail {
                BodyDetail::Full => ring % 2 == 0 && ring > 0 && ring != lost && ring != lost_pair,
                BodyDetail::Reduced => ring % 3 == 2,
                BodyDetail::Distant => false,
            };
            if plated {
                // The scutes ride the spine but walk either side of it with the
                // twist, so the row spirals instead of running dead straight,
                // and each is long enough to lap the ring behind like a tile.
                let sit = sin(index * WORM_TWIST + twist) * 0.55;
                let stand = width * 0.30;
                self.push_oriented(
                    body.ahead(
                        -back + step * 0.26,
                        rise + cos(sit) * stand,
                        side_now + sin(sit) * stand,
                    ),
                    [
                        width * (0.78 + body.vary_unit(50.0 + ring as f32) * 0.36),
                        width * (0.26 + body.vary_unit(52.0 + ring as f32) * 0.26),
                        step * 1.7,
                    ],
                    shaded(chitin, 0.92 + body.vary(54.0 + ring as f32) * 0.14),
                    Rotation::new(
                        aim.yaw,
                        aim.pitch + 0.14 + body.vary(56.0 + ring as f32) * 0.26,
                        sit,
                    ),
                    0.0,
                    Shape::Wedge,
                );
            }
            if body.detail.at_least(BodyDetail::Full) {
                // A paler keeled belly, rolled over so the wedge's ridge is the
                // keel. Only on show when the worm rears, which is exactly when
                // the player is looking straight at it.
                if ring % 2 == 1 && ring < BELLY_PLATES * 2 {
                    self.push_oriented(
                        body.ahead(-back - step * 0.22, rise - width * 0.40, side_now),
                        [width * 0.74, width * 0.30, step * 2.4],
                        shaded(belly, 0.90 + body.vary(70.0 + ring as f32) * 0.12),
                        Rotation::new(
                            aim.yaw,
                            aim.pitch,
                            core::f32::consts::PI + aim.roll * 0.15,
                        ),
                        0.0,
                        Shape::Wedge,
                    );
                }
                if SETAE_RINGS.contains(&ring) {
                    for flank in [1.0f32, -1.0] {
                        // raked back and down off the low flank, the way a
                        // burrower's bristles lie so the sand slides past
                        let salt = 100.0 + ring as f32 * 3.0 + flank;
                        // rooted inside the ring's own surface, so the bristle
                        // grows out of the flank rather than hovering off it
                        let out = width * 0.40;
                        let low = rise - width * 0.24;
                        let bristle = width * (0.34 + body.vary_unit(salt) * 0.28);
                        let droop = 0.40 + body.vary_unit(salt + 1.0) * 0.50;
                        let (at, rotation, length) = shaft(
                            body.ahead(-back, low, side_now + flank * out),
                            body.ahead(
                                -back - bristle * 1.5,
                                low - bristle * droop,
                                side_now + flank * (out + bristle * 0.5),
                            ),
                        );
                        self.push_oriented(
                            at,
                            [width * 0.09, length, width * 0.09],
                            shaded(chitin, 0.7),
                            rotation,
                            0.0,
                            Shape::Cone,
                        );
                    }
                }
                if ring == scar_ring {
                    // a healed rake across one flank, off something with claws
                    self.push_oriented(
                        body.ahead(-back, rise + width * 0.16, side_now + width * 0.44),
                        [width * 0.13, width * 0.26, step * 2.2],
                        bone,
                        Rotation::new(aim.yaw + 0.34, aim.pitch, aim.roll),
                        0.0,
                        Shape::Wedge,
                    );
                }
            }
            tail_back = back;
            tail_width = width;
            back += step;
        }

        // the head leads the wave, so the neck reads as dragging the body on
        let (head_side, head_lift) = wave(-1.1);
        let (neck_side, neck_lift) = wave(0.0);
        let reach = girth * 1.02;
        let head_yaw = body.facing + atan2(head_side - neck_side, reach);
        let head_pitch = -atan2(head_lift - neck_lift, reach);
        let head_up = head_lift + girth * 0.42;
        // The head leads the turn, so its own axis is not the body's heading.
        // Everything on the face hangs off that axis: place the mouth on the
        // heading instead and it slides round the muzzle every time it swings.
        let lead_pitch = head_pitch + 0.10;
        let lead_yaw = head_yaw - body.facing;
        let muzzle = |out: f32| -> (f32, f32) {
            (
                head_up - sin(lead_pitch) * out,
                head_side + sin(lead_yaw) * out,
            )
        };
        self.push_oriented(
            body.ahead(reach, head_up, head_side),
            [girth * 1.04, girth * 0.96, girth * 1.34],
            [
                clamp(hide[0] * 0.84, 0.0, 1.0),
                clamp(hide[1] * 0.82, 0.0, 1.0),
                clamp(hide[2] * 0.86, 0.0, 1.0),
            ],
            Rotation::new(head_yaw, lead_pitch, twist * 0.5),
            0.0,
            Shape::Boulder,
        );
        if !body.detail.at_least(BodyDetail::Reduced) {
            return;
        }

        let maw = reach + girth * 0.76;
        let (maw_up, maw_side) = muzzle(girth * 0.76);
        // The rim the teeth are set into, so they grow out of something rather
        // than floating on the face. It is wider than the throat, so what shows
        // round the hole is a lip of chitin and not the hide.
        let (rim_up, rim_side) = muzzle(girth * 0.60);
        self.push_oriented(
            body.ahead(maw - girth * 0.16, rim_up, rim_side),
            [girth * 0.92, girth * 0.44, girth * 0.92],
            shaded(chitin, 1.12),
            Rotation::new(head_yaw, lead_pitch + core::f32::consts::FRAC_PI_2, 0.0),
            0.0,
            Shape::Cylinder,
        );
        // The throat, as a cone with its point buried back inside the head: the
        // gullet narrows away from the viewer from every angle but dead ahead,
        // where a dark disc pasted across the face only ever reads as paint.
        let (throat_up, throat_side) = muzzle(girth * 0.40);
        self.push_oriented(
            body.ahead(maw - girth * 0.36, throat_up, throat_side),
            [girth * 0.74, girth * 0.90, girth * 0.74],
            [0.11, 0.04, 0.05],
            Rotation::new(head_yaw, lead_pitch - core::f32::consts::FRAC_PI_2, 0.0),
            0.08,
            Shape::Cone,
        );
        let teeth = if body.detail.at_least(BodyDetail::Full) {
            TEETH_OUTER + (body.vary_unit(4.0) * 2.99) as usize
        } else {
            4
        };
        // the mouth works, so the ring opens and closes as it comes
        let gape = 1.0 + sin(body.phase * 1.4) * 0.20;
        for tooth in 0..teeth {
            let around = tooth as f32 / teeth as f32 * core::f32::consts::TAU
                + body.vary(60.0 + tooth as f32) * 0.24;
            let radius = girth * 0.40 * gape;
            let length = girth * 0.38 * (0.66 + body.vary_unit(70.0 + tooth as f32) * 0.74);
            // a small cant about both of the mouth's own axes points the tooth
            // down the throat; at this angle the two compose without a basis
            self.push_oriented(
                body.ahead(
                    maw,
                    maw_up + sin(around) * radius,
                    maw_side + cos(around) * radius,
                ),
                [length * 0.30, length, length * 0.30],
                shaded(bone, 0.94 + body.vary(72.0 + tooth as f32) * 0.12),
                Rotation::new(
                    head_yaw,
                    lead_pitch + core::f32::consts::FRAC_PI_2 + WORM_TOOTH_CANT * sin(around),
                    WORM_TOOTH_CANT * cos(around),
                ),
                0.0,
                Shape::Cone,
            );
        }
        if !body.detail.at_least(BodyDetail::Full) {
            return;
        }

        // A second rosette, shorter and canted harder, standing a little proud
        // of the first and staggered half a tooth round from it, so the gape
        // reads as a rasp boring into the sand and not a ring of pegs. It has
        // to clear the throat cone, which is why it sits ahead of the rim.
        let (inner_up, inner_side) = muzzle(girth * 0.92);
        for tooth in 0..TEETH_INNER {
            let around = (tooth as f32 + 0.5) / TEETH_INNER as f32 * core::f32::consts::TAU
                + body.vary(64.0 + tooth as f32) * 0.34;
            let radius = girth * 0.24 * gape;
            let length = girth * 0.24 * (0.60 + body.vary_unit(74.0 + tooth as f32) * 0.70);
            let cant = WORM_TOOTH_CANT * 1.6;
            self.push_oriented(
                body.ahead(
                    maw + girth * 0.16,
                    inner_up + sin(around) * radius,
                    inner_side + cos(around) * radius,
                ),
                [length * 0.34, length, length * 0.34],
                shaded(bone, 0.76),
                Rotation::new(
                    head_yaw,
                    lead_pitch + core::f32::consts::FRAC_PI_2 + cant * sin(around),
                    cant * cos(around),
                ),
                0.0,
                Shape::Cone,
            );
        }

        // Grasping palps, jointed: a thick base swung out and forward off the
        // cheek, then a tapered finger hooking back in under the mouth. Some
        // worms carry stubs, some carry half a limb.
        let palp_reach = 0.55 + body.vary_unit(12.0) * 0.95;
        for side in [1.0f32, -1.0] {
            let salt = 80.0 + side * 5.0;
            let out = 1.0 + body.vary(salt) * 0.18;
            let socket = body.ahead(
                maw - girth * 0.46,
                maw_up - girth * 0.14,
                maw_side + side * girth * 0.44 * out,
            );
            let knuckle = body.ahead(
                maw - girth * 0.02,
                maw_up - girth * (0.28 + WORM_PALP_SPLAY * 0.20),
                maw_side + side * girth * (0.50 + WORM_PALP_SPLAY * 0.36) * out,
            );
            let tip = body.ahead(
                maw + girth * 0.36 * palp_reach,
                maw_up - girth * (0.48 + palp_reach * 0.34),
                maw_side + side * girth * (0.32 + body.vary(salt + 3.0) * 0.14),
            );
            let (at, rotation, length) = shaft(socket, knuckle);
            self.push_oriented(
                at,
                [girth * 0.17, length * 1.14, girth * 0.17],
                shaded(chitin, 1.14),
                rotation,
                0.0,
                Shape::Cylinder,
            );
            let (at, rotation, length) = shaft(knuckle, tip);
            self.push_oriented(
                at,
                [girth * 0.13, length * 1.06, girth * 0.13],
                shaded(chitin, 0.84),
                rotation,
                0.0,
                Shape::Cone,
            );
        }
        // Pit eyes set into the crown — the only part of the animal that glows,
        // and the one cue that says which end of a writhing tube is the front.
        let (crown_up, crown_side) = muzzle(girth * 0.40);
        for side in [1.0f32, -1.0] {
            let bead = girth * 0.14 * (1.0 + body.vary(20.0 + side) * 0.24);
            self.push_oriented(
                body.ahead(
                    reach + girth * 0.40,
                    crown_up + girth * (0.26 + body.vary(24.0 + side) * 0.06),
                    crown_side + side * girth * 0.28,
                ),
                [bead, bead * 0.84, bead * 1.10],
                [0.96, 0.74, 0.22],
                Rotation::new(head_yaw, lead_pitch, 0.0),
                0.55,
                Shape::Sphere,
            );
        }
        // A horn hinged on the last ring and kinked out of line with it, so the
        // worm ends in a point instead of stopping dead at the smallest lump.
        // No thicker than the ring it grows from, or it reads as a bolt-on.
        let (tail_side, tail_lift) = wave((rings - 1) as f32 * stretch);
        let tail_rise = tail_lift + tail_width * 0.34;
        let tail_length = girth * (0.55 + body.vary_unit(30.0) * 0.60);
        let (at, rotation, length) = shaft(
            body.ahead(-tail_back, tail_rise, tail_side),
            body.ahead(
                -tail_back - tail_length,
                tail_rise + girth * (0.06 + body.vary_unit(32.0) * 0.34),
                tail_side + body.vary(34.0) * girth * 0.22,
            ),
        );
        self.push_oriented(
            at,
            [tail_width * 0.92, length * 1.1, tail_width * 0.92],
            shaded(chitin, 0.9),
            rotation,
            0.0,
            Shape::Cone,
        );
    }

    pub(super) fn draw_troll(&mut self, body: &CreatureBody) {
        let bulk = 1.0 + body.vary(11.0) * 0.20;
        let stature = 1.0 + body.vary(12.0) * 0.15;
        let hide = body.tint([0.33, 0.42, 0.30], 13.0, 0.11);
        let dark = [hide[0] * 0.70, hide[1] * 0.68, hide[2] * 0.74];
        let lit = [clamp(hide[0], 0.0, 1.0), hide[1], hide[2]];
        let horn = [0.84, 0.80, 0.66];
        // one arm does all the work and has thickened for it, and which arm is
        // this troll's own — a pair of them must never read as one model twice
        let heavy = if body.quirk(14.0, 0.5) { 1.0f32 } else { -1.0 };

        let step = sin(body.phase * 1.6);
        // the weight goes over the planted foot: the trunk drops onto it and
        // rolls, rather than two legs swinging under a box held level
        let bob = -abs(step) * 0.9 * stature;
        let heel = step * 0.09;
        let hip = 9.6 * stature + bob;
        let chest = 12.8 * stature + bob;
        let shoulder = 15.0 * stature + bob;
        let crown = 16.4 * stature + bob;

        // gut, then a chest stood on its head so the shoulders are the widest
        // part of him and the waist the narrowest
        self.push_oriented(
            body.ahead(0.7, hip, 0.0),
            [7.8 * bulk, 7.0 * stature, 6.6 * bulk],
            lit,
            body.tilted(0.10, heel * 0.6),
            0.0,
            Shape::Boulder,
        );
        self.push_oriented(
            body.ahead(-0.4, chest, 0.0),
            [8.8 * bulk, 7.0 * stature, 5.8 * bulk],
            lit,
            upended(body.facing, 0.16, heel),
            0.0,
            Shape::Frustum,
        );
        self.push_oriented(
            body.ahead(2.6, crown, 0.0),
            [4.6 * bulk, 4.2 * stature, 5.0 * bulk],
            lit,
            Rotation::new(body.facing + heel * 0.5, 0.18, heel * 1.4),
            0.0,
            Shape::Boulder,
        );
        if !body.detail.at_least(BodyDetail::Reduced) {
            // enough to read as two legs under a mass, and nothing else
            for side in [1.0f32, -1.0] {
                self.push_oriented(
                    body.ahead(step * side * 1.4, 4.6 * stature + bob, side * 2.9 * bulk),
                    [3.4 * bulk, 10.0 * stature, 3.6 * bulk],
                    dark,
                    upended(body.facing, step * side * 0.2, side * 0.12),
                    0.0,
                    Shape::Frustum,
                );
            }
            return;
        }

        // a slumped neck, so the head hangs forward of the shoulders
        self.push_oriented(
            body.ahead(1.2, shoulder + 0.6 * stature, 0.0),
            [3.3 * bulk, 3.8 * stature, 3.3 * bulk],
            dark,
            body.tilted(0.62, heel * 0.5),
            0.0,
            Shape::Cylinder,
        );
        // the brow: ridged across the face, and overhanging it
        self.push_oriented(
            body.ahead(3.7, crown + 1.1 * stature, 0.0),
            [1.9, 1.7 * stature, 5.0 * bulk],
            dark,
            Rotation::new(
                body.facing + core::f32::consts::FRAC_PI_2,
                heel * 0.8,
                0.42 + body.vary(15.0) * 0.14,
            ),
            0.0,
            Shape::Wedge,
        );
        let tusks = if body.detail.at_least(BodyDetail::Full) {
            2 + (body.vary_unit(16.0) * 2.99) as usize
        } else {
            2
        };
        // one tusk in three is a stump, and it is never the same one twice
        let stump = if body.quirk(17.0, 0.35) {
            (body.vary_unit(18.0) * (tusks as f32 - 0.01)) as usize
        } else {
            tusks
        };
        for tusk in 0..tusks {
            let flank = if tusk % 2 == 0 { 1.0f32 } else { -1.0 };
            let rank = (tusk / 2) as f32;
            let length = if tusk == stump {
                1.4 * stature
            } else {
                (3.2 - rank * 0.8) * stature * (0.7 + body.vary_unit(70.0 + tusk as f32) * 0.7)
            };
            self.push_oriented(
                body.ahead(
                    3.4 - rank * 0.9,
                    crown - 1.6 * stature + length * 0.4,
                    flank * (1.3 + rank * 0.7) * bulk,
                ),
                [length * 0.36, length, length * 0.36],
                horn,
                body.tilted(-0.34 - rank * 0.1, -flank * (0.24 + rank * 0.12)),
                0.0,
                Shape::Cone,
            );
        }

        for side in [1.0f32, -1.0] {
            let swing = step * side;
            let thick = bulk * (1.0 + body.vary(50.0 + side) * 0.08);
            // thigh thickest at the hip, splayed out, and pitched by the walk
            self.push_oriented(
                body.ahead(swing * 1.3, 7.2 * stature + bob, side * 2.7 * bulk),
                [3.6 * thick, 6.4 * stature, 3.9 * thick],
                dark,
                upended(body.facing, swing * 0.30, side * 0.15 + heel),
                0.0,
                Shape::Frustum,
            );
            self.push_oriented(
                body.ahead(swing * 2.2, (TROLL_KNEE - 2.2) * stature + bob, side * 3.0 * bulk),
                [2.7 * thick, 5.2 * stature, 2.8 * thick],
                [dark[0] * 0.9, dark[1] * 0.9, dark[2] * 0.9],
                body.tilted(-swing * 0.24, -side * 0.12),
                0.0,
                Shape::Cylinder,
            );
            if body.detail.at_least(BodyDetail::Full) {
                // turned out: a foot square to the world reads as a plinth
                self.push_oriented(
                    body.ahead(swing * 2.9, 1.1 * stature + bob * 0.35, side * 3.2 * bulk),
                    [3.3 * thick, 2.0, 4.8 * thick],
                    dark,
                    Rotation::new(body.facing + side * 0.26, 0.06, side * 0.08),
                    0.0,
                    Shape::Boulder,
                );
                for toe in [0.9f32, -0.9] {
                    self.push_oriented(
                        body.ahead(
                            swing * 2.9 + 2.3,
                            1.0 * stature + bob * 0.35,
                            side * 3.2 * bulk + toe,
                        ),
                        [0.7, 1.6, 0.7],
                        horn,
                        Rotation::new(body.facing + side * 0.26, 1.72, 0.0),
                        0.0,
                        Shape::Cone,
                    );
                }
            }
        }

        for side in [1.0f32, -1.0] {
            let swing = -step * side;
            let heft = (if side * heavy > 0.0 { 1.18 } else { 0.94 })
                * (1.0 + body.vary(54.0 + side) * 0.07);
            self.push_oriented(
                body.ahead(swing * 1.0, chest + 0.2 * stature, side * 5.3 * bulk),
                [3.3 * heft * bulk, 6.4 * stature * heft, 3.3 * heft * bulk],
                lit,
                upended(body.facing, swing * 0.28, side * 0.24),
                0.0,
                Shape::Frustum,
            );
            self.push_oriented(
                body.ahead(swing * 2.3, 7.4 * stature + bob, side * 6.3 * bulk),
                [2.7 * heft * bulk, 5.6 * stature * heft, 2.8 * heft * bulk],
                dark,
                body.tilted(swing * 0.30, side * 0.12),
                0.0,
                Shape::Cylinder,
            );
            // knuckles hang below the knee — that is the whole silhouette
            self.push_oriented(
                body.ahead(
                    swing * 2.9,
                    TROLL_KNUCKLE * stature + bob,
                    side * 6.8 * bulk,
                ),
                [3.4 * heft, 3.2 * heft, 3.6 * heft],
                lit,
                Rotation::new(body.facing + side * 0.2, 0.2, side * 0.3),
                0.0,
                Shape::Boulder,
            );
            if body.detail.at_least(BodyDetail::Full) {
                for claw in [0.8f32, -0.8] {
                    self.push_oriented(
                        body.ahead(
                            swing * 2.9 + 1.4,
                            (TROLL_KNUCKLE - 1.2) * stature + bob,
                            side * 6.8 * bulk + claw,
                        ),
                        [0.6, 1.9 * heft, 0.6],
                        horn,
                        body.tilted(2.5, side * 0.2),
                        0.0,
                        Shape::Cone,
                    );
                }
            }
        }
        if !body.detail.at_least(BodyDetail::Full) {
            return;
        }

        // a humped back, set off-centre, and two mismatched shoulder caps
        self.push_oriented(
            body.ahead(-2.4, shoulder - 0.4 * stature, 0.9),
            [6.2 * bulk, 4.6 * stature, 5.2 * bulk],
            dark,
            Rotation::new(body.facing + 0.3, -0.22, heel),
            0.0,
            Shape::Boulder,
        );
        for side in [1.0f32, -1.0] {
            let cap = 4.0 * bulk * (1.0 + body.vary(58.0 + side) * 0.12);
            self.push_oriented(
                body.ahead(-0.4, shoulder + 0.3 * stature, side * 4.4 * bulk),
                [cap, cap * 0.86, cap * 0.94],
                lit,
                Rotation::new(body.facing + side * 0.4, 0.14, side * 0.2),
                0.0,
                Shape::Boulder,
            );
        }
        // jaw, narrowing to the chin, and two eyes deep under the brow
        self.push_oriented(
            body.ahead(3.5, crown - 2.2 * stature, 0.0),
            [4.0 * bulk, 2.8 * stature, 3.8 * bulk],
            lit,
            upended(body.facing, 0.26, heel * 0.5),
            0.0,
            Shape::Frustum,
        );
        for side in [1.0f32, -1.0] {
            self.push_oriented(
                body.ahead(4.0, crown + 0.2 * stature, side * 1.4 * bulk),
                [0.8, 0.7, 0.8],
                [0.98, 0.66, 0.18],
                Rotation::facing(body.facing),
                0.55,
                Shape::Sphere,
            );
            // ears: one always a little larger and hung at its own angle
            let ear = 1.0 + body.vary(62.0 + side) * 0.3;
            self.push_oriented(
                body.ahead(1.6, crown + 0.4 * stature, side * 2.6 * bulk),
                [1.8 * ear, 1.6, 3.4 * ear],
                [lit[0] * 0.92, lit[1] * 0.9, lit[2] * 0.92],
                Rotation::new(
                    body.facing + side * core::f32::consts::FRAC_PI_2,
                    0.42 + body.vary_unit(66.0 + side) * 0.5,
                    side * 0.2,
                ),
                0.0,
                Shape::Frond,
            );
        }
        // matted tufts off the crown and the nape, each at its own bearing
        for tuft in 0..3 {
            let lean = body.vary(76.0 + tuft as f32);
            self.push_oriented(
                body.ahead(
                    0.4 - tuft as f32 * 1.1,
                    crown + (1.4 - tuft as f32 * 0.3) * stature,
                    lean * 2.0 * bulk,
                ),
                [1.5, 1.4, 3.2 + lean * 1.2],
                [dark[0] * 0.55, dark[1] * 0.5, dark[2] * 0.5],
                Rotation::new(
                    body.facing + core::f32::consts::PI + lean * 1.1,
                    -0.9 - body.vary_unit(80.0 + tuft as f32) * 0.5,
                    lean * 0.6,
                ),
                0.0,
                Shape::Frond,
            );
        }
        // scraps of hide belted at the waist, on about half of them
        if body.quirk(19.0, 0.55) {
            self.push_oriented(
                body.ahead(0.4, hip - 3.2 * stature, 0.0),
                [8.2 * bulk, 1.5, 6.9 * bulk],
                [0.42, 0.30, 0.20],
                body.tilted(0.08, heel),
                0.0,
                Shape::Cuboid,
            );
            self.push_oriented(
                body.ahead(3.4, hip - 4.6 * stature, body.vary(21.0) * 1.5),
                [3.6 * bulk, 3.2, 2.4],
                [0.38, 0.26, 0.17],
                Rotation::new(body.facing, core::f32::consts::PI + 0.2, heel * 2.0),
                0.0,
                Shape::Wedge,
            );
        }
        // boils, wherever this one happens to have them
        for boil in 0..2 {
            let across = body.vary(84.0 + boil as f32);
            let lump = 1.4 + body.vary_unit(88.0 + boil as f32) * 1.3;
            self.push_oriented(
                body.ahead(
                    -1.6 - boil as f32 * 0.8,
                    chest + across * 3.0 * stature,
                    across * 4.2 * bulk,
                ),
                [lump, lump * 0.9, lump],
                [lit[0] * 1.1, lit[1] * 0.9, lit[2] * 0.86],
                Rotation::new(body.facing + across, 0.3, across),
                0.0,
                Shape::Boulder,
            );
        }
        // a club, in the working hand, on the ones that picked one up. Its
        // butt sits in the fist and it leans back over the shoulder, so the
        // head is offset by half the shaft along the shaft's own lean.
        if body.quirk(22.0, 0.45) {
            const CLUB_LEAN: f32 = -0.62;
            let cant = heavy * 0.26;
            let half = 6.0 * stature;
            self.push_oriented(
                body.ahead(3.4, (TROLL_KNUCKLE + 3.4) * stature + bob, heavy * 7.0 * bulk),
                [1.7, half * 2.0, 1.7],
                [0.36, 0.26, 0.16],
                body.tilted(CLUB_LEAN, cant),
                0.0,
                Shape::Cylinder,
            );
            let knob_up = (TROLL_KNUCKLE + 3.4) * stature + bob + half * cos(cant) * cos(CLUB_LEAN);
            let knob_forward = 3.4 + half * cos(cant) * sin(CLUB_LEAN);
            let knob_side = heavy * 7.0 * bulk - half * sin(cant);
            self.push_oriented(
                body.ahead(knob_forward, knob_up, knob_side),
                [4.4, 4.8, 4.4],
                [0.34, 0.30, 0.26],
                Rotation::new(body.facing + heavy, CLUB_LEAN, cant),
                0.0,
                Shape::Boulder,
            );
            for spike in [1.0f32, -1.0] {
                self.push_oriented(
                    body.ahead(knob_forward + spike * 1.3, knob_up + 0.6, knob_side + spike * 1.1),
                    [0.9, 2.6, 0.9],
                    horn,
                    body.tilted(spike * 1.3, cant + spike * 0.4),
                    0.0,
                    Shape::Cone,
                );
            }
        }
    }

    /// A brood mound, not a pincushion. The mound is several boulder masses
    /// shouldered into one lopsided hill, scaled over with chitin scutes; the
    /// maw is sunk into a collar behind a ring of swollen lips rather than
    /// capped flat; egg sacs are clutched on the flanks in twos and threes and
    /// tendrils root the whole thing to the terrain. Almost nothing here is
    /// stepped evenly round a ring — bearings are drawn, and drawn bearings
    /// clump and leave gaps, which is what an even ring never does.
    pub(super) fn draw_nest(&mut self, body: &CreatureBody) {
        /// Boulder masses the mound is built from: the hill, the collar the
        /// maw opens through, then bulges leaning on the loaded flank.
        const NEST_MASSES_FULL: usize = 5;
        const NEST_MASSES_REDUCED: usize = 3;
        const NEST_MASSES_DISTANT: usize = 2;
        /// Chitin scutes laid over the mound, overlapping down the slope the
        /// way bark does, so every seam is buried under the plate above it.
        const NEST_PLATES_FULL: usize = 7;
        const NEST_PLATES_REDUCED: usize = 1;
        /// Lobes the rim is broken into. One clean cylinder round the opening
        /// reads as the mouth of a plant pot.
        const NEST_LIPS_FULL: usize = 4;
        const NEST_LIPS_REDUCED: usize = 2;
        /// Teeth stood inside the rim, canting into the throat.
        const NEST_TEETH: usize = 5;

        let scale = 1.0 + body.vary(71.0) * 0.18;
        // a squat pustule at one end of the range and a steep spire at the
        // other; heights take this and widths are left alone, so the two read
        // as the same organism grown under different pressure
        let squat = 0.80 + body.vary_unit(73.0) * 0.46;
        // The flank this one has grown out over. Masses, scutes and sacs are
        // all weighted towards it, which is what gives the mound a front and a
        // back instead of a radius.
        let heavy = body.facing + body.vary(75.0) * 3.1;

        // not every brood is purple: some have gone bilious, some the colour
        // of dried blood, and the chitin and the sacs follow the flesh rather
        // than being painted on over it
        let (flesh_base, chitin_base, sac_base) = if body.quirk(76.0, 0.26) {
            ([0.26, 0.34, 0.18], [0.15, 0.21, 0.11], [0.50, 0.60, 0.28])
        } else if body.quirk(77.0, 0.34) {
            ([0.42, 0.15, 0.19], [0.27, 0.09, 0.12], [0.66, 0.30, 0.28])
        } else {
            ([0.36, 0.17, 0.42], [0.24, 0.10, 0.28], [0.56, 0.26, 0.54])
        };
        let flesh = body.tint(flesh_base, 72.0, 0.08);
        let chitin = body.tint(chitin_base, 74.0, 0.06);
        let sac = body.tint(sac_base, 78.0, 0.09);
        let bone = body.tint([0.78, 0.72, 0.60], 79.0, 0.07);
        let lit = [clamp(flesh[0], 0.0, 1.0), flesh[1], flesh[2]];
        // whatever is down the throat lights it in the brood's own colour, so
        // a bilious nest does not glow amethyst
        let ember = [
            clamp(sac[0] * 1.6, 0.0, 1.0),
            clamp(sac[1] * 1.4, 0.0, 1.0),
            clamp(sac[2] * 1.7, 0.0, 1.0),
        ];
        // Two pieces of one creature in the same flat colour read as one
        // moulded object; a shade between them reads as two parts.
        let shade = |base: [f32; 3], amount: f32| {
            [base[0] * amount, base[1] * amount, base[2] * amount]
        };
        // It breathes, and never evenly: every mass runs the one phase at its
        // own offset, so there is always a flank leading and a flank lagging.
        let breath = |offset: f32, depth: f32| 1.0 + sin(body.phase * 0.7 + offset) * depth;
        // Unit direction of a stalk — spine, tooth, tendril — leaning `lay`
        // radians off vertical along `bearing`, with `pitch` swinging it
        // sideways of that radius. Rooting a cone by its base and stepping
        // half its length along this is the difference between a spine growing
        // out of the mound and one hovering beside it. It matches exactly what
        // `Rotation::new(-bearing, pitch, -lay)` does to the mesh's own +Y.
        let stalk = |bearing: f32, lay: f32, pitch: f32| -> [f32; 3] {
            let out = sin(lay);
            let rise = cos(lay);
            [
                cos(bearing) * out - sin(bearing) * rise * sin(pitch),
                rise * cos(pitch),
                sin(bearing) * out + cos(bearing) * rise * sin(pitch),
            ]
        };

        // ---- the mound ------------------------------------------------------
        let masses = match body.detail {
            BodyDetail::Full => NEST_MASSES_FULL,
            BodyDetail::Reduced => NEST_MASSES_REDUCED,
            BodyDetail::Distant => NEST_MASSES_DISTANT,
        };
        // the hill itself, sunk to the shoulders and sitting off square
        let heave = breath(0.0, 0.05);
        self.push_oriented(
            [
                body.x + cos(heavy) * 1.6 * scale,
                body.y - 5.6 * scale,
                body.z + sin(heavy) * 1.6 * scale,
            ],
            [
                21.0 * scale * heave,
                15.0 * scale * squat,
                20.0 * scale / heave,
            ],
            shade(lit, 0.94),
            Rotation::new(-heavy, 0.06, body.vary(81.0) * 0.16),
            0.0,
            Shape::Boulder,
        );
        // The collar the maw opens through, offset so the crown never stands
        // centred over the hill it grew out of. Everything about the mouth is
        // measured off these, so a stubby nest's maw stays on its own crown.
        let collar_ragged = 0.84 + body.vary_unit(82.0) * 0.34;
        let collar_wide = 13.5 * scale * collar_ragged;
        let collar_tall = 15.0 * scale * squat * collar_ragged;
        let collar_bearing = heavy + body.vary(83.0) * 2.4;
        let collar_out = (0.6 + body.vary_unit(83.4) * 2.4) * scale;
        let collar_y = body.y + 0.4 * scale;
        // short of half the collar's height, so the mouth is sunk into it
        let rim = collar_y + collar_tall * 0.46;
        let lead = breath(1.7, 0.06);
        self.push_oriented(
            [
                body.x + cos(collar_bearing) * collar_out,
                collar_y,
                body.z + sin(collar_bearing) * collar_out,
            ],
            [collar_wide * lead, collar_tall, collar_wide * 0.92 / lead],
            shade(lit, 1.08),
            Rotation::new(-collar_bearing, body.vary(83.8) * 0.18, body.vary(84.2) * 0.24),
            0.0,
            Shape::Boulder,
        );
        for flank in 0..masses - 2 {
            let salt = 86.0 + flank as f32 * 2.0;
            // shouldered onto the loaded side rather than spaced round it, and
            // fatter the nearer they sit to it
            let bearing = heavy + flank as f32 * 2.27 + body.vary(salt) * 1.1;
            let loaded = cos(bearing - heavy);
            let ragged = 0.72 + body.vary_unit(salt + 0.2) * 0.66;
            let wide = (9.5 + loaded * 4.2) * scale * ragged;
            let out = (5.6 + loaded * 2.6) * scale;
            let puff = breath(flank as f32 * 1.9 + 0.9, 0.08);
            self.push_oriented(
                [
                    body.x + cos(bearing) * out,
                    body.y + (-3.2 + body.vary(salt + 0.4) * 3.4) * scale,
                    body.z + sin(bearing) * out,
                ],
                [wide * puff, wide * 0.82 * squat, wide * 0.94 / puff],
                shade(lit, 0.82 + body.vary_unit(salt + 0.6) * 0.34),
                Rotation::new(-bearing, body.vary(salt + 0.8) * 0.3, body.vary(salt + 1.0) * 0.36),
                0.0,
                Shape::Boulder,
            );
        }

        // The light welling out of the throat, the one thing here worth its
        // glow at range. It sits just clear of the dark funnel's mouth: sunk
        // any further and the funnel's own cap hides it from above, which is
        // the one angle a nest is usually seen from.
        self.push_instance(
            [body.x, rim + 0.6 * scale, body.z],
            [2.6 * scale, 2.2 * scale, 2.6 * scale],
            ember,
            body.phase * 0.1,
            1.0,
            Shape::Sphere,
        );

        // ---- spines ---------------------------------------------------------
        // Each nest sheds a few of its own, so the count differs between two
        // stood side by side.
        let spines = match body.detail {
            BodyDetail::Full => NEST_SPINES_FULL - (body.vary_unit(92.0) * 2.99) as usize,
            BodyDetail::Reduced => NEST_SPINES_REDUCED,
            BodyDetail::Distant => NEST_SPINES_DISTANT,
        };
        for spine in 0..spines {
            let salt = 130.0 + spine as f32 * 2.0;
            // three ridges of spines rather than one ring: each spine picks a
            // ridge and crowds it, leaving the mound bald down the other side
            let ridge = heavy + (spine % 3) as f32 * 2.1 + body.vary(94.0 + (spine % 3) as f32) * 1.3;
            let bearing = ridge + body.vary(salt) * 0.6;
            // low on the flank the surface is steep and the spine lies out
            // along it; up at the collar it stands almost straight
            // the radius closes as the spine climbs, so its base stays buried
            // in the flank it grew out of instead of hanging off the side
            let climb = body.vary_unit(salt + 0.4);
            let root_r = (10.2 - climb * 6.4) * scale;
            let root = [
                body.x + cos(bearing) * root_r,
                body.y + (-1.0 + climb * 7.0) * scale,
                body.z + sin(bearing) * root_r,
            ];
            let lay = 0.26 + (1.0 - climb) * 0.78 + body.vary_unit(salt + 0.8) * 0.5;
            let pitch = body.vary(salt + 1.0) * 0.55;
            // a fifth of them have been broken off, and a stub is blunt
            let snapped = body.quirk(salt + 1.4, 0.2);
            let reach = (5.0 + body.vary_unit(salt + 1.2) * 9.0)
                * scale
                * if snapped { 0.38 } else { 1.0 };
            let thick = reach * (0.13 + body.vary_unit(salt + 1.6) * 0.17)
                * if snapped { 1.9 } else { 1.0 };
            let axis = stalk(bearing, lay, pitch);
            self.push_oriented(
                [
                    root[0] + axis[0] * reach * 0.5,
                    root[1] + axis[1] * reach * 0.5,
                    root[2] + axis[2] * reach * 0.5,
                ],
                [thick, reach, thick * 0.86],
                // the break shows pale where the shell is worn through
                shade(chitin, if snapped { 1.35 } else { 0.72 + body.vary_unit(salt + 1.8) * 0.5 }),
                Rotation::new(-bearing, pitch, -lay),
                0.0,
                if snapped { Shape::Frustum } else { Shape::Cone },
            );
        }

        if !body.detail.at_least(BodyDetail::Reduced) {
            return;
        }

        // ---- the maw --------------------------------------------------------
        // roughly a third of them are clenched shut, lips pressed over the
        // throat and the teeth crossed behind them
        let open = body.quirk(156.0, 0.66);
        let gape = if open { 1.0 } else { 0.58 };
        // A throat that recedes: an inverted cone whose wide end sits at the
        // rim and whose apex is well down inside the collar. It carries almost
        // no glow of its own — the dark is the whole point of it.
        self.push_oriented(
            [body.x, rim - 4.0 * scale, body.z],
            [
                collar_wide * 0.60 * gape,
                7.6 * scale,
                collar_wide * 0.56 * gape,
            ],
            shade(chitin, 0.30),
            Rotation::new(body.facing, core::f32::consts::PI, body.vary(158.0) * 0.14),
            0.04,
            Shape::Cone,
        );
        let lips = if body.detail.at_least(BodyDetail::Full) {
            NEST_LIPS_FULL
        } else {
            NEST_LIPS_REDUCED
        };
        for lip in 0..lips {
            let salt = 160.0 + lip as f32 * 2.0;
            // stepped by less than a quarter turn and jittered on top, so the
            // lobes crowd round three sides and leave the fourth torn open
            let bearing = body.facing + lip as f32 * 1.7 + body.vary(salt) * 0.8;
            let girth = collar_wide * (0.30 + body.vary_unit(salt + 0.2) * 0.26);
            let out = collar_wide * 0.30 * gape;
            let pulse = breath(lip as f32 * 1.4 + 0.6, 0.07);
            self.push_oriented(
                [
                    body.x + cos(bearing) * out,
                    rim - 0.4 * scale,
                    body.z + sin(bearing) * out,
                ],
                [girth * pulse, girth * 0.62 * squat, girth * 0.86 / pulse],
                shade(lit, 1.02 + body.vary_unit(salt + 0.4) * 0.28),
                Rotation::new(
                    -bearing,
                    body.vary(salt + 0.6) * 0.3,
                    0.3 + body.vary_unit(salt + 0.8) * 0.38,
                ),
                0.0,
                Shape::Boulder,
            );
        }

        // ---- chitin scutes --------------------------------------------------
        let plates = if body.detail.at_least(BodyDetail::Full) {
            NEST_PLATES_FULL
        } else {
            NEST_PLATES_REDUCED
        };
        for plate in 0..plates {
            let salt = 100.0 + plate as f32 * 2.0;
            // two draws summed, so the scutes pile up on the loaded flank and
            // thin out round the back rather than tiling the whole mound
            let bearing = heavy + body.vary(salt) * 2.4 + body.vary(salt + 0.2) * 0.9;
            let climb = body.vary_unit(salt + 0.4);
            let out = (10.2 - climb * 5.0) * scale;
            // yaw a quarter turn off the bearing puts the wedge's own ridge on
            // the radius, so the keel runs down the slope; pitch then tips the
            // outer edge under the plate below it
            self.push_oriented(
                [
                    body.x + cos(bearing) * out,
                    body.y + (-2.4 + climb * 7.0) * scale,
                    body.z + sin(bearing) * out,
                ],
                [
                    (5.0 + body.vary_unit(salt + 0.6) * 4.5) * scale,
                    (1.4 + body.vary_unit(salt + 1.0) * 2.0) * scale,
                    (4.6 + body.vary_unit(salt + 0.8) * 3.8) * scale,
                ],
                shade(chitin, 0.76 + body.vary_unit(salt + 1.2) * 0.6),
                Rotation::new(
                    core::f32::consts::FRAC_PI_2 - bearing,
                    0.25 + (1.0 - climb) * 1.0,
                    body.vary(salt + 1.4) * 0.5,
                ),
                0.0,
                Shape::Wedge,
            );
        }

        // ---- egg sacs -------------------------------------------------------
        let sacs = if body.detail.at_least(BodyDetail::Full) {
            5 + (body.vary_unit(170.0) * 2.99) as usize
        } else {
            2
        };
        // two clutches and the odd loner. Sacs spaced round the mound read as
        // beads threaded on a necklace; sacs crowded in threes read as spawn.
        let clutch_a = heavy + body.vary(171.0) * 1.2;
        let clutch_b = body.facing + body.vary(172.0) * 3.1;
        for egg in 0..sacs {
            let salt = 200.0 + egg as f32 * 2.0;
            let seed = if egg % 4 == 3 {
                body.facing + body.vary(salt) * 3.1
            } else if egg % 2 == 0 {
                clutch_a
            } else {
                clutch_b
            };
            let bearing = seed + body.vary(salt + 0.2) * 0.62;
            let ripe = body.vary_unit(salt + 0.4);
            // the ripest are days from splitting: swollen, lit from inside and
            // breathing twice as hard as the rest
            let bursting = ripe > 0.74;
            let pulse = breath(egg as f32 * 1.3 + 2.2, if bursting { 0.16 } else { 0.07 });
            let girth = (3.4 + ripe * 4.6) * scale * pulse * if bursting { 1.28 } else { 1.0 };
            // the higher a sac is clutched the closer in it sits, so its inner
            // third is always sunk into the flesh under it
            let climb = body.vary_unit(salt + 0.6);
            let out = (9.8 - climb * 5.0) * scale + girth * 0.28;
            self.push_oriented(
                [
                    body.x + cos(bearing) * out,
                    body.y + (-1.6 + climb * 7.0) * scale,
                    body.z + sin(bearing) * out,
                ],
                [girth, girth * (0.78 + ripe * 0.28), girth * 0.92],
                if bursting {
                    [
                        min(sac[0] + 0.26, 1.0),
                        min(sac[1] + 0.12, 1.0),
                        min(sac[2] + 0.24, 1.0),
                    ]
                } else {
                    shade(sac, 0.78 + ripe * 0.42)
                },
                Rotation::new(
                    -bearing,
                    body.vary(salt + 1.0) * 0.5,
                    0.25 + body.vary(salt + 1.2) * 0.42,
                ),
                // the thin-shelled ones are lit through; the crusted ones are not
                if bursting { 0.30 } else { 0.05 + ripe * 0.15 },
                // some have hardened over into a crust and some have not
                if body.quirk(salt + 1.4, 0.34) {
                    Shape::Boulder
                } else {
                    Shape::Sphere
                },
            );
        }

        // ---- root tendrils --------------------------------------------------
        let roots = if body.detail.at_least(BodyDetail::Full) {
            4 + (body.vary_unit(220.0) * 2.99) as usize
        } else {
            2
        };
        // how many of them have split, on the nests that have been here longest
        let forks = if body.detail.at_least(BodyDetail::Full) {
            (body.vary_unit(221.0) * 3.99) as usize
        } else {
            0
        };
        // a fresh nest has barely gripped; an old one has run halfway to the dunes
        let spread = 0.7 + body.vary_unit(238.0) * 0.7;
        for root in 0..roots {
            let salt = 240.0 + root as f32 * 3.0;
            let bearing = body.facing + root as f32 * 2.4 + body.vary(salt) * 1.1;
            let reach = (9.0 + body.vary_unit(salt + 0.2) * 11.0) * scale * spread;
            let thick = reach * (0.10 + body.vary_unit(salt + 0.4) * 0.09);
            // Past the horizontal, so the tip creeps down onto the terrain
            // rather than sticking out into the air — but aimed at a fixed
            // depth rather than a fixed angle, or a long tendril at the same
            // lay as a short one buries most of its length in the sand.
            let dip = (1.6 + body.vary_unit(salt + 0.6) * 3.4) * scale;
            let lay = core::f32::consts::FRAC_PI_2 + dip / reach;
            let pitch = body.vary(salt + 0.8) * 0.45;
            let axis = stalk(bearing, lay, pitch);
            // rooted at the waterline of the mound, not out on the sand beside
            // it, so the tendril reads as leaving the body rather than lying
            // near it
            let anchor = [
                body.x + cos(bearing) * 7.6 * scale,
                body.y + (0.2 + body.vary(salt + 1.0) * 1.4) * scale,
                body.z + sin(bearing) * 7.6 * scale,
            ];
            self.push_oriented(
                [
                    anchor[0] + axis[0] * reach * 0.5,
                    anchor[1] + axis[1] * reach * 0.5,
                    anchor[2] + axis[2] * reach * 0.5,
                ],
                [thick, reach, thick * 0.78],
                shade(chitin, 0.6 + body.vary_unit(salt + 1.8) * 0.44),
                Rotation::new(-bearing, pitch, -lay),
                0.0,
                Shape::Cone,
            );
            if root >= forks {
                continue;
            }
            // the branch leaves the parent's own line half way out, which is
            // the only way the knuckle lands on the tendril instead of near it
            let fork_salt = salt + 1.2;
            let junction = [
                anchor[0] + axis[0] * reach * 0.55,
                anchor[1] + axis[1] * reach * 0.55,
                anchor[2] + axis[2] * reach * 0.55,
            ];
            let veer = if body.quirk(fork_salt, 0.5) { 1.0 } else { -1.0 };
            let fork_bearing = bearing + veer * (0.35 + body.vary_unit(fork_salt + 0.2) * 0.45);
            let fork_pitch = body.vary(fork_salt + 0.4) * 0.4;
            let fork_lay = lay + body.vary(fork_salt + 0.6) * 0.2;
            let fork_reach = reach * (0.42 + body.vary_unit(fork_salt + 0.8) * 0.34);
            let fork_axis = stalk(fork_bearing, fork_lay, fork_pitch);
            self.push_oriented(
                [
                    junction[0] + fork_axis[0] * fork_reach * 0.5,
                    junction[1] + fork_axis[1] * fork_reach * 0.5,
                    junction[2] + fork_axis[2] * fork_reach * 0.5,
                ],
                [thick * 0.62, fork_reach, thick * 0.5],
                shade(chitin, 0.54),
                Rotation::new(-fork_bearing, fork_pitch, -fork_lay),
                0.0,
                Shape::Cone,
            );
        }

        if !body.detail.at_least(BodyDetail::Full) {
            return;
        }

        // ---- teeth ----------------------------------------------------------
        // A negative lay leans the stalk in over the opening instead of out
        // from it, which is what makes these teeth and not more spines.
        for tooth in 0..NEST_TEETH {
            let salt = 180.0 + tooth as f32 * 2.0;
            let bearing = body.facing + tooth as f32 * 1.31 + body.vary(salt) * 0.7;
            let broken = body.quirk(salt + 0.6, 0.24);
            let length = (2.6 + body.vary_unit(salt + 0.2) * 3.4)
                * scale
                * if broken { 0.5 } else { 1.0 };
            let radius = collar_wide * 0.24 * gape + 0.4 * scale;
            let lay = -(0.45 + body.vary_unit(salt + 0.4) * 0.5 + if open { 0.0 } else { 0.38 });
            let pitch = body.vary(salt + 0.8) * 0.3;
            let axis = stalk(bearing, lay, pitch);
            let root = [
                body.x + cos(bearing) * radius,
                rim - 0.8 * scale,
                body.z + sin(bearing) * radius,
            ];
            self.push_oriented(
                [
                    root[0] + axis[0] * length * 0.5,
                    root[1] + axis[1] * length * 0.5,
                    root[2] + axis[2] * length * 0.5,
                ],
                [length * 0.26, length, length * 0.24],
                shade(bone, 0.78 + body.vary_unit(salt + 1.0) * 0.3),
                Rotation::new(-bearing, pitch, -lay),
                0.0,
                if broken { Shape::Frustum } else { Shape::Cone },
            );
        }
    }

    /// A point `forward`, `up` and `side` from `origin`, in the creature's own
    /// frame. `CreatureBody::ahead` measures from the creature's centre, which
    /// is no use once the joint being measured from has itself swung away.
    fn from_joint(
        body: &CreatureBody,
        origin: [f32; 3],
        forward: f32,
        up: f32,
        side: f32,
    ) -> [f32; 3] {
        [
            origin[0] + body.facing_sin * forward + body.facing_cos * side,
            origin[1] + up,
            origin[2] + body.facing_cos * forward - body.facing_sin * side,
        ]
    }

    /// Centre of a limb `length` long hanging from `joint` at `pitch`/`roll`.
    /// Placing a limb by its centre and then tilting it swings the hip as far
    /// as the foot and tears the socket open; pinning the top of the limb to
    /// the joint is most of what turns a stack of boxes into a leg. Pass twice
    /// the length to get the far end instead — the knee, elbow or wrist that
    /// the next piece hangs from.
    fn hung_from(
        body: &CreatureBody,
        joint: [f32; 3],
        length: f32,
        pitch: f32,
        roll: f32,
    ) -> [f32; 3] {
        let half = length * 0.5;
        let drop = cos(roll) * half;
        Self::from_joint(
            body,
            joint,
            -drop * sin(pitch),
            -drop * cos(pitch),
            sin(roll) * half,
        )
    }

    /// Townsfolk. One skeleton, but height, girth, gait, dye, skin, hair, hat,
    /// apron, footwear and load are all hashed off the villager's own slot: a
    /// keep's crowd used to be the same six boxes forty times over, which is
    /// exactly what reads as "made out of shapes".
    pub(super) fn draw_villager(&mut self, body: &CreatureBody) {
        /// What the keep's dye-vats manage. Undyed and oatmeal lead because
        /// they are much the commonest wools.
        const WOOLS: [[f32; 3]; 6] = [
            [0.56, 0.44, 0.32],
            [0.72, 0.66, 0.52],
            [0.50, 0.33, 0.28],
            [0.33, 0.39, 0.50],
            [0.60, 0.52, 0.24],
            [0.38, 0.46, 0.34],
        ];
        const SKINS: [[f32; 3]; 4] = [
            [0.90, 0.73, 0.56],
            [0.78, 0.60, 0.44],
            [0.60, 0.44, 0.31],
            [0.43, 0.30, 0.21],
        ];
        const HAIRS: [[f32; 3]; 5] = [
            [0.13, 0.11, 0.10],
            [0.29, 0.19, 0.11],
            [0.45, 0.25, 0.12],
            [0.64, 0.54, 0.32],
            [0.72, 0.71, 0.68],
        ];
        /// Radians of swing at the hip and at the shoulder, at full stride.
        const LEG_SWING: f32 = 0.46;
        const ARM_SWING: f32 = 0.34;
        /// What this one is carrying, if anything.
        const LOAD_BASKET: usize = 1;
        const LOAD_POT: usize = 2;
        const LOAD_BUNDLE: usize = 3;
        const LOAD_STAFF: usize = 4;
        /// And what they have on their head, if anything.
        const HAT_STRAW: usize = 1;
        const HAT_SCARF: usize = 2;
        const HAT_CAP: usize = 3;

        let full = body.detail.at_least(BodyDetail::Full);
        let build = 1.0 + body.vary(1.0) * 0.13;
        let girth = 1.0 + body.vary(2.0) * 0.17;
        let stride = sin(body.phase * (2.6 + body.vary(3.0) * 0.5));
        // the hips rise as the feet pass each other: twice a stride, not once
        let bob = (0.6 - abs(stride)) * 0.4 * build;
        // everyone leans into the walk, and a few of them are stooped
        let lean = 0.06 + body.vary_unit(4.0) * 0.11;
        let sway = stride * 0.05;

        let dye = WOOLS[(body.vary_unit(5.0) * WOOLS.len() as f32) as usize];
        // the two keeps grow different dye plants, so a crowd still reads as
        // one side's or the other's without every smock being one colour
        let bias = if body.faction.is_player() {
            [-0.03, 0.0, 0.10]
        } else {
            [0.10, 0.0, -0.04]
        };
        let wool = body.tint(
            [dye[0] + bias[0], dye[1] + bias[1], dye[2] + bias[2]],
            6.0,
            0.07,
        );
        let smock = [clamp(wool[0], 0.0, 1.0), wool[1], wool[2]];
        let bodice = [
            clamp(wool[0] * 0.90, 0.0, 1.0),
            wool[1] * 0.90,
            wool[2] * 0.94,
        ];
        let skin = body.tint(SKINS[(body.vary_unit(7.0) * SKINS.len() as f32) as usize], 8.0, 0.04);
        let hair =
            body.tint(HAIRS[(body.vary_unit(9.0) * HAIRS.len() as f32) as usize], 10.0, 0.05);
        let leather = body.tint([0.31, 0.22, 0.14], 11.0, 0.06);
        let linen = body.tint([0.80, 0.75, 0.64], 12.0, 0.06);
        let hose = if body.quirk(13.0, 0.30) {
            [skin[0] * 0.94, skin[1] * 0.94, skin[2] * 0.94]
        } else {
            body.tint([0.34, 0.29, 0.23], 14.0, 0.07)
        };

        let hip_y = 3.45 * build + bob;
        let waist_y = 4.20 * build + bob;
        let chest_y = 5.15 * build + bob;
        let shoulder_y = 6.05 * build + bob;
        let neck_y = 6.55 * build + bob;
        let head_y = 7.40 * build + bob;
        let head_turn = sin(body.phase * 0.7 + body.vary(15.0) * 3.0) * 0.16;
        let head_at = body.ahead(lean * 2.2, head_y, 0.0);

        // the smock: a frustum standing on its wide end, so the hem flares and
        // its narrow top vanishes inside the bodice instead of showing a seam
        self.push_oriented(
            body.ahead(lean * 0.4, hip_y + 0.15 * build, 0.0),
            [2.95 * girth, 3.60 * build, 2.30 * girth],
            smock,
            body.tilted(lean * 0.4, sway),
            0.0,
            Shape::Frustum,
        );
        self.push_oriented(
            body.ahead(lean * 1.1, chest_y, 0.0),
            [2.45 * girth, 3.10 * build, 1.80 * girth],
            bodice,
            body.tilted(lean, sway),
            0.0,
            Shape::Frustum,
        );
        self.push_oriented(
            head_at,
            [1.55 * girth, 1.80 * build, 1.60 * girth],
            skin,
            Rotation::new(body.facing + head_turn, lean * 0.5, sway * 0.5),
            0.0,
            Shape::Sphere,
        );
        // hair sits back off the brow, so the face still reads as a face
        self.push_oriented(
            Self::from_joint(body, head_at, -0.30, 0.28 * build, 0.0),
            [1.62 * girth, 1.40 * build, 1.58 * girth],
            hair,
            Rotation::new(body.facing + head_turn, 0.12, 0.0),
            0.0,
            Shape::Sphere,
        );

        if !body.detail.at_least(BodyDetail::Reduced) {
            return;
        }

        let hip_half = 0.70 * girth;
        for (index, side) in [(0usize, 1.0f32), (1, -1.0)] {
            let salt = 30.0 + index as f32 * 4.0;
            // a perfectly mirrored pair is the other half of why a figure
            // reads as manufactured, so no two legs are the same length
            let thigh_len = 1.75 * build * (1.0 + body.vary(salt) * 0.06);
            let shin_len = 1.60 * build * (1.0 + body.vary(salt + 1.0) * 0.06);
            let swing = -stride * side * LEG_SWING;
            let splay = side * (0.03 + body.vary_unit(salt + 2.0) * 0.06);
            let hip_at = body.ahead(0.0, hip_y, side * hip_half);
            let leg_len = if full { thigh_len } else { thigh_len + shin_len };
            self.push_oriented(
                Self::hung_from(body, hip_at, leg_len, swing, splay),
                [0.95 * girth, leg_len * 1.18, 1.00 * girth],
                hose,
                body.tilted(swing, splay),
                0.0,
                Shape::Cylinder,
            );
            if !full {
                continue;
            }
            let knee_at = Self::hung_from(body, hip_at, thigh_len * 2.0, swing, splay);
            // a knee only bends one way: the shin trails the thigh, never
            // leads it
            let shin_pitch = swing + max(swing, 0.0) * 0.85 + 0.07;
            self.push_oriented(
                Self::hung_from(body, knee_at, shin_len, shin_pitch, splay * 0.4),
                [0.78 * girth, shin_len * 1.22, 0.82 * girth],
                hose,
                body.tilted(shin_pitch, splay * 0.4),
                0.0,
                Shape::Cylinder,
            );
            let ankle_at = Self::hung_from(body, knee_at, shin_len * 2.0, shin_pitch, splay * 0.4);
            self.push_oriented(
                Self::from_joint(body, ankle_at, 0.30, 0.10, 0.0),
                [0.95 * girth, 0.70, 1.85],
                leather,
                Rotation::new(body.facing + side * 0.10, shin_pitch * 0.3, 0.0),
                0.0,
                Shape::Boulder,
            );
        }

        let load = (body.vary_unit(21.0) * 5.0) as usize;
        // hands that are full do not swing; they hold the load steady
        let (hold_right, hold_left) = match load {
            LOAD_BASKET => (-0.95, -0.95),
            LOAD_POT => (-0.28, 0.0),
            LOAD_BUNDLE => (0.0, -0.58),
            LOAD_STAFF => (-0.42, 0.0),
            _ => (0.0, 0.0),
        };
        let sleeved = body.quirk(16.0, 0.60);
        let sleeve = if sleeved { bodice } else { skin };
        let mut hand_at = [[0.0f32; 3]; 2];
        for (index, side) in [(0usize, 1.0f32), (1, -1.0)] {
            let salt = 40.0 + index as f32 * 5.0;
            let hold = if index == 0 { hold_right } else { hold_left };
            let swings = if abs(hold) > 0.001 { 0.22 } else { 1.0 };
            let upper_len = 1.55 * build * (1.0 + body.vary(salt) * 0.07);
            let fore_len = 1.45 * build * (1.0 + body.vary(salt + 1.0) * 0.07);
            let pitch =
                hold + stride * side * ARM_SWING * swings + body.vary(salt + 2.0) * 0.05;
            // arms hang off the shoulder, not flat against the ribs
            let roll = side * (0.13 + body.vary_unit(salt + 3.0) * 0.10);
            let shoulder_at = body.ahead(lean * 1.4, shoulder_y, side * 1.00 * girth);
            // a cap over the joint: butted, the shoulder seam shows from every
            // angle, and the shoulder is the first thing the eye checks
            self.push_oriented(
                Self::from_joint(body, shoulder_at, 0.0, 0.18 * build, side * 0.10),
                [1.25 * girth, 1.05 * build, 1.30 * girth],
                sleeve,
                body.tilted(0.0, -side * 0.35),
                0.0,
                Shape::Sphere,
            );
            let arm_len = if full { upper_len } else { upper_len + fore_len };
            self.push_oriented(
                Self::hung_from(body, shoulder_at, arm_len, pitch, roll),
                [0.72 * girth, arm_len * 1.24, 0.76 * girth],
                sleeve,
                body.tilted(pitch, roll),
                0.0,
                Shape::Cylinder,
            );
            if !full {
                hand_at[index] = Self::hung_from(body, shoulder_at, arm_len * 2.0, pitch, roll);
                continue;
            }
            let elbow_at = Self::hung_from(body, shoulder_at, upper_len * 2.0, pitch, roll);
            let fore_pitch = pitch - 0.26 - abs(hold) * 0.50;
            let fore_roll = roll * 0.45;
            self.push_oriented(
                Self::hung_from(body, elbow_at, fore_len, fore_pitch, fore_roll),
                [0.62 * girth, fore_len * 1.20, 0.66 * girth],
                skin,
                body.tilted(fore_pitch, fore_roll),
                0.0,
                Shape::Cylinder,
            );
            if sleeved {
                self.push_oriented(
                    elbow_at,
                    [0.88 * girth, 0.55 * build, 0.92 * girth],
                    linen,
                    body.tilted(fore_pitch, fore_roll),
                    0.0,
                    Shape::Cylinder,
                );
            }
            let wrist_at = Self::hung_from(body, elbow_at, fore_len * 2.0, fore_pitch, fore_roll);
            self.push_instance(
                wrist_at,
                [0.62 * girth, 0.66 * build, 0.60 * girth],
                skin,
                body.facing,
                0.0,
                Shape::Sphere,
            );
            hand_at[index] = wrist_at;
        }

        self.push_oriented(
            body.ahead(lean * 0.8, waist_y, 0.0),
            [2.55 * girth, 0.42 * build, 1.95 * girth],
            leather,
            body.tilted(lean, sway),
            0.0,
            Shape::Cylinder,
        );

        let hat = (body.vary_unit(17.0) * 4.0) as usize;
        let brow_y = head_y + 0.70 * build;
        let hat_rot = Rotation::new(
            body.facing + head_turn + body.vary(18.0) * 0.20,
            lean + body.vary(19.0) * 0.12,
            body.vary(20.0) * 0.14,
        );
        let cloth =
            body.tint(WOOLS[(body.vary_unit(22.0) * WOOLS.len() as f32) as usize], 23.0, 0.07);
        match hat {
            HAT_STRAW => {
                // a wide straw hat against the sun
                self.push_oriented(
                    body.ahead(lean * 2.4, brow_y + 0.45 * build, 0.0),
                    [2.40 * girth, 1.45 * build, 2.40 * girth],
                    body.tint([0.80, 0.72, 0.44], 24.0, 0.06),
                    hat_rot,
                    0.0,
                    Shape::Cone,
                );
                self.push_oriented(
                    body.ahead(lean * 2.4, brow_y, 0.0),
                    [3.50 * girth, 0.24, 3.35 * girth],
                    body.tint([0.72, 0.63, 0.38], 25.0, 0.06),
                    hat_rot,
                    0.0,
                    Shape::Cylinder,
                );
            }
            HAT_SCARF => {
                // a headscarf, knotted at the nape
                self.push_oriented(
                    Self::from_joint(body, head_at, -0.10, 0.30 * build, 0.0),
                    [1.78 * girth, 1.45 * build, 1.82 * girth],
                    cloth,
                    hat_rot,
                    0.0,
                    Shape::Sphere,
                );
                self.push_oriented(
                    Self::from_joint(body, head_at, -0.85, 0.10 * build, 0.0),
                    [0.75, 0.65, 0.90],
                    [cloth[0] * 0.9, cloth[1] * 0.9, cloth[2] * 0.9],
                    hat_rot,
                    0.0,
                    Shape::Boulder,
                );
            }
            HAT_CAP => {
                // a felt cap with the brim folded up at the front
                self.push_oriented(
                    body.ahead(lean * 2.4, brow_y + 0.05 * build, 0.0),
                    [1.66 * girth, 0.95 * build, 1.70 * girth],
                    cloth,
                    hat_rot,
                    0.0,
                    Shape::Cylinder,
                );
                self.push_oriented(
                    Self::from_joint(body, head_at, 0.72, 0.70 * build, 0.0),
                    [1.85 * girth, 0.45, 0.70],
                    [cloth[0] * 0.82, cloth[1] * 0.82, cloth[2] * 0.86],
                    Rotation::new(body.facing + head_turn, -0.25, 0.0),
                    0.0,
                    Shape::Wedge,
                );
            }
            _ => {}
        }

        match load {
            LOAD_BASKET => {
                let grip = [
                    (hand_at[0][0] + hand_at[1][0]) * 0.5,
                    (hand_at[0][1] + hand_at[1][1]) * 0.5,
                    (hand_at[0][2] + hand_at[1][2]) * 0.5,
                ];
                let basket_at = Self::from_joint(body, grip, 0.45, -0.30, 0.0);
                self.push_oriented(
                    basket_at,
                    [2.10 * girth, 1.55, 1.80 * girth],
                    body.tint([0.66, 0.52, 0.30], 26.0, 0.06),
                    body.tilted(0.08, sway),
                    0.0,
                    Shape::Cylinder,
                );
                if full {
                    self.push_oriented(
                        Self::from_joint(body, basket_at, 0.0, 0.72, 0.0),
                        [2.24 * girth, 0.28, 1.94 * girth],
                        body.tint([0.52, 0.40, 0.22], 27.0, 0.06),
                        body.tilted(0.08, sway),
                        0.0,
                        Shape::Cylinder,
                    );
                    self.push_oriented(
                        Self::from_joint(body, basket_at, 0.1, 0.80, -0.2),
                        [1.30, 0.85, 1.15],
                        body.tint([0.58, 0.30, 0.22], 28.0, 0.10),
                        body.tilted(0.2, 0.3),
                        0.0,
                        Shape::Boulder,
                    );
                }
            }
            LOAD_POT => {
                let pot_at = Self::from_joint(body, hand_at[0], 0.20, -0.20, 0.45);
                self.push_oriented(
                    pot_at,
                    [1.55, 1.65, 1.50],
                    body.tint([0.52, 0.34, 0.26], 26.0, 0.07),
                    body.tilted(0.06, -0.12),
                    0.0,
                    Shape::Sphere,
                );
                if full {
                    self.push_oriented(
                        Self::from_joint(body, pot_at, 0.0, 0.80, 0.06),
                        [0.85, 0.55, 0.82],
                        body.tint([0.44, 0.28, 0.21], 27.0, 0.07),
                        body.tilted(0.06, -0.12),
                        0.0,
                        Shape::Cylinder,
                    );
                }
            }
            LOAD_BUNDLE => {
                // firewood, tucked under the left arm
                let bundle_at = Self::from_joint(body, hand_at[1], -0.20, 0.55, -0.25);
                let sticks = if full { 3 } else { 1 };
                for stick in 0..sticks {
                    let skew = stick as f32 * 0.24 - 0.24;
                    self.push_oriented(
                        Self::from_joint(body, bundle_at, skew * 0.6, stick as f32 * 0.26, 0.0),
                        [0.30, 3.60 * build, 0.30],
                        body.tint([0.40, 0.30, 0.20], 26.0 + stick as f32, 0.08),
                        body.tilted(
                            core::f32::consts::FRAC_PI_2 + skew,
                            0.20 + skew * 0.5,
                        ),
                        0.0,
                        Shape::Cylinder,
                    );
                }
            }
            LOAD_STAFF => {
                let cant = -0.16 + body.vary(26.0) * 0.10;
                let tilt = body.vary(27.0) * 0.12;
                let staff_len = 7.40 * build;
                let along_f = cos(tilt) * sin(cant);
                let along_u = cos(tilt) * cos(cant);
                let along_s = -sin(tilt);
                let grip = -staff_len * 0.12;
                self.push_oriented(
                    Self::from_joint(
                        body,
                        hand_at[0],
                        along_f * grip,
                        along_u * grip,
                        along_s * grip,
                    ),
                    [0.34, staff_len, 0.34],
                    body.tint([0.42, 0.31, 0.19], 28.0, 0.07),
                    body.tilted(cant, tilt),
                    0.0,
                    Shape::Cylinder,
                );
                if full {
                    let top = staff_len * 0.38;
                    self.push_oriented(
                        Self::from_joint(
                            body,
                            hand_at[0],
                            along_f * top,
                            along_u * top,
                            along_s * top,
                        ),
                        [0.62, 0.70, 0.58],
                        body.tint([0.36, 0.27, 0.16], 29.0, 0.07),
                        body.tilted(cant, tilt),
                        0.0,
                        Shape::Boulder,
                    );
                }
            }
            _ => {}
        }

        if !full {
            return;
        }

        self.push_oriented(
            body.ahead(lean * 1.8, neck_y + 0.15 * build, 0.0),
            [0.62 * girth, 1.00 * build, 0.62 * girth],
            [skin[0] * 0.92, skin[1] * 0.92, skin[2] * 0.92],
            body.tilted(lean * 1.2, sway),
            0.0,
            Shape::Cylinder,
        );
        // a collar caps the small top face of the bodice frustum
        self.push_oriented(
            body.ahead(lean * 1.5, neck_y - 0.25 * build, 0.0),
            [1.60 * girth, 0.50 * build, 1.40 * girth],
            [wool[0] * 0.84, wool[1] * 0.84, wool[2] * 0.88],
            body.tilted(lean, sway),
            0.0,
            Shape::Cylinder,
        );
        self.push_oriented(
            Self::from_joint(body, head_at, 0.78, 0.02, 0.0),
            [0.32, 0.58, 0.32],
            [skin[0] * 0.96, skin[1] * 0.93, skin[2] * 0.90],
            Rotation::new(
                body.facing + head_turn,
                core::f32::consts::FRAC_PI_2 + 0.25,
                0.0,
            ),
            0.0,
            Shape::Cone,
        );
        for side in [1.0f32, -1.0] {
            self.push_instance(
                Self::from_joint(body, head_at, 0.66, 0.16 * build, side * 0.40),
                [0.26, 0.30, 0.26],
                [0.10, 0.09, 0.09],
                body.facing + head_turn,
                0.0,
                Shape::Sphere,
            );
        }
        if body.quirk(110.0, 0.45) {
            // long hair, fanning out over the shoulders
            self.push_oriented(
                Self::from_joint(body, head_at, -0.55, -0.55 * build, 0.0),
                [1.55 * girth, 1.90 * build, 1.15 * girth],
                [hair[0] * 0.92, hair[1] * 0.92, hair[2] * 0.92],
                Rotation::new(body.facing + head_turn, -0.12, sway),
                0.0,
                Shape::Frustum,
            );
        }
        if body.quirk(111.0, 0.28) {
            self.push_oriented(
                Self::from_joint(body, head_at, 0.62, -0.62 * build, 0.0),
                [1.15 * girth, 1.25 * build, 0.95],
                hair,
                Rotation::new(body.facing + head_turn, 0.28, 0.0),
                0.0,
                Shape::Boulder,
            );
        }
        if body.quirk(112.0, 0.45) {
            // the apron is tipped back at the top so it lies against the
            // smock rather than hovering a hand's breadth off it
            self.push_oriented(
                body.ahead(1.05 * girth, hip_y + 0.25 * build, 0.0),
                [2.15 * girth, 2.70 * build, 0.35],
                linen,
                body.tilted(-0.18, sway * 1.4),
                0.0,
                Shape::Frustum,
            );
        }
        if body.quirk(113.0, 0.40) {
            self.push_oriented(
                body.ahead(0.20, waist_y - 0.75 * build, 1.15 * girth),
                [0.85, 0.95, 0.70],
                [leather[0] * 1.1, leather[1] * 1.1, leather[2] * 1.05],
                body.tilted(0.10, -0.20),
                0.0,
                Shape::Boulder,
            );
        }
        self.push_oriented(
            body.ahead(1.00 * girth, waist_y, 0.0),
            [0.45, 0.40, 0.28],
            [0.72, 0.62, 0.34],
            body.tilted(lean, sway),
            0.0,
            Shape::Cuboid,
        );
    }

    /// Garrison troops. The same skeleton as a villager underneath, but the
    /// variation lives in the kit: helmet pattern, weapon, plume, and whether
    /// this one drew mail and plate or a leather jerkin. The livery is on the
    /// surcoat only, so what shows underneath stays honest steel.
    pub(super) fn draw_soldier(&mut self, body: &CreatureBody) {
        const SKINS: [[f32; 3]; 4] = [
            [0.88, 0.71, 0.55],
            [0.76, 0.58, 0.43],
            [0.58, 0.42, 0.30],
            [0.42, 0.29, 0.20],
        ];
        /// Household second colours: worn as a plume, painted on the shield.
        const PLUMES: [[f32; 3]; 4] = [
            [0.92, 0.90, 0.86],
            [0.86, 0.66, 0.18],
            [0.16, 0.15, 0.18],
            [0.58, 0.18, 0.20],
        ];
        /// A soldier marches. Shorter stride than a villager, and the arms are
        /// carrying weight, so they swing less.
        const LEG_SWING: f32 = 0.38;
        const ARM_SWING: f32 = 0.20;
        const HELM_KETTLE: usize = 0;
        const HELM_CRESTED: usize = 1;
        const HELM_NASAL: usize = 2;
        /// Whatever they were handed at the armoury door.
        const WEAPON_SPEAR: usize = 0;
        const WEAPON_SWORD: usize = 1;
        const WEAPON_BOW: usize = 2;

        let full = body.detail.at_least(BodyDetail::Full);
        let build = 1.0 + body.vary(51.0) * 0.11;
        let girth = 1.0 + body.vary(52.0) * 0.14;
        let stride = sin(body.phase * (2.3 + body.vary(53.0) * 0.35));
        let bob = (0.6 - abs(stride)) * 0.36 * build;
        let lean = 0.05 + body.vary_unit(54.0) * 0.07;
        let sway = stride * 0.04;

        let helm = (body.vary_unit(55.0) * 3.0) as usize;
        let weapon = (body.vary_unit(56.0) * 3.0) as usize;
        // half the garrison is armoured properly; the rest make do with
        // boiled leather, which changes the silhouette as much as the colour
        let heavy = body.quirk(57.0, 0.5);
        let livery = body.tint(
            if body.faction.is_player() {
                [0.24, 0.38, 0.72]
            } else {
                [0.64, 0.22, 0.20]
            },
            58.0,
            0.05,
        );
        let surcoat = [clamp(livery[0], 0.0, 1.0), livery[1], livery[2]];
        let plume =
            body.tint(PLUMES[(body.vary_unit(59.0) * PLUMES.len() as f32) as usize], 60.0, 0.05);
        let steel = body.tint([0.60, 0.62, 0.66], 61.0, 0.07);
        let leather = body.tint([0.30, 0.21, 0.14], 62.0, 0.06);
        let skin =
            body.tint(SKINS[(body.vary_unit(63.0) * SKINS.len() as f32) as usize], 64.0, 0.04);
        // a wash of the livery through the mail, so a rank of them still reads
        // as one side's even where no surcoat shows
        let hauberk = if heavy {
            [
                steel[0] * 0.56 + livery[0] * 0.16,
                steel[1] * 0.56 + livery[1] * 0.16,
                steel[2] * 0.56 + livery[2] * 0.16,
            ]
        } else {
            [leather[0] * 1.5, leather[1] * 1.5, leather[2] * 1.4]
        };
        let hose = body.tint([0.30, 0.26, 0.22], 65.0, 0.06);

        let hip_y = 4.30 * build + bob;
        let waist_y = 5.15 * build + bob;
        let chest_y = 6.40 * build + bob;
        let shoulder_y = 7.35 * build + bob;
        let neck_y = 7.95 * build + bob;
        let head_y = 8.80 * build + bob;
        let head_turn = sin(body.phase * 0.5 + body.vary(66.0) * 3.0) * 0.10;
        let head_at = body.ahead(lean * 1.8, head_y, 0.0);
        let helm_rot = Rotation::new(
            body.facing + head_turn,
            lean * 0.6 + body.vary(67.0) * 0.05,
            body.vary(68.0) * 0.07,
        );

        // the hauberk skirt, flaring at the hem over the thighs
        self.push_oriented(
            body.ahead(lean * 0.4, hip_y + 0.30 * build, 0.0),
            [3.35 * girth, 3.50 * build, 2.75 * girth],
            [clamp(hauberk[0], 0.0, 1.0), hauberk[1], hauberk[2]],
            body.tilted(lean * 0.4, sway),
            0.0,
            Shape::Frustum,
        );
        self.push_oriented(
            body.ahead(lean * 1.0, chest_y, 0.0),
            [3.05 * girth, 3.40 * build, 2.35 * girth],
            [clamp(hauberk[0], 0.0, 1.0), hauberk[1], hauberk[2]],
            body.tilted(lean, sway),
            0.0,
            Shape::Cylinder,
        );
        self.push_oriented(
            head_at,
            [1.75 * girth, 1.95 * build, 1.80 * girth],
            skin,
            Rotation::new(body.facing + head_turn, lean * 0.5, sway * 0.5),
            0.0,
            Shape::Sphere,
        );
        // the bowl rides high enough on the skull to leave a face below it;
        // sunk any lower it swallows the head and the soldier has no face at all
        self.push_oriented(
            Self::from_joint(body, head_at, -0.05, 0.72 * build, 0.0),
            [2.30 * girth, 1.95 * build, 2.30 * girth],
            steel,
            helm_rot,
            0.06,
            Shape::Sphere,
        );

        if !body.detail.at_least(BodyDetail::Reduced) {
            return;
        }

        for (index, side) in [(0usize, 1.0f32), (1, -1.0)] {
            let salt = 70.0 + index as f32 * 4.0;
            let thigh_len = 2.05 * build * (1.0 + body.vary(salt) * 0.05);
            let shin_len = 1.95 * build * (1.0 + body.vary(salt + 1.0) * 0.05);
            let swing = -stride * side * LEG_SWING;
            let splay = side * (0.04 + body.vary_unit(salt + 2.0) * 0.05);
            let hip_at = body.ahead(0.0, hip_y, side * 0.85 * girth);
            let leg_len = if full { thigh_len } else { thigh_len + shin_len };
            self.push_oriented(
                Self::hung_from(body, hip_at, leg_len, swing, splay),
                [1.15 * girth, leg_len * 1.16, 1.20 * girth],
                hose,
                body.tilted(swing, splay),
                0.0,
                Shape::Cylinder,
            );
            if !full {
                continue;
            }
            let knee_at = Self::hung_from(body, hip_at, thigh_len * 2.0, swing, splay);
            let shin_pitch = swing + max(swing, 0.0) * 0.80 + 0.06;
            // greaves over the shin for those who drew them, wrapped hose for
            // the rest
            self.push_oriented(
                Self::hung_from(body, knee_at, shin_len, shin_pitch, splay * 0.4),
                [0.98 * girth, shin_len * 1.20, 1.02 * girth],
                if heavy { steel } else { leather },
                body.tilted(shin_pitch, splay * 0.4),
                if heavy { 0.05 } else { 0.0 },
                Shape::Cylinder,
            );
            if heavy {
                self.push_oriented(
                    knee_at,
                    [1.15 * girth, 0.95, 1.15 * girth],
                    [steel[0] * 1.08, steel[1] * 1.08, steel[2] * 1.08],
                    body.tilted(shin_pitch * 0.5, splay),
                    0.06,
                    Shape::Sphere,
                );
            }
            let ankle_at = Self::hung_from(body, knee_at, shin_len * 2.0, shin_pitch, splay * 0.4);
            self.push_oriented(
                Self::from_joint(body, ankle_at, 0.35, 0.05, 0.0),
                [1.10 * girth, 0.85, 2.20],
                leather,
                Rotation::new(body.facing + side * 0.09, shin_pitch * 0.3, 0.0),
                0.0,
                Shape::Wedge,
            );
        }

        // both hands are busy, so the swing is small — but a soldier with no
        // arm movement at all reads as a statue being dragged along
        let (hold_right, hold_left) = match weapon {
            WEAPON_BOW => (-1.05, -1.15),
            WEAPON_SWORD => (-0.45, -0.62),
            _ => (-0.30, -0.62),
        };
        let mut hand_at = [[0.0f32; 3]; 2];
        for (index, side) in [(0usize, 1.0f32), (1, -1.0)] {
            let salt = 80.0 + index as f32 * 5.0;
            let hold = if index == 0 { hold_right } else { hold_left };
            let upper_len = 1.75 * build * (1.0 + body.vary(salt) * 0.06);
            let fore_len = 1.65 * build * (1.0 + body.vary(salt + 1.0) * 0.06);
            let pitch = hold + stride * side * ARM_SWING + body.vary(salt + 2.0) * 0.04;
            let roll = side * (0.10 + body.vary_unit(salt + 3.0) * 0.08);
            let shoulder_at = body.ahead(lean * 1.4, shoulder_y, side * 1.25 * girth);
            // a pauldron stands proud of the shoulder and sheds outward; the
            // lighter troops get a mail cap in the same place
            self.push_oriented(
                Self::from_joint(body, shoulder_at, -0.05, 0.32 * build, side * 0.28),
                if heavy {
                    [1.95 * girth, 1.05 * build, 2.15 * girth]
                } else {
                    [1.55 * girth, 1.15 * build, 1.60 * girth]
                },
                if heavy { steel } else { hauberk },
                body.tilted(0.06, -side * 0.52),
                if heavy { 0.06 } else { 0.0 },
                if heavy { Shape::Wedge } else { Shape::Sphere },
            );
            let arm_len = if full { upper_len } else { upper_len + fore_len };
            self.push_oriented(
                Self::hung_from(body, shoulder_at, arm_len, pitch, roll),
                [0.90 * girth, arm_len * 1.22, 0.94 * girth],
                hauberk,
                body.tilted(pitch, roll),
                0.0,
                Shape::Cylinder,
            );
            if !full {
                hand_at[index] = Self::hung_from(body, shoulder_at, arm_len * 2.0, pitch, roll);
                continue;
            }
            let elbow_at = Self::hung_from(body, shoulder_at, upper_len * 2.0, pitch, roll);
            let fore_pitch = pitch - 0.55 - abs(hold) * 0.30;
            let fore_roll = roll * 0.4;
            self.push_oriented(
                Self::hung_from(body, elbow_at, fore_len, fore_pitch, fore_roll),
                [0.78 * girth, fore_len * 1.20, 0.82 * girth],
                if heavy { steel } else { leather },
                body.tilted(fore_pitch, fore_roll),
                if heavy { 0.05 } else { 0.0 },
                Shape::Cylinder,
            );
            let wrist_at = Self::hung_from(body, elbow_at, fore_len * 2.0, fore_pitch, fore_roll);
            self.push_oriented(
                wrist_at,
                [0.72 * girth, 0.76 * build, 0.70 * girth],
                if heavy { steel } else { skin },
                body.tilted(fore_pitch, fore_roll),
                0.0,
                Shape::Sphere,
            );
            hand_at[index] = wrist_at;
        }

        // the surcoat: front and back panels hanging from the shoulders, so
        // the mail shows at the sides instead of the livery swallowing the
        // whole torso
        self.push_oriented(
            body.ahead(1.05 * girth, chest_y - 0.55 * build, 0.0),
            [2.10 * girth, 4.70 * build, 0.42],
            surcoat,
            body.tilted(lean * 1.4, sway),
            0.0,
            Shape::Frustum,
        );
        self.push_oriented(
            body.ahead(lean * 0.9, waist_y, 0.0),
            [3.15 * girth, 0.50 * build, 2.55 * girth],
            leather,
            body.tilted(lean, sway),
            0.0,
            Shape::Cylinder,
        );

        let cant = -0.30 + body.vary(90.0) * 0.16;
        let tilt = -0.12 + body.vary(91.0) * 0.14;
        let along_f = cos(tilt) * sin(cant);
        let along_u = cos(tilt) * cos(cant);
        let along_s = -sin(tilt);
        match weapon {
            WEAPON_SPEAR => {
                let shaft = 11.5 * build;
                let grip = shaft * 0.15;
                self.push_oriented(
                    Self::from_joint(
                        body,
                        hand_at[0],
                        along_f * grip,
                        along_u * grip,
                        along_s * grip,
                    ),
                    [0.42, shaft, 0.42],
                    body.tint([0.40, 0.29, 0.17], 92.0, 0.06),
                    body.tilted(cant, tilt),
                    0.0,
                    Shape::Cylinder,
                );
                let point = shaft * 0.62;
                self.push_oriented(
                    Self::from_joint(
                        body,
                        hand_at[0],
                        along_f * point,
                        along_u * point,
                        along_s * point,
                    ),
                    [0.80, 2.30, 0.80],
                    [steel[0] * 1.1, steel[1] * 1.1, steel[2] * 1.1],
                    body.tilted(cant, tilt),
                    0.10,
                    Shape::Cone,
                );
                if full {
                    let collar = shaft * 0.46;
                    self.push_oriented(
                        Self::from_joint(
                            body,
                            hand_at[0],
                            along_f * collar,
                            along_u * collar,
                            along_s * collar,
                        ),
                        [0.60, 0.55, 0.60],
                        leather,
                        body.tilted(cant, tilt),
                        0.0,
                        Shape::Cylinder,
                    );
                    // a pennon at the head, hanging off the shaft
                    let flag = shaft * 0.40;
                    self.push_oriented(
                        Self::from_joint(
                            body,
                            hand_at[0],
                            along_f * flag,
                            along_u * flag,
                            along_s * flag,
                        ),
                        [0.90, 0.80, 2.40],
                        plume,
                        Rotation::new(body.facing + 2.4, 0.85, 0.0),
                        0.0,
                        Shape::Frond,
                    );
                }
            }
            WEAPON_SWORD => {
                let blade = 3.60 * build;
                let reach = 0.70 + blade * 0.5;
                self.push_oriented(
                    Self::from_joint(
                        body,
                        hand_at[0],
                        sin(cant) * reach,
                        cos(cant) * reach,
                        0.0,
                    ),
                    [0.55, 0.80, blade],
                    [steel[0] * 1.15, steel[1] * 1.15, steel[2] * 1.15],
                    body.tilted(cant - core::f32::consts::FRAC_PI_2, tilt),
                    0.10,
                    Shape::Wedge,
                );
                if full {
                    self.push_oriented(
                        Self::from_joint(body, hand_at[0], sin(cant) * 0.6, cos(cant) * 0.6, 0.0),
                        [1.70, 0.30, 0.40],
                        [steel[0] * 0.8, steel[1] * 0.8, steel[2] * 0.85],
                        body.tilted(cant, tilt),
                        0.0,
                        Shape::Cuboid,
                    );
                    self.push_oriented(
                        Self::from_joint(body, hand_at[0], sin(cant) * 0.05, cos(cant) * 0.05, 0.0),
                        [0.34, 1.10, 0.34],
                        leather,
                        body.tilted(cant, tilt),
                        0.0,
                        Shape::Cylinder,
                    );
                    self.push_oriented(
                        Self::from_joint(
                            body,
                            hand_at[0],
                            -sin(cant) * 0.62,
                            -cos(cant) * 0.62,
                            0.0,
                        ),
                        [0.55, 0.55, 0.55],
                        [steel[0] * 0.9, steel[1] * 0.9, steel[2] * 0.95],
                        body.tilted(cant, tilt),
                        0.05,
                        Shape::Sphere,
                    );
                }
            }
            WEAPON_BOW => {
                let grip_at = Self::from_joint(body, hand_at[1], 0.35, 0.0, -0.15);
                let bow_tilt = 0.22 + body.vary(93.0) * 0.14;
                let wood = body.tint([0.44, 0.30, 0.18], 94.0, 0.07);
                for bend in [1.0f32, -1.0] {
                    let limb_pitch = cant * 0.3 + bend * 0.36;
                    let reach = bend * 1.90 * build;
                    self.push_oriented(
                        Self::from_joint(
                            body,
                            grip_at,
                            cos(bow_tilt) * sin(limb_pitch) * reach,
                            cos(bow_tilt) * cos(limb_pitch) * reach,
                            -sin(bow_tilt) * reach,
                        ),
                        [0.34, 3.90 * build, 0.34],
                        wood,
                        body.tilted(limb_pitch, bow_tilt),
                        0.0,
                        Shape::Cylinder,
                    );
                }
                if full {
                    self.push_oriented(
                        grip_at,
                        [0.50, 1.20, 0.50],
                        leather,
                        body.tilted(cant * 0.3, bow_tilt),
                        0.0,
                        Shape::Cylinder,
                    );
                    // a quiver slung across the back, over the far shoulder
                    let sling = body.ahead(-1.10, shoulder_y - 0.30 * build, -0.70 * girth);
                    self.push_oriented(
                        Self::hung_from(body, sling, 3.40 * build, 0.42, -0.34),
                        [1.05, 3.40 * build, 1.05],
                        leather,
                        body.tilted(0.42, -0.34),
                        0.0,
                        Shape::Cylinder,
                    );
                    self.push_oriented(
                        Self::from_joint(body, sling, 0.30, 0.75, -0.20),
                        [0.75, 1.60, 0.75],
                        plume,
                        body.tilted(0.42, -0.34),
                        0.0,
                        Shape::Cone,
                    );
                }
            }
            _ => {}
        }

        if weapon != WEAPON_BOW {
            // Held clear of the body and angled: pressed flat against the
            // surcoat a shield just widens the torso into a slab.
            let shield_roll = 1.25 + body.vary(95.0) * 0.18;
            let shield_yaw = 0.42 + body.vary(96.0) * 0.20;
            let face = 4.40 * build + body.vary(97.0) * 0.4;
            // outward normal of the disc, in the soldier's own frame
            let out_f = sin(shield_roll) * sin(shield_yaw);
            let out_u = cos(shield_roll);
            let out_s = -sin(shield_roll) * cos(shield_yaw);
            let shield_at = Self::from_joint(body, hand_at[1], 0.40, 0.80 * build, -0.35);
            let shield_rot = Rotation::new(body.facing + shield_yaw, 0.0, shield_roll);
            self.push_oriented(
                shield_at,
                [face, 0.42, face * 0.95],
                [surcoat[0] * 0.88, surcoat[1] * 0.88, surcoat[2] * 0.92],
                shield_rot,
                0.0,
                Shape::Cylinder,
            );
            self.push_oriented(
                Self::from_joint(body, shield_at, out_f * 0.30, out_u * 0.30, out_s * 0.30),
                [1.15, 0.85, 1.15],
                [steel[0] * 1.12, steel[1] * 1.12, steel[2] * 1.12],
                shield_rot,
                0.08,
                Shape::Sphere,
            );
            if full {
                // thinner than the face it surrounds, so the board stands
                // proud of the iron band rather than being swallowed by it
                self.push_oriented(
                    shield_at,
                    [face * 1.10, 0.30, face * 1.05],
                    [steel[0] * 0.78, steel[1] * 0.78, steel[2] * 0.82],
                    shield_rot,
                    0.04,
                    Shape::Cylinder,
                );
                self.push_oriented(
                    Self::from_joint(body, shield_at, out_f * 0.12, out_u * 0.12, out_s * 0.12),
                    [face * 0.55, 0.60, face * 0.52],
                    plume,
                    shield_rot,
                    0.0,
                    Shape::Cylinder,
                );
            }
        }

        if !full {
            return;
        }

        self.push_oriented(
            body.ahead(-1.05 * girth, chest_y - 0.55 * build, 0.0),
            [2.00 * girth, 4.50 * build, 0.42],
            [surcoat[0] * 0.86, surcoat[1] * 0.86, surcoat[2] * 0.90],
            body.tilted(-lean * 0.8, sway),
            0.0,
            Shape::Frustum,
        );
        // the household's device, painted on the surcoat only
        self.push_oriented(
            body.ahead(1.25 * girth, chest_y + 0.20 * build, 0.0),
            [1.40 * girth, 0.90, 0.55],
            plume,
            body.tilted(lean * 1.4 - 0.35, sway),
            0.0,
            Shape::Wedge,
        );
        self.push_oriented(
            body.ahead(1.35 * girth, waist_y, 0.0),
            [0.60, 0.55, 0.35],
            [0.74, 0.66, 0.38],
            body.tilted(lean, sway),
            0.05,
            Shape::Cuboid,
        );
        // sword belt over the shoulder, which also breaks up the surcoat
        self.push_oriented(
            body.ahead(1.08 * girth, chest_y - 0.10 * build, 0.0),
            [0.65, 3.60 * build, 0.30],
            [leather[0] * 0.85, leather[1] * 0.85, leather[2] * 0.85],
            body.tilted(0.0, 0.62 + body.vary(98.0) * 0.12),
            0.0,
            Shape::Cuboid,
        );
        self.push_oriented(
            body.ahead(lean * 1.6, neck_y - 0.10 * build, 0.0),
            [1.35 * girth, 0.95 * build, 1.30 * girth],
            [steel[0] * 0.85, steel[1] * 0.85, steel[2] * 0.90],
            body.tilted(lean, sway),
            0.04,
            Shape::Cylinder,
        );
        // a shadowed band across the eyes, under whichever helm this is
        self.push_oriented(
            Self::from_joint(body, head_at, 0.80, 0.05 * build, 0.0),
            [1.55 * girth, 0.36, 0.34],
            [0.09, 0.08, 0.09],
            Rotation::new(body.facing + head_turn, 0.10, 0.0),
            0.0,
            Shape::Cuboid,
        );
        if body.quirk(99.0, 0.40) {
            self.push_oriented(
                Self::from_joint(body, head_at, 0.60, -0.70 * build, 0.0),
                [1.25 * girth, 1.30 * build, 1.00],
                body.tint([0.26, 0.19, 0.13], 100.0, 0.10),
                Rotation::new(body.facing + head_turn, 0.30, 0.0),
                0.0,
                Shape::Boulder,
            );
        }

        match helm {
            HELM_KETTLE => {
                // a broad brim, angled off the level so it is not a dinner plate
                self.push_oriented(
                    Self::from_joint(body, head_at, 0.0, 0.45 * build, 0.0),
                    [3.40 * girth, 0.34, 3.30 * girth],
                    [steel[0] * 0.92, steel[1] * 0.92, steel[2] * 0.95],
                    helm_rot,
                    0.05,
                    Shape::Cylinder,
                );
                for side in [1.0f32, -1.0] {
                    self.push_oriented(
                        Self::from_joint(body, head_at, 0.10, -0.20 * build, side * 0.88 * girth),
                        [0.32, 1.30 * build, 1.15],
                        steel,
                        Rotation::new(body.facing + head_turn, 0.05, -side * 0.30),
                        0.04,
                        Shape::Wedge,
                    );
                }
            }
            HELM_CRESTED => {
                self.push_oriented(
                    Self::from_joint(body, head_at, 0.0, 0.42 * build, 0.0),
                    [2.50 * girth, 0.40, 2.50 * girth],
                    [steel[0] * 0.86, steel[1] * 0.86, steel[2] * 0.90],
                    helm_rot,
                    0.05,
                    Shape::Cylinder,
                );
                self.push_oriented(
                    Self::from_joint(body, head_at, 0.0, 1.55 * build, 0.0),
                    [0.55, 1.20 * build, 2.60 * girth],
                    [steel[0] * 1.1, steel[1] * 1.1, steel[2] * 1.1],
                    helm_rot,
                    0.06,
                    Shape::Wedge,
                );
                // the plume streams backwards: root forward, tip trailing
                self.push_oriented(
                    Self::from_joint(body, head_at, -0.30, 1.95 * build, 0.0),
                    [1.30 * girth, 1.10 * build, 3.40 * build],
                    plume,
                    Rotation::new(
                        body.facing + core::f32::consts::PI + body.vary(101.0) * 0.2,
                        0.28,
                        0.0,
                    ),
                    0.0,
                    Shape::Frond,
                );
            }
            HELM_NASAL => {
                self.push_oriented(
                    Self::from_joint(body, head_at, 0.0, 0.40 * build, 0.0),
                    [2.60 * girth, 0.42, 2.55 * girth],
                    [steel[0] * 0.88, steel[1] * 0.88, steel[2] * 0.92],
                    helm_rot,
                    0.05,
                    Shape::Cylinder,
                );
                self.push_oriented(
                    Self::from_joint(body, head_at, 0.84, -0.05 * build, 0.0),
                    [0.30, 1.60 * build, 0.42],
                    [steel[0] * 1.05, steel[1] * 1.05, steel[2] * 1.05],
                    Rotation::new(body.facing + head_turn, 0.12, 0.0),
                    0.05,
                    Shape::Cuboid,
                );
                for side in [1.0f32, -1.0] {
                    self.push_oriented(
                        Self::from_joint(body, head_at, 0.15, -0.30 * build, side * 0.86 * girth),
                        [0.30, 1.40 * build, 1.05],
                        steel,
                        Rotation::new(body.facing + head_turn, 0.06, -side * 0.26),
                        0.04,
                        Shape::Wedge,
                    );
                }
            }
            _ => {}
        }

        if weapon != WEAPON_BOW {
            // a sidearm at the left hip, canted back out of the stride
            let hang = body.ahead(-0.30, waist_y - 0.30 * build, -1.55 * girth);
            let scabbard = 4.60 * build;
            let sheath_pitch = 0.52 + body.vary(102.0) * 0.14;
            let sheath_roll = -0.26 + body.vary(103.0) * 0.10;
            self.push_oriented(
                Self::hung_from(body, hang, scabbard, sheath_pitch, sheath_roll),
                [0.60, scabbard, 0.48],
                leather,
                body.tilted(sheath_pitch, sheath_roll),
                0.0,
                Shape::Cylinder,
            );
            self.push_oriented(
                Self::hung_from(body, hang, scabbard * 2.0, sheath_pitch, sheath_roll),
                [0.55, 0.90, 0.45],
                [steel[0] * 0.9, steel[1] * 0.9, steel[2] * 0.95],
                body.tilted(sheath_pitch, sheath_roll + core::f32::consts::PI),
                0.04,
                Shape::Cone,
            );
        }
    }

}
