//! Creature behaviour, plus the projectile, orb and particle integrators.
//!
//! Nests and balloons do not fight at all and have their own routines; the
//! rest share one combat loop that picks a target, then either shoots or
//! closes.

use crate::math::*;
use crate::terrain::clamp_to_world;
use crate::types::*;
use crate::world::*;

/// How far a creature looks for something of another faction to attack.
const CREATURE_SIGHT_RANGE: f32 = 220.0;
/// Summons and rival pets hunt further afield than wild creatures.
const SUMMON_SIGHT_RANGE: f32 = 420.0;
/// Wild creatures prefer the player to a rival by this factor.
const PLAYER_TARGET_BIAS: f32 = 0.55;
const MELEE_RANGE: f32 = 22.0;
const MELEE_COOLDOWN: f32 = 1.45;
/// Radius of a melee swing, and how close a wizard must be to be caught by it.
const MELEE_BLAST_RADIUS: f32 = 20.0;
const MELEE_WIZARD_REACH: f32 = 26.0;
const CREATURE_CEILING: f32 = 300.0;
/// Serpentine flight: a dragon weaves this far to either side of its heading
/// and rises and falls on a slower beat, so it reads as swimming through the
/// air rather than walking on it.
const SERPENTINE_SWAY: f32 = 34.0;
const SERPENTINE_SWAY_RATE: f32 = 0.55;
const SERPENTINE_RISE: f32 = 26.0;
const SERPENTINE_RISE_RATE: f32 = 0.31;
/// How hard a flier is pulled back to its preferred cruising altitude.
const CRUISE_PULL: f32 = 0.55;
/// Villagers bolt when anything hostile comes within this.
const VILLAGER_PANIC_RANGE: f32 = 190.0;
/// Garrison creatures belong to their keep. Past this radius they are steered
/// back, so a fleeing villager does not end up two islands away.
const GARRISON_LEASH: f32 = 175.0;
const GARRISON_RECALL: f32 = 2.4;
const CREATURE_GRAVITY: f32 = 90.0;
/// Fraction of the spawn hover height a flier is allowed to sink to.
const FLIER_FLOOR_FRACTION: f32 = 0.6;
const GROUND_REST_HEIGHT: f32 = 3.0;
const TURN_RESPONSE: f32 = 2.0;
const WANDER_RESPONSE: f32 = 1.2;
const WANDER_SPEED_FRACTION: f32 = 0.45;

const NEST_HATCH_INTERVAL_BASE: f32 = 17.0;
const NEST_HATCH_INTERVAL_MIN: f32 = 6.0;
const NEST_HATCH_SPREAD: f32 = 30.0;
const NEST_HOVER: f32 = 3.0;

/// Balloons cruise high, but orbs hover just above the ground. Without
/// stooping the grab sphere can never close and nothing is ever delivered.
const BALLOON_CRUISE_HEIGHT: f32 = 34.0;
const BALLOON_STOOP_HEIGHT: f32 = 5.0;
const BALLOON_SEARCH_RANGE: f32 = 980.0;
const BALLOON_GRAB_RANGE: f32 = 30.0;
const BALLOON_DELIVER_RANGE: f32 = 52.0;
const BALLOON_SPEED: f32 = 66.0;
const BALLOON_TURN_RESPONSE: f32 = 1.5;
/// How far below the envelope a carried orb hangs.
const BALLOON_CARRY_DROP: f32 = 10.0;

impl World {
    pub fn update_creatures(&mut self, dt: f32) {
        for index in 0..MAX_CREATURES {
            if !self.creatures.alive[index] {
                continue;
            }
            let kind = self.creatures.kind[index];
            self.creatures.phase[index] += dt * kind.phase_rate();
            if self.creatures.hurt_flash[index] > 0.0 {
                self.creatures.hurt_flash[index] -= dt;
            }
            self.creatures.attack_cooldown[index] -= dt;

            match kind {
                CreatureKind::Nest => self.update_nest(index, dt),
                CreatureKind::Balloon => self.update_balloon(index, dt),
                _ => self.update_combatant(index, dt),
            }
        }
    }

    // ---- nests -------------------------------------------------------------
    fn update_nest(&mut self, index: usize, dt: f32) {
        self.creatures.timer[index] -= dt;
        if self.creatures.timer[index] <= 0.0 {
            // hatch faster in later realms, but never faster than the floor
            self.creatures.timer[index] = max(
                NEST_HATCH_INTERVAL_MIN,
                NEST_HATCH_INTERVAL_BASE - self.session.realm as f32 * 0.8,
            );
            if self.wild_population() < 8 + self.session.realm * 3 {
                self.hatch_from_nest(index);
            }
        }
        self.creatures.pos_y[index] =
            self.height_at(self.creatures.pos_x[index], self.creatures.pos_z[index]) + NEST_HOVER;
    }

    fn wild_population(&self) -> i32 {
        let mut count = 0;
        for i in 0..MAX_CREATURES {
            if self.creatures.alive[i]
                && self.creatures.faction[i].is_wild()
                && self.creatures.kind[i] != CreatureKind::Nest
            {
                count += 1;
            }
        }
        count
    }

    fn hatch_from_nest(&mut self, nest: usize) {
        let mut roll = self.rng.below(4);
        if roll == 4 {
            roll = 1;
        }
        let kind = CreatureKind::from_index(roll);
        let offset_x = self.rng.range(-NEST_HATCH_SPREAD, NEST_HATCH_SPREAD);
        let offset_z = self.rng.range(-NEST_HATCH_SPREAD, NEST_HATCH_SPREAD);
        let at = [
            self.creatures.pos_x[nest],
            self.creatures.pos_y[nest],
            self.creatures.pos_z[nest],
        ];
        if self
            .spawn_creature(kind, at[0] + offset_x, at[2] + offset_z, Faction::WILD)
            .is_some()
        {
            self.spawn_burst(
                [at[0], at[1] + 6.0, at[2]],
                8,
                8.0,
                2.5,
                [0.6, 0.3, 0.8],
                0.6,
            );
        }
    }

    // ---- balloons ----------------------------------------------------------
    fn update_balloon(&mut self, index: usize, dt: f32) {
        let Some(owner) = self.creatures.faction[index].wizard() else {
            return;
        };
        let mut destination = [
            self.castles.pos_x[owner],
            self.castles.pos_y[owner] + 46.0,
            self.castles.pos_z[owner],
        ];
        if self.castles.health[owner] <= 0.0 {
            // A ruined keep cannot receive mana. Drop any cargo and loiter over
            // the ruins without searching for another orb.
            self.release_balloon_cargo(index);
            self.steer_balloon(index, destination, BALLOON_CRUISE_HEIGHT, dt);
            return;
        }
        let mut floor_height = BALLOON_CRUISE_HEIGHT;
        let carrying = self.orb_carried_by(index);

        match carrying {
            Some(orb) => {
                self.carry_orb(index, orb);
                if self.try_deliver(index, orb, owner) {
                    // orb consumed; head home empty next frame
                }
            }
            None => {
                if let Some(target) = self.nearest_claimed_orb(index) {
                    destination = [
                        self.orbs.pos_x[target],
                        self.orbs.pos_y[target] + 3.0,
                        self.orbs.pos_z[target],
                    ];
                    floor_height = BALLOON_STOOP_HEIGHT;
                    self.try_grab(index, target);
                }
            }
        }

        self.steer_balloon(index, destination, floor_height, dt);
    }

    fn orb_carried_by(&self, balloon: usize) -> Option<usize> {
        for i in 0..MAX_ORBS {
            if self.orbs.alive[i] && self.orbs.carried_by[i] == Some(balloon) {
                return Some(i);
            }
        }
        None
    }

    /// Nearest orb of this balloon's own faction. Unclaimed gold is ignored —
    /// somebody has to cast Claim on it first.
    fn nearest_claimed_orb(&self, balloon: usize) -> Option<usize> {
        let faction = self.creatures.faction[balloon];
        let mut best = None;
        let mut best_dist_sq = BALLOON_SEARCH_RANGE * BALLOON_SEARCH_RANGE;
        for i in 0..MAX_ORBS {
            if !self.orbs.alive[i]
                || self.orbs.carried_by[i].is_some()
                || self.orbs.claimed_by[i] != faction
            {
                continue;
            }
            let dist_sq = length_sq2(
                self.orbs.pos_x[i] - self.creatures.pos_x[balloon],
                self.orbs.pos_z[i] - self.creatures.pos_z[balloon],
            );
            if dist_sq < best_dist_sq {
                best_dist_sq = dist_sq;
                best = Some(i);
            }
        }
        best
    }

    fn try_grab(&mut self, balloon: usize, orb: usize) {
        let reach = length_sq3(
            self.orbs.pos_x[orb] - self.creatures.pos_x[balloon],
            self.orbs.pos_y[orb] - self.creatures.pos_y[balloon],
            self.orbs.pos_z[orb] - self.creatures.pos_z[balloon],
        );
        if reach < BALLOON_GRAB_RANGE * BALLOON_GRAB_RANGE {
            self.orbs.carried_by[orb] = Some(balloon);
        }
    }

    fn carry_orb(&mut self, balloon: usize, orb: usize) {
        self.orbs.pos_x[orb] = self.creatures.pos_x[balloon];
        self.orbs.pos_y[orb] = self.creatures.pos_y[balloon] - BALLOON_CARRY_DROP;
        self.orbs.pos_z[orb] = self.creatures.pos_z[balloon];
    }

    fn try_deliver(&mut self, balloon: usize, orb: usize, owner: usize) -> bool {
        if self.castles.health[owner] <= 0.0 {
            self.release_balloon_cargo(balloon);
            return false;
        }
        let to_keep = [
            self.castles.pos_x[owner] - self.creatures.pos_x[balloon],
            self.castles.pos_y[owner] + 30.0 - self.creatures.pos_y[balloon],
            self.castles.pos_z[owner] - self.creatures.pos_z[balloon],
        ];
        if length_sq3(to_keep[0], to_keep[1], to_keep[2])
            >= BALLOON_DELIVER_RANGE * BALLOON_DELIVER_RANGE
        {
            return false;
        }
        // Rivals ferry at a handicap that closes as the realms get harder.
        let efficiency = if owner == PLAYER {
            1.0
        } else {
            0.34 + self.session.realm as f32 * 0.08
        };
        let capacity = self.castle_capacity(owner);
        self.castles.stored_mana[owner] = min(
            self.castles.stored_mana[owner] + self.orbs.amount[orb] * efficiency,
            capacity,
        );
        self.orbs.alive[orb] = false;
        let at = [
            self.castles.pos_x[owner],
            self.castles.pos_y[owner] + 22.0,
            self.castles.pos_z[owner],
        ];
        for _ in 0..10 {
            let drift = [
                self.rng.range(-10.0, 10.0),
                self.rng.range(4.0, 20.0),
                self.rng.range(-10.0, 10.0),
            ];
            self.spawn_particle(at, drift, 0.7, 2.4, [0.5, 0.8, 1.0], 0.0, 1.5);
        }
        true
    }

    fn steer_balloon(&mut self, index: usize, destination: [f32; 3], floor_height: f32, dt: f32) {
        let offset = [
            destination[0] - self.creatures.pos_x[index],
            destination[1] - self.creatures.pos_y[index],
            destination[2] - self.creatures.pos_z[index],
        ];
        let distance = length3(offset[0], offset[1], offset[2]) + 0.01;
        self.creatures.vel_x[index] = approach(
            self.creatures.vel_x[index],
            offset[0] / distance * BALLOON_SPEED,
            BALLOON_TURN_RESPONSE,
            dt,
        );
        self.creatures.vel_y[index] = approach(
            self.creatures.vel_y[index],
            offset[1] / distance * BALLOON_SPEED * 0.6,
            BALLOON_TURN_RESPONSE,
            dt,
        );
        self.creatures.vel_z[index] = approach(
            self.creatures.vel_z[index],
            offset[2] / distance * BALLOON_SPEED,
            BALLOON_TURN_RESPONSE,
            dt,
        );
        self.creatures.pos_x[index] =
            clamp_to_world(self.creatures.pos_x[index] + self.creatures.vel_x[index] * dt);
        self.creatures.pos_y[index] += self.creatures.vel_y[index] * dt;
        self.creatures.pos_z[index] =
            clamp_to_world(self.creatures.pos_z[index] + self.creatures.vel_z[index] * dt);
        let floor =
            self.surface_at(self.creatures.pos_x[index], self.creatures.pos_z[index]) + floor_height;
        if self.creatures.pos_y[index] < floor {
            self.creatures.pos_y[index] = floor;
        }
        self.creatures.facing[index] = atan2(offset[0], offset[2]);
    }

    // ---- everything that fights --------------------------------------------
    fn update_combatant(&mut self, index: usize, dt: f32) {
        let kind = self.creatures.kind[index];
        let faction = self.creatures.faction[index];

        // summons live on a timer and expire wherever they stand
        if kind == CreatureKind::Wraith && !faction.is_wild() {
            self.creatures.timer[index] -= dt;
            if self.creatures.timer[index] <= 0.0 {
                self.kill_creature(index);
                return;
            }
        }

        // Villagers never engage; they run from whatever is nearest and
        // otherwise mill about.
        if kind.is_civilian() {
            match self.nearest_enemy_creature(index, VILLAGER_PANIC_RANGE) {
                Some(threat) => self.flee_from(index, threat, dt),
                None => self.wander(index, dt),
            }
            self.leash_to_keep(index, dt);
            self.integrate_creature(index, dt);
            if self.creatures.health[index] <= 0.0 {
                self.kill_creature(index);
            }
            return;
        }

        let target = if faction.is_wild() {
            self.pick_target_for_wild(index)
        } else {
            self.pick_target_for_ally(index)
        };

        match target {
            Some((at, dist_sq)) => self.engage(index, at, dist_sq, dt),
            None => self.wander(index, dt),
        }
        self.leash_to_keep(index, dt);
        self.integrate_creature(index, dt);

        if self.creatures.health[index] <= 0.0 {
            self.kill_creature(index);
        }
    }

    /// Wild creatures go for the nearest wizard inside their aggro range,
    /// preferring the player, and otherwise pick a fight with any summon.
    fn pick_target_for_wild(&self, index: usize) -> Option<([f32; 3], f32)> {
        let position = self.creature_position(index);
        let mut best_wizard = None;
        let mut best_score = f32::MAX;
        for wizard in 0..=self.session.rival_count {
            if !self.wizards.alive[wizard] {
                continue;
            }
            let dist_sq = length_sq3(
                self.wizards.pos_x[wizard] - position[0],
                self.wizards.pos_y[wizard] - position[1],
                self.wizards.pos_z[wizard] - position[2],
            );
            let bias = if wizard == PLAYER {
                PLAYER_TARGET_BIAS
            } else {
                1.0
            };
            if dist_sq * bias < best_score {
                best_score = dist_sq * bias;
                best_wizard = Some(wizard);
            }
        }
        let aggro = self.creatures.kind[index].aggro_range();
        if let Some(wizard) = best_wizard {
            let at = [
                self.wizards.pos_x[wizard],
                self.wizards.pos_y[wizard],
                self.wizards.pos_z[wizard],
            ];
            let dist_sq = length_sq3(at[0] - position[0], at[1] - position[1], at[2] - position[2]);
            if dist_sq < aggro * aggro {
                return Some((at, dist_sq));
            }
        }
        self.nearest_enemy_creature(index, CREATURE_SIGHT_RANGE)
            .map(|other| self.creature_target(index, other))
    }

    /// Summons hunt hostile creatures first, then go for the opposing wizard.
    fn pick_target_for_ally(&self, index: usize) -> Option<([f32; 3], f32)> {
        if let Some(other) = self.nearest_enemy_creature(index, SUMMON_SIGHT_RANGE) {
            return Some(self.creature_target(index, other));
        }
        let position = self.creature_position(index);
        let opponent = if self.creatures.faction[index].is_player() {
            1
        } else {
            PLAYER
        };
        if opponent <= self.session.rival_count && self.wizards.alive[opponent] {
            let at = [
                self.wizards.pos_x[opponent],
                self.wizards.pos_y[opponent],
                self.wizards.pos_z[opponent],
            ];
            let dist_sq = length_sq3(at[0] - position[0], at[1] - position[1], at[2] - position[2]);
            return Some((at, dist_sq));
        }
        None
    }

    fn creature_position(&self, index: usize) -> [f32; 3] {
        [
            self.creatures.pos_x[index],
            self.creatures.pos_y[index],
            self.creatures.pos_z[index],
        ]
    }

    fn creature_target(&self, from: usize, other: usize) -> ([f32; 3], f32) {
        let at = self.creature_position(other);
        let here = self.creature_position(from);
        (
            at,
            length_sq3(at[0] - here[0], at[1] - here[1], at[2] - here[2]),
        )
    }

    /// Nearest living creature of a different faction. Balloons are cargo, not
    /// combatants, and are never targeted.
    fn nearest_enemy_creature(&self, index: usize, range: f32) -> Option<usize> {
        let mine = self.creatures.faction[index];
        let here = self.creature_position(index);
        let mut best = None;
        let mut best_dist_sq = range * range;
        for other in 0..MAX_CREATURES {
            if !self.creatures.alive[other] || other == index {
                continue;
            }
            if self.creatures.faction[other] == mine
                || self.creatures.kind[other] == CreatureKind::Balloon
            {
                continue;
            }
            let dist_sq = length_sq3(
                self.creatures.pos_x[other] - here[0],
                self.creatures.pos_y[other] - here[1],
                self.creatures.pos_z[other] - here[2],
            );
            if dist_sq < best_dist_sq {
                best_dist_sq = dist_sq;
                best = Some(other);
            }
        }
        best
    }

    fn engage(&mut self, index: usize, target: [f32; 3], dist_sq: f32, dt: f32) {
        let kind = self.creatures.kind[index];
        let here = self.creature_position(index);
        let offset = [
            target[0] - here[0],
            target[1] - here[1],
            target[2] - here[2],
        ];
        let distance = sqrt(dist_sq) + 0.01;
        self.creatures.facing[index] = atan2(offset[0], offset[2]);

        let holds_range = matches!(kind.firing_range(), Some(range) if distance < range);
        if holds_range {
            self.shoot_at(index, offset, distance);
        } else {
            self.close_on(index, offset, distance, dt);
        }
    }

    fn shoot_at(&mut self, index: usize, offset: [f32; 3], distance: f32) {
        const BOLT_SPEED: f32 = 140.0;
        const DRAGON_BOLT_SPEED: f32 = 190.0;
        /// Bolts are lobbed slightly upward so they arc onto a moving target.
        const BOLT_LOFT: f32 = 0.16;
        const BOLT_SPREAD: f32 = 0.08;
        const BOLT_VERTICAL_SPREAD: f32 = 0.05;
        let kind = self.creatures.kind[index];
        let is_dragon = kind == CreatureKind::Dragon;
        // hold station and fire
        self.creatures.vel_x[index] *= 0.9;
        self.creatures.vel_z[index] *= 0.9;
        if self.creatures.attack_cooldown[index] > 0.0 {
            return;
        }
        self.creatures.attack_cooldown[index] = if is_dragon {
            self.rng.range(1.1, 2.0)
        } else {
            self.rng.range(1.6, 3.0)
        };
        let speed = if is_dragon {
            DRAGON_BOLT_SPEED
        } else {
            BOLT_SPEED
        };
        let inv = 1.0 / distance;
        let volley = if is_dragon { 3 } else { 1 };
        let from = [
            self.creatures.pos_x[index],
            self.creatures.pos_y[index] + 4.0,
            self.creatures.pos_z[index],
        ];
        for _ in 0..volley {
            let jitter_x = self.rng.range(-BOLT_SPREAD, BOLT_SPREAD);
            let jitter_y = self.rng.range(-BOLT_VERTICAL_SPREAD, BOLT_VERTICAL_SPREAD);
            let jitter_z = self.rng.range(-BOLT_SPREAD, BOLT_SPREAD);
            self.spawn_projectile(
                if is_dragon {
                    ProjectileKind::DragonFire
                } else {
                    ProjectileKind::CreatureBolt
                },
                from,
                [
                    (offset[0] * inv + jitter_x) * speed,
                    (offset[1] * inv + BOLT_LOFT + jitter_y) * speed,
                    (offset[2] * inv + jitter_z) * speed,
                ],
                self.creatures.faction[index],
                if is_dragon { 15.0 } else { 7.0 },
                16.0,
                3.4,
            );
        }
    }

    fn close_on(&mut self, index: usize, offset: [f32; 3], distance: f32, dt: f32) {
        let kind = self.creatures.kind[index];
        let speed = kind.move_speed();
        self.creatures.vel_x[index] = approach(
            self.creatures.vel_x[index],
            offset[0] / distance * speed,
            TURN_RESPONSE,
            dt,
        );
        self.creatures.vel_z[index] = approach(
            self.creatures.vel_z[index],
            offset[2] / distance * speed,
            TURN_RESPONSE,
            dt,
        );
        if kind.flies() {
            self.creatures.vel_y[index] = approach(
                self.creatures.vel_y[index],
                offset[1] / distance * speed * 0.75,
                TURN_RESPONSE,
                dt,
            );
        }
        if distance < MELEE_RANGE && self.creatures.attack_cooldown[index] <= 0.0 {
            self.creatures.attack_cooldown[index] = MELEE_COOLDOWN;
            self.strike(index, offset, distance);
        }
    }

    fn strike(&mut self, index: usize, offset: [f32; 3], distance: f32) {
        let kind = self.creatures.kind[index];
        let faction = self.creatures.faction[index];
        let damage = kind.melee_damage();
        let here = self.creature_position(index);
        for wizard in 0..=self.session.rival_count {
            if !self.wizards.alive[wizard] || Faction::of_wizard(wizard) == faction {
                continue;
            }
            let reach = length_sq3(
                self.wizards.pos_x[wizard] - here[0],
                self.wizards.pos_y[wizard] - here[1],
                self.wizards.pos_z[wizard] - here[2],
            );
            if reach < MELEE_WIZARD_REACH * MELEE_WIZARD_REACH {
                self.wound_wizard(wizard, damage, DamageSource::Direct);
            }
        }
        self.apply_blast(here, MELEE_BLAST_RADIUS, damage, faction);
        self.spawn_burst(
            [
                here[0] + offset[0] / distance * 10.0,
                here[1] + offset[1] / distance * 10.0,
                here[2] + offset[2] / distance * 10.0,
            ],
            6,
            10.0,
            2.0,
            [1.0, 0.6, 0.3],
            0.35,
        );
    }

    /// Runs directly away from a threat, which for a villager is the whole of
    /// their combat repertoire.
    fn flee_from(&mut self, index: usize, threat: usize, dt: f32) {
        let away = [
            self.creatures.pos_x[index] - self.creatures.pos_x[threat],
            self.creatures.pos_z[index] - self.creatures.pos_z[threat],
        ];
        let distance = sqrt(length_sq2(away[0], away[1])) + 0.01;
        let speed = self.creatures.kind[index].move_speed();
        self.creatures.facing[index] = atan2(away[0], away[1]);
        self.creatures.vel_x[index] = approach(
            self.creatures.vel_x[index],
            away[0] / distance * speed,
            TURN_RESPONSE,
            dt,
        );
        self.creatures.vel_z[index] = approach(
            self.creatures.vel_z[index],
            away[1] / distance * speed,
            TURN_RESPONSE,
            dt,
        );
    }

    /// Pulls a keep's own villagers and soldiers back toward it once they
    /// stray past the leash. Anything else is left alone.
    fn leash_to_keep(&mut self, index: usize, dt: f32) {
        if !self.creatures.kind[index].is_garrison() {
            return;
        }
        let Some(wizard) = self.creatures.faction[index].wizard() else {
            return;
        };
        let home_x = self.castles.pos_x[wizard];
        let home_z = self.castles.pos_z[wizard];
        let offset_x = home_x - self.creatures.pos_x[index];
        let offset_z = home_z - self.creatures.pos_z[index];
        let distance_sq = length_sq2(offset_x, offset_z);
        if distance_sq < GARRISON_LEASH * GARRISON_LEASH {
            return;
        }
        let distance = sqrt(distance_sq) + 0.01;
        let speed = self.creatures.kind[index].move_speed();
        self.creatures.vel_x[index] = approach(
            self.creatures.vel_x[index],
            offset_x / distance * speed,
            GARRISON_RECALL,
            dt,
        );
        self.creatures.vel_z[index] = approach(
            self.creatures.vel_z[index],
            offset_z / distance * speed,
            GARRISON_RECALL,
            dt,
        );
    }

    fn wander(&mut self, index: usize, dt: f32) {
        self.creatures.timer[index] -= dt;
        if self.creatures.timer[index] <= 0.0 {
            self.creatures.timer[index] = self.rng.range(2.0, 5.0);
            self.creatures.facing[index] = self.rng.range(0.0, core::f32::consts::TAU);
        }
        let kind = self.creatures.kind[index];
        let drift = kind.move_speed() * WANDER_SPEED_FRACTION;
        let heading = self.creatures.facing[index];
        self.creatures.vel_x[index] = approach(
            self.creatures.vel_x[index],
            sin(heading) * drift,
            WANDER_RESPONSE,
            dt,
        );
        self.creatures.vel_z[index] = approach(
            self.creatures.vel_z[index],
            cos(heading) * drift,
            WANDER_RESPONSE,
            dt,
        );
        if kind.flies() {
            // a slow bob so idle fliers do not hang perfectly still
            let bob = sin(self.creatures.phase[index] * 0.3) * 8.0;
            self.creatures.vel_y[index] =
                approach(self.creatures.vel_y[index], bob, WANDER_RESPONSE, dt);
        }
    }

    fn integrate_creature(&mut self, index: usize, dt: f32) {
        // A dragon slides sideways across its own heading as it goes, which is
        // what turns a straight approach into a serpentine one.
        if self.creatures.kind[index] == CreatureKind::Dragon {
            let heading = self.creatures.facing[index];
            let sway = cos(self.creatures.phase[index] * SERPENTINE_SWAY_RATE) * SERPENTINE_SWAY;
            self.creatures.vel_x[index] += cos(heading) * sway * dt;
            self.creatures.vel_z[index] -= sin(heading) * sway * dt;
        }
        self.creatures.pos_x[index] =
            clamp_to_world(self.creatures.pos_x[index] + self.creatures.vel_x[index] * dt);
        self.creatures.pos_z[index] =
            clamp_to_world(self.creatures.pos_z[index] + self.creatures.vel_z[index] * dt);
        let ground = max(
            self.height_at(self.creatures.pos_x[index], self.creatures.pos_z[index]),
            SEA_LEVEL - 2.0,
        );
        let kind = self.creatures.kind[index];
        if kind.flies() {
            // Fliers are pulled back toward their cruising altitude, and the
            // big serpents weave around it, so they hold the sky instead of
            // sinking onto the hills.
            let cruise = kind.cruise_height();
            if cruise > 0.0 {
                let phase = self.creatures.phase[index];
                let weave = if kind == CreatureKind::Dragon {
                    sin(phase * SERPENTINE_RISE_RATE) * SERPENTINE_RISE
                } else {
                    0.0
                };
                let wanted = ground + cruise + weave;
                self.creatures.vel_y[index] = approach(
                    self.creatures.vel_y[index],
                    (wanted - self.creatures.pos_y[index]) * CRUISE_PULL,
                    CRUISE_PULL,
                    dt,
                );
            }
            self.creatures.pos_y[index] += self.creatures.vel_y[index] * dt;
            let floor = ground + kind.hover_height() * FLIER_FLOOR_FRACTION;
            if self.creatures.pos_y[index] < floor {
                self.creatures.pos_y[index] = floor;
                self.creatures.vel_y[index] = max(self.creatures.vel_y[index], 0.0);
            }
            if self.creatures.pos_y[index] > CREATURE_CEILING {
                self.creatures.pos_y[index] = CREATURE_CEILING;
                self.creatures.vel_y[index] = min(self.creatures.vel_y[index], 0.0);
            }
        } else {
            self.creatures.vel_y[index] -= CREATURE_GRAVITY * dt;
            self.creatures.pos_y[index] += self.creatures.vel_y[index] * dt;
            let rest = ground + GROUND_REST_HEIGHT;
            if self.creatures.pos_y[index] < rest {
                self.creatures.pos_y[index] = rest;
                self.creatures.vel_y[index] = 0.0;
            }
        }
    }

    // ---- projectiles -------------------------------------------------------
    pub fn update_projectiles(&mut self, dt: f32) {
        for index in 0..MAX_PROJECTILES {
            if !self.projectiles.alive[index] {
                continue;
            }
            let kind = self.projectiles.kind[index];
            self.projectiles.vel_y[index] -= kind.gravity() * dt;
            self.projectiles.pos_x[index] += self.projectiles.vel_x[index] * dt;
            self.projectiles.pos_y[index] += self.projectiles.vel_y[index] * dt;
            self.projectiles.pos_z[index] += self.projectiles.vel_z[index] * dt;
            self.projectiles.life[index] -= dt;
            self.spawn_projectile_trail(index);
            if self.projectile_has_landed(index) {
                self.detonate_projectile(index);
            }
        }
    }

    fn spawn_projectile_trail(&mut self, index: usize) {
        let kind = self.projectiles.kind[index];
        let colour = kind.trail_colour();
        let is_meteor = kind == ProjectileKind::Meteor;
        // fire leaves a thick wake; a plain bolt only needs a wisp
        let puffs = match kind {
            ProjectileKind::Meteor => 5,
            ProjectileKind::Firebolt | ProjectileKind::DragonFire => 3,
            ProjectileKind::CreatureBolt => 1,
        };
        let at = [
            self.projectiles.pos_x[index],
            self.projectiles.pos_y[index],
            self.projectiles.pos_z[index],
        ];
        for _ in 0..puffs {
            let jitter = [
                self.rng.range(-2.0, 2.0),
                self.rng.range(-2.0, 2.0),
                self.rng.range(-2.0, 2.0),
            ];
            let drift = [
                self.rng.range(-6.0, 6.0),
                self.rng.range(-2.0, 10.0),
                self.rng.range(-6.0, 6.0),
            ];
            self.spawn_particle(
                [at[0] + jitter[0], at[1] + jitter[1], at[2] + jitter[2]],
                drift,
                if is_meteor { 0.7 } else { 0.34 },
                if is_meteor { 6.5 } else { 3.4 },
                colour,
                0.0,
                1.5,
            );
        }
    }

    fn projectile_has_landed(&self, index: usize) -> bool {
        const CREATURE_HIT_RADIUS: f32 = 15.0;
        const WIZARD_HIT_RADIUS: f32 = 13.0;
        let at = [
            self.projectiles.pos_x[index],
            self.projectiles.pos_y[index],
            self.projectiles.pos_z[index],
        ];
        if at[1] <= self.height_at(at[0], at[2]) || at[1] <= SEA_LEVEL - 1.0 {
            return true;
        }
        let owner = self.projectiles.owner[index];
        for other in 0..MAX_CREATURES {
            if !self.creatures.alive[other]
                || self.creatures.faction[other] == owner
                || self.creatures.kind[other] == CreatureKind::Balloon
            {
                continue;
            }
            let dist_sq = length_sq3(
                self.creatures.pos_x[other] - at[0],
                self.creatures.pos_y[other] - at[1],
                self.creatures.pos_z[other] - at[2],
            );
            if dist_sq < CREATURE_HIT_RADIUS * CREATURE_HIT_RADIUS {
                return true;
            }
        }
        for wizard in 0..=self.session.rival_count {
            if !self.wizards.alive[wizard] || Faction::of_wizard(wizard) == owner {
                continue;
            }
            let dist_sq = length_sq3(
                self.wizards.pos_x[wizard] - at[0],
                self.wizards.pos_y[wizard] - at[1],
                self.wizards.pos_z[wizard] - at[2],
            );
            if dist_sq < WIZARD_HIT_RADIUS * WIZARD_HIT_RADIUS {
                return true;
            }
        }
        self.projectiles.life[index] <= 0.0
    }

    /// A direct wizard hit lands its full damage before the splash, which is
    /// why this is checked again rather than reusing the landing test.
    fn detonate_projectile(&mut self, index: usize) {
        const WIZARD_HIT_RADIUS: f32 = 13.0;
        let at = [
            self.projectiles.pos_x[index],
            self.projectiles.pos_y[index],
            self.projectiles.pos_z[index],
        ];
        let owner = self.projectiles.owner[index];
        let damage = self.projectiles.damage[index];
        let kind = self.projectiles.kind[index];
        let radius = self.projectiles.blast_radius[index];
        for wizard in 0..=self.session.rival_count {
            if !self.wizards.alive[wizard] || Faction::of_wizard(wizard) == owner {
                continue;
            }
            let dist_sq = length_sq3(
                self.wizards.pos_x[wizard] - at[0],
                self.wizards.pos_y[wizard] - at[1],
                self.wizards.pos_z[wizard] - at[2],
            );
            if dist_sq < WIZARD_HIT_RADIUS * WIZARD_HIT_RADIUS {
                self.wound_wizard(wizard, damage, DamageSource::Direct);
                break;
            }
        }
        self.projectiles.alive[index] = false;
        self.apply_blast(at, radius, damage, owner);
        match kind {
            ProjectileKind::Meteor => {
                self.deform(at[0], at[2], 74.0, 30.0, DeformKind::Crater);
                self.spawn_burst(at, 80, 40.0, 9.0, [1.0, 0.6, 0.2], 1.6);
                self.ignite_scenery(at, 110.0);
                self.session.screen_shake = 1.6;
                self.emit_sound(SoundCue::MeteorImpact, at);
            }
            ProjectileKind::Firebolt => {
                self.deform(at[0], at[2], 16.0, 2.2, DeformKind::Crater);
                self.spawn_burst(at, 22, 20.0, 4.4, [1.0, 0.6, 0.22], 0.7);
                // a firebolt is fire: whatever it lands in catches
                self.ignite_scenery(at, 34.0);
                self.emit_sound(SoundCue::Pop, at);
            }
            other => {
                self.spawn_burst(at, 14, 15.0, 3.4, other.trail_colour(), 0.6);
                self.emit_sound(SoundCue::Pop, at);
            }
        }
    }

    // ---- particles and orbs ------------------------------------------------
    pub fn update_particles(&mut self, dt: f32) {
        for i in 0..MAX_PARTICLES {
            if self.particles.life[i] <= 0.0 {
                continue;
            }
            self.particles.life[i] -= dt;
            if self.particles.life[i] <= 0.0 {
                continue;
            }
            self.particles.vel_y[i] += self.particles.gravity[i] * dt;
            let retained = max(1.0 - self.particles.drag[i] * dt, 0.0);
            self.particles.vel_x[i] *= retained;
            self.particles.vel_y[i] *= retained;
            self.particles.vel_z[i] *= retained;
            self.particles.pos_x[i] += self.particles.vel_x[i] * dt;
            self.particles.pos_y[i] += self.particles.vel_y[i] * dt;
            self.particles.pos_z[i] += self.particles.vel_z[i] * dt;
        }
    }

    pub fn update_orbs(&mut self, dt: f32) {
        /// Height an orb settles to above the surface, and how far it bobs.
        const ORB_HOVER: f32 = 5.0;
        const ORB_BOB: f32 = 1.6;
        const ORB_SETTLE_RATE: f32 = 0.8;
        /// Sparkles shed per second. Emitted here rather than while drawing:
        /// the draw pass runs inside `step`, so a random draw taken there
        /// shifts this generator's whole sequence, and a draw taken only for
        /// orbs near the camera makes the simulation depend on where the
        /// player is looking.
        const ORB_SPARKS_PER_SECOND: f32 = 6.0;
        for i in 0..MAX_ORBS {
            if !self.orbs.alive[i] || self.orbs.carried_by[i].is_some() {
                continue;
            }
            self.orbs.bob_phase[i] += dt * 2.4;
            if self.rng.chance(ORB_SPARKS_PER_SECOND * dt) {
                let at = [self.orbs.pos_x[i], self.orbs.pos_y[i], self.orbs.pos_z[i]];
                let drift = [
                    self.rng.range(-3.0, 3.0),
                    self.rng.range(3.0, 9.0),
                    self.rng.range(-3.0, 3.0),
                ];
                self.spawn_particle(at, drift, 0.8, 1.8, [0.45, 0.8, 1.0], 0.0, 1.2);
            }
            let resting = self.surface_at(self.orbs.pos_x[i], self.orbs.pos_z[i])
                + ORB_HOVER
                + sin(self.orbs.bob_phase[i]) * ORB_BOB;
            self.orbs.pos_y[i] = approach(self.orbs.pos_y[i], resting, ORB_SETTLE_RATE, dt);
            if self.orbs.pos_y[i] < resting - 2.0 {
                self.orbs.pos_y[i] = resting - 2.0;
            }
        }
    }
}
