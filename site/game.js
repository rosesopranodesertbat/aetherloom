// ============================================================================
// AETHERLOOM — game shell: input, HUD, audio, level flow.
// Simulation = sim.wasm (freestanding WebAssembly). Rendering = engine.js (WebGPU).
// ============================================================================
import { Renderer, BLOCK_MODES } from './engine.js';

const SPELLS = [
  { n: 'Firebolt',  k: '1', g: '✦', d: 'Fast bolt. Scorches a small crater.' },
  { n: 'Storm',     k: '2', g: '⚡', d: 'Lightning that leaps between foes.' },
  { n: 'Crater',    k: '3', g: '◉', d: 'Punches a hole in the world.' },
  { n: 'Volcano',   k: '4', g: '▲', d: 'Raises a burning mountain.' },
  { n: 'Quake',     k: '5', g: '≋', d: 'Sends a ripple tearing outward.' },
  { n: 'Meteor',    k: '6', g: '☄', d: 'Calls a rock down from the sky.' },
  { n: 'Ward',      k: '7', g: '◇', d: 'Blunts incoming magic for a while.' },
  { n: 'Mend',      k: '8', g: '✚', d: 'Knits your wounds closed.' },
  { n: 'Haste',     k: '9', g: '»', d: 'The carpet flies faster.' },
  { n: 'Siphon',    k: '0', g: '◐', d: 'Pulls loose mana in. Drains rivals.' },
  { n: 'Wraith',    k: '-', g: '☥', d: 'Binds a servant to fight for you.' },
  { n: 'Sunburst',  k: '=', g: '☀', d: 'Smites every wild thing at once.' },
];
const EVT = {
  0:'cast',1:'zap',2:'boom',3:'release',4:'bigboom',5:'collapse',6:'rumble',7:'rumble',
  8:'incoming',9:'shimmer',10:'heal',11:'buzz',12:'whoosh',13:'wash',14:'pickup',
  15:'deposit',16:'levelup',17:'hurt',18:'bigboom',19:'pop'
};

// ------------------------------------------------------------------ audio
class Audio {
  constructor() { this.on = true; this.ctx = null; }
  boot() {
    if (this.ctx) return;
    try {
      this.ctx = new (window.AudioContext || window.webkitAudioContext)();
      this.master = this.ctx.createGain(); this.master.gain.value = 0.32;
      this.master.connect(this.ctx.destination);
      const b = this.ctx.createBuffer(1, this.ctx.sampleRate * 2, this.ctx.sampleRate);
      const dat = b.getChannelData(0);
      for (let i = 0; i < dat.length; i++) dat[i] = Math.random() * 2 - 1;
      this.noise = b;
      // ambient wind bed
      const src = this.ctx.createBufferSource(); src.buffer = b; src.loop = true;
      const f = this.ctx.createBiquadFilter(); f.type = 'lowpass'; f.frequency.value = 320;
      const g = this.ctx.createGain(); g.gain.value = 0.05;
      src.connect(f); f.connect(g); g.connect(this.master); src.start();
      this.wind = g;
    } catch (e) { this.ctx = null; }
  }
  n(dur, freq, gain, type = 'lowpass') {
    if (!this.ctx || !this.on) return;
    const c = this.ctx, t = c.currentTime;
    const s = c.createBufferSource(); s.buffer = this.noise;
    s.playbackRate.value = 0.7 + Math.random() * 0.6;
    const f = c.createBiquadFilter(); f.type = type; f.frequency.value = freq; f.Q.value = 1.1;
    const g = c.createGain();
    g.gain.setValueAtTime(gain, t); g.gain.exponentialRampToValueAtTime(0.0008, t + dur);
    s.connect(f); f.connect(g); g.connect(this.master); s.start(t); s.stop(t + dur + 0.02);
  }
  t(f0, f1, dur, gain, type = 'triangle') {
    if (!this.ctx || !this.on) return;
    const c = this.ctx, t = c.currentTime;
    const o = c.createOscillator(); o.type = type;
    o.frequency.setValueAtTime(f0, t); o.frequency.exponentialRampToValueAtTime(Math.max(20, f1), t + dur);
    const g = c.createGain();
    g.gain.setValueAtTime(0.0001, t);
    g.gain.exponentialRampToValueAtTime(gain, t + 0.012);
    g.gain.exponentialRampToValueAtTime(0.0008, t + dur);
    o.connect(g); g.connect(this.master); o.start(t); o.stop(t + dur + 0.02);
  }
  play(kind, dist) {
    if (!this.ctx || !this.on) return;
    const att = Math.max(0.06, 1 - dist / 900);
    switch (EVT[kind]) {
      case 'cast':     this.t(680, 240, 0.16, 0.10 * att, 'sawtooth'); this.n(0.10, 2400, 0.05 * att); break;
      case 'zap':      this.n(0.34, 3400, 0.22 * att, 'highpass'); this.t(1500, 380, 0.22, 0.09 * att, 'square'); break;
      case 'boom':     this.n(0.72, 240, 0.42 * att); this.t(150, 40, 0.5, 0.20 * att, 'sine'); break;
      case 'bigboom':  this.n(1.35, 170, 0.58 * att); this.t(105, 26, 0.95, 0.30 * att, 'sine'); break;
      case 'collapse': this.n(1.9, 130, 0.62 * att); this.t(80, 20, 1.5, 0.30 * att, 'sine'); break;
      case 'rumble':   this.n(1.7, 105, 0.46 * att); this.t(62, 24, 1.4, 0.22 * att, 'sine'); break;
      case 'incoming': this.t(1300, 130, 1.5, 0.16 * att, 'sawtooth'); break;
      case 'release':  this.t(760, 1240, 0.20, 0.07 * att, 'sine'); break;
      case 'pickup':   this.t(1080, 1660, 0.10, 0.075 * att, 'sine'); break;
      case 'deposit':  this.t(560, 900, 0.14, 0.06 * att, 'triangle'); break;
      case 'levelup':  [523, 659, 784, 1047].forEach((f, i) => setTimeout(() => this.t(f, f, 0.42, 0.085), i * 95)); break;
      case 'heal':     this.t(430, 940, 0.42, 0.09 * att, 'sine'); break;
      case 'shimmer':  this.t(1250, 2100, 0.44, 0.06 * att, 'sine'); this.n(0.4, 5200, 0.05, 'highpass'); break;
      case 'buzz':     this.t(210, 130, 0.24, 0.07 * att, 'square'); break;
      case 'whoosh':   this.n(0.5, 1000, 0.16 * att, 'bandpass'); break;
      case 'wash':     this.n(1.25, 2700, 0.3, 'highpass'); this.t(320, 1500, 1.0, 0.12); break;
      case 'hurt':     this.n(0.2, 500, 0.26); this.t(240, 90, 0.2, 0.13, 'square'); break;
      case 'pop':      this.n(0.16, 1500, 0.13 * att); break;
    }
  }
}

// ------------------------------------------------------------------ storage
const Store = {
  async get(k, dflt) {
    try { if (window.storage) { const r = await window.storage.get(k); return r ? JSON.parse(r.value) : dflt; } } catch (e) {}
    return (this._m && k in this._m) ? this._m[k] : dflt;
  },
  async set(k, v) {
    try { if (window.storage) { await window.storage.set(k, JSON.stringify(v)); return; } } catch (e) {}
    this._m = this._m || {}; this._m[k] = v;
  }
};

// ------------------------------------------------------------------ game
class Game {
  constructor() {
    this.canvas = document.getElementById('view');
    this.audio = new Audio();
    this.keys = new Set();
    this.mdx = 0; this.mdy = 0;
    this.sens = 0.0022;
    this.chase = false;
    this.paused = false;
    this.level = 1;
    this.best = 1;
    this.hurt = 0;
    this.fps = 60; this.frames = 0; this.fpsT = 0;
    this.acc = 0;
    this.mapDirty = true;
  }

  async boot(wasmSource) {
    const el = (id) => document.getElementById(id);
    this.el = {
      hud: el('hud'), hp: el('hp'), mana: el('mana'), manaMark: el('manaMark'),
      claimMe: el('claimMe'), claimRival: el('claimRival'), claimTarget: el('claimTarget'),
      bar: el('spellbar'), map: el('map'), title: el('title'), overlay: el('overlay'),
      otitle: el('otitle'), obody: el('obody'), obtn: el('obtn'), stat: el('stat'),
      err: el('err'), lvl: el('lvl'), tip: el('tip'), boot: el('boot')
    };
    this.mapCtx = this.el.map.getContext('2d');

    // ---- wasm
    let bytes;
    if (wasmSource instanceof Uint8Array) bytes = wasmSource;
    else {
      const r = await fetch(wasmSource);
      if (!r.ok) throw new Error(`Could not load ${wasmSource} (HTTP ${r.status}).`);
      bytes = new Uint8Array(await r.arrayBuffer());
    }
    const { instance } = await WebAssembly.instantiate(bytes, {});
    this.sim = instance.exports;
    this.TW = this.sim.terrainWidth();
    this.cell = this.sim.cellSize();
    this.world = this.sim.worldSize();

    // ---- renderer
    this.r = new Renderer(this.canvas);
    await this.r.init(this.TW, this.cell, 0);

    this.buildSpellBar();
    this.bindInput();
    this.best = await Store.get('aetherloom.best', 1);
    this.level = this.best;
    this.startLevel(this.level, true);

    this.el.boot.remove();
    this.el.title.classList.add('show');
    requestAnimationFrame((t) => this.loop(t));
  }

  views() {
    const m = this.sim.memory.buffer;
    if (this._buf !== m) {
      this._buf = m;
      this.H = new Float32Array(m, this.sim.heightPtr(), this.TW * this.TW);
      this.ST = new Float32Array(m, this.sim.statePtr(), 128);
      this.INST = new Float32Array(m, this.sim.instPtr(), 8192 * 12);
      this.PART = new Float32Array(m, this.sim.partPtr(), 4096 * 8);
      this.MAP = new Float32Array(m, this.sim.mapPtr(), 1024 * 4);
      this.EVTB = new Float32Array(m, this.sim.evtPtr(), 128 * 4);
    }
  }

  startLevel(n, quiet) {
    this.level = n;
    this.sim.init((Math.random() * 0xffffffff) >>> 0, n);
    this.views();
    this.r.uploadHeights(this.H, 0, this.TW - 1);
    this.sim.clearDirty();
    this.mapDirty = true;
    this.ended = false;
    this.el.overlay.classList.remove('show');
    this.el.lvl.textContent = `Realm ${n}`;
    const newly = SPELLS.filter((s, i) => this.ST[60 + i] > 0).length;
    this.el.tip.textContent = `${newly} spells attuned · claim ${Math.round(this.ST[15] * 100)}% of the realm's mana before your rival does`;
    if (!quiet) this.audio.play(16, 0);
  }

  buildSpellBar() {
    this.el.bar.innerHTML = '';
    this.slots = SPELLS.map((s, i) => {
      const b = document.createElement('button');
      b.className = 'slot'; b.type = 'button';
      b.innerHTML = `<span class="cd"></span><span class="key">${s.k}</span>` +
                    `<span class="glyph">${s.g}</span><span class="nm">${s.n}</span><span class="cost">—</span>`;
      b.title = `${s.n} — ${s.d}`;
      b.addEventListener('click', (e) => { e.preventDefault(); this.sim.selectSpell(i); });
      this.el.bar.appendChild(b);
      return { b, cd: b.querySelector('.cd'), cost: b.querySelector('.cost') };
    });
  }

  bindInput() {
    const c = this.canvas;
    addEventListener('keydown', (e) => {
      if (e.repeat) return;
      const k = e.key.toLowerCase();
      this.keys.add(k);
      const si = SPELLS.findIndex((s) => s.k === e.key);
      if (si >= 0) { this.sim.selectSpell(si); e.preventDefault(); }
      if (k === 'q') this.sim.cycleSpell(-1);
      if (k === 'e') this.sim.cycleSpell(1);
      if (k === 'c') this.chase = !this.chase;
      if (k === 'm') { this.audio.on = !this.audio.on; this.flash(this.audio.on ? 'Sound on' : 'Sound off'); }
      if (k === 'b') { this.r.bloom = !this.r.bloom; this.flash(this.r.bloom ? 'Bloom on' : 'Bloom off'); }
      if (k === 'v') {
        const n = (BLOCK_MODES.indexOf(this.r.block) + 1) % BLOCK_MODES.length;
        this.r.block = BLOCK_MODES[n];
        this.flash(this.r.block ? `Edge pixels ${this.r.block}px` : 'Edge pixels off');
      }
      if (k === 'p' || k === 'escape') this.setPaused(!this.paused);
      if (k === 'r' && e.shiftKey) this.startLevel(this.level);
      if (k === ' ') e.preventDefault();
    });
    addEventListener('keyup', (e) => this.keys.delete(e.key.toLowerCase()));
    addEventListener('blur', () => this.keys.clear());

    const wantLock = () => {
      if (this.paused || this.ended) return;
      if (document.pointerLockElement !== c && c.requestPointerLock) c.requestPointerLock();
    };
    c.addEventListener('mousedown', (e) => {
      if (this.el.title.classList.contains('show')) return;
      if (e.button === 0) { if (document.pointerLockElement === c) this.firing = true; else wantLock(); }
      if (e.button === 2) this.sim.cycleSpell(1);
    });
    addEventListener('mouseup', (e) => { if (e.button === 0) this.firing = false; });
    c.addEventListener('contextmenu', (e) => e.preventDefault());
    addEventListener('mousemove', (e) => {
      if (document.pointerLockElement === c) { this.mdx += e.movementX; this.mdy += e.movementY; }
    });
    addEventListener('wheel', (e) => { this.sim.cycleSpell(e.deltaY > 0 ? 1 : -1); e.preventDefault(); }, { passive: false });
    document.addEventListener('pointerlockchange', () => {
      const locked = document.pointerLockElement === c;
      document.body.classList.toggle('locked', locked);
      if (!locked) this.firing = false;
    });
    this.el.obtn.addEventListener('click', () => {
      if (this.won) { this.startLevel(this.level + 1); } else { this.startLevel(this.level); }
      wantLock();
    });
    document.getElementById('begin').addEventListener('click', () => {
      this.el.title.classList.remove('show');
      this.audio.boot(); wantLock();
    });
    addEventListener('resize', () => { this.r.resize(); this.mapDirty = true; });

    // touch: left half steers, right half casts
    let tId = null, tx0 = 0, ty0 = 0;
    c.addEventListener('touchstart', (e) => {
      this.audio.boot();
      for (const t of e.changedTouches) {
        if (t.clientX > innerWidth * 0.55) { this.firing = true; }
        else if (tId === null) { tId = t.identifier; tx0 = t.clientX; ty0 = t.clientY; }
      }
      e.preventDefault();
    }, { passive: false });
    c.addEventListener('touchmove', (e) => {
      for (const t of e.changedTouches) if (t.identifier === tId) {
        this.mdx += (t.clientX - tx0) * 0.6; this.mdy += (t.clientY - ty0) * 0.6;
        tx0 = t.clientX; ty0 = t.clientY;
      }
      e.preventDefault();
    }, { passive: false });
    c.addEventListener('touchend', (e) => {
      for (const t of e.changedTouches) { if (t.identifier === tId) tId = null; else this.firing = false; }
    });
  }

  setPaused(p) {
    this.paused = p;
    if (p && document.pointerLockElement) document.exitPointerLock();
    this.el.hud.classList.toggle('dim', p);
    if (p) this.flash('Paused — press P to resume');
    else this.flash('');
  }
  flash(msg) { this.el.stat.textContent = msg; }
}

// ------------------------------------------------------------------ loop
Game.prototype.readInputs = function (first) {
  const k = this.keys;
  const fwd = (k.has('w') || k.has('arrowup') ? 1 : 0) - (k.has('s') || k.has('arrowdown') ? 1 : 0);
  const str = (k.has('d') || k.has('arrowright') ? 1 : 0) - (k.has('a') || k.has('arrowleft') ? 1 : 0);
  const up = (k.has(' ') ? 1 : 0) - (k.has('shift') || k.has('control') ? 1 : 0);
  const brake = k.has('x') ? 1 : 0;
  let dy = 0, dp = 0;
  if (first) { dy = this.mdx * this.sens; dp = this.mdy * this.sens; this.mdx = 0; this.mdy = 0; }
  this.sim.setInput(fwd, str, up, dy, dp, 0, brake);
};

Game.prototype.camera = function () {
  const st = this.ST;
  const px = st[0], py = st[1], pz = st[2], yaw = st[3], pit = st[4], rol = st[5];
  const cp = Math.cos(pit), sp = Math.sin(pit);
  const fx = Math.sin(yaw) * cp, fy = sp, fz = Math.cos(yaw) * cp;
  const rx = Math.cos(yaw), ry = 0, rz = -Math.sin(yaw);
  let ux = fy * rz - fz * ry, uy = fz * rx - fx * rz, uz = fx * ry - fy * rx;
  const cr = Math.cos(rol), sr = Math.sin(rol);
  const ux2 = ux * cr + rx * sr, uy2 = uy * cr + ry * sr, uz2 = uz * cr + rz * sr;
  let ex = px, ey = py + 3.0, ez = pz;
  if (this.chase) { ex = px - fx * 46 + ux * 12; ey = py - fy * 46 + uy * 12 + 6; ez = pz - fz * 46 + uz * 12; }
  const sh = st[19];
  if (sh > 0) {
    const a = sh * 2.4;
    ex += (Math.random() - 0.5) * a; ey += (Math.random() - 0.5) * a; ez += (Math.random() - 0.5) * a;
  }
  return { ex, ey, ez, cx: ex + fx, cy: ey + fy, cz: ez + fz, ux: ux2, uy: uy2, uz: uz2 };
};

Game.prototype.drawMinimap = function () {
  const ctx = this.mapCtx, S = 168, N = 84, st = this.ST;
  if (!this.mapOff) {
    this.mapOff = document.createElement('canvas');
    this.mapOff.width = N; this.mapOff.height = N;
    this.mapOffCtx = this.mapOff.getContext('2d');
    this.mapImg = this.mapOffCtx.createImageData(N, N);
  }
  if (this.mapDirty) {
    const d = this.mapImg.data, step = this.TW / N;
    for (let j = 0; j < N; j++) for (let i = 0; i < N; i++) {
      const h = this.H[((j * step) | 0) * this.TW + ((i * step) | 0)];
      let r, g, b;
      if (h < -1) { const t = Math.min(1, -h / 30); r = 14 + (1 - t) * 26; g = 34 + (1 - t) * 50; b = 62 + (1 - t) * 44; }
      else if (h < 8) { r = 178; g = 152; b = 98; }
      else if (h < 52) { r = 74 + h * 0.7; g = 96 + h * 0.5; b = 50; }
      else if (h < 118) { r = 106; g = 96; b = 84; }
      else { r = 210; g = 205; b = 193; }
      const o = (j * N + i) * 4;
      d[o] = r; d[o + 1] = g; d[o + 2] = b; d[o + 3] = 255;
    }
    this.mapOffCtx.putImageData(this.mapImg, 0, 0);
    this.mapDirty = false;
  }
  ctx.clearRect(0, 0, S, S);
  ctx.save();
  ctx.beginPath(); ctx.arc(S / 2, S / 2, S / 2 - 3, 0, 7); ctx.clip();
  ctx.imageSmoothingEnabled = false;   // chunky, to match the scene
  ctx.drawImage(this.mapOff, 0, 0, N, N, 0, 0, S, S);
  const n = this.sim.mapCount(), M = this.MAP, k2 = S / this.world;
  const COL = { 0: '#f2d580', 1: '#e5493a', 2: '#241b12', 3: '#7ad1a8', 4: '#84e6ff', 5: '#c47ce8', 6: '#5686ff', 7: '#ff5f3f', 8: '#e5493a', 9: '#d8c39a' };
  for (let i = 0; i < n; i++) {
    const x = M[i * 4] * k2, z = M[i * 4 + 1] * k2, kind = M[i * 4 + 2] | 0, sc = M[i * 4 + 3];
    ctx.fillStyle = COL[kind] || '#888';
    if (kind === 6 || kind === 7) {
      ctx.fillRect(x - 4, z - 4, 8, 8);
      ctx.strokeStyle = 'rgba(9,12,28,.85)'; ctx.lineWidth = 1.2; ctx.strokeRect(x - 4, z - 4, 8, 8);
    } else if (kind === 0) {
      ctx.save(); ctx.translate(x, z); ctx.rotate(st[3]);
      ctx.beginPath(); ctx.moveTo(0, -7); ctx.lineTo(4.6, 5.5); ctx.lineTo(0, 2.8); ctx.lineTo(-4.6, 5.5); ctx.closePath();
      ctx.fill(); ctx.strokeStyle = 'rgba(9,12,28,.9)'; ctx.lineWidth = 1; ctx.stroke(); ctx.restore();
    } else {
      ctx.beginPath(); ctx.arc(x, z, 1.6 * sc + 0.7, 0, 7); ctx.fill();
    }
  }
  ctx.restore();
};

Game.prototype.updateHUD = function () {
  const st = this.ST, e = this.el;
  e.hp.style.width = Math.max(0, Math.min(100, st[7] / st[8] * 100)) + '%';
  const mf = Math.max(0, Math.min(1, st[9] / st[10]));
  e.mana.style.width = mf * 100 + '%';
  e.manaMark.style.left = Math.min(100, 45 / st[10] * 100) + '%';
  e.claimMe.style.width = Math.min(100, st[13] * 100) + '%';
  e.claimRival.style.width = Math.min(100, st[14] * 100) + '%';
  e.claimTarget.style.left = Math.min(100, st[15] * 100) + '%';
  const sel = st[11] | 0;
  for (let i = 0; i < this.slots.length; i++) {
    const s = this.slots[i], unl = st[60 + i] > 0, aff = st[80 + i] > 0, cd = st[40 + i];
    s.b.classList.toggle('sel', i === sel);
    s.b.classList.toggle('locked', !unl);
    s.b.classList.toggle('poor', unl && !aff);
    s.cd.style.height = Math.max(0, Math.min(1, cd)) * 100 + '%';
    const c = st[100 + i] | 0;
    if (s._c !== c) { s.cost.textContent = c; s._c = c; }
  }
  e.stat.dataset.fps = this.fps;
};

Game.prototype.loop = function (t) {
  requestAnimationFrame((tt) => this.loop(tt));
  if (this.r.lost) { this.fail(`The GPU device was lost (${this.r.lost}). Reload to continue.`); return; }
  const dt = Math.min(0.1, this.last ? (t - this.last) / 1000 : 0.016);
  this.last = t;
  this.frames++; this.fpsT += dt;
  if (this.fpsT > 0.5) { this.fps = Math.round(this.frames / this.fpsT); this.frames = 0; this.fpsT = 0; }

  this.views();
  const playing = !this.paused && !this.ended && !this.el.title.classList.contains('show');
  if (playing) {
    this.acc += dt;
    let steps = 0;
    while (this.acc >= 1 / 60 && steps < 4) {
      this.readInputs(steps === 0);
      if (this.firing) this.sim.fireSelected();
      this.sim.step(1 / 60);
      this.acc -= 1 / 60; steps++;
    }
    // terrain edits -> GPU
    const lo = this.sim.dirtyLoRow(), hi = this.sim.dirtyHiRow();
    if (hi >= lo) { this.r.uploadHeights(this.H, lo, hi); this.sim.clearDirty(); this.mapDirty = true; }
    // audio events
    const en = this.sim.evtCount(), st = this.ST;
    for (let i = 0; i < en; i++) {
      const k = this.EVTB[i * 4] | 0;
      const dx = this.EVTB[i * 4 + 1] - st[0], dy = this.EVTB[i * 4 + 2] - st[1], dz = this.EVTB[i * 4 + 3] - st[2];
      this.audio.play(k, Math.hypot(dx, dy, dz));
      if (k === 17) this.hurt = 1;
    }
  }
  this.hurt = Math.max(0, this.hurt - dt * 2.6);

  const st = this.ST;
  // castle positions for territory tint, harvested from the minimap blips
  const castles = [];
  const mn = this.sim.mapCount();
  for (let i = 0; i < mn && castles.length < 3; i++) {
    const kind = this.MAP[i * 4 + 2] | 0;
    if (kind === 6) castles.push([this.MAP[i * 4], this.MAP[i * 4 + 1], 300, 0]);
    else if (kind === 7) castles.push([this.MAP[i * 4], this.MAP[i * 4 + 1], 300, 1]);
  }
  const sunT = 0.35;
  // First person means first person: drop the player's own carpet rather than
  // parking it across the bottom of the screen.
  const fp = !this.chase;
  this.r.frame(this.camera(), this.PART, this.sim.partCount(), this.INST, this.sim.instCount(), {
    fov: 1.16, time: st[32],
    sun: [Math.cos(sunT) * 0.55, 0.70, Math.sin(sunT) * 0.45], sunI: 1.0,
    fog: [0.70, 0.82, 0.95], fogD: 0.0009,
    skipLo: fp ? this.sim.carpetInstLo() : -1,
    skipHi: fp ? this.sim.carpetInstHi() : -1,
    exposure: 1.12, bloomStrength: 0.42, waterAlpha: 1.0,
    hurt: Math.max(this.hurt, st[7] < 30 ? 0.24 + 0.10 * Math.sin(t * 0.006) : 0),
    castles
  });
  this.drawMinimap();
  this.updateHUD();

  if (playing && st[18] !== 0) this.finish(st[18] | 0);
};

Game.prototype.finish = async function (status) {
  this.ended = true;
  this.won = status === 1;
  if (document.pointerLockElement) document.exitPointerLock();
  const e = this.el;
  if (status === 1) {
    e.otitle.textContent = 'Realm claimed';
    e.obody.textContent = `You banked ${Math.round(this.ST[13] * 100)}% of the realm's mana and held the sky. Realm ${this.level + 1} awaits — a new spell comes with it.`;
    e.obtn.textContent = `Fly to realm ${this.level + 1}`;
    this.best = Math.max(this.best, this.level + 1);
    await Store.set('aetherloom.best', this.best);
    this.audio.play(16, 0);
  } else if (status === 3) {
    e.otitle.textContent = 'Your rival claimed it first';
    e.obody.textContent = `They banked ${Math.round(this.ST[14] * 100)}% while you managed ${Math.round(this.ST[13] * 100)}%. Kill their balloons and crack their keep — a broken castle spills everything it holds.`;
    e.obtn.textContent = 'Try this realm again';
    this.audio.play(5, 0);
  } else {
    e.otitle.textContent = 'The carpet falls';
    e.obody.textContent = 'Fly higher when the swarms thicken, and go home to your keep to mend and re-arm.';
    e.obtn.textContent = 'Try this realm again';
    this.audio.play(5, 0);
  }
  e.overlay.classList.add('show');
};

Game.prototype.fail = function (msg) {
  this.el.err.textContent = msg;
  this.el.err.classList.add('show');
};

// ------------------------------------------------------------------ boot
export async function start(wasmSource) {
  const g = new Game();
  window.__game = g;
  try {
    await g.boot(wasmSource);
  } catch (err) {
    const b = document.getElementById('boot');
    if (b) b.remove();
    const e = document.getElementById('err');
    e.innerHTML = `<strong>Aetherloom could not start.</strong><br>${String(err && err.message || err)}` +
      `<br><br>This build needs WebGPU. Chrome or Edge 113+, Firefox 141+, or Safari 18.2+ will run it. ` +
      `If you are on one of those, check that hardware acceleration is enabled.`;
    e.classList.add('show');
    console.error(err);
  }
}
