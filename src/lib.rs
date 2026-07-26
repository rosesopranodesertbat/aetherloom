// ============================================================================
// AETHERLOOM — simulation core. Compiles to freestanding WebAssembly.
//
// Zero imports. All state lives in linear memory as flat f32/i32 arrays which
// JS maps directly as typed-array views: no marshalling, no copies, no GC.
//
// no_std on purpose: std would pull in a runtime this module has no use for,
// and the JS side instantiates with an empty import object, so anything that
// emitted an import would fail to instantiate at all.
// ============================================================================
#![no_std]
// No unsafe blocks anywhere in this crate. The only exemptions are the
// `#[no_mangle]` attributes below, which modern Rust classes as unsafe because
// an unmangled symbol can collide at link time — unavoidable for a module that
// has to expose a named ABI.
#![deny(unsafe_code)]

mod creatures;
mod math;
mod render;
mod session;
mod spells;
mod terrain;
mod types;
mod wizards;
mod world;

use core::panic::PanicInfo;
use types::*;
use world::*;

#[panic_handler]
fn on_panic(_: &PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

/// The whole simulation. `spin::Mutex` rather than `static mut` so the world
/// can be reached from a `static` without unsafe; wasm32-unknown-unknown is
/// single-threaded, so the lock is never contended and never re-entered — each
/// export takes it once and passes `&mut World` down.
static WORLD: spin::Mutex<World> = spin::Mutex::new(World::new());

/// Longest frame the simulation will integrate in one go. A tab that was
/// backgrounded for a minute must not teleport everything through terrain.
const MAX_TIMESTEP: f32 = 0.05;

// ============================================================================
// ABI — flat and C-like on purpose, so the core stays replaceable. `game.js`
// calls these by name and reads the buffers straight out of linear memory.
// ============================================================================

#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn init(seed: u32, realm: i32) {
    let world = &mut *WORLD.lock();
    world.begin_realm(seed, realm);
}

#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn step(dt: f32) {
    let world = &mut *WORLD.lock();
    world.advance(math::min(dt, MAX_TIMESTEP));
}

#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn setInput(
    thrust: f32,
    strafe: f32,
    lift: f32,
    yaw_delta: f32,
    pitch_delta: f32,
    _fire: i32,
    brake: i32,
) {
    let world = &mut *WORLD.lock();
    world.input = Input {
        thrust,
        strafe,
        lift,
        yaw_delta,
        pitch_delta,
        braking: brake != 0,
    };
}

#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn cast(slot: i32) {
    let world = &mut *WORLD.lock();
    if world.session.outcome != Outcome::InProgress {
        return;
    }
    if let Some(spell) = Spell::from_slot(slot) {
        world.cast(PLAYER, spell);
    }
}

#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn fireSelected() {
    let world = &mut *WORLD.lock();
    if world.session.outcome != Outcome::InProgress {
        return;
    }
    let spell = Spell::ALL[world.spells.selected];
    world.cast(PLAYER, spell);
}

#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn selectSpell(slot: i32) {
    let world = &mut *WORLD.lock();
    if let Some(spell) = Spell::from_slot(slot) {
        if world.spells.unlocked[spell.slot()] {
            world.spells.selected = spell.slot();
        }
    }
}

/// Steps to the next unlocked spell in `direction`, wrapping around.
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn cycleSpell(direction: i32) {
    let world = &mut *WORLD.lock();
    let count = SPELL_COUNT as i32;
    for _ in 0..count {
        let next = (world.spells.selected as i32 + direction + count) % count;
        world.spells.selected = next as usize;
        if world.spells.unlocked[world.spells.selected] {
            return;
        }
    }
}

// ---- pointers into linear memory -------------------------------------------
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn heightPtr() -> usize {
    WORLD.lock().terrain.height.as_ptr() as usize
}
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn instPtr() -> usize {
    WORLD.lock().render.instances.as_ptr() as usize
}
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn partPtr() -> usize {
    WORLD.lock().render.particles.as_ptr() as usize
}
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn mapPtr() -> usize {
    WORLD.lock().render.map_blips.as_ptr() as usize
}
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn evtPtr() -> usize {
    WORLD.lock().render.sound_cues.as_ptr() as usize
}
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn statePtr() -> usize {
    WORLD.lock().render.state.as_ptr() as usize
}

#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn instCount() -> i32 {
    WORLD.lock().render.instance_count as i32
}
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn carpetInstLo() -> i32 {
    WORLD.lock().render.carpet_first
}
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn carpetInstHi() -> i32 {
    WORLD.lock().render.carpet_last
}
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn partCount() -> i32 {
    WORLD.lock().render.particle_count as i32
}
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn mapCount() -> i32 {
    WORLD.lock().render.map_blip_count as i32
}
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn evtCount() -> i32 {
    WORLD.lock().render.sound_cue_count as i32
}

// ---- buffer layout ---------------------------------------------------------
// JS sizes its typed-array views from these rather than from copied numbers.
// A stale literal on the JS side silently truncates the frame — instances past
// the view are simulated, written, and never uploaded.
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn instCapacity() -> i32 {
    MAX_INSTANCES as i32
}
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn instStride() -> i32 {
    INSTANCE_STRIDE as i32
}
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn partCapacity() -> i32 {
    MAX_PARTICLES as i32
}
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn partStride() -> i32 {
    PARTICLE_STRIDE as i32
}
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn mapCapacity() -> i32 {
    MAX_MAP_BLIPS as i32
}
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn shapeCount() -> i32 {
    Shape::COUNT as i32
}

// ---- terrain upload band ---------------------------------------------------
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn dirtyLoRow() -> i32 {
    WORLD.lock().terrain.dirty_first_row
}
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn dirtyHiRow() -> i32 {
    WORLD.lock().terrain.dirty_last_row
}
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn clearDirty() {
    let world = &mut *WORLD.lock();
    world.terrain.dirty_first_row = GRID_WIDTH;
    world.terrain.dirty_last_row = -1;
}

// ---- world constants -------------------------------------------------------
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn terrainWidth() -> i32 {
    GRID_WIDTH
}
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn cellSize() -> f32 {
    CELL_SIZE
}
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn worldSize() -> f32 {
    WORLD_SIZE
}

// ---- renderer support ------------------------------------------------------
/// Builds the view-projection matrix and its inverse, and returns a pointer to
/// 32 floats: the matrix, then its inverse.
#[allow(unsafe_code)]
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn camera(
    fov_y: f32,
    aspect: f32,
    near: f32,
    far: f32,
    eye_x: f32,
    eye_y: f32,
    eye_z: f32,
    centre_x: f32,
    centre_y: f32,
    centre_z: f32,
    up_x: f32,
    up_y: f32,
    up_z: f32,
) -> usize {
    let world = &mut *WORLD.lock();
    world.update_camera(
        fov_y,
        aspect,
        near,
        far,
        [eye_x, eye_y, eye_z],
        [centre_x, centre_y, centre_z],
        [up_x, up_y, up_z],
    );
    world.camera_matrices.as_ptr() as usize
}

#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn buildMeshes() {
    WORLD.lock().meshes.build();
}
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn meshVertPtr(shape: i32) -> usize {
    WORLD.lock().meshes.vertices[shape as usize].as_ptr() as usize
}
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn meshVertFloats(shape: i32) -> i32 {
    WORLD.lock().meshes.vertex_floats[shape as usize] as i32
}
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn meshIdxPtr(shape: i32) -> usize {
    WORLD.lock().meshes.indices[shape as usize].as_ptr() as usize
}
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn meshIdxCount(shape: i32) -> i32 {
    WORLD.lock().meshes.index_counts[shape as usize] as i32
}

/// Rasterises the heightmap into an RGBA image for the minimap and returns a
/// pointer JS wraps in a `Uint8ClampedArray` for `putImageData`.
#[allow(unsafe_code)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn minimapRaster(side: i32) -> usize {
    let world = &mut *WORLD.lock();
    world.rasterise_minimap(side.max(1) as usize);
    world.minimap.pixels.as_ptr() as usize
}
