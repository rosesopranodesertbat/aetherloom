// ============================================================================
// AETHERLOOM - simulation core. Compiles to freestanding WebAssembly.
// Zero imports. All state lives in linear memory as flat f32/i32 statics which
// JS maps directly as typed-array views (no marshalling, no copies, no GC).
//
// no_std on purpose: std would pull in a runtime this module has no use for,
// and the JS side instantiates with an empty import object, so anything that
// emitted an import would fail to instantiate at all.
// ============================================================================
#![no_std]
// The ABI is camelCase because game.js calls these names directly, and the
// state arrays keep the names they had in the original core so the two can be
// read side by side.
#![allow(non_snake_case)]
#![allow(static_mut_refs)]

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

// ---- math ------------------------------------------------------------------
// core has arithmetic but no transcendentals; libm supplies them without
// touching the host.
#[inline(always)]
fn sin(x: f32) -> f32 {
    libm::sinf(x)
}
#[inline(always)]
fn cos(x: f32) -> f32 {
    libm::cosf(x)
}
#[inline(always)]
fn sqrt(x: f32) -> f32 {
    libm::sqrtf(x)
}
#[inline(always)]
fn floor(x: f32) -> f32 {
    libm::floorf(x)
}
#[inline(always)]
fn ceil(x: f32) -> f32 {
    libm::ceilf(x)
}
#[inline(always)]
fn atan2(y: f32, x: f32) -> f32 {
    libm::atan2f(y, x)
}
#[inline(always)]
fn fmin(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}
#[inline(always)]
fn fmax(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}

// ---- world constants -------------------------------------------------------
const TW: i32 = 320; // terrain grid width (320*4B = 1280B rows, GPU-aligned)
const TWM: i32 = TW - 1;
const CELL: f32 = 8.0; // world units per cell
const WORLD: f32 = TW as f32 * CELL; // 2560 units
const SEA: f32 = 0.0;

const MAXE: usize = 460; // creatures
const MAXP: usize = 128; // projectiles
const MAXO: usize = 768; // mana orbs
const MAXPT: usize = 4096; // particles
const MAXI: usize = 12288; // render instances
const MAXDEC: usize = 5200; // scenery
const NWIZ: usize = 3; // 1 player + up to 2 rivals
const NSPELL: usize = 13;
const MAXMAP: usize = 2048;
const TWU: usize = TW as usize;

// ---- terrain ---------------------------------------------------------------
static mut HEIGHT: [f32; TWU * TWU] = [0.0; TWU * TWU];
static mut DIRTY_LO: i32 = 0;
static mut DIRTY_HI: i32 = -1; // dirty row band for GPU upload

// ---- creatures (structure of arrays) --------------------------------------
static mut EX: [f32; MAXE] = [0.0; MAXE];
static mut EY: [f32; MAXE] = [0.0; MAXE];
static mut EZ: [f32; MAXE] = [0.0; MAXE];
static mut EVX: [f32; MAXE] = [0.0; MAXE];
static mut EVY: [f32; MAXE] = [0.0; MAXE];
static mut EVZ: [f32; MAXE] = [0.0; MAXE];
static mut EHP: [f32; MAXE] = [0.0; MAXE];
static mut EHPM: [f32; MAXE] = [0.0; MAXE];
static mut ETYPE: [i32; MAXE] = [0; MAXE];
static mut ESTATE: [i32; MAXE] = [0; MAXE];
static mut EOWN: [i32; MAXE] = [0; MAXE];
static mut ETIM: [f32; MAXE] = [0.0; MAXE];
static mut ECD: [f32; MAXE] = [0.0; MAXE];
static mut EYAW: [f32; MAXE] = [0.0; MAXE];
static mut EALIVE: [i32; MAXE] = [0; MAXE];
static mut EPHASE: [f32; MAXE] = [0.0; MAXE];
static mut EHURT: [f32; MAXE] = [0.0; MAXE];

// ---- projectiles -----------------------------------------------------------
static mut PX: [f32; MAXP] = [0.0; MAXP];
static mut PY: [f32; MAXP] = [0.0; MAXP];
static mut PZ: [f32; MAXP] = [0.0; MAXP];
static mut PVX: [f32; MAXP] = [0.0; MAXP];
static mut PVY: [f32; MAXP] = [0.0; MAXP];
static mut PVZ: [f32; MAXP] = [0.0; MAXP];
static mut PLIFE: [f32; MAXP] = [0.0; MAXP];
static mut PDMG: [f32; MAXP] = [0.0; MAXP];
static mut PRAD: [f32; MAXP] = [0.0; MAXP];
static mut PTYPE: [i32; MAXP] = [0; MAXP];
static mut POWN: [i32; MAXP] = [0; MAXP];
static mut PALIVE: [i32; MAXP] = [0; MAXP];

// ---- mana orbs -------------------------------------------------------------
static mut OX: [f32; MAXO] = [0.0; MAXO];
static mut OY: [f32; MAXO] = [0.0; MAXO];
static mut OZ: [f32; MAXO] = [0.0; MAXO];
static mut OAMT: [f32; MAXO] = [0.0; MAXO];
static mut OALIVE: [i32; MAXO] = [0; MAXO];
static mut OPHASE: [f32; MAXO] = [0.0; MAXO];
static mut OHELD: [i32; MAXO] = [0; MAXO]; // -1 free, else balloon entity index
// 0 = unclaimed (gold), 1 = claimed by the player, 2 = claimed by a rival.
// Only orbs matching a balloon's faction get ferried home.
static mut OOWN: [i32; MAXO] = [0; MAXO];
// personal mana pool bonus earned by possessing orbs
static mut WCAP: [f32; NWIZ] = [0.0; NWIZ];

// ---- particles (ring buffer) -----------------------------------------------
static mut QX: [f32; MAXPT] = [0.0; MAXPT];
static mut QY: [f32; MAXPT] = [0.0; MAXPT];
static mut QZ: [f32; MAXPT] = [0.0; MAXPT];
static mut QVX: [f32; MAXPT] = [0.0; MAXPT];
static mut QVY: [f32; MAXPT] = [0.0; MAXPT];
static mut QVZ: [f32; MAXPT] = [0.0; MAXPT];
static mut QL: [f32; MAXPT] = [0.0; MAXPT];
static mut QLM: [f32; MAXPT] = [0.0; MAXPT];
static mut QS: [f32; MAXPT] = [0.0; MAXPT];
static mut QR: [f32; MAXPT] = [0.0; MAXPT];
static mut QG: [f32; MAXPT] = [0.0; MAXPT];
static mut QB: [f32; MAXPT] = [0.0; MAXPT];
static mut QDRAG: [f32; MAXPT] = [0.0; MAXPT];
static mut QGRAV: [f32; MAXPT] = [0.0; MAXPT];
static mut QHEAD: usize = 0;

// ---- wizards (0 = player) --------------------------------------------------
static mut WX: [f32; NWIZ] = [0.0; NWIZ];
static mut WY: [f32; NWIZ] = [0.0; NWIZ];
static mut WZ: [f32; NWIZ] = [0.0; NWIZ];
static mut WVX: [f32; NWIZ] = [0.0; NWIZ];
static mut WVY: [f32; NWIZ] = [0.0; NWIZ];
static mut WVZ: [f32; NWIZ] = [0.0; NWIZ];
static mut WYAW: [f32; NWIZ] = [0.0; NWIZ];
static mut WPIT: [f32; NWIZ] = [0.0; NWIZ];
static mut WROL: [f32; NWIZ] = [0.0; NWIZ];
static mut WHP: [f32; NWIZ] = [0.0; NWIZ];
static mut WCAR: [f32; NWIZ] = [0.0; NWIZ];
static mut WSTORE: [f32; NWIZ] = [0.0; NWIZ];
static mut WMANA: [f32; NWIZ] = [0.0; NWIZ];
static mut WCD: [f32; NWIZ] = [0.0; NWIZ];
static mut WALIVE: [i32; NWIZ] = [0; NWIZ];
static mut WRESP: [f32; NWIZ] = [0.0; NWIZ];
static mut WSHIELD: [f32; NWIZ] = [0.0; NWIZ];
static mut WHASTE: [f32; NWIZ] = [0.0; NWIZ];
static mut WAI: [i32; NWIZ] = [0; NWIZ];
static mut WAITIM: [f32; NWIZ] = [0.0; NWIZ];
static mut WNOHIT: [f32; NWIZ] = [0.0; NWIZ];

// ---- castles ---------------------------------------------------------------
static mut CX: [f32; NWIZ] = [0.0; NWIZ];
static mut CZ: [f32; NWIZ] = [0.0; NWIZ];
static mut CY: [f32; NWIZ] = [0.0; NWIZ];
static mut CHP: [f32; NWIZ] = [0.0; NWIZ];
static mut CHPM: [f32; NWIZ] = [0.0; NWIZ];
static mut CLEV: [i32; NWIZ] = [0; NWIZ];
static mut CBT: [f32; NWIZ] = [0.0; NWIZ]; // balloon spawn timer

// ---- scenery ---------------------------------------------------------------
static mut DX: [f32; MAXDEC] = [0.0; MAXDEC];
static mut DZ: [f32; MAXDEC] = [0.0; MAXDEC];
static mut DKIND: [i32; MAXDEC] = [0; MAXDEC];
static mut DSC: [f32; MAXDEC] = [0.0; MAXDEC];
static mut DROT: [f32; MAXDEC] = [0.0; MAXDEC];
static mut DEC_COUNT: usize = 0;

// ---- render buffers --------------------------------------------------------
// instance stride 12: px,py,pz, sx,sy,sz, r,g,b, yaw, glow, shape
static mut INST: [f32; MAXI * 12] = [0.0; MAXI * 12];
static mut INST_N: usize = 0;
// index range of the player's own carpet within INST, for first-person culling
static mut CARPET_LO: i32 = 0;
static mut CARPET_HI: i32 = 0;

static mut PART: [f32; MAXPT * 8] = [0.0; MAXPT * 8];
static mut PART_N: usize = 0;

static mut MAP: [f32; MAXMAP * 4] = [0.0; MAXMAP * 4];
static mut MAP_N: usize = 0;

static mut EVT: [f32; 128 * 4] = [0.0; 128 * 4];
static mut EVT_N: usize = 0;

static mut ST: [f32; 128] = [0.0; 128];
static mut CD: [f32; NSPELL] = [0.0; NSPELL]; // cooldown remaining
static mut UNL: [i32; NSPELL] = [0; NSPELL]; // unlocked

// ---- input / game state ----------------------------------------------------
static mut I_FWD: f32 = 0.0;
static mut I_STR: f32 = 0.0;
static mut I_UP: f32 = 0.0;
static mut I_YAW: f32 = 0.0;
static mut I_PIT: f32 = 0.0;
static mut I_FIRE: i32 = 0;
static mut I_BRAKE: i32 = 0;
static mut SEL: i32 = 0;
static mut LEVEL: i32 = 1;
static mut STATUS: i32 = 0;
static mut TOTAL_MANA: f32 = 1.0;
static mut TARGET_FRAC: f32 = 0.5;
static mut GTIME: f32 = 0.0;
static mut SHAKE: f32 = 0.0;
static mut KILLS: f32 = 0.0;
static mut N_RIVAL: usize = 1;
static mut SPAWN_TIMER: f32 = 0.0;

// ---- rng -------------------------------------------------------------------
static mut RS: u32 = 0x9e37_79b9;

fn srand(s: u32) {
    unsafe {
        RS = s | 1;
    }
}
fn rnd() -> f32 {
    unsafe {
        RS ^= RS << 13;
        RS ^= RS >> 17;
        RS ^= RS << 5;
        (RS & 0xff_ffff) as f32 / 16_777_216.0
    }
}
fn rr(a: f32, b: f32) -> f32 {
    a + (b - a) * rnd()
}
fn ri(n: i32) -> i32 {
    (rnd() * n as f32) as i32 % n
}

// ---- value noise + fbm -----------------------------------------------------
// Every multiply here has to wrap, exactly as it did in the original core, or
// the terrain comes out different for the same seed.
#[inline]
fn hash2(x: i32, y: i32) -> f32 {
    unsafe {
        let mut h: u32 = x
            .wrapping_mul(374_761_393)
            .wrapping_add(y.wrapping_mul(668_265_263))
            .wrapping_add(RS as i32) as u32;
        h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
        h ^= h >> 16;
        (h & 0xffff) as f32 / 32768.0 - 1.0
    }
}
fn vnoise(x: f32, y: f32) -> f32 {
    let xi = floor(x) as i32;
    let yi = floor(y) as i32;
    let fx = x - xi as f32;
    let fy = y - yi as f32;
    let ux = fx * fx * (3.0 - 2.0 * fx);
    let uy = fy * fy * (3.0 - 2.0 * fy);
    let a = hash2(xi, yi);
    let b = hash2(xi + 1, yi);
    let c = hash2(xi, yi + 1);
    let d = hash2(xi + 1, yi + 1);
    (a + (b - a) * ux) + ((c + (d - c) * ux) - (a + (b - a) * ux)) * uy
}
fn fbm(x: f32, y: f32, oct: i32) -> f32 {
    let mut s: f32 = 0.0;
    let mut a: f32 = 0.5;
    let mut f: f32 = 1.0;
    for _ in 0..oct {
        s += a * vnoise(x * f, y * f);
        f *= 2.03;
        a *= 0.5;
    }
    s
}

// ---- height access ---------------------------------------------------------
#[inline]
fn h_get(i: i32, j: i32) -> f32 {
    unsafe {
        let i = if i < 0 {
            0
        } else if i > TWM {
            TWM
        } else {
            i
        };
        let j = if j < 0 {
            0
        } else if j > TWM {
            TWM
        } else {
            j
        };
        HEIGHT[(j * TW + i) as usize]
    }
}
#[inline]
fn h_set(i: i32, j: i32, v: f32) {
    unsafe {
        if i < 0 || i > TWM || j < 0 || j > TWM {
            return;
        }
        HEIGHT[(j * TW + i) as usize] = v;
        if j < DIRTY_LO {
            DIRTY_LO = j;
        }
        if j > DIRTY_HI {
            DIRTY_HI = j;
        }
    }
}
// bilinear world-space height
fn height_at(x: f32, z: f32) -> f32 {
    let gx = x / CELL;
    let gz = z / CELL;
    let i = floor(gx) as i32;
    let j = floor(gz) as i32;
    let fx = gx - i as f32;
    let fz = gz - j as f32;
    let h00 = h_get(i, j);
    let h10 = h_get(i + 1, j);
    let h01 = h_get(i, j + 1);
    let h11 = h_get(i + 1, j + 1);
    let a = h00 + (h10 - h00) * fx;
    let b = h01 + (h11 - h01) * fx;
    a + (b - a) * fz
}
#[inline]
fn clamp_world(v: f32) -> f32 {
    if v < 8.0 {
        8.0
    } else if v > WORLD - 8.0 {
        WORLD - 8.0
    } else {
        v
    }
}

// ---- terrain deformation ---------------------------------------------------
// mode 0 = crater (subtract), 1 = raise cone, 2 = smooth ripple
fn deform(wx0: f32, wz0: f32, radius: f32, amount: f32, mode: i32) {
    let gi = wx0 / CELL;
    let gj = wz0 / CELL;
    let gr = radius / CELL;
    let i0 = floor(gi - gr) as i32 - 1;
    let i1 = ceil(gi + gr) as i32 + 1;
    let j0 = floor(gj - gr) as i32 - 1;
    let j1 = ceil(gj + gr) as i32 + 1;
    for j in j0..=j1 {
        for i in i0..=i1 {
            let dx = i as f32 - gi;
            let dz = j as f32 - gj;
            let d = sqrt(dx * dx + dz * dz) / gr;
            if d > 1.0 {
                continue;
            }
            let w: f32;
            if mode == 2 {
                w = cos(d * 3.14159) * (1.0 - d);
            } else {
                let t = 1.0 - d * d;
                w = t * t;
            }
            let cur = h_get(i, j);
            if mode == 1 {
                let cone = amount * (1.0 - d);
                h_set(i, j, cur + cone * 0.55 + amount * 0.45 * w);
            } else {
                h_set(i, j, cur - amount * w);
            }
        }
    }
}

// ---- particles -------------------------------------------------------------
fn spawn_part(
    x: f32,
    y: f32,
    z: f32,
    vx: f32,
    vy: f32,
    vz: f32,
    life: f32,
    size: f32,
    r: f32,
    g: f32,
    b: f32,
    grav: f32,
    drag: f32,
) {
    unsafe {
        let i = QHEAD;
        QHEAD = (QHEAD + 1) % MAXPT;
        QX[i] = x;
        QY[i] = y;
        QZ[i] = z;
        QVX[i] = vx;
        QVY[i] = vy;
        QVZ[i] = vz;
        QL[i] = life;
        QLM[i] = life;
        QS[i] = size;
        QR[i] = r;
        QG[i] = g;
        QB[i] = b;
        QGRAV[i] = grav;
        QDRAG[i] = drag;
    }
}
fn burst(x: f32, y: f32, z: f32, n: i32, spd: f32, size: f32, r: f32, g: f32, b: f32, life: f32) {
    for _ in 0..n {
        let a = rr(0.0, 6.2832);
        let e = rr(-1.0, 1.0);
        let s = spd * rr(0.3, 1.0);
        let ch = sqrt(1.0 - e * e);
        spawn_part(
            x,
            y,
            z,
            cos(a) * ch * s,
            e * s + spd * 0.25,
            sin(a) * ch * s,
            life * rr(0.6, 1.2),
            size * rr(0.6, 1.4),
            r,
            g,
            b,
            -14.0,
            1.6,
        );
    }
}
fn push_event(kind: f32, x: f32, y: f32, z: f32) {
    unsafe {
        if EVT_N >= 128 {
            return;
        }
        let o = EVT_N * 4;
        EVT[o] = kind;
        EVT[o + 1] = x;
        EVT[o + 2] = y;
        EVT[o + 3] = z;
        EVT_N += 1;
    }
}

// ---- spawning --------------------------------------------------------------
fn add_creature(t: i32, x: f32, z: f32, own: i32) -> i32 {
    unsafe {
        for i in 0..MAXE {
            if EALIVE[i] != 0 {
                continue;
            }
            EALIVE[i] = 1;
            ETYPE[i] = t;
            EOWN[i] = own;
            ESTATE[i] = 0;
            EX[i] = x;
            EZ[i] = z;
            EY[i] = height_at(x, z) + creature_float(t);
            EVX[i] = 0.0;
            EVY[i] = 0.0;
            EVZ[i] = 0.0;
            EYAW[i] = rr(0.0, 6.2832);
            ETIM[i] = rr(0.0, 3.0);
            ECD[i] = 0.0;
            EPHASE[i] = rr(0.0, 6.2832);
            EHURT[i] = 0.0;
            // Big things are meant to be a fight, not a speed bump. Swarm types
            // stay thin so they still read as chaff.
            let hp: f32 = match t {
                0 => 165.0,  // worm
                1 => 22.0,   // wasp
                2 => 340.0,  // troll
                3 => 200.0,  // griffin
                4 => 430.0,  // nest
                5 => 110.0,  // wraith (ally)
                6 => 30.0,   // balloon
                7 => 1500.0, // dragon
                _ => 30.0,
            };
            EHP[i] = hp;
            EHPM[i] = hp;
            return i as i32;
        }
        -1
    }
}
#[inline]
fn creature_float(t: i32) -> f32 {
    match t {
        1 => 26.0,
        3 => 34.0,
        5 => 20.0,
        6 => 46.0,
        7 => 52.0,
        _ => 3.0,
    }
}
fn add_orb(x: f32, y: f32, z: f32, amt: f32) {
    unsafe {
        for i in 0..MAXO {
            if OALIVE[i] != 0 {
                continue;
            }
            OALIVE[i] = 1;
            OX[i] = x;
            OY[i] = y;
            OZ[i] = z;
            OAMT[i] = amt;
            OOWN[i] = 0;
            OPHASE[i] = rr(0.0, 6.2832);
            OHELD[i] = -1;
            return;
        }
    }
}

// ---- spell table -----------------------------------------------------------
// cost, cooldown seconds
static mut SCOST: [f32; NSPELL] = [0.0; NSPELL];
static mut SCD: [f32; NSPELL] = [0.0; NSPELL];
// each tier costs more than the last
#[inline]
fn fortress_cost(w: usize) -> f32 {
    unsafe { 34.0 + CLEV[w] as f32 * 26.0 }
}
fn init_spell_table() {
    unsafe {
        SCOST[0] = 4.0;
        SCD[0] = 0.28; // Firebolt
        SCOST[1] = 9.0;
        SCD[1] = 1.10; // Chain lightning
        SCOST[2] = 14.0;
        SCD[2] = 2.20; // Crater
        SCOST[3] = 30.0;
        SCD[3] = 6.00; // Volcano
        SCOST[4] = 24.0;
        SCD[4] = 5.00; // Earthquake
        SCOST[5] = 46.0;
        SCD[5] = 8.00; // Meteor
        SCOST[6] = 16.0;
        SCD[6] = 9.00; // Shield
        SCOST[7] = 18.0;
        SCD[7] = 7.00; // Mend
        SCOST[8] = 12.0;
        SCD[8] = 8.00; // Haste
        SCOST[9] = 4.0;
        SCD[9] = 0.55; // Claim
        SCOST[10] = 22.0;
        SCD[10] = 6.0; // Wraith
        SCOST[11] = 60.0;
        SCD[11] = 22.0; // Sunburst
        SCOST[12] = 34.0;
        SCD[12] = 1.60; // Fortress (real cost is fortress_cost)
    }
}
fn unlock_for(lv: i32) {
    unsafe {
        for i in 0..NSPELL {
            UNL[i] = 0;
        }
        // firebolt, claim, mend, fortress
        UNL[0] = 1;
        UNL[9] = 1;
        UNL[7] = 1;
        UNL[12] = 1;
        if lv >= 2 {
            UNL[1] = 1;
        }
        if lv >= 3 {
            UNL[2] = 1;
        }
        if lv >= 3 {
            UNL[6] = 1;
        }
        if lv >= 4 {
            UNL[8] = 1;
        }
        if lv >= 4 {
            UNL[4] = 1;
        }
        if lv >= 5 {
            UNL[10] = 1;
        }
        if lv >= 6 {
            UNL[3] = 1;
        }
        if lv >= 7 {
            UNL[5] = 1;
        }
        if lv >= 8 {
            UNL[11] = 1;
        }
    }
}

// ---- level generation ------------------------------------------------------
fn gen_terrain() {
    unsafe {
        // archipelago: sum of radial island masks modulated by fbm
        let n_isl = 3 + ri(3);
        let mut ix = [0.0f32; 8];
        let mut iz = [0.0f32; 8];
        let mut ir = [0.0f32; 8];
        let mut ih = [0.0f32; 8];
        for k in 0..n_isl as usize {
            ix[k] = rr(0.18, 0.82) * WORLD;
            iz[k] = rr(0.18, 0.82) * WORLD;
            ir[k] = rr(0.16, 0.30) * WORLD;
            ih[k] = rr(38.0, 96.0);
        }
        for j in 0..TW {
            for i in 0..TW {
                let wxp = i as f32 * CELL;
                let wzp = j as f32 * CELL;
                let mut h: f32 = -26.0;
                for k in 0..n_isl as usize {
                    let dx = wxp - ix[k];
                    let dz = wzp - iz[k];
                    let d = sqrt(dx * dx + dz * dz) / ir[k];
                    if d < 1.4 {
                        let mut m: f32 = 1.0 - d;
                        if m < 0.0 {
                            m = 0.0;
                        }
                        m = m * m * (3.0 - 2.0 * m);
                        h += ih[k] * m * 1.6;
                    }
                }
                let n = fbm(wxp * 0.0055, wzp * 0.0055, 5) * 34.0
                    + fbm(wxp * 0.021, wzp * 0.021, 3) * 9.0;
                h += n * if h > -10.0 { 1.0 } else { 0.35 };
                // beach flattening near sea level
                if h > -4.0 && h < 8.0 {
                    h *= 0.55;
                }
                HEIGHT[(j * TW + i) as usize] = h;
            }
        }
        DIRTY_LO = 0;
        DIRTY_HI = TWM;
    }
}
fn find_land(min_h: f32) -> i32 {
    for _ in 0..4000 {
        let i = 12 + ri(TW - 24);
        let j = 12 + ri(TW - 24);
        if h_get(i, j) > min_h {
            return j * TW + i;
        }
    }
    (TW / 2) * TW + TW / 2
}
fn place_scenery() {
    unsafe {
        DEC_COUNT = 0;
        for _ in 0..MAXDEC {
            let x = rr(0.0, WORLD);
            let z = rr(0.0, WORLD);
            let h = height_at(x, z);
            if h < 2.0 {
                continue;
            }
            let kind = if h < 16.0 {
                0 // palm
            } else if h < 60.0 {
                if ri(3) == 0 {
                    0
                } else {
                    1
                } // rock/palm mix
            } else {
                1 // rock
            };
            DX[DEC_COUNT] = x;
            DZ[DEC_COUNT] = z;
            DKIND[DEC_COUNT] = kind;
            DSC[DEC_COUNT] = rr(0.7, 1.5);
            DROT[DEC_COUNT] = rr(0.0, 6.2832);
            DEC_COUNT += 1;
        }
    }
}

// ---- combat ----------------------------------------------------------------
fn drop_mana(x: f32, y: f32, z: f32, amt: f32) {
    let mut left = amt;
    while left > 0.5 {
        let chunk: f32 = if left > 9.0 { 9.0 } else { left };
        add_orb(x + rr(-9.0, 9.0), y + rr(2.0, 9.0), z + rr(-9.0, 9.0), chunk);
        left -= chunk;
    }
}
fn kill_creature(i: usize) {
    unsafe {
        let t = ETYPE[i];
        let mana: f32 = match t {
            0 => 15.0,
            1 => 8.0,
            2 => 28.0,
            3 => 20.0,
            4 => 60.0,
            7 => 110.0,
            5 | 6 => 0.0,
            _ => 8.0,
        };
        EALIVE[i] = 0;
        if EOWN[i] == 0 {
            KILLS += 1.0;
        }
        burst(EX[i], EY[i], EZ[i], 16, 16.0, 3.0, 1.0, 0.55, 0.15, 0.7);
        if mana > 0.0 {
            drop_mana(EX[i], EY[i], EZ[i], mana);
            push_event(3.0, EX[i], EY[i], EZ[i]);
        }
    }
}
// fac: 0 = wild, w+1 = wizard w and everything it owns
fn damage_area(x: f32, y: f32, z: f32, rad: f32, dmg: f32, fac: i32) {
    unsafe {
        let r2 = rad * rad;
        for i in 0..MAXE {
            if EALIVE[i] == 0 {
                continue;
            }
            if EOWN[i] == fac {
                continue;
            }
            let dx = EX[i] - x;
            let dy = EY[i] - y;
            let dz = EZ[i] - z;
            let d2 = dx * dx + dy * dy + dz * dz;
            if d2 > r2 {
                continue;
            }
            let fall: f32 = 1.0 - sqrt(d2) / rad;
            EHP[i] -= dmg * (0.45 + 0.55 * fall);
            EHURT[i] = 0.25;
            let inv: f32 = 1.0 / (sqrt(d2) + 0.001);
            EVX[i] += dx * inv * dmg * 0.5;
            EVY[i] += dy * inv * dmg * 0.25 + 4.0;
            EVZ[i] += dz * inv * dmg * 0.5;
            if EHP[i] <= 0.0 {
                kill_creature(i);
            }
        }
        for w in 0..=N_RIVAL {
            if WALIVE[w] == 0 || w as i32 + 1 == fac {
                continue;
            }
            let dx = WX[w] - x;
            let dy = WY[w] - y;
            let dz = WZ[w] - z;
            let d2 = dx * dx + dy * dy + dz * dz;
            if d2 > r2 {
                continue;
            }
            let fall: f32 = 1.0 - sqrt(d2) / rad;
            let mut d: f32 = dmg * (0.4 + 0.6 * fall);
            if WSHIELD[w] > 0.0 {
                d *= 0.25;
            }
            WHP[w] -= d;
            WNOHIT[w] = 0.0;
            if w == 0 {
                SHAKE = if SHAKE > 0.5 { SHAKE } else { 0.5 };
            }
            if WHP[w] <= 0.0 {
                WHP[w] = 0.0;
                if w == 0 {
                    STATUS = 2;
                } else {
                    WALIVE[w] = 0;
                    WRESP[w] = 14.0;
                    drop_mana(WX[w], WY[w], WZ[w], WMANA[w] * 0.65);
                    WMANA[w] *= 0.35;
                    burst(WX[w], WY[w], WZ[w], 40, 26.0, 4.0, 0.9, 0.3, 0.9, 1.1);
                    push_event(4.0, WX[w], WY[w], WZ[w]);
                }
            }
        }
        // castles
        for w in 0..=N_RIVAL {
            if w as i32 + 1 == fac || CHP[w] <= 0.0 {
                continue;
            }
            let dx = CX[w] - x;
            let dz = CZ[w] - z;
            let dy = CY[w] + 20.0 - y;
            let d2 = dx * dx + dz * dz + dy * dy;
            if d2 > (rad + 26.0) * (rad + 26.0) {
                continue;
            }
            CHP[w] -= dmg * 0.8;
            if CHP[w] <= 0.0 {
                CHP[w] = 0.0;
                drop_mana(CX[w], CY[w] + 18.0, CZ[w], WSTORE[w] * 0.75);
                WSTORE[w] *= 0.25;
                burst(CX[w], CY[w] + 16.0, CZ[w], 70, 34.0, 6.0, 1.0, 0.7, 0.3, 1.6);
                SHAKE = 1.2;
                push_event(5.0, CX[w], CY[w], CZ[w]);
            }
        }
    }
}
fn add_proj(
    x: f32,
    y: f32,
    z: f32,
    vx: f32,
    vy: f32,
    vz: f32,
    t: i32,
    own: i32,
    dmg: f32,
    rad: f32,
    life: f32,
) {
    unsafe {
        for i in 0..MAXP {
            if PALIVE[i] != 0 {
                continue;
            }
            PALIVE[i] = 1;
            PX[i] = x;
            PY[i] = y;
            PZ[i] = z;
            PVX[i] = vx;
            PVY[i] = vy;
            PVZ[i] = vz;
            PTYPE[i] = t;
            POWN[i] = own;
            PDMG[i] = dmg;
            PRAD[i] = rad;
            PLIFE[i] = life;
            return;
        }
    }
}
// march a ray until it hits terrain; returns hit distance (or maxd)
fn ray_ground(x: f32, y: f32, z: f32, dx: f32, dy: f32, dz: f32, maxd: f32) -> f32 {
    let mut t: f32 = 4.0;
    while t < maxd {
        let hx = x + dx * t;
        let hy = y + dy * t;
        let hz = z + dz * t;
        let g = height_at(hx, hz);
        if hy <= g {
            return t;
        }
        let gap = hy - g;
        t += if gap > 12.0 { gap * 0.6 } else { 5.0 };
    }
    maxd
}

// ---- casting ---------------------------------------------------------------
fn fwd_vec(w: usize) -> (f32, f32, f32) {
    unsafe {
        let cp = cos(WPIT[w]);
        let sp = sin(WPIT[w]);
        (sin(WYAW[w]) * cp, sp, cos(WYAW[w]) * cp)
    }
}

fn arc_bolt(x0: f32, y0: f32, z0: f32, x1: f32, y1: f32, z1: f32) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let dz = z1 - z0;
    let len = sqrt(dx * dx + dy * dy + dz * dz);
    let n = (len / 5.0) as i32;
    let steps = if n < 4 {
        4
    } else if n > 90 {
        90
    } else {
        n
    };
    for k in 0..=steps {
        let t = k as f32 / steps as f32;
        let j: f32 = 5.0 * sin(t * 3.14159);
        spawn_part(
            x0 + dx * t + rr(-j, j),
            y0 + dy * t + rr(-j, j),
            z0 + dz * t + rr(-j, j),
            rr(-3.0, 3.0),
            rr(-3.0, 3.0),
            rr(-3.0, 3.0),
            rr(0.14, 0.34),
            rr(2.0, 4.2),
            0.62,
            0.80,
            1.0,
            0.0,
            1.0,
        );
    }
}

fn cast_spell(w: usize, s: i32) -> i32 {
    unsafe {
        if s < 0 || s >= NSPELL as i32 {
            return 0;
        }
        let si = s as usize;
        if w == 0 && UNL[si] == 0 {
            return 0;
        }
        if w == 0 && CD[si] > 0.0 {
            return 0;
        }
        // Fortress only works standing over your own keep, and only while there
        // is a tier left to buy. Checked before any mana is spent.
        if s == 12 {
            let dxc = CX[w] - WX[w];
            let dyc = CY[w] - WY[w];
            let dzc = CZ[w] - WZ[w];
            if CHP[w] <= 0.0 || CLEV[w] >= 6 {
                return 0;
            }
            if dxc * dxc + dyc * dyc + dzc * dzc > 210.0 * 210.0 {
                return 0;
            }
        }
        let need: f32 = if s == 12 {
            fortress_cost(w)
        } else {
            SCOST[si]
        };
        if WMANA[w] < need {
            return 0;
        }
        WMANA[w] -= need;
        if w == 0 {
            CD[si] = SCD[si];
        }
        let fac = w as i32 + 1;
        let (fx, fy, fz) = fwd_vec(w);
        let sx = WX[w] + fx * 6.0;
        let sy = WY[w] - 2.0 + fy * 6.0;
        let sz = WZ[w] + fz * 6.0;

        if s == 0 {
            // Firebolt
            let fdmg: f32 = if w == 0 {
                34.0
            } else {
                11.0 + LEVEL as f32 * 2.3
            };
            add_proj(
                sx,
                sy,
                sz,
                fx * 220.0,
                fy * 220.0,
                fz * 220.0,
                0,
                fac,
                fdmg,
                20.0,
                3.2,
            );
            push_event(0.0, sx, sy, sz);
        } else if s == 1 {
            // Chain lightning
            let mut best: i32 = -1;
            let mut bd: f32 = 340.0 * 340.0;
            for i in 0..MAXE {
                if EALIVE[i] == 0 || EOWN[i] == fac {
                    continue;
                }
                let dx = EX[i] - WX[w];
                let dy = EY[i] - WY[w];
                let dz = EZ[i] - WZ[w];
                let d2 = dx * dx + dy * dy + dz * dz;
                if d2 > bd {
                    continue;
                }
                let inv = 1.0 / sqrt(d2);
                if dx * inv * fx + dy * inv * fy + dz * inv * fz < 0.35 {
                    continue;
                }
                bd = d2;
                best = i as i32;
            }
            let mut lx = WX[w];
            let mut ly = WY[w];
            let mut lz = WZ[w];
            if best >= 0 {
                let mut cur = best;
                let mut hop = 0;
                while hop < 4 && cur >= 0 {
                    let c = cur as usize;
                    arc_bolt(lx, ly, lz, EX[c], EY[c], EZ[c]);
                    damage_area(EX[c], EY[c], EZ[c], 16.0, 52.0, fac);
                    lx = EX[c];
                    ly = EY[c];
                    lz = EZ[c];
                    let mut nx: i32 = -1;
                    let mut nd: f32 = 150.0 * 150.0;
                    for i in 0..MAXE {
                        if EALIVE[i] == 0 || i as i32 == cur || EOWN[i] == fac {
                            continue;
                        }
                        let dx = EX[i] - lx;
                        let dy = EY[i] - ly;
                        let dz = EZ[i] - lz;
                        let d2 = dx * dx + dy * dy + dz * dz;
                        if d2 < nd {
                            nd = d2;
                            nx = i as i32;
                        }
                    }
                    cur = nx;
                    hop += 1;
                }
            } else {
                let d = ray_ground(WX[w], WY[w], WZ[w], fx, fy, fz, 340.0);
                arc_bolt(
                    lx,
                    ly,
                    lz,
                    WX[w] + fx * d,
                    WY[w] + fy * d,
                    WZ[w] + fz * d,
                );
                damage_area(
                    WX[w] + fx * d,
                    WY[w] + fy * d,
                    WZ[w] + fz * d,
                    26.0,
                    40.0,
                    fac,
                );
            }
            push_event(1.0, WX[w], WY[w], WZ[w]);
        } else if s == 2 {
            // Crater
            let d = ray_ground(WX[w], WY[w], WZ[w], fx, fy, fz, 420.0);
            let tx = clamp_world(WX[w] + fx * d);
            let tz = clamp_world(WZ[w] + fz * d);
            deform(tx, tz, 46.0, 20.0, 0);
            damage_area(tx, height_at(tx, tz), tz, 54.0, 46.0, fac);
            burst(
                tx,
                height_at(tx, tz) + 6.0,
                tz,
                44,
                22.0,
                6.0,
                0.75,
                0.6,
                0.45,
                1.4,
            );
            if w == 0 {
                SHAKE = 0.9;
            }
            push_event(2.0, tx, height_at(tx, tz), tz);
        } else if s == 3 {
            // Volcano
            let d = ray_ground(WX[w], WY[w], WZ[w], fx, fy, fz, 420.0);
            let tx = clamp_world(WX[w] + fx * d);
            let tz = clamp_world(WZ[w] + fz * d);
            deform(tx, tz, 60.0, 62.0, 1);
            damage_area(tx, height_at(tx, tz), tz, 66.0, 60.0, fac);
            for _ in 0..90 {
                let a = rr(0.0, 6.2832);
                let sp2 = rr(20.0, 70.0);
                spawn_part(
                    tx,
                    height_at(tx, tz) + 30.0,
                    tz,
                    cos(a) * sp2 * 0.4,
                    rr(40.0, 105.0),
                    sin(a) * sp2 * 0.4,
                    rr(1.4, 3.0),
                    rr(4.0, 9.0),
                    1.0,
                    rr(0.25, 0.6),
                    0.08,
                    -30.0,
                    0.5,
                );
            }
            if w == 0 {
                SHAKE = 1.3;
            }
            push_event(6.0, tx, height_at(tx, tz), tz);
        } else if s == 4 {
            // Earthquake — travelling ripple
            for k in 1..=7 {
                let dd = k as f32 * 52.0;
                let tx = clamp_world(WX[w] + fx * dd);
                let tz = clamp_world(WZ[w] + fz * dd);
                deform(tx, tz, 40.0, 13.0 - k as f32 * 0.9, 2);
                damage_area(tx, height_at(tx, tz), tz, 46.0, 34.0, fac);
                burst(
                    tx,
                    height_at(tx, tz) + 3.0,
                    tz,
                    14,
                    15.0,
                    5.0,
                    0.65,
                    0.5,
                    0.36,
                    1.1,
                );
            }
            if w == 0 {
                SHAKE = 1.5;
            }
            push_event(7.0, WX[w], WY[w], WZ[w]);
        } else if s == 5 {
            // Meteor
            let d = ray_ground(WX[w], WY[w], WZ[w], fx, fy, fz, 520.0);
            let tx = clamp_world(WX[w] + fx * d);
            let tz = clamp_world(WZ[w] + fz * d);
            add_proj(
                tx - 60.0,
                420.0,
                tz - 60.0,
                60.0 * 0.55,
                -240.0,
                60.0 * 0.55,
                2,
                fac,
                150.0,
                78.0,
                6.0,
            );
            push_event(8.0, tx, 300.0, tz);
        } else if s == 6 {
            WSHIELD[w] = 10.0;
            push_event(9.0, WX[w], WY[w], WZ[w]);
        } else if s == 7 {
            WHP[w] += 55.0;
            if WHP[w] > 100.0 {
                WHP[w] = 100.0;
            }
            for _ in 0..30 {
                spawn_part(
                    WX[w] + rr(-8.0, 8.0),
                    WY[w] + rr(-6.0, 6.0),
                    WZ[w] + rr(-8.0, 8.0),
                    0.0,
                    rr(6.0, 20.0),
                    0.0,
                    1.0,
                    3.0,
                    0.4,
                    1.0,
                    0.6,
                    4.0,
                    1.0,
                );
            }
            push_event(10.0, WX[w], WY[w], WZ[w]);
        } else if s == 8 {
            WHASTE[w] = 12.0;
            push_event(9.0, WX[w], WY[w], WZ[w]);
        } else if s == 9 {
            // Claim — possess loose mana.
            // Turns unclaimed gold orbs white and marks them for your balloons.
            // Possessing mana also permanently widens your own pool, which is
            // the only way to afford the expensive spells later on.
            let mut n2 = 0;
            let mut gain: f32 = 0.0;
            for i in 0..MAXO {
                if OALIVE[i] == 0 || OHELD[i] >= 0 || OOWN[i] == fac {
                    continue;
                }
                let dx = OX[i] - WX[w];
                let dy = OY[i] - WY[w];
                let dz = OZ[i] - WZ[w];
                if dx * dx + dy * dy + dz * dz > 200.0 * 200.0 {
                    continue;
                }
                OOWN[i] = fac;
                n2 += 1;
                gain += OAMT[i];
                for _ in 0..7 {
                    spawn_part(
                        OX[i],
                        OY[i],
                        OZ[i],
                        rr(-9.0, 9.0),
                        rr(2.0, 12.0),
                        rr(-9.0, 9.0),
                        0.55,
                        2.0,
                        0.75,
                        0.9,
                        1.0,
                        0.0,
                        1.2,
                    );
                }
            }
            if n2 > 0 {
                WCAP[w] += gain * 0.28;
                WMANA[w] += gain * 0.42;
                let cp = mana_cap(w);
                if WMANA[w] > cp {
                    WMANA[w] = cp;
                }
                if w == 0 {
                    push_event(3.0, WX[w], WY[w], WZ[w]);
                }
            }
        } else if s == 10 {
            // Wraith ally
            let d = ray_ground(WX[w], WY[w], WZ[w], fx, fy, fz, 260.0);
            let tx = clamp_world(WX[w] + fx * d);
            let tz = clamp_world(WZ[w] + fz * d);
            let id = add_creature(5, tx, tz, fac);
            if id >= 0 {
                ETIM[id as usize] = 45.0;
                burst(
                    tx,
                    height_at(tx, tz) + 12.0,
                    tz,
                    30,
                    14.0,
                    4.0,
                    0.55,
                    0.35,
                    1.0,
                    1.2,
                );
            }
            push_event(12.0, tx, height_at(tx, tz), tz);
        } else if s == 12 {
            // Fortress — raise your keep a tier
            CLEV[w] += 1;
            CHPM[w] += 220.0;
            CHP[w] = CHPM[w];
            burst(
                CX[w],
                CY[w] + 26.0,
                CZ[w],
                70,
                30.0,
                12.0,
                0.55,
                0.85,
                1.0,
                2.2,
            );
            if w == 0 {
                SHAKE = 0.7;
                push_event(16.0, CX[w], CY[w], CZ[w]);
            }
        } else if s == 11 {
            // Sunburst — smites every hostile creature
            for i in 0..MAXE {
                if EALIVE[i] == 0 || EOWN[i] == fac {
                    continue;
                }
                if ETYPE[i] == 4 {
                    EHP[i] -= 120.0;
                } else {
                    EHP[i] -= 200.0;
                }
                arc_bolt(EX[i], EY[i] + 260.0, EZ[i], EX[i], EY[i], EZ[i]);
                if EHP[i] <= 0.0 {
                    kill_creature(i);
                }
            }
            if w == 0 {
                SHAKE = 1.0;
            }
            push_event(13.0, WX[w], WY[w], WZ[w]);
        }
        1
    }
}

// ---- wizards ---------------------------------------------------------------
// Regen ceiling. Nothing can be laundered into progress through it any more
// (the fortress is filled by balloons), so it can afford to be generous — it
// has to at least reach the price of the first Fortress tier.
const BANK_FLOOR: f32 = 72.0;
#[inline]
fn mana_cap(w: usize) -> f32 {
    unsafe { 120.0 + CLEV[w] as f32 * 50.0 + WCAP[w] }
}
// A fortress can only hold so much; raising a tier is what buys headroom.
#[inline]
fn castle_cap(w: usize) -> f32 {
    unsafe { 240.0 * CLEV[w] as f32 }
}
// mana that must sit inside your fortress to take the realm
#[inline]
fn level_target() -> f32 {
    unsafe { 180.0 + LEVEL as f32 * 160.0 }
}

fn update_player(dt: f32) {
    unsafe {
        let w = 0usize;
        // Same handedness trap as the strafe vector: d(forward)/d(yaw) points
        // along -screen_right, so a rising yaw swings the view left. Mouse
        // right must decrease it. (Pitch is already the right way round.)
        WYAW[w] -= I_YAW;
        WPIT[w] -= I_PIT;
        if WPIT[w] > 1.05 {
            WPIT[w] = 1.05;
        }
        if WPIT[w] < -1.05 {
            WPIT[w] = -1.05;
        }
        let roll = -I_STR * 0.42;
        WROL[w] += (roll - WROL[w]) * fmin(1.0, dt * 6.0);
        let (fx, fy, fz) = fwd_vec(w);
        // Screen-right. The view matrix puts the camera's x axis at
        // (-cos yaw, 0, sin yaw); strafing along +(cos yaw, 0, -sin yaw) sent
        // you the opposite way, so D slid left.
        let rx = -cos(WYAW[w]);
        let rz = sin(WYAW[w]);
        let mut maxs: f32 = 132.0;
        if WHASTE[w] > 0.0 {
            maxs *= 1.75;
        }
        if I_BRAKE != 0 {
            maxs *= 0.28;
        }
        let tvx = fx * I_FWD * maxs + rx * I_STR * maxs * 0.55;
        let tvy = fy * I_FWD * maxs + I_UP * 78.0;
        let tvz = fz * I_FWD * maxs + rz * I_STR * maxs * 0.55;
        // Brisk enough that turning re-aims the carpet rather than leaving it
        // skating along its old heading.
        let k = fmin(1.0, dt * 5.2);
        WVX[w] += (tvx - WVX[w]) * k;
        WVY[w] += (tvy - WVY[w]) * k;
        WVZ[w] += (tvz - WVZ[w]) * k;
        WX[w] += WVX[w] * dt;
        WY[w] += WVY[w] * dt;
        WZ[w] += WVZ[w] * dt;
        WX[w] = clamp_world(WX[w]);
        WZ[w] = clamp_world(WZ[w]);
        let g = height_at(WX[w], WZ[w]);
        let floor_y = fmax(g, SEA) + 7.5;
        if WY[w] < floor_y {
            let pen = floor_y - WY[w];
            WY[w] += pen * fmin(1.0, dt * 14.0);
            if WVY[w] < -60.0 {
                WHP[w] -= (-WVY[w] - 60.0) * dt * 1.2;
                SHAKE = 0.4;
            }
            if WVY[w] < 0.0 {
                WVY[w] *= 0.25;
            }
        }
        if WY[w] > 430.0 {
            WY[w] = 430.0;
            if WVY[w] > 0.0 {
                WVY[w] = 0.0;
            }
        }
        if WSHIELD[w] > 0.0 {
            WSHIELD[w] -= dt;
        }
        if WHASTE[w] > 0.0 {
            WHASTE[w] -= dt;
        }
        // carpet trail
        if rnd() < 0.55 {
            spawn_part(
                WX[w] + rr(-4.0, 4.0),
                WY[w] - 3.0,
                WZ[w] + rr(-4.0, 4.0),
                0.0,
                rr(-2.0, 2.0),
                0.0,
                0.55,
                1.6,
                0.45,
                0.35,
                0.85,
                0.0,
                2.0,
            );
        }
    }
}

fn wizard_common(w: usize, dt: f32) {
    unsafe {
        WNOHIT[w] += dt;
        if WNOHIT[w] > 6.0 && WHP[w] < 100.0 {
            WHP[w] += 1.5 * dt;
            if WHP[w] > 100.0 {
                WHP[w] = 100.0;
            }
        }
        // passive spell fuel from your own castle, up to the regen ceiling
        if CHP[w] > 0.0 && WMANA[w] < BANK_FLOOR {
            WMANA[w] += (5.0 + CLEV[w] as f32 * 2.2) * dt;
            if WMANA[w] > BANK_FLOOR {
                WMANA[w] = BANK_FLOOR;
            }
        }
        // Orbs are not vacuumed up by flying over them any more: you possess
        // them with Claim and your balloons carry them to the fortress.
        if CHP[w] > 0.0 {
            let dx = CX[w] - WX[w];
            let dz = CZ[w] - WZ[w];
            let dy = CY[w] + 16.0 - WY[w];
            if dx * dx + dz * dz + dy * dy < 62.0 * 62.0 {
                WHP[w] += 7.0 * dt;
                if WHP[w] > 100.0 {
                    WHP[w] = 100.0;
                }
            }
        }
        // Tier is bought with the Fortress spell now, never derived from the
        // store. Rivals buy their own once their keep is nearly full.
        if w != 0
            && CHP[w] > 0.0
            && CLEV[w] < 6
            && WSTORE[w] > castle_cap(w) * 0.82
            && WMANA[w] > 44.0
        {
            CLEV[w] += 1;
            WMANA[w] -= 44.0;
            CHPM[w] += 220.0;
            CHP[w] = CHPM[w];
        }
        // balloons ferry mana home
        if CHP[w] > 0.0 {
            CBT[w] -= dt;
            if CBT[w] <= 0.0 {
                CBT[w] = 7.0;
                let mut n = 0;
                for i in 0..MAXE {
                    if EALIVE[i] != 0 && ETYPE[i] == 6 && EOWN[i] == w as i32 + 1 {
                        n += 1;
                    }
                }
                let cap = if w == 0 {
                    CLEV[w] + 1
                } else if 1 + LEVEL / 2 < CLEV[w] {
                    1 + LEVEL / 2
                } else {
                    CLEV[w]
                };
                if n < cap {
                    add_creature(
                        6,
                        CX[w] + rr(-20.0, 20.0),
                        CZ[w] + rr(-20.0, 20.0),
                        w as i32 + 1,
                    );
                }
            }
        }
    }
}

fn aim_at(w: usize, tx: f32, ty: f32, tz: f32) {
    unsafe {
        let dx = tx - WX[w];
        let dy = ty - WY[w];
        let dz = tz - WZ[w];
        let hd = sqrt(dx * dx + dz * dz);
        WYAW[w] = atan2(dx, dz);
        WPIT[w] = atan2(dy, hd);
    }
}
fn aim_and_fire(w: usize, tx: f32, ty: f32, tz: f32, lead: i32) {
    unsafe {
        let mut ax = tx;
        let mut ay = ty;
        let mut az = tz;
        if lead != 0 {
            ax += WVX[0] * 0.42;
            ay += WVY[0] * 0.42;
            az += WVZ[0] * 0.42;
        }
        aim_at(w, ax, ay, az);
        let mut j: f32 = 0.13 - LEVEL as f32 * 0.013;
        if j < 0.015 {
            j = 0.015;
        }
        WYAW[w] += rr(-j, j);
        WPIT[w] += rr(-j, j) * 0.6;
        cast_spell(w, 0);
    }
}

fn update_rival(w: usize, dt: f32) {
    unsafe {
        if WALIVE[w] == 0 {
            WRESP[w] -= dt;
            if WRESP[w] <= 0.0 && CHP[w] > 0.0 {
                WALIVE[w] = 1;
                WHP[w] = 100.0;
                WX[w] = CX[w];
                WZ[w] = CZ[w] + 30.0;
                WY[w] = CY[w] + 45.0;
                WMANA[w] = 60.0;
            }
            return;
        }
        if WSHIELD[w] > 0.0 {
            WSHIELD[w] -= dt;
        }
        if WHASTE[w] > 0.0 {
            WHASTE[w] -= dt;
        }
        WAITIM[w] -= dt;
        // pick behaviour
        if WHP[w] < 34.0 {
            WAI[w] = 3;
        } else if WAITIM[w] <= 0.0 {
            WAITIM[w] = rr(1.6, 3.4);
            let dpx = WX[0] - WX[w];
            let dpz = WZ[0] - WZ[w];
            let dpy = WY[0] - WY[w];
            let dp2 = dpx * dpx + dpz * dpz + dpy * dpy;
            if WALIVE[0] != 0
                && dp2 < 640.0 * 640.0
                && WMANA[w] > 26.0
                && rnd() < 0.24 + LEVEL as f32 * 0.06
            {
                WAI[w] = 2;
            } else if WMANA[w] > mana_cap(w) * 0.8 {
                WAI[w] = 1;
            } else {
                WAI[w] = 0;
            }
        }
        let mut tx = CX[w];
        let mut ty = CY[w] + 34.0;
        let mut tz = CZ[w];
        if WAI[w] == 0 {
            let mut best: i32 = -1;
            let mut bd: f32 = 1e18;
            for i in 0..MAXO {
                if OALIVE[i] == 0 || OHELD[i] >= 0 {
                    continue;
                }
                let dx = OX[i] - WX[w];
                let dz = OZ[i] - WZ[w];
                let d2 = dx * dx + dz * dz;
                if d2 < bd {
                    bd = d2;
                    best = i as i32;
                }
            }
            if best >= 0 {
                let b = best as usize;
                tx = OX[b];
                ty = OY[b] + 6.0;
                tz = OZ[b];
                // Possess what it is flying to, so its balloons have something
                // to carry. Rate-limited: AI casts skip the player's cooldown
                // table, so an ungated call here fires 60x a second and
                // hoovers the map instantly.
                if bd < 190.0 * 190.0 && rnd() < 0.015 {
                    cast_spell(w, 9);
                }
            } else {
                // hunt creatures for fresh mana
                let mut bc: i32 = -1;
                bd = 1e18;
                for i in 0..MAXE {
                    if EALIVE[i] == 0 || EOWN[i] != 0 {
                        continue;
                    }
                    let dx = EX[i] - WX[w];
                    let dz = EZ[i] - WZ[w];
                    let d2 = dx * dx + dz * dz;
                    if d2 < bd {
                        bd = d2;
                        bc = i as i32;
                    }
                }
                if bc >= 0 {
                    let c = bc as usize;
                    tx = EX[c];
                    ty = EY[c] + 26.0;
                    tz = EZ[c];
                    if bd < 540.0 * 540.0 && WCD[w] <= 0.0 {
                        aim_and_fire(w, EX[c], EY[c], EZ[c], 0);
                        WCD[w] = rr(0.5, 1.1);
                    }
                }
            }
        } else if WAI[w] == 2 && WALIVE[0] != 0 {
            tx = WX[0] - sin(WYAW[0]) * 130.0;
            ty = WY[0] + 16.0;
            tz = WZ[0] - cos(WYAW[0]) * 130.0;
            let dx = WX[0] - WX[w];
            let dy = WY[0] - WY[w];
            let dz = WZ[0] - WZ[w];
            let d2 = dx * dx + dy * dy + dz * dz;
            if d2 < 620.0 * 620.0 && WCD[w] <= 0.0 {
                let r = rnd();
                if r < 0.16 && WMANA[w] > 26.0 && LEVEL >= 3 {
                    aim_at(w, WX[0], WY[0], WZ[0]);
                    cast_spell(w, 2);
                    WCD[w] = rr(2.4, 4.0);
                } else if r < 0.36 && WMANA[w] > 12.0 && LEVEL >= 2 {
                    aim_at(w, WX[0], WY[0], WZ[0]);
                    cast_spell(w, 1);
                    WCD[w] = rr(1.4, 2.6);
                } else {
                    aim_and_fire(w, WX[0], WY[0], WZ[0], 1);
                    WCD[w] = rr(0.42, 0.9) + (9.0 - LEVEL as f32) * 0.11;
                }
            }
        } else if WAI[w] == 3 {
            if WHP[w] < 70.0 && WMANA[w] > 20.0 && WCD[w] <= 0.0 {
                cast_spell(w, 7);
                WCD[w] = 3.0;
            }
            if WHP[w] > 82.0 {
                WAI[w] = 0;
            }
        }
        WCD[w] -= dt;
        // steer
        let dx = tx - WX[w];
        let dy = ty - WY[w];
        let dz = tz - WZ[w];
        let d = sqrt(dx * dx + dy * dy + dz * dz) + 0.001;
        let mut spd: f32 = 74.0 + LEVEL as f32 * 4.5;
        if WHASTE[w] > 0.0 {
            spd *= 1.6;
        }
        let k = fmin(1.0, dt * 2.2);
        WVX[w] += (dx / d * spd - WVX[w]) * k;
        WVY[w] += (dy / d * spd * 0.7 - WVY[w]) * k;
        WVZ[w] += (dz / d * spd - WVZ[w]) * k;
        WX[w] = clamp_world(WX[w] + WVX[w] * dt);
        WY[w] += WVY[w] * dt;
        WZ[w] = clamp_world(WZ[w] + WVZ[w] * dt);
        let floor_y = fmax(height_at(WX[w], WZ[w]), SEA) + 16.0;
        if WY[w] < floor_y {
            WY[w] = floor_y;
            if WVY[w] < 0.0 {
                WVY[w] = 0.0;
            }
        }
        if WY[w] > 280.0 {
            WY[w] = 280.0;
        }
        if WAI[w] != 2 {
            WYAW[w] = atan2(dx, dz);
        }
        if rnd() < 0.5 {
            spawn_part(
                WX[w] + rr(-4.0, 4.0),
                WY[w] - 3.0,
                WZ[w] + rr(-4.0, 4.0),
                0.0,
                rr(-2.0, 2.0),
                0.0,
                0.6,
                2.0,
                1.0,
                0.35,
                0.3,
                0.0,
                2.0,
            );
        }
    }
}

// ---- creatures -------------------------------------------------------------
fn nearest_hostile(i: usize, maxd: f32) -> i32 {
    unsafe {
        let mut best: i32 = -1;
        let mut bd = maxd * maxd;
        let mine = EOWN[i];
        for k in 0..MAXE {
            if EALIVE[k] == 0 || k == i {
                continue;
            }
            if EOWN[k] == mine {
                continue;
            }
            if ETYPE[k] == 6 {
                continue;
            }
            let dx = EX[k] - EX[i];
            let dy = EY[k] - EY[i];
            let dz = EZ[k] - EZ[i];
            let d2 = dx * dx + dy * dy + dz * dz;
            if d2 < bd {
                bd = d2;
                best = k as i32;
            }
        }
        best
    }
}
fn hurt_wizard(w: usize, d: f32) {
    unsafe {
        if WALIVE[w] == 0 {
            return;
        }
        let mut dd = d;
        if WSHIELD[w] > 0.0 {
            dd *= 0.25;
        }
        WHP[w] -= dd;
        WNOHIT[w] = 0.0;
        if w == 0 {
            SHAKE = if SHAKE > 0.32 { SHAKE } else { 0.32 };
            push_event(17.0, WX[0], WY[0], WZ[0]);
        }
        if WHP[w] <= 0.0 {
            WHP[w] = 0.0;
            if w == 0 {
                STATUS = 2;
            } else {
                WALIVE[w] = 0;
                WRESP[w] = 14.0;
                drop_mana(WX[w], WY[w], WZ[w], WMANA[w] * 0.65);
                WMANA[w] *= 0.35;
            }
        }
    }
}

fn update_creatures(dt: f32) {
    unsafe {
        for i in 0..MAXE {
            if EALIVE[i] == 0 {
                continue;
            }
            let t = ETYPE[i];
            let own = EOWN[i];
            EPHASE[i] += dt * if t == 1 { 26.0 } else { 4.0 };
            if EHURT[i] > 0.0 {
                EHURT[i] -= dt;
            }
            ECD[i] -= dt;

            if t == 4 {
                // nest
                ETIM[i] -= dt;
                if ETIM[i] <= 0.0 {
                    ETIM[i] = fmax(6.0, 17.0 - LEVEL as f32 * 0.8);
                    let mut n = 0;
                    for k in 0..MAXE {
                        if EALIVE[k] != 0 && EOWN[k] == 0 && ETYPE[k] != 4 {
                            n += 1;
                        }
                    }
                    if n < 8 + LEVEL * 3 {
                        let mut ct = ri(4);
                        if ct == 4 {
                            ct = 1;
                        }
                        let id = add_creature(
                            ct,
                            EX[i] + rr(-30.0, 30.0),
                            EZ[i] + rr(-30.0, 30.0),
                            0,
                        );
                        if id >= 0 {
                            burst(EX[i], EY[i] + 6.0, EZ[i], 8, 8.0, 2.5, 0.6, 0.3, 0.8, 0.6);
                        }
                    }
                }
                EY[i] = height_at(EX[i], EZ[i]) + 3.0;
                continue;
            }
            if t == 6 {
                // mana balloon
                let ow = (own - 1) as usize;
                // Balloons cruise high, but orbs hover at ground + 5. Without
                // stooping, the grab sphere can never close and nothing is
                // ever delivered.
                let mut floor_lift: f32 = 34.0;
                let mut tx = CX[ow];
                let mut ty = CY[ow] + 46.0;
                let mut tz = CZ[ow];
                let mut carrying: i32 = -1;
                for k in 0..MAXO {
                    if OALIVE[k] != 0 && OHELD[k] == i as i32 {
                        carrying = k as i32;
                        break;
                    }
                }
                if carrying < 0 {
                    let mut best: i32 = -1;
                    let mut bd: f32 = 980.0 * 980.0;
                    for k in 0..MAXO {
                        if OALIVE[k] == 0 || OHELD[k] >= 0 || OOWN[k] != own {
                            continue;
                        }
                        let dx = OX[k] - EX[i];
                        let dz = OZ[k] - EZ[i];
                        let d2 = dx * dx + dz * dz;
                        if d2 < bd {
                            bd = d2;
                            best = k as i32;
                        }
                    }
                    if best >= 0 {
                        let b = best as usize;
                        tx = OX[b];
                        ty = OY[b] + 3.0;
                        tz = OZ[b];
                        floor_lift = 5.0;
                        let dx = OX[b] - EX[i];
                        let dy = OY[b] - EY[i];
                        let dz = OZ[b] - EZ[i];
                        if dx * dx + dy * dy + dz * dz < 30.0 * 30.0 {
                            OHELD[b] = i as i32;
                        }
                    }
                } else {
                    let c = carrying as usize;
                    OX[c] = EX[i];
                    OY[c] = EY[i] - 10.0;
                    OZ[c] = EZ[i];
                    let dx = CX[ow] - EX[i];
                    let dz = CZ[ow] - EZ[i];
                    let dy = CY[ow] + 30.0 - EY[i];
                    if dx * dx + dz * dz + dy * dy < 52.0 * 52.0 {
                        // Rivals ferry at a handicap that closes as the realms
                        // get harder — the old economy had the same ramp on
                        // their banking rate.
                        let eff: f32 = if ow == 0 {
                            1.0
                        } else {
                            0.34 + LEVEL as f32 * 0.08
                        };
                        WSTORE[ow] = fmin(WSTORE[ow] + OAMT[c] * eff, castle_cap(ow));
                        OALIVE[c] = 0;
                        for _ in 0..10 {
                            spawn_part(
                                CX[ow],
                                CY[ow] + 22.0,
                                CZ[ow],
                                rr(-10.0, 10.0),
                                rr(4.0, 20.0),
                                rr(-10.0, 10.0),
                                0.7,
                                2.4,
                                0.5,
                                0.8,
                                1.0,
                                0.0,
                                1.5,
                            );
                        }
                    }
                }
                let dx = tx - EX[i];
                let dy = ty - EY[i];
                let dz = tz - EZ[i];
                let d = sqrt(dx * dx + dy * dy + dz * dz) + 0.01;
                let sp: f32 = 66.0; // the map is 2.5x wider than it was
                EVX[i] += (dx / d * sp - EVX[i]) * fmin(1.0, dt * 1.5);
                EVY[i] += (dy / d * sp * 0.6 - EVY[i]) * fmin(1.0, dt * 1.5);
                EVZ[i] += (dz / d * sp - EVZ[i]) * fmin(1.0, dt * 1.5);
                EX[i] = clamp_world(EX[i] + EVX[i] * dt);
                EY[i] += EVY[i] * dt;
                EZ[i] = clamp_world(EZ[i] + EVZ[i] * dt);
                let fy = fmax(height_at(EX[i], EZ[i]), SEA) + floor_lift;
                if EY[i] < fy {
                    EY[i] = fy;
                }
                EYAW[i] = atan2(dx, dz);
                if EHP[i] <= 0.0 {
                    if carrying >= 0 {
                        OHELD[carrying as usize] = -1;
                    }
                    EALIVE[i] = 0;
                }
                continue;
            }

            // combat creatures
            let flying = t == 1 || t == 3 || t == 5 || t == 7;
            let mut tx: f32 = 0.0;
            let mut ty: f32 = 0.0;
            let mut tz: f32 = 0.0;
            let mut has_t = false;
            let mut td2: f32 = 1e18;
            if own == 0 {
                // hostile: prefer player, else allied creatures
                let mut bd: f32 = 1e18;
                let mut bw: i32 = -1;
                for w in 0..=N_RIVAL {
                    if WALIVE[w] == 0 {
                        continue;
                    }
                    let dx = WX[w] - EX[i];
                    let dy = WY[w] - EY[i];
                    let dz = WZ[w] - EZ[i];
                    let d2 = dx * dx + dy * dy + dz * dz;
                    let bias: f32 = if w == 0 { 0.55 } else { 1.0 };
                    if d2 * bias < bd {
                        bd = d2 * bias;
                        bw = w as i32;
                    }
                }
                let ally = nearest_hostile(i, 220.0);
                let aggro: f32 = if t == 1 || t == 3 {
                    300.0
                } else if t == 7 {
                    380.0
                } else {
                    200.0
                };
                let mut real_d2: f32 = 1e18;
                if bw >= 0 {
                    let b = bw as usize;
                    let ax = WX[b] - EX[i];
                    let ay = WY[b] - EY[i];
                    let az = WZ[b] - EZ[i];
                    real_d2 = ax * ax + ay * ay + az * az;
                }
                if bw >= 0 && real_d2 < aggro * aggro {
                    let b = bw as usize;
                    tx = WX[b];
                    ty = WY[b];
                    tz = WZ[b];
                    has_t = true;
                    td2 = real_d2;
                } else if ally >= 0 {
                    let a = ally as usize;
                    tx = EX[a];
                    ty = EY[a];
                    tz = EZ[a];
                    has_t = true;
                    let ddx = tx - EX[i];
                    let ddy = ty - EY[i];
                    let ddz = tz - EZ[i];
                    td2 = ddx * ddx + ddy * ddy + ddz * ddz;
                }
            } else {
                if t == 5 {
                    ETIM[i] -= dt;
                    if ETIM[i] <= 0.0 {
                        kill_creature(i);
                        continue;
                    }
                }
                let h = nearest_hostile(i, 420.0);
                if h >= 0 {
                    let hh = h as usize;
                    tx = EX[hh];
                    ty = EY[hh];
                    tz = EZ[hh];
                    has_t = true;
                    let ddx = tx - EX[i];
                    let ddy = ty - EY[i];
                    let ddz = tz - EZ[i];
                    td2 = ddx * ddx + ddy * ddy + ddz * ddz;
                } else {
                    let opp = if own == 1 { 1usize } else { 0usize };
                    if WALIVE[opp] != 0 && opp <= N_RIVAL {
                        tx = WX[opp];
                        ty = WY[opp];
                        tz = WZ[opp];
                        has_t = true;
                        let ddx = tx - EX[i];
                        let ddy = ty - EY[i];
                        let ddz = tz - EZ[i];
                        td2 = ddx * ddx + ddy * ddy + ddz * ddz;
                    }
                }
            }

            let spd: f32 = match t {
                0 => 24.0,
                1 => 74.0,
                2 => 20.0,
                3 => 82.0,
                5 => 58.0,
                7 => 66.0,
                _ => 26.0,
            };

            if has_t {
                let dx = tx - EX[i];
                let dy = ty - EY[i];
                let dz = tz - EZ[i];
                let d = sqrt(td2) + 0.01;
                EYAW[i] = atan2(dx, dz);
                let rng: f32 = if t == 0 {
                    150.0
                } else if t == 2 {
                    170.0
                } else if t == 7 {
                    210.0
                } else {
                    0.0
                };
                if rng > 0.0 && d < rng {
                    // ranged: hold position and shoot
                    EVX[i] *= 0.9;
                    EVZ[i] *= 0.9;
                    if ECD[i] <= 0.0 {
                        ECD[i] = if t == 7 { rr(1.1, 2.0) } else { rr(1.6, 3.0) };
                        let sp2: f32 = if t == 7 { 190.0 } else { 140.0 };
                        let inv: f32 = 1.0 / d;
                        let shots = if t == 7 { 3 } else { 1 };
                        for _ in 0..shots {
                            add_proj(
                                EX[i],
                                EY[i] + 4.0,
                                EZ[i],
                                (dx * inv + rr(-0.08, 0.08)) * sp2,
                                (dy * inv + 0.16 + rr(-0.05, 0.05)) * sp2,
                                (dz * inv + rr(-0.08, 0.08)) * sp2,
                                if t == 7 { 3 } else { 1 },
                                own,
                                if t == 7 { 15.0 } else { 7.0 },
                                16.0,
                                3.4,
                            );
                        }
                    }
                } else {
                    let k = fmin(1.0, dt * 2.0);
                    EVX[i] += (dx / d * spd - EVX[i]) * k;
                    EVZ[i] += (dz / d * spd - EVZ[i]) * k;
                    if flying {
                        EVY[i] += (dy / d * spd * 0.75 - EVY[i]) * k;
                    }
                    // melee
                    if d < 22.0 && ECD[i] <= 0.0 {
                        ECD[i] = 1.45;
                        let dmg: f32 = match t {
                            1 => 4.0,
                            2 => 11.0,
                            3 => 7.0,
                            5 => 20.0,
                            7 => 17.0,
                            _ => 6.0,
                        };
                        for w in 0..=N_RIVAL {
                            if WALIVE[w] == 0 || w as i32 + 1 == own {
                                continue;
                            }
                            let ddx = WX[w] - EX[i];
                            let ddy = WY[w] - EY[i];
                            let ddz = WZ[w] - EZ[i];
                            if ddx * ddx + ddy * ddy + ddz * ddz < 26.0 * 26.0 {
                                hurt_wizard(w, dmg);
                            }
                        }
                        damage_area(EX[i], EY[i], EZ[i], 20.0, dmg, own);
                        burst(
                            EX[i] + dx / d * 10.0,
                            EY[i] + dy / d * 10.0,
                            EZ[i] + dz / d * 10.0,
                            6,
                            10.0,
                            2.0,
                            1.0,
                            0.6,
                            0.3,
                            0.35,
                        );
                    }
                }
            } else {
                ETIM[i] -= dt;
                if ETIM[i] <= 0.0 {
                    ETIM[i] = rr(2.0, 5.0);
                    EYAW[i] = rr(0.0, 6.2832);
                }
                let k = fmin(1.0, dt * 1.2);
                EVX[i] += (sin(EYAW[i]) * spd * 0.45 - EVX[i]) * k;
                EVZ[i] += (cos(EYAW[i]) * spd * 0.45 - EVZ[i]) * k;
                if flying {
                    EVY[i] += (sin(EPHASE[i] * 0.3) * 8.0 - EVY[i]) * k;
                }
            }
            EX[i] = clamp_world(EX[i] + EVX[i] * dt);
            EZ[i] = clamp_world(EZ[i] + EVZ[i] * dt);
            let gh = fmax(height_at(EX[i], EZ[i]), SEA - 2.0);
            if flying {
                EY[i] += EVY[i] * dt;
                let min_y = gh + creature_float(t) * 0.6;
                if EY[i] < min_y {
                    EY[i] = min_y;
                    if EVY[i] < 0.0 {
                        EVY[i] = 0.0;
                    }
                }
                if EY[i] > 260.0 {
                    EY[i] = 260.0;
                    if EVY[i] > 0.0 {
                        EVY[i] = 0.0;
                    }
                }
            } else {
                EVY[i] -= 90.0 * dt;
                EY[i] += EVY[i] * dt;
                let rest = gh + 3.0;
                if EY[i] < rest {
                    EY[i] = rest;
                    EVY[i] = 0.0;
                }
            }
            if EHP[i] <= 0.0 {
                kill_creature(i);
            }
        }
    }
}

// ---- projectiles -----------------------------------------------------------
fn update_projectiles(dt: f32) {
    unsafe {
        for i in 0..MAXP {
            if PALIVE[i] == 0 {
                continue;
            }
            let t = PTYPE[i];
            if t == 1 || t == 3 {
                PVY[i] -= 42.0 * dt;
            }
            if t == 2 {
                PVY[i] -= 34.0 * dt;
            }
            PX[i] += PVX[i] * dt;
            PY[i] += PVY[i] * dt;
            PZ[i] += PVZ[i] * dt;
            PLIFE[i] -= dt;
            // trail
            let (cr, cg, cb) = match t {
                1 => (0.55f32, 0.95f32, 0.35f32),
                2 => (1.0, 0.45, 0.12),
                3 => (1.0, 0.32, 0.5),
                _ => (1.0, 0.55, 0.18),
            };
            let n_trail = if t == 2 { 4 } else { 1 };
            for _ in 0..n_trail {
                spawn_part(
                    PX[i] + rr(-2.0, 2.0),
                    PY[i] + rr(-2.0, 2.0),
                    PZ[i] + rr(-2.0, 2.0),
                    rr(-6.0, 6.0),
                    rr(-2.0, 10.0),
                    rr(-6.0, 6.0),
                    if t == 2 { 0.7 } else { 0.34 },
                    if t == 2 { 6.5 } else { 3.4 },
                    cr,
                    cg,
                    cb,
                    0.0,
                    1.5,
                );
            }
            let mut hit = false;
            if PY[i] <= height_at(PX[i], PZ[i]) {
                hit = true;
            }
            if PY[i] <= SEA - 1.0 {
                hit = true;
            }
            if !hit {
                for e in 0..MAXE {
                    if EALIVE[e] == 0 || EOWN[e] == POWN[i] || ETYPE[e] == 6 {
                        continue;
                    }
                    let dx = EX[e] - PX[i];
                    let dy = EY[e] - PY[i];
                    let dz = EZ[e] - PZ[i];
                    if dx * dx + dy * dy + dz * dz < 15.0 * 15.0 {
                        hit = true;
                        break;
                    }
                }
            }
            if !hit {
                for w in 0..=N_RIVAL {
                    if WALIVE[w] == 0 || w as i32 + 1 == POWN[i] {
                        continue;
                    }
                    let dx = WX[w] - PX[i];
                    let dy = WY[w] - PY[i];
                    let dz = WZ[w] - PZ[i];
                    if dx * dx + dy * dy + dz * dz < 13.0 * 13.0 {
                        hurt_wizard(w, PDMG[i]);
                        hit = true;
                        break;
                    }
                }
            }
            if !hit && PLIFE[i] <= 0.0 {
                hit = true;
            }
            if hit {
                PALIVE[i] = 0;
                damage_area(PX[i], PY[i], PZ[i], PRAD[i], PDMG[i], POWN[i]);
                if t == 2 {
                    deform(PX[i], PZ[i], 74.0, 30.0, 0);
                    burst(PX[i], PY[i], PZ[i], 80, 40.0, 9.0, 1.0, 0.6, 0.2, 1.6);
                    SHAKE = 1.6;
                    push_event(18.0, PX[i], PY[i], PZ[i]);
                } else if t == 0 {
                    deform(PX[i], PZ[i], 16.0, 2.2, 0);
                    burst(PX[i], PY[i], PZ[i], 22, 20.0, 4.4, 1.0, 0.6, 0.22, 0.7);
                    push_event(19.0, PX[i], PY[i], PZ[i]);
                } else {
                    burst(PX[i], PY[i], PZ[i], 14, 15.0, 3.4, cr, cg, cb, 0.6);
                    push_event(19.0, PX[i], PY[i], PZ[i]);
                }
            }
        }
    }
}

// ---- particles / orbs ------------------------------------------------------
fn update_particles(dt: f32) {
    unsafe {
        for i in 0..MAXPT {
            if QL[i] <= 0.0 {
                continue;
            }
            QL[i] -= dt;
            if QL[i] <= 0.0 {
                continue;
            }
            QVY[i] += QGRAV[i] * dt;
            let dr: f32 = 1.0 - QDRAG[i] * dt;
            let d2: f32 = if dr < 0.0 { 0.0 } else { dr };
            QVX[i] *= d2;
            QVY[i] *= d2;
            QVZ[i] *= d2;
            QX[i] += QVX[i] * dt;
            QY[i] += QVY[i] * dt;
            QZ[i] += QVZ[i] * dt;
        }
    }
}
fn update_orbs(dt: f32) {
    unsafe {
        for i in 0..MAXO {
            if OALIVE[i] == 0 || OHELD[i] >= 0 {
                continue;
            }
            OPHASE[i] += dt * 2.4;
            let g = fmax(height_at(OX[i], OZ[i]), SEA) + 5.0 + sin(OPHASE[i]) * 1.6;
            // settle toward hover height, but weakly so a nearby carpet can lift it
            OY[i] += (g - OY[i]) * fmin(1.0, dt * 0.8);
            if OY[i] < g - 2.0 {
                OY[i] = g - 2.0;
            }
        }
    }
}

// ---- render buffer ---------------------------------------------------------
#[inline]
fn push_inst(
    x: f32,
    y: f32,
    z: f32,
    sx: f32,
    sy: f32,
    sz: f32,
    r: f32,
    g: f32,
    b: f32,
    yaw: f32,
    glow: f32,
    shape: f32,
) {
    unsafe {
        if INST_N >= MAXI {
            return;
        }
        let o = INST_N * 12;
        INST[o] = x;
        INST[o + 1] = y;
        INST[o + 2] = z;
        INST[o + 3] = sx;
        INST[o + 4] = sy;
        INST[o + 5] = sz;
        INST[o + 6] = r;
        INST[o + 7] = g;
        INST[o + 8] = b;
        INST[o + 9] = yaw;
        INST[o + 10] = glow;
        INST[o + 11] = shape;
        INST_N += 1;
    }
}
#[inline]
fn push_map(x: f32, z: f32, kind: f32, sc: f32) {
    unsafe {
        if MAP_N >= MAXMAP {
            return;
        }
        let o = MAP_N * 4;
        MAP[o] = x;
        MAP[o + 1] = z;
        MAP[o + 2] = kind;
        MAP[o + 3] = sc;
        MAP_N += 1;
    }
}
// Lowest ground under a footprint of radius r. A castle placed at the height of
// its own centre point hangs in the air on the downhill side of any slope, so
// the plinth is sunk to reach this instead.
fn ground_min(x: f32, z: f32, r: f32) -> f32 {
    let mut m = height_at(x, z);
    for k in 0..8 {
        let a: f32 = k as f32 * 0.7854;
        let h = height_at(x + cos(a) * r, z + sin(a) * r);
        if h < m {
            m = h;
        }
    }
    m
}
const FOOT: f32 = 27.0; // plinth half-width; must clear the tower ring

fn draw_castle(w: usize) {
    unsafe {
        let gm = ground_min(CX[w], CZ[w], FOOT);
        if CHP[w] <= 0.0 {
            let top = CY[w] + 3.5;
            let bot = gm - 5.0;
            push_inst(
                CX[w],
                (top + bot) * 0.5,
                CZ[w],
                30.0,
                top - bot,
                30.0,
                0.22,
                0.19,
                0.17,
                0.0,
                0.0,
                0.0,
            );
            return;
        }
        let lv = CLEV[w];
        let (r, g, b) = if w != 0 {
            (0.70f32, 0.20f32, 0.16f32)
        } else {
            (0.24, 0.34, 0.72)
        };
        let dmg_f = CHP[w] / CHPM[w];
        let base = CY[w];
        // Plinth: buried below the lowest ground it covers so it reads as cut
        // into the hill. The pad is levelled at placement, so this normally
        // only has a few units to make up — the clamp is for ground later
        // blown away under it.
        let deck = base + 6.0;
        let mut bot = gm - 7.0;
        if bot < base - 26.0 {
            bot = base - 26.0;
        }
        push_inst(
            CX[w],
            (deck + bot) * 0.5,
            CZ[w],
            FOOT * 2.0,
            deck - bot,
            FOOT * 2.0,
            r * 0.5 + 0.14,
            g * 0.5 + 0.12,
            b * 0.5 + 0.10,
            0.0,
            0.0,
            0.0,
        );
        // a narrower skirt just under the deck reads as a stepped foundation
        push_inst(
            CX[w],
            deck - 1.6,
            CZ[w],
            FOOT * 2.0 - 7.0,
            5.0,
            FOOT * 2.0 - 7.0,
            r * 0.34 + 0.10,
            g * 0.34 + 0.09,
            b * 0.34 + 0.08,
            0.0,
            0.0,
            0.0,
        );
        let keep_h: f32 = 14.0 + lv as f32 * 7.0;
        push_inst(
            CX[w],
            deck + keep_h * 0.5,
            CZ[w],
            19.0,
            keep_h,
            19.0,
            r,
            g,
            b,
            0.0,
            0.0,
            0.0,
        );
        push_inst(
            CX[w],
            deck + 2.0 + keep_h,
            CZ[w],
            15.0,
            13.0,
            15.0,
            r * 1.25,
            g * 1.25,
            b * 1.25,
            0.785,
            0.15,
            2.0,
        );
        let towers = 2 + lv;
        for k in 0..towers {
            let a: f32 = k as f32 / towers as f32 * 6.2832 + 0.4;
            let rad: f32 = 19.0;
            let txx = CX[w] + cos(a) * rad;
            let tzz = CZ[w] + sin(a) * rad;
            let th: f32 = 10.0 + lv as f32 * 3.4 + sin(k as f32 * 2.1) * 3.0;
            push_inst(
                txx,
                deck - 2.0 + th * 0.5,
                tzz,
                7.0,
                th,
                7.0,
                r * 0.9,
                g * 0.9,
                b * 0.9,
                a,
                0.0,
                0.0,
            );
            push_inst(
                txx,
                deck - 1.0 + th,
                tzz,
                6.5,
                8.0,
                6.5,
                r * 1.4,
                g * 1.4,
                b * 1.4,
                a,
                0.25,
                2.0,
            );
        }
        // banner glow scales with stored mana
        let glow_h: f32 = 6.0 + lv as f32 * 2.0;
        push_inst(
            CX[w],
            deck + 12.0 + keep_h,
            CZ[w],
            3.0,
            glow_h,
            3.0,
            0.55,
            0.85,
            1.0,
            0.0,
            1.0,
            1.0,
        );
        if dmg_f < 0.6 && rnd() < 0.4 {
            spawn_part(
                CX[w] + rr(-16.0, 16.0),
                base + rr(6.0, keep_h),
                CZ[w] + rr(-16.0, 16.0),
                rr(-3.0, 3.0),
                rr(6.0, 16.0),
                rr(-3.0, 3.0),
                1.4,
                5.0,
                0.35,
                0.33,
                0.3,
                2.0,
                0.9,
            );
        }
        push_map(CX[w], CZ[w], if w == 0 { 6.0 } else { 7.0 }, 1.0);
    }
}

fn draw_creature(i: usize) {
    unsafe {
        let t = ETYPE[i];
        let x = EX[i];
        let y = EY[i];
        let z = EZ[i];
        let ya = EYAW[i];
        let hurt: f32 = if EHURT[i] > 0.0 { 1.0 } else { 0.0 };
        let ph = EPHASE[i];
        // sf/cf are the facing unit vector; +side is the creature's own right.
        //   forward = (sf, cf)      right = (cf, -sf)
        // Parts are kept flat and high-contrast, with dark or glowing eyes: the
        // renderer's edge pass keys off local colour contrast, so hard internal
        // borders are what give these a crisp pixel-art silhouette.
        let sf = sin(ya);
        let cf = cos(ya);
        if t == 0 {
            // sand worm: segmented chain
            for s in 0..6 {
                let off: f32 = s as f32 * 5.4;
                let sx2 = x - sf * off;
                let sz2 = z - cf * off;
                let bob: f32 = sin(ph - s as f32 * 0.8) * 3.2;
                let sc: f32 = 6.8 - s as f32 * 0.62;
                push_inst(
                    sx2,
                    y + bob + 1.0,
                    sz2,
                    sc,
                    sc * 0.85,
                    sc,
                    0.62 + hurt * 0.35,
                    0.52 - hurt * 0.2,
                    0.28,
                    ya,
                    hurt * 0.5,
                    1.0,
                );
                if s < 5 {
                    push_inst(
                        sx2,
                        y + bob + 1.0 + sc * 0.42,
                        sz2,
                        sc * 0.5,
                        sc * 0.36,
                        sc * 0.8,
                        0.40,
                        0.30,
                        0.15,
                        ya,
                        0.0,
                        0.0,
                    );
                }
            }
            let hb: f32 = sin(ph) * 3.2;
            push_inst(
                x + sf * 3.4,
                y + hb + 0.2,
                z + cf * 3.4,
                3.6,
                1.8,
                2.2,
                0.28,
                0.09,
                0.09,
                ya,
                0.0,
                0.0,
            );
            push_inst(
                x + sf * 2.4 + cf * 1.7,
                y + hb + 2.4,
                z + cf * 2.4 - sf * 1.7,
                1.5,
                1.5,
                1.5,
                0.04,
                0.03,
                0.04,
                ya,
                0.0,
                1.0,
            );
            push_inst(
                x + sf * 2.4 - cf * 1.7,
                y + hb + 2.4,
                z + cf * 2.4 + sf * 1.7,
                1.5,
                1.5,
                1.5,
                0.04,
                0.03,
                0.04,
                ya,
                0.0,
                1.0,
            );
        } else if t == 1 {
            // wasp
            push_inst(
                x,
                y,
                z,
                3.4,
                3.0,
                5.0,
                0.85 + hurt * 0.15,
                0.72,
                0.18,
                ya,
                hurt * 0.6,
                1.0,
            );
            push_inst(
                x - sf * 1.4,
                y,
                z - cf * 1.4,
                3.2,
                2.8,
                1.2,
                0.11,
                0.08,
                0.05,
                ya,
                0.0,
                0.0,
            );
            push_inst(
                x - sf * 2.8,
                y,
                z - cf * 2.8,
                2.5,
                2.2,
                1.1,
                0.11,
                0.08,
                0.05,
                ya,
                0.0,
                0.0,
            );
            push_inst(
                x - sf * 4.4,
                y,
                z - cf * 4.4,
                1.4,
                2.8,
                1.4,
                0.16,
                0.12,
                0.10,
                ya,
                0.0,
                2.0,
            );
            push_inst(
                x + sf * 2.9,
                y + 0.3,
                z + cf * 2.9,
                2.7,
                2.5,
                2.5,
                0.26,
                0.19,
                0.07,
                ya,
                0.0,
                1.0,
            );
            push_inst(
                x + sf * 3.7 + cf * 0.9,
                y + 0.7,
                z + cf * 3.7 - sf * 0.9,
                1.2,
                1.2,
                1.2,
                0.03,
                0.03,
                0.03,
                ya,
                0.0,
                1.0,
            );
            push_inst(
                x + sf * 3.7 - cf * 0.9,
                y + 0.7,
                z + cf * 3.7 + sf * 0.9,
                1.2,
                1.2,
                1.2,
                0.03,
                0.03,
                0.03,
                ya,
                0.0,
                1.0,
            );
            let fl = sin(ph) * 0.9;
            push_inst(
                x,
                y + 2.2,
                z,
                7.5,
                0.4,
                2.2,
                0.9,
                0.9,
                0.95,
                ya + fl,
                0.1,
                0.0,
            );
        } else if t == 2 {
            // troll
            let sk: f32 = 0.35 + hurt * 0.5;
            push_inst(
                x,
                y + 5.8,
                z,
                8.0,
                10.0,
                6.5,
                sk,
                0.42,
                0.32,
                ya,
                hurt * 0.5,
                0.0,
            );
            push_inst(
                x,
                y + 12.6,
                z,
                5.4,
                4.6,
                5.0,
                sk + 0.07,
                0.48,
                0.36,
                ya,
                hurt * 0.5,
                0.0,
            );
            push_inst(
                x + sf * 2.5 + cf * 1.3,
                y + 13.4,
                z + cf * 2.5 - sf * 1.3,
                1.2,
                1.2,
                0.7,
                1.0,
                0.80,
                0.22,
                ya,
                0.7,
                0.0,
            );
            push_inst(
                x + sf * 2.5 - cf * 1.3,
                y + 13.4,
                z + cf * 2.5 + sf * 1.3,
                1.2,
                1.2,
                0.7,
                1.0,
                0.80,
                0.22,
                ya,
                0.7,
                0.0,
            );
            push_inst(
                x + cf * 2.3,
                y + 15.8,
                z - sf * 2.3,
                1.6,
                3.6,
                1.6,
                0.88,
                0.84,
                0.72,
                ya,
                0.0,
                2.0,
            );
            push_inst(
                x - cf * 2.3,
                y + 15.8,
                z + sf * 2.3,
                1.6,
                3.6,
                1.6,
                0.88,
                0.84,
                0.72,
                ya,
                0.0,
                2.0,
            );
            let sw = sin(ph) * 1.6;
            push_inst(
                x - cf * 5.5,
                y + 6.0 + sw,
                z + sf * 5.5,
                2.6,
                8.0,
                2.6,
                0.30,
                0.36,
                0.28,
                ya,
                0.0,
                0.0,
            );
            push_inst(
                x + cf * 5.5,
                y + 6.0 - sw,
                z - sf * 5.5,
                2.6,
                8.0,
                2.6,
                0.30,
                0.36,
                0.28,
                ya,
                0.0,
                0.0,
            );
            push_inst(
                x - cf * 2.2,
                y + 1.5,
                z + sf * 2.2,
                2.9,
                4.4,
                3.2,
                0.26,
                0.31,
                0.24,
                ya,
                0.0,
                0.0,
            );
            push_inst(
                x + cf * 2.2,
                y + 1.5,
                z - sf * 2.2,
                2.9,
                4.4,
                3.2,
                0.26,
                0.31,
                0.24,
                ya,
                0.0,
                0.0,
            );
        } else if t == 3 {
            // griffin
            push_inst(
                x,
                y,
                z,
                4.6,
                4.2,
                8.0,
                0.80 + hurt * 0.2,
                0.68,
                0.42,
                ya,
                hurt * 0.5,
                1.0,
            );
            push_inst(
                x + sf * 4.4,
                y + 1.6,
                z + cf * 4.4,
                3.4,
                3.2,
                3.4,
                0.94,
                0.90,
                0.78,
                ya,
                0.0,
                1.0,
            );
            push_inst(
                x + sf * 6.1,
                y + 1.3,
                z + cf * 6.1,
                1.7,
                1.9,
                2.6,
                0.95,
                0.70,
                0.14,
                ya,
                0.0,
                2.0,
            );
            push_inst(
                x + sf * 5.0 + cf * 1.2,
                y + 2.4,
                z + cf * 5.0 - sf * 1.2,
                1.0,
                1.0,
                1.0,
                0.04,
                0.03,
                0.03,
                ya,
                0.0,
                1.0,
            );
            push_inst(
                x + sf * 5.0 - cf * 1.2,
                y + 2.4,
                z + cf * 5.0 + sf * 1.2,
                1.0,
                1.0,
                1.0,
                0.04,
                0.03,
                0.03,
                ya,
                0.0,
                1.0,
            );
            push_inst(
                x - sf * 5.4,
                y - 0.4,
                z - cf * 5.4,
                2.2,
                2.2,
                5.2,
                0.68,
                0.55,
                0.32,
                ya,
                0.0,
                2.0,
            );
            let fl = sin(ph * 2.0) * 0.55;
            push_inst(
                x - cf * 7.0,
                y + 1.5 + fl * 4.0,
                z + sf * 7.0,
                12.0,
                1.2,
                6.0,
                0.92,
                0.86,
                0.7,
                ya,
                0.05,
                2.0,
            );
            push_inst(
                x + cf * 7.0,
                y + 1.5 - fl * 4.0,
                z - sf * 7.0,
                12.0,
                1.2,
                6.0,
                0.92,
                0.86,
                0.7,
                ya,
                0.05,
                2.0,
            );
        } else if t == 4 {
            // nest
            let pu: f32 = 1.0 + sin(ph * 0.7) * 0.06;
            // sunk well below its own ground point so it never perches on a slope
            push_inst(
                x,
                y - 5.0,
                z,
                19.0,
                17.0,
                19.0,
                0.24,
                0.16,
                0.26,
                0.0,
                0.0,
                0.0,
            );
            push_inst(
                x,
                y + 9.0,
                z,
                15.0 * pu,
                17.0 * pu,
                15.0 * pu,
                0.40,
                0.18,
                0.44,
                ph * 0.1,
                0.25,
                2.0,
            );
            for k in 0..4 {
                let a: f32 = k as f32 * 1.5708 + 0.7854;
                push_inst(
                    x + cos(a) * 7.6,
                    y + 4.2,
                    z + sin(a) * 7.6,
                    2.2,
                    7.4,
                    2.2,
                    0.28,
                    0.11,
                    0.32,
                    a,
                    0.0,
                    2.0,
                );
            }
            push_inst(x, y + 20.0, z, 4.0, 4.0, 4.0, 0.85, 0.35, 1.0, 0.0, 1.0, 1.0);
        } else if t == 5 {
            // wraith
            push_inst(x, y, z, 5.0, 6.5, 5.0, 0.55, 0.42, 1.0, ya, 0.75, 1.0);
            push_inst(
                x,
                y + 3.7,
                z,
                5.8,
                5.2,
                5.8,
                0.28,
                0.20,
                0.64,
                ya,
                0.2,
                2.0,
            );
            push_inst(
                x + sf * 1.9 + cf * 1.2,
                y + 1.1,
                z + cf * 1.9 - sf * 1.2,
                1.1,
                1.1,
                1.1,
                1.0,
                0.95,
                0.55,
                ya,
                1.0,
                1.0,
            );
            push_inst(
                x + sf * 1.9 - cf * 1.2,
                y + 1.1,
                z + cf * 1.9 + sf * 1.2,
                1.1,
                1.1,
                1.1,
                1.0,
                0.95,
                0.55,
                ya,
                1.0,
                1.0,
            );
            push_inst(
                x,
                y - 5.5,
                z,
                6.0,
                7.0,
                6.0,
                0.36,
                0.28,
                0.85,
                ya,
                0.5,
                2.0,
            );
            if rnd() < 0.4 {
                spawn_part(
                    x + rr(-4.0, 4.0),
                    y + rr(-4.0, 4.0),
                    z + rr(-4.0, 4.0),
                    0.0,
                    rr(2.0, 8.0),
                    0.0,
                    0.6,
                    2.4,
                    0.5,
                    0.4,
                    1.0,
                    0.0,
                    1.0,
                );
            }
        } else if t == 6 {
            // balloon
            let is_p = EOWN[i] == 1;
            push_inst(
                x,
                y,
                z,
                9.0,
                11.0,
                9.0,
                if is_p { 0.35 } else { 0.85 },
                if is_p { 0.6 } else { 0.3 },
                if is_p { 1.0 } else { 0.28 },
                ya,
                0.15,
                1.0,
            );
            push_inst(
                x,
                y + 5.6,
                z,
                4.6,
                3.4,
                4.6,
                0.92,
                0.88,
                0.72,
                ya,
                0.1,
                2.0,
            );
            push_inst(
                x,
                y - 2.0,
                z,
                9.4,
                1.4,
                9.4,
                0.20,
                0.16,
                0.12,
                ya,
                0.0,
                0.0,
            );
            push_inst(x, y - 6.0, z, 0.9, 6.0, 0.9, 0.28, 0.22, 0.16, ya, 0.0, 0.0);
            push_inst(x, y - 9.0, z, 4.0, 3.4, 4.0, 0.42, 0.32, 0.2, ya, 0.0, 0.0);
        } else if t == 7 {
            // dragon
            for s in 0..4 {
                let off: f32 = s as f32 * 7.0;
                let sx2 = x - sf * off;
                let sz2 = z - cf * off;
                let sc: f32 = 8.0 - s as f32 * 1.3;
                let sy2 = y + sin(ph - s as f32) * 1.4;
                push_inst(
                    sx2,
                    sy2,
                    sz2,
                    sc,
                    sc * 0.9,
                    sc * 1.2,
                    0.55 + hurt * 0.4,
                    0.16,
                    0.18,
                    ya,
                    hurt * 0.5,
                    1.0,
                );
                if s > 0 {
                    push_inst(
                        sx2,
                        sy2 + sc * 0.52,
                        sz2,
                        sc * 0.26,
                        sc * 0.55,
                        sc * 0.72,
                        0.28,
                        0.07,
                        0.09,
                        ya,
                        0.0,
                        2.0,
                    );
                }
            }
            let fl = sin(ph * 1.6) * 0.6;
            push_inst(
                x - cf * 11.0,
                y + 2.0 + fl * 6.0,
                z + sf * 11.0,
                20.0,
                1.6,
                9.0,
                0.35,
                0.10,
                0.14,
                ya,
                0.05,
                2.0,
            );
            push_inst(
                x + cf * 11.0,
                y + 2.0 - fl * 6.0,
                z - sf * 11.0,
                20.0,
                1.6,
                9.0,
                0.35,
                0.10,
                0.14,
                ya,
                0.05,
                2.0,
            );
            push_inst(
                x + sf * 8.0,
                y + 1.0,
                z + cf * 8.0,
                5.0,
                5.0,
                7.0,
                0.9,
                0.4,
                0.2,
                ya,
                0.4,
                2.0,
            );
            push_inst(
                x + sf * 6.8 + cf * 1.8,
                y + 3.6,
                z + cf * 6.8 - sf * 1.8,
                1.4,
                3.2,
                1.4,
                0.86,
                0.80,
                0.66,
                ya,
                0.0,
                2.0,
            );
            push_inst(
                x + sf * 6.8 - cf * 1.8,
                y + 3.6,
                z + cf * 6.8 + sf * 1.8,
                1.4,
                3.2,
                1.4,
                0.86,
                0.80,
                0.66,
                ya,
                0.0,
                2.0,
            );
            push_inst(
                x + sf * 9.6 + cf * 1.5,
                y + 1.7,
                z + cf * 9.6 - sf * 1.5,
                1.3,
                1.3,
                1.3,
                1.0,
                0.86,
                0.20,
                ya,
                1.0,
                1.0,
            );
            push_inst(
                x + sf * 9.6 - cf * 1.5,
                y + 1.7,
                z + cf * 9.6 + sf * 1.5,
                1.3,
                1.3,
                1.3,
                1.0,
                0.86,
                0.20,
                ya,
                1.0,
                1.0,
            );
            push_inst(
                x - sf * 25.0,
                y,
                z - cf * 25.0,
                2.4,
                5.2,
                2.4,
                0.38,
                0.09,
                0.11,
                ya,
                0.0,
                2.0,
            );
        }
        if t != 6 {
            let mut mk: f32 = 2.0;
            if EOWN[i] == 1 {
                mk = 3.0;
            } else if EOWN[i] == 2 {
                mk = 8.0;
            }
            if t == 4 {
                mk = 5.0;
            }
            push_map(x, z, mk, if t == 4 { 1.6 } else if t == 7 { 1.5 } else { 1.0 });
        } else {
            push_map(x, z, 9.0, 0.8);
        }
    }
}

fn build_render() {
    unsafe {
        INST_N = 0;
        PART_N = 0;
        MAP_N = 0;
        // scenery (distance culled around the player)
        let cull: f32 = 1050.0;
        for i in 0..DEC_COUNT {
            let dx = DX[i] - WX[0];
            let dz = DZ[i] - WZ[0];
            if dx * dx + dz * dz > cull * cull {
                continue;
            }
            let h = height_at(DX[i], DZ[i]);
            if h < 1.0 {
                continue; // sank under water (crater/quake)
            }
            let s = DSC[i];
            // Both are sunk past their own ground sample: the height is taken
            // at one point, so anything sitting exactly on it lifts off the
            // downhill side.
            if DKIND[i] == 0 {
                push_inst(
                    DX[i],
                    h + 5.0 * s,
                    DZ[i],
                    1.5 * s,
                    14.0 * s,
                    1.5 * s,
                    0.36,
                    0.27,
                    0.16,
                    DROT[i],
                    0.0,
                    0.0,
                );
                push_inst(
                    DX[i],
                    h + 14.0 * s,
                    DZ[i],
                    9.0 * s,
                    5.0 * s,
                    9.0 * s,
                    0.22,
                    0.44,
                    0.20,
                    DROT[i],
                    0.0,
                    2.0,
                );
            } else {
                push_inst(
                    DX[i],
                    h + 0.4 * s,
                    DZ[i],
                    5.0 * s,
                    6.0 * s,
                    4.4 * s,
                    0.44,
                    0.41,
                    0.38,
                    DROT[i],
                    0.0,
                    0.0,
                );
            }
        }
        for w in 0..=N_RIVAL {
            draw_castle(w);
        }
        for i in 0..MAXE {
            if EALIVE[i] != 0 {
                draw_creature(i);
            }
        }
        // rival carpets
        for w in 1..=N_RIVAL {
            if WALIVE[w] == 0 {
                continue;
            }
            push_inst(
                WX[w],
                WY[w],
                WZ[w],
                13.0,
                1.1,
                17.0,
                0.72,
                0.16,
                0.14,
                WYAW[w],
                0.2,
                0.0,
            );
            push_inst(
                WX[w],
                WY[w] + 4.0,
                WZ[w],
                4.0,
                6.0,
                4.0,
                0.9,
                0.8,
                0.6,
                WYAW[w],
                0.1,
                1.0,
            );
            if WSHIELD[w] > 0.0 {
                push_inst(
                    WX[w],
                    WY[w] + 2.0,
                    WZ[w],
                    20.0,
                    20.0,
                    20.0,
                    1.0,
                    0.5,
                    0.4,
                    0.0,
                    0.9,
                    1.0,
                );
            }
            push_map(WX[w], WZ[w], 1.0, 1.4);
        }
        // The player's own carpet and shield. Emitted as one contiguous run so
        // the renderer can drop it wholesale in first person — see
        // carpetInstLo/carpetInstHi.
        CARPET_LO = INST_N as i32;
        {
            let ya = WYAW[0];
            let bob = sin(GTIME * 2.3) * 0.35;
            push_inst(
                WX[0],
                WY[0] - 2.7 + bob,
                WZ[0],
                16.6,
                0.5,
                20.6,
                0.88,
                0.65,
                0.24,
                ya,
                0.05,
                0.0,
            );
            push_inst(
                WX[0],
                WY[0] - 2.1 + bob,
                WZ[0],
                15.0,
                0.9,
                19.0,
                0.13,
                0.20,
                0.56,
                ya,
                0.02,
                0.0,
            );
            push_inst(
                WX[0],
                WY[0] - 1.6 + bob,
                WZ[0],
                5.2,
                0.5,
                6.6,
                0.72,
                0.20,
                0.16,
                ya,
                0.04,
                0.0,
            );
            if WSHIELD[0] > 0.0 {
                push_inst(
                    WX[0],
                    WY[0],
                    WZ[0],
                    24.0,
                    24.0,
                    24.0,
                    0.42,
                    0.72,
                    1.0,
                    0.0,
                    0.8,
                    1.0,
                );
            }
        }
        CARPET_HI = INST_N as i32;
        // mana orbs
        let mut orb_n = 0;
        for i in 0..MAXO {
            if OALIVE[i] == 0 {
                continue;
            }
            orb_n += 1;
            let s: f32 = 2.0 + OAMT[i] * 0.18;
            let pl: f32 = 0.75 + sin(OPHASE[i] * 2.0) * 0.25;
            // gold = nobody's yet, white = yours and inbound on a balloon,
            // red = theirs
            let (orr, org, orb2, mk) = if OOWN[i] == 1 {
                (0.72f32, 0.92f32, 1.00f32, 4.0f32)
            } else if OOWN[i] == 2 {
                (1.00, 0.36, 0.30, 8.0)
            } else {
                (1.00, 0.78, 0.16, 10.0)
            };
            push_inst(
                OX[i],
                OY[i],
                OZ[i],
                s,
                s,
                s,
                orr * pl,
                org * pl,
                orb2 * pl,
                0.0,
                1.0,
                1.0,
            );
            push_map(OX[i], OZ[i], mk, 0.7);
            if rnd() < 0.10 {
                spawn_part(
                    OX[i],
                    OY[i],
                    OZ[i],
                    rr(-3.0, 3.0),
                    rr(3.0, 9.0),
                    rr(-3.0, 3.0),
                    0.8,
                    1.8,
                    0.45,
                    0.8,
                    1.0,
                    0.0,
                    1.2,
                );
            }
        }
        // projectiles
        for i in 0..MAXP {
            if PALIVE[i] == 0 {
                continue;
            }
            let t = PTYPE[i];
            let (r, g, b, s) = match t {
                1 => (0.5f32, 1.0f32, 0.4f32, 2.6f32),
                2 => (1.0, 0.45, 0.12, 9.0),
                3 => (1.0, 0.3, 0.5, 3.4),
                _ => (1.0, 0.6, 0.2, 3.0),
            };
            push_inst(PX[i], PY[i], PZ[i], s, s, s, r, g, b, 0.0, 1.0, 1.0);
        }
        // particles -> flat buffer
        for i in 0..MAXPT {
            if QL[i] <= 0.0 {
                continue;
            }
            let f = QL[i] / QLM[i];
            let o = PART_N * 8;
            PART[o] = QX[i];
            PART[o + 1] = QY[i];
            PART[o + 2] = QZ[i];
            PART[o + 3] = QS[i] * (0.35 + f * 0.9);
            PART[o + 4] = QR[i];
            PART[o + 5] = QG[i];
            PART[o + 6] = QB[i];
            PART[o + 7] = f * f;
            PART_N += 1;
        }
        push_map(WX[0], WZ[0], 0.0, 1.6);

        // ---- state block ----
        let g = height_at(WX[0], WZ[0]);
        ST[0] = WX[0];
        ST[1] = WY[0];
        ST[2] = WZ[0];
        ST[3] = WYAW[0];
        ST[4] = WPIT[0];
        ST[5] = WROL[0];
        ST[6] = sqrt(WVX[0] * WVX[0] + WVY[0] * WVY[0] + WVZ[0] * WVZ[0]);
        ST[7] = WHP[0];
        ST[8] = 100.0;
        ST[9] = WMANA[0];
        ST[10] = mana_cap(0);
        ST[11] = SEL as f32;
        ST[12] = NSPELL as f32;
        let tgt = level_target();
        ST[13] = WSTORE[0] / tgt;
        let mut rb: f32 = 0.0;
        for w in 1..=N_RIVAL {
            if WSTORE[w] > rb {
                rb = WSTORE[w];
            }
        }
        ST[14] = rb / tgt;
        ST[15] = 1.0;
        // 53..56: the gap between the cooldown block (40..52) and the unlocks
        ST[53] = castle_cap(0) / tgt; // how much of the target this tier holds
        ST[54] = WSTORE[0];
        ST[55] = castle_cap(0);
        ST[56] = tgt;
        ST[57] = BANK_FLOOR;
        ST[16] = CLEV[0] as f32;
        ST[17] = LEVEL as f32;
        ST[18] = STATUS as f32;
        ST[19] = SHAKE;
        ST[20] = WSHIELD[0];
        ST[21] = WHASTE[0];
        ST[22] = g;
        ST[23] = WY[0] - fmax(g, SEA);
        ST[24] = KILLS;
        ST[25] = WSTORE[0];
        ST[26] = TOTAL_MANA;
        ST[27] = CHP[0] / CHPM[0];
        ST[28] = if N_RIVAL >= 1 { WHP[1] / 100.0 } else { 0.0 };
        let rdx = WX[1] - WX[0];
        let rdz = WZ[1] - WZ[0];
        ST[29] = if WALIVE[1] != 0 {
            sqrt(rdx * rdx + rdz * rdz)
        } else {
            -1.0
        };
        ST[30] = orb_n as f32;
        ST[31] = 0.0;
        ST[32] = GTIME;
        ST[33] = CX[0];
        ST[34] = CZ[0];
        ST[35] = CY[0];
        ST[36] = N_RIVAL as f32;
        ST[37] = TW as f32;
        ST[38] = CELL;
        for i in 0..NSPELL {
            ST[40 + i] = if SCD[i] > 0.0 { CD[i] / SCD[i] } else { 0.0 };
            ST[60 + i] = UNL[i] as f32;
            let price: f32 = if i == 12 { fortress_cost(0) } else { SCOST[i] };
            ST[80 + i] = if WMANA[0] >= price { 1.0 } else { 0.0 };
            ST[100 + i] = price;
        }
    }
}

// ============================================================================
// Renderer support — the parts of the draw path that are pure arithmetic and
// so have no business being in JavaScript. Camera matrices, the three mesh
// prototypes and the minimap raster all live here; what stays in engine.js is
// the WebGPU API surface itself, which wasm cannot reach without imports.
// ============================================================================

static mut CAMM: [f32; 32] = [0.0; 32]; // view-projection, then its inverse

// column-major, o[i + j*4]
fn mat_mul(a: &[f32; 16], b: &[f32; 16], o: &mut [f32; 16]) {
    for j in 0..4 {
        for i in 0..4 {
            let mut s = 0.0f32;
            for k in 0..4 {
                s += a[i + k * 4] * b[k + j * 4];
            }
            o[i + j * 4] = s;
        }
    }
}
fn mat_invert(m: &[f32; 16], o: &mut [f32; 16]) {
    let (a00, a01, a02, a03) = (m[0], m[1], m[2], m[3]);
    let (a10, a11, a12, a13) = (m[4], m[5], m[6], m[7]);
    let (a20, a21, a22, a23) = (m[8], m[9], m[10], m[11]);
    let (a30, a31, a32, a33) = (m[12], m[13], m[14], m[15]);
    let b00 = a00 * a11 - a01 * a10;
    let b01 = a00 * a12 - a02 * a10;
    let b02 = a00 * a13 - a03 * a10;
    let b03 = a01 * a12 - a02 * a11;
    let b04 = a01 * a13 - a03 * a11;
    let b05 = a02 * a13 - a03 * a12;
    let b06 = a20 * a31 - a21 * a30;
    let b07 = a20 * a32 - a22 * a30;
    let b08 = a20 * a33 - a23 * a30;
    let b09 = a21 * a32 - a22 * a31;
    let b10 = a21 * a33 - a23 * a31;
    let b11 = a22 * a33 - a23 * a32;
    let mut det = b00 * b11 - b01 * b10 + b02 * b09 + b03 * b08 - b04 * b07 + b05 * b06;
    if det == 0.0 {
        for v in o.iter_mut() {
            *v = 0.0;
        }
        return;
    }
    det = 1.0 / det;
    o[0] = (a11 * b11 - a12 * b10 + a13 * b09) * det;
    o[1] = (a02 * b10 - a01 * b11 - a03 * b09) * det;
    o[2] = (a31 * b05 - a32 * b04 + a33 * b03) * det;
    o[3] = (a22 * b04 - a21 * b05 - a23 * b03) * det;
    o[4] = (a12 * b08 - a10 * b11 - a13 * b07) * det;
    o[5] = (a00 * b11 - a02 * b08 + a03 * b07) * det;
    o[6] = (a32 * b02 - a30 * b05 - a33 * b01) * det;
    o[7] = (a20 * b05 - a22 * b02 + a23 * b01) * det;
    o[8] = (a10 * b10 - a11 * b08 + a13 * b06) * det;
    o[9] = (a01 * b08 - a00 * b10 - a03 * b06) * det;
    o[10] = (a30 * b04 - a31 * b02 + a33 * b00) * det;
    o[11] = (a21 * b02 - a20 * b04 - a23 * b00) * det;
    o[12] = (a11 * b07 - a10 * b09 - a12 * b06) * det;
    o[13] = (a00 * b09 - a01 * b07 + a02 * b06) * det;
    o[14] = (a31 * b01 - a30 * b03 - a32 * b00) * det;
    o[15] = (a20 * b03 - a21 * b01 + a22 * b00) * det;
}

// ---- mesh prototypes -------------------------------------------------------
// All three live in a -1..1 box; the vertex shader scales by iscl*0.5, so an
// instance scale of s spans exactly s world units. Interleaved pos+normal.
const MESHV_CAP: usize = 1024 * 6;
const MESHI_CAP: usize = 1024;
static mut MESHV: [[f32; MESHV_CAP]; 3] = [[0.0; MESHV_CAP]; 3];
static mut MESHI: [[u16; MESHI_CAP]; 3] = [[0; MESHI_CAP]; 3];
static mut MESHVN: [usize; 3] = [0; 3]; // floats written
static mut MESHIN: [usize; 3] = [0; 3]; // indices written

struct MeshBuf {
    shape: usize,
    nv: usize, // vertices pushed (not floats)
}
impl MeshBuf {
    fn vert(&mut self, p: [f32; 3], n: [f32; 3]) {
        unsafe {
            let o = self.nv * 6;
            if o + 6 > MESHV_CAP {
                return;
            }
            MESHV[self.shape][o] = p[0];
            MESHV[self.shape][o + 1] = p[1];
            MESHV[self.shape][o + 2] = p[2];
            MESHV[self.shape][o + 3] = n[0];
            MESHV[self.shape][o + 4] = n[1];
            MESHV[self.shape][o + 5] = n[2];
            self.nv += 1;
            MESHVN[self.shape] = self.nv * 6;
        }
    }
    fn idx(&mut self, a: usize, b: usize, c: usize) {
        unsafe {
            let n = MESHIN[self.shape];
            if n + 3 > MESHI_CAP {
                return;
            }
            MESHI[self.shape][n] = a as u16;
            MESHI[self.shape][n + 1] = b as u16;
            MESHI[self.shape][n + 2] = c as u16;
            MESHIN[self.shape] = n + 3;
        }
    }
}

fn build_meshes() {
    unsafe {
        for s in 0..3 {
            MESHVN[s] = 0;
            MESHIN[s] = 0;
        }
    }
    // --- box
    {
        let mut m = MeshBuf { shape: 0, nv: 0 };
        const FACES: [([f32; 3], [[f32; 3]; 4]); 6] = [
            ([1.0, 0.0, 0.0], [[1.0, -1.0, -1.0], [1.0, 1.0, -1.0], [1.0, 1.0, 1.0], [1.0, -1.0, 1.0]]),
            ([-1.0, 0.0, 0.0], [[-1.0, -1.0, 1.0], [-1.0, 1.0, 1.0], [-1.0, 1.0, -1.0], [-1.0, -1.0, -1.0]]),
            ([0.0, 1.0, 0.0], [[-1.0, 1.0, -1.0], [-1.0, 1.0, 1.0], [1.0, 1.0, 1.0], [1.0, 1.0, -1.0]]),
            ([0.0, -1.0, 0.0], [[-1.0, -1.0, 1.0], [-1.0, -1.0, -1.0], [1.0, -1.0, -1.0], [1.0, -1.0, 1.0]]),
            ([0.0, 0.0, 1.0], [[-1.0, -1.0, 1.0], [1.0, -1.0, 1.0], [1.0, 1.0, 1.0], [-1.0, 1.0, 1.0]]),
            ([0.0, 0.0, -1.0], [[-1.0, 1.0, -1.0], [1.0, 1.0, -1.0], [1.0, -1.0, -1.0], [-1.0, -1.0, -1.0]]),
        ];
        for (nv, quad) in FACES.iter() {
            let base = m.nv;
            for v in quad.iter() {
                m.vert(*v, *nv);
            }
            m.idx(base, base + 1, base + 2);
            m.idx(base, base + 2, base + 3);
        }
    }
    // --- sphere. Wound so the outward face is front-facing under cullMode
    // 'back', same as the box; getting this backwards renders it inside out.
    {
        let mut m = MeshBuf { shape: 1, nv: 0 };
        let seg = 14usize;
        let ring = 9usize;
        for j in 0..=ring {
            let phi = j as f32 / ring as f32 * 3.14159265;
            for i in 0..=seg {
                let th = i as f32 / seg as f32 * 6.28318531;
                let x = sin(phi) * cos(th);
                let y = cos(phi);
                let z = sin(phi) * sin(th);
                m.vert([x, y, z], [x, y, z]);
            }
        }
        for j in 0..ring {
            for i in 0..seg {
                let a = j * (seg + 1) + i;
                let b = a + seg + 1;
                m.idx(a, a + 1, b);
                m.idx(a + 1, b + 1, b);
            }
        }
    }
    // --- cone
    {
        let mut m = MeshBuf { shape: 2, nv: 0 };
        let seg = 14usize;
        for i in 0..seg {
            let a0 = i as f32 / seg as f32 * 6.28318531;
            let a1 = (i + 1) as f32 / seg as f32 * 6.28318531;
            let (x0, z0) = (cos(a0), sin(a0));
            let (x1, z1) = (cos(a1), sin(a1));
            let mx = cos((a0 + a1) * 0.5);
            let mz = sin((a0 + a1) * 0.5);
            let ny = 0.45f32;
            let s = sqrt(mx * mx + ny * ny + mz * mz);
            let side = [mx / s, ny / s, mz / s];
            let base = m.nv;
            m.vert([x0, -1.0, z0], side);
            m.vert([x1, -1.0, z1], side);
            m.vert([0.0, 1.0, 0.0], side);
            m.idx(base, base + 2, base + 1);
            let b2 = m.nv;
            let down = [0.0, -1.0, 0.0];
            m.vert([0.0, -1.0, 0.0], down);
            m.vert([x1, -1.0, z1], down);
            m.vert([x0, -1.0, z0], down);
            m.idx(b2, b2 + 2, b2 + 1);
        }
    }
}

// ---- minimap raster --------------------------------------------------------
const MINIMAP_MAX: usize = 160;
static mut MINIMAP: [u8; MINIMAP_MAX * MINIMAP_MAX * 4] = [0; MINIMAP_MAX * MINIMAP_MAX * 4];

// ============================================================================
// ABI — flat and C-like on purpose. game.js calls these by name and reads the
// buffers straight out of linear memory; nothing is marshalled.
// ============================================================================

/// Builds the view-projection matrix and its inverse. Returns a pointer to 32
/// f32: vp in 0..16, ivp in 16..32.
#[no_mangle]
pub extern "C" fn camera(
    fovy: f32,
    aspect: f32,
    near: f32,
    far: f32,
    ex: f32,
    ey: f32,
    ez: f32,
    cx: f32,
    cy: f32,
    cz: f32,
    ux: f32,
    uy: f32,
    uz: f32,
) -> usize {
    unsafe {
        // perspective with a 0..1 depth range (WebGPU/D3D convention)
        let f = 1.0 / libm::tanf(fovy * 0.5);
        let nf = 1.0 / (near - far);
        let proj: [f32; 16] = [
            f / aspect, 0.0, 0.0, 0.0,
            0.0, f, 0.0, 0.0,
            0.0, 0.0, far * nf, -1.0,
            0.0, 0.0, far * near * nf, 0.0,
        ];
        // look-at
        let (mut zx, mut zy, mut zz) = (ex - cx, ey - cy, ez - cz);
        let mut l = sqrt(zx * zx + zy * zy + zz * zz);
        if l == 0.0 {
            l = 1.0;
        }
        zx /= l;
        zy /= l;
        zz /= l;
        let (mut xx, mut xy, mut xz) = (uy * zz - uz * zy, uz * zx - ux * zz, ux * zy - uy * zx);
        l = sqrt(xx * xx + xy * xy + xz * xz);
        if l == 0.0 {
            l = 1.0;
        }
        xx /= l;
        xy /= l;
        xz /= l;
        let yx = zy * xz - zz * xy;
        let yy = zz * xx - zx * xz;
        let yz = zx * xy - zy * xx;
        let view: [f32; 16] = [
            xx, yx, zx, 0.0,
            xy, yy, zy, 0.0,
            xz, yz, zz, 0.0,
            -(xx * ex + xy * ey + xz * ez),
            -(yx * ex + yy * ey + yz * ez),
            -(zx * ex + zy * ey + zz * ez),
            1.0,
        ];
        let mut vp = [0.0f32; 16];
        mat_mul(&proj, &view, &mut vp);
        let mut ivp = [0.0f32; 16];
        mat_invert(&vp, &mut ivp);
        CAMM[..16].copy_from_slice(&vp);
        CAMM[16..].copy_from_slice(&ivp);
        CAMM.as_ptr() as usize
    }
}

#[no_mangle]
pub extern "C" fn buildMeshes() {
    build_meshes();
}
#[no_mangle]
pub extern "C" fn meshVertPtr(s: i32) -> usize {
    unsafe { MESHV[s as usize].as_ptr() as usize }
}
#[no_mangle]
pub extern "C" fn meshVertFloats(s: i32) -> i32 {
    unsafe { MESHVN[s as usize] as i32 }
}
#[no_mangle]
pub extern "C" fn meshIdxPtr(s: i32) -> usize {
    unsafe { MESHI[s as usize].as_ptr() as usize }
}
#[no_mangle]
pub extern "C" fn meshIdxCount(s: i32) -> i32 {
    unsafe { MESHIN[s as usize] as i32 }
}

/// Rasterises the heightmap into an n x n RGBA image for the minimap.
/// Returns a pointer JS wraps in a Uint8ClampedArray for putImageData.
#[no_mangle]
pub extern "C" fn minimapRaster(n: i32) -> usize {
    unsafe {
        let n = if n < 1 {
            1
        } else if n as usize > MINIMAP_MAX {
            MINIMAP_MAX as i32
        } else {
            n
        };
        let nu = n as usize;
        let step = TW as f32 / n as f32;
        for j in 0..nu {
            for i in 0..nu {
                let si = (i as f32 * step) as i32;
                let sj = (j as f32 * step) as i32;
                let h = h_get(si, sj);
                let (r, g, b): (f32, f32, f32) = if h < -1.0 {
                    let t = fmin(1.0, -h / 30.0);
                    (14.0 + (1.0 - t) * 26.0, 34.0 + (1.0 - t) * 50.0, 62.0 + (1.0 - t) * 44.0)
                } else if h < 8.0 {
                    (178.0, 152.0, 98.0)
                } else if h < 52.0 {
                    (74.0 + h * 0.7, 96.0 + h * 0.5, 50.0)
                } else if h < 118.0 {
                    (106.0, 96.0, 84.0)
                } else {
                    (210.0, 205.0, 193.0)
                };
                let o = (j * nu + i) * 4;
                MINIMAP[o] = r as u8;
                MINIMAP[o + 1] = g as u8;
                MINIMAP[o + 2] = b as u8;
                MINIMAP[o + 3] = 255;
            }
        }
        MINIMAP.as_ptr() as usize
    }
}

#[no_mangle]
pub extern "C" fn init(seed: u32, lv: i32) {
    unsafe {
        srand(seed);
        LEVEL = lv;
        STATUS = 0;
        GTIME = 0.0;
        SHAKE = 0.0;
        KILLS = 0.0;
        init_spell_table();
        unlock_for(lv);
        N_RIVAL = if lv >= 5 { 2 } else { 1 };
        for i in 0..MAXE {
            EALIVE[i] = 0;
        }
        for i in 0..MAXP {
            PALIVE[i] = 0;
        }
        for i in 0..MAXO {
            OALIVE[i] = 0;
        }
        for i in 0..MAXPT {
            QL[i] = 0.0;
        }
        for i in 0..NSPELL {
            CD[i] = 0.0;
        }
        gen_terrain();
        place_scenery();
        TOTAL_MANA = 420.0 + lv as f32 * 90.0;
        TARGET_FRAC = 0.50;
        SEL = 0;
        I_FWD = 0.0;
        I_STR = 0.0;
        I_UP = 0.0;

        // castles: one per wizard, on distinct high ground
        for w in 0..NWIZ {
            WALIVE[w] = 0;
        }
        for w in 0..=N_RIVAL {
            let mut idx = find_land(24.0);
            // push rivals away from the player
            for _ in 0..200 {
                idx = find_land(24.0);
                let tx = (idx % TW) as f32 * CELL;
                let tz = (idx / TW) as f32 * CELL;
                let mut ok = true;
                for u in 0..w {
                    let dd = (tx - CX[u]) * (tx - CX[u]) + (tz - CZ[u]) * (tz - CZ[u]);
                    if dd < 820.0 * 820.0 {
                        ok = false;
                        break;
                    }
                }
                if ok {
                    break;
                }
            }
            let bx = (idx % TW) as f32 * CELL;
            let bz = (idx / TW) as f32 * CELL;
            // level the castle pad
            deform(bx, bz, 30.0, 0.0, 2);
            let pad_h = height_at(bx, bz);
            // Cut a real shelf: dead flat right out past the keep's footprint,
            // then a smooth skirt down to the original hill. A 1-d^2 falloff is
            // only fully flat at the exact centre, so on a steep peak the castle
            // still has slope under its edges and reads as hanging off the
            // mountain.
            let gi = (bx / CELL) as i32;
            let gj = (bz / CELL) as i32;
            const PAD: i32 = 13; // cells; 13 * CELL = 52 units
            for j in (gj - PAD)..=(gj + PAD) {
                for i in (gi - PAD)..=(gi + PAD) {
                    let d: f32 =
                        sqrt(((i - gi) * (i - gi) + (j - gj) * (j - gj)) as f32) / PAD as f32;
                    if d > 1.0 {
                        continue;
                    }
                    // flat well past the plinth (FOOT=27) so ground wraps its
                    // base and you can never see under the terrace edge
                    let mut t: f32 = (d - 0.72) / 0.28;
                    if t < 0.0 {
                        t = 0.0;
                    }
                    if t > 1.0 {
                        t = 1.0;
                    }
                    let w2: f32 = 1.0 - t * t * (3.0 - 2.0 * t);
                    let cur = h_get(i, j);
                    h_set(i, j, cur + (pad_h - cur) * w2);
                }
            }
            CX[w] = bx;
            CZ[w] = bz;
            CY[w] = height_at(bx, bz);
            CHPM[w] = 600.0 + lv as f32 * 60.0;
            CHP[w] = CHPM[w];
            CLEV[w] = 1;
            CBT[w] = 3.0;
            WALIVE[w] = 1;
            WX[w] = bx;
            WZ[w] = bz + 40.0;
            WY[w] = CY[w] + 40.0;
            WVX[w] = 0.0;
            WVY[w] = 0.0;
            WVZ[w] = 0.0;
            WYAW[w] = 0.0;
            WPIT[w] = 0.0;
            WROL[w] = 0.0;
            WHP[w] = 100.0;
            WCAR[w] = 0.0;
            WSTORE[w] = 0.0;
            WMANA[w] = if w == 0 { 45.0 } else { 60.0 };
            WCAP[w] = 0.0;
            WCD[w] = 0.0;
            WRESP[w] = 0.0;
            WSHIELD[w] = 0.0;
            WHASTE[w] = 0.0;
            WAI[w] = 0;
            WAITIM[w] = 0.0;
            WNOHIT[w] = 0.0;
        }

        // Creature nests + starting wildlife. Counts scale with the map, which
        // is 6x the area of the original — the old numbers left everything
        // piled in one corner.
        let n_nests = 9 + lv * 2;
        for _ in 0..n_nests {
            let idx = find_land(14.0);
            add_creature(
                4,
                (idx % TW) as f32 * CELL,
                (idx / TW) as f32 * CELL,
                0,
            );
        }
        let n_wild = 24 + lv * 6;
        for _ in 0..n_wild {
            let idx = find_land(6.0);
            let mut t = ri(4);
            if t == 4 {
                t = 0;
            }
            if lv >= 6 && ri(14) == 0 {
                t = 7;
            }
            add_creature(t, (idx % TW) as f32 * CELL, (idx / TW) as f32 * CELL, 0);
        }
        // seed some free mana so the map is immediately playable
        for _ in 0..96 {
            let idx = find_land(4.0);
            let axx = (idx % TW) as f32 * CELL;
            let azz = (idx / TW) as f32 * CELL;
            add_orb(axx, height_at(axx, azz) + 4.0, azz, rr(5.0, 10.0));
        }
        SPAWN_TIMER = 4.0;
        clearDirty();
        DIRTY_LO = 0;
        DIRTY_HI = TWM;
        build_render();
    }
}

#[no_mangle]
pub extern "C" fn step(dt: f32) {
    unsafe {
        EVT_N = 0;
        let mut d = dt;
        if d > 0.05 {
            d = 0.05;
        }
        if STATUS == 0 {
            GTIME += d;
            for i in 0..NSPELL {
                if CD[i] > 0.0 {
                    CD[i] -= d;
                }
            }
            update_player(d);
            for w in 1..=N_RIVAL {
                update_rival(w, d);
            }
            for w in 0..=N_RIVAL {
                if WALIVE[w] != 0 {
                    wizard_common(w, d);
                }
            }
            update_creatures(d);
            update_projectiles(d);
            update_orbs(d);
            // trickle of new wildlife so the realm never empties
            SPAWN_TIMER -= d;
            if SPAWN_TIMER <= 0.0 {
                SPAWN_TIMER = 9.0;
                let mut n = 0;
                for i in 0..MAXE {
                    if EALIVE[i] != 0 && EOWN[i] == 0 && ETYPE[i] != 4 {
                        n += 1;
                    }
                }
                if n < 7 + LEVEL {
                    let idx = find_land(6.0);
                    let t = if ri(4) == 3 { 3 } else { ri(3) };
                    add_creature(t, (idx % TW) as f32 * CELL, (idx / TW) as f32 * CELL, 0);
                }
            }
            // win / lose
            if WSTORE[0] >= level_target() {
                STATUS = 1;
            }
            for w in 1..=N_RIVAL {
                if WSTORE[w] >= level_target() {
                    STATUS = 3;
                }
            }
            if WHP[0] <= 0.0 {
                STATUS = 2;
            }
        }
        update_particles(d);
        if SHAKE > 0.0 {
            SHAKE -= d * 1.8;
            if SHAKE < 0.0 {
                SHAKE = 0.0;
            }
        }
        build_render();
    }
}

#[no_mangle]
pub extern "C" fn setInput(fwd: f32, str_: f32, up: f32, dyaw: f32, dpit: f32, fire: i32, brake: i32) {
    unsafe {
        I_FWD = fwd;
        I_STR = str_;
        I_UP = up;
        I_YAW = dyaw;
        I_PIT = dpit;
        I_FIRE = fire;
        I_BRAKE = brake;
    }
}
#[no_mangle]
pub extern "C" fn cast(s: i32) {
    unsafe {
        if STATUS == 0 {
            cast_spell(0, s);
        }
    }
}
#[no_mangle]
pub extern "C" fn fireSelected() {
    unsafe {
        if STATUS == 0 {
            cast_spell(0, SEL);
        }
    }
}
#[no_mangle]
pub extern "C" fn selectSpell(s: i32) {
    unsafe {
        if s >= 0 && s < NSPELL as i32 && UNL[s as usize] == 1 {
            SEL = s;
        }
    }
}
#[no_mangle]
pub extern "C" fn cycleSpell(dir: i32) {
    unsafe {
        for _ in 0..NSPELL {
            SEL = (SEL + dir + NSPELL as i32) % NSPELL as i32;
            if UNL[SEL as usize] == 1 {
                return;
            }
        }
    }
}

// ---- pointers into linear memory -------------------------------------------
#[no_mangle]
pub extern "C" fn heightPtr() -> usize {
    unsafe { HEIGHT.as_ptr() as usize }
}
#[no_mangle]
pub extern "C" fn instPtr() -> usize {
    unsafe { INST.as_ptr() as usize }
}
#[no_mangle]
pub extern "C" fn partPtr() -> usize {
    unsafe { PART.as_ptr() as usize }
}
#[no_mangle]
pub extern "C" fn mapPtr() -> usize {
    unsafe { MAP.as_ptr() as usize }
}
#[no_mangle]
pub extern "C" fn evtPtr() -> usize {
    unsafe { EVT.as_ptr() as usize }
}
#[no_mangle]
pub extern "C" fn statePtr() -> usize {
    unsafe { ST.as_ptr() as usize }
}
#[no_mangle]
pub extern "C" fn instCount() -> i32 {
    unsafe { INST_N as i32 }
}
#[no_mangle]
pub extern "C" fn carpetInstLo() -> i32 {
    unsafe { CARPET_LO }
}
#[no_mangle]
pub extern "C" fn carpetInstHi() -> i32 {
    unsafe { CARPET_HI }
}
#[no_mangle]
pub extern "C" fn partCount() -> i32 {
    unsafe { PART_N as i32 }
}
#[no_mangle]
pub extern "C" fn mapCount() -> i32 {
    unsafe { MAP_N as i32 }
}
#[no_mangle]
pub extern "C" fn evtCount() -> i32 {
    unsafe { EVT_N as i32 }
}
#[no_mangle]
pub extern "C" fn dirtyLoRow() -> i32 {
    unsafe { DIRTY_LO }
}
#[no_mangle]
pub extern "C" fn dirtyHiRow() -> i32 {
    unsafe { DIRTY_HI }
}
#[no_mangle]
pub extern "C" fn clearDirty() {
    unsafe {
        DIRTY_LO = TW;
        DIRTY_HI = -1;
    }
}
#[no_mangle]
pub extern "C" fn terrainWidth() -> i32 {
    TW
}
#[no_mangle]
pub extern "C" fn cellSize() -> f32 {
    CELL
}
#[no_mangle]
pub extern "C" fn worldSize() -> f32 {
    WORLD
}
