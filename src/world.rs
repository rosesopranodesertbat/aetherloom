//! World state and the small shared operations every subsystem needs.
//!
//! Storage is structure-of-arrays with fixed capacity: no allocator, no
//! resizing, and JS can map any of it as a typed array without copying.

use crate::math::*;
use crate::types::*;

// ---- world dimensions ------------------------------------------------------
/// Heightmap resolution. 320 * 4 bytes per row = 1280, a multiple of 256, which
/// is what `writeTexture` demands for `bytesPerRow`.
pub const GRID_WIDTH: i32 = 320;
pub const GRID_MAX_INDEX: i32 = GRID_WIDTH - 1;
pub const CELL_SIZE: f32 = 8.0;
pub const WORLD_SIZE: f32 = GRID_WIDTH as f32 * CELL_SIZE;
pub const SEA_LEVEL: f32 = 0.0;
/// Nothing may sit closer than this to the world edge.
pub const WORLD_MARGIN: f32 = 8.0;

// ---- capacities ------------------------------------------------------------
pub const MAX_CREATURES: usize = 460;
pub const MAX_PROJECTILES: usize = 128;
pub const MAX_ORBS: usize = 768;
pub const MAX_PARTICLES: usize = 4096;
// Raised for the palm groves: scenery is drawn last and to a budget, so this
// is really the ceiling on how much of a forest is on screen at once.
pub const MAX_INSTANCES: usize = 20480;
pub const MAX_SCENERY: usize = 5200;
pub const MAX_MAP_BLIPS: usize = 2048;
pub const MAX_SOUND_CUES: usize = 128;
pub const MAX_WIZARDS: usize = 3;
/// Floats per render instance: position, scale, colour, yaw, glow, shape.
// position(3) size(3) colour(3) yaw/pitch/roll(3) glow shape
pub const INSTANCE_STRIDE: usize = 14;
/// Floats per particle: position, size, colour, alpha.
pub const PARTICLE_STRIDE: usize = 8;
pub const MAP_BLIP_STRIDE: usize = 4;
pub const SOUND_CUE_STRIDE: usize = 4;
pub const STATE_WORDS: usize = 128;

// ---- economy ---------------------------------------------------------------
/// Mana a fortress can hold per tier.
pub const CASTLE_CAPACITY_PER_TIER: f32 = 240.0;
/// Passive regeneration ceiling. Nothing can be laundered into realm progress
/// through it (the fortress is filled by balloons), so it can afford to be
/// generous — it has to at least reach the price of the first Fortress tier.
pub const MANA_REGEN_CEILING: f32 = 72.0;
pub const BASE_MANA_CAP: f32 = 120.0;
pub const MANA_CAP_PER_TIER: f32 = 50.0;
pub const WIZARD_MAX_HEALTH: f32 = 100.0;
/// Mana that must sit inside your fortress to take a realm.
pub const REALM_TARGET_BASE: f32 = 180.0;
pub const REALM_TARGET_PER_LEVEL: f32 = 160.0;

// ---- flight ----------------------------------------------------------------
pub const CARPET_TOP_SPEED: f32 = 132.0;
pub const CARPET_STRAFE_FRACTION: f32 = 0.55;
pub const CARPET_CLIMB_SPEED: f32 = 78.0;
pub const CARPET_BRAKE_FACTOR: f32 = 0.28;
pub const HASTE_SPEED_MULTIPLIER: f32 = 1.75;
/// How briskly velocity swings to the direction you are facing. Brisk enough
/// that turning re-aims the carpet rather than leaving it skating along its
/// previous heading.
pub const CARPET_TURN_RESPONSE: f32 = 5.2;
pub const CARPET_MAX_PITCH: f32 = 1.05;
pub const CARPET_BANK_ANGLE: f32 = 0.42;
pub const CARPET_GROUND_CLEARANCE: f32 = 7.5;
pub const CARPET_CEILING: f32 = 430.0;
/// Descent rate above which hitting the ground starts hurting.
pub const SAFE_DESCENT_SPEED: f32 = 60.0;

pub const RIVAL_CEILING: f32 = 280.0;
pub const RIVAL_GROUND_CLEARANCE: f32 = 16.0;
pub const RESPAWN_DELAY: f32 = 14.0;

// ---- a deterministic xorshift ----------------------------------------------
/// The whole realm is generated from this, so its call order is load-bearing:
/// two runs that draw in a different order produce different worlds.
pub struct Rng {
    state: u32,
}

/// Golden-ratio constant, used whenever the generator would otherwise be at
/// its one dead value.
const RNG_DEFAULT_STATE: u32 = 0x9e37_79b9;

impl Rng {
    /// Starts at zero so `World` is an all-zero constant and lands in `.bss`;
    /// a non-zero field anywhere in the struct would push two megabytes of
    /// arrays into the wasm as initialised data. `unit` repairs the state on
    /// first use, and `begin_realm` reseeds before anything draws.
    pub const fn new() -> Rng {
        Rng { state: 0 }
    }
    pub fn reseed(&mut self, seed: u32) {
        self.state = seed | 1;
    }
    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f32 {
        // xorshift can never reach zero from a live state, so this only ever
        // fires on an un-seeded generator.
        if self.state == 0 {
            self.state = RNG_DEFAULT_STATE;
        }
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        (self.state & 0x00ff_ffff) as f32 / 16_777_216.0
    }
    pub fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.unit()
    }
    /// Uniform integer in [0, count).
    pub fn below(&mut self, count: i32) -> i32 {
        (self.unit() * count as f32) as i32 % count
    }
    pub fn chance(&mut self, probability: f32) -> bool {
        self.unit() < probability
    }
    /// Raw state, mixed into the terrain hash so each seed gets its own noise.
    pub fn state(&self) -> u32 {
        self.state
    }
}

// ---- terrain ---------------------------------------------------------------
pub struct Terrain {
    pub height: [f32; (GRID_WIDTH * GRID_WIDTH) as usize],
    /// Rows touched since the renderer last uploaded, so only a band is sent.
    pub dirty_first_row: i32,
    pub dirty_last_row: i32,
}

// ---- creatures -------------------------------------------------------------
pub struct Creatures {
    pub alive: [bool; MAX_CREATURES],
    pub kind: [CreatureKind; MAX_CREATURES],
    pub faction: [Faction; MAX_CREATURES],
    pub pos_x: [f32; MAX_CREATURES],
    pub pos_y: [f32; MAX_CREATURES],
    pub pos_z: [f32; MAX_CREATURES],
    pub vel_x: [f32; MAX_CREATURES],
    pub vel_y: [f32; MAX_CREATURES],
    pub vel_z: [f32; MAX_CREATURES],
    pub health: [f32; MAX_CREATURES],
    pub max_health: [f32; MAX_CREATURES],
    pub facing: [f32; MAX_CREATURES],
    /// Counts down to the next wander turn, nest hatch, or summon expiry.
    pub timer: [f32; MAX_CREATURES],
    pub attack_cooldown: [f32; MAX_CREATURES],
    /// Animation phase, also used for bobbing and wing beats.
    pub phase: [f32; MAX_CREATURES],
}

// ---- projectiles -----------------------------------------------------------
pub struct Projectiles {
    pub alive: [bool; MAX_PROJECTILES],
    pub kind: [ProjectileKind; MAX_PROJECTILES],
    pub owner: [Faction; MAX_PROJECTILES],
    pub pos_x: [f32; MAX_PROJECTILES],
    pub pos_y: [f32; MAX_PROJECTILES],
    pub pos_z: [f32; MAX_PROJECTILES],
    pub vel_x: [f32; MAX_PROJECTILES],
    pub vel_y: [f32; MAX_PROJECTILES],
    pub vel_z: [f32; MAX_PROJECTILES],
    pub life: [f32; MAX_PROJECTILES],
    pub damage: [f32; MAX_PROJECTILES],
    pub blast_radius: [f32; MAX_PROJECTILES],
}

// ---- mana orbs -------------------------------------------------------------
pub struct Orbs {
    pub alive: [bool; MAX_ORBS],
    pub pos_x: [f32; MAX_ORBS],
    pub pos_y: [f32; MAX_ORBS],
    pub pos_z: [f32; MAX_ORBS],
    pub amount: [f32; MAX_ORBS],
    pub bob_phase: [f32; MAX_ORBS],
    /// `Faction::WILD` until someone casts Claim on it. Balloons only fetch
    /// orbs matching their own faction.
    pub claimed_by: [Faction; MAX_ORBS],
    /// Index of the balloon carrying this orb, if any.
    pub carried_by: [Option<usize>; MAX_ORBS],
}

// ---- particles (ring buffer, oldest silently recycled) ---------------------
pub struct Particles {
    pub pos_x: [f32; MAX_PARTICLES],
    pub pos_y: [f32; MAX_PARTICLES],
    pub pos_z: [f32; MAX_PARTICLES],
    pub vel_x: [f32; MAX_PARTICLES],
    pub vel_y: [f32; MAX_PARTICLES],
    pub vel_z: [f32; MAX_PARTICLES],
    pub life: [f32; MAX_PARTICLES],
    pub initial_life: [f32; MAX_PARTICLES],
    pub size: [f32; MAX_PARTICLES],
    pub red: [f32; MAX_PARTICLES],
    pub green: [f32; MAX_PARTICLES],
    pub blue: [f32; MAX_PARTICLES],
    pub drag: [f32; MAX_PARTICLES],
    pub gravity: [f32; MAX_PARTICLES],
    pub next: usize,
}

/// One particle before it is inserted into the world's ring buffer.
///
/// Fireball impacts are generated as data first so gameplay and the
/// deterministic visual-QA scene exercise the same composition.
#[derive(Clone, Copy)]
pub(crate) struct ParticleSpec {
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub life: f32,
    pub size: f32,
    pub colour: [f32; 3],
    pub gravity: f32,
    pub drag: f32,
}

impl ParticleSpec {
    pub const ZERO: ParticleSpec = ParticleSpec {
        position: [0.0; 3],
        velocity: [0.0; 3],
        life: 0.0,
        size: 0.0,
        colour: [0.0; 3],
        gravity: 0.0,
        drag: 0.0,
    };
}

pub(crate) const MAX_FIREBALL_IMPACT_PARTICLES: usize = 24;

// ---- wizards (index 0 is the player) --------------------------------------
pub struct Wizards {
    pub alive: [bool; MAX_WIZARDS],
    pub pos_x: [f32; MAX_WIZARDS],
    pub pos_y: [f32; MAX_WIZARDS],
    pub pos_z: [f32; MAX_WIZARDS],
    pub vel_x: [f32; MAX_WIZARDS],
    pub vel_y: [f32; MAX_WIZARDS],
    pub vel_z: [f32; MAX_WIZARDS],
    pub yaw: [f32; MAX_WIZARDS],
    pub pitch: [f32; MAX_WIZARDS],
    pub roll: [f32; MAX_WIZARDS],
    pub health: [f32; MAX_WIZARDS],
    pub mana: [f32; MAX_WIZARDS],
    /// Extra personal mana capacity earned by possessing orbs.
    pub mana_cap_bonus: [f32; MAX_WIZARDS],
    pub ward_remaining: [f32; MAX_WIZARDS],
    pub haste_remaining: [f32; MAX_WIZARDS],
    pub respawn_remaining: [f32; MAX_WIZARDS],
    pub cast_cooldown: [f32; MAX_WIZARDS],
    /// Seconds since last taking damage; drives out-of-combat regeneration.
    pub time_since_hit: [f32; MAX_WIZARDS],
    pub plan: [RivalPlan; MAX_WIZARDS],
    pub plan_remaining: [f32; MAX_WIZARDS],
}

// ---- castles ---------------------------------------------------------------
pub struct Castles {
    pub pos_x: [f32; MAX_WIZARDS],
    pub pos_y: [f32; MAX_WIZARDS],
    pub pos_z: [f32; MAX_WIZARDS],
    pub health: [f32; MAX_WIZARDS],
    pub max_health: [f32; MAX_WIZARDS],
    pub tier: [i32; MAX_WIZARDS],
    /// Mana delivered by balloons and held inside. Filling this wins the realm.
    pub stored_mana: [f32; MAX_WIZARDS],
    pub balloon_timer: [f32; MAX_WIZARDS],
}

// ---- scenery ---------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SceneryKind {
    Palm,
    Rock,
}

pub struct Scenery {
    pub pos_x: [f32; MAX_SCENERY],
    pub pos_z: [f32; MAX_SCENERY],
    pub kind: [SceneryKind; MAX_SCENERY],
    pub scale: [f32; MAX_SCENERY],
    pub rotation: [f32; MAX_SCENERY],
    /// Cleared when a palm burns down to a stump.
    pub standing: [bool; MAX_SCENERY],
    /// Seconds of fire left. Zero means not alight.
    pub burn_remaining: [f32; MAX_SCENERY],
    pub count: usize,
}

// ---- output buffers JS reads directly --------------------------------------
pub struct RenderBuffers {
    pub instances: [f32; MAX_INSTANCES * INSTANCE_STRIDE],
    pub instance_count: usize,
    /// Index range of the player's own carpet, so the renderer can drop it
    /// wholesale in first person.
    pub carpet_first: i32,
    pub carpet_last: i32,
    pub particles: [f32; MAX_PARTICLES * PARTICLE_STRIDE],
    pub particle_count: usize,
    pub map_blips: [f32; MAX_MAP_BLIPS * MAP_BLIP_STRIDE],
    pub map_blip_count: usize,
    /// Events accumulate across simulation catch-up steps until the host calls
    /// `clearEvents`, so no intermediate step silently discards its sounds.
    pub sound_cues: [f32; MAX_SOUND_CUES * SOUND_CUE_STRIDE],
    pub sound_cue_count: usize,
    pub state: [f32; STATE_WORDS],
}

// ---- player input ----------------------------------------------------------
#[derive(Clone, Copy)]
pub struct Input {
    pub thrust: f32,
    pub strafe: f32,
    pub lift: f32,
    pub yaw_delta: f32,
    pub pitch_delta: f32,
    pub braking: bool,
}

// ---- spell bookkeeping -----------------------------------------------------
pub struct SpellState {
    pub cooldown_remaining: [f32; SPELL_COUNT],
    pub unlocked: [bool; SPELL_COUNT],
    pub selected: usize,
}

// ---- per-realm session ------------------------------------------------------
pub struct Session {
    pub realm: i32,
    pub outcome: Outcome,
    pub elapsed: f32,
    pub screen_shake: f32,
    pub kills: f32,
    pub rival_count: usize,
    pub restock_timer: f32,
}

// ---- everything ------------------------------------------------------------
pub struct World {
    pub rng: Rng,
    pub terrain: Terrain,
    pub creatures: Creatures,
    pub projectiles: Projectiles,
    pub orbs: Orbs,
    pub particles: Particles,
    pub wizards: Wizards,
    pub castles: Castles,
    pub scenery: Scenery,
    pub render: RenderBuffers,
    pub input: Input,
    pub spells: SpellState,
    pub session: Session,
    /// Centre (xyz) and framing radius of the last isolated model preview.
    pub preview_focus: [f32; 4],
    /// View-projection then its inverse, handed to the renderer each frame.
    pub camera_matrices: [f32; 32],
    pub meshes: crate::render::MeshLibrary,
    pub minimap: crate::render::Minimap,
}

/// Build the complete layered contact effect without touching world state.
///
/// The caller supplies the RNG. Gameplay passes the realm generator so the
/// historical draw budget stays unchanged; previews pass a local fixed seed.
pub(crate) fn build_fireball_impact_particles(
    rng: &mut Rng,
    position: [f32; 3],
    incoming: [f32; 3],
    kind: ProjectileKind,
    output: &mut [ParticleSpec; MAX_FIREBALL_IMPACT_PARTICLES],
) -> usize {
    // Each stochastic particle below consumes exactly five RNG draws.
    // Keeping 22 for Firebolt and 14 for DragonFire matches their former
    // generic bursts, so richer visuals do not perturb later simulation
    // randomness.
    let (shell_count, ember_count, smoke_count) = match kind {
        ProjectileKind::Firebolt => (12, 6, 4),
        ProjectileKind::DragonFire => (8, 4, 2),
        _ => return 0,
    };
    let (warm, head_size) = kind.head_style();
    let scale = head_size / 3.0;
    let hot = [
        1.0,
        min(warm[1] + 0.34, 1.0),
        min(warm[2] + 0.50, 1.0),
    ];
    let smoke = [warm[0] * 0.22, warm[1] * 0.18, warm[2] * 0.20];
    let incoming_length = length3(incoming[0], incoming[1], incoming[2]);
    let recoil = if incoming_length > 0.001 {
        [
            -incoming[0] / incoming_length,
            -incoming[1] / incoming_length,
            -incoming[2] / incoming_length,
        ]
    } else {
        [0.0, 0.0, -1.0]
    };
    let mut count = 0usize;

    // Two nested flashes prevent the centre disappearing between the
    // projectile head and the first expanding shell particles.
    output[count] = ParticleSpec {
        position,
        velocity: [
            recoil[0] * 3.0 * scale,
            (5.0 + recoil[1] * 3.0) * scale,
            recoil[2] * 3.0 * scale,
        ],
        life: 0.13,
        size: 10.0 * scale,
        colour: [1.0, 0.96, 0.82],
        gravity: 0.0,
        drag: 7.0,
    };
    count += 1;
    output[count] = ParticleSpec {
        position,
        velocity: [
            recoil[0] * 5.0 * scale,
            (7.0 + recoil[1] * 5.0) * scale,
            recoil[2] * 5.0 * scale,
        ],
        life: 0.24,
        size: 7.2 * scale,
        colour: hot,
        gravity: 4.0,
        drag: 4.5,
    };
    count += 1;

    // Broad flame shell: biased upward so ground contacts roll into a
    // fireball, and back along the arrival path so a hit reads directionally.
    for _ in 0..shell_count {
        let angle = rng.range(0.0, core::f32::consts::TAU);
        let vertical = rng.range(-0.30, 0.86);
        let horizontal = sqrt(1.0 - vertical * vertical);
        let speed = rng.range(15.0, 33.0) * scale;
        let life = rng.range(0.30, 0.58);
        let size = rng.range(3.6, 6.5) * scale;
        let heat = (vertical + 0.30) * 0.15;
        output[count] = ParticleSpec {
            position,
            velocity: [
                cos(angle) * horizontal * speed + recoil[0] * speed * 0.24,
                vertical * speed + (5.0 + recoil[1] * 4.0) * scale,
                sin(angle) * horizontal * speed + recoil[2] * speed * 0.24,
            ],
            life,
            size,
            colour: [
                min(warm[0] + heat, 1.0),
                min(warm[1] + heat * 0.72, 1.0),
                min(warm[2] + heat * 0.48, 1.0),
            ],
            gravity: 7.0,
            drag: 2.2,
        };
        count += 1;
    }

    // Small fast embers make the recoil direction legible after the broad
    // shell has faded.
    for _ in 0..ember_count {
        let angle = rng.range(0.0, core::f32::consts::TAU);
        let vertical = rng.range(-0.18, 0.94);
        let horizontal = sqrt(1.0 - vertical * vertical);
        let speed = rng.range(30.0, 58.0) * scale;
        let life = rng.range(0.38, 0.78);
        let size = rng.range(1.1, 2.2) * scale;
        output[count] = ParticleSpec {
            position,
            velocity: [
                cos(angle) * horizontal * speed + recoil[0] * speed * 0.36,
                vertical * speed + (8.0 + recoil[1] * 7.0) * scale,
                sin(angle) * horizontal * speed + recoil[2] * speed * 0.36,
            ],
            life,
            size,
            colour: hot,
            gravity: -32.0,
            drag: 1.2,
        };
        count += 1;
    }

    // Sparse smoke follows the recoil rather than hiding the struck model.
    for _ in 0..smoke_count {
        let angle = rng.range(0.0, core::f32::consts::TAU);
        let radius = rng.range(0.4, 2.5) * scale;
        let speed = rng.range(5.0, 12.0) * scale;
        let life = rng.range(0.55, 0.92);
        let size = rng.range(4.2, 7.2) * scale;
        output[count] = ParticleSpec {
            position: [
                position[0] + cos(angle) * radius,
                position[1] + radius * 0.35,
                position[2] + sin(angle) * radius,
            ],
            velocity: [
                cos(angle + 1.1) * speed + recoil[0] * speed * 0.75,
                8.0 * scale + speed + recoil[1] * speed * 0.4,
                sin(angle + 1.1) * speed + recoil[2] * speed * 0.75,
            ],
            life,
            size,
            colour: smoke,
            gravity: 5.0,
            drag: 2.8,
        };
        count += 1;
    }

    count
}

impl World {
    /// Everything starts zeroed so the whole struct lands in `.bss` rather
    /// than being emitted as multi-megabyte initialised data in the wasm.
    pub const fn new() -> World {
        World {
            rng: Rng::new(),
            terrain: Terrain {
                height: [0.0; (GRID_WIDTH * GRID_WIDTH) as usize],
                dirty_first_row: 0,
                dirty_last_row: 0,
            },
            creatures: Creatures {
                alive: [false; MAX_CREATURES],
                kind: [CreatureKind::SandWorm; MAX_CREATURES],
                faction: [Faction::WILD; MAX_CREATURES],
                pos_x: [0.0; MAX_CREATURES],
                pos_y: [0.0; MAX_CREATURES],
                pos_z: [0.0; MAX_CREATURES],
                vel_x: [0.0; MAX_CREATURES],
                vel_y: [0.0; MAX_CREATURES],
                vel_z: [0.0; MAX_CREATURES],
                health: [0.0; MAX_CREATURES],
                max_health: [0.0; MAX_CREATURES],
                facing: [0.0; MAX_CREATURES],
                timer: [0.0; MAX_CREATURES],
                attack_cooldown: [0.0; MAX_CREATURES],
                phase: [0.0; MAX_CREATURES],
            },
            projectiles: Projectiles {
                alive: [false; MAX_PROJECTILES],
                kind: [ProjectileKind::Firebolt; MAX_PROJECTILES],
                owner: [Faction::WILD; MAX_PROJECTILES],
                pos_x: [0.0; MAX_PROJECTILES],
                pos_y: [0.0; MAX_PROJECTILES],
                pos_z: [0.0; MAX_PROJECTILES],
                vel_x: [0.0; MAX_PROJECTILES],
                vel_y: [0.0; MAX_PROJECTILES],
                vel_z: [0.0; MAX_PROJECTILES],
                life: [0.0; MAX_PROJECTILES],
                damage: [0.0; MAX_PROJECTILES],
                blast_radius: [0.0; MAX_PROJECTILES],
            },
            orbs: Orbs {
                alive: [false; MAX_ORBS],
                pos_x: [0.0; MAX_ORBS],
                pos_y: [0.0; MAX_ORBS],
                pos_z: [0.0; MAX_ORBS],
                amount: [0.0; MAX_ORBS],
                bob_phase: [0.0; MAX_ORBS],
                claimed_by: [Faction::WILD; MAX_ORBS],
                carried_by: [None; MAX_ORBS],
            },
            particles: Particles {
                pos_x: [0.0; MAX_PARTICLES],
                pos_y: [0.0; MAX_PARTICLES],
                pos_z: [0.0; MAX_PARTICLES],
                vel_x: [0.0; MAX_PARTICLES],
                vel_y: [0.0; MAX_PARTICLES],
                vel_z: [0.0; MAX_PARTICLES],
                life: [0.0; MAX_PARTICLES],
                initial_life: [0.0; MAX_PARTICLES],
                size: [0.0; MAX_PARTICLES],
                red: [0.0; MAX_PARTICLES],
                green: [0.0; MAX_PARTICLES],
                blue: [0.0; MAX_PARTICLES],
                drag: [0.0; MAX_PARTICLES],
                gravity: [0.0; MAX_PARTICLES],
                next: 0,
            },
            wizards: Wizards {
                alive: [false; MAX_WIZARDS],
                pos_x: [0.0; MAX_WIZARDS],
                pos_y: [0.0; MAX_WIZARDS],
                pos_z: [0.0; MAX_WIZARDS],
                vel_x: [0.0; MAX_WIZARDS],
                vel_y: [0.0; MAX_WIZARDS],
                vel_z: [0.0; MAX_WIZARDS],
                yaw: [0.0; MAX_WIZARDS],
                pitch: [0.0; MAX_WIZARDS],
                roll: [0.0; MAX_WIZARDS],
                health: [0.0; MAX_WIZARDS],
                mana: [0.0; MAX_WIZARDS],
                mana_cap_bonus: [0.0; MAX_WIZARDS],
                ward_remaining: [0.0; MAX_WIZARDS],
                haste_remaining: [0.0; MAX_WIZARDS],
                respawn_remaining: [0.0; MAX_WIZARDS],
                cast_cooldown: [0.0; MAX_WIZARDS],
                time_since_hit: [0.0; MAX_WIZARDS],
                plan: [RivalPlan::GatherMana; MAX_WIZARDS],
                plan_remaining: [0.0; MAX_WIZARDS],
            },
            castles: Castles {
                pos_x: [0.0; MAX_WIZARDS],
                pos_y: [0.0; MAX_WIZARDS],
                pos_z: [0.0; MAX_WIZARDS],
                health: [0.0; MAX_WIZARDS],
                max_health: [0.0; MAX_WIZARDS],
                tier: [0; MAX_WIZARDS],
                stored_mana: [0.0; MAX_WIZARDS],
                balloon_timer: [0.0; MAX_WIZARDS],
            },
            scenery: Scenery {
                pos_x: [0.0; MAX_SCENERY],
                pos_z: [0.0; MAX_SCENERY],
                kind: [SceneryKind::Palm; MAX_SCENERY],
                scale: [0.0; MAX_SCENERY],
                rotation: [0.0; MAX_SCENERY],
                standing: [false; MAX_SCENERY],
                burn_remaining: [0.0; MAX_SCENERY],
                count: 0,
            },
            render: RenderBuffers {
                instances: [0.0; MAX_INSTANCES * INSTANCE_STRIDE],
                instance_count: 0,
                carpet_first: 0,
                carpet_last: 0,
                particles: [0.0; MAX_PARTICLES * PARTICLE_STRIDE],
                particle_count: 0,
                map_blips: [0.0; MAX_MAP_BLIPS * MAP_BLIP_STRIDE],
                map_blip_count: 0,
                sound_cues: [0.0; MAX_SOUND_CUES * SOUND_CUE_STRIDE],
                sound_cue_count: 0,
                state: [0.0; STATE_WORDS],
            },
            input: Input {
                thrust: 0.0,
                strafe: 0.0,
                lift: 0.0,
                yaw_delta: 0.0,
                pitch_delta: 0.0,
                braking: false,
            },
            spells: SpellState {
                cooldown_remaining: [0.0; SPELL_COUNT],
                unlocked: [false; SPELL_COUNT],
                selected: 0,
            },
            // realm and rival_count are set by begin_realm; zero here keeps
            // the whole struct in .bss
            session: Session {
                realm: 0,
                outcome: Outcome::InProgress,
                elapsed: 0.0,
                screen_shake: 0.0,
                kills: 0.0,
                rival_count: 0,
                restock_timer: 0.0,
            },
            preview_focus: [0.0; 4],
            camera_matrices: [0.0; 32],
            meshes: crate::render::MeshLibrary::new(),
            minimap: crate::render::Minimap::new(),
        }
    }

    // ---- derived economy figures ------------------------------------------
    pub fn mana_cap(&self, wizard: usize) -> f32 {
        BASE_MANA_CAP + self.castles.tier[wizard] as f32 * MANA_CAP_PER_TIER
            + self.wizards.mana_cap_bonus[wizard]
    }
    pub fn castle_capacity(&self, wizard: usize) -> f32 {
        CASTLE_CAPACITY_PER_TIER * self.castles.tier[wizard] as f32
    }
    pub fn realm_target(&self) -> f32 {
        REALM_TARGET_BASE + self.session.realm as f32 * REALM_TARGET_PER_LEVEL
    }
    /// Each Fortress tier costs more than the last.
    pub fn fortress_price(&self, wizard: usize) -> f32 {
        34.0 + self.castles.tier[wizard] as f32 * 26.0
    }
    pub fn spell_price(&self, spell: Spell, wizard: usize) -> f32 {
        match spell {
            Spell::Fortress => self.fortress_price(wizard),
            other => other.base_cost(),
        }
    }

    /// Facing direction of a wizard as a unit vector.
    pub fn facing_of(&self, wizard: usize) -> [f32; 3] {
        let pitch_cos = cos(self.wizards.pitch[wizard]);
        [
            sin(self.wizards.yaw[wizard]) * pitch_cos,
            sin(self.wizards.pitch[wizard]),
            cos(self.wizards.yaw[wizard]) * pitch_cos,
        ]
    }

    /// Raise the shake to at least `amount`; smaller jolts never cancel bigger.
    pub fn add_shake(&mut self, amount: f32) {
        if self.session.screen_shake < amount {
            self.session.screen_shake = amount;
        }
    }

    // ---- particles ---------------------------------------------------------
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_particle(
        &mut self,
        position: [f32; 3],
        velocity: [f32; 3],
        life: f32,
        size: f32,
        colour: [f32; 3],
        gravity: f32,
        drag: f32,
    ) {
        let p = &mut self.particles;
        let i = p.next;
        p.next = (p.next + 1) % MAX_PARTICLES;
        p.pos_x[i] = position[0];
        p.pos_y[i] = position[1];
        p.pos_z[i] = position[2];
        p.vel_x[i] = velocity[0];
        p.vel_y[i] = velocity[1];
        p.vel_z[i] = velocity[2];
        p.life[i] = life;
        p.initial_life[i] = life;
        p.size[i] = size;
        p.red[i] = colour[0];
        p.green[i] = colour[1];
        p.blue[i] = colour[2];
        p.gravity[i] = gravity;
        p.drag[i] = drag;
    }

    /// A roughly spherical spray, used for every impact in the game.
    pub fn spawn_burst(
        &mut self,
        position: [f32; 3],
        count: i32,
        speed: f32,
        size: f32,
        colour: [f32; 3],
        life: f32,
    ) {
        const BURST_GRAVITY: f32 = -14.0;
        const BURST_DRAG: f32 = 1.6;
        for _ in 0..count {
            let angle = self.rng.range(0.0, core::f32::consts::TAU);
            let vertical = self.rng.range(-1.0, 1.0);
            let magnitude = speed * self.rng.range(0.3, 1.0);
            let horizontal = sqrt(1.0 - vertical * vertical);
            let life_jitter = life * self.rng.range(0.6, 1.2);
            let size_jitter = size * self.rng.range(0.6, 1.4);
            self.spawn_particle(
                position,
                [
                    cos(angle) * horizontal * magnitude,
                    vertical * magnitude + speed * 0.25,
                    sin(angle) * horizontal * magnitude,
                ],
                life_jitter,
                size_jitter,
                colour,
                BURST_GRAVITY,
                BURST_DRAG,
            );
        }
    }

    /// A fireball contact is more than a spherical spray: a short white-hot
    /// flash sits inside an expanding coloured flame shell, with fast sparks
    /// escaping ahead of a slower dark smoke layer. Firebolt and dragon fire
    /// share the motion but inherit their own projectile palette and scale.
    pub(crate) fn spawn_fireball_impact(
        &mut self,
        position: [f32; 3],
        incoming: [f32; 3],
        kind: ProjectileKind,
    ) {
        let mut particles = [ParticleSpec::ZERO; MAX_FIREBALL_IMPACT_PARTICLES];
        let count = build_fireball_impact_particles(
            &mut self.rng,
            position,
            incoming,
            kind,
            &mut particles,
        );
        for particle in &particles[..count] {
            self.spawn_particle(
                particle.position,
                particle.velocity,
                particle.life,
                particle.size,
                particle.colour,
                particle.gravity,
                particle.drag,
            );
        }
    }

    /// A jagged line of sparks, for lightning and smite beams.
    pub fn spawn_arc(&mut self, from: [f32; 3], to: [f32; 3]) {
        const SPACING: f32 = 5.0;
        const MIN_STEPS: i32 = 4;
        const MAX_STEPS: i32 = 90;
        const MAX_JITTER: f32 = 5.0;
        let delta = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
        let length = length3(delta[0], delta[1], delta[2]);
        let steps = ((length / SPACING) as i32).clamp(MIN_STEPS, MAX_STEPS);
        for step in 0..=steps {
            let t = step as f32 / steps as f32;
            // widest wobble in the middle of the span, tapering to the ends
            let jitter = MAX_JITTER * sin(t * core::f32::consts::PI);
            let offset = [
                self.rng.range(-jitter, jitter),
                self.rng.range(-jitter, jitter),
                self.rng.range(-jitter, jitter),
            ];
            let drift = [
                self.rng.range(-3.0, 3.0),
                self.rng.range(-3.0, 3.0),
                self.rng.range(-3.0, 3.0),
            ];
            let life = self.rng.range(0.14, 0.34);
            let size = self.rng.range(2.0, 4.2);
            self.spawn_particle(
                [
                    from[0] + delta[0] * t + offset[0],
                    from[1] + delta[1] * t + offset[1],
                    from[2] + delta[2] * t + offset[2],
                ],
                drift,
                life,
                size,
                [0.62, 0.80, 1.0],
                0.0,
                1.0,
            );
        }
    }

    /// A hit needs to read as a hit, not just a tint change: a tight cone of
    /// sparks thrown back along the incoming direction, plus a couple of dark
    /// chips knocked loose. Scaled by the blow so a firebolt and a meteor do
    /// not look the same.
    pub fn spawn_impact(&mut self, at: [f32; 3], away: [f32; 3], damage: f32) {
        const MAX_SPARKS: i32 = 9;
        let weight = clamp(damage / 60.0, 0.25, 1.6);
        let sparks = 3 + (weight * MAX_SPARKS as f32) as i32;
        let spread = length3(away[0], away[1], away[2]) + 0.001;
        let push = [away[0] / spread, away[1] / spread, away[2] / spread];
        for _ in 0..sparks {
            let speed = self.rng.range(16.0, 46.0) * weight;
            let scatter = [
                self.rng.range(-0.55, 0.55),
                self.rng.range(-0.35, 0.75),
                self.rng.range(-0.55, 0.55),
            ];
            let life = self.rng.range(0.18, 0.42);
            let size = self.rng.range(1.2, 2.6) * weight;
            let heat = self.rng.range(0.55, 1.0);
            self.spawn_particle(
                at,
                [
                    (push[0] + scatter[0]) * speed,
                    (push[1] + scatter[1]) * speed + 8.0,
                    (push[2] + scatter[2]) * speed,
                ],
                life,
                size,
                [1.0, heat, 0.22],
                -46.0,
                2.4,
            );
        }
        for _ in 0..3 {
            let drift = [
                self.rng.range(-14.0, 14.0),
                self.rng.range(6.0, 22.0),
                self.rng.range(-14.0, 14.0),
            ];
            let life = self.rng.range(0.35, 0.7);
            self.spawn_particle(at, drift, life, 1.8 * weight, [0.22, 0.15, 0.12], -58.0, 1.1);
        }
    }

    /// Sets light to any palm near a blast. Rocks do not burn.
    pub fn ignite_scenery(&mut self, at: [f32; 3], radius: f32) {
        /// A tree burns for this long before it is gone.
        const BURN_SECONDS: f32 = 7.0;
        let radius_sq = radius * radius;
        for i in 0..self.scenery.count {
            if !self.scenery.standing[i]
                || self.scenery.kind[i] != SceneryKind::Palm
                || self.scenery.burn_remaining[i] > 0.0
            {
                continue;
            }
            if length_sq2(self.scenery.pos_x[i] - at[0], self.scenery.pos_z[i] - at[2]) > radius_sq {
                continue;
            }
            self.scenery.burn_remaining[i] = BURN_SECONDS;
        }
    }

    /// Burns down anything alight, throwing flame as it goes. Fire spreads to
    /// close neighbours, so a grove goes up rather than a single tree.
    pub fn update_fires(&mut self, dt: f32) {
        const SPREAD_RADIUS: f32 = 46.0;
        /// Chance per second that a burning tree lights its neighbours.
        const SPREAD_CHANCE: f32 = 0.35;
        for i in 0..self.scenery.count {
            if self.scenery.burn_remaining[i] <= 0.0 {
                continue;
            }
            self.scenery.burn_remaining[i] -= dt;
            let x = self.scenery.pos_x[i];
            let z = self.scenery.pos_z[i];
            let scale = self.scenery.scale[i];
            let ground = self.height_at(x, z);
            if self.rng.chance(0.55) {
                let jitter_x = self.rng.range(-4.0, 4.0) * scale;
                let jitter_z = self.rng.range(-4.0, 4.0) * scale;
                let rise = self.rng.range(14.0, 40.0);
                let life = self.rng.range(0.4, 0.9);
                let size = self.rng.range(2.5, 5.5) * scale;
                let heat = self.rng.range(0.3, 0.75);
                self.spawn_particle(
                    [x + jitter_x, ground + 10.0 * scale, z + jitter_z],
                    [0.0, rise, 0.0],
                    life,
                    size,
                    [1.0, heat, 0.1],
                    6.0,
                    0.9,
                );
            }
            if self.rng.chance(0.18) {
                let jitter_x = self.rng.range(-5.0, 5.0) * scale;
                let jitter_z = self.rng.range(-5.0, 5.0) * scale;
                let rise = self.rng.range(10.0, 26.0);
                self.spawn_particle(
                    [x + jitter_x, ground + 18.0 * scale, z + jitter_z],
                    [0.0, rise, 0.0],
                    1.6,
                    6.0 * scale,
                    [0.24, 0.22, 0.21],
                    4.0,
                    0.7,
                );
            }
            if self.rng.chance(SPREAD_CHANCE * dt) {
                self.ignite_scenery([x, ground, z], SPREAD_RADIUS);
            }
            if self.scenery.burn_remaining[i] <= 0.0 {
                self.scenery.standing[i] = false;
                self.spawn_burst([x, ground + 6.0, z], 12, 12.0, 3.0, [0.3, 0.26, 0.24], 1.1);
            }
        }
    }

    // ---- sound -------------------------------------------------------------
    pub fn emit_sound(&mut self, cue: SoundCue, position: [f32; 3]) {
        let r = &mut self.render;
        if r.sound_cue_count >= MAX_SOUND_CUES {
            return;
        }
        let base = r.sound_cue_count * SOUND_CUE_STRIDE;
        r.sound_cues[base] = cue.as_f32();
        r.sound_cues[base + 1] = position[0];
        r.sound_cues[base + 2] = position[1];
        r.sound_cues[base + 3] = position[2];
        r.sound_cue_count += 1;
    }

    // ---- orbs --------------------------------------------------------------
    pub fn spawn_orb(&mut self, position: [f32; 3], amount: f32) {
        let phase = self.rng.range(0.0, core::f32::consts::TAU);
        let orbs = &mut self.orbs;
        for i in 0..MAX_ORBS {
            if orbs.alive[i] {
                continue;
            }
            orbs.alive[i] = true;
            orbs.pos_x[i] = position[0];
            orbs.pos_y[i] = position[1];
            orbs.pos_z[i] = position[2];
            orbs.amount[i] = amount;
            orbs.claimed_by[i] = Faction::WILD;
            orbs.bob_phase[i] = phase;
            orbs.carried_by[i] = None;
            return;
        }
    }

    /// Scatters `amount` of mana as orbs of a manageable size.
    pub fn scatter_mana(&mut self, position: [f32; 3], amount: f32) {
        const MAX_PER_ORB: f32 = 9.0;
        const DUST_THRESHOLD: f32 = 0.5;
        let mut remaining = amount;
        while remaining > DUST_THRESHOLD {
            let chunk = min(remaining, MAX_PER_ORB);
            let offset = [
                self.rng.range(-9.0, 9.0),
                self.rng.range(2.0, 9.0),
                self.rng.range(-9.0, 9.0),
            ];
            self.spawn_orb(
                [
                    position[0] + offset[0],
                    position[1] + offset[1],
                    position[2] + offset[2],
                ],
                chunk,
            );
            remaining -= chunk;
        }
    }

    // ---- creatures ---------------------------------------------------------
    pub fn spawn_creature(
        &mut self,
        kind: CreatureKind,
        x: f32,
        z: f32,
        faction: Faction,
    ) -> Option<usize> {
        let ground = self.height_at(x, z);
        let facing = self.rng.range(0.0, core::f32::consts::TAU);
        let timer = self.rng.range(0.0, 3.0);
        let phase = self.rng.range(0.0, core::f32::consts::TAU);
        let c = &mut self.creatures;
        for i in 0..MAX_CREATURES {
            if c.alive[i] {
                continue;
            }
            c.alive[i] = true;
            c.kind[i] = kind;
            c.faction[i] = faction;
            c.pos_x[i] = x;
            c.pos_z[i] = z;
            c.pos_y[i] = ground + kind.hover_height();
            c.vel_x[i] = 0.0;
            c.vel_y[i] = 0.0;
            c.vel_z[i] = 0.0;
            c.facing[i] = facing;
            c.timer[i] = timer;
            c.attack_cooldown[i] = 0.0;
            c.phase[i] = phase;
            c.health[i] = kind.max_health();
            c.max_health[i] = kind.max_health();
            return Some(i);
        }
        None
    }

    pub(crate) fn release_balloon_cargo(&mut self, balloon: usize) {
        for orb in 0..MAX_ORBS {
            if self.orbs.carried_by[orb] == Some(balloon) {
                self.orbs.carried_by[orb] = None;
            }
        }
    }

    pub fn kill_creature(&mut self, index: usize) {
        let kind = self.creatures.kind[index];
        let position = [
            self.creatures.pos_x[index],
            self.creatures.pos_y[index],
            self.creatures.pos_z[index],
        ];
        // Cargo ownership is indexed by creature slot. Clear every matching
        // reference before the slot can be reused, or a later balloon can
        // inherit an orb from the dead one.
        self.release_balloon_cargo(index);
        self.creatures.alive[index] = false;
        if self.creatures.faction[index].is_wild() {
            self.session.kills += 1.0;
        }
        self.spawn_burst(position, 16, 16.0, 3.0, [1.0, 0.55, 0.15], 0.7);
        let reward = kind.mana_reward();
        if reward > 0.0 {
            self.scatter_mana(position, reward);
            self.emit_sound(SoundCue::Release, position);
        }
    }

    // ---- projectiles -------------------------------------------------------
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_projectile(
        &mut self,
        kind: ProjectileKind,
        position: [f32; 3],
        velocity: [f32; 3],
        owner: Faction,
        damage: f32,
        blast_radius: f32,
        life: f32,
    ) {
        let p = &mut self.projectiles;
        for i in 0..MAX_PROJECTILES {
            if p.alive[i] {
                continue;
            }
            p.alive[i] = true;
            p.kind[i] = kind;
            p.owner[i] = owner;
            p.pos_x[i] = position[0];
            p.pos_y[i] = position[1];
            p.pos_z[i] = position[2];
            p.vel_x[i] = velocity[0];
            p.vel_y[i] = velocity[1];
            p.vel_z[i] = velocity[2];
            p.damage[i] = damage;
            p.blast_radius[i] = blast_radius;
            p.life[i] = life;
            return;
        }
    }

    // ---- damage ------------------------------------------------------------
    /// Applies splash damage to every creature, wizard and castle in range
    /// that does not belong to `attacker`.
    pub fn apply_blast(&mut self, centre: [f32; 3], radius: f32, damage: f32, attacker: Faction) {
        self.blast_creatures(centre, radius, damage, attacker);
        self.blast_wizards(centre, radius, damage, attacker);
        self.blast_castles(centre, radius, damage, attacker);
    }

    fn blast_creatures(&mut self, centre: [f32; 3], radius: f32, damage: f32, attacker: Faction) {
        let radius_sq = radius * radius;
        for i in 0..MAX_CREATURES {
            if !self.creatures.alive[i] || self.creatures.faction[i] == attacker {
                continue;
            }
            let offset = [
                self.creatures.pos_x[i] - centre[0],
                self.creatures.pos_y[i] - centre[1],
                self.creatures.pos_z[i] - centre[2],
            ];
            let dist_sq = length_sq3(offset[0], offset[1], offset[2]);
            if dist_sq > radius_sq {
                continue;
            }
            // full damage at the centre, 45% at the rim
            let falloff = 1.0 - sqrt(dist_sq) / radius;
            self.creatures.health[i] -= damage * (0.45 + 0.55 * falloff);
            self.spawn_impact(
                [
                    self.creatures.pos_x[i],
                    self.creatures.pos_y[i],
                    self.creatures.pos_z[i],
                ],
                [offset[0], offset[1], offset[2]],
                damage,
            );
            let inv_dist = 1.0 / (sqrt(dist_sq) + 0.001);
            self.creatures.vel_x[i] += offset[0] * inv_dist * damage * 0.5;
            self.creatures.vel_y[i] += offset[1] * inv_dist * damage * 0.25 + 4.0;
            self.creatures.vel_z[i] += offset[2] * inv_dist * damage * 0.5;
            if self.creatures.health[i] <= 0.0 {
                self.kill_creature(i);
            }
        }
    }

    fn blast_wizards(&mut self, centre: [f32; 3], radius: f32, damage: f32, attacker: Faction) {
        let radius_sq = radius * radius;
        for wizard in 0..=self.session.rival_count {
            if !self.wizards.alive[wizard] || Faction::of_wizard(wizard) == attacker {
                continue;
            }
            let offset = [
                self.wizards.pos_x[wizard] - centre[0],
                self.wizards.pos_y[wizard] - centre[1],
                self.wizards.pos_z[wizard] - centre[2],
            ];
            let dist_sq = length_sq3(offset[0], offset[1], offset[2]);
            if dist_sq > radius_sq {
                continue;
            }
            let falloff = 1.0 - sqrt(dist_sq) / radius;
            self.wound_wizard(wizard, damage * (0.4 + 0.6 * falloff), DamageSource::Blast);
        }
    }

    fn blast_castles(&mut self, centre: [f32; 3], radius: f32, damage: f32, attacker: Faction) {
        /// A castle is a big target; its hit sphere is padded well beyond the
        /// nominal blast radius.
        const CASTLE_HIT_PADDING: f32 = 26.0;
        const CASTLE_DAMAGE_SCALE: f32 = 0.8;
        /// A cracked keep spills three quarters of what it was holding.
        const SPILL_FRACTION: f32 = 0.75;
        let reach = radius + CASTLE_HIT_PADDING;
        for wizard in 0..=self.session.rival_count {
            if Faction::of_wizard(wizard) == attacker || self.castles.health[wizard] <= 0.0 {
                continue;
            }
            let offset = [
                self.castles.pos_x[wizard] - centre[0],
                self.castles.pos_y[wizard] + 20.0 - centre[1],
                self.castles.pos_z[wizard] - centre[2],
            ];
            if length_sq3(offset[0], offset[1], offset[2]) > reach * reach {
                continue;
            }
            self.castles.health[wizard] -= damage * CASTLE_DAMAGE_SCALE;
            if self.castles.health[wizard] > 0.0 {
                continue;
            }
            self.castles.health[wizard] = 0.0;
            let spill = self.castles.stored_mana[wizard] * SPILL_FRACTION;
            let at = [
                self.castles.pos_x[wizard],
                self.castles.pos_y[wizard] + 18.0,
                self.castles.pos_z[wizard],
            ];
            self.scatter_mana(at, spill);
            self.castles.stored_mana[wizard] *= 1.0 - SPILL_FRACTION;
            self.spawn_burst(
                [at[0], self.castles.pos_y[wizard] + 16.0, at[2]],
                70,
                34.0,
                6.0,
                [1.0, 0.7, 0.3],
                1.6,
            );
            self.session.screen_shake = 1.2;
            self.emit_sound(SoundCue::Collapse, at);
        }
    }

    /// Single-target damage. The two sources differ in their feedback: a blast
    /// jolts the camera harder and blows a slain rival apart, while a direct
    /// hit is quieter but plays the wince cue.
    pub fn wound_wizard(&mut self, wizard: usize, damage: f32, source: DamageSource) {
        const WARD_ABSORPTION: f32 = 0.25;
        /// Fraction of personal mana a slain rival drops.
        const DEATH_MANA_DROP: f32 = 0.65;
        const BLAST_SHAKE: f32 = 0.5;
        const DIRECT_SHAKE: f32 = 0.32;
        if !self.wizards.alive[wizard] {
            return;
        }
        let mut taken = damage;
        if self.wizards.ward_remaining[wizard] > 0.0 {
            taken *= WARD_ABSORPTION;
        }
        self.wizards.health[wizard] -= taken;
        self.wizards.time_since_hit[wizard] = 0.0;
        let position = [
            self.wizards.pos_x[wizard],
            self.wizards.pos_y[wizard],
            self.wizards.pos_z[wizard],
        ];
        if wizard == PLAYER {
            match source {
                DamageSource::Blast => self.add_shake(BLAST_SHAKE),
                DamageSource::Direct => {
                    self.add_shake(DIRECT_SHAKE);
                    self.emit_sound(SoundCue::Hurt, position);
                }
            }
        }
        if self.wizards.health[wizard] > 0.0 {
            return;
        }
        self.wizards.health[wizard] = 0.0;
        if wizard == PLAYER {
            self.session.outcome = Outcome::Died;
            return;
        }
        self.wizards.alive[wizard] = false;
        self.wizards.respawn_remaining[wizard] = RESPAWN_DELAY;
        let dropped = self.wizards.mana[wizard] * DEATH_MANA_DROP;
        self.scatter_mana(position, dropped);
        self.wizards.mana[wizard] *= 1.0 - DEATH_MANA_DROP;
        if source == DamageSource::Blast {
            self.spawn_burst(position, 40, 26.0, 4.0, [0.9, 0.3, 0.9], 1.1);
            self.emit_sound(SoundCue::BigBoom, position);
        }
    }
}

/// The player is always wizard zero.
pub const PLAYER: usize = 0;
