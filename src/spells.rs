//! Casting. `cast` validates and charges, then dispatches to one function per
//! spell — the chain of `else if`s this replaced was where every new spell
//! went to hide.

use crate::math::*;
use crate::terrain::clamp_to_world;
use crate::types::*;
use crate::world::*;

/// Where a spell leaves the caster's hand, along their facing.
const MUZZLE_OFFSET: f32 = 6.0;
const MUZZLE_DROP: f32 = 2.0;

const FIREBOLT_SPEED: f32 = 220.0;
const FIREBOLT_PLAYER_DAMAGE: f32 = 34.0;
const FIREBOLT_BLAST_RADIUS: f32 = 20.0;
const FIREBOLT_LIFETIME: f32 = 3.2;

const LIGHTNING_SEEK_RANGE: f32 = 340.0;
/// A target must be within this cosine of straight ahead to be chained to.
const LIGHTNING_AIM_TOLERANCE: f32 = 0.35;
const LIGHTNING_CHAIN_RANGE: f32 = 150.0;
const LIGHTNING_MAX_HOPS: i32 = 4;
const LIGHTNING_DAMAGE: f32 = 52.0;
const LIGHTNING_BLAST_RADIUS: f32 = 16.0;

const CRATER_RANGE: f32 = 420.0;
const CRATER_RADIUS: f32 = 46.0;
const CRATER_DEPTH: f32 = 20.0;
const CRATER_DAMAGE: f32 = 46.0;

const VOLCANO_RADIUS: f32 = 60.0;
const VOLCANO_HEIGHT: f32 = 62.0;
const VOLCANO_DAMAGE: f32 = 60.0;
const VOLCANO_EMBER_COUNT: i32 = 90;

/// The quake walks outward in evenly spaced pulses that weaken as they go.
const QUAKE_PULSES: i32 = 7;
const QUAKE_PULSE_SPACING: f32 = 52.0;
const QUAKE_RADIUS: f32 = 40.0;
const QUAKE_BASE_AMOUNT: f32 = 13.0;
const QUAKE_FALLOFF_PER_PULSE: f32 = 0.9;
const QUAKE_DAMAGE: f32 = 34.0;

const METEOR_RANGE: f32 = 520.0;
const METEOR_SPAWN_HEIGHT: f32 = 420.0;
const METEOR_LATERAL_OFFSET: f32 = 60.0;
const METEOR_FALL_SPEED: f32 = 240.0;
const METEOR_DAMAGE: f32 = 150.0;
const METEOR_BLAST_RADIUS: f32 = 78.0;

const WARD_DURATION: f32 = 10.0;
const HASTE_DURATION: f32 = 12.0;
const MEND_AMOUNT: f32 = 55.0;

/// Orbs within this of the caster are possessed.
const CLAIM_RADIUS: f32 = 200.0;
/// Share of possessed mana that permanently widens the personal pool.
const CLAIM_POOL_GAIN: f32 = 0.28;
/// Share refunded immediately as castable mana.
const CLAIM_MANA_REFUND: f32 = 0.42;

const WRAITH_SUMMON_RANGE: f32 = 260.0;
const WRAITH_LIFETIME: f32 = 45.0;

const SUNBURST_NEST_DAMAGE: f32 = 120.0;
const SUNBURST_CREATURE_DAMAGE: f32 = 200.0;
/// Height the smiting beams come down from.
const SUNBURST_BEAM_HEIGHT: f32 = 260.0;

/// You must be this close to your own keep to raise it.
const FORTRESS_CAST_RANGE: f32 = 210.0;
const FORTRESS_HEALTH_PER_TIER: f32 = 220.0;

impl World {
    /// Attempts a cast. Returns whether it actually went off.
    pub fn cast(&mut self, wizard: usize, spell: Spell) -> bool {
        if !self.can_cast(wizard, spell) {
            return false;
        }
        let price = self.spell_price(spell, wizard);
        if self.wizards.mana[wizard] < price {
            return false;
        }
        self.wizards.mana[wizard] -= price;
        if wizard == PLAYER {
            self.spells.cooldown_remaining[spell.slot()] = spell.cooldown();
        }
        self.invoke(wizard, spell);
        true
    }

    /// Everything that can stop a cast before any mana is spent.
    fn can_cast(&self, wizard: usize, spell: Spell) -> bool {
        if wizard == PLAYER {
            if !self.spells.unlocked[spell.slot()] {
                return false;
            }
            if self.spells.cooldown_remaining[spell.slot()] > 0.0 {
                return false;
            }
        }
        // Fortress only works standing over your own intact keep, and only
        // while there is a tier left to buy.
        if spell == Spell::Fortress {
            if self.castles.health[wizard] <= 0.0 || self.castles.tier[wizard] >= MAX_CASTLE_TIER {
                return false;
            }
            let to_keep = [
                self.castles.pos_x[wizard] - self.wizards.pos_x[wizard],
                self.castles.pos_y[wizard] - self.wizards.pos_y[wizard],
                self.castles.pos_z[wizard] - self.wizards.pos_z[wizard],
            ];
            if length_sq3(to_keep[0], to_keep[1], to_keep[2])
                > FORTRESS_CAST_RANGE * FORTRESS_CAST_RANGE
            {
                return false;
            }
        }
        true
    }

    fn invoke(&mut self, wizard: usize, spell: Spell) {
        match spell {
            Spell::Firebolt => self.cast_firebolt(wizard),
            Spell::ChainLightning => self.cast_chain_lightning(wizard),
            Spell::Crater => self.cast_crater(wizard),
            Spell::Volcano => self.cast_volcano(wizard),
            Spell::Earthquake => self.cast_earthquake(wizard),
            Spell::Meteor => self.cast_meteor(wizard),
            Spell::Ward => self.cast_ward(wizard),
            Spell::Mend => self.cast_mend(wizard),
            Spell::Haste => self.cast_haste(wizard),
            Spell::Claim => self.cast_claim(wizard),
            Spell::SummonWraith => self.cast_summon_wraith(wizard),
            Spell::Fortress => self.cast_fortress(wizard),
            Spell::Sunburst => self.cast_sunburst(wizard),
        }
    }

    // ---- helpers shared by several spells ----------------------------------
    fn position_of(&self, wizard: usize) -> [f32; 3] {
        [
            self.wizards.pos_x[wizard],
            self.wizards.pos_y[wizard],
            self.wizards.pos_z[wizard],
        ]
    }

    fn muzzle(&self, wizard: usize, facing: [f32; 3]) -> [f32; 3] {
        let origin = self.position_of(wizard);
        [
            origin[0] + facing[0] * MUZZLE_OFFSET,
            origin[1] - MUZZLE_DROP + facing[1] * MUZZLE_OFFSET,
            origin[2] + facing[2] * MUZZLE_OFFSET,
        ]
    }

    /// Where the caster is aiming on the ground, clamped inside the world.
    fn aim_point(&self, wizard: usize, facing: [f32; 3], range: f32) -> (f32, f32) {
        let origin = self.position_of(wizard);
        let distance = self.distance_to_ground(origin, facing, range);
        (
            clamp_to_world(origin[0] + facing[0] * distance),
            clamp_to_world(origin[2] + facing[2] * distance),
        )
    }

    /// Shakes the camera only when the player was the caster; a rival levelling
    /// a mountain on the far side of the map should not rattle your screen.
    fn shake_if_player(&mut self, wizard: usize, amount: f32) {
        if wizard == PLAYER {
            self.session.screen_shake = amount;
        }
    }

    // ---- the spells --------------------------------------------------------
    fn cast_firebolt(&mut self, wizard: usize) {
        let facing = self.facing_of(wizard);
        let muzzle = self.muzzle(wizard, facing);
        // rivals scale up with the realm; the player's bolt is flat
        let damage = if wizard == PLAYER {
            FIREBOLT_PLAYER_DAMAGE
        } else {
            11.0 + self.session.realm as f32 * 2.3
        };
        self.spawn_projectile(
            ProjectileKind::Firebolt,
            muzzle,
            [
                facing[0] * FIREBOLT_SPEED,
                facing[1] * FIREBOLT_SPEED,
                facing[2] * FIREBOLT_SPEED,
            ],
            Faction::of_wizard(wizard),
            damage,
            FIREBOLT_BLAST_RADIUS,
            FIREBOLT_LIFETIME,
        );
        self.emit_sound(SoundCue::Cast, muzzle);
    }

    /// Nearest hostile creature that is both in range and roughly in front.
    fn lightning_first_target(&self, wizard: usize, facing: [f32; 3]) -> Option<usize> {
        let attacker = Faction::of_wizard(wizard);
        let origin = self.position_of(wizard);
        let mut best = None;
        let mut best_dist_sq = LIGHTNING_SEEK_RANGE * LIGHTNING_SEEK_RANGE;
        for i in 0..MAX_CREATURES {
            if !self.creatures.alive[i] || self.creatures.faction[i] == attacker {
                continue;
            }
            let offset = [
                self.creatures.pos_x[i] - origin[0],
                self.creatures.pos_y[i] - origin[1],
                self.creatures.pos_z[i] - origin[2],
            ];
            let dist_sq = length_sq3(offset[0], offset[1], offset[2]);
            if dist_sq > best_dist_sq {
                continue;
            }
            let inv = 1.0 / sqrt(dist_sq);
            let alignment =
                offset[0] * inv * facing[0] + offset[1] * inv * facing[1] + offset[2] * inv * facing[2];
            if alignment < LIGHTNING_AIM_TOLERANCE {
                continue;
            }
            best_dist_sq = dist_sq;
            best = Some(i);
        }
        best
    }

    fn lightning_next_hop(&self, from: [f32; 3], current: usize, attacker: Faction) -> Option<usize> {
        let mut best = None;
        let mut best_dist_sq = LIGHTNING_CHAIN_RANGE * LIGHTNING_CHAIN_RANGE;
        for i in 0..MAX_CREATURES {
            if !self.creatures.alive[i] || i == current || self.creatures.faction[i] == attacker {
                continue;
            }
            let dist_sq = length_sq3(
                self.creatures.pos_x[i] - from[0],
                self.creatures.pos_y[i] - from[1],
                self.creatures.pos_z[i] - from[2],
            );
            if dist_sq < best_dist_sq {
                best_dist_sq = dist_sq;
                best = Some(i);
            }
        }
        best
    }

    fn cast_chain_lightning(&mut self, wizard: usize) {
        let attacker = Faction::of_wizard(wizard);
        let facing = self.facing_of(wizard);
        let origin = self.position_of(wizard);
        let mut from = origin;
        match self.lightning_first_target(wizard, facing) {
            Some(first) => {
                let mut current = Some(first);
                for _ in 0..LIGHTNING_MAX_HOPS {
                    let Some(index) = current else { break };
                    let at = [
                        self.creatures.pos_x[index],
                        self.creatures.pos_y[index],
                        self.creatures.pos_z[index],
                    ];
                    self.spawn_arc(from, at);
                    self.apply_blast(at, LIGHTNING_BLAST_RADIUS, LIGHTNING_DAMAGE, attacker);
                    from = at;
                    current = self.lightning_next_hop(at, index, attacker);
                }
            }
            None => {
                // nothing to chain through, so it earths itself
                let distance = self.distance_to_ground(origin, facing, LIGHTNING_SEEK_RANGE);
                let impact = [
                    origin[0] + facing[0] * distance,
                    origin[1] + facing[1] * distance,
                    origin[2] + facing[2] * distance,
                ];
                self.spawn_arc(from, impact);
                self.apply_blast(impact, 26.0, 40.0, attacker);
            }
        }
        self.emit_sound(SoundCue::Zap, origin);
    }

    fn cast_crater(&mut self, wizard: usize) {
        let facing = self.facing_of(wizard);
        let (x, z) = self.aim_point(wizard, facing, CRATER_RANGE);
        self.deform(x, z, CRATER_RADIUS, CRATER_DEPTH, DeformKind::Crater);
        let ground = self.height_at(x, z);
        self.apply_blast(
            [x, ground, z],
            54.0,
            CRATER_DAMAGE,
            Faction::of_wizard(wizard),
        );
        let ground = self.height_at(x, z);
        self.spawn_burst([x, ground + 6.0, z], 44, 22.0, 6.0, [0.75, 0.6, 0.45], 1.4);
        self.shake_if_player(wizard, 0.9);
        let ground = self.height_at(x, z);
        self.emit_sound(SoundCue::Boom, [x, ground, z]);
    }

    fn cast_volcano(&mut self, wizard: usize) {
        let facing = self.facing_of(wizard);
        let (x, z) = self.aim_point(wizard, facing, CRATER_RANGE);
        self.deform(x, z, VOLCANO_RADIUS, VOLCANO_HEIGHT, DeformKind::Cone);
        let ground = self.height_at(x, z);
        self.apply_blast(
            [x, ground, z],
            66.0,
            VOLCANO_DAMAGE,
            Faction::of_wizard(wizard),
        );
        for _ in 0..VOLCANO_EMBER_COUNT {
            let angle = self.rng.range(0.0, core::f32::consts::TAU);
            let spread = self.rng.range(20.0, 70.0);
            let vertical = self.rng.range(40.0, 105.0);
            let life = self.rng.range(1.4, 3.0);
            let size = self.rng.range(4.0, 9.0);
            let heat = self.rng.range(0.25, 0.6);
            let ground = self.height_at(x, z);
            self.spawn_particle(
                [x, ground + 30.0, z],
                [cos(angle) * spread * 0.4, vertical, sin(angle) * spread * 0.4],
                life,
                size,
                [1.0, heat, 0.08],
                -30.0,
                0.5,
            );
        }
        self.shake_if_player(wizard, 1.3);
        let ground = self.height_at(x, z);
        self.emit_sound(SoundCue::Rumble, [x, ground, z]);
    }

    fn cast_earthquake(&mut self, wizard: usize) {
        let attacker = Faction::of_wizard(wizard);
        let facing = self.facing_of(wizard);
        let origin = self.position_of(wizard);
        for pulse in 1..=QUAKE_PULSES {
            let distance = pulse as f32 * QUAKE_PULSE_SPACING;
            let x = clamp_to_world(origin[0] + facing[0] * distance);
            let z = clamp_to_world(origin[2] + facing[2] * distance);
            let strength = QUAKE_BASE_AMOUNT - pulse as f32 * QUAKE_FALLOFF_PER_PULSE;
            self.deform(x, z, QUAKE_RADIUS, strength, DeformKind::Ripple);
            let ground = self.height_at(x, z);
            self.apply_blast([x, ground, z], 46.0, QUAKE_DAMAGE, attacker);
            let ground = self.height_at(x, z);
            self.spawn_burst([x, ground + 3.0, z], 14, 15.0, 5.0, [0.65, 0.5, 0.36], 1.1);
        }
        self.shake_if_player(wizard, 1.5);
        self.emit_sound(SoundCue::Quake, origin);
    }

    fn cast_meteor(&mut self, wizard: usize) {
        let facing = self.facing_of(wizard);
        let (x, z) = self.aim_point(wizard, facing, METEOR_RANGE);
        // launched up-range so it arcs in rather than dropping straight down
        let lateral = METEOR_LATERAL_OFFSET * 0.55;
        self.spawn_projectile(
            ProjectileKind::Meteor,
            [x - METEOR_LATERAL_OFFSET, METEOR_SPAWN_HEIGHT, z - METEOR_LATERAL_OFFSET],
            [lateral, -METEOR_FALL_SPEED, lateral],
            Faction::of_wizard(wizard),
            METEOR_DAMAGE,
            METEOR_BLAST_RADIUS,
            6.0,
        );
        self.emit_sound(SoundCue::Incoming, [x, 300.0, z]);
    }

    fn cast_ward(&mut self, wizard: usize) {
        self.wizards.ward_remaining[wizard] = WARD_DURATION;
        let at = self.position_of(wizard);
        self.emit_sound(SoundCue::Shimmer, at);
    }

    fn cast_haste(&mut self, wizard: usize) {
        self.wizards.haste_remaining[wizard] = HASTE_DURATION;
        let at = self.position_of(wizard);
        self.emit_sound(SoundCue::Shimmer, at);
    }

    fn cast_mend(&mut self, wizard: usize) {
        self.wizards.health[wizard] =
            min(self.wizards.health[wizard] + MEND_AMOUNT, WIZARD_MAX_HEALTH);
        let at = self.position_of(wizard);
        for _ in 0..30 {
            let offset = [
                self.rng.range(-8.0, 8.0),
                self.rng.range(-6.0, 6.0),
                self.rng.range(-8.0, 8.0),
            ];
            let rise = self.rng.range(6.0, 20.0);
            self.spawn_particle(
                [at[0] + offset[0], at[1] + offset[1], at[2] + offset[2]],
                [0.0, rise, 0.0],
                1.0,
                3.0,
                [0.4, 1.0, 0.6],
                4.0,
                1.0,
            );
        }
        self.emit_sound(SoundCue::Heal, at);
    }

    /// Possesses nearby unclaimed orbs, turning them white and marking them
    /// for your balloons. Possessing mana also permanently widens your own
    /// pool, which is the only way to afford the expensive spells later.
    fn cast_claim(&mut self, wizard: usize) {
        let owner = Faction::of_wizard(wizard);
        let at = self.position_of(wizard);
        let mut claimed = 0;
        let mut total = 0.0f32;
        for i in 0..MAX_ORBS {
            if !self.orbs.alive[i] || self.orbs.carried_by[i].is_some() {
                continue;
            }
            if self.orbs.claimed_by[i] == owner {
                continue;
            }
            let offset = [
                self.orbs.pos_x[i] - at[0],
                self.orbs.pos_y[i] - at[1],
                self.orbs.pos_z[i] - at[2],
            ];
            if length_sq3(offset[0], offset[1], offset[2]) > CLAIM_RADIUS * CLAIM_RADIUS {
                continue;
            }
            self.orbs.claimed_by[i] = owner;
            claimed += 1;
            total += self.orbs.amount[i];
            let orb_at = [self.orbs.pos_x[i], self.orbs.pos_y[i], self.orbs.pos_z[i]];
            for _ in 0..7 {
                let drift = [
                    self.rng.range(-9.0, 9.0),
                    self.rng.range(2.0, 12.0),
                    self.rng.range(-9.0, 9.0),
                ];
                self.spawn_particle(orb_at, drift, 0.55, 2.0, [0.75, 0.9, 1.0], 0.0, 1.2);
            }
        }
        if claimed == 0 {
            return;
        }
        self.wizards.mana_cap_bonus[wizard] += total * CLAIM_POOL_GAIN;
        self.wizards.mana[wizard] += total * CLAIM_MANA_REFUND;
        let cap = self.mana_cap(wizard);
        self.wizards.mana[wizard] = min(self.wizards.mana[wizard], cap);
        if wizard == PLAYER {
            self.emit_sound(SoundCue::Release, at);
        }
    }

    fn cast_summon_wraith(&mut self, wizard: usize) {
        let facing = self.facing_of(wizard);
        let (x, z) = self.aim_point(wizard, facing, WRAITH_SUMMON_RANGE);
        if let Some(index) =
            self.spawn_creature(CreatureKind::Wraith, x, z, Faction::of_wizard(wizard))
        {
            self.creatures.timer[index] = WRAITH_LIFETIME;
            let ground = self.height_at(x, z);
            self.spawn_burst([x, ground + 12.0, z], 30, 14.0, 4.0, [0.55, 0.35, 1.0], 1.2);
        }
        let ground = self.height_at(x, z);
        self.emit_sound(SoundCue::Whoosh, [x, ground, z]);
    }

    fn cast_fortress(&mut self, wizard: usize) {
        self.castles.tier[wizard] += 1;
        self.castles.max_health[wizard] += FORTRESS_HEALTH_PER_TIER;
        self.castles.health[wizard] = self.castles.max_health[wizard];
        let at = [
            self.castles.pos_x[wizard],
            self.castles.pos_y[wizard] + 26.0,
            self.castles.pos_z[wizard],
        ];
        self.spawn_burst(at, 70, 30.0, 12.0, [0.55, 0.85, 1.0], 2.2);
        if wizard == PLAYER {
            self.session.screen_shake = 0.7;
            self.emit_sound(
                SoundCue::LevelUp,
                [
                    self.castles.pos_x[wizard],
                    self.castles.pos_y[wizard],
                    self.castles.pos_z[wizard],
                ],
            );
        }
    }

    fn cast_sunburst(&mut self, wizard: usize) {
        let attacker = Faction::of_wizard(wizard);
        for i in 0..MAX_CREATURES {
            if !self.creatures.alive[i] || self.creatures.faction[i] == attacker {
                continue;
            }
            let damage = if self.creatures.kind[i] == CreatureKind::Nest {
                SUNBURST_NEST_DAMAGE
            } else {
                SUNBURST_CREATURE_DAMAGE
            };
            self.creatures.health[i] -= damage;
            let at = [
                self.creatures.pos_x[i],
                self.creatures.pos_y[i],
                self.creatures.pos_z[i],
            ];
            self.spawn_arc([at[0], at[1] + SUNBURST_BEAM_HEIGHT, at[2]], at);
            if self.creatures.health[i] <= 0.0 {
                self.kill_creature(i);
            }
        }
        self.shake_if_player(wizard, 1.0);
        let at = self.position_of(wizard);
        self.emit_sound(SoundCue::Wash, at);
    }
}
