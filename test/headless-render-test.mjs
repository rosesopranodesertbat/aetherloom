import fs from 'fs';
const errs = [];
const note = (m) => { errs.push(m); };

// ---------- GPU constant enums
global.GPUBufferUsage = { UNIFORM:1, COPY_DST:2, VERTEX:4, INDEX:8, STORAGE:16 };
global.GPUTextureUsage = { TEXTURE_BINDING:1, COPY_DST:2, RENDER_ATTACHMENT:4 };
global.GPUShaderStage = { VERTEX:1, FRAGMENT:2, COMPUTE:4 };

const stub = (name, over={}) => new Proxy(over, {
  get(t, k) {
    if (k in t) return t[k];
    if (k === 'then' || k === Symbol.toPrimitive) return undefined;
    return (...a) => stub(`${name}.${String(k)}`);
  }
});

let buffers = new Map(), textures = new Map();
const queue = {
  writeBuffer(buf, off, data, dOff, size) {
    const cap = buffers.get(buf) ?? Infinity;
    if (off % 4) note(`writeBuffer offset ${off} not 4-aligned`);
    let bytes;
    if (data.BYTES_PER_ELEMENT) {
      const es = data.BYTES_PER_ELEMENT;
      const n = size === undefined ? data.length - (dOff||0) : size;
      if ((dOff||0) + n > data.length) note(`writeBuffer reads past source (${(dOff||0)+n} > ${data.length})`);
      bytes = n * es;
    } else {
      bytes = size === undefined ? data.byteLength - (dOff||0) : size;
      if ((dOff||0) + bytes > data.byteLength) note(`writeBuffer reads past ArrayBuffer`);
    }
    if (bytes % 4) note(`writeBuffer size ${bytes} not 4-aligned`);
    if (off + bytes > cap) note(`writeBuffer overflow: ${off}+${bytes} > buffer ${cap}`);
  },
  writeTexture(dst, data, layout, size) {
    const t = textures.get(dst.texture);
    if (layout.bytesPerRow % 256) note(`writeTexture bytesPerRow ${layout.bytesPerRow} not 256-aligned`);
    const need = (layout.offset||0) + layout.bytesPerRow * (size.height - 1) + size.width * 4;
    const have = data.byteLength;
    if (need > have) note(`writeTexture reads past source: need ${need} have ${have}`);
    if (t && (dst.origin.y + size.height > t.h)) note(`writeTexture rows exceed texture height`);
  },
  submit() {}
};
const device = stub('device', {
  queue,
  lost: new Promise(() => {}),
  createBuffer(d) { const b = stub('buffer', { destroy(){} }); buffers.set(b, d.size); return b; },
  createTexture(d) {
    const t = stub('texture', { destroy(){}, createView: () => stub('view') });
    textures.set(t, { w: d.size[0], h: d.size[1] }); return t;
  },
  createShaderModule(){ return stub('module'); },
  createCommandEncoder(){ return stub('enc', {
    beginRenderPass(d){
      if (d.colorAttachments?.[0]?.resolveTarget === undefined && d.depthStencilAttachment) {}
      return stub('pass', { end(){} });
    },
    finish(){ return stub('cmd'); }
  }); }
});
Object.defineProperty(global,'navigator',{value:{ gpu: {
  getPreferredCanvasFormat: () => 'bgra8unorm',
  requestAdapter: async () => stub('adapter', { info:{vendor:'test'}, requestDevice: async () => device })
} },configurable:true,writable:true});

// ---------- DOM
const mkEl = (id) => {
  const e = {
    id, style: new Proxy({}, { set(t,k,v){ t[k]=v; return true; } }),
    dataset: {}, textContent: '', innerHTML: '', width: 168, height: 168,
    clientWidth: 1440, clientHeight: 900,
    classList: { s:new Set(),
      add(...c){c.forEach(x=>this.s.add(x))}, remove(...c){c.forEach(x=>this.s.delete(x))},
      toggle(c,f){ f===undefined ? (this.s.has(c)?this.s.delete(c):this.s.add(c)) : (f?this.s.add(c):this.s.delete(c)); },
      contains(c){return this.s.has(c)} },
    addEventListener(){}, removeEventListener(){}, remove(){}, appendChild(){},
    querySelector(){ return mkEl('q'); },
    getContext(kind){
      if (kind === 'webgpu') return stub('ctx', { configure(){}, getCurrentTexture: () => stub('tex',{createView:()=>stub('v')}) });
      return stub('2d', {
        createImageData: (w,h) => ({ data: new Uint8ClampedArray(w*h*4), width:w, height:h }),
        putImageData(){}, drawImage(){}, clearRect(){}, save(){}, restore(){}, beginPath(){},
        arc(){}, clip(){}, fill(){}, stroke(){}, fillRect(){}, strokeRect(){}, translate(){},
        rotate(){}, moveTo(){}, lineTo(){}, closePath(){},
      });
    },
    requestPointerLock(){}
  };
  return e;
};
const els = {};
global.document = {
  getElementById: (id) => (els[id] = els[id] || mkEl(id)),
  createElement: (t) => mkEl(t),
  addEventListener(){}, exitPointerLock(){}, pointerLockElement: null,
  body: mkEl('body')
};
global.devicePixelRatio = 1;
global.AudioContext = null;
global.window = global;
const localStorageValues = new Map();
Object.defineProperty(global, 'localStorage', {
  configurable: true,
  value: {
    getItem(key) {
      return localStorageValues.has(key) ? localStorageValues.get(key) : null;
    },
    setItem(key, value) {
      localStorageValues.set(key, String(value));
    },
  },
});
global.addEventListener = () => {};
global.innerWidth = 1440;
let rafCbs = [];
global.requestAnimationFrame = (cb) => { rafCbs.push(cb); return rafCbs.length; };
global.fetch = async (p) => ({ ok:true, status:200, arrayBuffer: async () => fs.readFileSync('site/'+p.replace('./','')).buffer });

// ---------- run
const { start } = await import('../site/game.js');
await start('./sim.wasm');
const g = global.window.__game;
if (els.err && els.err.innerHTML) console.log('ERR PANEL:', els.err.innerHTML.slice(0,400));
if (!g || !g.sim) { console.log('BOOT FAILED'); process.exit(1); }
console.log('booted. terrain', g.TW, 'cell', g.cell, 'world', g.world);

// Prototype meshes: the builders drop vertices and triangles silently when a
// mesh outgrows its arrays, so a shape that overflows just renders with holes
// and nothing says why. Check every one fits and that no index dangles.
{
  const names = ['cuboid','sphere','cone','shadow','cylinder','frustum','wedge','frond','boulder','crenels'];
  g.sim.buildMeshes();
  const shapes = [];
  for (let i = 0; i < g.sim.shapeCount(); i++) {
    const verts = g.sim.meshVertFloats(i) / 6, idx = g.sim.meshIdxCount(i);
    const name = names[i] || `shape${i}`;
    if (verts === 0 || idx === 0) note(`mesh ${name} is empty`);
    let worst = -1;
    const view = new Uint16Array(g.sim.memory.buffer, g.sim.meshIdxPtr(i), idx);
    for (const v of view) if (v > worst) worst = v;
    if (worst >= verts) note(`mesh ${name} index ${worst} past its ${verts} vertices — the builder truncated`);
    if (idx % 3 !== 0) note(`mesh ${name} has ${idx} indices, not a whole number of triangles`);
    shapes.push(`${name} ${verts}v/${idx}i`);
  }
  console.log('meshes:', shapes.join('  '));
}

// drive frames: title screen, then gameplay with input and casting
let t = 0;
const step = (n, setup) => {
  for (let i = 0; i < n; i++) {
    setup && setup(i);
    t += 16.7;
    const cbs = rafCbs; rafCbs = [];
    for (const cb of cbs) cb(t);
  }
};
const stepMs = (milliseconds) => {
  t += milliseconds;
  const cbs = rafCbs; rafCbs = [];
  for (const cb of cbs) cb(t);
};
step(3);                                   // title screen frames
g.el.title.classList.remove('show');       // "begin"
console.log('entering play...');

// A hitch longer than the old 100 ms clamp must survive the per-frame
// 32-tick catch-up bound. A 500 ms gap is exactly 64 authoritative ticks:
// process 32 now, retain 32, then drain them on a zero-elapsed render.
{
  const before = g.sim.simulationTick();
  stepMs(500);
  const afterFirst = g.sim.simulationTick();
  if (afterFirst - before !== 32) {
    note(`500ms hitch first frame advanced ${afterFirst - before} ticks, expected bounded 32`);
  }
  if (g.acc < 31 * g.tickDt) {
    note(`500ms hitch discarded backlog: retained ${g.acc.toFixed(6)}s`);
  }
  stepMs(0);
  const afterDrain = g.sim.simulationTick();
  if (afterDrain - before !== 64) {
    note(`500ms hitch recovered ${afterDrain - before} ticks, expected all 64`);
  }
  if (g.acc >= g.tickDt) {
    note(`500ms hitch left ${g.acc.toFixed(6)}s after bounded drain`);
  }
}

step(240, (i) => {
  g.keys.clear();
  g.keys.add('w'); if (i % 40 < 12) g.keys.add(' ');
  if (i % 30 === 0) g.mdx += 14;
  g.firing = (i % 20) < 6;
  if (i % 55 === 0) g.sim.selectSpell(i % 12);
});
const st = g.ST;
console.log(`after 240 frames: hp=${st[7].toFixed(0)} mana=${st[9].toFixed(0)} claim=${(st[13]*100).toFixed(1)}% inst=${g.sim.instCount()} part=${g.sim.partCount()} blips=${g.sim.mapCount()} status=${st[18]}`);
// heavy terrain-deforming spells to exercise the dirty-row upload path
for (const sp of [2,3,4,5]) { g.sim.selectSpell(sp); g.firing = true; step(30); }
console.log(`after terrain spells: inst=${g.sim.instCount()} part=${g.sim.partCount()}`);
g.chase = true; step(20);
g.setPaused(true); step(5); g.setPaused(false); step(5);
const uniqueErrs = [...new Set(errs)];
if (uniqueErrs.length) {
  console.error('\nvalidation errors:\n  - ' + uniqueErrs.join('\n  - '));
  process.exitCode = 1;
} else {
  console.log('\nvalidation errors: NONE');
}
