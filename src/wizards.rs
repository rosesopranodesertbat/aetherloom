//! Flight and upkeep for the player, and the rival AI.

use crate::math::*;
use crate::terrain::clamp_to_world;
use crate::types::*;
use crate::world::*;

/// Distance at which the player's keep tops up health.
const KEEP_HEAL_RANGE: f32 = 62.0;
const KEEP_HEAL_RATE: f32 = 7.0;
/// Seconds out of combat before health starts creeping back.
const OUT_OF_COMBAT_DELAY: f32 = 6.0;
const OUT_OF_COMBAT_REGEN: f32 = 1.5;
const MANA_REGEN_BASE: f32 = 5.0;
const MANA_REGEN_PER_TIER: f32 = 2.2;
/// Seconds between balloon launches while below the fleet cap.
const BALLOON_LAUNCH_INTERVAL: f32 = 7.0;
const BALLOON_SPAWN_SPREAD: f32 = 20.0;
/// Store fraction at which a rival decides to buy the next tier.
const RIVAL_UPGRADE_THRESHOLD: f32 = 0.82;
const RIVAL_UPGRADE_PRICE: f32 = 44.0;

const RIVAL_BASE_SPEED: f32 = 74.0;
const RIVAL_SPEED_PER_REALM: f32 = 4.5;
const RIVAL_TURN_RESPONSE: f32 = 2.2;
const RIVAL_HASTE_MULTIPLIER: f32 = 1.6;
/// Health below which a rival breaks off and heals.
const RIVAL_RETREAT_HEALTH: f32 = 34.0;
const RIVAL_RECOVERED_HEALTH: f32 = 82.0;
/// How far a rival will chase the player before losing interest.
const RIVAL_DUEL_RANGE: f32 = 640.0;
const RIVAL_FIRING_RANGE: f32 = 620.0;
const RIVAL_HUNT_RANGE: f32 = 540.0;
/// Rivals park this far behind the player when duelling.
const RIVAL_STANDOFF: f32 = 130.0;
/// Rivals only bother possessing an orb they are nearly on top of.
const RIVAL_CLAIM_RANGE: f32 = 190.0;
/// AI casts bypass the player's cooldown table, so Claim is throttled by hand;
/// ungated it fires every frame and hoovers the map instantly.
const RIVAL_CLAIM_CHANCE_PER_FRAME: f32 = 0.015;

impl World {
    // ---- the player --------------------------------------------------------
    pub fn update_player(&mut self, dt: f32) {
        self.steer_player(dt);
        self.integrate_player(dt);
        self.expire_player_buffs(dt);
        self.trail_player();
    }

    fn steer_player(&mut self, dt: f32) {
        let input = self.input;
        // Handedness: d(forward)/d(yaw) points along -screen_right, so a rising
        // yaw swings the view left. Mouse right therefore decreases it.
        self.wizards.yaw[PLAYER] -= input.yaw_delta;
        self.wizards.pitch[PLAYER] =
            clamp(self.wizards.pitch[PLAYER] - input.pitch_delta, -CARPET_MAX_PITCH, CARPET_MAX_PITCH);
        self.wizards.roll[PLAYER] = approach(
            self.wizards.roll[PLAYER],
            -input.strafe * CARPET_BANK_ANGLE,
            6.0,
            dt,
        );

        let facing = self.facing_of(PLAYER);
        // Screen-right. The view matrix puts the camera's x axis here; strafing
        // along its negation is what used to send D the wrong way.
        let right = [-cos(self.wizards.yaw[PLAYER]), sin(self.wizards.yaw[PLAYER])];

        let mut top_speed = CARPET_TOP_SPEED;
        if self.wizards.haste_remaining[PLAYER] > 0.0 {
            top_speed *= HASTE_SPEED_MULTIPLIER;
        }
        if input.braking {
            top_speed *= CARPET_BRAKE_FACTOR;
        }
        let strafe_speed = top_speed * CARPET_STRAFE_FRACTION;
        let target = [
            facing[0] * input.thrust * top_speed + right[0] * input.strafe * strafe_speed,
            facing[1] * input.thrust * top_speed + input.lift * CARPET_CLIMB_SPEED,
            facing[2] * input.thrust * top_speed + right[1] * input.strafe * strafe_speed,
        ];
        self.wizards.vel_x[PLAYER] =
            approach(self.wizards.vel_x[PLAYER], target[0], CARPET_TURN_RESPONSE, dt);
        self.wizards.vel_y[PLAYER] =
            approach(self.wizards.vel_y[PLAYER], target[1], CARPET_TURN_RESPONSE, dt);
        self.wizards.vel_z[PLAYER] =
            approach(self.wizards.vel_z[PLAYER], target[2], CARPET_TURN_RESPONSE, dt);
    }

    fn integrate_player(&mut self, dt: f32) {
        self.wizards.pos_x[PLAYER] =
            clamp_to_world(self.wizards.pos_x[PLAYER] + self.wizards.vel_x[PLAYER] * dt);
        self.wizards.pos_y[PLAYER] += self.wizards.vel_y[PLAYER] * dt;
        self.wizards.pos_z[PLAYER] =
            clamp_to_world(self.wizards.pos_z[PLAYER] + self.wizards.vel_z[PLAYER] * dt);

        let floor = self.surface_at(self.wizards.pos_x[PLAYER], self.wizards.pos_z[PLAYER])
            + CARPET_GROUND_CLEARANCE;
        if self.wizards.pos_y[PLAYER] < floor {
            // eased out of the ground rather than snapped, so scraping a hill
            // does not read as a collision
            let penetration = floor - self.wizards.pos_y[PLAYER];
            self.wizards.pos_y[PLAYER] += penetration * min(1.0, dt * 14.0);
            if self.wizards.vel_y[PLAYER] < -SAFE_DESCENT_SPEED {
                let excess = -self.wizards.vel_y[PLAYER] - SAFE_DESCENT_SPEED;
                self.wizards.health[PLAYER] -= excess * dt * 1.2;
                self.session.screen_shake = 0.4;
            }
            if self.wizards.vel_y[PLAYER] < 0.0 {
                self.wizards.vel_y[PLAYER] *= 0.25;
            }
        }
        if self.wizards.pos_y[PLAYER] > CARPET_CEILING {
            self.wizards.pos_y[PLAYER] = CARPET_CEILING;
            self.wizards.vel_y[PLAYER] = min(self.wizards.vel_y[PLAYER], 0.0);
        }
    }

    fn expire_player_buffs(&mut self, dt: f32) {
        if self.wizards.ward_remaining[PLAYER] > 0.0 {
            self.wizards.ward_remaining[PLAYER] -= dt;
        }
        if self.wizards.haste_remaining[PLAYER] > 0.0 {
            self.wizards.haste_remaining[PLAYER] -= dt;
        }
    }

    fn trail_player(&mut self) {
        if !self.rng.chance(0.55) {
            return;
        }
        let jitter_x = self.rng.range(-4.0, 4.0);
        let jitter_z = self.rng.range(-4.0, 4.0);
        let drift = self.rng.range(-2.0, 2.0);
        self.spawn_particle(
            [
                self.wizards.pos_x[PLAYER] + jitter_x,
                self.wizards.pos_y[PLAYER] - 3.0,
                self.wizards.pos_z[PLAYER] + jitter_z,
            ],
            [0.0, drift, 0.0],
            0.55,
            1.6,
            [0.45, 0.35, 0.85],
            0.0,
            2.0,
        );
    }

    // ---- shared upkeep -----------------------------------------------------
    pub fn update_wizard_upkeep(&mut self, wizard: usize, dt: f32) {
        self.wizards.time_since_hit[wizard] += dt;
        if self.wizards.time_since_hit[wizard] > OUT_OF_COMBAT_DELAY
            && self.wizards.health[wizard] < WIZARD_MAX_HEALTH
        {
            self.wizards.health[wizard] =
                min(self.wizards.health[wizard] + OUT_OF_COMBAT_REGEN * dt, WIZARD_MAX_HEALTH);
        }
        self.regenerate_mana(wizard, dt);
        self.heal_at_own_keep(wizard, dt);
        self.rival_buys_tier(wizard);
        self.launch_balloons(wizard, dt);
    }

    fn regenerate_mana(&mut self, wizard: usize, dt: f32) {
        if self.castles.health[wizard] <= 0.0 || self.wizards.mana[wizard] >= MANA_REGEN_CEILING {
            return;
        }
        let rate = MANA_REGEN_BASE + self.castles.tier[wizard] as f32 * MANA_REGEN_PER_TIER;
        self.wizards.mana[wizard] = min(self.wizards.mana[wizard] + rate * dt, MANA_REGEN_CEILING);
    }

    fn heal_at_own_keep(&mut self, wizard: usize, dt: f32) {
        if self.castles.health[wizard] <= 0.0 {
            return;
        }
        let to_keep = [
            self.castles.pos_x[wizard] - self.wizards.pos_x[wizard],
            self.castles.pos_y[wizard] + 16.0 - self.wizards.pos_y[wizard],
            self.castles.pos_z[wizard] - self.wizards.pos_z[wizard],
        ];
        if length_sq3(to_keep[0], to_keep[1], to_keep[2]) < KEEP_HEAL_RANGE * KEEP_HEAL_RANGE {
            self.wizards.health[wizard] =
                min(self.wizards.health[wizard] + KEEP_HEAL_RATE * dt, WIZARD_MAX_HEALTH);
        }
    }

    /// The player buys tiers with the Fortress spell; rivals do it themselves
    /// once their keep is nearly full.
    fn rival_buys_tier(&mut self, wizard: usize) {
        if wizard == PLAYER
            || self.castles.health[wizard] <= 0.0
            || self.castles.tier[wizard] >= MAX_CASTLE_TIER
        {
            return;
        }
        let nearly_full =
            self.castles.stored_mana[wizard] > self.castle_capacity(wizard) * RIVAL_UPGRADE_THRESHOLD;
        if nearly_full && self.wizards.mana[wizard] > RIVAL_UPGRADE_PRICE {
            self.castles.tier[wizard] += 1;
            self.wizards.mana[wizard] -= RIVAL_UPGRADE_PRICE;
            self.castles.max_health[wizard] += 220.0;
            self.castles.health[wizard] = self.castles.max_health[wizard];
        }
    }

    fn launch_balloons(&mut self, wizard: usize, dt: f32) {
        if self.castles.health[wizard] <= 0.0 {
            return;
        }
        self.castles.balloon_timer[wizard] -= dt;
        if self.castles.balloon_timer[wizard] > 0.0 {
            return;
        }
        self.castles.balloon_timer[wizard] = BALLOON_LAUNCH_INTERVAL;
        let faction = Faction::of_wizard(wizard);
        let mut fleet = 0;
        for i in 0..MAX_CREATURES {
            if self.creatures.alive[i]
                && self.creatures.kind[i] == CreatureKind::Balloon
                && self.creatures.faction[i] == faction
            {
                fleet += 1;
            }
        }
        // the player gets one more than their tier; rivals are capped by realm
        let cap = if wizard == PLAYER {
            self.castles.tier[wizard] + 1
        } else {
            min_i32(1 + self.session.realm / 2, self.castles.tier[wizard])
        };
        if fleet >= cap {
            return;
        }
        let offset_x = self.rng.range(-BALLOON_SPAWN_SPREAD, BALLOON_SPAWN_SPREAD);
        let offset_z = self.rng.range(-BALLOON_SPAWN_SPREAD, BALLOON_SPAWN_SPREAD);
        self.spawn_creature(
            CreatureKind::Balloon,
            self.castles.pos_x[wizard] + offset_x,
            self.castles.pos_z[wizard] + offset_z,
            faction,
        );
    }

    // ---- rivals ------------------------------------------------------------
    pub fn update_rival(&mut self, wizard: usize, dt: f32) {
        if !self.wizards.alive[wizard] {
            self.tick_respawn(wizard, dt);
            return;
        }
        if self.wizards.ward_remaining[wizard] > 0.0 {
            self.wizards.ward_remaining[wizard] -= dt;
        }
        if self.wizards.haste_remaining[wizard] > 0.0 {
            self.wizards.haste_remaining[wizard] -= dt;
        }
        self.wizards.plan_remaining[wizard] -= dt;
        self.choose_rival_plan(wizard);
        let destination = self.pursue_plan(wizard);
        self.wizards.cast_cooldown[wizard] -= dt;
        self.steer_rival(wizard, destination, dt);
    }

    fn tick_respawn(&mut self, wizard: usize, dt: f32) {
        self.wizards.respawn_remaining[wizard] -= dt;
        if self.wizards.respawn_remaining[wizard] > 0.0 || self.castles.health[wizard] <= 0.0 {
            return;
        }
        self.wizards.alive[wizard] = true;
        self.wizards.health[wizard] = WIZARD_MAX_HEALTH;
        self.wizards.pos_x[wizard] = self.castles.pos_x[wizard];
        self.wizards.pos_z[wizard] = self.castles.pos_z[wizard] + 30.0;
        self.wizards.pos_y[wizard] = self.castles.pos_y[wizard] + 45.0;
        self.wizards.mana[wizard] = 60.0;
    }

    fn choose_rival_plan(&mut self, wizard: usize) {
        if self.wizards.health[wizard] < RIVAL_RETREAT_HEALTH {
            self.wizards.plan[wizard] = RivalPlan::Retreat;
            return;
        }
        if self.wizards.plan_remaining[wizard] > 0.0 {
            return;
        }
        self.wizards.plan_remaining[wizard] = self.rng.range(1.6, 3.4);
        let to_player = length_sq3(
            self.wizards.pos_x[PLAYER] - self.wizards.pos_x[wizard],
            self.wizards.pos_y[PLAYER] - self.wizards.pos_y[wizard],
            self.wizards.pos_z[PLAYER] - self.wizards.pos_z[wizard],
        );
        // The aggression roll has to stay inside the condition: drawing it
        // unconditionally consumes a random number on frames where the rival
        // was never going to engage, which shifts the entire sequence.
        let plan = if self.wizards.alive[PLAYER]
            && to_player < RIVAL_DUEL_RANGE * RIVAL_DUEL_RANGE
            && self.wizards.mana[wizard] > 26.0
            && self.rng.chance(0.24 + self.session.realm as f32 * 0.06)
        {
            RivalPlan::DuelPlayer
        } else if self.wizards.mana[wizard] > self.mana_cap(wizard) * 0.8 {
            RivalPlan::ReturnHome
        } else {
            RivalPlan::GatherMana
        };
        self.wizards.plan[wizard] = plan;
    }

    /// Returns where the rival wants to be, casting along the way.
    fn pursue_plan(&mut self, wizard: usize) -> [f32; 3] {
        let home = [
            self.castles.pos_x[wizard],
            self.castles.pos_y[wizard] + 34.0,
            self.castles.pos_z[wizard],
        ];
        match self.wizards.plan[wizard] {
            RivalPlan::GatherMana => self.gather_mana(wizard).unwrap_or(home),
            RivalPlan::DuelPlayer if self.wizards.alive[PLAYER] => self.duel_player(wizard),
            RivalPlan::Retreat => {
                if self.wizards.health[wizard] < 70.0
                    && self.wizards.mana[wizard] > 20.0
                    && self.wizards.cast_cooldown[wizard] <= 0.0
                {
                    self.cast(wizard, Spell::Mend);
                    self.wizards.cast_cooldown[wizard] = 3.0;
                }
                if self.wizards.health[wizard] > RIVAL_RECOVERED_HEALTH {
                    self.wizards.plan[wizard] = RivalPlan::GatherMana;
                }
                home
            }
            _ => home,
        }
    }

    fn gather_mana(&mut self, wizard: usize) -> Option<[f32; 3]> {
        if let Some((orb, dist_sq)) = self.nearest_free_orb(wizard) {
            let destination = [
                self.orbs.pos_x[orb],
                self.orbs.pos_y[orb] + 6.0,
                self.orbs.pos_z[orb],
            ];
            if dist_sq < RIVAL_CLAIM_RANGE * RIVAL_CLAIM_RANGE
                && self.rng.chance(RIVAL_CLAIM_CHANCE_PER_FRAME)
            {
                self.cast(wizard, Spell::Claim);
            }
            return Some(destination);
        }
        // nothing loose, so make some by killing wildlife
        let (creature, dist_sq) = self.nearest_wild_creature(wizard)?;
        let destination = [
            self.creatures.pos_x[creature],
            self.creatures.pos_y[creature] + 26.0,
            self.creatures.pos_z[creature],
        ];
        if dist_sq < RIVAL_HUNT_RANGE * RIVAL_HUNT_RANGE
            && self.wizards.cast_cooldown[wizard] <= 0.0
        {
            let at = [
                self.creatures.pos_x[creature],
                self.creatures.pos_y[creature],
                self.creatures.pos_z[creature],
            ];
            self.aim_and_fire(wizard, at, false);
            self.wizards.cast_cooldown[wizard] = self.rng.range(0.5, 1.1);
        }
        Some(destination)
    }

    fn nearest_free_orb(&self, wizard: usize) -> Option<(usize, f32)> {
        let mut best = None;
        let mut best_dist_sq = f32::MAX;
        for i in 0..MAX_ORBS {
            if !self.orbs.alive[i] || self.orbs.carried_by[i].is_some() {
                continue;
            }
            let dist_sq = length_sq2(
                self.orbs.pos_x[i] - self.wizards.pos_x[wizard],
                self.orbs.pos_z[i] - self.wizards.pos_z[wizard],
            );
            if dist_sq < best_dist_sq {
                best_dist_sq = dist_sq;
                best = Some(i);
            }
        }
        best.map(|i| (i, best_dist_sq))
    }

    fn nearest_wild_creature(&self, wizard: usize) -> Option<(usize, f32)> {
        let mut best = None;
        let mut best_dist_sq = f32::MAX;
        for i in 0..MAX_CREATURES {
            if !self.creatures.alive[i] || !self.creatures.faction[i].is_wild() {
                continue;
            }
            let dist_sq = length_sq2(
                self.creatures.pos_x[i] - self.wizards.pos_x[wizard],
                self.creatures.pos_z[i] - self.wizards.pos_z[wizard],
            );
            if dist_sq < best_dist_sq {
                best_dist_sq = dist_sq;
                best = Some(i);
            }
        }
        best.map(|i| (i, best_dist_sq))
    }

    fn duel_player(&mut self, wizard: usize) -> [f32; 3] {
        // sit off the player's tail rather than flying into them
        let destination = [
            self.wizards.pos_x[PLAYER] - sin(self.wizards.yaw[PLAYER]) * RIVAL_STANDOFF,
            self.wizards.pos_y[PLAYER] + 16.0,
            self.wizards.pos_z[PLAYER] - cos(self.wizards.yaw[PLAYER]) * RIVAL_STANDOFF,
        ];
        let player_at = [
            self.wizards.pos_x[PLAYER],
            self.wizards.pos_y[PLAYER],
            self.wizards.pos_z[PLAYER],
        ];
        let dist_sq = length_sq3(
            player_at[0] - self.wizards.pos_x[wizard],
            player_at[1] - self.wizards.pos_y[wizard],
            player_at[2] - self.wizards.pos_z[wizard],
        );
        if dist_sq >= RIVAL_FIRING_RANGE * RIVAL_FIRING_RANGE
            || self.wizards.cast_cooldown[wizard] > 0.0
        {
            return destination;
        }
        let roll = self.rng.unit();
        let realm = self.session.realm;
        if roll < 0.16 && self.wizards.mana[wizard] > 26.0 && realm >= 3 {
            self.aim_at(wizard, player_at);
            self.cast(wizard, Spell::Crater);
            self.wizards.cast_cooldown[wizard] = self.rng.range(2.4, 4.0);
        } else if roll < 0.36 && self.wizards.mana[wizard] > 12.0 && realm >= 2 {
            self.aim_at(wizard, player_at);
            self.cast(wizard, Spell::ChainLightning);
            self.wizards.cast_cooldown[wizard] = self.rng.range(1.4, 2.6);
        } else {
            self.aim_and_fire(wizard, player_at, true);
            // later realms fire faster
            self.wizards.cast_cooldown[wizard] =
                self.rng.range(0.42, 0.9) + (9.0 - realm as f32) * 0.11;
        }
        destination
    }

    fn aim_at(&mut self, wizard: usize, target: [f32; 3]) {
        let offset = [
            target[0] - self.wizards.pos_x[wizard],
            target[1] - self.wizards.pos_y[wizard],
            target[2] - self.wizards.pos_z[wizard],
        ];
        let horizontal = sqrt(offset[0] * offset[0] + offset[2] * offset[2]);
        self.wizards.yaw[wizard] = atan2(offset[0], offset[2]);
        self.wizards.pitch[wizard] = atan2(offset[1], horizontal);
    }

    /// Aims with a spread that tightens as the realms get harder, optionally
    /// leading the player's current velocity.
    fn aim_and_fire(&mut self, wizard: usize, target: [f32; 3], lead: bool) {
        const LEAD_SECONDS: f32 = 0.42;
        let mut aim = target;
        if lead {
            aim[0] += self.wizards.vel_x[PLAYER] * LEAD_SECONDS;
            aim[1] += self.wizards.vel_y[PLAYER] * LEAD_SECONDS;
            aim[2] += self.wizards.vel_z[PLAYER] * LEAD_SECONDS;
        }
        self.aim_at(wizard, aim);
        let spread = max(0.13 - self.session.realm as f32 * 0.013, 0.015);
        let yaw_error = self.rng.range(-spread, spread);
        let pitch_error = self.rng.range(-spread, spread);
        self.wizards.yaw[wizard] += yaw_error;
        self.wizards.pitch[wizard] += pitch_error * 0.6;
        self.cast(wizard, Spell::Firebolt);
    }

    fn steer_rival(&mut self, wizard: usize, destination: [f32; 3], dt: f32) {
        let offset = [
            destination[0] - self.wizards.pos_x[wizard],
            destination[1] - self.wizards.pos_y[wizard],
            destination[2] - self.wizards.pos_z[wizard],
        ];
        let distance = length3(offset[0], offset[1], offset[2]) + 0.001;
        let mut speed = RIVAL_BASE_SPEED + self.session.realm as f32 * RIVAL_SPEED_PER_REALM;
        if self.wizards.haste_remaining[wizard] > 0.0 {
            speed *= RIVAL_HASTE_MULTIPLIER;
        }
        self.wizards.vel_x[wizard] = approach(
            self.wizards.vel_x[wizard],
            offset[0] / distance * speed,
            RIVAL_TURN_RESPONSE,
            dt,
        );
        self.wizards.vel_y[wizard] = approach(
            self.wizards.vel_y[wizard],
            offset[1] / distance * speed * 0.7,
            RIVAL_TURN_RESPONSE,
            dt,
        );
        self.wizards.vel_z[wizard] = approach(
            self.wizards.vel_z[wizard],
            offset[2] / distance * speed,
            RIVAL_TURN_RESPONSE,
            dt,
        );
        self.wizards.pos_x[wizard] =
            clamp_to_world(self.wizards.pos_x[wizard] + self.wizards.vel_x[wizard] * dt);
        self.wizards.pos_y[wizard] += self.wizards.vel_y[wizard] * dt;
        self.wizards.pos_z[wizard] =
            clamp_to_world(self.wizards.pos_z[wizard] + self.wizards.vel_z[wizard] * dt);
        let floor = self.surface_at(self.wizards.pos_x[wizard], self.wizards.pos_z[wizard])
            + RIVAL_GROUND_CLEARANCE;
        if self.wizards.pos_y[wizard] < floor {
            self.wizards.pos_y[wizard] = floor;
            self.wizards.vel_y[wizard] = max(self.wizards.vel_y[wizard], 0.0);
        }
        if self.wizards.pos_y[wizard] > RIVAL_CEILING {
            self.wizards.pos_y[wizard] = RIVAL_CEILING;
        }
        // while duelling the rival keeps its aim, otherwise it faces its path
        if self.wizards.plan[wizard] != RivalPlan::DuelPlayer {
            self.wizards.yaw[wizard] = atan2(offset[0], offset[2]);
        }
        if self.rng.chance(0.5) {
            let jitter_x = self.rng.range(-4.0, 4.0);
            let jitter_z = self.rng.range(-4.0, 4.0);
            let drift = self.rng.range(-2.0, 2.0);
            self.spawn_particle(
                [
                    self.wizards.pos_x[wizard] + jitter_x,
                    self.wizards.pos_y[wizard] - 3.0,
                    self.wizards.pos_z[wizard] + jitter_z,
                ],
                [0.0, drift, 0.0],
                0.6,
                2.0,
                [1.0, 0.35, 0.3],
                0.0,
                2.0,
            );
        }
    }
}

#[inline]
fn min_i32(a: i32, b: i32) -> i32 {
    if a < b {
        a
    } else {
        b
    }
}
