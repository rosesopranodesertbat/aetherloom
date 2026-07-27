//! Deterministic, isolated model scenes for visual regression and manual QA.
//!
//! These scenes write only the ordinary render buffers plus `preview_focus`.
//! They do not require a realm to be initialised and never advance gameplay.

use super::*;

pub(crate) const PREVIEW_SCENE_COUNT: i32 = 30;
pub(crate) const PREVIEW_VARIANT_COUNT: i32 = 8;

const PREVIEW_Y: f32 = 512.0;

impl World {
    /// Populate the existing instance buffer with one stable, isolated model.
    ///
    /// Scene IDs are ABI: keep the comments and the web catalogue in lockstep.
    pub fn preview_scene(&mut self, scene: i32, variant: i32) -> i32 {
        self.render.instance_count = 0;
        self.render.particle_count = 0;
        self.render.map_blip_count = 0;
        self.render.carpet_first = -1;
        self.render.carpet_last = -1;
        self.preview_focus = [0.0; 4];

        if !(0..PREVIEW_SCENE_COUNT).contains(&scene)
            || !(0..PREVIEW_VARIANT_COUNT).contains(&variant)
        {
            return 0;
        }

        let origin = [WORLD_SIZE * 0.5, PREVIEW_Y, WORLD_SIZE * 0.5];
        match scene {
            // 0: Sand Worm
            0 => self.draw_preview_creature(CreatureKind::SandWorm, variant, origin),
            // 1: Wasp
            1 => self.draw_preview_creature(CreatureKind::Wasp, variant, origin),
            // 2: Troll
            2 => self.draw_preview_creature(CreatureKind::Troll, variant, origin),
            // 3: Griffin
            3 => self.draw_preview_creature(CreatureKind::Griffin, variant, origin),
            // 4: Nest
            4 => self.draw_preview_creature(CreatureKind::Nest, variant, origin),
            // 5: Wraith
            5 => self.draw_preview_creature(CreatureKind::Wraith, variant, origin),
            // 6: Balloon
            6 => self.draw_preview_creature(CreatureKind::Balloon, variant, origin),
            // 7: Dragon
            7 => self.draw_preview_creature(CreatureKind::Dragon, variant, origin),
            // 8: Villager
            8 => self.draw_preview_creature(CreatureKind::Villager, variant, origin),
            // 9: Soldier
            9 => self.draw_preview_creature(CreatureKind::Soldier, variant, origin),
            // 10..15: Castle tiers 1..6
            10..=15 => self.draw_preview_castle(scene - 9, variant, origin, false),
            // 16: Castle ruin
            16 => self.draw_preview_castle(6, variant, origin, true),
            // 17: Palm
            17 => self.draw_preview_scenery(0, variant, origin),
            // 18: Boulder
            18 => self.draw_preview_scenery(1, variant, origin),
            // 19: Burnt stump
            19 => self.draw_preview_scenery(2, variant, origin),
            // 20: Player carpet
            20 => self.draw_preview_effect(0, variant, origin),
            // 21: Rival carpet
            21 => self.draw_preview_effect(1, variant, origin),
            // 22: Unclaimed mana orb
            22 => self.draw_preview_effect(2, variant, origin),
            // 23: Player mana orb
            23 => self.draw_preview_effect(3, variant, origin),
            // 24: Rival mana orb
            24 => self.draw_preview_effect(4, variant, origin),
            // 25: Firebolt
            25 => self.draw_preview_effect(5, variant, origin),
            // 26: Meteor
            26 => self.draw_preview_effect(6, variant, origin),
            // 27: Creature bolt
            27 => self.draw_preview_effect(7, variant, origin),
            // 28: Dragon fire
            28 => self.draw_preview_effect(8, variant, origin),
            // 29: Fireball impact against an unflashed enemy
            29 => self.draw_preview_fireball_impact(variant, origin),
            _ => {}
        }

        self.derive_preview_focus();
        self.render.instance_count as i32
    }

    /// Derive one camera target from the geometry and particles actually
    /// emitted.
    ///
    /// Prototype meshes live inside the -1..1 cube and the shader applies
    /// half the instance size. Half the size-vector diagonal is therefore a
    /// rotation-invariant sphere around an instance. The aggregate centre is
    /// taken from the extrema of those spheres, then the final radius encloses
    /// every instance sphere with a small camera-framing margin.
    fn derive_preview_focus(&mut self) {
        if self.render.instance_count == 0 && self.render.particle_count == 0 {
            return;
        }

        let mut lower = [f32::MAX; 3];
        let mut upper = [f32::MIN; 3];
        for instance in 0..self.render.instance_count {
            let base = instance * INSTANCE_STRIDE;
            let size_x = self.render.instances[base + 3];
            let size_y = self.render.instances[base + 4];
            let size_z = self.render.instances[base + 5];
            let extent = 0.5 * length3(size_x, size_y, size_z);
            for axis in 0..3 {
                let position = self.render.instances[base + axis];
                lower[axis] = min(lower[axis], position - extent);
                upper[axis] = max(upper[axis], position + extent);
            }
        }
        // Particle quads face the camera. Their corner is sqrt(2) * size from
        // the centre, which is a conservative orientation-independent bound.
        for particle in 0..self.render.particle_count {
            let base = particle * PARTICLE_STRIDE;
            let extent = self.render.particles[base + 3] * 1.414_214;
            for axis in 0..3 {
                let position = self.render.particles[base + axis];
                lower[axis] = min(lower[axis], position - extent);
                upper[axis] = max(upper[axis], position + extent);
            }
        }

        let centre = [
            (lower[0] + upper[0]) * 0.5,
            (lower[1] + upper[1]) * 0.5,
            (lower[2] + upper[2]) * 0.5,
        ];
        let mut radius = 0.0f32;
        for instance in 0..self.render.instance_count {
            let base = instance * INSTANCE_STRIDE;
            let offset_x = self.render.instances[base] - centre[0];
            let offset_y = self.render.instances[base + 1] - centre[1];
            let offset_z = self.render.instances[base + 2] - centre[2];
            let size_x = self.render.instances[base + 3];
            let size_y = self.render.instances[base + 4];
            let size_z = self.render.instances[base + 5];
            let extent = 0.5 * length3(size_x, size_y, size_z);
            radius = max(
                radius,
                length3(offset_x, offset_y, offset_z) + extent,
            );
        }
        for particle in 0..self.render.particle_count {
            let base = particle * PARTICLE_STRIDE;
            let offset_x = self.render.particles[base] - centre[0];
            let offset_y = self.render.particles[base + 1] - centre[1];
            let offset_z = self.render.particles[base + 2] - centre[2];
            let extent = self.render.particles[base + 3] * 1.414_214;
            radius = max(
                radius,
                length3(offset_x, offset_y, offset_z) + extent,
            );
        }

        const CAMERA_MARGIN: f32 = 1.05;
        self.preview_focus = [centre[0], centre[1], centre[2], radius * CAMERA_MARGIN];
    }

    fn draw_preview_creature(
        &mut self,
        kind: CreatureKind,
        variant: i32,
        origin: [f32; 3],
    ) {
        let facing = variant as f32 * core::f32::consts::TAU / PREVIEW_VARIANT_COUNT as f32;
        let faction = if matches!(
            kind,
            CreatureKind::Wraith
                | CreatureKind::Balloon
                | CreatureKind::Villager
                | CreatureKind::Soldier
        ) {
            Faction::of_wizard((variant as usize) & 1)
        } else {
            Faction::WILD
        };
        let body = CreatureBody {
            x: origin[0],
            y: origin[1],
            z: origin[2],
            facing,
            facing_sin: sin(facing),
            facing_cos: cos(facing),
            phase: variant as f32 * 0.71 + 0.35,
            faction,
            detail: BodyDetail::Full,
            slot: 10_000 + variant as usize,
        };
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
    }

    /// Show the real contact-particle recipe around a normal, unflashed enemy.
    ///
    /// This writes the render particle buffer directly, leaving the live
    /// particle ring and the realm RNG untouched.
    fn draw_preview_fireball_impact(&mut self, variant: i32, origin: [f32; 3]) {
        self.draw_preview_creature(CreatureKind::Troll, variant, origin);

        let angle =
            variant as f32 * core::f32::consts::TAU / PREVIEW_VARIANT_COUNT as f32;
        let outward = [sin(angle), 0.0, cos(angle)];
        let impact = [
            origin[0] + outward[0] * 7.0,
            origin[1] + 13.0,
            origin[2] + outward[2] * 7.0,
        ];
        let incoming = [-outward[0] * 70.0, -5.0, -outward[2] * 70.0];
        let kind = if variant & 1 == 0 {
            ProjectileKind::Firebolt
        } else {
            ProjectileKind::DragonFire
        };
        let mut rng = Rng::new();
        rng.reseed(0x4649_5245 ^ (variant as u32).wrapping_mul(0x9e37_79b9));
        let mut particles = [ParticleSpec::ZERO; MAX_FIREBALL_IMPACT_PARTICLES];
        let count =
            build_fireball_impact_particles(&mut rng, impact, incoming, kind, &mut particles);
        let age = 0.095 + (variant % 3) as f32 * 0.012;
        for particle in &particles[..count] {
            self.push_preview_particle(*particle, age);
        }
    }

    /// Advance one particle exactly like the fixed-step gameplay integrator,
    /// then pack it into the ordinary eight-float render ABI.
    fn push_preview_particle(&mut self, mut particle: ParticleSpec, age: f32) {
        let initial_life = particle.life;
        let mut elapsed = 0.0;
        for _ in 0..8 {
            let dt = min(0.05, age - elapsed);
            if dt <= 0.0 {
                break;
            }
            particle.life -= dt;
            if particle.life <= 0.0 {
                return;
            }
            particle.velocity[1] += particle.gravity * dt;
            let retained = max(1.0 - particle.drag * dt, 0.0);
            particle.velocity[0] *= retained;
            particle.velocity[1] *= retained;
            particle.velocity[2] *= retained;
            particle.position[0] += particle.velocity[0] * dt;
            particle.position[1] += particle.velocity[1] * dt;
            particle.position[2] += particle.velocity[2] * dt;
            elapsed += dt;
        }

        if self.render.particle_count >= MAX_PARTICLES {
            return;
        }
        let remaining = particle.life / initial_life;
        let base = self.render.particle_count * PARTICLE_STRIDE;
        self.render.particles[base] = particle.position[0];
        self.render.particles[base + 1] = particle.position[1];
        self.render.particles[base + 2] = particle.position[2];
        self.render.particles[base + 3] =
            particle.size * (0.35 + remaining * 0.9);
        self.render.particles[base + 4] = particle.colour[0];
        self.render.particles[base + 5] = particle.colour[1];
        self.render.particles[base + 6] = particle.colour[2];
        self.render.particles[base + 7] = remaining * remaining;
        self.render.particle_count += 1;
    }
}
