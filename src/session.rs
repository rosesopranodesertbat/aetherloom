//! Realm setup and the per-frame update order.

use crate::math::*;
use crate::types::*;
use crate::world::*;

/// Rival wizards appear one at a time; the second joins from this realm on.
const SECOND_RIVAL_FROM_REALM: i32 = 5;
/// Keeps rival keeps from being generated on top of each other.
const MIN_CASTLE_SEPARATION: f32 = 820.0;
const CASTLE_PLACEMENT_ATTEMPTS: i32 = 200;
/// Minimum ground height a castle, nest or creature will be placed on.
const CASTLE_MIN_GROUND: f32 = 24.0;
const NEST_MIN_GROUND: f32 = 14.0;
const CREATURE_MIN_GROUND: f32 = 6.0;
const ORB_MIN_GROUND: f32 = 4.0;

/// Counts scale with the map, which is six times the area of the original —
/// the old numbers left everything piled into one corner.
const NESTS_BASE: i32 = 9;
const NESTS_PER_REALM: i32 = 2;
const WILDLIFE_BASE: i32 = 24;
const WILDLIFE_PER_REALM: i32 = 6;
/// Nothing hostile is placed within this radius of the player's keep, so the
/// opening minute is exploration rather than a dogfight on the doorstep.
const SAFE_RADIUS: f32 = 900.0;
/// Nests are pushed out further still — they are what refills the realm.
const NEST_SAFE_RADIUS: f32 = 1350.0;
/// Attempts to find a site outside the safe radius before giving up and
/// accepting whatever the last draw was.
const DISPERSAL_ATTEMPTS: i32 = 24;

/// Each keep keeps a small standing population: mostly villagers, with a
/// handful of soldiers to defend them.
const GARRISON_VILLAGERS: i32 = 7;
const GARRISON_SOLDIERS: i32 = 4;
const GARRISON_RING: (f32, f32) = (46.0, 130.0);

/// Loose mana seeded at the start so the realm is immediately playable.
const SEEDED_ORBS: i32 = 96;
const SEEDED_ORB_VALUE: (f32, f32) = (5.0, 10.0);
/// From this realm on, dragons start appearing in the initial spread.
const DRAGON_FROM_REALM: i32 = 6;
const DRAGON_ODDS: i32 = 14;

const CASTLE_BASE_HEALTH: f32 = 600.0;
const CASTLE_HEALTH_PER_REALM: f32 = 60.0;
const PLAYER_STARTING_MANA: f32 = 45.0;
const RIVAL_STARTING_MANA: f32 = 60.0;
const FIRST_BALLOON_DELAY: f32 = 3.0;

/// Seconds between checks for whether the realm needs restocking.
const RESTOCK_INTERVAL: f32 = 9.0;
const RESTOCK_FIRST_CHECK: f32 = 4.0;
const RESTOCK_FLOOR_BASE: i32 = 7;
const SHAKE_DECAY_RATE: f32 = 1.8;

impl World {
    pub fn begin_realm(&mut self, seed: u32, realm: i32) {
        self.rng.reseed(seed);
        self.session = Session {
            realm,
            outcome: Outcome::InProgress,
            elapsed: 0.0,
            screen_shake: 0.0,
            kills: 0.0,
            rival_count: if realm >= SECOND_RIVAL_FROM_REALM { 2 } else { 1 },
            restock_timer: RESTOCK_FIRST_CHECK,
        };
        self.unlock_spells_for(realm);
        self.clear_entities();
        self.generate_terrain();
        self.scatter_scenery();
        self.spells.selected = 0;
        self.input = Input {
            thrust: 0.0,
            strafe: 0.0,
            lift: 0.0,
            yaw_delta: 0.0,
            pitch_delta: 0.0,
            braking: false,
        };
        self.place_castles(realm);
        self.populate(realm);
        self.garrison_castles();
        self.terrain.dirty_first_row = 0;
        self.terrain.dirty_last_row = GRID_MAX_INDEX;
        self.build_frame();
    }

    fn unlock_spells_for(&mut self, realm: i32) {
        for slot in 0..SPELL_COUNT {
            self.spells.unlocked[slot] = Spell::ALL[slot].unlocked_at_realm() <= realm;
            self.spells.cooldown_remaining[slot] = 0.0;
        }
    }

    fn clear_entities(&mut self) {
        for i in 0..MAX_CREATURES {
            self.creatures.alive[i] = false;
        }
        for i in 0..MAX_PROJECTILES {
            self.projectiles.alive[i] = false;
        }
        for i in 0..MAX_ORBS {
            self.orbs.alive[i] = false;
        }
        for i in 0..MAX_PARTICLES {
            self.particles.life[i] = 0.0;
        }
        for wizard in 0..MAX_WIZARDS {
            self.wizards.alive[wizard] = false;
        }
    }

    fn place_castles(&mut self, realm: i32) {
        for wizard in 0..=self.session.rival_count {
            let site = self.pick_castle_site(wizard);
            let (x, z) = World::cell_to_world(site);
            self.level_castle_pad(x, z);
            let pad_height = self.height_at(x, z);

            self.castles.pos_x[wizard] = x;
            self.castles.pos_z[wizard] = z;
            self.castles.pos_y[wizard] = pad_height;
            self.castles.max_health[wizard] =
                CASTLE_BASE_HEALTH + realm as f32 * CASTLE_HEALTH_PER_REALM;
            self.castles.health[wizard] = self.castles.max_health[wizard];
            self.castles.tier[wizard] = 1;
            self.castles.stored_mana[wizard] = 0.0;
            self.castles.balloon_timer[wizard] = FIRST_BALLOON_DELAY;

            self.wizards.alive[wizard] = true;
            self.wizards.pos_x[wizard] = x;
            self.wizards.pos_z[wizard] = z + 40.0;
            self.wizards.pos_y[wizard] = pad_height + 40.0;
            self.wizards.vel_x[wizard] = 0.0;
            self.wizards.vel_y[wizard] = 0.0;
            self.wizards.vel_z[wizard] = 0.0;
            self.wizards.yaw[wizard] = 0.0;
            self.wizards.pitch[wizard] = 0.0;
            self.wizards.roll[wizard] = 0.0;
            self.wizards.health[wizard] = WIZARD_MAX_HEALTH;
            self.wizards.mana[wizard] = if wizard == PLAYER {
                PLAYER_STARTING_MANA
            } else {
                RIVAL_STARTING_MANA
            };
            self.wizards.mana_cap_bonus[wizard] = 0.0;
            self.wizards.ward_remaining[wizard] = 0.0;
            self.wizards.haste_remaining[wizard] = 0.0;
            self.wizards.respawn_remaining[wizard] = 0.0;
            self.wizards.cast_cooldown[wizard] = 0.0;
            self.wizards.time_since_hit[wizard] = 0.0;
            self.wizards.plan[wizard] = RivalPlan::GatherMana;
            self.wizards.plan_remaining[wizard] = 0.0;
        }
    }

    /// Tries repeatedly for high ground far enough from the keeps already
    /// placed, and settles for the last candidate if the realm is too cramped.
    fn pick_castle_site(&mut self, wizard: usize) -> i32 {
        let mut site = self.find_land(CASTLE_MIN_GROUND);
        for _ in 0..CASTLE_PLACEMENT_ATTEMPTS {
            site = self.find_land(CASTLE_MIN_GROUND);
            let (x, z) = World::cell_to_world(site);
            let mut clear = true;
            for other in 0..wizard {
                let gap = length_sq2(x - self.castles.pos_x[other], z - self.castles.pos_z[other]);
                if gap < MIN_CASTLE_SEPARATION * MIN_CASTLE_SEPARATION {
                    clear = false;
                    break;
                }
            }
            if clear {
                break;
            }
        }
        site
    }

    /// Like `find_land`, but rejects sites within `keep_clear` of the player's
    /// keep. Falls back to an unfiltered draw so a cramped realm still fills.
    fn find_land_away_from_home(&mut self, min_height: f32, keep_clear: f32) -> i32 {
        let home_x = self.castles.pos_x[PLAYER];
        let home_z = self.castles.pos_z[PLAYER];
        for _ in 0..DISPERSAL_ATTEMPTS {
            let site = self.find_land(min_height);
            let (x, z) = World::cell_to_world(site);
            if length_sq2(x - home_x, z - home_z) > keep_clear * keep_clear {
                return site;
            }
        }
        self.find_land(min_height)
    }

    /// Villagers and a few soldiers live around every keep. They are noncombat
    /// scenery with a pulse: worth mana to a raider, and a reason to defend.
    fn garrison_castles(&mut self) {
        for wizard in 0..=self.session.rival_count {
            let centre_x = self.castles.pos_x[wizard];
            let centre_z = self.castles.pos_z[wizard];
            let faction = Faction::of_wizard(wizard);
            for slot in 0..(GARRISON_VILLAGERS + GARRISON_SOLDIERS) {
                let angle = self.rng.range(0.0, core::f32::consts::TAU);
                let radius = self.rng.range(GARRISON_RING.0, GARRISON_RING.1);
                let x = clamp(centre_x + cos(angle) * radius, 0.0, WORLD_SIZE);
                let z = clamp(centre_z + sin(angle) * radius, 0.0, WORLD_SIZE);
                let kind = if slot < GARRISON_VILLAGERS {
                    CreatureKind::Villager
                } else {
                    CreatureKind::Soldier
                };
                self.spawn_creature(kind, x, z, faction);
            }
        }
    }

    fn populate(&mut self, realm: i32) {
        for _ in 0..(NESTS_BASE + realm * NESTS_PER_REALM) {
            let site = self.find_land_away_from_home(NEST_MIN_GROUND, NEST_SAFE_RADIUS);
            let (x, z) = World::cell_to_world(site);
            self.spawn_creature(CreatureKind::Nest, x, z, Faction::WILD);
        }
        for _ in 0..(WILDLIFE_BASE + realm * WILDLIFE_PER_REALM) {
            let site = self.find_land_away_from_home(CREATURE_MIN_GROUND, SAFE_RADIUS);
            let (x, z) = World::cell_to_world(site);
            let mut kind = CreatureKind::from_index(self.rng.below(4));
            if realm >= DRAGON_FROM_REALM && self.rng.below(DRAGON_ODDS) == 0 {
                kind = CreatureKind::Dragon;
            }
            self.spawn_creature(kind, x, z, Faction::WILD);
        }
        for _ in 0..SEEDED_ORBS {
            let site = self.find_land(ORB_MIN_GROUND);
            let (x, z) = World::cell_to_world(site);
            let amount = self.rng.range(SEEDED_ORB_VALUE.0, SEEDED_ORB_VALUE.1);
            let y = self.height_at(x, z) + 4.0;
            self.spawn_orb([x, y, z], amount);
        }
    }

    // ---- per frame ---------------------------------------------------------
    pub fn advance(&mut self, dt: f32) {
        self.render.sound_cue_count = 0;
        if self.session.outcome == Outcome::InProgress {
            self.session.elapsed += dt;
            self.tick_spell_cooldowns(dt);
            self.update_player(dt);
            for wizard in 1..=self.session.rival_count {
                self.update_rival(wizard, dt);
            }
            for wizard in 0..=self.session.rival_count {
                if self.wizards.alive[wizard] {
                    self.update_wizard_upkeep(wizard, dt);
                }
            }
            self.update_creatures(dt);
            self.update_projectiles(dt);
            self.update_orbs(dt);
            self.smoke_damaged_keeps(dt);
            self.update_fires(dt);
            self.restock_wildlife(dt);
            self.settle_outcome();
        }
        self.update_particles(dt);
        if self.session.screen_shake > 0.0 {
            self.session.screen_shake =
                max(self.session.screen_shake - dt * SHAKE_DECAY_RATE, 0.0);
        }
        self.build_frame();
    }

    /// Cracked keeps smoke. This lives in the simulation rather than in the
    /// draw pass because it draws random numbers, and `build_frame` runs inside
    /// `step` — a random draw taken while drawing shifts this generator's whole
    /// sequence, so the world would depend on the renderer.
    fn smoke_damaged_keeps(&mut self, dt: f32) {
        /// Below this share of health a keep starts smoking.
        const SMOKE_THRESHOLD: f32 = 0.6;
        /// Puffs per second from one burning keep.
        const PUFFS_PER_SECOND: f32 = 24.0;
        /// Spread of the plume across the keep, and how high it starts.
        const PLUME_SPREAD: f32 = 16.0;
        const PLUME_RISE: (f32, f32) = (6.0, 28.0);
        for wizard in 0..=self.session.rival_count {
            let full = self.castles.max_health[wizard];
            if full <= 0.0 || self.castles.health[wizard] / full >= SMOKE_THRESHOLD {
                continue;
            }
            if !self.rng.chance(PUFFS_PER_SECOND * dt) {
                continue;
            }
            let offset_x = self.rng.range(-PLUME_SPREAD, PLUME_SPREAD);
            let offset_z = self.rng.range(-PLUME_SPREAD, PLUME_SPREAD);
            let rise = self.rng.range(PLUME_RISE.0, PLUME_RISE.1);
            let drift = [
                self.rng.range(-3.0, 3.0),
                self.rng.range(6.0, 16.0),
                self.rng.range(-3.0, 3.0),
            ];
            self.spawn_particle(
                [
                    self.castles.pos_x[wizard] + offset_x,
                    self.castles.pos_y[wizard] + rise,
                    self.castles.pos_z[wizard] + offset_z,
                ],
                drift,
                1.4,
                5.0,
                [0.35, 0.33, 0.3],
                2.0,
                0.9,
            );
        }
    }

    fn tick_spell_cooldowns(&mut self, dt: f32) {
        for slot in 0..SPELL_COUNT {
            if self.spells.cooldown_remaining[slot] > 0.0 {
                self.spells.cooldown_remaining[slot] -= dt;
            }
        }
    }

    /// A trickle of new wildlife so a cleared realm never goes empty.
    fn restock_wildlife(&mut self, dt: f32) {
        self.session.restock_timer -= dt;
        if self.session.restock_timer > 0.0 {
            return;
        }
        self.session.restock_timer = RESTOCK_INTERVAL;
        let mut wild = 0;
        for i in 0..MAX_CREATURES {
            if self.creatures.alive[i]
                && self.creatures.faction[i].is_wild()
                && self.creatures.kind[i] != CreatureKind::Nest
            {
                wild += 1;
            }
        }
        if wild >= RESTOCK_FLOOR_BASE + self.session.realm {
            return;
        }
        let site = self.find_land_away_from_home(CREATURE_MIN_GROUND, SAFE_RADIUS);
        let (x, z) = World::cell_to_world(site);
        // an occasional griffin among the ground types
        let kind = if self.rng.below(4) == 3 {
            CreatureKind::Griffin
        } else {
            CreatureKind::from_index(self.rng.below(3))
        };
        self.spawn_creature(kind, x, z, Faction::WILD);
    }

    fn settle_outcome(&mut self) {
        let target = self.realm_target();
        if self.castles.stored_mana[PLAYER] >= target {
            self.session.outcome = Outcome::Won;
        }
        for wizard in 1..=self.session.rival_count {
            if self.castles.stored_mana[wizard] >= target {
                self.session.outcome = Outcome::RivalWon;
            }
        }
        if self.wizards.health[PLAYER] <= 0.0 {
            self.session.outcome = Outcome::Died;
        }
    }
}
