//! Creatures that fly, and the wing they share.

use super::*;

/// A quarter turn. A wing, a leg or a rag has its own length running out to
/// the side rather than along the body's heading, and that is where it starts.
const QUARTER_TURN: f32 = core::f32::consts::FRAC_PI_2;

/// Where a piece's far end points once `rotation` is applied. The mesh
/// convention is root at -Z, tip at +Z, and roll turns about that axis, so it
/// does not enter here.
fn tip_direction(rotation: Rotation) -> [f32; 3] {
    let level = cos(rotation.pitch);
    [
        sin(rotation.yaw) * level,
        -sin(rotation.pitch),
        cos(rotation.yaw) * level,
    ]
}

/// Centre for a piece of length `length` whose root should land on `root`.
/// Instances are placed by their centre, so anything hinged at one end — a
/// frond on its stem, a rag on a shoulder — has to be pushed half its length
/// back out along its own axis or the hinge floats free of the joint.
fn rooted(root: [f32; 3], rotation: Rotation, length: f32) -> [f32; 3] {
    let direction = tip_direction(rotation);
    [
        root[0] + direction[0] * length * 0.5,
        root[1] + direction[1] * length * 0.5,
        root[2] + direction[2] * length * 0.5,
    ]
}

/// Centre, orientation and length for a Y-axis piece — cylinder or cone —
/// running from `from` to `to`. Limbs, bones and rigging are described by
/// their two ends, and solving for the ends is what stops a chain of parts
/// stepping down the world axes like a staircase.
fn shaft(from: [f32; 3], to: [f32; 3]) -> ([f32; 3], Rotation, f32) {
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
}

/// The same for a Z-axis piece — frond, wedge, strap — laid from `from` to
/// `to`, with `roll` turning it about its own length. Pair it with `rooted`
/// to place the piece by the end it hangs from.
fn blade(from: [f32; 3], to: [f32; 3], roll: f32) -> (Rotation, f32) {
    let delta = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let length = max(length3(delta[0], delta[1], delta[2]), 0.001);
    let flat = sqrt(length_sq2(delta[0], delta[2]));
    (
        Rotation::new(atan2(delta[0], delta[2]), atan2(-delta[1], flat), roll),
        length,
    )
}

/// `base` scaled toward black. Two pieces of one creature in the same flat
/// colour read as one moulded object; a shade between them reads as two parts.
fn shaded(base: [f32; 3], amount: f32) -> [f32; 3] {
    [base[0] * amount, base[1] * amount, base[2] * amount]
}

impl World {
    /// A wing built as a fan of panels hinged on one shoulder: the leading
    /// panel reaches furthest and carries the whole stroke, the trailing ones
    /// are shorter, sweep harder and lag behind it, so the wing has camber and
    /// washout along the span instead of see-sawing as one flat slab.
    ///
    /// The panels are `Frond`s because a wing wants exactly what that mesh
    /// already is — a tapered, cambered, double-sided sheet — and because a
    /// wing seen from underneath is the usual case, not the exception.
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
        /// Panels across the wing at each tier. One still reads as a wing at a
        /// mile; four is the fewest that shows the twist changing along it.
        const PANELS_FULL: i32 = 4;
        const PANELS_REDUCED: i32 = 2;
        /// How far the trailing panel is swept behind the leading one, about
        /// the shoulder.
        const FAN_SWEEP: f32 = 0.62;
        /// Sweep of the leading edge itself — no wing is square to the body.
        const LEAD_SWEEP: f32 = 0.17;

        // The two wings of one creature are never the same wing: a shade more
        // span on one side, a shade more twist on the other. A perfect mirror
        // is the single loudest tell that a model was generated.
        let salt = 41.0 + side * 7.0;
        let span = span * (1.0 + body.vary(salt) * 0.06);
        let chord = chord * (1.0 + body.vary(salt + 2.3) * 0.08);
        let camber = 0.26 + body.vary_unit(salt + 5.1) * 0.18;
        let membrane = body.tint(membrane, salt + 8.7, 0.05);
        let bone = body.tint(bone, salt + 11.3, 0.05);
        // The stroke as an angle about the shoulder, which is how a wing
        // actually moves. The caller still passes it as the tip's travel.
        let stroke = flap / max(span, 1.0);

        let panels = if body.detail.at_least(BodyDetail::Full) {
            PANELS_FULL
        } else if body.detail.at_least(BodyDetail::Reduced) {
            PANELS_REDUCED
        } else {
            1
        };

        let shoulder = body.ahead(chord * 0.16, 0.5, side * chord * 0.20);
        let lead = Rotation::new(
            body.facing + side * (QUARTER_TURN + LEAD_SWEEP),
            -stroke,
            -side * (0.12 - stroke * 0.30),
        );
        let lead_direction = tip_direction(lead);
        // a point a fraction of the span out along the leading edge
        let along = |reach: f32| {
            [
                shoulder[0] + lead_direction[0] * span * reach,
                shoulder[1] + lead_direction[1] * span * reach,
                shoulder[2] + lead_direction[2] * span * reach,
            ]
        };

        for panel in 0..panels {
            let fan = if panels > 1 {
                panel as f32 / (panels - 1) as f32
            } else {
                0.0
            };
            let quiver = body.vary(salt + 3.0 + panel as f32 * 4.0);
            let reach = span * (1.06 - fan * 0.34) * (1.0 + quiver * 0.07);
            // trailing panels lag the stroke, which is the whole of the
            // rippling a wing does and none of what a rigid plate does
            let lag = stroke * (1.0 - fan * 0.42);
            let rotation = Rotation::new(
                body.facing + side * (QUARTER_TURN + LEAD_SWEEP + fan * FAN_SWEEP),
                -lag + quiver * 0.05,
                -side * (0.12 + fan * 0.26 - lag * 0.30),
            );
            // roots march down and back so consecutive panels overlap rather
            // than meeting edge to edge
            let root = body.ahead(
                chord * (0.22 - fan * 0.20),
                0.5 - fan * 0.40,
                side * chord * 0.16,
            );
            self.push_oriented(
                rooted(root, rotation, reach),
                [chord * (0.68 - fan * 0.18), chord * camber, reach],
                shaded(membrane, 1.0 - fan * 0.17),
                rotation,
                0.04,
                Shape::Frond,
            );
        }

        if !body.detail.at_least(BodyDetail::Reduced) {
            return;
        }
        // humerus, out to the elbow
        let (at, rotation, length) = shaft(shoulder, along(0.44));
        self.push_oriented(
            at,
            [chord * 0.20, length, chord * 0.20],
            bone,
            rotation,
            0.0,
            Shape::Cylinder,
        );
        if !body.detail.at_least(BodyDetail::Full) {
            return;
        }
        // shoulder mass, so the wing is joined on rather than pushed through
        self.push_instance(
            shoulder,
            [chord * 0.48, chord * 0.44, chord * 0.56],
            shaded(bone, 1.14),
            body.facing,
            0.0,
            Shape::Sphere,
        );
        // elbow, deliberately fatter than either bone it joins
        self.push_instance(
            along(0.42),
            [chord * 0.25, chord * 0.25, chord * 0.25],
            shaded(bone, 1.08),
            body.facing,
            0.0,
            Shape::Sphere,
        );
        // forearm, out to the wrist
        let (at, rotation, length) = shaft(along(0.39), along(0.82));
        self.push_oriented(
            at,
            [chord * 0.14, length, chord * 0.14],
            bone,
            rotation,
            0.0,
            Shape::Cylinder,
        );

        if feathered {
            // primaries splay off the wrist, each a shade longer or shorter
            // than its neighbour so the tip is ragged rather than cut
            for feather in 0..3 {
                let spread = feather as f32 * 0.22;
                let rotation = Rotation::new(
                    body.facing + side * (QUARTER_TURN + 0.30 + spread),
                    -stroke * 0.8 + 0.10 + spread * 0.16,
                    -side * (0.30 + spread),
                );
                let length = span
                    * (0.54 - spread * 0.26)
                    * (1.0 + body.vary(salt + 17.0 + feather as f32 * 3.0) * 0.12);
                self.push_oriented(
                    rooted(along(0.76), rotation, length),
                    [chord * (0.28 - spread * 0.06), chord * 0.14, length],
                    shaded(membrane, 0.92 - spread * 0.1),
                    rotation,
                    0.0,
                    Shape::Frond,
                );
            }
        } else {
            // finger struts pinch the membrane into scallops, the way a bat's
            // do — without them the panels read as one sheet of rubber
            for finger in 0..3 {
                let fan = 0.22 + finger as f32 * 0.34;
                let rotation = Rotation::new(
                    body.facing + side * (QUARTER_TURN + LEAD_SWEEP + fan * FAN_SWEEP),
                    -stroke * (1.0 - fan * 0.42),
                    0.0,
                );
                let direction = tip_direction(rotation);
                let reach = span * (1.0 - fan * 0.30);
                let knuckle = along(0.34);
                let tip = [
                    shoulder[0] + direction[0] * reach,
                    shoulder[1] + direction[1] * reach,
                    shoulder[2] + direction[2] * reach,
                ];
                let (at, strut, length) = shaft(knuckle, tip);
                self.push_oriented(
                    at,
                    [chord * 0.09, length, chord * 0.09],
                    shaded(bone, 0.9),
                    strut,
                    0.0,
                    Shape::Cylinder,
                );
            }
            // claw at the leading tip, hooked forward
            let wingtip = along(1.0);
            let hook = [
                wingtip[0] + body.facing_sin * chord * 0.5,
                wingtip[1] - chord * 0.18,
                wingtip[2] + body.facing_cos * chord * 0.5,
            ];
            let (at, rotation, length) = shaft(along(0.96), hook);
            self.push_oriented(
                at,
                [chord * 0.15, length, chord * 0.15],
                [0.90, 0.86, 0.74],
                rotation,
                0.0,
                Shape::Cone,
            );
        }
    }

    /// Eagle in front, lion behind, and the join is the whole problem.
    ///
    /// Mass first, trim second, because the previous version had it the other
    /// way round and read as a spray of pale parts with no centre. One deep
    /// chest outweighs everything else in the silhouette, a heavy neck widens
    /// into the withers the wings hang off, and leaner hindquarters sit behind
    /// a marked fur line. Only then the plumage, and always in groups sharing
    /// one angle: single feathers sprinkled about read as noise, overlapping
    /// sets read as feathering.
    pub(super) fn draw_griffin(&mut self, body: &CreatureBody) {
        /// Plumages. One cream nudged by a tint is the same bird three times;
        /// three bases is a golden eagle, a snowy and a dark bronze.
        const PLUMAGE: [[f32; 3]; 3] = [
            [0.47, 0.34, 0.19],
            [0.89, 0.87, 0.83],
            [0.31, 0.25, 0.18],
        ];
        /// The lion half, warmer than the plumage it meets and never its shade.
        const PELT: [[f32; 3]; 3] = [
            [0.76, 0.58, 0.30],
            [0.85, 0.79, 0.65],
            [0.47, 0.36, 0.23],
        ];
        /// Bill and talons, picked apart from the plumage so a pale bird can
        /// carry a slate bill and a dark one horn yellow.
        const BILL: [[f32; 3]; 3] = [
            [0.94, 0.76, 0.20],
            [0.83, 0.80, 0.53],
            [0.37, 0.36, 0.35],
        ];
        /// Hackles down the nape and sheets over the breast. Three and two are
        /// the fewest that still overlap into a group rather than a row.
        const HACKLES: i32 = 3;
        const BREAST_SHEETS: i32 = 2;
        /// Fronds in the tail fan, at full detail.
        const TAIL_FEATHERS: i32 = 3;

        let morph = min(body.vary_unit(7.0) * 3.0, 2.0) as usize;
        let bill_morph = min(body.vary_unit(13.0) * 3.0, 2.0) as usize;
        let build = 1.0 + body.vary(3.0) * 0.15;
        // how deep through the chest this one is, quite apart from how long
        let barrel = 1.0 + body.vary(5.0) * 0.14;
        let plume = body.tint(PLUMAGE[morph], 8.0, 0.07);
        let pelt = body.tint(PELT[morph], 11.0, 0.09);
        let horn = body.tint(BILL[bill_morph], 17.0, 0.07);
        // The nape is the one place a dark eagle goes bright, so it is lifted
        // out of the plumage rather than tinted away from it — the head has to
        // stay the same bird as the body.
        let hackle = [
            min(plume[0] * 1.26 + 0.14, 1.0),
            min(plume[1] * 1.16 + 0.09, 1.0),
            min(plume[2] * 0.98 + 0.02, 1.0),
        ];
        let flash = body.hurt * 0.5;
        let lit = [min(plume[0] + body.hurt * 0.2, 1.0), plume[1], plume[2]];
        // this one holds its head a little off the line of flight, and always
        // the same little
        let peer = body.vary(23.0) * 0.30;
        let lean = body.vary(29.0) * 0.9;
        let sway = sin(body.phase * 1.3);
        let head_at = |forward: f32, up: f32, side: f32| {
            body.ahead(7.0 * build + forward, 4.5 * build + up, lean + side)
        };

        // The chest, and it has to win: breast, trunk and haunch were all one
        // size before, which is exactly why there was no telling where the
        // body stopped and the limbs began.
        self.push_instance(
            body.ahead(1.9 * build, 0.5 * build, 0.0),
            [5.2 * build * barrel, 5.8 * build, 7.8 * build],
            lit,
            body.facing,
            flash,
            Shape::Sphere,
        );
        // hindquarters: leaner than the chest, and lumpy where it is smooth,
        // which is most of what sells the two halves as two animals
        self.push_instance(
            body.ahead(-3.9 * build, -0.5 * build, 0.0),
            [4.1 * build, 4.0 * build, 6.6 * build],
            pelt,
            body.facing + body.vary(37.0) * 0.20,
            flash,
            Shape::Boulder,
        );
        // Skull as a boulder rather than a sphere: a feathered head has no
        // smooth dome on it anywhere, and this one carries the beak.
        self.push_instance(
            head_at(0.0, 0.0, 0.0),
            [3.1 * build, 3.0 * build, 3.7 * build],
            hackle,
            body.facing + peer,
            flash,
            Shape::Boulder,
        );

        let beat = sin(body.phase * 2.0) * (3.4 + body.vary_unit(17.0) * 1.5);
        // A third of them are short a handful of primaries down one side, and
        // a gappy wing is a shorter, duller wing — all draw_wing will take.
        let moult = if body.quirk(19.0, 0.34) { 1.0 } else { -1.0 };
        for side in [1.0f32, -1.0] {
            let (reach, wear) = if side == moult { (0.86, 0.84) } else { (1.0, 1.0) };
            self.draw_wing(
                body,
                side,
                2.0 + beat,
                11.2 * build * reach,
                6.4 * build,
                shaded(plume, 1.06 * wear),
                shaded(hackle, 0.86),
                true,
            );
        }

        if !body.detail.at_least(BodyDetail::Reduced) {
            return;
        }
        // Neck and withers in one piece. A frustum sat on the shoulders, wide
        // enough at the base to be the mass the wings hang off and tapering
        // into the skull: a thin neck and a separate shoulder ball is what
        // left the old head floating over a gap.
        let withers = body.ahead(1.4 * build, 1.9 * build, lean * 0.3);
        let (at, rotation, length) = shaft(withers, head_at(-1.0 * build, -1.2 * build, 0.0));
        self.push_oriented(
            at,
            [5.4 * build, length * 1.14, 4.8 * build],
            shaded(plume, 0.92),
            rotation,
            flash,
            Shape::Frustum,
        );
        // The fur line, sunk into the crease between the two masses rather
        // than painted across one of them — where the breast plumage stops
        // being feathers and the pelt starts being fur.
        let (at, rotation, _) = shaft(
            body.ahead(-2.4 * build, -0.1 * build, 0.0),
            body.ahead(-0.6 * build, 0.1 * build, 0.0),
        );
        self.push_oriented(
            at,
            [4.9 * build * barrel, 1.8 * build, 5.3 * build],
            shaded(pelt, 0.56),
            rotation,
            0.0,
            Shape::Cylinder,
        );
        // Upper mandible: a wedge is a beak already — ridged culmen, flat
        // cheeks, a cutting edge along the bottom — and pitching it hard is
        // what carries the hook out past the jaw below it.
        self.push_oriented(
            head_at(2.3 * build, -0.5 * build, 0.0),
            [1.75 * build, 2.0 * build, 4.0 * build],
            horn,
            Rotation::new(body.facing + peer, 0.40, 0.0),
            0.0,
            Shape::Wedge,
        );
        // Lower mandible, rolled over so its keel is the ridge underneath, and
        // stopping a good way short so the hook genuinely overhangs it.
        self.push_oriented(
            head_at(1.7 * build, -1.9 * build, 0.0),
            [1.3 * build, 1.05 * build, 2.8 * build],
            shaded(horn, 0.78),
            Rotation::new(body.facing + peer, 0.14, core::f32::consts::PI),
            0.0,
            Shape::Wedge,
        );
        // The tail is a fan off one root, not a rod: far off it is the middle
        // feather alone, which keeps the silhouette the same length at every
        // tier and costs one instance to do it.
        let tail_root = body.ahead(
            -6.5 * build,
            0.2 * build + sway * 0.4,
            sway * 1.1 + body.vary(31.0) * 0.6,
        );
        let fan_width = 0.26 + body.vary_unit(43.0) * 0.24;
        let tail_feathers = if body.detail.at_least(BodyDetail::Full) {
            TAIL_FEATHERS
        } else {
            1
        };
        for feather in 0..tail_feathers {
            let spread = feather as f32 - (tail_feathers - 1) as f32 * 0.5;
            let salt = 61.0 + feather as f32 * 6.0;
            let rotation = Rotation::new(
                body.facing + core::f32::consts::PI - spread * (fan_width + body.vary(salt) * 0.07),
                -0.08 + body.vary(salt + 3.0) * 0.18,
                spread * 0.46,
            );
            let length =
                (5.4 - abs(spread) * 0.7) * build * (1.0 + body.vary(salt + 5.0) * 0.09);
            self.push_oriented(
                rooted(tail_root, rotation, length),
                [3.0 * build, 1.2 * build, length],
                shaded(plume, 0.92 - abs(spread) * 0.06),
                rotation,
                0.0,
                Shape::Frond,
            );
        }

        if !body.detail.at_least(BodyDetail::Full) {
            return;
        }
        // collar, fattening the neck where it meets the shoulders
        let (at, rotation, length) =
            shaft(neck_base, body.ahead(4.4 * build, 2.4 * build, lean * 0.5));
        self.push_oriented(
            at,
            [3.2 * build, length * 1.5, 3.2 * build],
            shaded(plume, 0.84),
            rotation,
            0.0,
            Shape::Cylinder,
        );
        // breast feathering, laid on in two overlapping sheets
        for side in [1.0f32, -1.0] {
            let salt = 45.0 + side * 3.0;
            let rotation = Rotation::new(
                body.facing + side * (0.28 + body.vary(salt) * 0.10),
                1.02 + body.vary(salt + 2.0) * 0.16,
                -side * 0.38,
            );
            let length = 4.4 * build * (1.0 + body.vary(salt + 4.0) * 0.10);
            let root = body.ahead(3.3 * build, 1.1 * build, side * 1.0 * build);
            self.push_oriented(
                rooted(root, rotation, length),
                [2.7 * build, 1.0 * build, length],
                shaded(plume, 0.97),
                rotation,
                0.0,
                Shape::Frond,
            );
        }
        // the ruff: feather groups swept back over the shoulders in two tiers
        for feather in 0..RUFF_FEATHERS {
            let side = if feather % 2 == 0 { 1.0 } else { -1.0 };
            let tier = (feather / 2) as f32;
            let salt = 31.0 + feather as f32 * 5.0;
            let rotation = Rotation::new(
                body.facing + side * (1.55 + tier * 0.55 + body.vary(salt) * 0.20),
                0.30 + tier * 0.34 + body.vary(salt + 2.0) * 0.18,
                -side * (0.50 + tier * 0.30),
            );
            let length = (3.7 - tier * 0.8) * build * (1.0 + body.vary(salt + 4.0) * 0.14);
            let root = body.ahead(
                2.6 * build - tier * 1.1 * build,
                1.9 * build - tier * 0.5 * build,
                side * 0.9 * build,
            );
            self.push_oriented(
                rooted(root, rotation, length),
                [2.0 * build, 0.9 * build, length],
                shaded(plume, 0.88 - tier * 0.06),
                rotation,
                0.0,
                Shape::Frond,
            );
        }
        // the hook, steeper than the culmen behind it
        self.push_oriented(
            head_at(3.0 * build, -1.3 * build, 0.0),
            [1.2 * build, 1.7 * build, 1.7 * build],
            shaded(horn, 0.9),
            Rotation::new(body.facing + peer, 1.05, 0.0),
            0.0,
            Shape::Wedge,
        );
        for side in [1.0f32, -1.0] {
            // ear tufts, swept back and never the same length
            let salt = 51.0 + side * 4.0;
            let base = head_at(-0.5 * build, 1.2 * build, side * 1.0 * build);
            let tip = head_at(
                -2.1 * build - body.vary_unit(salt) * 0.9,
                3.3 * build + body.vary(salt + 2.0) * 0.8,
                side * 2.2 * build,
            );
            let (at, rotation, length) = shaft(base, tip);
            self.push_oriented(
                at,
                [0.8 * build, length, 0.8 * build],
                shaded(plume, 0.72),
                rotation,
                0.0,
                Shape::Cone,
            );
        }
        // tail fan
        for feather in 0..TAIL_FEATHERS {
            let spread = feather as f32 - 1.0;
            let salt = 61.0 + feather as f32 * 6.0;
            let rotation = Rotation::new(
                body.facing + core::f32::consts::PI + spread * (0.32 + body.vary(salt) * 0.10),
                -0.16 + body.vary(salt + 3.0) * 0.22,
                spread * 0.42,
            );
            let length = (4.6 - abs(spread) * 0.8) * build;
            self.push_oriented(
                rooted(tail_tip, rotation, length),
                [2.1 * build, 1.2 * build, length],
                shaded(plume, 0.86),
                rotation,
                0.0,
                Shape::Frond,
            );
        }
        for side in [1.0f32, -1.0] {
            // lion haunch and hock, tucked up under the belly
            let salt = 83.0 + side * 5.0;
            let haunch = body.ahead(
                -3.6 * build,
                -1.4 * build,
                side * (2.0 * build + body.vary(salt) * 0.35),
            );
            self.push_instance(
                haunch,
                [2.7 * build, 3.3 * build, 3.7 * build],
                shaded(pelt, 1.05),
                body.facing + side * 0.22,
                0.0,
                Shape::Boulder,
            );
            let paw = body.ahead(
                -2.3 * build,
                -4.5 * build + body.vary(salt + 2.0) * 0.5,
                side * 1.7 * build,
            );
            let (at, rotation, length) = shaft(haunch, paw);
            self.push_oriented(
                at,
                [1.5 * build, length * 1.1, 1.5 * build],
                shaded(pelt, 0.88),
                rotation,
                0.0,
                Shape::Cylinder,
            );
        }
    }

    /// An insect, not a bee-coloured bean: a segmented abdomen banded between
    /// its segments, a waist thin enough to see daylight through, a fuzzy
    /// thorax, six legs folded under at angles no two of which agree, and four
    /// wings beating out of phase with one another.
    pub(super) fn draw_wasp(&mut self, body: &CreatureBody) {
        /// Spheres in the abdomen. Fewer reads as one bean; more and the dark
        /// rings between them crowd into a single stripe.
        const ABDOMEN_SEGMENTS: i32 = 4;
        /// Pairs of legs, front to back.
        const LEG_PAIRS: i32 = 3;
        /// Spacing of the abdomen segments before the tier stretch.
        const SEGMENT_STEP: f32 = 1.55;

        let build = 1.0 + body.vary(2.0) * 0.18;
        let coat = body.tint([0.88, 0.70, 0.14], 5.0, 0.13);
        let band = body.tint([0.10, 0.07, 0.05], 9.0, 0.04);
        let fuzz = body.tint([0.46, 0.34, 0.13], 13.0, 0.11);
        let chitin = body.tint([0.24, 0.17, 0.07], 17.0, 0.07);
        let lit = [min(coat[0] + body.hurt * 0.15, 1.0), coat[1], coat[2]];
        // the abdomen pumps as it breathes; a still insect reads as a brooch
        let pump = 1.0 + sin(body.phase * 2.4) * 0.05;
        // how far under itself this one curls its abdomen
        let curl = 0.18 + body.vary_unit(21.0) * 0.18;

        // The drop is quadratic over the abdomen and straightens out past its
        // last segment, so the sting carries on along the tangent instead of
        // being flung at the ground by a curve that was only ever meant to
        // describe four segments.
        let segment_at = |along: f32| {
            body.ahead(
                -1.5 * build - along * SEGMENT_STEP * build,
                -0.2 * build - along * min(along, ABDOMEN_SEGMENTS as f32 - 1.0) * curl * build,
                0.0,
            )
        };
        let segment_girth = |along: f32| (2.95 - along * 0.40) * build * pump;

        // Far off the whole abdomen is one stretched segment; up close it is
        // four, and the stretch keeps the silhouette the same length at both.
        let segments = if body.detail.at_least(BodyDetail::Full) {
            ABDOMEN_SEGMENTS
        } else if body.detail.at_least(BodyDetail::Reduced) {
            2
        } else {
            1
        };
        let stretch = ABDOMEN_SEGMENTS as f32 / segments as f32;
        for index in 0..segments {
            let along = index as f32 * stretch;
            let girth = segment_girth(along);
            self.push_instance(
                segment_at(along),
                [
                    girth,
                    girth * 0.94,
                    girth * 1.22 * min(stretch, 3.0),
                ],
                if index % 2 == 0 { lit } else { shaded(lit, 0.88) },
                body.facing,
                body.hurt * 0.6,
                Shape::Sphere,
            );
        }
        // thorax
        self.push_instance(
            body.ahead(1.7 * build, 0.1 * build, 0.0),
            [3.3 * build, 3.1 * build, 3.8 * build],
            fuzz,
            body.facing,
            body.hurt * 0.4,
            Shape::Sphere,
        );

        let fore = sin(body.phase * 3.1);
        let hind = sin(body.phase * 3.1 + 2.2);
        for side in [1.0f32, -1.0] {
            for (beat, reach, sweep, rank) in
                [(fore, 1.0f32, 0.42f32, 0.0f32), (hind, 0.62, 0.94, 1.0)]
            {
                if rank > 0.0 && !body.detail.at_least(BodyDetail::Reduced) {
                    continue;
                }
                let salt = 90.0 + side * 6.0 + rank * 3.0;
                let length = 7.6 * build * reach * (1.0 + body.vary(salt) * 0.09);
                let rotation = Rotation::new(
                    body.facing + side * (QUARTER_TURN + sweep + body.vary(salt + 2.0) * 0.08),
                    -beat * 0.55 - 0.10,
                    -side * (0.24 + beat * 0.48),
                );
                let root = body.ahead(
                    1.9 * build - rank * 1.2 * build,
                    1.5 * build - rank * 0.5 * build,
                    side * 0.8 * build,
                );
                self.push_oriented(
                    rooted(root, rotation, length),
                    [2.2 * build * reach, 0.9 * build, length],
                    [0.86, 0.88, 0.94],
                    rotation,
                    0.12,
                    Shape::Frond,
                );
            }
        }

        if !body.detail.at_least(BodyDetail::Reduced) {
            return;
        }
        // sting, continuing the curve the abdomen was already on
        let sting_reach = 2.4 + body.vary_unit(19.0) * 1.9;
        let (at, rotation, length) = shaft(
            segment_at(ABDOMEN_SEGMENTS as f32 - 1.0),
            segment_at(ABDOMEN_SEGMENTS as f32 - 1.0 + sting_reach),
        );
        self.push_oriented(
            at,
            [0.8 * build, length, 0.8 * build],
            shaded(chitin, 0.55),
            rotation,
            0.0,
            Shape::Cone,
        );
        self.push_instance(
            body.ahead(4.3 * build, 0.3 * build, 0.0),
            [2.6 * build, 2.5 * build, 2.1 * build],
            chitin,
            body.facing,
            0.0,
            Shape::Sphere,
        );
        for side in [1.0f32, -1.0] {
            // compound eyes, wrapped round the head rather than stuck on it
            self.push_oriented(
                body.ahead(4.4 * build, 0.5 * build, side * 1.10 * build),
                [1.5 * build, 2.0 * build, 1.9 * build],
                [0.07, 0.06, 0.05],
                Rotation::new(body.facing, 0.0, -side * 0.35),
                0.0,
                Shape::Sphere,
            );
        }
        for pair in 0..LEG_PAIRS {
            let rank = pair as f32;
            for side in [1.0f32, -1.0] {
                let salt = 60.0 + rank * 9.0 + side * 4.0;
                let out = 1.0 + body.vary(salt) * 0.18;
                let mount = 2.6 * build - rank * 1.5 * build;
                let coxa = body.ahead(mount, -1.2 * build, side * 1.1 * build);
                let knee = body.ahead(
                    mount + 0.7 * build - rank * 0.6 * build,
                    -2.9 * build * out,
                    side * 3.0 * build * out,
                );
                let (at, rotation, length) = shaft(coxa, knee);
                self.push_oriented(
                    at,
                    [0.64 * build, length * 1.1, 0.64 * build],
                    chitin,
                    rotation,
                    0.0,
                    Shape::Cylinder,
                );
                if !body.detail.at_least(BodyDetail::Full) {
                    continue;
                }
                let ankle = body.ahead(
                    mount - 1.5 * build - rank * 0.6 * build,
                    -4.5 * build * out,
                    side * 3.7 * build * out,
                );
                let toe = body.ahead(
                    mount - 2.8 * build,
                    -5.1 * build * out + body.vary(salt + 2.0) * 0.4,
                    side * 3.0 * build,
                );
                let (at, rotation, length) = shaft(knee, ankle);
                self.push_oriented(
                    at,
                    [0.48 * build, length * 1.08, 0.48 * build],
                    shaded(chitin, 0.82),
                    rotation,
                    0.0,
                    Shape::Cylinder,
                );
                let (at, rotation, length) = shaft(ankle, toe);
                self.push_oriented(
                    at,
                    [0.36 * build, length, 0.36 * build],
                    shaded(chitin, 0.6),
                    rotation,
                    0.0,
                    Shape::Cone,
                );
            }
        }

        if !body.detail.at_least(BodyDetail::Full) {
            return;
        }
        for index in 0..ABDOMEN_SEGMENTS - 1 {
            // the dark ring sits in the crease between two segments, where a
            // painted-on stripe never does
            let along = index as f32;
            let (at, rotation, _) = shaft(segment_at(along), segment_at(along + 1.0));
            let girth = segment_girth(along + 0.5) * 1.04;
            self.push_oriented(
                at,
                [girth, 0.85 * build, girth * 0.96],
                band,
                rotation,
                0.0,
                Shape::Cylinder,
            );
        }
        // the waist, thin enough to see daylight through
        let (at, rotation, length) = shaft(
            body.ahead(0.5 * build, -0.3 * build, 0.0),
            segment_at(0.0),
        );
        self.push_oriented(
            at,
            [0.85 * build, length * 1.25, 0.85 * build],
            shaded(chitin, 0.75),
            rotation,
            0.0,
            Shape::Cylinder,
        );
        // fur collar over the shoulders, paler than the thorax under it
        self.push_instance(
            body.ahead(2.8 * build, 0.4 * build, 0.0),
            [3.2 * build, 3.0 * build, 2.3 * build],
            shaded(fuzz, 1.22),
            body.facing,
            0.0,
            Shape::Sphere,
        );
        // scutellum: the hard plate between the wing roots
        self.push_oriented(
            body.ahead(0.6 * build, 1.5 * build, 0.0),
            [2.5 * build, 1.0 * build, 2.9 * build],
            shaded(chitin, 1.15),
            Rotation::new(body.facing, -0.18, 0.0),
            0.0,
            Shape::Wedge,
        );
        for side in [1.0f32, -1.0] {
            let salt = 45.0 + side * 7.0;
            // mandibles, crossed under the head
            let (at, rotation, length) = shaft(
                body.ahead(5.2 * build, -0.7 * build, side * 0.55 * build),
                body.ahead(
                    6.3 * build + body.vary(salt) * 0.4,
                    -1.7 * build,
                    side * 0.15 * build,
                ),
            );
            self.push_oriented(
                at,
                [0.7 * build, length, 0.7 * build],
                shaded(chitin, 0.7),
                rotation,
                0.0,
                Shape::Cone,
            );
            // antennae: elbowed, and each with its own kink and droop
            let base = body.ahead(5.0 * build, 1.1 * build, side * 0.6 * build);
            let elbow = body.ahead(
                6.6 * build + body.vary(salt + 2.0) * 0.6,
                2.3 * build,
                side * 1.3 * build,
            );
            let tip = body.ahead(
                7.9 * build,
                0.9 * build - body.vary_unit(salt + 4.0) * 1.2,
                side * (2.1 * build + body.vary(salt + 6.0) * 0.5),
            );
            let (at, rotation, length) = shaft(base, elbow);
            self.push_oriented(
                at,
                [0.34 * build, length, 0.34 * build],
                shaded(chitin, 0.5),
                rotation,
                0.0,
                Shape::Cylinder,
            );
            let (at, rotation, length) = shaft(elbow, tip);
            self.push_oriented(
                at,
                [0.30 * build, length, 0.30 * build],
                shaded(chitin, 0.45),
                rotation,
                0.0,
                Shape::Cone,
            );
        }
    }

    /// A hooded thing coming apart into smoke. The trick is that it is not
    /// uniformly lit: the inside of the cowl is very nearly black, and a face
    /// is the hole that darkness leaves. Everything else glows, hardest at the
    /// core and least at the rags, and the bottom frays into separate wisps
    /// rather than ending in one clean cone.
    pub(super) fn draw_wraith(&mut self, body: &CreatureBody) {
        /// Rag panels hanging off the shoulders at full detail.
        const ROBE_PANELS: i32 = 7;
        /// Wisps the lower body frays into.
        const WISPS: i32 = 6;
        /// Inside the cowl. Bright enough to be seen as a surface, dark enough
        /// that the eyes in it read as eyes.
        const HOLLOW: [f32; 3] = [0.04, 0.03, 0.07];
        /// How far back the cowl leans, which is what turns a cone into a hood
        /// with an opening rather than a party hat.
        const COWL_LEAN: f32 = -0.42;

        let build = 1.0 + body.vary(2.0) * 0.16;
        let spectre = body.tint([0.52, 0.40, 1.00], 5.0, 0.15);
        let cloth = body.tint([0.26, 0.18, 0.58], 11.0, 0.13);
        let bone = body.tint([0.86, 0.84, 0.94], 17.0, 0.05);
        let churn = sin(body.phase * 1.7);
        // where this one's rags happen to sit, so two wraiths are not one
        // costume worn twice
        let hang = body.vary(23.0);

        self.push_instance(
            [body.x, body.y, body.z],
            [4.4 * build, 5.8 * build, 4.0 * build],
            spectre,
            body.facing,
            0.72,
            Shape::Sphere,
        );
        self.push_oriented(
            [body.x, body.y + 4.2 * build, body.z],
            [5.8 * build, 5.6 * build, 5.6 * build],
            cloth,
            Rotation::new(body.facing, COWL_LEAN, churn * 0.05),
            0.16,
            Shape::Cone,
        );

        let wisps = if body.detail.at_least(BodyDetail::Full) {
            WISPS
        } else if body.detail.at_least(BodyDetail::Reduced) {
            3
        } else {
            2
        };
        for wisp in 0..wisps {
            let salt = 101.0 + wisp as f32 * 6.0;
            let bearing =
                body.facing + wisp as f32 / wisps as f32 * core::f32::consts::TAU + hang;
            let drift = sin(body.phase * 2.1 + wisp as f32 * 1.3);
            let spread = (1.4 + body.vary_unit(salt) * 2.0) * build;
            let head = [
                body.x + sin(bearing) * spread * 0.35,
                body.y - 2.4 * build,
                body.z + cos(bearing) * spread * 0.35,
            ];
            let tail = [
                body.x + sin(bearing) * spread * (1.7 + drift * 0.5),
                body.y - (7.0 + body.vary_unit(salt + 3.0) * 5.0) * build,
                body.z + cos(bearing) * spread * (1.7 + drift * 0.5),
            ];
            let (at, rotation, length) = shaft(head, tail);
            let girth = (2.2 + body.vary_unit(salt + 5.0) * 1.4) * build;
            self.push_oriented(
                at,
                [girth, length, girth],
                shaded(spectre, 0.8 + body.vary_unit(salt + 7.0) * 0.3),
                rotation,
                0.34 + body.vary_unit(salt + 9.0) * 0.30,
                Shape::Cone,
            );
        }
        // a shred peels off now and then. Driven by the phase, not the rng: a
        // draw pass that pulls on the simulation's random stream shifts every
        // roll the world makes afterwards.
        if body.detail.at_least(BodyDetail::Reduced) && sin(body.phase * 5.3) > 0.88 {
            let bearing = body.facing + body.phase * 2.7;
            self.spawn_particle(
                [
                    body.x + sin(bearing) * 3.4 * build,
                    body.y - 2.0 * build + churn * 2.2,
                    body.z + cos(bearing) * 3.4 * build,
                ],
                [0.0, 3.0 + body.vary_unit(29.0) * 4.5, 0.0],
                0.6,
                2.4,
                spectre,
                0.0,
                1.0,
            );
        }

        if !body.detail.at_least(BodyDetail::Reduced) {
            return;
        }
        // the back of the cowl, bulking out the hood behind the opening
        self.push_instance(
            body.ahead(-1.3 * build, 3.6 * build, 0.0),
            [4.8 * build, 3.8 * build, 3.8 * build],
            shaded(cloth, 0.82),
            body.facing,
            0.12,
            Shape::Sphere,
        );
        // The hollow: the one part of a wraith that is not emissive. It fills
        // the hood so that what shows through the opening is a hole.
        self.push_instance(
            body.ahead(0.9 * build, 2.3 * build, 0.0),
            [3.4 * build, 3.2 * build, 2.8 * build],
            HOLLOW,
            body.facing,
            0.0,
            Shape::Sphere,
        );
        for side in [1.0f32, -1.0] {
            // just clear of the hollow's face, so the eyes are in the opening
            // rather than buried behind it
            self.push_instance(
                body.ahead(
                    2.4 * build,
                    1.7 * build + body.vary(31.0 + side) * 0.2,
                    side * 0.8 * build,
                ),
                [0.75 * build, 0.8 * build, 0.75 * build],
                [1.0, 0.93, 0.58],
                body.facing,
                1.0,
                Shape::Sphere,
            );
        }

        let panels = if body.detail.at_least(BodyDetail::Full) {
            ROBE_PANELS
        } else {
            4
        };
        for panel in 0..panels {
            let salt = 41.0 + panel as f32 * 7.0;
            let bearing =
                body.facing + panel as f32 / panels as f32 * core::f32::consts::TAU + hang;
            let flutter = sin(body.phase * 1.6 + panel as f32 * 1.1);
            let rotation = Rotation::new(
                bearing,
                1.22 + body.vary(salt) * 0.30 + flutter * 0.16,
                body.vary(salt + 3.0) * 0.8,
            );
            let length = (7.5 + body.vary_unit(salt + 5.0) * 5.5) * build;
            let root = [
                body.x + sin(bearing) * 2.3 * build,
                body.y + 1.3 * build,
                body.z + cos(bearing) * 2.3 * build,
            ];
            self.push_oriented(
                rooted(root, rotation, length),
                [
                    (2.4 + body.vary_unit(salt + 8.0) * 1.4) * build,
                    0.9 * build,
                    length,
                ],
                shaded(cloth, 1.0 + flutter * 0.12),
                rotation,
                0.26 + body.vary_unit(salt + 11.0) * 0.16,
                Shape::Frond,
            );
        }

        for side in [1.0f32, -1.0] {
            let salt = 71.0 + side * 5.0;
            let reach = 1.0 + body.vary(salt) * 0.22;
            let shoulder = body.ahead(0.2 * build, 1.6 * build, side * 2.4 * build);
            let elbow = body.ahead(
                1.9 * build * reach,
                -0.4 * build,
                side * 4.0 * build * reach,
            );
            let wrist = body.ahead(
                3.9 * build * reach,
                0.6 * build + churn * 0.6 * side,
                side * 2.9 * build,
            );
            self.push_instance(
                wrist,
                [1.0 * build, 0.95 * build, 1.0 * build],
                bone,
                body.facing,
                0.14,
                Shape::Sphere,
            );
            if !body.detail.at_least(BodyDetail::Full) {
                continue;
            }
            // sleeve: a rag over the upper arm, hiding where it joins
            let (rotation, length) = blade(shoulder, elbow, side * 0.5);
            self.push_oriented(
                rooted(shoulder, rotation, length * 1.35),
                [2.6 * build, 0.9 * build, length * 1.35],
                shaded(cloth, 0.92),
                rotation,
                0.22,
                Shape::Frond,
            );
            let (at, rotation, length) = shaft(elbow, wrist);
            self.push_oriented(
                at,
                [0.55 * build, length * 1.1, 0.55 * build],
                bone,
                rotation,
                0.1,
                Shape::Cylinder,
            );
            for finger in 0..3 {
                let splay = finger as f32 - 1.0;
                let tip = body.ahead(
                    5.0 * build * reach + splay * 0.3 * build,
                    -0.1 * build - abs(splay) * 0.5 * build,
                    side * 2.6 * build + splay * 0.9 * build,
                );
                let (at, rotation, length) = shaft(wrist, tip);
                self.push_oriented(
                    at,
                    [0.34 * build, length, 0.34 * build],
                    bone,
                    rotation,
                    0.1,
                    Shape::Cone,
                );
            }
        }

        if !body.detail.at_least(BodyDetail::Full) {
            return;
        }
        // the cowl's rim, sat on the mouth of the cone and catching light
        // where the hollow inside it does not
        self.push_oriented(
            body.ahead(1.15 * build, 1.65 * build, 0.0),
            [5.8 * build, 0.9 * build, 5.6 * build],
            shaded(cloth, 1.35),
            Rotation::new(body.facing, COWL_LEAN, 0.0),
            0.30,
            Shape::Cylinder,
        );
        for side in [1.0f32, -1.0] {
            self.push_instance(
                body.ahead(
                    -0.2 * build,
                    1.7 * build,
                    side * (2.5 * build + body.vary(53.0 + side) * 0.3),
                ),
                [3.2 * build, 2.6 * build, 2.9 * build],
                shaded(cloth, 0.9),
                body.facing + side * 0.2,
                0.18,
                Shape::Boulder,
            );
        }
    }

    /// A sewn balloon rather than a ball on a stick. The envelope is built
    /// from vertical gores in two alternating tints: each is a `Frond`, whose
    /// own droop is what curves a straight panel out into a belly and back in
    /// at the throat — a straight piece from the crown could only ever cut a
    /// chord through the shape it is meant to describe.
    pub(super) fn draw_balloon(&mut self, body: &CreatureBody) {
        const GORES: i32 = 8;
        const RIGGING: i32 = 6;
        const BALLAST: i32 = 4;
        /// Radius the gores are rooted at. A balloon has a vent at the crown,
        /// so they ring it rather than meeting at a point — and the ring is
        /// also what gives the envelope its width.
        const CROWN_RING: f32 = 2.3;
        /// A gore's angle at the crown, its length, and how far its own droop
        /// hauls it back toward the axis. These three set the whole profile.
        /// Steep enough, and with little enough belly, that the tips gather
        /// back onto the axis. Slacker than this and the panels peel outward
        /// and the envelope reads as an open tulip rather than a closed bag.
        const GORE_PITCH: f32 = 1.12;
        const GORE_LENGTH: f32 = 9.4;
        const GORE_BELLY: f32 = 9.4;

        let build = 0.86 * (1.0 + body.vary(5.0) * 0.15);
        let (first, second) = if body.faction.is_player() {
            ([0.30, 0.55, 0.95], [0.92, 0.90, 0.82])
        } else {
            ([0.86, 0.28, 0.24], [0.94, 0.80, 0.30])
        };
        // both gore colours vary, so a fleet is not one prop stamped out
        let gore_a = body.tint(first, 3.0, 0.16);
        let gore_b = body.tint(second, 9.0, 0.16);
        let canvas = body.tint([0.80, 0.74, 0.62], 15.0, 0.08);
        let wicker = body.tint([0.52, 0.37, 0.19], 21.0, 0.10);
        let iron = [0.24, 0.21, 0.18];
        // the burner is not steady, and neither is what it throws up the throat
        let burn = 0.55 + 0.45 * sin(body.phase * 5.7);
        let sway = sin(body.phase * 0.7) * 0.35;
        let crown = [body.x, body.y + 6.0 * build, body.z];

        // The core is an opacity fill, not the silhouette: at full detail it
        // sits inside the gores so they are what the eye reads.
        // Only a shade under the gores: any more and the sky shows between
        // the panels, which is what makes a bag look like petals.
        let fill = if body.detail.at_least(BodyDetail::Full) {
            0.97
        } else {
            1.0
        };
        self.push_instance(
            [body.x, body.y + 2.0 * build, body.z],
            [8.8 * build * fill, 10.8 * build * fill, 8.8 * build * fill],
            [
                (gore_a[0] + gore_b[0]) * 0.5,
                (gore_a[1] + gore_b[1]) * 0.5,
                (gore_a[2] + gore_b[2]) * 0.5,
            ],
            body.facing,
            0.10,
            Shape::Sphere,
        );
        // the throat, gathered below the gore tips and lit from inside
        self.push_oriented(
            [body.x, body.y - 4.2 * build, body.z],
            [8.2 * build, 5.0 * build, 8.2 * build],
            [
                min(canvas[0] + burn * 0.25, 1.0),
                canvas[1] * 0.92,
                canvas[2] * 0.70,
            ],
            Rotation::new(body.facing, 0.0, core::f32::consts::PI),
            0.20 + burn * 0.16,
            Shape::Cone,
        );
        // the basket flares upward, which a frustum does once it is turned over
        self.push_oriented(
            [body.x + sway, body.y - 12.5 * build, body.z],
            [4.6 * build, 3.4 * build, 4.6 * build],
            wicker,
            Rotation::new(body.facing, 0.0, core::f32::consts::PI),
            0.0,
            Shape::Frustum,
        );

        let lines = if body.detail.at_least(BodyDetail::Full) {
            RIGGING
        } else if body.detail.at_least(BodyDetail::Reduced) {
            3
        } else {
            1
        };
        for line in 0..lines {
            let bearing = body.facing + line as f32 / lines as f32 * core::f32::consts::TAU;
            let top = [
                body.x + sin(bearing) * 1.9 * build,
                body.y - 6.6 * build,
                body.z + cos(bearing) * 1.9 * build,
            ];
            let foot = [
                body.x + sway + sin(bearing) * 2.4 * build,
                body.y - 10.8 * build,
                body.z + cos(bearing) * 2.4 * build,
            ];
            let (at, rotation, length) = shaft(top, foot);
            self.push_oriented(
                at,
                [0.26 * build, length, 0.26 * build],
                iron,
                rotation,
                0.0,
                Shape::Cylinder,
            );
        }

        if !body.detail.at_least(BodyDetail::Reduced) {
            return;
        }
        let gores = if body.detail.at_least(BodyDetail::Full) {
            GORES
        } else {
            4
        };
        for gore in 0..gores {
            let salt = 30.0 + gore as f32 * 5.0;
            let bearing = body.facing + gore as f32 / gores as f32 * core::f32::consts::TAU;
            // per-gore slack, kept small: the envelope is sewn, not draped
            let length = GORE_LENGTH * build * (1.0 + body.vary(salt) * 0.025);
            let rotation = Rotation::new(bearing, GORE_PITCH, body.vary(salt + 2.0) * 0.06);
            let root = [
                crown[0] + sin(bearing) * CROWN_RING * build,
                crown[1],
                crown[2] + cos(bearing) * CROWN_RING * build,
            ];
            self.push_oriented(
                rooted(root, rotation, length),
                [4.5 * build, GORE_BELLY * build, length],
                if gore % 2 == 0 { gore_a } else { gore_b },
                rotation,
                0.08,
                Shape::Frond,
            );
        }
        // crown cap, closing the ring the gores are hung from
        self.push_instance(
            [crown[0], crown[1] + 0.3 * build, crown[2]],
            [7.6 * build, 2.4 * build, 7.6 * build],
            canvas,
            body.facing,
            0.08,
            Shape::Sphere,
        );
        self.push_instance(
            [body.x, body.y - 6.6 * build, body.z],
            [3.8 * build, 0.7 * build, 3.8 * build],
            iron,
            body.facing,
            0.0,
            Shape::Cylinder,
        );
        self.push_instance(
            [body.x + sway, body.y - 10.8 * build, body.z],
            [5.2 * build, 0.7 * build, 5.2 * build],
            shaded(wicker, 1.32),
            body.facing,
            0.0,
            Shape::Cylinder,
        );
        // the flame itself, thrown up the throat
        self.push_instance(
            [body.x, body.y - 6.4 * build + burn * 0.6, body.z],
            [2.3 * build, (2.6 + burn * 1.6) * build, 2.3 * build],
            [1.0, 0.55, 0.12],
            body.facing,
            0.85,
            Shape::Cone,
        );
        let bags = if body.detail.at_least(BodyDetail::Full) {
            BALLAST
        } else {
            2
        };
        for bag in 0..bags {
            let salt = 60.0 + bag as f32 * 8.0;
            let bearing = body.facing
                + bag as f32 / bags as f32 * core::f32::consts::TAU
                + body.vary(11.0);
            let radius = 2.7 * build;
            self.push_instance(
                [
                    body.x + sway + sin(bearing) * radius,
                    body.y - (11.6 + body.vary_unit(salt) * 0.8) * build,
                    body.z + cos(bearing) * radius,
                ],
                [
                    (1.5 + body.vary_unit(salt + 2.0) * 0.5) * build,
                    (1.8 + body.vary_unit(salt + 4.0) * 0.6) * build,
                    1.5 * build,
                ],
                body.tint([0.44, 0.38, 0.28], salt, 0.06),
                bearing,
                0.0,
                Shape::Boulder,
            );
        }

        if !body.detail.at_least(BodyDetail::Full) {
            return;
        }
        self.push_instance(
            [crown[0], crown[1] + 1.5 * build, crown[2]],
            [3.0 * build, 0.6 * build, 3.0 * build],
            iron,
            body.facing,
            0.0,
            Shape::Cylinder,
        );
        // inner flame, hotter and smaller than the one around it
        self.push_instance(
            [body.x, body.y - 6.8 * build + burn * 0.5, body.z],
            [1.1 * build, (1.7 + burn * 1.1) * build, 1.1 * build],
            [1.0, 0.95, 0.72],
            body.facing,
            1.0,
            Shape::Cone,
        );
        // burner can, hanging off the load ring
        self.push_instance(
            [body.x, body.y - 7.6 * build, body.z],
            [1.7 * build, 1.8 * build, 1.7 * build],
            iron,
            body.facing,
            0.0,
            Shape::Cylinder,
        );
        // weave bands round the basket, tracking its taper
        for (depth, width) in [(9.9f32, 4.3f32), (11.5, 3.1)] {
            self.push_instance(
                [body.x + sway, body.y - depth * build, body.z],
                [width * build, 0.5 * build, width * build],
                shaded(wicker, 0.72),
                body.facing,
                0.0,
                Shape::Cylinder,
            );
        }
        // a pennant on a short mast, streaming off the back of the basket
        let mast_foot = [body.x + sway, body.y - 10.9 * build, body.z];
        let mast_head = [
            mast_foot[0] - body.facing_cos * 2.4 * build,
            mast_foot[1] + 4.4 * build,
            mast_foot[2] + body.facing_sin * 2.4 * build,
        ];
        let (at, rotation, length) = shaft(mast_foot, mast_head);
        self.push_oriented(
            at,
            [0.24 * build, length, 0.24 * build],
            shaded(wicker, 0.6),
            rotation,
            0.0,
            Shape::Cone,
        );
        let flag = Rotation::new(
            body.facing + core::f32::consts::PI + sway * 0.5,
            0.24 + sway * 0.3,
            sway * 0.8,
        );
        let flag_length = 3.6 * build;
        self.push_oriented(
            rooted(mast_head, flag, flag_length),
            [1.5 * build, 0.5 * build, flag_length],
            gore_b,
            flag,
            0.06,
            Shape::Frond,
        );
    }

    /// A serpent-dragon. Every piece of it — masses, scutes, spines, limbs,
    /// skull, barb — is placed by asking one travelling wave where the spine
    /// is at that station, so the whole animal rides a single curve instead of
    /// stepping along a straight line. Worst case 89 instances at `Full`, 21
    /// at `Reduced`, 6 at `Distant`.
    pub(super) fn draw_dragon(&mut self, body: &CreatureBody) {
        /// How many stations the spine is sampled at.
        const STATIONS: usize = 10;
        /// Where each station sits along the forward axis, measured from the
        /// shoulders. Deliberately uneven: a long open neck, a short deep
        /// chest, a tail that closes up as it thins.
        const AXIS: [f32; STATIONS] =
            [26.0, 19.6, 13.2, 6.8, 0.0, -6.6, -12.6, -18.2, -23.2, -27.6];
        /// Girth at each station as a fraction of the deepest part. The neck
        /// runs at a third of the chest, which is what keeps the animal from
        /// reading as one sausage with a face on the end.
        const GIRTH: [f32; STATIONS] =
            [0.34, 0.40, 0.50, 0.66, 0.94, 1.0, 0.88, 0.68, 0.48, 0.30];
        /// Station the shoulders — and so the wings and forelimbs — hang from.
        const SHOULDER: f32 = 4.0;
        /// The travelling wave: how far the body swings, how fast the wave runs
        /// down the spine, how much it heaves, and how far the body yaws,
        /// pitches and banks with its own curve. The bank is the important one
        /// — a serpent leans into a bend, and without it the wave reads as
        /// beads sliding along a wire.
        const WAVE_SWING: f32 = 5.4;
        const WAVE_LAG: f32 = 0.62;
        const WAVE_RISE: f32 = 2.4;
        const WAVE_YAW: f32 = 0.30;
        const WAVE_PITCH: f32 = 0.20;
        const WAVE_BANK: f32 = 0.62;
        /// How high the neck arches the head above the trunk line, and how far
        /// the tail trails below it.
        const NECK_ARCH: f32 = 1.15;
        const TAIL_DROOP: f32 = 1.6;
        /// Hides a dragon can be born with: deep red, bronze, slate, jade.
        /// Four families rather than one hue nudged — two dragons met in the
        /// same realm should not read as siblings.
        const HIDES: [[f32; 3]; 4] = [
            [0.44, 0.10, 0.12],
            [0.50, 0.33, 0.11],
            [0.26, 0.30, 0.36],
            [0.16, 0.31, 0.18],
        ];
        /// Belly plating and eye colour that go with each hide.
        const BELLIES: [[f32; 3]; 4] = [
            [0.74, 0.50, 0.28],
            [0.80, 0.66, 0.33],
            [0.58, 0.62, 0.66],
            [0.62, 0.66, 0.38],
        ];
        const EYES: [[f32; 3]; 4] = [
            [1.0, 0.78, 0.22],
            [1.0, 0.60, 0.14],
            [0.60, 0.86, 1.0],
            [0.70, 1.0, 0.34],
        ];
        const HALF_TURN: f32 = core::f32::consts::PI;

        let full = body.detail.at_least(BodyDetail::Full);
        let near = body.detail.at_least(BodyDetail::Reduced);

        // ---- what this particular dragon is built like ----
        let family = clamp(body.vary_unit(2.0) * 4.0, 0.0, 3.0) as usize;
        let length = 1.0 + body.vary(3.5) * 0.20;
        let bulk = 1.0 + body.vary(5.0) * 0.18;
        let unit = 9.4 * bulk;

        let flash = body.hurt * 0.42;
        let hide = body.tint(HIDES[family], 7.0, 0.05);
        let scale_lit = [
            clamp(hide[0] + flash, 0.0, 1.0),
            clamp(hide[1] + flash * 0.25, 0.0, 1.0),
            clamp(hide[2] + flash * 0.25, 0.0, 1.0),
        ];
        let scale_mid = [scale_lit[0] * 0.78, scale_lit[1] * 0.76, scale_lit[2] * 0.82];
        let scale_dark = [scale_lit[0] * 0.50, scale_lit[1] * 0.48, scale_lit[2] * 0.56];
        let belly = body.tint(BELLIES[family], 8.6, 0.05);
        let ivory = body.tint([0.90, 0.86, 0.72], 10.2, 0.07);
        // the membrane is lit from behind in flight, so it runs hotter and
        // less saturated than the hide, and drifts its own way per dragon
        let membrane = [
            clamp(scale_dark[0] * 1.55 + body.vary(11.4) * 0.10, 0.03, 1.0),
            clamp(scale_dark[1] * 1.40 + body.vary(12.6) * 0.08, 0.03, 1.0),
            clamp(scale_dark[2] * 1.50 + body.vary(13.8) * 0.10, 0.03, 1.0),
        ];
        let strut = shaded(scale_dark, 0.70);

        // ---- the spine, and the frames hung off it ----
        // Linear between the two nearest stations, extrapolating off both ends
        // so the head and the tail tip can ask for stations that are not there.
        let sample = |table: [f32; STATIONS], along: f32| -> f32 {
            let low = clamp(floor(along), 0.0, (STATIONS - 2) as f32);
            let index = low as usize;
            table[index] + (table[index + 1] - table[index]) * (along - low)
        };
        // `CreatureBody::ahead`, but about an arbitrary joint and heading, so a
        // horn can be placed relative to a skull that is itself riding a wave.
        let offset = |origin: [f32; 3], yaw: f32, forward: f32, up: f32, side: f32| -> [f32; 3] {
            let heading_sin = sin(yaw);
            let heading_cos = cos(yaw);
            [
                origin[0] + heading_sin * forward + heading_cos * side,
                origin[1] + up,
                origin[2] + heading_cos * forward - heading_sin * side,
            ]
        };
        // Where a part's own +Y ends up once it is pitched and rolled, as
        // (side, up, forward). Limbs, horns and spines are laid out by walking
        // this joint to joint instead of by guessing at offsets, which is the
        // only way angled chains stay joined.
        let bone_axis = |pitch: f32, roll: f32| -> [f32; 3] {
            [-sin(roll), cos(roll) * cos(pitch), cos(roll) * sin(pitch)]
        };
        let swing = WAVE_SWING * (0.85 + body.vary_unit(14.0) * 0.40);
        let station = |along: f32| -> ([f32; 3], Rotation) {
            let travel = body.phase - along * WAVE_LAG;
            let sway = sin(travel);
            let heave = cos(travel);
            // the wave grows toward the tail, as a swimming snake's does
            let reach = swing * (0.42 + along * 0.10);
            let arch = max(2.2 - along, 0.0);
            let sink = max(along - 6.4, 0.0);
            (
                body.ahead(
                    sample(AXIS, along) * length,
                    heave * WAVE_RISE + arch * arch * NECK_ARCH - sink * TAIL_DROOP,
                    sway * reach,
                ),
                Rotation::new(
                    body.facing + heave * WAVE_YAW,
                    -sway * WAVE_PITCH - arch * 0.20 + sink * 0.12,
                    sway * WAVE_BANK,
                ),
            )
        };
        let girth_at = |along: f32| max(sample(GIRTH, along), 0.05) * unit;

        // ---- trunk masses ----
        let masses = match body.detail {
            BodyDetail::Full => 10,
            BodyDetail::Reduced => 5,
            BodyDetail::Distant => 4,
        };
        let step = (STATIONS - 1) as f32 / (masses - 1) as f32;
        for mass in 0..masses {
            let along = mass as f32 * step;
            let (at, spine) = station(along);
            let girth = girth_at(along);
            let salt = 20.0 + mass as f32 * 0.9;
            // faceted flanks, each mass turned a little differently and long
            // enough to bury the joint with its neighbour
            self.push_oriented(
                at,
                [
                    girth * (0.92 + body.vary(salt) * 0.06),
                    girth * (1.00 + body.vary(salt + 0.4) * 0.07),
                    girth * 0.70 + step * 6.4 * length,
                ],
                if mass % 2 == 0 { scale_lit } else { scale_mid },
                Rotation::new(
                    spine.yaw,
                    spine.pitch,
                    spine.roll + body.vary(salt + 0.8) * 0.30,
                ),
                body.hurt * 0.4,
                Shape::Boulder,
            );
        }

        // ---- belly scutes ----
        if full {
            for plate in 0..5 {
                let along = 2.3 + plate as f32 * 1.10;
                let (at, spine) = station(along);
                let girth = girth_at(along);
                let salt = 26.0 + plate as f32 * 0.7;
                let shade = 0.82 + (plate % 2) as f32 * 0.13 + body.vary(salt) * 0.04;
                // shingled: each plate is longer than the gap to the next and
                // tips nose-down, so the underside reads as overlapping scutes
                // rather than as a painted stripe
                self.push_oriented(
                    offset(at, spine.yaw, 0.0, -girth * 0.38, 0.0),
                    [girth * 0.66, girth * 0.32, girth * 0.40 + 7.0 * length],
                    shaded(belly, shade),
                    Rotation::new(
                        spine.yaw,
                        spine.pitch + 0.18 + body.vary(salt + 0.4) * 0.06,
                        spine.roll + HALF_TURN,
                    ),
                    0.0,
                    Shape::Wedge,
                );
            }
        }

        // ---- dorsal ridge ----
        if near {
            let spines = if full {
                5 + (body.vary_unit(16.0) * 3.99) as i32
            } else {
                3
            };
            // one dragon in three has taken a spine off against something
            let snapped = if full && body.quirk(17.4, 0.35) {
                (body.vary_unit(18.6) * spines as f32) as i32
            } else {
                -1
            };
            let ridge_step = 7.4 / (spines - 1) as f32;
            for spine in 0..spines {
                let along = 0.8 + spine as f32 * ridge_step;
                let (at, curve) = station(along);
                let girth = girth_at(along);
                let salt = 30.0 + spine as f32 * 1.3;
                let broken = spine == snapped;
                let height = girth * (0.45 + body.vary_unit(salt) * 0.55);
                let stand = if broken { height * 0.38 } else { height };
                // every spine rakes back — negative pitch, since positive tips
                // a part's own axis forward — and none by the same amount
                let rake = 0.30 + body.vary_unit(salt + 0.5) * 0.40;
                let cant = body.vary(salt + 1.0) * 0.24;
                let tone = 0.88 + body.vary_unit(salt + 1.6) * 0.16;
                self.push_oriented(
                    offset(at, curve.yaw, 0.0, girth * 0.34 + stand * 0.34, 0.0),
                    if broken {
                        [girth * 0.22, stand, girth * 0.22]
                    } else {
                        [girth * 0.16, stand, girth * 0.30]
                    },
                    if broken { ivory } else { shaded(scale_dark, tone) },
                    Rotation::new(curve.yaw, curve.pitch - rake, curve.roll + cant),
                    0.0,
                    if broken { Shape::Cylinder } else { Shape::Cone },
                );
            }
        }

        // ---- tail, carrying the wave past the last mass ----
        if near {
            let tip_girth = girth_at(9.0);
            if full {
                let (fin_at, fin) = station(9.6);
                self.push_oriented(
                    fin_at,
                    [
                        tip_girth * 0.34,
                        tip_girth * (1.4 + body.vary_unit(19.2) * 1.4),
                        11.0 * length,
                    ],
                    scale_dark,
                    Rotation::new(fin.yaw, fin.pitch + 0.22, fin.roll),
                    0.0,
                    Shape::Wedge,
                );
            }
            let (tip_at, tip) = station(10.4);
            if body.quirk(21.0, 0.5) {
                // a barb: the cone's apex is swung round to trail behind
                self.push_oriented(
                    tip_at,
                    [tip_girth * 0.34, 8.0 * length, tip_girth * 0.34],
                    ivory,
                    Rotation::new(tip.yaw, tip.pitch - 1.52, tip.roll),
                    0.0,
                    Shape::Cone,
                );
            } else {
                self.push_oriented(
                    tip_at,
                    [tip_girth * 0.95, tip_girth * 0.85, tip_girth * 1.25],
                    scale_mid,
                    tip,
                    0.0,
                    Shape::Boulder,
                );
            }
        }

        // ---- shoulders and wings ----
        let (shoulder_at, shoulder) = station(SHOULDER);
        let shoulder_girth = girth_at(SHOULDER);
        if full {
            for hand in 0..2 {
                let side = if hand == 0 { 1.0f32 } else { -1.0 };
                let salt = 34.0 + hand as f32 * 1.1;
                self.push_oriented(
                    offset(
                        shoulder_at,
                        shoulder.yaw,
                        shoulder_girth * 0.10,
                        shoulder_girth * 0.30,
                        side * shoulder_girth * 0.50,
                    ),
                    [
                        shoulder_girth * (0.50 + body.vary_unit(salt) * 0.12),
                        shoulder_girth * 0.58,
                        shoulder_girth * (0.72 + body.vary_unit(salt + 0.5) * 0.14),
                    ],
                    scale_mid,
                    Rotation::new(shoulder.yaw, shoulder.pitch, shoulder.roll + side * 0.35),
                    0.0,
                    Shape::Boulder,
                );
            }
        }
        let beat = sin(body.phase * (1.30 + body.vary_unit(23.0) * 0.5))
            * (6.0 + body.vary_unit(24.2) * 3.0);
        let span = 19.5 * length * (1.0 + body.vary(25.4) * 0.12);
        let chord = 10.0 * bulk * (1.0 + body.vary(26.6) * 0.12);
        // hung off the shoulder station rather than off the creature's centre,
        // so the wings ride the body wave instead of sliding along it
        let perch = CreatureBody {
            x: shoulder_at[0],
            y: shoulder_at[1] + shoulder_girth * 0.34,
            z: shoulder_at[2],
            ..*body
        };
        for side in [1.0f32, -1.0] {
            self.draw_wing(&perch, side, 3.0 + beat, span, chord, membrane, strut, false);
        }

        // ---- head ----
        if !near {
            return;
        }
        // a third of a station past the last neck mass, not a whole one: the
        // arch climbs fast enough that a full station would leave the skull
        // hanging clear of the neck
        let (neck_end, neck) = station(-0.35);
        let skull = unit * (0.50 + body.vary_unit(27.8) * 0.10);
        let head_yaw = neck.yaw + body.vary(28.4) * 0.06;
        // the neck is reared, but the skull levels off on top of it rather
        // than pointing wherever the neck happens to be going
        let head_pitch = neck.pitch * 0.35;
        let head_roll = neck.roll * 0.45;
        let head = offset(neck_end, head_yaw, skull * 0.35, skull * 0.20, 0.0);
        self.push_oriented(
            head,
            [skull * 1.02, skull * 0.94, skull * 1.34],
            scale_lit,
            Rotation::new(head_yaw, head_pitch, head_roll),
            body.hurt * 0.4,
            Shape::Boulder,
        );
        // snout: the muzzle breaks downward off the skull line instead of
        // running out in one straight taper
        self.push_oriented(
            offset(head, head_yaw, skull * 1.18, -skull * 0.02, 0.0),
            [skull * 0.80, skull * 0.64, skull * 1.32],
            scale_lit,
            Rotation::new(head_yaw, head_pitch + 0.10, head_roll),
            0.0,
            Shape::Wedge,
        );
        for side in [1.0f32, -1.0] {
            // small and bright: a big glowing ball reads as a lamp, and bloom
            // blows it out into a white blob
            self.push_instance(
                offset(head, head_yaw, skull * 0.66, skull * 0.30, side * skull * 0.52),
                [skull * 0.16, skull * 0.18, skull * 0.16],
                EYES[family],
                head_yaw,
                1.0,
                Shape::Sphere,
            );
        }
        let horn_pairs = if full && body.quirk(29.6, 0.55) { 2 } else { 1 };
        let horn_snapped = body.quirk(30.8, 0.30);
        let snapped_hand = if body.vary(31.4) > 0.0 { 0 } else { 1 };
        let horn_hue = body.tint([0.86, 0.80, 0.66], 32.0, 0.10);
        for pair in 0..horn_pairs {
            for hand in 0..2 {
                let side = if hand == 0 { 1.0f32 } else { -1.0 };
                let salt = 44.0 + pair as f32 * 2.0 + hand as f32 * 0.9;
                let sweep = 0.62 + pair as f32 * 0.28 + body.vary_unit(salt) * 0.44;
                let flare = 0.30 + pair as f32 * 0.24 + body.vary_unit(salt + 0.5) * 0.26;
                let broken = horn_snapped && pair == 0 && hand == snapped_hand;
                let long = skull
                    * (1.35 - pair as f32 * 0.42)
                    * (0.80 + body.vary_unit(salt + 1.0) * 0.50);
                let stand = if broken { long * 0.38 } else { long };
                // back and out: negative pitch sweeps the horn tailwards,
                // negative roll on the right side swings it clear of the skull
                let pitch = head_pitch - sweep;
                let roll = head_roll - side * flare;
                let axis = bone_axis(pitch, roll);
                let root = offset(
                    head,
                    head_yaw,
                    -skull * (0.30 + pair as f32 * 0.34),
                    skull * 0.44,
                    side * skull * (0.42 + pair as f32 * 0.16),
                );
                self.push_oriented(
                    offset(
                        root,
                        head_yaw,
                        axis[2] * stand * 0.42,
                        axis[1] * stand * 0.42,
                        axis[0] * stand * 0.42,
                    ),
                    [skull * 0.20, stand, skull * 0.22],
                    horn_hue,
                    Rotation::new(head_yaw, pitch, roll),
                    0.0,
                    if broken { Shape::Cylinder } else { Shape::Cone },
                );
            }
        }
        if !full {
            return;
        }
        self.push_oriented(
            offset(head, head_yaw, skull * 2.08, -skull * 0.20, 0.0),
            [skull * 0.58, skull * 0.48, skull * 1.06],
            scale_mid,
            Rotation::new(head_yaw, head_pitch + 0.26, head_roll),
            0.0,
            Shape::Wedge,
        );
        for side in [1.0f32, -1.0] {
            self.push_oriented(
                offset(
                    head,
                    head_yaw,
                    skull * 2.42,
                    skull * 0.04,
                    side * skull * 0.20,
                ),
                [skull * 0.15, skull * 0.13, skull * 0.20],
                scale_dark,
                Rotation::new(head_yaw, head_pitch + 0.30, head_roll),
                0.0,
                Shape::Boulder,
            );
            // brow ridge, canted outward so the skull has a hard edge over
            // the eye instead of a smooth dome
            self.push_oriented(
                offset(head, head_yaw, skull * 0.62, skull * 0.50, side * skull * 0.46),
                [skull * 0.28, skull * 0.26, skull * 1.05],
                scale_dark,
                Rotation::new(head_yaw, head_pitch + 0.20, head_roll + side * 0.55),
                0.0,
                Shape::Wedge,
            );
            // cheek, which also buries the jaw hinge
            self.push_oriented(
                offset(head, head_yaw, skull * 0.55, -skull * 0.22, side * skull * 0.60),
                [skull * 0.32, skull * 0.62, skull * 0.90],
                scale_mid,
                Rotation::new(head_yaw, head_pitch, head_roll + side * 0.20),
                0.0,
                Shape::Boulder,
            );
        }
        // the lower jaw swings on a hinge under the cheek, on a slow cycle of
        // its own, so the head is not one welded lump
        let gape = 0.10 + (0.5 + 0.5 * sin(body.phase * 0.6 + body.vary(33.2) * 3.0)) * 0.42;
        let jaw_pitch = head_pitch + gape;
        let jaw_forward = cos(jaw_pitch);
        let jaw_up = -sin(jaw_pitch);
        let jaw_long = skull * 2.20;
        let hinge_forward = skull * 0.62;
        let hinge_up = -skull * 0.36;
        self.push_oriented(
            offset(
                head,
                head_yaw,
                hinge_forward + jaw_forward * jaw_long * 0.5,
                hinge_up + jaw_up * jaw_long * 0.5,
                0.0,
            ),
            [skull * 0.62, skull * 0.40, jaw_long],
            scale_mid,
            Rotation::new(head_yaw, jaw_pitch, head_roll + HALF_TURN),
            0.0,
            Shape::Wedge,
        );
        for side in [1.0f32, -1.0] {
            // two fangs in the upper jaw, raked back, and one in the lower
            for fang in 0..2 {
                let salt = 36.0 + fang as f32 * 1.7 + side * 0.5;
                let tooth = skull * (0.26 + body.vary_unit(salt) * 0.28) * (1.0 - fang as f32 * 0.2);
                self.push_oriented(
                    offset(
                        head,
                        head_yaw,
                        skull * (1.35 + fang as f32 * 0.82),
                        -skull * (0.40 + fang as f32 * 0.10) - tooth * 0.40,
                        side * skull * 0.34,
                    ),
                    [skull * 0.11, tooth, skull * 0.12],
                    ivory,
                    Rotation::new(head_yaw, head_pitch + 0.18, head_roll + HALF_TURN),
                    0.0,
                    Shape::Cone,
                );
            }
            let salt = 40.0 + side * 0.8;
            let tooth = skull * (0.22 + body.vary_unit(salt) * 0.22);
            let along_jaw = jaw_long * 0.58;
            self.push_oriented(
                offset(
                    head,
                    head_yaw,
                    hinge_forward + jaw_forward * along_jaw,
                    hinge_up + jaw_up * along_jaw + tooth * 0.42,
                    side * skull * 0.30,
                ),
                [skull * 0.10, tooth, skull * 0.11],
                ivory,
                Rotation::new(head_yaw, jaw_pitch, head_roll + side * 0.10),
                0.0,
                Shape::Cone,
            );
            // jaw fringe, raked back off the cheek
            for quill in 0..2 {
                let salt = 50.0 + quill as f32 * 1.5 + side * 0.6;
                let spike =
                    skull * (0.55 + body.vary_unit(salt) * 0.45) * (1.0 - quill as f32 * 0.18);
                let rake = 1.10 + quill as f32 * 0.22 + body.vary_unit(salt + 0.5) * 0.25;
                let flare = 0.75 + quill as f32 * 0.18;
                let pitch = head_pitch - rake;
                let roll = head_roll - side * flare;
                let axis = bone_axis(pitch, roll);
                let root = offset(
                    head,
                    head_yaw,
                    -skull * (0.05 + quill as f32 * 0.30),
                    -skull * (0.10 - quill as f32 * 0.34),
                    side * skull * (0.62 + quill as f32 * 0.06),
                );
                self.push_oriented(
                    offset(
                        root,
                        head_yaw,
                        axis[2] * spike * 0.44,
                        axis[1] * spike * 0.44,
                        axis[0] * spike * 0.44,
                    ),
                    [skull * 0.13, spike, skull * 0.20],
                    scale_dark,
                    Rotation::new(head_yaw, pitch, roll),
                    0.0,
                    Shape::Cone,
                );
            }
        }

        // ---- limbs, tucked ----
        for limb in 0..4 {
            let side = if limb % 2 == 0 { 1.0f32 } else { -1.0 };
            let along = if limb < 2 { SHOULDER + 0.5 } else { SHOULDER + 2.2 };
            let (at, curve) = station(along);
            let girth = girth_at(along);
            let salt = 56.0 + limb as f32 * 1.9;
            let thick = girth * (0.20 + body.vary_unit(salt) * 0.06);
            let upper_long = girth * (0.85 + body.vary_unit(salt + 0.5) * 0.30);
            let fore_long = upper_long * (0.80 + body.vary_unit(salt + 1.0) * 0.25);
            let hip = offset(at, curve.yaw, 0.0, -girth * 0.20, side * girth * 0.44);
            // the upper bone hangs down and back, the forearm folds forward
            // under the belly and the foot hooks in — a Z-fold, not a strut
            let upper_pitch =
                curve.pitch - HALF_TURN + (0.50 + body.vary_unit(salt + 1.5) * 0.35);
            let upper_roll = curve.roll - side * (0.26 + body.vary_unit(salt + 2.0) * 0.22);
            let upper_axis = bone_axis(upper_pitch, upper_roll);
            self.push_oriented(
                offset(
                    hip,
                    curve.yaw,
                    upper_axis[2] * upper_long * 0.46,
                    upper_axis[1] * upper_long * 0.46,
                    upper_axis[0] * upper_long * 0.46,
                ),
                [thick, upper_long * 1.12, thick * 0.90],
                scale_mid,
                Rotation::new(curve.yaw, upper_pitch, upper_roll),
                0.0,
                Shape::Frustum,
            );
            let elbow = offset(
                hip,
                curve.yaw,
                upper_axis[2] * upper_long,
                upper_axis[1] * upper_long,
                upper_axis[0] * upper_long,
            );
            let fore_pitch =
                curve.pitch + HALF_TURN - (1.05 + body.vary_unit(salt + 2.5) * 0.30);
            let fore_roll = curve.roll - side * (0.14 + body.vary_unit(salt + 3.0) * 0.16);
            let fore_axis = bone_axis(fore_pitch, fore_roll);
            self.push_oriented(
                offset(
                    elbow,
                    curve.yaw,
                    fore_axis[2] * fore_long * 0.46,
                    fore_axis[1] * fore_long * 0.46,
                    fore_axis[0] * fore_long * 0.46,
                ),
                [thick * 0.78, fore_long * 1.14, thick * 0.72],
                scale_dark,
                Rotation::new(curve.yaw, fore_pitch, fore_roll),
                0.0,
                Shape::Cylinder,
            );
            let wrist = offset(
                elbow,
                curve.yaw,
                fore_axis[2] * fore_long,
                fore_axis[1] * fore_long,
                fore_axis[0] * fore_long,
            );
            let claw_pitch = fore_pitch - 0.40 + body.vary(salt + 3.5) * 0.12;
            let claw_roll = fore_roll + side * 0.30;
            let claw_axis = bone_axis(claw_pitch, claw_roll);
            let claw_long = girth * (0.36 + body.vary_unit(salt + 4.0) * 0.14);
            self.push_oriented(
                offset(
                    wrist,
                    curve.yaw,
                    claw_axis[2] * claw_long * 0.32,
                    claw_axis[1] * claw_long * 0.32,
                    claw_axis[0] * claw_long * 0.32,
                ),
                [thick * 1.05, claw_long, thick * 1.35],
                ivory,
                Rotation::new(curve.yaw, claw_pitch, claw_roll),
                0.0,
                Shape::Cone,
            );
        }
    }

}
