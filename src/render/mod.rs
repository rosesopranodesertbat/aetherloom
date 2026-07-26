//! Everything the renderer reads: instance and particle buffers, minimap
//! blips, the state block, the prototype meshes and the minimap raster.
//!
//! Every model is assembled here from the prototype meshes in `mesh.rs`.
//! Two rules keep that from reading as a pile of boxes: nothing is placed
//! by a formula that gives every instance the same answer — each entity
//! carries a stable seed and jitters its own proportions off it — and
//! pieces overlap rather than butt together.
//!
//! ```text
//! mod.rs      buffers, the frame, shared body/detail helpers
//! mesh.rs     the ten prototype meshes
//! scenery.rs  palms, boulders, stumps
//! castle.rs   keeps and ruins
//! ground.rs   walkers: worm, troll, villager, soldier, nest
//! flyer.rs    wasp, griffin, dragon, wraith, balloon, and the wing
//! effects.rs  carpets, orbs, projectiles, fireball
//! minimap.rs  the minimap raster
//! ```

use crate::math::*;
use crate::types::*;
use crate::world::*;

/// Scenery beyond this is not submitted at all; the haze hides the pop-in.
const SCENERY_CULL_RANGE: f32 = 1050.0;
/// Scenery is built from a dozen-odd pieces up close and a couple far away.
/// Without this the instance buffer is exhausted by trees before a single
/// creature is drawn — half the map is inside the cull range at any time.
const SCENERY_DETAIL_RANGE: f32 = 380.0;
const SCENERY_MID_RANGE: f32 = 610.0;
/// Scenery stops being submitted at this instance count, leaving the rest of
/// the buffer for whatever the frame still has to draw.
const SCENERY_INSTANCE_BUDGET: usize = MAX_INSTANCES * 3 / 4;
/// Fronds per palm crown, and the droop of their outer half.
const PALM_FRONDS: i32 = 7;
const PALM_FROND_DROOP: f32 = 2.6;
/// Lumps per boulder. Overlapping ellipsoids read as weathered stone where a
/// single box reads as a crate.
const ROCK_LUMPS: i32 = 3;
/// Above this the caster is too high for its shadow to read, and it is dropped.
const SHADOW_MAX_ALTITUDE: f32 = 210.0;
/// How much wider the blob gets at that altitude, and how much it fades.
const SHADOW_SPREAD: f32 = 1.9;
const SHADOW_FADE: f32 = 0.78;
/// Clearance above the ground, so the disc never fights the terrain in depth.
const SHADOW_LIFT: f32 = 0.6;
/// Plinth half-width. The tower ring must fit inside it.
const CASTLE_FOOTPRINT: f32 = 27.0;
/// Height of the terrace above the levelled pad.
const CASTLE_DECK_HEIGHT: f32 = 6.0;
/// How far the plinth is sunk below the lowest ground it covers.
const CASTLE_PLINTH_SINK: f32 = 7.0;
/// Cap on that sinking, for ground later blown out from under it.
const CASTLE_PLINTH_MAX_DROP: f32 = 26.0;
const CASTLE_TOWER_RING: f32 = 19.0;
const CASTLE_KEEP_BASE_HEIGHT: f32 = 14.0;
const CASTLE_KEEP_HEIGHT_PER_TIER: f32 = 7.0;


mod castle;
mod effects;
mod flyer;
mod ground;
mod mesh;
mod minimap;
mod preview;
mod scenery;

pub use mesh::MeshLibrary;
pub use minimap::Minimap;
pub(crate) use preview::{PREVIEW_SCENE_COUNT, PREVIEW_VARIANT_COUNT};

impl World {
    // ---- buffer writers ----------------------------------------------------
    fn push_instance(
        &mut self,
        position: [f32; 3],
        size: [f32; 3],
        colour: [f32; 3],
        yaw: f32,
        glow: f32,
        shape: Shape,
    ) {
        let r = &mut self.render;
        if r.instance_count >= MAX_INSTANCES {
            return;
        }
        let base = r.instance_count * INSTANCE_STRIDE;
        r.instances[base] = position[0];
        r.instances[base + 1] = position[1];
        r.instances[base + 2] = position[2];
        r.instances[base + 3] = size[0];
        r.instances[base + 4] = size[1];
        r.instances[base + 5] = size[2];
        r.instances[base + 6] = colour[0];
        r.instances[base + 7] = colour[1];
        r.instances[base + 8] = colour[2];
        r.instances[base + 9] = yaw;
        r.instances[base + 10] = 0.0;
        r.instances[base + 11] = 0.0;
        r.instances[base + 12] = glow;
        r.instances[base + 13] = shape.as_f32();
        r.instance_count += 1;
    }

    /// The same, tilted. Yaw alone leaves every piece of every model standing
    /// to attention on the world axes, which is most of why a model assembled
    /// from prototypes reads as a stack of blocks. A drooping frond, a splayed
    /// limb, a canted roof and a banked wing all need the other two angles.
    #[allow(clippy::too_many_arguments)]
    fn push_oriented(
        &mut self,
        position: [f32; 3],
        size: [f32; 3],
        colour: [f32; 3],
        rotation: Rotation,
        glow: f32,
        shape: Shape,
    ) {
        let r = &mut self.render;
        if r.instance_count >= MAX_INSTANCES {
            return;
        }
        let base = r.instance_count * INSTANCE_STRIDE;
        r.instances[base] = position[0];
        r.instances[base + 1] = position[1];
        r.instances[base + 2] = position[2];
        r.instances[base + 3] = size[0];
        r.instances[base + 4] = size[1];
        r.instances[base + 5] = size[2];
        r.instances[base + 6] = colour[0];
        r.instances[base + 7] = colour[1];
        r.instances[base + 8] = colour[2];
        r.instances[base + 9] = rotation.yaw;
        r.instances[base + 10] = rotation.pitch;
        r.instances[base + 11] = rotation.roll;
        r.instances[base + 12] = glow;
        r.instances[base + 13] = shape.as_f32();
        r.instance_count += 1;
    }

    /// A blob shadow on the ground. It broadens and fades the higher the
    /// caster is, which is the only altitude cue you get looking straight down
    /// at a flat sea of terrain.
    fn push_shadow(&mut self, x: f32, z: f32, y: f32, radius: f32, strength: f32) {
        let ground = self.height_at(x, z);
        if ground < SEA_LEVEL {
            return; // over water: nothing to fall on
        }
        let altitude = max(y - ground, 0.0);
        if altitude > SHADOW_MAX_ALTITUDE {
            return;
        }
        let spread = 1.0 + altitude / SHADOW_MAX_ALTITUDE * SHADOW_SPREAD;
        let fade = 1.0 - altitude / SHADOW_MAX_ALTITUDE * SHADOW_FADE;
        self.push_instance(
            [x, ground + SHADOW_LIFT, z],
            [radius * 2.0 * spread, 1.0, radius * 2.0 * spread],
            [strength * fade, 0.0, 0.0],
            0.0,
            0.0,
            Shape::Shadow,
        );
    }

    fn push_blip(&mut self, x: f32, z: f32, blip: MapBlip, scale: f32) {
        let r = &mut self.render;
        if r.map_blip_count >= MAX_MAP_BLIPS {
            return;
        }
        let base = r.map_blip_count * MAP_BLIP_STRIDE;
        r.map_blips[base] = x;
        r.map_blips[base + 1] = z;
        r.map_blips[base + 2] = blip.as_f32();
        r.map_blips[base + 3] = scale;
        r.map_blip_count += 1;
    }

    // ---- frame -------------------------------------------------------------
    pub fn build_frame(&mut self) {
        self.render.instance_count = 0;
        self.render.particle_count = 0;
        self.render.map_blip_count = 0;
        // Scenery is drawn last, and to a budget: there is far more of it than
        // of anything else, and a hillside of palms must never crowd out the
        // creature about to eat you.
        for wizard in 0..=self.session.rival_count {
            self.draw_castle(wizard);
        }
        for index in 0..MAX_CREATURES {
            if self.creatures.alive[index] {
                self.draw_creature(index);
            }
        }
        self.draw_rival_carpets();
        self.draw_player_carpet();
        let orb_count = self.draw_orbs();
        self.draw_projectiles();
        self.draw_scenery();
        self.pack_particles();
        self.push_blip(
            self.wizards.pos_x[PLAYER],
            self.wizards.pos_z[PLAYER],
            MapBlip::Player,
            1.6,
        );
        self.write_state_block(orb_count);
    }

    fn draw_creature(&mut self, index: usize) {
        let kind = self.creatures.kind[index];
        let body = CreatureBody::of(self, index);
        match kind {
            CreatureKind::SandWorm => self.draw_sand_worm(&body),
            CreatureKind::Wasp => self.draw_wasp(&body),
            CreatureKind::Troll => self.draw_troll(&body),
            CreatureKind::Griffin => self.draw_griffin(&body),
            CreatureKind::Nest => self.draw_nest(&body),
            CreatureKind::Wraith => self.draw_wraith(&body),
            CreatureKind::Balloon => self.draw_balloon(&body),
            CreatureKind::Dragon => self.draw_dragon(&body),
            CreatureKind::Villager => self.draw_villager(&body),
            CreatureKind::Soldier => self.draw_soldier(&body),
        }
        let blip = match (kind, self.creatures.faction[index]) {
            (CreatureKind::Balloon, _) => MapBlip::Balloon,
            (CreatureKind::Nest, _) => MapBlip::Nest,
            (_, faction) if faction.is_player() => MapBlip::AlliedCreature,
            (_, Faction(2)) => MapBlip::RivalOrb,
            _ => MapBlip::WildCreature,
        };
        // Dot size tracks the creature's bulk, so the minimap says what is out
        // there and not merely that something is. Every kind gets its own size.
        let scale = match kind {
            CreatureKind::Dragon => 1.7,
            CreatureKind::Nest => 1.5,
            CreatureKind::Troll => 1.25,
            CreatureKind::Griffin => 1.1,
            CreatureKind::SandWorm => 1.0,
            CreatureKind::Wraith => 0.9,
            CreatureKind::Balloon => 0.8,
            CreatureKind::Wasp => 0.7,
            CreatureKind::Soldier => 0.55,
            CreatureKind::Villager => 0.45,
        };
        self.push_blip(body.x, body.z, blip, scale);
        let shadow_radius = match kind {
            CreatureKind::Dragon => 20.0,
            CreatureKind::Nest => 14.0,
            CreatureKind::Balloon => 9.0,
            CreatureKind::Troll => 8.0,
            CreatureKind::SandWorm => 11.0,
            CreatureKind::Griffin => 10.0,
            CreatureKind::Wasp => 5.0,
            CreatureKind::Wraith => 6.0,
            CreatureKind::Villager => 2.4,
            CreatureKind::Soldier => 3.0,
        };
        self.push_shadow(body.x, body.z, body.y, shadow_radius, 0.55);
    }

    fn pack_particles(&mut self) {
        for index in 0..MAX_PARTICLES {
            if self.particles.life[index] <= 0.0 {
                continue;
            }
            // fades and shrinks together as it ages
            let remaining = self.particles.life[index] / self.particles.initial_life[index];
            let base = self.render.particle_count * PARTICLE_STRIDE;
            self.render.particles[base] = self.particles.pos_x[index];
            self.render.particles[base + 1] = self.particles.pos_y[index];
            self.render.particles[base + 2] = self.particles.pos_z[index];
            self.render.particles[base + 3] =
                self.particles.size[index] * (0.35 + remaining * 0.9);
            self.render.particles[base + 4] = self.particles.red[index];
            self.render.particles[base + 5] = self.particles.green[index];
            self.render.particles[base + 6] = self.particles.blue[index];
            self.render.particles[base + 7] = remaining * remaining;
            self.render.particle_count += 1;
        }
    }

    // ---- the state block ---------------------------------------------------
    /// Layout is fixed: `game.js` indexes it directly. Slots 40..52 are spell
    /// cooldowns, 60..72 unlocks, 80..92 affordability, 100..112 prices.
    fn write_state_block(&mut self, orb_count: usize) {
        let ground = self.height_at(self.wizards.pos_x[PLAYER], self.wizards.pos_z[PLAYER]);
        let target = self.realm_target();
        let mut best_rival_store = 0.0f32;
        for wizard in 1..=self.session.rival_count {
            best_rival_store = max(best_rival_store, self.castles.stored_mana[wizard]);
        }
        let state = &mut self.render.state;
        state[0] = self.wizards.pos_x[PLAYER];
        state[1] = self.wizards.pos_y[PLAYER];
        state[2] = self.wizards.pos_z[PLAYER];
        state[3] = self.wizards.yaw[PLAYER];
        state[4] = self.wizards.pitch[PLAYER];
        state[5] = self.wizards.roll[PLAYER];
        state[6] = length3(
            self.wizards.vel_x[PLAYER],
            self.wizards.vel_y[PLAYER],
            self.wizards.vel_z[PLAYER],
        );
        state[7] = self.wizards.health[PLAYER];
        state[8] = WIZARD_MAX_HEALTH;
        state[9] = self.wizards.mana[PLAYER];
        state[10] = BASE_MANA_CAP
            + self.castles.tier[PLAYER] as f32 * MANA_CAP_PER_TIER
            + self.wizards.mana_cap_bonus[PLAYER];
        state[11] = self.spells.selected as f32;
        state[12] = SPELL_COUNT as f32;
        state[13] = self.castles.stored_mana[PLAYER] / target;
        state[14] = best_rival_store / target;
        state[15] = 1.0;
        state[16] = self.castles.tier[PLAYER] as f32;
        state[17] = self.session.realm as f32;
        state[18] = self.session.outcome.as_f32();
        state[19] = self.session.screen_shake;
        state[20] = self.wizards.ward_remaining[PLAYER];
        state[21] = self.wizards.haste_remaining[PLAYER];
        state[22] = ground;
        state[23] = self.wizards.pos_y[PLAYER] - max(ground, SEA_LEVEL);
        state[24] = self.session.kills;
        state[25] = self.castles.stored_mana[PLAYER];
        state[26] = 0.0;
        state[27] = self.castles.health[PLAYER] / self.castles.max_health[PLAYER];
        state[28] = self.wizards.health[1] / WIZARD_MAX_HEALTH;
        state[29] = if self.wizards.alive[1] {
            sqrt(length_sq2(
                self.wizards.pos_x[1] - self.wizards.pos_x[PLAYER],
                self.wizards.pos_z[1] - self.wizards.pos_z[PLAYER],
            ))
        } else {
            -1.0
        };
        state[30] = orb_count as f32;
        state[31] = 0.0;
        state[32] = self.session.elapsed;
        state[33] = self.castles.pos_x[PLAYER];
        state[34] = self.castles.pos_z[PLAYER];
        state[35] = self.castles.pos_y[PLAYER];
        state[36] = self.session.rival_count as f32;
        state[37] = GRID_WIDTH as f32;
        state[38] = CELL_SIZE;
        // fortress progress: fill, what this tier can hold, and the target
        state[53] = CASTLE_CAPACITY_PER_TIER * self.castles.tier[PLAYER] as f32 / target;
        state[54] = self.castles.stored_mana[PLAYER];
        state[55] = CASTLE_CAPACITY_PER_TIER * self.castles.tier[PLAYER] as f32;
        state[56] = target;
        state[57] = MANA_REGEN_CEILING;

        for slot in 0..SPELL_COUNT {
            let spell = Spell::ALL[slot];
            let price = match spell {
                Spell::Fortress => {
                    34.0 + self.castles.tier[PLAYER] as f32 * 26.0
                }
                other => other.base_cost(),
            };
            let cooldown = spell.cooldown();
            self.render.state[40 + slot] = if cooldown > 0.0 {
                self.spells.cooldown_remaining[slot] / cooldown
            } else {
                0.0
            };
            self.render.state[60 + slot] = if self.spells.unlocked[slot] { 1.0 } else { 0.0 };
            self.render.state[80 + slot] = if self.wizards.mana[PLAYER] >= price {
                1.0
            } else {
                0.0
            };
            self.render.state[100 + slot] = price;
        }
    }

    // ---- camera ------------------------------------------------------------
    /// Builds the view-projection matrix and its inverse for the renderer.
    #[allow(clippy::too_many_arguments)]
    pub fn update_camera(
        &mut self,
        fov_y: f32,
        aspect: f32,
        near: f32,
        far: f32,
        eye: [f32; 3],
        centre: [f32; 3],
        up: [f32; 3],
    ) {
        let projection = perspective(fov_y, aspect, near, far);
        let view = look_at(eye, centre, up);
        let view_projection = multiply(&projection, &view);
        let inverse = invert(&view_projection);
        self.camera_matrices[..16].copy_from_slice(&view_projection);
        self.camera_matrices[16..].copy_from_slice(&inverse);
    }
}

/// Orientation of one instance. The shader applies these as yaw about Y, then
/// pitch about X, then roll about Z, all in the instance's own space.
#[derive(Clone, Copy)]
pub(crate) struct Rotation {
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
}

impl Rotation {
    pub fn new(yaw: f32, pitch: f32, roll: f32) -> Rotation {
        Rotation { yaw, pitch, roll }
    }
    /// Heading only, for anything that genuinely does stand upright.
    pub fn facing(yaw: f32) -> Rotation {
        Rotation { yaw, pitch: 0.0, roll: 0.0 }
    }
}

/// How much geometry a creature is worth at its current distance. A troll is
/// forty pieces close up and six across the bay; without this the instance
/// buffer is spent on things two pixels wide.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BodyDetail {
    Full,
    Reduced,
    Distant,
}

impl BodyDetail {
    pub fn at_least(self, tier: BodyDetail) -> bool {
        self.rank() >= tier.rank()
    }
    fn rank(self) -> u8 {
        match self {
            BodyDetail::Distant => 0,
            BodyDetail::Reduced => 1,
            BodyDetail::Full => 2,
        }
    }
}

/// Beyond these the creature loses its trim, then its limbs.
const BODY_FULL_RANGE: f32 = 300.0;
const BODY_REDUCED_RANGE: f32 = 680.0;

/// How much geometry a scenery item is worth at its current distance.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SceneryDetail {
    Full,
    Reduced,
    Distant,
}

struct CreatureBody {
    x: f32,
    y: f32,
    z: f32,
    facing: f32,
    facing_sin: f32,
    facing_cos: f32,
    phase: f32,
    /// 1.0 while the hit flash is showing, 0.0 otherwise.
    hurt: f32,
    faction: Faction,
    /// How much of this creature is worth drawing from here.
    detail: BodyDetail,
    /// Slot this creature occupies. Everything that should differ between two
    /// creatures of the same kind — proportions, tint, which horn is broken —
    /// is hashed off this, so the difference holds still instead of flickering
    /// frame to frame the way an RNG draw would.
    slot: usize,
}

impl CreatureBody {
    fn of(world: &World, index: usize) -> CreatureBody {
        let facing = world.creatures.facing[index];
        CreatureBody {
            x: world.creatures.pos_x[index],
            y: world.creatures.pos_y[index],
            z: world.creatures.pos_z[index],
            facing,
            facing_sin: sin(facing),
            facing_cos: cos(facing),
            phase: world.creatures.phase[index],
            hurt: if world.creatures.hurt_flash[index] > 0.0 {
                1.0
            } else {
                0.0
            },
            faction: world.creatures.faction[index],
            detail: {
                let range_sq = length_sq3(
                    world.creatures.pos_x[index] - world.wizards.pos_x[PLAYER],
                    world.creatures.pos_y[index] - world.wizards.pos_y[PLAYER],
                    world.creatures.pos_z[index] - world.wizards.pos_z[PLAYER],
                );
                if range_sq < BODY_FULL_RANGE * BODY_FULL_RANGE {
                    BodyDetail::Full
                } else if range_sq < BODY_REDUCED_RANGE * BODY_REDUCED_RANGE {
                    BodyDetail::Reduced
                } else {
                    BodyDetail::Distant
                }
            },
            slot: index,
        }
    }

    /// Stable pseudo-random in -1..1 for this creature and this `salt`. Two
    /// different salts give two uncorrelated numbers for the same creature,
    /// which is what lets one routine vary a dozen proportions independently.
    fn vary(&self, salt: f32) -> f32 {
        // An integer hash, not a sin-fract: at f32 precision the usual
        // `fract(sin(x) * 43758.5)` trick quantises to about a two-hundredth
        // and correlates badly between nearby slots, which shows up as a row
        // of villagers all built to the same three heights.
        let mut hash = (self.slot as u32)
            .wrapping_mul(747_796_405)
            .wrapping_add((salt * 97.0) as i32 as u32);
        hash ^= hash >> 15;
        hash = hash.wrapping_mul(2_246_822_519);
        hash ^= hash >> 13;
        hash = hash.wrapping_mul(3_266_489_917);
        hash ^= hash >> 16;
        (hash & 0xffff) as f32 / 32768.0 - 1.0
    }

    /// Stable pseudo-random in 0..1.
    fn vary_unit(&self, salt: f32) -> f32 {
        self.vary(salt) * 0.5 + 0.5
    }

    /// True for roughly `odds` of creatures, fixed per creature. For features
    /// that are present or absent rather than scaled: a broken horn, a torn
    /// ear, a hat.
    fn quirk(&self, salt: f32, odds: f32) -> bool {
        self.vary_unit(salt) < odds
    }

    /// `base` nudged by this creature's own variation, so a crowd of the same
    /// kind is not a crowd of clones.
    fn tint(&self, base: [f32; 3], salt: f32, amount: f32) -> [f32; 3] {
        let shift = self.vary(salt) * amount;
        let warm = self.vary(salt + 1.7) * amount * 0.6;
        [
            clamp(base[0] + shift + warm, 0.0, 1.0),
            clamp(base[1] + shift, 0.0, 1.0),
            clamp(base[2] + shift - warm, 0.0, 1.0),
        ]
    }

    /// Orientation with this creature's heading and an extra tilt, for limbs
    /// and plates that should not sit square to the world.
    fn tilted(&self, pitch: f32, roll: f32) -> Rotation {
        Rotation::new(self.facing, pitch, roll)
    }

    /// A point `forward` along the facing, `up` above, and `side` to the
    /// creature's own right.
    fn ahead(&self, forward: f32, up: f32, side: f32) -> [f32; 3] {
        [
            self.x + self.facing_sin * forward + self.facing_cos * side,
            self.y + up,
            self.z + self.facing_cos * forward - self.facing_sin * side,
        ]
    }
}
