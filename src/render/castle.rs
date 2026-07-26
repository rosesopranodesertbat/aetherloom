//! The keeps: motte, curtain wall, mural towers, gatehouse and donjon, plus
//! the ruin left when one falls.
//!
//! There are never more than three of these on a map and a keep is the thing
//! the player flies home to, so it is allowed to be the most detailed object
//! in the frame. It is also visible from right across the realm, so every
//! fitting is gated on distance, and growth per tier is structural — another
//! mural tower, a second string course, corner turrets, more banners — rather
//! than a scale factor on the same building.
//!
//! Nothing here is placed by a formula that would give the player's keep and
//! the rival's the same answer. There is no `CreatureBody` behind a castle, so
//! the seed is the owner plus the pad it was built on, and it settles the
//! bearing the gate faces, the tower count, whether the donjon is a drum or a
//! square, the stone it was quarried from and where the banners fly.

use super::*;

/// Beyond these a keep loses its fittings, then everything but its skyline.
const CASTLE_FULL_RANGE: f32 = 430.0;
const CASTLE_REDUCED_RANGE: f32 = 900.0;

/// Mural towers on the enceinte: this many at tier one, one more every second
/// tier, and a coin flip per keep so two rings of the same tier differ.
const CASTLE_TOWERS_MIN: i32 = 4;
const CASTLE_TOWERS_MAX: i32 = 8;
/// Nominal radius of a mural tower, before its own variation.
const CASTLE_TOWER_RADIUS: f32 = 4.1;
/// Mural tower height at tier one, and what each further tier adds.
const CASTLE_TOWER_BASE_HEIGHT: f32 = 11.0;
const CASTLE_TOWER_HEIGHT_PER_TIER: f32 = 1.7;
/// Curtain wall between two towers.
const CASTLE_WALL_HEIGHT: f32 = 8.4;
const CASTLE_WALL_THICKNESS: f32 = 3.2;
/// Every parapet on a keep is this tall, so the courses line up by eye.
const CASTLE_PARAPET_HEIGHT: f32 = 3.1;
/// Boulders spilling from the revetment at the foot of the mound.
const CASTLE_APRON_ROCKS: usize = 5;
/// Banners never fly from more than this many mural towers whatever the tier.
/// A pole and a cloth is two instances and the ring runs to eight towers.
const CASTLE_MAX_TOWER_BANNERS: usize = 3;
/// Anything meant to read as a hole rather than a rectangle painted on stone.
const CASTLE_RECESS: [f32; 3] = [0.05, 0.045, 0.05];
/// Oak: banner poles, hoardings, bridge decks, snapped roof timbers.
const CASTLE_TIMBER: [f32; 3] = [0.27, 0.20, 0.13];

/// Seed for one keep. Two keeps in a realm are different buildings, and the
/// same keep is the same building next frame — which an RNG draw inside a draw
/// routine would not be.
fn castle_seed(wizard: usize, x: f32, z: f32) -> u32 {
    let mut hash = (wizard as u32).wrapping_mul(2_654_435_761);
    hash = hash.wrapping_add(((x * 0.25) as i32 as u32).wrapping_mul(374_761_393));
    hash = hash.wrapping_add(((z * 0.25) as i32 as u32).wrapping_mul(668_265_263));
    hash ^= hash >> 13;
    hash.wrapping_mul(1_274_126_177)
}

/// Stable pseudo-random in -1..1 for a seed and a `salt`. An integer hash for
/// the same reason `CreatureBody::vary` uses one: the `fract(sin(x))` trick
/// quantises badly at f32 precision and correlates between neighbouring seeds.
fn castle_vary(seed: u32, salt: f32) -> f32 {
    let mut hash = seed.wrapping_add(((salt * 89.0) as i32 as u32).wrapping_mul(747_796_405));
    hash ^= hash >> 15;
    hash = hash.wrapping_mul(2_246_822_519);
    hash ^= hash >> 13;
    hash = hash.wrapping_mul(3_266_489_917);
    hash ^= hash >> 16;
    (hash & 0xffff) as f32 / 32768.0 - 1.0
}

/// One keep's fixed facts, resolved once per frame and handed to every routine
/// that draws a piece of it.
struct Keep {
    x: f32,
    z: f32,
    /// Paving level of the terrace. Everything above ground is measured here.
    deck: f32,
    /// Underside of the mound, already sunk below the lowest ground it covers.
    foot: f32,
    tier: i32,
    /// Bearing the gatehouse faces. The whole enceinte is laid out from it, so
    /// no two keeps present the same face to the same approach.
    bearing: f32,
    /// The owner's colours, for banners and roof tiles.
    livery: [f32; 3],
    /// Stone this keep was quarried from — per keep, not per realm.
    stone: [f32; 3],
    /// 0 while the walls are sound, 1 when they are about to come down.
    damage: f32,
    detail: BodyDetail,
    /// Mural towers on the ring, and the radius the curtain runs at between
    /// them. Everything inside has to fit within that radius.
    towers: usize,
    wall_radius: f32,
    /// A drum donjon or a square one, and how wide it came out.
    drum: bool,
    donjon: f32,
    seed: u32,
}

impl Keep {
    fn vary(&self, salt: f32) -> f32 {
        castle_vary(self.seed, salt)
    }

    fn vary_unit(&self, salt: f32) -> f32 {
        self.vary(salt) * 0.5 + 0.5
    }

    /// True for roughly `odds` of keeps, fixed per keep. For the features that
    /// are present or absent rather than scaled: a hoarding, a stair turret, a
    /// woodpile in the ward.
    fn quirk(&self, salt: f32, odds: f32) -> bool {
        self.vary_unit(salt) < odds
    }

    /// One course of stone, a shade off the last. A wall carrying a single
    /// flat colour across eight instances reads as one plastic object however
    /// many pieces it is cut into.
    fn course(&self, salt: f32, shade: f32) -> [f32; 3] {
        let step = self.vary(salt) * shade;
        let warm = self.vary(salt + 3.3) * shade * 0.5;
        [
            clamp(self.stone[0] + step + warm, 0.0, 1.0),
            clamp(self.stone[1] + step, 0.0, 1.0),
            clamp(self.stone[2] + step - warm, 0.0, 1.0),
        ]
    }

    /// Roof tiles: the owner's colours knocked back to something that could be
    /// fired clay rather than paint.
    fn roof(&self, salt: f32) -> [f32; 3] {
        let shade = self.vary(salt) * 0.06;
        [
            clamp(self.livery[0] * 0.60 + 0.15 + shade, 0.0, 1.0),
            clamp(self.livery[1] * 0.60 + 0.13 + shade, 0.0, 1.0),
            clamp(self.livery[2] * 0.60 + 0.12 + shade, 0.0, 1.0),
        ]
    }

    /// Dyed cloth, which is brighter than anything the masons had.
    fn cloth(&self, salt: f32) -> [f32; 3] {
        let shade = self.vary(salt) * 0.07;
        [
            clamp(self.livery[0] * 1.24 + 0.07 + shade, 0.0, 1.0),
            clamp(self.livery[1] * 1.24 + 0.06 + shade, 0.0, 1.0),
            clamp(self.livery[2] * 1.24 + 0.05 + shade, 0.0, 1.0),
        ]
    }

    /// A point on a ring about the keep centre.
    fn ring(&self, angle: f32, radius: f32, y: f32) -> [f32; 3] {
        [self.x + cos(angle) * radius, y, self.z + sin(angle) * radius]
    }

    /// Half the angle between two neighbouring mural towers.
    fn half_step(&self) -> f32 {
        core::f32::consts::PI / self.towers as f32
    }

    /// Instance yaw that lays local +X on the outward radial at `angle` and
    /// local +Z along the ring. Wall panels, arrow slits, sills, buttresses and
    /// machicolations all want this, and are ninety degrees wrong without it —
    /// which is most of why a ring of parts reads as a pile of crates.
    fn outward(angle: f32) -> f32 {
        -angle
    }

    /// Instance yaw that points local +Z straight out from the centre, for the
    /// few things that run away from the keep rather than around it.
    fn radial(angle: f32) -> f32 {
        core::f32::consts::FRAC_PI_2 - angle
    }
}

impl World {
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

    fn keep_of(&self, wizard: usize, ground: f32) -> Keep {
        let x = self.castles.pos_x[wizard];
        let z = self.castles.pos_z[wizard];
        let base = self.castles.pos_y[wizard];
        let seed = castle_seed(wizard, x, z);
        let mut tier = self.castles.tier[wizard];
        if tier < 1 {
            tier = 1;
        }
        if tier > MAX_CASTLE_TIER {
            tier = MAX_CASTLE_TIER;
        }
        let damage = clamp(
            1.0 - self.castles.health[wizard] / max(self.castles.max_health[wizard], 1.0),
            0.0,
            1.0,
        );

        // The player builds in blue and the rivals in red, but no two rivals
        // fly the same red.
        let swing = castle_vary(seed, 11.0);
        let livery = if wizard == PLAYER {
            [0.22 + swing * 0.05, 0.33 + swing * 0.06, 0.72 - swing * 0.06]
        } else {
            [0.70 + swing * 0.10, 0.20 + swing * 0.09, 0.16 - swing * 0.06]
        };
        // Grey, swung between warm sandstone and cold granite, with a wash of
        // the owner's colours so the keep still reads as theirs from the air,
        // and blackened as it takes damage.
        let warmth = castle_vary(seed, 5.0);
        let value = castle_vary(seed, 7.0);
        let scorch = damage * damage * 0.16;
        let stone = [
            clamp(0.41 + value * 0.08 + warmth * 0.06 + livery[0] * 0.10 - scorch, 0.0, 1.0),
            clamp(0.39 + value * 0.08 + livery[1] * 0.10 - scorch, 0.0, 1.0),
            clamp(0.37 + value * 0.07 - warmth * 0.05 + livery[2] * 0.10 - scorch, 0.0, 1.0),
        ];

        let mut towers = CASTLE_TOWERS_MIN + tier / 2;
        if castle_vary(seed, 2.5) < 0.0 {
            towers += 1;
        }
        if towers > CASTLE_TOWERS_MAX {
            towers = CASTLE_TOWERS_MAX;
        }
        if towers < CASTLE_TOWERS_MIN {
            towers = CASTLE_TOWERS_MIN;
        }
        let towers = towers as usize;
        // The curtain runs as a chord between neighbouring towers, so it sits
        // closer in than the tower ring does — and a four-tower enceinte is a
        // much tighter one than an eight-tower enceinte.
        let wall_radius = CASTLE_TOWER_RING * cos(core::f32::consts::PI / towers as f32);
        let drum = castle_vary(seed, 31.0) < -0.1;
        // A square donjon is widest across its corners, so it has to be sized
        // more meanly than a drum for the same enceinte. Sizing either to the
        // room the curtain leaves — with a ward's width still to spare — is
        // also what makes a tier-six keep look bulkier and not merely taller.
        let room = wall_radius - CASTLE_WALL_THICKNESS;
        let donjon = min(
            15.4 + castle_vary(seed, 30.0) * 2.8,
            room / if drum { 0.80 } else { 0.95 },
        );

        let range_sq = length_sq3(
            x - self.wizards.pos_x[PLAYER],
            base - self.wizards.pos_y[PLAYER],
            z - self.wizards.pos_z[PLAYER],
        );
        Keep {
            x,
            z,
            deck: base + CASTLE_DECK_HEIGHT,
            // sunk so it reads as cut into the hill, but never a skyscraper
            foot: max(ground - CASTLE_PLINTH_SINK, base - CASTLE_PLINTH_MAX_DROP),
            tier,
            bearing: castle_vary(seed, 1.0) * core::f32::consts::PI,
            livery,
            stone,
            damage,
            detail: if range_sq < CASTLE_FULL_RANGE * CASTLE_FULL_RANGE {
                BodyDetail::Full
            } else if range_sq < CASTLE_REDUCED_RANGE * CASTLE_REDUCED_RANGE {
                BodyDetail::Reduced
            } else {
                BodyDetail::Distant
            },
            towers,
            wall_radius,
            drum,
            donjon,
            seed,
        }
    }

    pub(super) fn draw_castle(&mut self, wizard: usize) {
        let centre_x = self.castles.pos_x[wizard];
        let centre_z = self.castles.pos_z[wizard];
        let ground = self.lowest_ground_under_castle(centre_x, centre_z);
        if self.castles.health[wizard] <= 0.0 {
            self.draw_castle_ruin(wizard, ground);
            return;
        }
        let keep = self.keep_of(wizard, ground);
        self.draw_castle_motte(&keep);
        self.draw_castle_enceinte(&keep);
        self.draw_castle_donjon(&keep);
        if keep.detail.at_least(BodyDetail::Full) {
            self.draw_castle_ward(&keep);
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

    // ---- shared pieces -----------------------------------------------------

    /// A shaft that leans in as it rises. A frustum used honestly pinches to
    /// 45% of its base and reads as a funnel; sunk so that only the gentle
    /// lower part of the taper stands above its footing, it reads as coursed
    /// stone battered for strength.
    #[allow(clippy::too_many_arguments)]
    fn push_castle_shaft(
        &mut self,
        foot: [f32; 3],
        height: f32,
        width: f32,
        buried: f32,
        colour: [f32; 3],
        yaw: f32,
    ) {
        /// What the frustum mesh has left at its top, as a fraction of base.
        const FRUSTUM_TOP: f32 = 0.45;
        let full = height + buried;
        let base = width / (1.0 - (1.0 - FRUSTUM_TOP) * (buried / full));
        self.push_instance(
            [foot[0], foot[1] - buried + full * 0.5, foot[2]],
            [base, full, base],
            colour,
            yaw,
            0.0,
            Shape::Frustum,
        );
    }

    /// A pole with the owner's colours hanging off it. `Frond` rolled almost
    /// flat makes a far better banner than a box: it narrows toward the fly,
    /// the droop already in the mesh becomes the sag of the cloth, and it is
    /// drawn from both faces so it does not vanish when flown under.
    fn push_castle_banner(&mut self, keep: &Keep, at: [f32; 3], height: f32, salt: f32) {
        self.push_instance(
            [at[0], at[1] + height * 0.5, at[2]],
            [0.55, height, 0.55],
            CASTLE_TIMBER,
            0.0,
            0.0,
            Shape::Cylinder,
        );
        let hang = height * (0.52 + keep.vary_unit(salt) * 0.24);
        // pitched just short of straight down, so the cloth lifts on the wind
        let lift = 0.16 + keep.vary_unit(salt + 1.0) * 0.30;
        self.push_oriented(
            [at[0], at[1] + height - hang * 0.5, at[2]],
            [hang * 0.58, hang * 0.5, hang],
            keep.cloth(salt + 2.0),
            Rotation::new(
                keep.bearing + keep.vary(salt + 3.0) * 2.6,
                core::f32::consts::FRAC_PI_2 - lift,
                keep.vary(salt + 4.0) * 0.34,
            ),
            0.0,
            Shape::Frond,
        );
    }

    // ---- the mound ---------------------------------------------------------

    /// The motte the whole keep stands on. Round, because the pad under it is
    /// levelled in a circle and a square plinth on a round shelf shows daylight
    /// at its corners.
    fn draw_castle_motte(&mut self, keep: &Keep) {
        let radius = CASTLE_FOOTPRINT;
        let mound = keep.deck - keep.foot;
        let rock = keep.course(0.5, 0.04);
        let dark = [rock[0] * 0.64, rock[1] * 0.62, rock[2] * 0.60];
        self.push_instance(
            [keep.x, (keep.deck + keep.foot) * 0.5, keep.z],
            [radius * 2.0, mound, radius * 2.0],
            dark,
            keep.bearing,
            0.0,
            Shape::Cylinder,
        );
        if !keep.detail.at_least(BodyDetail::Reduced) {
            return;
        }
        // a wider skirt where the mound meets the hill, so it is a mound and
        // not a drum stood on the grass
        self.push_instance(
            [keep.x, keep.foot + mound * 0.30, keep.z],
            [radius * 2.0 + 7.0, mound * 0.66, radius * 2.0 + 7.0],
            [dark[0] * 0.88, dark[1] * 0.88, dark[2] * 0.86],
            -keep.bearing,
            0.0,
            Shape::Cylinder,
        );
        // corbelled course under the terrace: the overhang is what stops the
        // whole mound reading as one tapering lump
        self.push_instance(
            [keep.x, keep.deck - 1.4, keep.z],
            [radius * 2.0 + 2.6, 3.6, radius * 2.0 + 2.6],
            keep.course(0.9, 0.06),
            keep.bearing * 0.5,
            0.0,
            Shape::Cylinder,
        );
        // paving, a shade off the revetment it sits inside
        self.push_instance(
            [keep.x, keep.deck + 0.2, keep.z],
            [radius * 2.0 - 1.4, 2.0, radius * 2.0 - 1.4],
            keep.course(1.4, 0.07),
            0.0,
            0.0,
            Shape::Cylinder,
        );
        // coping along the terrace edge, proud of the paving
        self.push_instance(
            [keep.x, keep.deck + 1.1, keep.z],
            [radius * 2.0 + 0.4, 1.5, radius * 2.0 + 0.4],
            keep.course(1.9, 0.05),
            0.0,
            0.0,
            Shape::Cylinder,
        );
        if !keep.detail.at_least(BodyDetail::Full) {
            return;
        }
        // boulders spilling out of the revetment, in a different place on
        // every keep
        for lump in 0..CASTLE_APRON_ROCKS {
            let salt = 20.0 + lump as f32 * 3.0;
            let angle = keep.vary(salt) * core::f32::consts::PI;
            let out = radius + 1.4 + keep.vary_unit(salt + 1.0) * 5.4;
            let size = 3.4 + keep.vary_unit(salt + 2.0) * 4.4;
            let shade = keep.vary(salt + 2.4) * 0.04;
            self.push_oriented(
                keep.ring(
                    angle,
                    out,
                    keep.foot + mound * (0.16 + keep.vary_unit(salt + 1.5) * 0.52),
                ),
                [size, size * 0.82, size * 1.14],
                [dark[0] + shade, dark[1] + shade, dark[2] + shade],
                Rotation::new(
                    angle,
                    keep.vary(salt + 2.6) * 0.55,
                    keep.vary(salt + 3.0) * 0.55,
                ),
                0.0,
                Shape::Boulder,
            );
        }
    }

    // ---- the enceinte ------------------------------------------------------

    /// The ring: mural towers, the curtain runs between them, and the gatehouse
    /// on the one run that faces the keep's own bearing.
    fn draw_castle_enceinte(&mut self, keep: &Keep) {
        let step = core::f32::consts::TAU / keep.towers as f32;

        if !keep.detail.at_least(BodyDetail::Reduced) {
            // silhouette only: four drums, and nothing standing on them
            for index in 0..4 {
                let angle = keep.bearing + index as f32 * core::f32::consts::FRAC_PI_2;
                self.push_instance(
                    keep.ring(angle, CASTLE_TOWER_RING, keep.deck + 7.0),
                    [CASTLE_TOWER_RADIUS * 2.2, 16.0, CASTLE_TOWER_RADIUS * 2.2],
                    keep.stone,
                    Keep::outward(angle),
                    0.0,
                    Shape::Cylinder,
                );
            }
            return;
        }

        // Banners fly from a fixed few towers rather than from however many
        // happen to roll high, so the worst-case instance count is knowable.
        let mut banners = if keep.detail.at_least(BodyDetail::Full) {
            1 + keep.tier as usize / 3
        } else {
            0
        };
        if banners > CASTLE_MAX_TOWER_BANNERS {
            banners = CASTLE_MAX_TOWER_BANNERS;
        }
        let offset = (keep.vary_unit(4.0) * keep.towers as f32) as usize;

        for index in 0..keep.towers {
            let angle = keep.bearing + (index as f32 + 0.5) * step;
            let flagged = banners > 0 && (index + offset) % keep.towers < banners;
            self.draw_castle_tower(keep, index, angle, flagged);
        }
        for run in 0..keep.towers {
            // the last run is centred on the bearing itself: that is the gate
            let mid = keep.bearing + (run as f32 + 1.0) * step;
            if run + 1 == keep.towers {
                self.draw_castle_gatehouse(keep, mid);
            } else {
                self.draw_castle_curtain(keep, run, mid);
            }
        }
    }

    /// One mural tower. Height, girth, roof pitch, whether it is roofed at all
    /// and whether it was built round or square are hashed off the keep and the
    /// tower's own index, so no two drums on a ring match and no two rings in a
    /// realm match either.
    fn draw_castle_tower(&mut self, keep: &Keep, index: usize, angle: f32, banner: bool) {
        let salt = 40.0 + index as f32 * 5.0;
        let mut height = CASTLE_TOWER_BASE_HEIGHT
            + keep.tier as f32 * CASTLE_TOWER_HEIGHT_PER_TIER
            + keep.vary(salt) * 3.6;
        let radius = CASTLE_TOWER_RADIUS * (0.86 + keep.vary_unit(salt + 1.0) * 0.34);
        // A tower shelled off its top once the keep is being taken apart: the
        // damage belongs in the skyline, not only in a health bar.
        let shelled = keep.damage > 0.35 && keep.vary_unit(salt + 2.0) < keep.damage;
        if shelled {
            height *= 0.48 + keep.vary_unit(salt + 2.5) * 0.24;
        }
        // one style per keep, with the odd tower put up by a different mason
        let square = keep.quirk(3.0, 0.4) ^ keep.quirk(salt + 3.0, 0.18);
        let foot = keep.deck - 1.6;
        let head = foot + height;
        let yaw = Keep::outward(angle);
        let full = keep.detail.at_least(BodyDetail::Full);
        let stone = keep.course(salt + 4.0, 0.05);

        if full {
            // Plinth, overlapping both the paving and the shaft so neither
            // joint shows a seam. Square towers get a block rather than a
            // frustum: the frustum's own taper would pinch it in under a shaft
            // that is still wider at that height, which reads as an hourglass.
            self.push_instance(
                keep.ring(angle, CASTLE_TOWER_RING, foot + 2.0),
                [radius * 2.62, 6.6, radius * 2.62],
                [stone[0] * 0.88, stone[1] * 0.87, stone[2] * 0.86],
                yaw,
                0.0,
                if square { Shape::Cuboid } else { Shape::Cylinder },
            );
        }
        if square {
            self.push_castle_shaft(
                keep.ring(angle, CASTLE_TOWER_RING, foot),
                height,
                radius * 2.0,
                13.0,
                stone,
                yaw,
            );
        } else {
            self.push_instance(
                keep.ring(angle, CASTLE_TOWER_RING, foot + height * 0.5),
                [radius * 2.0, height, radius * 2.0],
                stone,
                yaw,
                0.0,
                Shape::Cylinder,
            );
        }

        if full {
            // A string course partway up. One band of slightly different stone
            // is most of what stops a drum reading as a length of pipe.
            self.push_instance(
                keep.ring(
                    angle,
                    CASTLE_TOWER_RING,
                    foot + height * (0.36 + keep.vary_unit(salt + 5.0) * 0.18),
                ),
                [radius * 2.3, 1.3, radius * 2.3],
                keep.course(salt + 6.0, 0.08),
                yaw,
                0.0,
                if square { Shape::Cuboid } else { Shape::Cylinder },
            );
            // corbels carrying the parapet out past the shaft
            self.push_instance(
                keep.ring(angle, CASTLE_TOWER_RING, head - 0.9),
                [radius * 2.46, 1.9, radius * 2.46],
                keep.course(salt + 7.0, 0.05),
                yaw,
                0.0,
                Shape::Cylinder,
            );
            // two arrow slits up the outward face, the upper one shorter
            for slit in 0..2 {
                let y = foot + height * (0.30 + slit as f32 * 0.32);
                self.push_instance(
                    keep.ring(angle, CASTLE_TOWER_RING + radius * 0.92, y),
                    [1.0, 3.0 - slit as f32 * 0.6, 0.55],
                    CASTLE_RECESS,
                    yaw,
                    0.0,
                    Shape::Cuboid,
                );
            }
        }

        if !shelled {
            // A ring even on a square tower: a crown corbelled out round over a
            // square shaft is a real form, and four straight runs a tower is
            // four instances a tower the ring cannot afford.
            self.push_instance(
                keep.ring(angle, CASTLE_TOWER_RING, head + CASTLE_PARAPET_HEIGHT * 0.38),
                [radius * 2.4, CASTLE_PARAPET_HEIGHT, radius * 2.4],
                keep.course(salt + 8.0, 0.06),
                yaw,
                0.0,
                Shape::Crenels,
            );
        }

        let roofed = !shelled && keep.vary_unit(salt + 9.0) < 0.72;
        let sill = head + CASTLE_PARAPET_HEIGHT * 0.55;
        if roofed {
            let pitch = 6.0 + keep.vary_unit(salt + 10.0) * 6.4 + keep.tier as f32 * 0.4;
            // wider than the parapet, so the eaves overhang and throw a shadow
            // line rather than continuing the shaft
            self.push_instance(
                keep.ring(angle, CASTLE_TOWER_RING, sill + pitch * 0.5),
                [radius * 2.74, pitch, radius * 2.74],
                keep.roof(salt + 11.0),
                yaw + keep.vary(salt + 12.0) * 0.5,
                0.0,
                Shape::Cone,
            );
            if full {
                self.push_instance(
                    keep.ring(angle, CASTLE_TOWER_RING, sill + pitch + 0.8),
                    [0.9, 2.4, 0.9],
                    keep.course(salt + 13.0, 0.10),
                    0.0,
                    0.0,
                    Shape::Cone,
                );
            }
            if banner {
                self.push_castle_banner(
                    keep,
                    keep.ring(angle, CASTLE_TOWER_RING, sill + pitch + 1.4),
                    5.0 + keep.tier as f32 * 0.8,
                    salt + 14.0,
                );
            }
        } else if banner {
            self.push_castle_banner(
                keep,
                keep.ring(angle, CASTLE_TOWER_RING, head + CASTLE_PARAPET_HEIGHT * 0.4),
                5.5 + keep.tier as f32 * 0.8,
                salt + 14.0,
            );
        }
    }

    /// One curtain run between two towers.
    fn draw_castle_curtain(&mut self, keep: &Keep, run: usize, mid: f32) {
        let salt = 80.0 + run as f32 * 6.0;
        // long enough to bury both ends inside the drums: a wall that stops
        // where the tower starts shows a seam from every angle
        let length = 2.0 * CASTLE_TOWER_RING * sin(keep.half_step()) + CASTLE_TOWER_RADIUS * 2.4;
        let height = CASTLE_WALL_HEIGHT + keep.tier as f32 * 0.45 + keep.vary(salt) * 1.5;
        let yaw = Keep::outward(mid);

        // Two courses, the lower one thicker. The batter is what makes it a
        // wall rather than a fence panel stood on end.
        self.push_instance(
            keep.ring(mid, keep.wall_radius, keep.deck + height * 0.28),
            [CASTLE_WALL_THICKNESS * 1.26, height * 0.64, length],
            keep.course(salt + 1.0, 0.05),
            yaw,
            0.0,
            Shape::Cuboid,
        );
        self.push_instance(
            keep.ring(mid, keep.wall_radius, keep.deck + height * 0.74),
            [CASTLE_WALL_THICKNESS, height * 0.54, length * 0.99],
            keep.course(salt + 2.0, 0.05),
            yaw,
            0.0,
            Shape::Cuboid,
        );
        // `Crenels` is a ring, so a straight parapet is that ring squashed
        // until its two long sides lie along the wall head. The merlons come
        // out unevenly spaced, which is closer to laid stone than a comb is.
        self.push_instance(
            keep.ring(
                mid,
                keep.wall_radius,
                keep.deck + height + CASTLE_PARAPET_HEIGHT * 0.38,
            ),
            [
                CASTLE_WALL_THICKNESS * 1.5,
                CASTLE_PARAPET_HEIGHT,
                length * 0.94,
            ],
            keep.course(salt + 3.0, 0.06),
            yaw,
            0.0,
            Shape::Crenels,
        );

        if !keep.detail.at_least(BodyDetail::Full) {
            return;
        }
        // A pier on some runs and a timber hoarding on others, never both, so
        // the ring is not one panel repeated round a circle.
        let along = keep.vary(salt + 5.0) * length * 0.24;
        let drift = [-sin(mid) * along, cos(mid) * along];
        if keep.quirk(salt + 4.0, 0.55) {
            let at = keep.ring(
                mid,
                keep.wall_radius + CASTLE_WALL_THICKNESS * 0.62,
                keep.deck + height * 0.34,
            );
            self.push_instance(
                [at[0] + drift[0], at[1], at[2] + drift[1]],
                [2.8, height * 0.88, 2.3],
                keep.course(salt + 6.0, 0.04),
                yaw,
                0.0,
                Shape::Frustum,
            );
        } else if keep.quirk(salt + 7.0, 0.5) {
            let at = keep.ring(
                mid,
                keep.wall_radius + CASTLE_WALL_THICKNESS * 0.9,
                keep.deck + height + CASTLE_PARAPET_HEIGHT * 0.9,
            );
            self.push_oriented(
                [at[0] + drift[0], at[1], at[2] + drift[1]],
                [3.4, 2.6, length * 0.34],
                CASTLE_TIMBER,
                Rotation::new(yaw, 0.0, keep.vary(salt + 8.0) * 0.12),
                0.0,
                Shape::Wedge,
            );
        }
    }

    /// The gatehouse: twin flanking turrets, a machicolated head, a recessed
    /// passage under a round-headed vault, a half-dropped portcullis and the
    /// ramp down the flank of the mound.
    fn draw_castle_gatehouse(&mut self, keep: &Keep, mid: f32) {
        let frontage =
            (2.0 * CASTLE_TOWER_RING * sin(keep.half_step()) + CASTLE_TOWER_RADIUS * 1.6) * 0.62;
        let height = CASTLE_WALL_HEIGHT + 4.0 + keep.tier as f32 * 0.9;
        let yaw = Keep::outward(mid);
        let full = keep.detail.at_least(BodyDetail::Full);

        // the block itself, deeper than the curtain it interrupts
        self.push_instance(
            keep.ring(mid, keep.wall_radius, keep.deck + height * 0.5),
            [CASTLE_WALL_THICKNESS * 2.2, height, frontage],
            keep.course(120.0, 0.04),
            yaw,
            0.0,
            Shape::Cuboid,
        );
        if full {
            // Machicolation: the head juts out over the passage so defenders
            // can drop things through it, and it puts a shadow line across the
            // one face of the keep anybody ever walks up to.
            self.push_instance(
                keep.ring(mid, keep.wall_radius + 0.8, keep.deck + height - 1.1),
                [CASTLE_WALL_THICKNESS * 3.0, 2.1, frontage * 1.06],
                keep.course(121.0, 0.07),
                yaw,
                0.0,
                Shape::Cuboid,
            );
        }
        self.push_instance(
            keep.ring(
                mid,
                keep.wall_radius,
                keep.deck + height + CASTLE_PARAPET_HEIGHT * 0.38,
            ),
            [
                CASTLE_WALL_THICKNESS * 2.6,
                CASTLE_PARAPET_HEIGHT,
                frontage * 1.04,
            ],
            keep.course(122.0, 0.05),
            yaw,
            0.0,
            Shape::Crenels,
        );

        // the passage, cut as a recess rather than painted on
        let opening = 7.6;
        let mouth = keep.wall_radius + CASTLE_WALL_THICKNESS * 0.95;
        self.push_instance(
            keep.ring(mid, mouth, keep.deck + opening * 0.40),
            [1.8, opening * 0.80, 5.6],
            CASTLE_RECESS,
            yaw,
            0.0,
            Shape::Cuboid,
        );
        // A round-headed vault over it: a cylinder rolled onto its side so its
        // axis runs through the wall, which is the way a barrel vault runs.
        self.push_oriented(
            keep.ring(mid, keep.wall_radius, keep.deck + opening * 0.66),
            [5.6, CASTLE_WALL_THICKNESS * 2.6, 5.6],
            CASTLE_RECESS,
            Rotation::new(yaw, 0.0, core::f32::consts::FRAC_PI_2),
            0.0,
            Shape::Cylinder,
        );
        if full {
            // portcullis, half dropped, its bars not quite even
            for bar in 0..4 {
                let across = (bar as f32 - 1.5) * 1.45;
                let at = keep.ring(mid, mouth + 0.6, keep.deck + opening * 0.52);
                self.push_instance(
                    [at[0] - sin(mid) * across, at[1], at[2] + cos(mid) * across],
                    [
                        0.5,
                        opening * (0.52 + keep.vary_unit(130.0 + bar as f32) * 0.1),
                        0.5,
                    ],
                    [0.13, 0.12, 0.12],
                    yaw,
                    0.0,
                    Shape::Cuboid,
                );
            }
            self.push_instance(
                keep.ring(mid, mouth + 0.6, keep.deck + opening * 0.80),
                [0.7, 0.7, 5.2],
                [0.13, 0.12, 0.12],
                yaw,
                0.0,
                Shape::Cuboid,
            );
        }

        // Twin flanking turrets, deliberately not a matched pair.
        for side in 0..2 {
            let salt = 140.0 + side as f32 * 9.0;
            let across = (side as f32 - 0.5) * frontage * 1.12;
            let girth = 3.1 + keep.vary_unit(salt) * 0.9;
            let rise = height + 3.0 + keep.vary_unit(salt + 1.0) * 4.0;
            let at = keep.ring(mid, keep.wall_radius + 0.6, keep.deck + rise * 0.5);
            let base = [at[0] - sin(mid) * across, at[1], at[2] + cos(mid) * across];
            self.push_instance(
                base,
                [girth * 2.0, rise, girth * 2.0],
                keep.course(salt + 2.0, 0.05),
                yaw,
                0.0,
                Shape::Cylinder,
            );
            if !full {
                continue;
            }
            self.push_instance(
                [
                    base[0],
                    keep.deck + rise + CASTLE_PARAPET_HEIGHT * 0.38,
                    base[2],
                ],
                [girth * 2.3, CASTLE_PARAPET_HEIGHT, girth * 2.3],
                keep.course(salt + 3.0, 0.06),
                yaw,
                0.0,
                Shape::Crenels,
            );
            let pitch = 5.4 + keep.vary_unit(salt + 4.0) * 3.6;
            self.push_instance(
                [
                    base[0],
                    keep.deck + rise + CASTLE_PARAPET_HEIGHT * 0.55 + pitch * 0.5,
                    base[2],
                ],
                [girth * 2.6, pitch, girth * 2.6],
                keep.roof(salt + 5.0),
                yaw + keep.vary(salt + 6.0) * 0.4,
                0.0,
                Shape::Cone,
            );
        }

        // The ramp down the flank of the mound: the only way up to the terrace
        // that is not a cliff, and it hides the join between mound and ground.
        let inner = keep.wall_radius + CASTLE_WALL_THICKNESS * 1.6;
        let outer = CASTLE_FOOTPRINT + 9.0;
        let run = outer - inner;
        let fall = clamp(keep.deck - (keep.foot + CASTLE_PLINTH_SINK), 3.0, 12.0);
        let slope = atan2(fall, run);
        let length = length3(run, fall, 0.0);
        let spine = keep.ring(mid, (inner + outer) * 0.5, keep.deck - fall * 0.5);
        self.push_oriented(
            spine,
            [7.4, 1.6, length],
            CASTLE_TIMBER,
            Rotation::new(Keep::radial(mid), slope, 0.0),
            0.0,
            Shape::Cuboid,
        );
        if full {
            for side in 0..2 {
                let across = (side as f32 - 0.5) * 8.6;
                self.push_oriented(
                    [
                        spine[0] - sin(mid) * across,
                        spine[1] + 0.7,
                        spine[2] + cos(mid) * across,
                    ],
                    [1.3, 2.6, length * 0.98],
                    keep.course(150.0 + side as f32, 0.05),
                    Rotation::new(Keep::radial(mid), slope, 0.0),
                    0.0,
                    Shape::Cuboid,
                );
            }
        }
    }

    // ---- the donjon --------------------------------------------------------

    /// The great tower in the middle. Drum or square is settled per keep, and
    /// it is the single biggest reason the player's fortress and the rival's
    /// are two buildings rather than one repainted.
    fn draw_castle_donjon(&mut self, keep: &Keep) {
        let height = CASTLE_KEEP_BASE_HEIGHT + keep.tier as f32 * CASTLE_KEEP_HEIGHT_PER_TIER;
        let width = keep.donjon;
        let head = keep.deck + height;
        let roof_pitch = 8.0 + keep.vary_unit(32.0) * 7.0 + keep.tier as f32 * 0.6;
        // one face of a square donjon always looks out over its own gate
        let body_yaw = Keep::outward(keep.bearing);

        if !keep.detail.at_least(BodyDetail::Reduced) {
            self.push_instance(
                [keep.x, keep.deck + height * 0.5, keep.z],
                [width, height, width],
                keep.stone,
                body_yaw,
                0.0,
                if keep.drum {
                    Shape::Cylinder
                } else {
                    Shape::Cuboid
                },
            );
            self.push_instance(
                [keep.x, head + roof_pitch * 0.5, keep.z],
                [width * 1.05, roof_pitch, width * 1.05],
                keep.roof(33.0),
                body_yaw,
                0.0,
                Shape::Cone,
            );
            return;
        }
        let full = keep.detail.at_least(BodyDetail::Full);

        if full {
            // Footing, sunk into the paving so the base has no seam. It stops
            // below the door so the stoop can land on top of it.
            self.push_instance(
                [keep.x, keep.deck + 1.0, keep.z],
                [width * 1.14, 4.0, width * 1.14],
                keep.course(34.0, 0.04),
                body_yaw,
                0.0,
                if keep.drum {
                    Shape::Cylinder
                } else {
                    Shape::Cuboid
                },
            );
        }
        if keep.drum {
            self.push_instance(
                [keep.x, keep.deck + height * 0.5, keep.z],
                [width, height, width],
                keep.course(35.0, 0.04),
                body_yaw,
                0.0,
                Shape::Cylinder,
            );
        } else {
            // two storeys with a course between them, so the mass steps in as
            // it rises instead of standing as one extruded square
            self.push_instance(
                [keep.x, keep.deck + height * 0.31, keep.z],
                [width, height * 0.66, width],
                keep.course(35.0, 0.04),
                body_yaw,
                0.0,
                Shape::Cuboid,
            );
            self.push_instance(
                [keep.x, keep.deck + height * 0.77, keep.z],
                [width * 0.92, height * 0.52, width * 0.92],
                keep.course(36.0, 0.05),
                body_yaw,
                0.0,
                Shape::Cuboid,
            );
        }

        let banded = if keep.drum {
            Shape::Cylinder
        } else {
            Shape::Cuboid
        };
        if full {
            self.push_instance(
                [keep.x, keep.deck + height * 0.44, keep.z],
                [width * 1.08, 1.6, width * 1.08],
                keep.course(37.0, 0.08),
                body_yaw,
                0.0,
                banded,
            );
            // a keep only earns a second string course once it is tall enough
            // to need one
            if keep.tier >= 3 {
                self.push_instance(
                    [keep.x, keep.deck + height * 0.74, keep.z],
                    [width * 1.03, 1.3, width * 1.03],
                    keep.course(38.0, 0.07),
                    body_yaw,
                    0.0,
                    banded,
                );
            }
            self.push_instance(
                [keep.x, head - 1.2, keep.z],
                [width * 1.16, 2.3, width * 1.16],
                keep.course(39.0, 0.05),
                body_yaw,
                0.0,
                banded,
            );
        }

        if keep.drum || !full {
            self.push_instance(
                [keep.x, head + CASTLE_PARAPET_HEIGHT * 0.38, keep.z],
                [width * 1.14, CASTLE_PARAPET_HEIGHT, width * 1.14],
                keep.course(40.0, 0.05),
                body_yaw,
                0.0,
                Shape::Crenels,
            );
        } else {
            // four straight runs, one per face: a ring parapet on a square keep
            // leaves its corners standing open
            for face in 0..4 {
                let bearing = keep.bearing + face as f32 * core::f32::consts::FRAC_PI_2;
                self.push_instance(
                    keep.ring(bearing, width * 0.5, head + CASTLE_PARAPET_HEIGHT * 0.38),
                    [2.9, CASTLE_PARAPET_HEIGHT, width * 1.08],
                    keep.course(41.0 + face as f32, 0.05),
                    Keep::outward(bearing),
                    0.0,
                    Shape::Crenels,
                );
            }
            // Angle buttresses, set just inside the corners so they overlap the
            // body rather than butting it. They are what a square keep has
            // instead of a curve to break its walls, and the frustum's taper
            // weathers them back into the corner as they rise.
            for face in 0..4 {
                let bearing = keep.bearing + (face as f32 + 0.5) * core::f32::consts::FRAC_PI_2;
                self.push_instance(
                    keep.ring(bearing, width * 0.66, keep.deck + height * 0.22),
                    [3.2, height * 0.44, 3.6],
                    keep.course(46.0 + face as f32, 0.05),
                    Keep::outward(bearing),
                    0.0,
                    Shape::Frustum,
                );
            }
        }

        // Corner turrets, running down the face rather than perched on the lid.
        // They are the one thing that stops a great tower ending in a flat top.
        if full && keep.tier >= 3 {
            let turrets = if keep.drum { 3 } else { 4 };
            for turret in 0..turrets {
                let salt = 55.0 + turret as f32 * 2.5;
                let bearing = if keep.drum {
                    keep.bearing
                        + turret as f32 * core::f32::consts::TAU / 3.0
                        + keep.vary(salt) * 0.3
                } else {
                    keep.bearing
                        + core::f32::consts::FRAC_PI_4
                        + turret as f32 * core::f32::consts::FRAC_PI_2
                };
                let out = if keep.drum { width * 0.50 } else { width * 0.62 };
                let girth = 3.0 + keep.vary_unit(salt + 0.6) * 1.3;
                let rise = 4.6 + keep.vary_unit(salt + 1.0) * 4.0 + keep.tier as f32 * 0.5;
                let bottom = keep.deck + height * (0.38 + keep.vary_unit(salt + 1.4) * 0.14);
                let top = head + rise;
                self.push_instance(
                    keep.ring(bearing, out, (bottom + top) * 0.5),
                    [girth * 2.0, top - bottom, girth * 2.0],
                    keep.course(salt + 2.0, 0.06),
                    Keep::outward(bearing),
                    0.0,
                    Shape::Cylinder,
                );
                self.push_instance(
                    keep.ring(bearing, out, top + girth * 0.9),
                    [girth * 2.5, girth * 2.4, girth * 2.5],
                    keep.roof(salt + 3.0),
                    Keep::outward(bearing) + keep.vary(salt + 4.0) * 0.4,
                    0.0,
                    Shape::Cone,
                );
            }
        }

        let sill = head + CASTLE_PARAPET_HEIGHT * 0.55;
        if keep.drum {
            self.push_instance(
                [keep.x, sill + roof_pitch * 0.5, keep.z],
                [width * 1.02, roof_pitch, width * 1.02],
                keep.roof(50.0),
                body_yaw,
                0.0,
                Shape::Cone,
            );
        } else {
            // a gable, so a square donjon never reads as a cone on a box
            self.push_instance(
                [keep.x, sill + roof_pitch * 0.5, keep.z],
                [width * 0.98, roof_pitch, width * 1.06],
                keep.roof(50.0),
                body_yaw + keep.vary(51.0) * 0.16,
                0.0,
                Shape::Wedge,
            );
        }

        if full {
            // Windows with sills. The ledge under an opening is what tells the
            // eye it is a window and not a dark patch.
            for opening in 0..3 {
                let salt = 60.0 + opening as f32 * 3.0;
                let bearing = if keep.drum {
                    keep.bearing + 1.1 + opening as f32 * 2.2 + keep.vary(salt) * 0.4
                } else {
                    keep.bearing + (opening as f32 + 1.0) * core::f32::consts::FRAC_PI_2
                };
                let y = keep.deck
                    + height * (0.44 + opening as f32 * 0.17 + keep.vary_unit(salt + 1.0) * 0.07);
                let tall = 3.2 + keep.vary_unit(salt + 2.0) * 1.8;
                self.push_instance(
                    keep.ring(bearing, width * 0.5, y),
                    [1.6, tall, 1.9],
                    CASTLE_RECESS,
                    Keep::outward(bearing),
                    0.0,
                    Shape::Cuboid,
                );
                self.push_instance(
                    keep.ring(bearing, width * 0.5 + 0.3, y - tall * 0.56),
                    [2.2, 0.7, 3.0],
                    keep.course(salt + 3.0, 0.08),
                    Keep::outward(bearing),
                    0.0,
                    Shape::Cuboid,
                );
            }
            // the door, on the face that looks at its own gate, under a hood,
            // starting where the footing stops and the stoop arrives
            self.push_instance(
                keep.ring(keep.bearing, width * 0.5, keep.deck + 6.0),
                [1.8, 5.8, 3.2],
                CASTLE_RECESS,
                Keep::outward(keep.bearing),
                0.0,
                Shape::Cuboid,
            );
            self.push_instance(
                keep.ring(keep.bearing, width * 0.5 + 0.6, keep.deck + 9.4),
                [2.6, 1.8, 4.6],
                keep.course(66.0, 0.06),
                Keep::outward(keep.bearing),
                0.0,
                Shape::Wedge,
            );
            // A stair turret up one flank of most drum keeps. A square keep
            // gets the chimney instead: a turret hung off an arbitrary bearing
            // sits inside a square body and only its cap clears the roof.
            if keep.drum && keep.quirk(67.0, 0.72) {
                let bearing = keep.bearing + 2.4 + keep.vary(68.0) * 0.6;
                let top = head + 3.0 + keep.vary_unit(69.0) * 3.0;
                self.push_instance(
                    keep.ring(bearing, width * 0.46, (keep.deck + top) * 0.5),
                    [5.2, top - keep.deck, 5.2],
                    keep.course(70.0, 0.06),
                    Keep::outward(bearing),
                    0.0,
                    Shape::Cylinder,
                );
                self.push_instance(
                    keep.ring(bearing, width * 0.46, top + 2.2),
                    [6.0, 5.0, 6.0],
                    keep.roof(71.0),
                    Keep::outward(bearing),
                    0.0,
                    Shape::Cone,
                );
            } else {
                let bearing = keep.bearing - 1.9 + keep.vary(72.0) * 0.5;
                let stack = sill + roof_pitch * 0.55;
                let hearth = keep.deck + height * 0.6;
                self.push_castle_shaft(
                    keep.ring(bearing, width * 0.30, hearth),
                    stack - hearth,
                    3.4,
                    16.0,
                    keep.course(73.0, 0.07),
                    Keep::outward(bearing),
                );
                self.push_instance(
                    keep.ring(bearing, width * 0.30, stack + 0.6),
                    [4.2, 1.4, 4.2],
                    CASTLE_RECESS,
                    Keep::outward(bearing),
                    0.0,
                    Shape::Cuboid,
                );
            }
        }

        // The mast on the apex, with the mana beacon as its finial. The beacon
        // stays small: bloom turns anything larger into a hole in the sky.
        let apex = sill + roof_pitch;
        let mast = 6.0 + keep.tier as f32 * 1.2;
        if full {
            self.push_castle_banner(keep, [keep.x, apex - 0.8, keep.z], mast, 90.0);
        }
        self.push_instance(
            [keep.x, apex - 0.8 + mast, keep.z],
            [2.3, 2.3, 2.3],
            [0.62, 0.86, 1.0],
            0.0,
            1.0,
            Shape::Sphere,
        );
    }

    // ---- the ward ----------------------------------------------------------

    /// The clutter in the inner ward. Without it the terrace is a paved disc
    /// with buildings stood on it, which is what makes a keep read as a model
    /// of one. Every piece is optional and hashed, so two keeps are not tidied
    /// the same way.
    fn draw_castle_ward(&mut self, keep: &Keep) {
        // The stoop up to the donjon door lands on top of its footing. The
        // inner ward is only as wide as the curtain leaves it, so the treads
        // are spaced across whatever gap there actually is rather than at a
        // fixed radius that a four-tower enceinte would put inside its wall.
        let inner = keep.donjon * 0.57;
        let gap = max(keep.wall_radius - CASTLE_WALL_THICKNESS * 0.9 - inner, 2.4);
        for tread in 0..2 {
            let rise = 1.6 + tread as f32 * 1.4;
            self.push_instance(
                keep.ring(
                    keep.bearing,
                    inner + gap * (0.62 - tread as f32 * 0.36),
                    keep.deck + rise * 0.5,
                ),
                [gap * 0.34, rise, 6.4 - tread as f32 * 0.8],
                keep.course(160.0 + tread as f32, 0.06),
                Keep::outward(keep.bearing),
                0.0,
                Shape::Cuboid,
            );
        }

        // Everything else stands on the outer terrace, against the curtain.
        // That berm is clear at any tower count, where the inner ward is not,
        // and the bays chosen are curtain runs — never the gate, never a tower.
        let step = core::f32::consts::TAU / keep.towers as f32;
        let runs = keep.towers - 1;
        let turn = (keep.vary_unit(159.0) * runs as f32) as usize;
        let mut bay = [0.0f32; 3];
        for (slot, angle) in bay.iter_mut().enumerate() {
            *angle = keep.bearing + (((slot + turn) % runs) + 1) as f32 * step;
        }
        let berm = (keep.wall_radius + CASTLE_WALL_THICKNESS + CASTLE_FOOTPRINT) * 0.5;

        // a well, roofed on a single post
        let well = bay[0] + keep.vary(161.0) * 0.2;
        self.push_instance(
            keep.ring(well, berm, keep.deck + 1.6),
            [4.0, 3.0, 4.0],
            keep.course(162.0, 0.07),
            Keep::outward(well),
            0.0,
            Shape::Cylinder,
        );
        self.push_instance(
            keep.ring(well, berm, keep.deck + 3.6),
            [0.6, 4.4, 3.6],
            CASTLE_TIMBER,
            Keep::outward(well),
            0.0,
            Shape::Cuboid,
        );
        self.push_oriented(
            keep.ring(well, berm, keep.deck + 6.2),
            [4.6, 2.0, 5.0],
            [
                CASTLE_TIMBER[0] * 0.8,
                CASTLE_TIMBER[1] * 0.8,
                CASTLE_TIMBER[2] * 0.8,
            ],
            Rotation::new(Keep::outward(well), 0.0, keep.vary(163.0) * 0.14),
            0.0,
            Shape::Wedge,
        );

        // a brazier, lit, and small enough that the bloom stays a flame
        let fire = bay[1] + keep.vary(164.0) * 0.2;
        self.push_instance(
            keep.ring(fire, berm, keep.deck + 1.4),
            [2.6, 2.6, 2.6],
            [0.20, 0.18, 0.17],
            0.0,
            0.0,
            Shape::Cylinder,
        );
        let flare = 0.78 + sin(self.session.elapsed * 7.0 + keep.bearing * 3.0) * 0.22;
        self.push_instance(
            keep.ring(fire, berm, keep.deck + 3.4),
            [2.0 * flare, 3.4 * flare, 2.0 * flare],
            [1.0, 0.62, 0.22],
            keep.bearing + self.session.elapsed * 1.7,
            1.0,
            Shape::Cone,
        );

        // stores against the curtain: barrels and a crate on some keeps, a heap
        // of quarried stone that never got used on others
        if keep.quirk(165.0, 0.7) {
            let stores = bay[2] + keep.vary(166.0) * 0.2;
            for barrel in 0..2 {
                let salt = 167.0 + barrel as f32 * 2.0;
                self.push_oriented(
                    keep.ring(
                        stores + barrel as f32 * 0.18,
                        berm + keep.vary(salt) * 1.4,
                        keep.deck + 1.6,
                    ),
                    [2.2, 3.0, 2.2],
                    CASTLE_TIMBER,
                    Rotation::new(
                        Keep::outward(stores),
                        keep.vary(salt + 1.0) * 0.12,
                        keep.vary(salt + 1.5) * 0.12,
                    ),
                    0.0,
                    Shape::Cylinder,
                );
            }
            self.push_oriented(
                keep.ring(stores - 0.34, berm - 1.4, keep.deck + 1.3),
                [3.0, 2.4, 2.6],
                [
                    CASTLE_TIMBER[0] * 1.1,
                    CASTLE_TIMBER[1] * 1.1,
                    CASTLE_TIMBER[2] * 1.1,
                ],
                Rotation::new(Keep::outward(stores), 0.0, keep.vary(170.0) * 0.16),
                0.0,
                Shape::Cuboid,
            );
        }
        if keep.quirk(171.0, 0.6) {
            let pile = bay[2] - 0.5 + keep.vary(172.0) * 0.2;
            for lump in 0..2 {
                let salt = 173.0 + lump as f32 * 2.0;
                let size = 2.4 + keep.vary_unit(salt) * 1.8;
                self.push_oriented(
                    keep.ring(pile + lump as f32 * 0.24, berm, keep.deck + size * 0.35),
                    [size, size * 0.8, size * 1.1],
                    keep.course(salt + 1.0, 0.06),
                    Rotation::new(
                        pile,
                        keep.vary(salt + 2.0) * 0.5,
                        keep.vary(salt + 3.0) * 0.5,
                    ),
                    0.0,
                    Shape::Boulder,
                );
            }
        }
    }

    // ---- the ruin ----------------------------------------------------------

    /// What is left after a keep comes down: the mound, stubs of curtain at
    /// whatever height they snapped at, tower stumps, one drum lying where it
    /// toppled, a fallen length of parapet, rubble and charred roof timbers.
    pub(super) fn draw_castle_ruin(&mut self, wizard: usize, ground: f32) {
        let keep = self.keep_of(wizard, ground);
        // Fire-blackened. The mound is earth and rock, and it is the one part
        // of a keep that survives being knocked down.
        let ash = [
            keep.stone[0] * 0.44 + 0.03,
            keep.stone[1] * 0.42 + 0.03,
            keep.stone[2] * 0.42 + 0.03,
        ];
        let crest = keep.deck - 2.6;
        let mound = crest - keep.foot;
        self.push_instance(
            [keep.x, (crest + keep.foot) * 0.5, keep.z],
            [CASTLE_FOOTPRINT * 1.6, mound, CASTLE_FOOTPRINT * 1.6],
            ash,
            keep.bearing,
            0.0,
            Shape::Cylinder,
        );
        if !keep.detail.at_least(BodyDetail::Reduced) {
            return;
        }
        // the spread of the collapse, wider than the mound and lower
        self.push_instance(
            [keep.x, keep.foot + mound * 0.32, keep.z],
            [CASTLE_FOOTPRINT * 2.1, mound * 0.62, CASTLE_FOOTPRINT * 2.1],
            [ash[0] * 0.86, ash[1] * 0.86, ash[2] * 0.84],
            -keep.bearing,
            0.0,
            Shape::Cylinder,
        );

        let full = keep.detail.at_least(BodyDetail::Full);
        // Wall stubs at whatever height they broke off at, some of them out of
        // plumb. A ruin drawn to one height is a low box.
        let stubs = if full { 6 } else { 3 };
        for stub in 0..stubs {
            let salt = 200.0 + stub as f32 * 7.0;
            let angle = keep.bearing
                + (stub as f32 + 0.35) * core::f32::consts::TAU / stubs as f32
                + keep.vary(salt) * 0.45;
            let rise = 1.8 + keep.vary_unit(salt + 1.0) * 7.4;
            let length = 5.0 + keep.vary_unit(salt + 2.0) * 9.0;
            let shade = keep.vary(salt + 5.0) * 0.04;
            self.push_oriented(
                keep.ring(
                    angle,
                    CASTLE_TOWER_RING * (0.82 + keep.vary_unit(salt + 3.0) * 0.3),
                    crest + rise * 0.34,
                ),
                [
                    CASTLE_WALL_THICKNESS * (0.9 + keep.vary_unit(salt + 4.0) * 0.6),
                    rise,
                    length,
                ],
                [ash[0] + shade, ash[1] + shade, ash[2] + shade],
                Rotation::new(
                    Keep::outward(angle),
                    keep.vary(salt + 6.0) * 0.16,
                    keep.vary(salt + 7.0) * 0.22,
                ),
                0.0,
                if keep.quirk(salt + 8.0, 0.5) {
                    Shape::Cuboid
                } else {
                    Shape::Frustum
                },
            );
        }

        let rubble = if full { 9 } else { 3 };
        for lump in 0..rubble {
            let salt = 260.0 + lump as f32 * 4.0;
            let angle = keep.vary(salt) * core::f32::consts::PI;
            let out = keep.vary_unit(salt + 1.0) * (CASTLE_FOOTPRINT + 6.0);
            let size = 2.2 + keep.vary_unit(salt + 2.0) * 5.4;
            let shade = keep.vary(salt + 3.0) * 0.05;
            self.push_oriented(
                keep.ring(angle, out, crest + size * 0.2),
                [size, size * 0.78, size * 1.16],
                [ash[0] + shade, ash[1] + shade, ash[2] + shade],
                Rotation::new(
                    angle,
                    keep.vary(salt + 4.0) * 0.6,
                    keep.vary(salt + 5.0) * 0.6,
                ),
                0.0,
                Shape::Boulder,
            );
        }
        if !full {
            return;
        }

        // tower stumps, snapped at three different heights
        for stump in 0..3 {
            let salt = 300.0 + stump as f32 * 6.0;
            let angle = keep.bearing + (stump as f32 + 0.8) * 2.1 + keep.vary(salt) * 0.5;
            let rise = 2.6 + keep.vary_unit(salt + 1.0) * 8.0;
            let girth = CASTLE_TOWER_RADIUS * (0.9 + keep.vary_unit(salt + 2.0) * 0.4);
            self.push_oriented(
                keep.ring(angle, CASTLE_TOWER_RING * 0.94, crest + rise * 0.4),
                [girth * 2.0, rise, girth * 2.0],
                keep.course(salt + 3.0, 0.05),
                Rotation::new(
                    Keep::outward(angle),
                    keep.vary(salt + 4.0) * 0.2,
                    keep.vary(salt + 5.0) * 0.24,
                ),
                0.0,
                Shape::Cylinder,
            );
        }
        // one drum that went over whole and is lying where it fell
        let fallen = keep.bearing + 1.3 + keep.vary(320.0) * 1.2;
        self.push_oriented(
            keep.ring(fallen, CASTLE_FOOTPRINT * 0.72, crest + 2.2),
            [7.4, 15.0, 7.4],
            keep.course(321.0, 0.05),
            Rotation::new(
                Keep::radial(fallen),
                core::f32::consts::FRAC_PI_2 - 0.18 + keep.vary(322.0) * 0.2,
                keep.vary(323.0) * 0.3,
            ),
            0.0,
            Shape::Cylinder,
        );
        // a length of parapet that came down in one piece and is half buried
        let slab = keep.bearing - 2.2 + keep.vary(330.0) * 1.0;
        self.push_oriented(
            keep.ring(slab, CASTLE_FOOTPRINT * 0.62, crest + 0.6),
            [4.0, CASTLE_PARAPET_HEIGHT, 10.0],
            [ash[0] * 1.12, ash[1] * 1.12, ash[2] * 1.12],
            Rotation::new(
                Keep::outward(slab),
                keep.vary(331.0) * 0.4,
                1.1 + keep.vary(332.0) * 0.4,
            ),
            0.0,
            Shape::Crenels,
        );
        // charred roof timbers, thrown clear and leaning on the rubble
        for beam in 0..4 {
            let salt = 340.0 + beam as f32 * 5.0;
            let angle = keep.vary(salt) * core::f32::consts::PI;
            let out = 6.0 + keep.vary_unit(salt + 1.0) * (CASTLE_FOOTPRINT - 4.0);
            let length = 7.0 + keep.vary_unit(salt + 2.0) * 8.0;
            self.push_oriented(
                keep.ring(angle, out, crest + 1.6 + keep.vary_unit(salt + 3.0) * 2.0),
                [0.9, length, 0.9],
                [0.11, 0.09, 0.08],
                Rotation::new(
                    angle,
                    1.1 + keep.vary(salt + 4.0) * 0.45,
                    keep.vary(salt + 5.0) * 0.7,
                ),
                0.0,
                Shape::Cylinder,
            );
        }
    }

    // ---- creatures ---------------------------------------------------------
}
