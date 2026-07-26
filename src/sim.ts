// ============================================================================
// AETHERLOOM - simulation core. Compiles to freestanding WebAssembly.
// Zero imports. All state lives in linear memory as flat f32/i32 arrays which
// JS maps directly as typed-array views (no marshalling, no copies).
// ============================================================================

// ---- world constants -------------------------------------------------------
export const TW: i32 = 320;             // terrain grid width (320*4B = 1280B rows, GPU-aligned)
const TWM: i32 = TW - 1;
export const CELL: f32 = 8.0;           // world units per cell
const WORLD: f32 = <f32>TW * CELL;      // 2560 units — 6x the area of the old map
const SEA: f32 = 0.0;

const MAXE: i32 = 460;                  // creatures
const MAXP: i32 = 128;                  // projectiles
const MAXO: i32 = 768;                  // mana orbs
const MAXPT: i32 = 4096;                // particles
const MAXI: i32 = 12288;                // render instances
const MAXDEC: i32 = 5200;               // scenery
const NWIZ: i32 = 3;                    // 1 player + up to 2 rivals
const NSPELL: i32 = 13;

// ---- terrain ---------------------------------------------------------------
const HEIGHT = new StaticArray<f32>(TW * TW);
let dirtyLo: i32 = 0, dirtyHi: i32 = -1;   // dirty row band for GPU upload

// ---- creatures (structure of arrays) --------------------------------------
const ex = new StaticArray<f32>(MAXE), ey = new StaticArray<f32>(MAXE), ez = new StaticArray<f32>(MAXE);
const evx = new StaticArray<f32>(MAXE), evy = new StaticArray<f32>(MAXE), evz = new StaticArray<f32>(MAXE);
const ehp = new StaticArray<f32>(MAXE), ehpm = new StaticArray<f32>(MAXE);
const etype = new StaticArray<i32>(MAXE), estate = new StaticArray<i32>(MAXE), eown = new StaticArray<i32>(MAXE);
const etim = new StaticArray<f32>(MAXE), ecd = new StaticArray<f32>(MAXE), eyaw = new StaticArray<f32>(MAXE);
const ealive = new StaticArray<i32>(MAXE), ephase = new StaticArray<f32>(MAXE), ehurt = new StaticArray<f32>(MAXE);

// ---- projectiles -----------------------------------------------------------
const px_ = new StaticArray<f32>(MAXP), py_ = new StaticArray<f32>(MAXP), pz_ = new StaticArray<f32>(MAXP);
const pvx = new StaticArray<f32>(MAXP), pvy = new StaticArray<f32>(MAXP), pvz = new StaticArray<f32>(MAXP);
const plife = new StaticArray<f32>(MAXP), pdmg = new StaticArray<f32>(MAXP), prad = new StaticArray<f32>(MAXP);
const ptype = new StaticArray<i32>(MAXP), pown = new StaticArray<i32>(MAXP), palive = new StaticArray<i32>(MAXP);

// ---- mana orbs -------------------------------------------------------------
const ox = new StaticArray<f32>(MAXO), oy = new StaticArray<f32>(MAXO), oz = new StaticArray<f32>(MAXO);
const oamt = new StaticArray<f32>(MAXO), oalive = new StaticArray<i32>(MAXO), ophase = new StaticArray<f32>(MAXO);
const oheld = new StaticArray<i32>(MAXO); // -1 free, else balloon entity index
// 0 = unclaimed (gold), 1 = claimed by the player, 2 = claimed by a rival.
// Only orbs matching a balloon's faction get ferried home.
const oown = new StaticArray<i32>(MAXO);
// personal mana pool bonus earned by possessing orbs
const wcap = new StaticArray<f32>(NWIZ);

// ---- particles -------------------------------------------------------------
const qx = new StaticArray<f32>(MAXPT), qy = new StaticArray<f32>(MAXPT), qz = new StaticArray<f32>(MAXPT);
const qvx = new StaticArray<f32>(MAXPT), qvy = new StaticArray<f32>(MAXPT), qvz = new StaticArray<f32>(MAXPT);
const ql = new StaticArray<f32>(MAXPT), qlm = new StaticArray<f32>(MAXPT), qs = new StaticArray<f32>(MAXPT);
const qr = new StaticArray<f32>(MAXPT), qg = new StaticArray<f32>(MAXPT), qb = new StaticArray<f32>(MAXPT);
const qdrag = new StaticArray<f32>(MAXPT), qgrav = new StaticArray<f32>(MAXPT);
let qhead: i32 = 0;

// ---- wizards (0 = player) --------------------------------------------------
const wx = new StaticArray<f32>(NWIZ), wy = new StaticArray<f32>(NWIZ), wz = new StaticArray<f32>(NWIZ);
const wvx = new StaticArray<f32>(NWIZ), wvy = new StaticArray<f32>(NWIZ), wvz = new StaticArray<f32>(NWIZ);
const wyaw = new StaticArray<f32>(NWIZ), wpit = new StaticArray<f32>(NWIZ), wrol = new StaticArray<f32>(NWIZ);
const whp = new StaticArray<f32>(NWIZ), wcar = new StaticArray<f32>(NWIZ), wstore = new StaticArray<f32>(NWIZ);
const wmana = new StaticArray<f32>(NWIZ), wcd = new StaticArray<f32>(NWIZ), walive = new StaticArray<i32>(NWIZ);
const wresp = new StaticArray<f32>(NWIZ), wshield = new StaticArray<f32>(NWIZ), whaste = new StaticArray<f32>(NWIZ);
const wai = new StaticArray<i32>(NWIZ), waitim = new StaticArray<f32>(NWIZ);
const wnohit = new StaticArray<f32>(NWIZ);
// castles
const cx = new StaticArray<f32>(NWIZ), cz = new StaticArray<f32>(NWIZ), cy = new StaticArray<f32>(NWIZ);
const chp = new StaticArray<f32>(NWIZ), chpm = new StaticArray<f32>(NWIZ), clev = new StaticArray<i32>(NWIZ);
const cbt = new StaticArray<f32>(NWIZ); // balloon spawn timer

// ---- scenery ---------------------------------------------------------------
const dx_ = new StaticArray<f32>(MAXDEC), dz_ = new StaticArray<f32>(MAXDEC);
const dkind = new StaticArray<i32>(MAXDEC), dsc = new StaticArray<f32>(MAXDEC), drot = new StaticArray<f32>(MAXDEC);
let decCount: i32 = 0;

// ---- render output buffers -------------------------------------------------
// instance stride 12: px,py,pz, sx,sy,sz, r,g,b, yaw, glow, shape
const INST = new StaticArray<f32>(MAXI * 12);
let instN: i32 = 0;
// index range of the player's own carpet within INST, for first-person culling
let carpetLo: i32 = 0, carpetHi: i32 = 0;
// particle stride 8: x,y,z, size, r,g,b, alpha
const PART = new StaticArray<f32>(MAXPT * 8);
let partN: i32 = 0;
// minimap blip stride 4: x,z,kind,scale
const MAXMAP: i32 = 2048;
const MAP = new StaticArray<f32>(MAXMAP * 4);
let mapN: i32 = 0;
// audio/vfx events stride 4: kind,x,y,z
const EVT = new StaticArray<f32>(128 * 4);
let evtN: i32 = 0;
// shared state block read by HUD
const ST = new StaticArray<f32>(128);
const CD = new StaticArray<f32>(NSPELL);   // cooldown remaining
const UNL = new StaticArray<i32>(NSPELL);  // unlocked

// ---- input -----------------------------------------------------------------
let iFwd: f32 = 0, iStr: f32 = 0, iUp: f32 = 0, iYaw: f32 = 0, iPit: f32 = 0;
let iFire: i32 = 0, iBrake: i32 = 0;
let sel: i32 = 0;
let level: i32 = 1, status: i32 = 0;
let totalMana: f32 = 1, targetFrac: f32 = 0.5;
let gtime: f32 = 0, shake: f32 = 0, kills: f32 = 0;
let nRival: i32 = 1;
let spawnTimer: f32 = 0;

// ---- rng -------------------------------------------------------------------
let rs: u32 = 0x9e3779b9;
function srand(s: u32): void { rs = s | 1; }
function rnd(): f32 { rs ^= rs << 13; rs ^= rs >> 17; rs ^= rs << 5; return <f32>(rs & 0xffffff) / 16777216.0; }
function rr(a: f32, b: f32): f32 { return a + (b - a) * rnd(); }
function ri(n: i32): i32 { return <i32>(rnd() * <f32>n) % n; }

// ---- value noise + fbm -----------------------------------------------------
@inline function hash2(x: i32, y: i32): f32 {
  let h: u32 = <u32>(x * 374761393 + y * 668265263 + <i32>rs);
  h = (h ^ (h >> 13)) * 1274126177; h ^= h >> 16;
  return <f32>(h & 0xffff) / 32768.0 - 1.0;
}
function vnoise(x: f32, y: f32): f32 {
  const xi = <i32>Mathf.floor(x), yi = <i32>Mathf.floor(y);
  const fx = x - <f32>xi, fy = y - <f32>yi;
  const ux = fx * fx * (3.0 - 2.0 * fx), uy = fy * fy * (3.0 - 2.0 * fy);
  const a = hash2(xi, yi), b = hash2(xi + 1, yi), c = hash2(xi, yi + 1), d = hash2(xi + 1, yi + 1);
  return (a + (b - a) * ux) + ((c + (d - c) * ux) - (a + (b - a) * ux)) * uy;
}
function fbm(x: f32, y: f32, oct: i32): f32 {
  let s: f32 = 0, a: f32 = 0.5, f: f32 = 1.0;
  for (let i = 0; i < oct; i++) { s += a * vnoise(x * f, y * f); f *= 2.03; a *= 0.5; }
  return s;
}

// ---- height access ---------------------------------------------------------
@inline export function hGet(i: i32, j: i32): f32 {
  if (i < 0) i = 0; if (i > TWM) i = TWM; if (j < 0) j = 0; if (j > TWM) j = TWM;
  return HEIGHT[j * TW + i];
}
@inline function hSet(i: i32, j: i32, v: f32): void {
  if (i < 0 || i > TWM || j < 0 || j > TWM) return;
  HEIGHT[j * TW + i] = v;
  if (j < dirtyLo) dirtyLo = j; if (j > dirtyHi) dirtyHi = j;
}
// bilinear world-space height
export function heightAt(x: f32, z: f32): f32 {
  const gx = x / CELL, gz = z / CELL;
  const i = <i32>Mathf.floor(gx), j = <i32>Mathf.floor(gz);
  const fx = gx - <f32>i, fz = gz - <f32>j;
  const h00 = hGet(i, j), h10 = hGet(i + 1, j), h01 = hGet(i, j + 1), h11 = hGet(i + 1, j + 1);
  const a = h00 + (h10 - h00) * fx, b = h01 + (h11 - h01) * fx;
  return a + (b - a) * fz;
}
@inline function clampWorld(v: f32): f32 { return v < 8.0 ? 8.0 : (v > WORLD - 8.0 ? WORLD - 8.0 : v); }

// ---- terrain deformation ---------------------------------------------------
// mode 0 = crater (subtract), 1 = raise cone, 2 = smooth ripple
function deform(wx0: f32, wz0: f32, radius: f32, amount: f32, mode: i32): void {
  const gi = wx0 / CELL, gj = wz0 / CELL, gr = radius / CELL;
  const i0 = <i32>Mathf.floor(gi - gr) - 1, i1 = <i32>Mathf.ceil(gi + gr) + 1;
  const j0 = <i32>Mathf.floor(gj - gr) - 1, j1 = <i32>Mathf.ceil(gj + gr) + 1;
  for (let j = j0; j <= j1; j++) {
    for (let i = i0; i <= i1; i++) {
      const dx = <f32>i - gi, dz = <f32>j - gj;
      const d = Mathf.sqrt(dx * dx + dz * dz) / gr;
      if (d > 1.0) continue;
      let w: f32;
      if (mode == 2) { w = Mathf.cos(d * 3.14159) * (1.0 - d); }
      else { w = 1.0 - d * d; w = w * w; }
      const cur = hGet(i, j);
      if (mode == 1) {
        const cone = amount * (1.0 - d);
        hSet(i, j, cur + cone * 0.55 + amount * 0.45 * w);
      } else {
        hSet(i, j, cur - amount * w);
      }
    }
  }
}

// ---- particles -------------------------------------------------------------
function spawnPart(x: f32, y: f32, z: f32, vx: f32, vy: f32, vz: f32,
                   life: f32, size: f32, r: f32, g: f32, b: f32, grav: f32, drag: f32): void {
  const i = qhead; qhead = (qhead + 1) % MAXPT;
  qx[i] = x; qy[i] = y; qz[i] = z; qvx[i] = vx; qvy[i] = vy; qvz[i] = vz;
  ql[i] = life; qlm[i] = life; qs[i] = size; qr[i] = r; qg[i] = g; qb[i] = b;
  qgrav[i] = grav; qdrag[i] = drag;
}
function burst(x: f32, y: f32, z: f32, n: i32, spd: f32, size: f32, r: f32, g: f32, b: f32, life: f32): void {
  for (let k = 0; k < n; k++) {
    const a = rr(0, 6.2832), e = rr(-1.0, 1.0), s = spd * rr(0.3, 1.0);
    const ch = Mathf.sqrt(1.0 - e * e);
    spawnPart(x, y, z, Mathf.cos(a) * ch * s, e * s + spd * 0.25, Mathf.sin(a) * ch * s,
      life * rr(0.6, 1.2), size * rr(0.6, 1.4), r, g, b, -14.0, 1.6);
  }
}
function pushEvent(kind: f32, x: f32, y: f32, z: f32): void {
  if (evtN >= 128) return;
  const o = evtN * 4; EVT[o] = kind; EVT[o + 1] = x; EVT[o + 2] = y; EVT[o + 3] = z; evtN++;
}

// ---- spawning --------------------------------------------------------------
function addCreature(t: i32, x: f32, z: f32, own: i32): i32 {
  for (let i = 0; i < MAXE; i++) {
    if (ealive[i] != 0) continue;
    ealive[i] = 1; etype[i] = t; eown[i] = own; estate[i] = 0;
    ex[i] = x; ez[i] = z; ey[i] = heightAt(x, z) + creatureFloat(t);
    evx[i] = 0; evy[i] = 0; evz[i] = 0; eyaw[i] = rr(0, 6.2832);
    etim[i] = rr(0, 3); ecd[i] = 0; ephase[i] = rr(0, 6.2832); ehurt[i] = 0;
    // Big things are meant to be a fight, not a speed bump. Swarm types stay
    // thin so they still read as chaff.
    let hp: f32 = 30;
    if (t == 0) hp = 165;       // worm
    else if (t == 1) hp = 22;   // wasp
    else if (t == 2) hp = 340;  // troll
    else if (t == 3) hp = 200;  // griffin
    else if (t == 4) hp = 430;  // nest
    else if (t == 5) hp = 110;  // wraith (ally)
    else if (t == 6) hp = 30;   // balloon
    else if (t == 7) hp = 1500; // dragon
    ehp[i] = hp; ehpm[i] = hp;
    return i;
  }
  return -1;
}
@inline function creatureFloat(t: i32): f32 {
  if (t == 1) return 26.0; if (t == 3) return 34.0; if (t == 5) return 20.0;
  if (t == 6) return 46.0; if (t == 7) return 52.0; return 3.0;
}
function addOrb(x: f32, y: f32, z: f32, amt: f32): void {
  for (let i = 0; i < MAXO; i++) {
    if (oalive[i] != 0) continue;
    oalive[i] = 1; ox[i] = x; oy[i] = y; oz[i] = z; oamt[i] = amt; oown[i] = 0;
    ophase[i] = rr(0, 6.2832); oheld[i] = -1; return;
  }
}

// ---- spell table -----------------------------------------------------------
// cost, cooldown seconds
const SCOST = new StaticArray<f32>(NSPELL);
// each tier costs more than the last
@inline function fortressCost(w: i32): f32 { return 34.0 + <f32>clev[w] * 26.0; }
const SCD = new StaticArray<f32>(NSPELL);
function initSpellTable(): void {
  SCOST[0] = 4;  SCD[0] = 0.28;   // Firebolt
  SCOST[1] = 9;  SCD[1] = 1.10;   // Chain lightning
  SCOST[2] = 14; SCD[2] = 2.20;   // Crater
  SCOST[3] = 30; SCD[3] = 6.00;   // Volcano
  SCOST[4] = 24; SCD[4] = 5.00;   // Earthquake
  SCOST[5] = 46; SCD[5] = 8.00;   // Meteor
  SCOST[6] = 16; SCD[6] = 9.00;   // Shield
  SCOST[7] = 18; SCD[7] = 7.00;   // Mend
  SCOST[8] = 12; SCD[8] = 8.00;   // Haste
  SCOST[9] = 4;  SCD[9] = 0.55;   // Claim
  SCOST[10] = 22; SCD[10] = 6.0;  // Wraith
  SCOST[11] = 60; SCD[11] = 22.0; // Sunburst
  SCOST[12] = 34; SCD[12] = 1.60;  // Fortress (real cost is fortressCost)
}
function unlockFor(lv: i32): void {
  for (let i = 0; i < NSPELL; i++) UNL[i] = 0;
  UNL[0] = 1; UNL[9] = 1; UNL[7] = 1; UNL[12] = 1;   // firebolt, claim, mend, fortress
  if (lv >= 2) UNL[1] = 1;
  if (lv >= 3) UNL[2] = 1;
  if (lv >= 3) UNL[6] = 1;
  if (lv >= 4) UNL[8] = 1;
  if (lv >= 4) UNL[4] = 1;
  if (lv >= 5) UNL[10] = 1;
  if (lv >= 6) UNL[3] = 1;
  if (lv >= 7) UNL[5] = 1;
  if (lv >= 8) UNL[11] = 1;
}

// ---- level generation ------------------------------------------------------
function genTerrain(): void {
  // archipelago: sum of radial island masks modulated by fbm
  const nIsl = 3 + ri(3);
  const ix = new StaticArray<f32>(8), iz = new StaticArray<f32>(8), ir = new StaticArray<f32>(8), ih = new StaticArray<f32>(8);
  for (let k = 0; k < nIsl; k++) {
    ix[k] = rr(0.18, 0.82) * WORLD; iz[k] = rr(0.18, 0.82) * WORLD;
    ir[k] = rr(0.16, 0.30) * WORLD; ih[k] = rr(38, 96);
  }
  for (let j = 0; j < TW; j++) {
    for (let i = 0; i < TW; i++) {
      const wxp = <f32>i * CELL, wzp = <f32>j * CELL;
      let h: f32 = -26.0;
      for (let k = 0; k < nIsl; k++) {
        const dx = wxp - ix[k], dz = wzp - iz[k];
        const d = Mathf.sqrt(dx * dx + dz * dz) / ir[k];
        if (d < 1.4) {
          let m: f32 = 1.0 - d; if (m < 0) m = 0;
          m = m * m * (3.0 - 2.0 * m);
          h += ih[k] * m * 1.6;
        }
      }
      const n = fbm(wxp * 0.0055, wzp * 0.0055, 5) * 34.0 + fbm(wxp * 0.021, wzp * 0.021, 3) * 9.0;
      h += n * (h > -10.0 ? 1.0 : 0.35);
      // beach flattening near sea level
      if (h > -4.0 && h < 8.0) h *= 0.55;
      HEIGHT[j * TW + i] = h;
    }
  }
  dirtyLo = 0; dirtyHi = TWM;
}
function findLand(minH: f32): i32 {
  for (let tries = 0; tries < 4000; tries++) {
    const i = 12 + ri(TW - 24), j = 12 + ri(TW - 24);
    if (hGet(i, j) > minH) return j * TW + i;
  }
  return (TW / 2) * TW + TW / 2;
}
function placeScenery(): void {
  decCount = 0;
  for (let k = 0; k < MAXDEC; k++) {
    const x = rr(0, WORLD), z = rr(0, WORLD);
    const h = heightAt(x, z);
    if (h < 2.0) continue;
    let kind = 0;
    if (h < 16.0) kind = 0;            // palm
    else if (h < 60.0) kind = (ri(3) == 0) ? 0 : 1;   // rock/palm mix
    else kind = 1;                     // rock
    dx_[decCount] = x; dz_[decCount] = z; dkind[decCount] = kind;
    dsc[decCount] = rr(0.7, 1.5); drot[decCount] = rr(0, 6.2832);
    decCount++;
  }
}

export function init(seed: u32, lv: i32): void {
  srand(seed);
  level = lv; status = 0; gtime = 0; shake = 0; kills = 0;
  initSpellTable(); unlockFor(lv);
  nRival = lv >= 5 ? 2 : 1;
  for (let i = 0; i < MAXE; i++) ealive[i] = 0;
  for (let i = 0; i < MAXP; i++) palive[i] = 0;
  for (let i = 0; i < MAXO; i++) oalive[i] = 0;
  for (let i = 0; i < MAXPT; i++) ql[i] = 0;
  for (let i = 0; i < NSPELL; i++) CD[i] = 0;
  genTerrain(); placeScenery();
  totalMana = 420.0 + <f32>lv * 90.0;
  targetFrac = 0.50;
  sel = 0; iFwd = 0; iStr = 0; iUp = 0;

  // castles: one per wizard, on distinct high ground
  for (let w = 0; w < NWIZ; w++) { walive[w] = 0; }
  for (let w = 0; w <= nRival; w++) {
    let idx = findLand(24.0);
    // push rivals away from player
    for (let t = 0; t < 200; t++) {
      idx = findLand(24.0);
      const tx = <f32>(idx % TW) * CELL, tz = <f32>(idx / TW) * CELL;
      let ok = true;
      for (let u = 0; u < w; u++) {
        const dd = (tx - cx[u]) * (tx - cx[u]) + (tz - cz[u]) * (tz - cz[u]);
        if (dd < 820.0 * 820.0) { ok = false; break; }
      }
      if (ok) break;
    }
    const bx = <f32>(idx % TW) * CELL, bz = <f32>(idx / TW) * CELL;
    // level the castle pad
    deform(bx, bz, 30.0, 0.0, 2);
    const padH = heightAt(bx, bz);
    // Cut a real shelf: dead flat right out past the keep's footprint, then a
    // smooth skirt down to the original hill. The old 1-d^2 falloff was only
    // fully flat at the exact centre, so on a steep peak the castle still had
    // slope under its edges and read as hanging off the mountain.
    const gi = <i32>(bx / CELL), gj = <i32>(bz / CELL);
    const PAD: i32 = 13;                       // cells; 13 * CELL = 52 units
    for (let j = gj - PAD; j <= gj + PAD; j++)
      for (let i = gi - PAD; i <= gi + PAD; i++) {
        const d: f32 = Mathf.sqrt(<f32>((i - gi) * (i - gi) + (j - gj) * (j - gj))) / <f32>PAD;
        if (d > 1.0) continue;
        // flat well past the plinth (FOOT=27) so ground wraps its base and you
        // can never see under the terrace edge
        let t: f32 = (d - 0.72) / 0.28;
        if (t < 0.0) t = 0.0; if (t > 1.0) t = 1.0;
        const w2: f32 = 1.0 - t * t * (3.0 - 2.0 * t);
        const cur = hGet(i, j);
        hSet(i, j, cur + (padH - cur) * w2);
      }
    cx[w] = bx; cz[w] = bz; cy[w] = heightAt(bx, bz);
    chpm[w] = 600.0 + <f32>lv * 60.0; chp[w] = chpm[w]; clev[w] = 1; cbt[w] = 3.0;
    walive[w] = 1;
    wx[w] = bx; wz[w] = bz + 40.0; wy[w] = cy[w] + 40.0;
    wvx[w] = 0; wvy[w] = 0; wvz[w] = 0; wyaw[w] = 0; wpit[w] = 0; wrol[w] = 0;
    whp[w] = 100.0; wcar[w] = 0; wstore[w] = 0; wmana[w] = w == 0 ? 45.0 : 60.0; wcap[w] = 0;
    wcd[w] = 0; wresp[w] = 0; wshield[w] = 0; whaste[w] = 0; wai[w] = 0; waitim[w] = 0; wnohit[w] = 0;
  }

  // Creature nests + starting wildlife. Counts scale with the map, which is
  // now 6x the area — the old numbers left everything piled in one corner.
  const nNests = 9 + lv * 2;
  for (let k = 0; k < nNests; k++) {
    const idx = findLand(14.0);
    addCreature(4, <f32>(idx % TW) * CELL, <f32>(idx / TW) * CELL, 0);
  }
  const nWild = 24 + lv * 6;
  for (let k = 0; k < nWild; k++) {
    const idx = findLand(6.0);
    let t = ri(4); if (t == 4) t = 0;
    if (lv >= 6 && ri(14) == 0) t = 7;
    addCreature(t, <f32>(idx % TW) * CELL, <f32>(idx / TW) * CELL, 0);
  }
  // seed some free mana so the map is immediately playable
  for (let k = 0; k < 96; k++) {
    const idx = findLand(4.0);
    const axx = <f32>(idx % TW) * CELL, azz = <f32>(idx / TW) * CELL;
    addOrb(axx, heightAt(axx, azz) + 4.0, azz, rr(5, 10));
  }
  spawnTimer = 4.0;
  clearDirty(); dirtyLo = 0; dirtyHi = TWM;
  buildRender();
}

// ---- combat ----------------------------------------------------------------
function dropMana(x: f32, y: f32, z: f32, amt: f32): void {
  let left = amt;
  while (left > 0.5) {
    const chunk: f32 = left > 9.0 ? 9.0 : left;
    addOrb(x + rr(-9, 9), y + rr(2, 9), z + rr(-9, 9), chunk);
    left -= chunk;
  }
}
function killCreature(i: i32): void {
  const t = etype[i];
  let mana: f32 = 8.0;
  if (t == 0) mana = 15; else if (t == 1) mana = 8; else if (t == 2) mana = 28;
  else if (t == 3) mana = 20; else if (t == 4) mana = 60; else if (t == 7) mana = 110;
  else if (t == 5 || t == 6) mana = 0;
  ealive[i] = 0;
  if (eown[i] == 0) kills += 1.0;
  burst(ex[i], ey[i], ez[i], 16, 16.0, 3.0, 1.0, 0.55, 0.15, 0.7);
  if (mana > 0) { dropMana(ex[i], ey[i], ez[i], mana); pushEvent(3, ex[i], ey[i], ez[i]); }
}
// fac: 0 = wild, w+1 = wizard w and everything it owns
function damageArea(x: f32, y: f32, z: f32, rad: f32, dmg: f32, fac: i32): void {
  const r2 = rad * rad;
  for (let i = 0; i < MAXE; i++) {
    if (ealive[i] == 0) continue;
    if (eown[i] == fac) continue;
    const dx = ex[i] - x, dy = ey[i] - y, dz = ez[i] - z;
    const d2 = dx * dx + dy * dy + dz * dz;
    if (d2 > r2) continue;
    const fall: f32 = 1.0 - Mathf.sqrt(d2) / rad;
    ehp[i] -= dmg * (0.45 + 0.55 * fall);
    ehurt[i] = 0.25;
    const inv: f32 = 1.0 / (Mathf.sqrt(d2) + 0.001);
    evx[i] += dx * inv * dmg * 0.5; evy[i] += dy * inv * dmg * 0.25 + 4.0; evz[i] += dz * inv * dmg * 0.5;
    if (ehp[i] <= 0) killCreature(i);
  }
  for (let w = 0; w <= nRival; w++) {
    if (walive[w] == 0 || w + 1 == fac) continue;
    const dx = wx[w] - x, dy = wy[w] - y, dz = wz[w] - z;
    const d2 = dx * dx + dy * dy + dz * dz;
    if (d2 > r2) continue;
    const fall: f32 = 1.0 - Mathf.sqrt(d2) / rad;
    let d: f32 = dmg * (0.4 + 0.6 * fall);
    if (wshield[w] > 0) d *= 0.25;
    whp[w] -= d; wnohit[w] = 0;
    if (w == 0) shake = shake > 0.5 ? shake : 0.5;
    if (whp[w] <= 0) {
      whp[w] = 0;
      if (w == 0) { status = 2; }
      else {
        walive[w] = 0; wresp[w] = 14.0;
        dropMana(wx[w], wy[w], wz[w], wmana[w] * 0.65); wmana[w] *= 0.35;
        burst(wx[w], wy[w], wz[w], 40, 26.0, 4.0, 0.9, 0.3, 0.9, 1.1);
        pushEvent(4, wx[w], wy[w], wz[w]);
      }
    }
  }
  // castles
  for (let w = 0; w <= nRival; w++) {
    if (w + 1 == fac || chp[w] <= 0) continue;
    const dx = cx[w] - x, dz = cz[w] - z, dy = cy[w] + 20.0 - y;
    const d2 = dx * dx + dz * dz + dy * dy;
    if (d2 > (rad + 26.0) * (rad + 26.0)) continue;
    chp[w] -= dmg * 0.8;
    if (chp[w] <= 0) {
      chp[w] = 0;
      dropMana(cx[w], cy[w] + 18.0, cz[w], wstore[w] * 0.75);
      wstore[w] *= 0.25;
      burst(cx[w], cy[w] + 16.0, cz[w], 70, 34.0, 6.0, 1.0, 0.7, 0.3, 1.6);
      shake = 1.2; pushEvent(5, cx[w], cy[w], cz[w]);
    }
  }
}
function addProj(x: f32, y: f32, z: f32, vx: f32, vy: f32, vz: f32,
                 t: i32, own: i32, dmg: f32, rad: f32, life: f32): void {
  for (let i = 0; i < MAXP; i++) {
    if (palive[i] != 0) continue;
    palive[i] = 1; px_[i] = x; py_[i] = y; pz_[i] = z;
    pvx[i] = vx; pvy[i] = vy; pvz[i] = vz;
    ptype[i] = t; pown[i] = own; pdmg[i] = dmg; prad[i] = rad; plife[i] = life;
    return;
  }
}
// march a ray until it hits terrain; returns hit distance (or maxd)
function rayGround(x: f32, y: f32, z: f32, dx: f32, dy: f32, dz: f32, maxd: f32): f32 {
  let t: f32 = 4.0;
  while (t < maxd) {
    const hx = x + dx * t, hy = y + dy * t, hz = z + dz * t;
    const g = heightAt(hx, hz);
    if (hy <= g) return t;
    const gap = hy - g;
    t += gap > 12.0 ? gap * 0.6 : 5.0;
  }
  return maxd;
}

// ---- casting ---------------------------------------------------------------
function fwdVec(w: i32, out: StaticArray<f32>): void {
  const cp = Mathf.cos(wpit[w]), sp = Mathf.sin(wpit[w]);
  out[0] = Mathf.sin(wyaw[w]) * cp; out[1] = sp; out[2] = Mathf.cos(wyaw[w]) * cp;
}
const tmpv = new StaticArray<f32>(3);

function castSpell(w: i32, s: i32): i32 {
  if (s < 0 || s >= NSPELL) return 0;
  if (w == 0 && UNL[s] == 0) return 0;
  if (w == 0 && CD[s] > 0) return 0;
  // Fortress only works standing over your own keep, and only while there is
  // a tier left to buy. Checked before any mana is spent.
  if (s == 12) {
    const dxc = cx[w] - wx[w], dyc = cy[w] - wy[w], dzc = cz[w] - wz[w];
    if (chp[w] <= 0 || clev[w] >= 6) return 0;
    if (dxc * dxc + dyc * dyc + dzc * dzc > 210.0 * 210.0) return 0;
  }
  const need: f32 = s == 12 ? fortressCost(w) : SCOST[s];
  if (wmana[w] < need) return 0;
  wmana[w] -= need;
  if (w == 0) CD[s] = SCD[s];
  const fac = w + 1;
  fwdVec(w, tmpv);
  const fx = tmpv[0], fy = tmpv[1], fz = tmpv[2];
  const sx = wx[w] + fx * 6.0, sy = wy[w] - 2.0 + fy * 6.0, sz = wz[w] + fz * 6.0;

  if (s == 0) {                       // Firebolt
    const fdmg: f32 = w == 0 ? 34.0 : (11.0 + <f32>level * 2.3);
    addProj(sx, sy, sz, fx * 220.0, fy * 220.0, fz * 220.0, 0, fac, fdmg, 20.0, 3.2);
    pushEvent(0, sx, sy, sz);
  } else if (s == 1) {                // Chain lightning
    let best = -1; let bd: f32 = 340.0 * 340.0;
    for (let i = 0; i < MAXE; i++) {
      if (ealive[i] == 0 || eown[i] == fac) continue;
      const dx = ex[i] - wx[w], dy = ey[i] - wy[w], dz = ez[i] - wz[w];
      const d2 = dx * dx + dy * dy + dz * dz;
      if (d2 > bd) continue;
      const inv = 1.0 / Mathf.sqrt(d2);
      if (dx * inv * fx + dy * inv * fy + dz * inv * fz < 0.35) continue;
      bd = d2; best = i;
    }
    let lx = wx[w], ly = wy[w], lz = wz[w];
    if (best >= 0) {
      let cur = best;
      for (let hop = 0; hop < 4 && cur >= 0; hop++) {
        arcBolt(lx, ly, lz, ex[cur], ey[cur], ez[cur]);
        damageArea(ex[cur], ey[cur], ez[cur], 16.0, 52.0, fac);
        lx = ex[cur]; ly = ey[cur]; lz = ez[cur];
        let nx = -1; let nd: f32 = 150.0 * 150.0;
        for (let i = 0; i < MAXE; i++) {
          if (ealive[i] == 0 || i == cur || eown[i] == fac) continue;
          const dx = ex[i] - lx, dy = ey[i] - ly, dz = ez[i] - lz;
          const d2 = dx * dx + dy * dy + dz * dz;
          if (d2 < nd) { nd = d2; nx = i; }
        }
        cur = nx;
      }
    } else {
      const d = rayGround(wx[w], wy[w], wz[w], fx, fy, fz, 340.0);
      arcBolt(lx, ly, lz, wx[w] + fx * d, wy[w] + fy * d, wz[w] + fz * d);
      damageArea(wx[w] + fx * d, wy[w] + fy * d, wz[w] + fz * d, 26.0, 40.0, fac);
    }
    pushEvent(1, wx[w], wy[w], wz[w]);
  } else if (s == 2) {                // Crater
    const d = rayGround(wx[w], wy[w], wz[w], fx, fy, fz, 420.0);
    const tx = clampWorld(wx[w] + fx * d), tz = clampWorld(wz[w] + fz * d);
    deform(tx, tz, 46.0, 20.0, 0);
    damageArea(tx, heightAt(tx, tz), tz, 54.0, 46.0, fac);
    burst(tx, heightAt(tx, tz) + 6.0, tz, 44, 22.0, 6.0, 0.75, 0.6, 0.45, 1.4);
    shake = w == 0 ? 0.9 : shake; pushEvent(2, tx, heightAt(tx, tz), tz);
  } else if (s == 3) {                // Volcano
    const d = rayGround(wx[w], wy[w], wz[w], fx, fy, fz, 420.0);
    const tx = clampWorld(wx[w] + fx * d), tz = clampWorld(wz[w] + fz * d);
    deform(tx, tz, 60.0, 62.0, 1);
    damageArea(tx, heightAt(tx, tz), tz, 66.0, 60.0, fac);
    for (let k = 0; k < 90; k++) {
      const a = rr(0, 6.2832), sp2 = rr(20, 70);
      spawnPart(tx, heightAt(tx, tz) + 30.0, tz, Mathf.cos(a) * sp2 * 0.4, rr(40, 105), Mathf.sin(a) * sp2 * 0.4,
        rr(1.4, 3.0), rr(4, 9), 1.0, rr(0.25, 0.6), 0.08, -30.0, 0.5);
    }
    shake = w == 0 ? 1.3 : shake; pushEvent(6, tx, heightAt(tx, tz), tz);
  } else if (s == 4) {                // Earthquake — travelling ripple
    for (let k = 1; k <= 7; k++) {
      const dd = <f32>k * 52.0;
      const tx = clampWorld(wx[w] + fx * dd), tz = clampWorld(wz[w] + fz * dd);
      deform(tx, tz, 40.0, 13.0 - <f32>k * 0.9, 2);
      damageArea(tx, heightAt(tx, tz), tz, 46.0, 34.0, fac);
      burst(tx, heightAt(tx, tz) + 3.0, tz, 14, 15.0, 5.0, 0.65, 0.5, 0.36, 1.1);
    }
    shake = w == 0 ? 1.5 : shake; pushEvent(7, wx[w], wy[w], wz[w]);
  } else if (s == 5) {                // Meteor
    const d = rayGround(wx[w], wy[w], wz[w], fx, fy, fz, 520.0);
    const tx = clampWorld(wx[w] + fx * d), tz = clampWorld(wz[w] + fz * d);
    addProj(tx - 60.0, 420.0, tz - 60.0, 60.0 * 0.55, -240.0, 60.0 * 0.55, 2, fac, 150.0, 78.0, 6.0);
    pushEvent(8, tx, 300.0, tz);
  } else if (s == 6) { wshield[w] = 10.0; pushEvent(9, wx[w], wy[w], wz[w]); }
  else if (s == 7) {
    whp[w] += 55.0; if (whp[w] > 100.0) whp[w] = 100.0;
    for (let k = 0; k < 30; k++) spawnPart(wx[w] + rr(-8, 8), wy[w] + rr(-6, 6), wz[w] + rr(-8, 8), 0, rr(6, 20), 0, 1.0, 3.0, 0.4, 1.0, 0.6, 4.0, 1.0);
    pushEvent(10, wx[w], wy[w], wz[w]);
  } else if (s == 8) { whaste[w] = 12.0; pushEvent(9, wx[w], wy[w], wz[w]); }
  else if (s == 9) {                  // Claim — possess loose mana
    // Turns unclaimed gold orbs white and marks them for your balloons.
    // Possessing mana also permanently widens your own pool, which is the
    // only way to afford the expensive spells later on.
    let n2 = 0; let gain: f32 = 0;
    for (let i = 0; i < MAXO; i++) {
      if (oalive[i] == 0 || oheld[i] >= 0 || oown[i] == fac) continue;
      const dx = ox[i] - wx[w], dy = oy[i] - wy[w], dz = oz[i] - wz[w];
      if (dx * dx + dy * dy + dz * dz > 200.0 * 200.0) continue;
      oown[i] = fac; n2++; gain += oamt[i];
      for (let k = 0; k < 7; k++)
        spawnPart(ox[i], oy[i], oz[i], rr(-9, 9), rr(2, 12), rr(-9, 9), 0.55, 2.0, 0.75, 0.9, 1.0, 0, 1.2);
    }
    if (n2 > 0) {
      wcap[w] += gain * 0.28;
      wmana[w] += gain * 0.42;
      const cp = manaCap(w); if (wmana[w] > cp) wmana[w] = cp;
      if (w == 0) pushEvent(3, wx[w], wy[w], wz[w]);
    }
  } else if (s == 10) {               // Wraith ally
    const d = rayGround(wx[w], wy[w], wz[w], fx, fy, fz, 260.0);
    const tx = clampWorld(wx[w] + fx * d), tz = clampWorld(wz[w] + fz * d);
    const id = addCreature(5, tx, tz, fac);
    if (id >= 0) { etim[id] = 45.0; burst(tx, heightAt(tx, tz) + 12.0, tz, 30, 14.0, 4.0, 0.55, 0.35, 1.0, 1.2); }
    pushEvent(12, tx, heightAt(tx, tz), tz);
  } else if (s == 12) {               // Fortress — raise your keep a tier
    clev[w] += 1;
    chpm[w] += 220.0; chp[w] = chpm[w];
    burst(cx[w], cy[w] + 26.0, cz[w], 70, 30.0, 12.0, 0.55, 0.85, 1.0, 2.2);
    shake = w == 0 ? 0.7 : shake;
    if (w == 0) pushEvent(16, cx[w], cy[w], cz[w]);
  } else if (s == 11) {               // Sunburst — smites every hostile creature
    for (let i = 0; i < MAXE; i++) {
      if (ealive[i] == 0 || eown[i] == fac) continue;
      if (etype[i] == 4) { ehp[i] -= 120.0; } else { ehp[i] -= 200.0; }
      arcBolt(ex[i], ey[i] + 260.0, ez[i], ex[i], ey[i], ez[i]);
      if (ehp[i] <= 0) killCreature(i);
    }
    shake = w == 0 ? 1.0 : shake; pushEvent(13, wx[w], wy[w], wz[w]);
  }
  return 1;
}
function arcBolt(x0: f32, y0: f32, z0: f32, x1: f32, y1: f32, z1: f32): void {
  const dx = x1 - x0, dy = y1 - y0, dz = z1 - z0;
  const len = Mathf.sqrt(dx * dx + dy * dy + dz * dz);
  const n = <i32>(len / 5.0); const steps = n < 4 ? 4 : (n > 90 ? 90 : n);
  for (let k = 0; k <= steps; k++) {
    const t = <f32>k / <f32>steps;
    const j: f32 = 5.0 * Mathf.sin(t * 3.14159);
    spawnPart(x0 + dx * t + rr(-j, j), y0 + dy * t + rr(-j, j), z0 + dz * t + rr(-j, j),
      rr(-3, 3), rr(-3, 3), rr(-3, 3), rr(0.14, 0.34), rr(2.0, 4.2), 0.62, 0.80, 1.0, 0, 1.0);
  }
}
export function cast(s: i32): void { if (status == 0) castSpell(0, s); }
export function selectSpell(s: i32): void { if (s >= 0 && s < NSPELL && UNL[s] == 1) sel = s; }
export function cycleSpell(dir: i32): void {
  for (let k = 0; k < NSPELL; k++) {
    sel = (sel + dir + NSPELL) % NSPELL;
    if (UNL[sel] == 1) return;
  }
}

// ---- wizards ---------------------------------------------------------------
// Regen ceiling. Nothing can be laundered into progress through it any more
// (the fortress is filled by balloons), so it can afford to be generous —
// it has to at least reach the price of the first Fortress tier.
const BANK_FLOOR: f32 = 72.0;
@inline function manaCap(w: i32): f32 { return 120.0 + <f32>clev[w] * 50.0 + wcap[w]; }
// A fortress can only hold so much; raising a tier is what buys headroom.
@inline function castleCap(w: i32): f32 { return 240.0 * <f32>clev[w]; }
// mana that must sit inside your fortress to take the realm
@inline function levelTarget(): f32 { return 180.0 + <f32>level * 160.0; }

function updatePlayer(dt: f32): void {
  const w = 0;
  // Same handedness trap as the strafe vector: d(forward)/d(yaw) points along
  // -screen_right, so a rising yaw swings the view left. Mouse right must
  // decrease it. (Pitch is already the right way round.)
  wyaw[w] -= iYaw; wpit[w] -= iPit;
  if (wpit[w] > 1.05) wpit[w] = 1.05; if (wpit[w] < -1.05) wpit[w] = -1.05;
  const roll = -iStr * 0.42;
  wrol[w] += (roll - wrol[w]) * Mathf.min(1.0, dt * 6.0);
  fwdVec(w, tmpv);
  const fx = tmpv[0], fy = tmpv[1], fz = tmpv[2];
  // Screen-right. The view matrix puts the camera's x axis at
  // (-cos yaw, 0, sin yaw); strafing along +(cos yaw, 0, -sin yaw) sent you
  // the opposite way, so D slid left.
  const rx = -Mathf.cos(wyaw[w]), rz = Mathf.sin(wyaw[w]);
  let maxs: f32 = 132.0; if (whaste[w] > 0) maxs *= 1.75;
  if (iBrake != 0) maxs *= 0.28;
  const tvx = fx * iFwd * maxs + rx * iStr * maxs * 0.55;
  const tvy = fy * iFwd * maxs + iUp * 78.0;
  const tvz = fz * iFwd * maxs + rz * iStr * maxs * 0.55;
  // Brisk enough that turning re-aims the carpet rather than leaving it
  // skating along its old heading.
  const k = Mathf.min(1.0, dt * 5.2);
  wvx[w] += (tvx - wvx[w]) * k; wvy[w] += (tvy - wvy[w]) * k; wvz[w] += (tvz - wvz[w]) * k;
  wx[w] += wvx[w] * dt; wy[w] += wvy[w] * dt; wz[w] += wvz[w] * dt;
  wx[w] = clampWorld(wx[w]); wz[w] = clampWorld(wz[w]);
  const g = heightAt(wx[w], wz[w]);
  const floorY = Mathf.max(g, SEA) + 7.5;
  if (wy[w] < floorY) {
    const pen = floorY - wy[w];
    wy[w] += pen * Mathf.min(1.0, dt * 14.0);
    if (wvy[w] < -60.0) { whp[w] -= (-wvy[w] - 60.0) * dt * 1.2; shake = 0.4; }
    if (wvy[w] < 0) wvy[w] *= 0.25;
  }
  if (wy[w] > 430.0) { wy[w] = 430.0; if (wvy[w] > 0) wvy[w] = 0; }
  if (wshield[w] > 0) wshield[w] -= dt;
  if (whaste[w] > 0) whaste[w] -= dt;
  // carpet trail
  if (rnd() < 0.55) spawnPart(wx[w] + rr(-4, 4), wy[w] - 3.0, wz[w] + rr(-4, 4), 0, rr(-2, 2), 0, 0.55, 1.6, 0.45, 0.35, 0.85, 0, 2.0);
}

function wizardCommon(w: i32, dt: f32): void {
  wnohit[w] += dt;
  if (wnohit[w] > 6.0 && whp[w] < 100.0) { whp[w] += 1.5 * dt; if (whp[w] > 100.0) whp[w] = 100.0; }
  // passive mana from own castle
  // spell fuel regenerates only up to the bankable floor, so it can never be
  // laundered into claim progress by parking on your own castle
  if (chp[w] > 0 && wmana[w] < BANK_FLOOR) {
    wmana[w] += (5.0 + <f32>clev[w] * 2.2) * dt;
    if (wmana[w] > BANK_FLOOR) wmana[w] = BANK_FLOOR;
  }
  // Orbs are not vacuumed up by flying over them any more: you possess them
  // with Claim and your balloons carry them to the fortress.
  // deposit at own castle
  if (chp[w] > 0) {
    const dx = cx[w] - wx[w], dz = cz[w] - wz[w], dy = cy[w] + 16.0 - wy[w];
    if (dx * dx + dz * dz + dy * dy < 62.0 * 62.0) {
      whp[w] += 7.0 * dt; if (whp[w] > 100.0) whp[w] = 100.0;
    }
  }
  // Tier is bought with the Fortress spell now, never derived from the store.
  // Rivals buy their own once their keep is nearly full.
  if (w != 0 && chp[w] > 0 && clev[w] < 6 && wstore[w] > castleCap(w) * 0.82 && wmana[w] > 44.0) {
    clev[w] += 1; wmana[w] -= 44.0; chpm[w] += 220.0; chp[w] = chpm[w];
  }
  // balloons ferry mana home
  if (chp[w] > 0) {
    cbt[w] -= dt;
    if (cbt[w] <= 0) {
      cbt[w] = 7.0;
      let n = 0;
      for (let i = 0; i < MAXE; i++) if (ealive[i] != 0 && etype[i] == 6 && eown[i] == w + 1) n++;
      const cap = w == 0 ? clev[w] + 1 : (1 + level / 2 < clev[w] ? 1 + level / 2 : clev[w]);
      if (n < cap) addCreature(6, cx[w] + rr(-20, 20), cz[w] + rr(-20, 20), w + 1);
    }
  }
}

function updateRival(w: i32, dt: f32): void {
  if (walive[w] == 0) {
    wresp[w] -= dt;
    if (wresp[w] <= 0 && chp[w] > 0) {
      walive[w] = 1; whp[w] = 100.0; wx[w] = cx[w]; wz[w] = cz[w] + 30.0; wy[w] = cy[w] + 45.0;
      wmana[w] = 60.0;
    }
    return;
  }
  if (wshield[w] > 0) wshield[w] -= dt;
  if (whaste[w] > 0) whaste[w] -= dt;
  waitim[w] -= dt;
  // pick behaviour
  if (whp[w] < 34.0) wai[w] = 3;
  else if (waitim[w] <= 0) {
    waitim[w] = rr(1.6, 3.4);
    const dpx = wx[0] - wx[w], dpz = wz[0] - wz[w], dpy = wy[0] - wy[w];
    const dp2 = dpx * dpx + dpz * dpz + dpy * dpy;
    if (walive[0] != 0 && dp2 < 640.0 * 640.0 && wmana[w] > 26.0 && rnd() < 0.24 + <f32>level * 0.06) wai[w] = 2;
    else if (wmana[w] > manaCap(w) * 0.8) wai[w] = 1;
    else wai[w] = 0;
  }
  let tx = cx[w], ty = cy[w] + 34.0, tz = cz[w];
  if (wai[w] == 0) {
    let best = -1; let bd: f32 = 1e18;
    for (let i = 0; i < MAXO; i++) {
      if (oalive[i] == 0 || oheld[i] >= 0) continue;
      const dx = ox[i] - wx[w], dz = oz[i] - wz[w];
      const d2 = dx * dx + dz * dz;
      if (d2 < bd) { bd = d2; best = i; }
    }
    if (best >= 0) {
      tx = ox[best]; ty = oy[best] + 6.0; tz = oz[best];
      // Possess what it is flying to, so its balloons have something to carry.
      // Rate-limited: AI casts skip the player's cooldown table, so an
      // ungated call here fires 60x a second and hoovers the map instantly.
      if (bd < 190.0 * 190.0 && rnd() < 0.015) castSpell(w, 9);
    }
    else {
      // hunt creatures for fresh mana
      let bc = -1; bd = 1e18;
      for (let i = 0; i < MAXE; i++) {
        if (ealive[i] == 0 || eown[i] != 0) continue;
        const dx = ex[i] - wx[w], dz = ez[i] - wz[w];
        const d2 = dx * dx + dz * dz;
        if (d2 < bd) { bd = d2; bc = i; }
      }
      if (bc >= 0) {
        tx = ex[bc]; ty = ey[bc] + 26.0; tz = ez[bc];
        if (bd < 540.0 * 540.0 && wcd[w] <= 0) {
          aimAndFire(w, ex[bc], ey[bc], ez[bc], 0); wcd[w] = rr(0.5, 1.1);
        }
      }
    }
  } else if (wai[w] == 2 && walive[0] != 0) {
    tx = wx[0] - Mathf.sin(wyaw[0]) * 130.0; ty = wy[0] + 16.0; tz = wz[0] - Mathf.cos(wyaw[0]) * 130.0;
    const dx = wx[0] - wx[w], dy = wy[0] - wy[w], dz = wz[0] - wz[w];
    const d2 = dx * dx + dy * dy + dz * dz;
    if (d2 < 620.0 * 620.0 && wcd[w] <= 0) {
      const r = rnd();
      if (r < 0.16 && wmana[w] > 26.0 && level >= 3) { aimAt(w, wx[0], wy[0], wz[0]); castSpell(w, 2); wcd[w] = rr(2.4, 4.0); }
      else if (r < 0.36 && wmana[w] > 12.0 && level >= 2) { aimAt(w, wx[0], wy[0], wz[0]); castSpell(w, 1); wcd[w] = rr(1.4, 2.6); }
      else { aimAndFire(w, wx[0], wy[0], wz[0], 1); wcd[w] = rr(0.42, 0.9) + (9.0 - <f32>level) * 0.11; }
    }
  } else if (wai[w] == 3) {
    if (whp[w] < 70.0 && wmana[w] > 20.0 && wcd[w] <= 0) { castSpell(w, 7); wcd[w] = 3.0; }
    if (whp[w] > 82.0) wai[w] = 0;
  }
  wcd[w] -= dt;
  // steer
  const dx = tx - wx[w], dy = ty - wy[w], dz = tz - wz[w];
  const d = Mathf.sqrt(dx * dx + dy * dy + dz * dz) + 0.001;
  let spd: f32 = 74.0 + <f32>level * 4.5; if (whaste[w] > 0) spd *= 1.6;
  const k = Mathf.min(1.0, dt * 2.2);
  wvx[w] += (dx / d * spd - wvx[w]) * k;
  wvy[w] += (dy / d * spd * 0.7 - wvy[w]) * k;
  wvz[w] += (dz / d * spd - wvz[w]) * k;
  wx[w] = clampWorld(wx[w] + wvx[w] * dt);
  wy[w] += wvy[w] * dt;
  wz[w] = clampWorld(wz[w] + wvz[w] * dt);
  const floorY = Mathf.max(heightAt(wx[w], wz[w]), SEA) + 16.0;
  if (wy[w] < floorY) { wy[w] = floorY; if (wvy[w] < 0) wvy[w] = 0; }
  if (wy[w] > 280.0) wy[w] = 280.0;
  if (wai[w] != 2) wyaw[w] = Mathf.atan2(dx, dz);
  if (rnd() < 0.5) spawnPart(wx[w] + rr(-4, 4), wy[w] - 3.0, wz[w] + rr(-4, 4), 0, rr(-2, 2), 0, 0.6, 2.0, 1.0, 0.35, 0.3, 0, 2.0);
}
function aimAt(w: i32, tx: f32, ty: f32, tz: f32): void {
  const dx = tx - wx[w], dy = ty - wy[w], dz = tz - wz[w];
  const hd = Mathf.sqrt(dx * dx + dz * dz);
  wyaw[w] = Mathf.atan2(dx, dz); wpit[w] = Mathf.atan2(dy, hd);
}
function aimAndFire(w: i32, tx: f32, ty: f32, tz: f32, lead: i32): void {
  let ax = tx, ay = ty, az = tz;
  if (lead != 0) { ax += wvx[0] * 0.42; ay += wvy[0] * 0.42; az += wvz[0] * 0.42; }
  aimAt(w, ax, ay, az);
  let j: f32 = 0.13 - <f32>level * 0.013; if (j < 0.015) j = 0.015;
  wyaw[w] += rr(-j, j); wpit[w] += rr(-j, j) * 0.6;
  castSpell(w, 0);
}

// ---- creatures -------------------------------------------------------------
function nearestHostile(i: i32, maxd: f32): i32 {
  let best = -1; let bd = maxd * maxd;
  const mine = eown[i];
  for (let k = 0; k < MAXE; k++) {
    if (ealive[k] == 0 || k == i) continue;
    if (eown[k] == mine) continue;
    if (etype[k] == 6) continue;
    const dx = ex[k] - ex[i], dy = ey[k] - ey[i], dz = ez[k] - ez[i];
    const d2 = dx * dx + dy * dy + dz * dz;
    if (d2 < bd) { bd = d2; best = k; }
  }
  return best;
}
function hurtWizard(w: i32, d: f32): void {
  if (walive[w] == 0) return;
  let dd = d; if (wshield[w] > 0) dd *= 0.25;
  whp[w] -= dd; wnohit[w] = 0;
  if (w == 0) { shake = shake > 0.32 ? shake : 0.32; pushEvent(17, wx[0], wy[0], wz[0]); }
  if (whp[w] <= 0) {
    whp[w] = 0;
    if (w == 0) status = 2;
    else { walive[w] = 0; wresp[w] = 14.0; dropMana(wx[w], wy[w], wz[w], wmana[w] * 0.65); wmana[w] *= 0.35; }
  }
}
function updateCreatures(dt: f32): void {
  for (let i = 0; i < MAXE; i++) {
    if (ealive[i] == 0) continue;
    const t = etype[i], own = eown[i];
    ephase[i] += dt * (t == 1 ? 26.0 : 4.0);
    if (ehurt[i] > 0) ehurt[i] -= dt;
    ecd[i] -= dt;

    if (t == 4) {                          // nest
      etim[i] -= dt;
      if (etim[i] <= 0) {
        etim[i] = Mathf.max(6.0, 17.0 - <f32>level * 0.8);
        let n = 0;
        for (let k = 0; k < MAXE; k++) if (ealive[k] != 0 && eown[k] == 0 && etype[k] != 4) n++;
        if (n < 8 + level * 3) {
          let ct = ri(4); if (ct == 4) ct = 1;
          const id = addCreature(ct, ex[i] + rr(-30, 30), ez[i] + rr(-30, 30), 0);
          if (id >= 0) burst(ex[i], ey[i] + 6.0, ez[i], 8, 8.0, 2.5, 0.6, 0.3, 0.8, 0.6);
        }
      }
      ey[i] = heightAt(ex[i], ez[i]) + 3.0;
      continue;
    }
    if (t == 6) {                          // mana balloon
      const ow = own - 1;
      // Balloons cruise high, but orbs hover at ground + 5. Without stooping,
      // the grab sphere can never close and nothing is ever delivered.
      let floorLift: f32 = 34.0;
      let tx = cx[ow], ty = cy[ow] + 46.0, tz = cz[ow];
      let carrying = -1;
      for (let k = 0; k < MAXO; k++) if (oalive[k] != 0 && oheld[k] == i) { carrying = k; break; }
      if (carrying < 0) {
        let best = -1; let bd: f32 = 980.0 * 980.0;
        for (let k = 0; k < MAXO; k++) {
          if (oalive[k] == 0 || oheld[k] >= 0 || oown[k] != own) continue;
          const dx = ox[k] - ex[i], dz = oz[k] - ez[i];
          const d2 = dx * dx + dz * dz;
          if (d2 < bd) { bd = d2; best = k; }
        }
        if (best >= 0) {
          tx = ox[best]; ty = oy[best] + 3.0; tz = oz[best];
          floorLift = 5.0;
          const dx = ox[best] - ex[i], dy = oy[best] - ey[i], dz = oz[best] - ez[i];
          if (dx * dx + dy * dy + dz * dz < 30.0 * 30.0) oheld[best] = i;
        }
      } else {
        ox[carrying] = ex[i]; oy[carrying] = ey[i] - 10.0; oz[carrying] = ez[i];
        const dx = cx[ow] - ex[i], dz = cz[ow] - ez[i], dy = cy[ow] + 30.0 - ey[i];
        if (dx * dx + dz * dz + dy * dy < 52.0 * 52.0) {
          // Rivals ferry at a handicap that closes as the realms get harder —
          // the old economy had the same ramp on their banking rate.
          const eff: f32 = ow == 0 ? 1.0 : (0.34 + <f32>level * 0.08);
          wstore[ow] = Mathf.min(wstore[ow] + oamt[carrying] * eff, castleCap(ow)); oalive[carrying] = 0;
          for (let k = 0; k < 10; k++) spawnPart(cx[ow], cy[ow] + 22.0, cz[ow], rr(-10, 10), rr(4, 20), rr(-10, 10), 0.7, 2.4, 0.5, 0.8, 1.0, 0, 1.5);
        }
      }
      const dx = tx - ex[i], dy = ty - ey[i], dz = tz - ez[i];
      const d = Mathf.sqrt(dx * dx + dy * dy + dz * dz) + 0.01;
      const sp: f32 = 66.0;   // the map is 2.5x wider than it was
      evx[i] += (dx / d * sp - evx[i]) * Mathf.min(1.0, dt * 1.5);
      evy[i] += (dy / d * sp * 0.6 - evy[i]) * Mathf.min(1.0, dt * 1.5);
      evz[i] += (dz / d * sp - evz[i]) * Mathf.min(1.0, dt * 1.5);
      ex[i] = clampWorld(ex[i] + evx[i] * dt); ey[i] += evy[i] * dt; ez[i] = clampWorld(ez[i] + evz[i] * dt);
      const fy = Mathf.max(heightAt(ex[i], ez[i]), SEA) + floorLift; if (ey[i] < fy) ey[i] = fy;
      eyaw[i] = Mathf.atan2(dx, dz);
      if (ehp[i] <= 0) { if (carrying >= 0) oheld[carrying] = -1; ealive[i] = 0; }
      continue;
    }

    // combat creatures
    const flying = (t == 1 || t == 3 || t == 5 || t == 7);
    let tx: f32 = 0, ty: f32 = 0, tz: f32 = 0; let hasT = false; let td2: f32 = 1e18;
    if (own == 0) {
      // hostile: prefer player, else allied creatures
      let bd: f32 = 1e18; let bw = -1;
      for (let w = 0; w <= nRival; w++) {
        if (walive[w] == 0) continue;
        const dx = wx[w] - ex[i], dy = wy[w] - ey[i], dz = wz[w] - ez[i];
        const d2 = dx * dx + dy * dy + dz * dz;
        const bias: f32 = w == 0 ? 0.55 : 1.0;
        if (d2 * bias < bd) { bd = d2 * bias; bw = w; }
      }
      const ally = nearestHostile(i, 220.0);
      let aggro: f32 = 200.0;
      if (t == 1 || t == 3) aggro = 300.0; else if (t == 7) aggro = 380.0;
      let realD2: f32 = 1e18;
      if (bw >= 0) { const ax = wx[bw] - ex[i], ay = wy[bw] - ey[i], az = wz[bw] - ez[i]; realD2 = ax * ax + ay * ay + az * az; }
      if (bw >= 0 && realD2 < aggro * aggro) { tx = wx[bw]; ty = wy[bw]; tz = wz[bw]; hasT = true; td2 = realD2; }
      else if (ally >= 0) { tx = ex[ally]; ty = ey[ally]; tz = ez[ally]; hasT = true; const ddx = tx - ex[i], ddy = ty - ey[i], ddz = tz - ez[i]; td2 = ddx * ddx + ddy * ddy + ddz * ddz; }
    } else {
      if (t == 5) { etim[i] -= dt; if (etim[i] <= 0) { killCreature(i); continue; } }
      const h = nearestHostile(i, 420.0);
      if (h >= 0) { tx = ex[h]; ty = ey[h]; tz = ez[h]; hasT = true; const ddx = tx - ex[i], ddy = ty - ey[i], ddz = tz - ez[i]; td2 = ddx * ddx + ddy * ddy + ddz * ddz; }
      else {
        const opp = own == 1 ? 1 : 0;
        if (walive[opp] != 0 && opp <= nRival) { tx = wx[opp]; ty = wy[opp]; tz = wz[opp]; hasT = true; const ddx = tx - ex[i], ddy = ty - ey[i], ddz = tz - ez[i]; td2 = ddx * ddx + ddy * ddy + ddz * ddz; }
      }
    }

    let spd: f32 = 26.0;
    if (t == 0) spd = 24.0; else if (t == 1) spd = 74.0; else if (t == 2) spd = 20.0;
    else if (t == 3) spd = 82.0; else if (t == 5) spd = 58.0; else if (t == 7) spd = 66.0;

    if (hasT) {
      const dx = tx - ex[i], dy = ty - ey[i], dz = tz - ez[i];
      const d = Mathf.sqrt(td2) + 0.01;
      eyaw[i] = Mathf.atan2(dx, dz);
      const rng: f32 = (t == 0) ? 150.0 : ((t == 2) ? 170.0 : ((t == 7) ? 210.0 : 0.0));
      if (rng > 0 && d < rng) {
        // ranged: hold position and shoot
        evx[i] *= 0.9; evz[i] *= 0.9;
        if (ecd[i] <= 0) {
          ecd[i] = t == 7 ? rr(1.1, 2.0) : rr(1.6, 3.0);
          const sp2: f32 = t == 7 ? 190.0 : 140.0;
          const inv: f32 = 1.0 / d;
          const shots = t == 7 ? 3 : 1;
          for (let s2 = 0; s2 < shots; s2++) {
            addProj(ex[i], ey[i] + 4.0, ez[i],
              (dx * inv + rr(-0.08, 0.08)) * sp2, (dy * inv + 0.16 + rr(-0.05, 0.05)) * sp2, (dz * inv + rr(-0.08, 0.08)) * sp2,
              t == 7 ? 3 : 1, own, t == 7 ? 15.0 : 7.0, 16.0, 3.4);
          }
        }
      } else {
        const k = Mathf.min(1.0, dt * 2.0);
        evx[i] += (dx / d * spd - evx[i]) * k;
        evz[i] += (dz / d * spd - evz[i]) * k;
        if (flying) evy[i] += (dy / d * spd * 0.75 - evy[i]) * k;
        // melee
        if (d < 22.0 && ecd[i] <= 0) {
          ecd[i] = 1.45;
          let dmg: f32 = t == 1 ? 4.0 : (t == 2 ? 11.0 : (t == 3 ? 7.0 : (t == 5 ? 20.0 : (t == 7 ? 17.0 : 6.0))));
          {
            for (let w = 0; w <= nRival; w++) {
              if (walive[w] == 0 || w + 1 == own) continue;
              const ddx = wx[w] - ex[i], ddy = wy[w] - ey[i], ddz = wz[w] - ez[i];
              if (ddx * ddx + ddy * ddy + ddz * ddz < 26.0 * 26.0) hurtWizard(w, dmg);
            }
          }
          damageArea(ex[i], ey[i], ez[i], 20.0, dmg, own);
          burst(ex[i] + dx / d * 10.0, ey[i] + dy / d * 10.0, ez[i] + dz / d * 10.0, 6, 10.0, 2.0, 1.0, 0.6, 0.3, 0.35);
        }
      }
    } else {
      etim[i] -= dt;
      if (etim[i] <= 0) { etim[i] = rr(2.0, 5.0); eyaw[i] = rr(0, 6.2832); }
      const k = Mathf.min(1.0, dt * 1.2);
      evx[i] += (Mathf.sin(eyaw[i]) * spd * 0.45 - evx[i]) * k;
      evz[i] += (Mathf.cos(eyaw[i]) * spd * 0.45 - evz[i]) * k;
      if (flying) evy[i] += (Mathf.sin(ephase[i] * 0.3) * 8.0 - evy[i]) * k;
    }
    ex[i] = clampWorld(ex[i] + evx[i] * dt);
    ez[i] = clampWorld(ez[i] + evz[i] * dt);
    const gh = Mathf.max(heightAt(ex[i], ez[i]), SEA - 2.0);
    if (flying) {
      ey[i] += evy[i] * dt;
      const minY = gh + creatureFloat(t) * 0.6;
      if (ey[i] < minY) { ey[i] = minY; if (evy[i] < 0) evy[i] = 0; }
      if (ey[i] > 260.0) { ey[i] = 260.0; if (evy[i] > 0) evy[i] = 0; }
    } else {
      evy[i] -= 90.0 * dt; ey[i] += evy[i] * dt;
      const rest = gh + 3.0;
      if (ey[i] < rest) { ey[i] = rest; evy[i] = 0; }
    }
    if (ehp[i] <= 0) killCreature(i);
  }
}

// ---- projectiles -----------------------------------------------------------
function updateProjectiles(dt: f32): void {
  for (let i = 0; i < MAXP; i++) {
    if (palive[i] == 0) continue;
    const t = ptype[i];
    if (t == 1 || t == 3) pvy[i] -= 42.0 * dt;
    if (t == 2) pvy[i] -= 34.0 * dt;
    px_[i] += pvx[i] * dt; py_[i] += pvy[i] * dt; pz_[i] += pvz[i] * dt;
    plife[i] -= dt;
    // trail
    let cr: f32 = 1.0, cg: f32 = 0.55, cb: f32 = 0.18;
    if (t == 1) { cr = 0.55; cg = 0.95; cb = 0.35; }
    else if (t == 2) { cr = 1.0; cg = 0.45; cb = 0.12; }
    else if (t == 3) { cr = 1.0; cg = 0.32; cb = 0.5; }
    const nTrail = t == 2 ? 4 : 1;
    for (let k = 0; k < nTrail; k++)
      spawnPart(px_[i] + rr(-2, 2), py_[i] + rr(-2, 2), pz_[i] + rr(-2, 2), rr(-6, 6), rr(-2, 10), rr(-6, 6),
        t == 2 ? 0.7 : 0.34, t == 2 ? 6.5 : 3.4, cr, cg, cb, 0, 1.5);
    let hit = false;
    if (py_[i] <= heightAt(px_[i], pz_[i])) hit = true;
    if (py_[i] <= SEA - 1.0) hit = true;
    if (!hit) {
      for (let e = 0; e < MAXE; e++) {
        if (ealive[e] == 0 || eown[e] == pown[i] || etype[e] == 6) continue;
        const dx = ex[e] - px_[i], dy = ey[e] - py_[i], dz = ez[e] - pz_[i];
        if (dx * dx + dy * dy + dz * dz < 15.0 * 15.0) { hit = true; break; }
      }
    }
    if (!hit) {
      for (let w = 0; w <= nRival; w++) {
        if (walive[w] == 0 || w + 1 == pown[i]) continue;
        const dx = wx[w] - px_[i], dy = wy[w] - py_[i], dz = wz[w] - pz_[i];
        if (dx * dx + dy * dy + dz * dz < 13.0 * 13.0) {
          hurtWizard(w, pdmg[i]); hit = true; break;
        }
      }
    }
    if (!hit && plife[i] <= 0) hit = true;
    if (hit) {
      palive[i] = 0;
      damageArea(px_[i], py_[i], pz_[i], prad[i], pdmg[i], pown[i]);
      if (t == 2) {
        deform(px_[i], pz_[i], 74.0, 30.0, 0);
        burst(px_[i], py_[i], pz_[i], 80, 40.0, 9.0, 1.0, 0.6, 0.2, 1.6);
        shake = 1.6; pushEvent(18, px_[i], py_[i], pz_[i]);
      } else if (t == 0) {
        deform(px_[i], pz_[i], 16.0, 2.2, 0);
        burst(px_[i], py_[i], pz_[i], 22, 20.0, 4.4, 1.0, 0.6, 0.22, 0.7);
        pushEvent(19, px_[i], py_[i], pz_[i]);
      } else {
        burst(px_[i], py_[i], pz_[i], 14, 15.0, 3.4, cr, cg, cb, 0.6);
        pushEvent(19, px_[i], py_[i], pz_[i]);
      }
    }
  }
}

// ---- particles / orbs ------------------------------------------------------
function updateParticles(dt: f32): void {
  for (let i = 0; i < MAXPT; i++) {
    if (ql[i] <= 0) continue;
    ql[i] -= dt;
    if (ql[i] <= 0) continue;
    qvy[i] += qgrav[i] * dt;
    const dr: f32 = 1.0 - qdrag[i] * dt; const d2: f32 = dr < 0 ? 0 : dr;
    qvx[i] *= d2; qvy[i] *= d2; qvz[i] *= d2;
    qx[i] += qvx[i] * dt; qy[i] += qvy[i] * dt; qz[i] += qvz[i] * dt;
  }
}
function updateOrbs(dt: f32): void {
  for (let i = 0; i < MAXO; i++) {
    if (oalive[i] == 0 || oheld[i] >= 0) continue;
    ophase[i] += dt * 2.4;
    const g = Mathf.max(heightAt(ox[i], oz[i]), SEA) + 5.0 + Mathf.sin(ophase[i]) * 1.6;
    // settle toward hover height, but weakly so a nearby carpet can lift it
    oy[i] += (g - oy[i]) * Mathf.min(1.0, dt * 0.8);
    if (oy[i] < g - 2.0) oy[i] = g - 2.0;
  }
}

// ---- render buffer ---------------------------------------------------------
@inline function pushInst(x: f32, y: f32, z: f32, sx: f32, sy: f32, sz: f32,
                          r: f32, g: f32, b: f32, yaw: f32, glow: f32, shape: f32): void {
  if (instN >= MAXI) return;
  const o = instN * 12;
  INST[o] = x; INST[o + 1] = y; INST[o + 2] = z;
  INST[o + 3] = sx; INST[o + 4] = sy; INST[o + 5] = sz;
  INST[o + 6] = r; INST[o + 7] = g; INST[o + 8] = b;
  INST[o + 9] = yaw; INST[o + 10] = glow; INST[o + 11] = shape;
  instN++;
}
@inline function pushMap(x: f32, z: f32, kind: f32, sc: f32): void {
  if (mapN >= MAXMAP) return;
  const o = mapN * 4; MAP[o] = x; MAP[o + 1] = z; MAP[o + 2] = kind; MAP[o + 3] = sc; mapN++;
}
// Lowest ground under a footprint of radius r. A castle placed at the height
// of its own centre point hangs in the air on the downhill side of any slope,
// so the plinth is sunk to reach this instead.
function groundMin(x: f32, z: f32, r: f32): f32 {
  let m = heightAt(x, z);
  for (let k = 0; k < 8; k++) {
    const a: f32 = <f32>k * 0.7854;
    const h = heightAt(x + Mathf.cos(a) * r, z + Mathf.sin(a) * r);
    if (h < m) m = h;
  }
  return m;
}
const FOOT: f32 = 27.0;      // plinth half-width; must clear the tower ring
function drawCastle(w: i32): void {
  const gm = groundMin(cx[w], cz[w], FOOT);
  if (chp[w] <= 0) {
    const top = cy[w] + 3.5, bot = gm - 5.0;
    pushInst(cx[w], (top + bot) * 0.5, cz[w], 30, top - bot, 30, 0.22, 0.19, 0.17, 0, 0, 0);
    return;
  }
  const lv = clev[w];
  let r: f32 = 0.24, g: f32 = 0.34, b: f32 = 0.72;
  if (w != 0) { r = 0.70; g = 0.20; b = 0.16; }
  const dmgF = chp[w] / chpm[w];
  const base = cy[w];
  // Plinth: buried below the lowest ground it covers so it reads as cut into
  // the hill. The pad is levelled at placement, so this normally only has a
  // few units to make up — the clamp is for ground later blown away under it.
  const deck = base + 6.0;
  let bot = gm - 7.0;
  if (bot < base - 26.0) bot = base - 26.0;
  pushInst(cx[w], (deck + bot) * 0.5, cz[w], FOOT * 2.0, deck - bot, FOOT * 2.0,
           r * 0.5 + 0.14, g * 0.5 + 0.12, b * 0.5 + 0.10, 0, 0, 0);
  // a narrower skirt just under the deck reads as a stepped foundation
  pushInst(cx[w], deck - 1.6, cz[w], FOOT * 2.0 - 7.0, 5.0, FOOT * 2.0 - 7.0,
           r * 0.34 + 0.10, g * 0.34 + 0.09, b * 0.34 + 0.08, 0, 0, 0);
  const keepH: f32 = 14.0 + <f32>lv * 7.0;
  pushInst(cx[w], deck + keepH * 0.5, cz[w], 19, keepH, 19, r, g, b, 0, 0, 0);
  pushInst(cx[w], deck + 2.0 + keepH, cz[w], 15, 13, 15, r * 1.25, g * 1.25, b * 1.25, 0.785, 0.15, 2);
  const towers = 2 + lv;
  for (let k = 0; k < towers; k++) {
    const a: f32 = <f32>k / <f32>towers * 6.2832 + 0.4;
    const rad: f32 = 19.0;
    const txx = cx[w] + Mathf.cos(a) * rad, tzz = cz[w] + Mathf.sin(a) * rad;
    const th: f32 = 10.0 + <f32>lv * 3.4 + Mathf.sin(<f32>k * 2.1) * 3.0;
    pushInst(txx, deck - 2.0 + th * 0.5, tzz, 7, th, 7, r * 0.9, g * 0.9, b * 0.9, a, 0, 0);
    pushInst(txx, deck - 1.0 + th, tzz, 6.5, 8, 6.5, r * 1.4, g * 1.4, b * 1.4, a, 0.25, 2);
  }
  // banner glow scales with stored mana
  const glowH: f32 = 6.0 + <f32>lv * 2.0;
  pushInst(cx[w], deck + 12.0 + keepH, cz[w], 3.0, glowH, 3.0, 0.55, 0.85, 1.0, 0, 1.0, 1);
  if (dmgF < 0.6) {
    if (rnd() < 0.4) spawnPart(cx[w] + rr(-16, 16), base + rr(6, keepH), cz[w] + rr(-16, 16), rr(-3, 3), rr(6, 16), rr(-3, 3), 1.4, 5.0, 0.35, 0.33, 0.3, 2.0, 0.9);
  }
  pushMap(cx[w], cz[w], w == 0 ? 6.0 : 7.0, 1.0);
}
function drawCreature(i: i32): void {
  const t = etype[i], x = ex[i], y = ey[i], z = ez[i], ya = eyaw[i];
  const hurt: f32 = ehurt[i] > 0 ? 1.0 : 0.0;
  const ph = ephase[i];
  // sf/cf are the facing unit vector; +side is the creature's own right.
  //   forward = (sf, cf)      right = (cf, -sf)
  // Parts are kept flat and high-contrast, with dark or glowing eyes: the
  // renderer's edge pass keys off local colour contrast, so hard internal
  // borders are what give these a crisp pixel-art silhouette.
  const sf = Mathf.sin(ya), cf = Mathf.cos(ya);
  if (t == 0) {                       // sand worm: segmented chain
    for (let s = 0; s < 6; s++) {
      const off: f32 = <f32>s * 5.4;
      const sx2 = x - sf * off, sz2 = z - cf * off;
      const bob: f32 = Mathf.sin(ph - <f32>s * 0.8) * 3.2;
      const sc: f32 = 6.8 - <f32>s * 0.62;
      pushInst(sx2, y + bob + 1.0, sz2, sc, sc * 0.85, sc, 0.62 + hurt * 0.35, 0.52 - hurt * 0.2, 0.28, ya, hurt * 0.5, 1);
      if (s < 5) pushInst(sx2, y + bob + 1.0 + sc * 0.42, sz2, sc * 0.5, sc * 0.36, sc * 0.8, 0.40, 0.30, 0.15, ya, 0, 0);
    }
    const hb: f32 = Mathf.sin(ph) * 3.2;
    pushInst(x + sf * 3.4, y + hb + 0.2, z + cf * 3.4, 3.6, 1.8, 2.2, 0.28, 0.09, 0.09, ya, 0, 0);
    pushInst(x + sf * 2.4 + cf * 1.7, y + hb + 2.4, z + cf * 2.4 - sf * 1.7, 1.5, 1.5, 1.5, 0.04, 0.03, 0.04, ya, 0, 1);
    pushInst(x + sf * 2.4 - cf * 1.7, y + hb + 2.4, z + cf * 2.4 + sf * 1.7, 1.5, 1.5, 1.5, 0.04, 0.03, 0.04, ya, 0, 1);
  } else if (t == 1) {                // wasp
    pushInst(x, y, z, 3.4, 3.0, 5.0, 0.85 + hurt * 0.15, 0.72, 0.18, ya, hurt * 0.6, 1);
    pushInst(x - sf * 1.4, y, z - cf * 1.4, 3.2, 2.8, 1.2, 0.11, 0.08, 0.05, ya, 0, 0);
    pushInst(x - sf * 2.8, y, z - cf * 2.8, 2.5, 2.2, 1.1, 0.11, 0.08, 0.05, ya, 0, 0);
    pushInst(x - sf * 4.4, y, z - cf * 4.4, 1.4, 2.8, 1.4, 0.16, 0.12, 0.10, ya, 0, 2);
    pushInst(x + sf * 2.9, y + 0.3, z + cf * 2.9, 2.7, 2.5, 2.5, 0.26, 0.19, 0.07, ya, 0, 1);
    pushInst(x + sf * 3.7 + cf * 0.9, y + 0.7, z + cf * 3.7 - sf * 0.9, 1.2, 1.2, 1.2, 0.03, 0.03, 0.03, ya, 0, 1);
    pushInst(x + sf * 3.7 - cf * 0.9, y + 0.7, z + cf * 3.7 + sf * 0.9, 1.2, 1.2, 1.2, 0.03, 0.03, 0.03, ya, 0, 1);
    const fl = Mathf.sin(ph) * 0.9;
    pushInst(x, y + 2.2, z, 7.5, 0.4, 2.2, 0.9, 0.9, 0.95, ya + fl, 0.1, 0);
  } else if (t == 2) {                // troll
    const sk: f32 = 0.35 + hurt * 0.5;
    pushInst(x, y + 5.8, z, 8.0, 10.0, 6.5, sk, 0.42, 0.32, ya, hurt * 0.5, 0);
    pushInst(x, y + 12.6, z, 5.4, 4.6, 5.0, sk + 0.07, 0.48, 0.36, ya, hurt * 0.5, 0);
    pushInst(x + sf * 2.5 + cf * 1.3, y + 13.4, z + cf * 2.5 - sf * 1.3, 1.2, 1.2, 0.7, 1.0, 0.80, 0.22, ya, 0.7, 0);
    pushInst(x + sf * 2.5 - cf * 1.3, y + 13.4, z + cf * 2.5 + sf * 1.3, 1.2, 1.2, 0.7, 1.0, 0.80, 0.22, ya, 0.7, 0);
    pushInst(x + cf * 2.3, y + 15.8, z - sf * 2.3, 1.6, 3.6, 1.6, 0.88, 0.84, 0.72, ya, 0, 2);
    pushInst(x - cf * 2.3, y + 15.8, z + sf * 2.3, 1.6, 3.6, 1.6, 0.88, 0.84, 0.72, ya, 0, 2);
    const sw = Mathf.sin(ph) * 1.6;
    pushInst(x - cf * 5.5, y + 6.0 + sw, z + sf * 5.5, 2.6, 8.0, 2.6, 0.30, 0.36, 0.28, ya, 0, 0);
    pushInst(x + cf * 5.5, y + 6.0 - sw, z - sf * 5.5, 2.6, 8.0, 2.6, 0.30, 0.36, 0.28, ya, 0, 0);
    pushInst(x - cf * 2.2, y + 1.5, z + sf * 2.2, 2.9, 4.4, 3.2, 0.26, 0.31, 0.24, ya, 0, 0);
    pushInst(x + cf * 2.2, y + 1.5, z - sf * 2.2, 2.9, 4.4, 3.2, 0.26, 0.31, 0.24, ya, 0, 0);
  } else if (t == 3) {                // griffin
    pushInst(x, y, z, 4.6, 4.2, 8.0, 0.80 + hurt * 0.2, 0.68, 0.42, ya, hurt * 0.5, 1);
    pushInst(x + sf * 4.4, y + 1.6, z + cf * 4.4, 3.4, 3.2, 3.4, 0.94, 0.90, 0.78, ya, 0, 1);
    pushInst(x + sf * 6.1, y + 1.3, z + cf * 6.1, 1.7, 1.9, 2.6, 0.95, 0.70, 0.14, ya, 0, 2);
    pushInst(x + sf * 5.0 + cf * 1.2, y + 2.4, z + cf * 5.0 - sf * 1.2, 1.0, 1.0, 1.0, 0.04, 0.03, 0.03, ya, 0, 1);
    pushInst(x + sf * 5.0 - cf * 1.2, y + 2.4, z + cf * 5.0 + sf * 1.2, 1.0, 1.0, 1.0, 0.04, 0.03, 0.03, ya, 0, 1);
    pushInst(x - sf * 5.4, y - 0.4, z - cf * 5.4, 2.2, 2.2, 5.2, 0.68, 0.55, 0.32, ya, 0, 2);
    const fl = Mathf.sin(ph * 2.0) * 0.55;
    pushInst(x - cf * 7.0, y + 1.5 + fl * 4.0, z + sf * 7.0, 12.0, 1.2, 6.0, 0.92, 0.86, 0.7, ya, 0.05, 2);
    pushInst(x + cf * 7.0, y + 1.5 - fl * 4.0, z - sf * 7.0, 12.0, 1.2, 6.0, 0.92, 0.86, 0.7, ya, 0.05, 2);
  } else if (t == 4) {                // nest
    const pu: f32 = 1.0 + Mathf.sin(ph * 0.7) * 0.06;
    // sunk well below its own ground point so it never perches on a slope
    pushInst(x, y - 5.0, z, 19, 17, 19, 0.24, 0.16, 0.26, 0, 0, 0);
    pushInst(x, y + 9.0, z, 15 * pu, 17 * pu, 15 * pu, 0.40, 0.18, 0.44, ph * 0.1, 0.25, 2);
    for (let k = 0; k < 4; k++) {
      const a: f32 = <f32>k * 1.5708 + 0.7854;
      pushInst(x + Mathf.cos(a) * 7.6, y + 4.2, z + Mathf.sin(a) * 7.6, 2.2, 7.4, 2.2, 0.28, 0.11, 0.32, a, 0, 2);
    }
    pushInst(x, y + 20.0, z, 4, 4, 4, 0.85, 0.35, 1.0, 0, 1.0, 1);
  } else if (t == 5) {                // wraith
    pushInst(x, y, z, 5.0, 6.5, 5.0, 0.55, 0.42, 1.0, ya, 0.75, 1);
    pushInst(x, y + 3.7, z, 5.8, 5.2, 5.8, 0.28, 0.20, 0.64, ya, 0.2, 2);
    pushInst(x + sf * 1.9 + cf * 1.2, y + 1.1, z + cf * 1.9 - sf * 1.2, 1.1, 1.1, 1.1, 1.0, 0.95, 0.55, ya, 1.0, 1);
    pushInst(x + sf * 1.9 - cf * 1.2, y + 1.1, z + cf * 1.9 + sf * 1.2, 1.1, 1.1, 1.1, 1.0, 0.95, 0.55, ya, 1.0, 1);
    pushInst(x, y - 5.5, z, 6.0, 7.0, 6.0, 0.36, 0.28, 0.85, ya, 0.5, 2);
    if (rnd() < 0.4) spawnPart(x + rr(-4, 4), y + rr(-4, 4), z + rr(-4, 4), 0, rr(2, 8), 0, 0.6, 2.4, 0.5, 0.4, 1.0, 0, 1.0);
  } else if (t == 6) {                // balloon
    const isP = eown[i] == 1;
    pushInst(x, y, z, 9.0, 11.0, 9.0, isP ? 0.35 : 0.85, isP ? 0.6 : 0.3, isP ? 1.0 : 0.28, ya, 0.15, 1);
    pushInst(x, y + 5.6, z, 4.6, 3.4, 4.6, 0.92, 0.88, 0.72, ya, 0.1, 2);
    pushInst(x, y - 2.0, z, 9.4, 1.4, 9.4, 0.20, 0.16, 0.12, ya, 0, 0);
    pushInst(x, y - 6.0, z, 0.9, 6.0, 0.9, 0.28, 0.22, 0.16, ya, 0, 0);
    pushInst(x, y - 9.0, z, 4.0, 3.4, 4.0, 0.42, 0.32, 0.2, ya, 0, 0);
  } else if (t == 7) {                // dragon
    for (let s = 0; s < 4; s++) {
      const off: f32 = <f32>s * 7.0;
      const sx2 = x - sf * off, sz2 = z - cf * off;
      const sc: f32 = 8.0 - <f32>s * 1.3;
      const sy2 = y + Mathf.sin(ph - <f32>s) * 1.4;
      pushInst(sx2, sy2, sz2, sc, sc * 0.9, sc * 1.2, 0.55 + hurt * 0.4, 0.16, 0.18, ya, hurt * 0.5, 1);
      if (s > 0) pushInst(sx2, sy2 + sc * 0.52, sz2, sc * 0.26, sc * 0.55, sc * 0.72, 0.28, 0.07, 0.09, ya, 0, 2);
    }
    const fl = Mathf.sin(ph * 1.6) * 0.6;
    pushInst(x - cf * 11.0, y + 2.0 + fl * 6.0, z + sf * 11.0, 20.0, 1.6, 9.0, 0.35, 0.10, 0.14, ya, 0.05, 2);
    pushInst(x + cf * 11.0, y + 2.0 - fl * 6.0, z - sf * 11.0, 20.0, 1.6, 9.0, 0.35, 0.10, 0.14, ya, 0.05, 2);
    pushInst(x + sf * 8.0, y + 1.0, z + cf * 8.0, 5.0, 5.0, 7.0, 0.9, 0.4, 0.2, ya, 0.4, 2);
    pushInst(x + sf * 6.8 + cf * 1.8, y + 3.6, z + cf * 6.8 - sf * 1.8, 1.4, 3.2, 1.4, 0.86, 0.80, 0.66, ya, 0, 2);
    pushInst(x + sf * 6.8 - cf * 1.8, y + 3.6, z + cf * 6.8 + sf * 1.8, 1.4, 3.2, 1.4, 0.86, 0.80, 0.66, ya, 0, 2);
    pushInst(x + sf * 9.6 + cf * 1.5, y + 1.7, z + cf * 9.6 - sf * 1.5, 1.3, 1.3, 1.3, 1.0, 0.86, 0.20, ya, 1.0, 1);
    pushInst(x + sf * 9.6 - cf * 1.5, y + 1.7, z + cf * 9.6 + sf * 1.5, 1.3, 1.3, 1.3, 1.0, 0.86, 0.20, ya, 1.0, 1);
    pushInst(x - sf * 25.0, y, z - cf * 25.0, 2.4, 5.2, 2.4, 0.38, 0.09, 0.11, ya, 0, 2);
  }
  if (t != 6) {
    let mk: f32 = 2.0;
    if (eown[i] == 1) mk = 3.0; else if (eown[i] == 2) mk = 8.0;
    if (t == 4) mk = 5.0;
    pushMap(x, z, mk, t == 4 ? 1.6 : (t == 7 ? 1.5 : 1.0));
  } else pushMap(x, z, 9.0, 0.8);
}

function buildRender(): void {
  instN = 0; partN = 0; mapN = 0;
  // scenery (distance culled around the player)
  const cull: f32 = 1050.0;
  for (let i = 0; i < decCount; i++) {
    const dx = dx_[i] - wx[0], dz = dz_[i] - wz[0];
    if (dx * dx + dz * dz > cull * cull) continue;
    const h = heightAt(dx_[i], dz_[i]);
    if (h < 1.0) continue;                       // sank under water (crater/quake)
    const s = dsc[i];
    // Both are sunk past their own ground sample: the height is taken at one
    // point, so anything sitting exactly on it lifts off the downhill side.
    if (dkind[i] == 0) {
      pushInst(dx_[i], h + 5.0 * s, dz_[i], 1.5 * s, 14.0 * s, 1.5 * s, 0.36, 0.27, 0.16, drot[i], 0, 0);
      pushInst(dx_[i], h + 14.0 * s, dz_[i], 9.0 * s, 5.0 * s, 9.0 * s, 0.22, 0.44, 0.20, drot[i], 0, 2);
    } else {
      pushInst(dx_[i], h + 0.4 * s, dz_[i], 5.0 * s, 6.0 * s, 4.4 * s, 0.44, 0.41, 0.38, drot[i], 0, 0);
    }
  }
  for (let w = 0; w <= nRival; w++) drawCastle(w);
  for (let i = 0; i < MAXE; i++) if (ealive[i] != 0) drawCreature(i);
  // rival carpets
  for (let w = 1; w <= nRival; w++) {
    if (walive[w] == 0) continue;
    pushInst(wx[w], wy[w], wz[w], 13.0, 1.1, 17.0, 0.72, 0.16, 0.14, wyaw[w], 0.2, 0);
    pushInst(wx[w], wy[w] + 4.0, wz[w], 4.0, 6.0, 4.0, 0.9, 0.8, 0.6, wyaw[w], 0.1, 1);
    if (wshield[w] > 0) pushInst(wx[w], wy[w] + 2.0, wz[w], 20, 20, 20, 1.0, 0.5, 0.4, 0, 0.9, 1);
    pushMap(wx[w], wz[w], 1.0, 1.4);
  }
  // The player's own carpet and shield. Emitted as one contiguous run so the
  // renderer can drop it wholesale in first person — see carpetLo/carpetHi.
  carpetLo = instN;
  {
    const ya = wyaw[0], bob = Mathf.sin(gtime * 2.3) * 0.35;
    pushInst(wx[0], wy[0] - 2.7 + bob, wz[0], 16.6, 0.5, 20.6, 0.88, 0.65, 0.24, ya, 0.05, 0);
    pushInst(wx[0], wy[0] - 2.1 + bob, wz[0], 15.0, 0.9, 19.0, 0.13, 0.20, 0.56, ya, 0.02, 0);
    pushInst(wx[0], wy[0] - 1.6 + bob, wz[0], 5.2, 0.5, 6.6, 0.72, 0.20, 0.16, ya, 0.04, 0);
    if (wshield[0] > 0.0) pushInst(wx[0], wy[0], wz[0], 24, 24, 24, 0.42, 0.72, 1.0, 0, 0.8, 1);
  }
  carpetHi = instN;
  // mana orbs
  let orbN = 0;
  for (let i = 0; i < MAXO; i++) {
    if (oalive[i] == 0) continue;
    orbN++;
    const s: f32 = 2.0 + oamt[i] * 0.18;
    const pl: f32 = 0.75 + Mathf.sin(ophase[i] * 2.0) * 0.25;
    // gold = nobody's yet, white = yours and inbound on a balloon, red = theirs
    let orr: f32 = 1.00, org: f32 = 0.78, orb2: f32 = 0.16;
    let mk: f32 = 10.0;
    if (oown[i] == 1) { orr = 0.72; org = 0.92; orb2 = 1.00; mk = 4.0; }
    else if (oown[i] == 2) { orr = 1.00; org = 0.36; orb2 = 0.30; mk = 8.0; }
    pushInst(ox[i], oy[i], oz[i], s, s, s, orr * pl, org * pl, orb2 * pl, 0, 1.0, 1);
    pushMap(ox[i], oz[i], mk, 0.7);
    if (rnd() < 0.10) spawnPart(ox[i], oy[i], oz[i], rr(-3, 3), rr(3, 9), rr(-3, 3), 0.8, 1.8, 0.45, 0.8, 1.0, 0, 1.2);
  }
  // projectiles
  for (let i = 0; i < MAXP; i++) {
    if (palive[i] == 0) continue;
    const t = ptype[i];
    let r: f32 = 1.0, g: f32 = 0.6, b: f32 = 0.2, s: f32 = 3.0;
    if (t == 1) { r = 0.5; g = 1.0; b = 0.4; s = 2.6; }
    else if (t == 2) { r = 1.0; g = 0.45; b = 0.12; s = 9.0; }
    else if (t == 3) { r = 1.0; g = 0.3; b = 0.5; s = 3.4; }
    pushInst(px_[i], py_[i], pz_[i], s, s, s, r, g, b, 0, 1.0, 1);
  }
  // particles -> flat buffer
  for (let i = 0; i < MAXPT; i++) {
    if (ql[i] <= 0) continue;
    const f = ql[i] / qlm[i];
    const o = partN * 8;
    PART[o] = qx[i]; PART[o + 1] = qy[i]; PART[o + 2] = qz[i];
    PART[o + 3] = qs[i] * (0.35 + f * 0.9);
    PART[o + 4] = qr[i]; PART[o + 5] = qg[i]; PART[o + 6] = qb[i];
    PART[o + 7] = f * f;
    partN++;
  }
  pushMap(wx[0], wz[0], 0.0, 1.6);

  // ---- state block ----
  const g = heightAt(wx[0], wz[0]);
  ST[0] = wx[0]; ST[1] = wy[0]; ST[2] = wz[0];
  ST[3] = wyaw[0]; ST[4] = wpit[0]; ST[5] = wrol[0];
  ST[6] = Mathf.sqrt(wvx[0] * wvx[0] + wvy[0] * wvy[0] + wvz[0] * wvz[0]);
  ST[7] = whp[0]; ST[8] = 100.0;
  ST[9] = wmana[0]; ST[10] = manaCap(0);
  ST[11] = <f32>sel; ST[12] = <f32>NSPELL;
  const tgt = levelTarget();
  ST[13] = wstore[0] / tgt;
  let rb: f32 = 0;
  for (let w = 1; w <= nRival; w++) if (wstore[w] > rb) rb = wstore[w];
  ST[14] = rb / tgt; ST[15] = 1.0;
  // how much of the target your current fortress tier can physically hold
  // 53..56: the gap between the cooldown block (40..52) and the unlock block
  ST[53] = castleCap(0) / tgt;                 // how much of the target this tier holds
  ST[54] = wstore[0]; ST[55] = castleCap(0); ST[56] = tgt; ST[57] = BANK_FLOOR;
  ST[16] = <f32>clev[0]; ST[17] = <f32>level; ST[18] = <f32>status; ST[19] = shake;
  ST[20] = wshield[0]; ST[21] = whaste[0]; ST[22] = g; ST[23] = wy[0] - Mathf.max(g, SEA);
  ST[24] = kills; ST[25] = wstore[0]; ST[26] = totalMana;
  ST[27] = chp[0] / chpm[0];
  ST[28] = nRival >= 1 ? whp[1] / 100.0 : 0;
  const rdx = wx[1] - wx[0], rdz = wz[1] - wz[0];
  ST[29] = walive[1] != 0 ? Mathf.sqrt(rdx * rdx + rdz * rdz) : -1.0;
  ST[30] = <f32>orbN; ST[31] = 0;
  ST[32] = gtime;
  ST[33] = cx[0]; ST[34] = cz[0]; ST[35] = cy[0];
  ST[36] = <f32>nRival;
  ST[37] = <f32>TW; ST[38] = CELL;
  for (let i = 0; i < NSPELL; i++) {
    ST[40 + i] = SCD[i] > 0 ? CD[i] / SCD[i] : 0;
    ST[60 + i] = <f32>UNL[i];
    const price: f32 = i == 12 ? fortressCost(0) : SCOST[i];
    ST[80 + i] = wmana[0] >= price ? 1.0 : 0.0;
    ST[100 + i] = price;
  }
}

// ---- main step -------------------------------------------------------------
export function step(dt: f32): void {
  evtN = 0;
  let d = dt; if (d > 0.05) d = 0.05;
  if (status == 0) {
    gtime += d;
    for (let i = 0; i < NSPELL; i++) if (CD[i] > 0) CD[i] -= d;
    updatePlayer(d);
    for (let w = 0; w <= nRival; w++) if (walive[w] != 0 || w > 0) { if (w > 0) updateRival(w, d); }
    for (let w = 0; w <= nRival; w++) if (walive[w] != 0) wizardCommon(w, d);
    updateCreatures(d);
    updateProjectiles(d);
    updateOrbs(d);
    // trickle of new wildlife so the realm never empties
    spawnTimer -= d;
    if (spawnTimer <= 0) {
      spawnTimer = 9.0;
      let n = 0;
      for (let i = 0; i < MAXE; i++) if (ealive[i] != 0 && eown[i] == 0 && etype[i] != 4) n++;
      if (n < 7 + level) {
        const idx = findLand(6.0);
        addCreature(ri(4) == 3 ? 3 : ri(3), <f32>(idx % TW) * CELL, <f32>(idx / TW) * CELL, 0);
      }
    }
    // win / lose
    if (wstore[0] >= levelTarget()) status = 1;
    for (let w = 1; w <= nRival; w++) if (wstore[w] >= levelTarget()) status = 3;
    if (whp[0] <= 0) status = 2;
  }
  updateParticles(d);
  if (shake > 0) { shake -= d * 1.8; if (shake < 0) shake = 0; }
  buildRender();
}

export function setInput(fwd: f32, str: f32, up: f32, dyaw: f32, dpit: f32, fire: i32, brake: i32): void {
  iFwd = fwd; iStr = str; iUp = up; iYaw = dyaw; iPit = dpit; iFire = fire; iBrake = brake;
}
export function fireSelected(): void { if (status == 0) castSpell(0, sel); }

// ---- ABI: pointers into linear memory --------------------------------------
export function heightPtr(): usize { return changetype<usize>(HEIGHT); }
export function instPtr(): usize { return changetype<usize>(INST); }
export function partPtr(): usize { return changetype<usize>(PART); }
export function mapPtr(): usize { return changetype<usize>(MAP); }
export function evtPtr(): usize { return changetype<usize>(EVT); }
export function statePtr(): usize { return changetype<usize>(ST); }
export function instCount(): i32 { return instN; }
export function carpetInstLo(): i32 { return carpetLo; }
export function carpetInstHi(): i32 { return carpetHi; }
export function partCount(): i32 { return partN; }
export function mapCount(): i32 { return mapN; }
export function evtCount(): i32 { return evtN; }
export function dirtyLoRow(): i32 { return dirtyLo; }
export function dirtyHiRow(): i32 { return dirtyHi; }
export function clearDirty(): void { dirtyLo = TW; dirtyHi = -1; }
export function terrainWidth(): i32 { return TW; }
export function cellSize(): f32 { return CELL; }
export function worldSize(): f32 { return WORLD; }
