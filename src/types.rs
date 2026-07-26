//! The vocabulary of the simulation.
//!
//! Everything the renderer and `game.js` used to receive as a bare number —
//! creature type, spell slot, instance shape, minimap blip, audio event — is a
//! named type here. The numeric values are part of the ABI, so each conversion
//! is written out rather than derived, and changing one is a deliberate act.

/// Who something belongs to. `Wild` is nobody's; every wizard owns the faction
/// one above its own index, which is the encoding the render buffers use.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Faction(pub i32);

impl Faction {
    pub const WILD: Faction = Faction(0);

    #[inline]
    pub fn of_wizard(wizard: usize) -> Faction {
        Faction(wizard as i32 + 1)
    }
    /// The wizard that owns this faction, if any.
    #[inline]
    pub fn wizard(self) -> Option<usize> {
        if self.0 > 0 {
            Some((self.0 - 1) as usize)
        } else {
            None
        }
    }
    #[inline]
    pub fn is_wild(self) -> bool {
        self.0 == Faction::WILD.0
    }
    #[inline]
    pub fn is_player(self) -> bool {
        self.0 == 1
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CreatureKind {
    SandWorm,
    Wasp,
    Troll,
    Griffin,
    Nest,
    Wraith,
    Balloon,
    Dragon,
    /// Lives around a keep. Harmless, and flees anything hostile.
    Villager,
    /// Garrisons a keep and fights whatever comes for it.
    Soldier,
}

impl CreatureKind {
    /// Used by the spawners, which pick a kind from a random integer.
    pub fn from_index(index: i32) -> CreatureKind {
        match index {
            0 => CreatureKind::SandWorm,
            1 => CreatureKind::Wasp,
            2 => CreatureKind::Troll,
            3 => CreatureKind::Griffin,
            4 => CreatureKind::Nest,
            5 => CreatureKind::Wraith,
            6 => CreatureKind::Balloon,
            7 => CreatureKind::Dragon,
            8 => CreatureKind::Villager,
            9 => CreatureKind::Soldier,
            _ => CreatureKind::SandWorm,
        }
    }

    /// Villagers run rather than fight, and are never chosen as a target by
    /// their own side.
    pub fn is_civilian(self) -> bool {
        matches!(self, CreatureKind::Villager)
    }
    /// Anything that belongs to a keep rather than the wild.
    pub fn is_garrison(self) -> bool {
        matches!(self, CreatureKind::Villager | CreatureKind::Soldier)
    }

    /// Starting and maximum health. Big things are meant to be a fight, not a
    /// speed bump; swarm types stay thin so they still read as chaff.
    pub fn max_health(self) -> f32 {
        match self {
            CreatureKind::SandWorm => 165.0,
            CreatureKind::Wasp => 22.0,
            CreatureKind::Troll => 340.0,
            CreatureKind::Griffin => 200.0,
            CreatureKind::Nest => 430.0,
            CreatureKind::Wraith => 110.0,
            CreatureKind::Balloon => 30.0,
            CreatureKind::Dragon => 1500.0,
            CreatureKind::Villager => 40.0,
            CreatureKind::Soldier => 150.0,
        }
    }

    /// How far above the ground this kind sits when it spawns.
    pub fn hover_height(self) -> f32 {
        match self {
            CreatureKind::Wasp => 26.0,
            CreatureKind::Griffin => 34.0,
            CreatureKind::Wraith => 20.0,
            CreatureKind::Balloon => 46.0,
            CreatureKind::Dragon => 96.0,
            _ => 3.0,
        }
    }

    /// How high above the ground this kind prefers to cruise once airborne.
    /// Dragons ride far higher than anything else, which is what makes them
    /// read as circling rather than trudging.
    pub fn cruise_height(self) -> f32 {
        match self {
            CreatureKind::Dragon => 150.0,
            CreatureKind::Griffin => 62.0,
            CreatureKind::Wasp => 34.0,
            CreatureKind::Wraith => 26.0,
            _ => 0.0,
        }
    }

    /// Kinds that hold altitude instead of falling under gravity.
    pub fn flies(self) -> bool {
        matches!(
            self,
            CreatureKind::Wasp | CreatureKind::Griffin | CreatureKind::Wraith | CreatureKind::Dragon
        )
    }

    pub fn move_speed(self) -> f32 {
        match self {
            CreatureKind::SandWorm => 24.0,
            CreatureKind::Wasp => 74.0,
            CreatureKind::Troll => 20.0,
            CreatureKind::Griffin => 82.0,
            CreatureKind::Wraith => 58.0,
            CreatureKind::Dragon => 78.0,
            CreatureKind::Villager => 17.0,
            CreatureKind::Soldier => 30.0,
            _ => 26.0,
        }
    }

    /// Distance at which a wild creature will break off and come for a wizard.
    pub fn aggro_range(self) -> f32 {
        match self {
            CreatureKind::Wasp | CreatureKind::Griffin => 300.0,
            CreatureKind::Dragon => 380.0,
            _ => 200.0,
        }
    }

    /// Range at which this kind stops closing and starts shooting. `None`
    /// means it only ever fights in melee.
    pub fn firing_range(self) -> Option<f32> {
        match self {
            CreatureKind::SandWorm => Some(150.0),
            CreatureKind::Troll => Some(170.0),
            CreatureKind::Dragon => Some(210.0),
            _ => None,
        }
    }

    pub fn melee_damage(self) -> f32 {
        match self {
            CreatureKind::Wasp => 4.0,
            CreatureKind::Troll => 11.0,
            CreatureKind::Griffin => 7.0,
            CreatureKind::Wraith => 20.0,
            CreatureKind::Dragon => 17.0,
            CreatureKind::Soldier => 9.0,
            CreatureKind::Villager => 0.0,
            _ => 6.0,
        }
    }

    /// Mana dropped on death. Summons and balloons carry none.
    pub fn mana_reward(self) -> f32 {
        match self {
            CreatureKind::SandWorm => 15.0,
            CreatureKind::Wasp => 8.0,
            CreatureKind::Troll => 28.0,
            CreatureKind::Griffin => 20.0,
            CreatureKind::Nest => 60.0,
            CreatureKind::Dragon => 110.0,
            CreatureKind::Wraith
            | CreatureKind::Balloon
            | CreatureKind::Villager
            | CreatureKind::Soldier => 0.0,
        }
    }

    /// How fast the animation phase advances; wings beat far quicker than legs.
    pub fn phase_rate(self) -> f32 {
        match self {
            CreatureKind::Wasp => 26.0,
            _ => 4.0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Spell {
    Firebolt,
    ChainLightning,
    Crater,
    Volcano,
    Earthquake,
    Meteor,
    Ward,
    Mend,
    Haste,
    Claim,
    SummonWraith,
    Sunburst,
    Fortress,
}

pub const SPELL_COUNT: usize = 13;

impl Spell {
    pub const ALL: [Spell; SPELL_COUNT] = [
        Spell::Firebolt,
        Spell::ChainLightning,
        Spell::Crater,
        Spell::Volcano,
        Spell::Earthquake,
        Spell::Meteor,
        Spell::Ward,
        Spell::Mend,
        Spell::Haste,
        Spell::Claim,
        Spell::SummonWraith,
        Spell::Sunburst,
        Spell::Fortress,
    ];

    pub fn from_slot(slot: i32) -> Option<Spell> {
        if slot < 0 || slot as usize >= SPELL_COUNT {
            None
        } else {
            Some(Spell::ALL[slot as usize])
        }
    }
    #[inline]
    pub fn slot(self) -> usize {
        self as usize
    }

    /// Base mana price. Fortress is the exception: its real price rises per
    /// tier, so this entry is only a placeholder — see `fortress_price`.
    pub fn base_cost(self) -> f32 {
        match self {
            Spell::Firebolt => 4.0,
            Spell::ChainLightning => 9.0,
            Spell::Crater => 14.0,
            Spell::Volcano => 30.0,
            Spell::Earthquake => 24.0,
            Spell::Meteor => 46.0,
            Spell::Ward => 16.0,
            Spell::Mend => 18.0,
            Spell::Haste => 12.0,
            Spell::Claim => 4.0,
            Spell::SummonWraith => 22.0,
            Spell::Sunburst => 60.0,
            Spell::Fortress => 34.0,
        }
    }

    pub fn cooldown(self) -> f32 {
        match self {
            Spell::Firebolt => 0.28,
            Spell::ChainLightning => 1.10,
            Spell::Crater => 2.20,
            Spell::Volcano => 6.00,
            Spell::Earthquake => 5.00,
            Spell::Meteor => 8.00,
            Spell::Ward => 9.00,
            Spell::Mend => 7.00,
            Spell::Haste => 8.00,
            Spell::Claim => 0.55,
            Spell::SummonWraith => 6.00,
            Spell::Sunburst => 22.00,
            Spell::Fortress => 1.60,
        }
    }

    /// The realm at which this spell becomes available. Firebolt, Claim, Mend
    /// and Fortress are the core loop and are there from the first realm.
    pub fn unlocked_at_realm(self) -> i32 {
        match self {
            Spell::Firebolt | Spell::Claim | Spell::Mend | Spell::Fortress => 1,
            Spell::ChainLightning => 2,
            Spell::Crater | Spell::Ward => 3,
            Spell::Haste | Spell::Earthquake => 4,
            Spell::SummonWraith => 5,
            Spell::Volcano => 6,
            Spell::Meteor => 7,
            Spell::Sunburst => 8,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProjectileKind {
    Firebolt,
    CreatureBolt,
    Meteor,
    DragonFire,
}

impl ProjectileKind {
    /// Downward acceleration. Firebolts fly flat.
    pub fn gravity(self) -> f32 {
        match self {
            ProjectileKind::CreatureBolt | ProjectileKind::DragonFire => 42.0,
            ProjectileKind::Meteor => 34.0,
            ProjectileKind::Firebolt => 0.0,
        }
    }
    pub fn trail_colour(self) -> [f32; 3] {
        match self {
            ProjectileKind::Firebolt => [1.0, 0.55, 0.18],
            ProjectileKind::CreatureBolt => [0.55, 0.95, 0.35],
            ProjectileKind::Meteor => [1.0, 0.45, 0.12],
            ProjectileKind::DragonFire => [1.0, 0.32, 0.5],
        }
    }
    /// Colour and size of the drawn head, which differs slightly from the trail.
    pub fn head_style(self) -> ([f32; 3], f32) {
        match self {
            ProjectileKind::Firebolt => ([1.0, 0.6, 0.2], 3.0),
            ProjectileKind::CreatureBolt => ([0.5, 1.0, 0.4], 2.6),
            ProjectileKind::Meteor => ([1.0, 0.45, 0.12], 9.0),
            ProjectileKind::DragonFire => ([1.0, 0.3, 0.5], 3.4),
        }
    }
}

/// Which prototype mesh an instance is drawn with. The numbers are the index
/// the renderer partitions instances by, so they are part of the ABI.
#[derive(Clone, Copy)]
pub enum Shape {
    Cuboid,
    Sphere,
    Cone,
    /// A flat ground disc, drawn by its own darkening pass. Only the red
    /// channel is read, as the shadow's strength.
    Shadow,
    /// Round shaft along Y. Tower drums, trunks, limbs, poles, barrels.
    Cylinder,
    /// Box tapering to 45% at the top. Tower shafts, thighs, tree trunks —
    /// anything that should not read as a packing crate.
    Frustum,
    /// Triangular prism ridged along Z. Roofs, blades, buttresses, fins.
    Wedge,
    /// A palm leaf: rib, drooping curve and cut leaflets, all in the mesh.
    /// Seven of these make a crown that a stack of boxes never will.
    Frond,
    /// An irregular faceted lump. Flat-shaded, so it catches light as stone
    /// rather than as a billiard ball.
    Boulder,
    /// A whole crenellated parapet ring in one instance — merlons, embrasures
    /// and a walkway lip. Battlements built merlon-by-merlon cost forty
    /// instances a tower and still look like a row of boxes.
    Crenels,
}

impl Shape {
    pub const COUNT: usize = 10;
    #[inline]
    pub fn as_f32(self) -> f32 {
        match self {
            Shape::Cuboid => 0.0,
            Shape::Sphere => 1.0,
            Shape::Cone => 2.0,
            Shape::Shadow => 3.0,
            Shape::Cylinder => 4.0,
            Shape::Frustum => 5.0,
            Shape::Wedge => 6.0,
            Shape::Frond => 7.0,
            Shape::Boulder => 8.0,
            Shape::Crenels => 9.0,
        }
    }
}

/// Marker kinds on the minimap. `game.js` colours by these.
// Some variants are only produced in certain realms; all are part of the ABI.
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub enum MapBlip {
    Player,
    RivalWizard,
    WildCreature,
    AlliedCreature,
    ClaimedOrb,
    Nest,
    PlayerCastle,
    RivalCastle,
    RivalOrb,
    Balloon,
    UnclaimedOrb,
}

impl MapBlip {
    #[inline]
    pub fn as_f32(self) -> f32 {
        match self {
            MapBlip::Player => 0.0,
            MapBlip::RivalWizard => 1.0,
            MapBlip::WildCreature => 2.0,
            MapBlip::AlliedCreature => 3.0,
            MapBlip::ClaimedOrb => 4.0,
            MapBlip::Nest => 5.0,
            MapBlip::PlayerCastle => 6.0,
            MapBlip::RivalCastle => 7.0,
            MapBlip::RivalOrb => 8.0,
            MapBlip::Balloon => 9.0,
            MapBlip::UnclaimedOrb => 10.0,
        }
    }
}

/// Positioned audio cues. `game.js` maps each to a synthesised sound.
// The full cue table game.js knows about; not every one fires every realm.
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub enum SoundCue {
    Cast,
    Zap,
    Boom,
    Release,
    BigBoom,
    Collapse,
    Rumble,
    Quake,
    Incoming,
    Shimmer,
    Heal,
    Buzz,
    Whoosh,
    Wash,
    Pickup,
    Deposit,
    LevelUp,
    Hurt,
    MeteorImpact,
    Pop,
}

impl SoundCue {
    #[inline]
    pub fn as_f32(self) -> f32 {
        match self {
            SoundCue::Cast => 0.0,
            SoundCue::Zap => 1.0,
            SoundCue::Boom => 2.0,
            SoundCue::Release => 3.0,
            SoundCue::BigBoom => 4.0,
            SoundCue::Collapse => 5.0,
            SoundCue::Rumble => 6.0,
            SoundCue::Quake => 7.0,
            SoundCue::Incoming => 8.0,
            SoundCue::Shimmer => 9.0,
            SoundCue::Heal => 10.0,
            SoundCue::Buzz => 11.0,
            SoundCue::Whoosh => 12.0,
            SoundCue::Wash => 13.0,
            SoundCue::Pickup => 14.0,
            SoundCue::Deposit => 15.0,
            SoundCue::LevelUp => 16.0,
            SoundCue::Hurt => 17.0,
            SoundCue::MeteorImpact => 18.0,
            SoundCue::Pop => 19.0,
        }
    }
}

/// How a realm ended. Mirrored into the state block for `game.js`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    InProgress,
    Won,
    Died,
    RivalWon,
}

impl Outcome {
    #[inline]
    pub fn as_f32(self) -> f32 {
        match self {
            Outcome::InProgress => 0.0,
            Outcome::Won => 1.0,
            Outcome::Died => 2.0,
            Outcome::RivalWon => 3.0,
        }
    }
}

/// What a rival wizard is currently trying to do.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RivalPlan {
    GatherMana,
    ReturnHome,
    DuelPlayer,
    Retreat,
}

/// Shape of a terrain edit.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DeformKind {
    /// Scoop material out, as a crater does.
    Crater,
    /// Push a cone of material up, as a volcano does.
    Cone,
    /// A signed ripple that leaves the average height roughly alone.
    Ripple,
}

/// Where a wound came from. Blasts and direct hits give different feedback,
/// and a slain rival only bursts apart when something exploded on them.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DamageSource {
    Blast,
    Direct,
}
