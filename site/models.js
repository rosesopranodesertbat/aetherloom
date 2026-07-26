// AETHERLOOM — deterministic procedural-model gallery.
// The preview builder lives in sim.wasm; this page deliberately uses the same
// Renderer, mesh prototypes, instance ABI, shaders, and post stack as the game.
import { Renderer } from './engine.js';

const CATALOG = [
  { label: 'Sand Worm', slug: 'sand-worm', group: 'Creatures', copy: 'Segmented desert predator with lapped chitin, a toothed maw, and variant-driven surface detail.' },
  { label: 'Wasp', slug: 'wasp', group: 'Creatures', copy: 'Aerial hunter built from a pinched thorax, translucent wings, striped abdomen, legs, and stinger.' },
  { label: 'Troll', slug: 'troll', group: 'Creatures', copy: 'Heavy ground creature with an asymmetrical stone-hide silhouette and broad, weight-bearing limbs.' },
  { label: 'Griffin', slug: 'griffin', group: 'Creatures', copy: 'Feathered forequarters, deep-set eyes, hooked bill, fur transition, talons, and a swept wing pose.' },
  { label: 'Nest', slug: 'nest', group: 'Creatures', copy: 'An organic spawning mass assembled from layered scutes, roots, egg sacs, and a receding maw.' },
  { label: 'Wraith', slug: 'wraith', group: 'Creatures', copy: 'A bound spectral servant with a tapering body, hooded face, and emissive magical accents.' },
  { label: 'Balloon', slug: 'balloon', group: 'Creatures', copy: 'Mana courier with a sewn, vented envelope, suspension lines, basket, and faction detailing.' },
  { label: 'Dragon', slug: 'dragon', group: 'Creatures', copy: 'Long airborne body with travelling-wave segments, plated belly, horns, jaw, frills, and membrane wings.' },
  { label: 'Villager', slug: 'villager', group: 'Keep Folk', copy: 'Small keep resident with a readable layered outfit and a deliberately non-combat silhouette.' },
  { label: 'Soldier', slug: 'soldier', group: 'Keep Folk', copy: 'Fortress defender carrying a rimmed shield and spear, posed for a clean combat read at game distance.' },
  { label: 'Castle — Tier 1', slug: 'castle-tier-1', group: 'Fortresses', copy: 'The first keep tier: compact walls, gate, towers, and a central stronghold.' },
  { label: 'Castle — Tier 2', slug: 'castle-tier-2', group: 'Fortresses', copy: 'A growing keep with stronger enclosure rhythm and a taller central silhouette.' },
  { label: 'Castle — Tier 3', slug: 'castle-tier-3', group: 'Fortresses', copy: 'Mid-tier fortress with expanded walls, towers, parapets, and variant-specific proportions.' },
  { label: 'Castle — Tier 4', slug: 'castle-tier-4', group: 'Fortresses', copy: 'A mature fortified complex whose ring, roofs, gate bearing, and stone tint vary deterministically.' },
  { label: 'Castle — Tier 5', slug: 'castle-tier-5', group: 'Fortresses', copy: 'Late-game keep with a dense skyline, deep battlements, and a dominant donjon.' },
  { label: 'Castle — Tier 6', slug: 'castle-tier-6', group: 'Fortresses', copy: 'The maximum visual fortress tier, shown with its full wall and tower vocabulary.' },
  { label: 'Castle Ruin', slug: 'castle-ruin', group: 'Fortresses', copy: 'The collapsed keep state: broken enclosure, reduced skyline, rubble, and exposed interior.' },
  { label: 'Palm', slug: 'palm', group: 'Scenery', copy: 'Curved notched trunk, root flare, coconuts, dead growth, and a jittered crown of drooping fronds.' },
  { label: 'Boulder', slug: 'boulder', group: 'Scenery', copy: 'Overlapping weathered masses and an angular slab, varied by scale, spin, and stone tint.' },
  { label: 'Burnt Stump', slug: 'burnt-stump', group: 'Scenery', copy: 'The fire-damaged scenery state, with a low charred trunk and broken remains.' },
  { label: 'Player Carpet', slug: 'player-carpet', group: 'Carpets', copy: 'The player’s woven flying carpet, presented outside the first-person hide range.' },
  { label: 'Rival Carpet', slug: 'rival-carpet', group: 'Carpets', copy: 'The rival wizard’s carpet and rider treatment with opposing faction colours.' },
  { label: 'Unclaimed Mana Orb', slug: 'unclaimed-mana-orb', group: 'Mana', copy: 'Loose gold mana: neutral until Claim magic marks it for a balloon fleet.' },
  { label: 'Player Mana Orb', slug: 'player-mana-orb', group: 'Mana', copy: 'Player-claimed mana rendered in its cool white-blue faction treatment.' },
  { label: 'Rival Mana Orb', slug: 'rival-mana-orb', group: 'Mana', copy: 'Rival-claimed mana rendered in its hot red faction treatment.' },
  { label: 'Firebolt', slug: 'firebolt', group: 'Projectiles', copy: 'Fast player projectile with a white-hot core, layered flame body, and cooling wake.' },
  { label: 'Meteor', slug: 'meteor', group: 'Projectiles', copy: 'A heavy falling spell projectile with an incandescent leading face and rocky mass.' },
  { label: 'Creature Bolt', slug: 'creature-bolt', group: 'Projectiles', copy: 'Hostile creature projectile, isolated to inspect its silhouette and emissive balance.' },
  { label: 'Dragon Fire', slug: 'dragon-fire', group: 'Projectiles', copy: 'Dragon breath projectile with its distinct hot-magenta palette and compact flame shape.' },
].map((scene, id) => ({ ...scene, id }));

const el = (id) => document.getElementById(id);
const ui = {
  canvas: el('view'), scene: el('scene'), variant: el('variant'),
  variantPrev: el('variantPrev'), variantNext: el('variantNext'), resetView: el('resetView'),
  sceneCopy: el('sceneCopy'), sceneId: el('sceneId'), variantStat: el('variantStat'),
  instanceCount: el('instanceCount'), radius: el('radius'),
  status: el('status'), error: el('error'),
};

const gallery = {
  ready: false,
  state: 'loading',
  sceneId: null,
  scene: null,
  variant: null,
  variantCount: 0,
  instanceCount: 0,
  hash: null,
  error: null,
  setScene: null,
  whenReady: null,
  snapshot: () => ({
    ready: gallery.ready, state: gallery.state, sceneId: gallery.sceneId,
    scene: gallery.scene, variant: gallery.variant, variantCount: gallery.variantCount,
    instanceCount: gallery.instanceCount, hash: gallery.hash, error: gallery.error,
  }),
};
window.__modelGallery = gallery;

let sim;
let renderer;
let instView;
let partView;
let current;
let dirty = false;
let selectionGeneration = 0;
let readyWaiters = [];

const orbit = {
  azimuth: -0.72,
  elevation: 0.24,
  distance: 100,
  minDistance: 20,
  maxDistance: 500,
};

function setStatus(state, message, error = null) {
  gallery.state = state;
  gallery.ready = state === 'pass';
  gallery.error = error ? String(error.message || error) : null;
  ui.status.dataset.state = state;
  ui.status.textContent = message;
  document.documentElement.dataset.galleryState = state;
  if (state !== 'loading') {
    const waiters = readyWaiters;
    readyWaiters = [];
    for (const { resolve, reject } of waiters) {
      if (state === 'pass') resolve(gallery.snapshot());
      else reject(error || new Error(message));
    }
  }
}

function showFailure(error) {
  const err = error instanceof Error ? error : new Error(String(error));
  setStatus('fail', `FAIL — ${err.message}`, err);
  ui.error.textContent = err.stack || err.message;
  ui.error.classList.add('show');
  console.error(err);
}

function waitUntilReady() {
  if (gallery.state === 'pass') return Promise.resolve(gallery.snapshot());
  if (gallery.state === 'fail') return Promise.reject(new Error(gallery.error || 'Gallery failed.'));
  return new Promise((resolve, reject) => readyWaiters.push({ resolve, reject }));
}
gallery.whenReady = waitUntilReady;

function populateScenes() {
  const groups = new Map();
  for (const scene of CATALOG) {
    let group = groups.get(scene.group);
    if (!group) {
      group = document.createElement('optgroup');
      group.label = scene.group;
      groups.set(scene.group, group);
      ui.scene.appendChild(group);
    }
    const option = document.createElement('option');
    option.value = String(scene.id);
    option.textContent = `${String(scene.id).padStart(2, '0')} · ${scene.label}`;
    group.appendChild(option);
  }
}

function sceneFrom(value) {
  if (typeof value === 'number' && Number.isInteger(value)) return CATALOG[value] || null;
  const raw = String(value ?? '').trim().toLowerCase();
  if (/^\d+$/.test(raw)) return CATALOG[Number(raw)] || null;
  return CATALOG.find((scene) => scene.slug === raw) || null;
}

function clampVariant(value, count) {
  const parsed = Number.parseInt(String(value), 10);
  return Math.max(0, Math.min(count - 1, Number.isFinite(parsed) ? parsed : 0));
}

function refreshViews() {
  const memory = sim.memory.buffer;
  instView = new Float32Array(memory, sim.instPtr(), sim.instCapacity() * sim.instStride());
  partView = new Float32Array(memory, sim.partPtr(), sim.partCapacity() * sim.partStride());
}

function fnv1a(...blocks) {
  let hash = 0x811c9dc5;
  for (const bytes of blocks) {
    for (let i = 0; i < bytes.length; i++) {
      hash ^= bytes[i];
      hash = Math.imul(hash, 0x01000193);
    }
  }
  return (hash >>> 0).toString(16).padStart(8, '0');
}

function previewBytes(count) {
  return new Uint8Array(sim.memory.buffer, sim.instPtr(), count * sim.instStride() * 4);
}

function validatePreview(scene, variant) {
  const variantCount = sim.previewVariantCount(scene.id);
  if (variantCount !== 8) {
    throw new Error(`Scene ${scene.id} reported ${variantCount} variants; expected 8.`);
  }

  const firstCount = sim.previewScene(scene.id, variant);
  if (firstCount <= 0 || firstCount !== sim.instCount()) {
    throw new Error(`Preview ${scene.id}/${variant} returned an invalid instance count.`);
  }
  if (firstCount >= sim.instCapacity()) {
    throw new Error(`Preview filled its ${sim.instCapacity()}-instance buffer and may be truncated.`);
  }
  if (sim.partCount() !== 0) {
    throw new Error(`Preview emitted ${sim.partCount()} unexpected particles.`);
  }
  refreshViews();

  const usedFloats = firstCount * sim.instStride();
  for (let i = 0; i < usedFloats; i++) {
    if (!Number.isFinite(instView[i])) throw new Error(`Preview contains a non-finite instance value at ${i}.`);
  }
  const firstBytes = new Uint8Array(previewBytes(firstCount));
  const firstFocusBytes = new Uint8Array(
    new Uint8Array(sim.memory.buffer, sim.previewFocusPtr(), 4 * 4),
  );
  const firstFocus = Array.from(
    new Float32Array(firstFocusBytes.buffer, firstFocusBytes.byteOffset, 4),
  );
  if (!firstFocus.every(Number.isFinite) || firstFocus[3] <= 0) {
    throw new Error(`Preview ${scene.id}/${variant} returned an invalid focus sphere.`);
  }
  const firstHash = fnv1a(firstBytes, firstFocusBytes);

  // A gallery PASS is meaningful: building the same pair twice must reproduce
  // every used instance and focus byte, not merely the same count or picture.
  const secondCount = sim.previewScene(scene.id, variant);
  if (secondCount !== firstCount || secondCount !== sim.instCount()) {
    throw new Error(`Preview ${scene.id}/${variant} changed count between identical builds.`);
  }
  refreshViews();
  const secondBytes = previewBytes(secondCount);
  if (secondBytes.length !== firstBytes.length) {
    throw new Error(`Preview ${scene.id}/${variant} changed byte length between identical builds.`);
  }
  for (let i = 0; i < secondBytes.length; i++) {
    if (secondBytes[i] !== firstBytes[i]) {
      throw new Error(`Preview ${scene.id}/${variant} is nondeterministic at byte ${i}.`);
    }
  }

  const secondFocusBytes = new Uint8Array(sim.memory.buffer, sim.previewFocusPtr(), 4 * 4);
  for (let i = 0; i < secondFocusBytes.length; i++) {
    if (secondFocusBytes[i] !== firstFocusBytes[i]) {
      throw new Error(`Preview ${scene.id}/${variant} has a nondeterministic focus sphere.`);
    }
  }
  return { count: secondCount, focus: firstFocus, hash: firstHash, variantCount };
}

function resetOrbit() {
  if (!current) return;
  const radius = current.focus[3];
  orbit.azimuth = -0.72;
  orbit.elevation = 0.24;
  orbit.minDistance = Math.max(8, radius * 1.25);
  orbit.maxDistance = Math.max(orbit.minDistance + 1, Math.min(renderer.far * 0.82, radius * 8));
  orbit.distance = Math.max(orbit.minDistance, Math.min(orbit.maxDistance, radius * 2.75));
  dirty = true;
}

function syncUrl() {
  try {
    const url = new URL(location.href);
    url.searchParams.set('scene', current.scene.slug);
    url.searchParams.set('variant', String(current.variant));
    history.replaceState(null, '', url);
  } catch (e) {
    // file:// and embedded hosts can disallow history writes; selection still works.
  }
}

function updateUi() {
  const { scene, variant, variantCount, count, focus, hash } = current;
  ui.scene.value = String(scene.id);
  ui.variant.min = '0';
  ui.variant.max = String(variantCount - 1);
  ui.variant.value = String(variant);
  ui.sceneCopy.textContent = scene.copy;
  ui.sceneId.textContent = String(scene.id).padStart(2, '0');
  ui.variantStat.textContent = `${variant + 1} / ${variantCount}`;
  ui.instanceCount.textContent = count.toLocaleString();
  ui.radius.textContent = focus[3].toFixed(1);

  gallery.sceneId = scene.id;
  gallery.scene = scene.slug;
  gallery.variant = variant;
  gallery.variantCount = variantCount;
  gallery.instanceCount = count;
  gallery.hash = hash;
}

async function selectPreview(sceneLike, variantLike, { reset = true, updateUrl = true } = {}) {
  const scene = sceneFrom(sceneLike);
  if (!scene) throw new Error(`Unknown preview scene: ${sceneLike}`);
  const variantCount = sim.previewVariantCount(scene.id);
  if (variantCount <= 0) throw new Error(`Scene ${scene.id} has no preview variants.`);
  const variant = clampVariant(variantLike, variantCount);

  selectionGeneration++;
  setStatus('loading', `LOADING — ${scene.label} · variant ${variant + 1}/${variantCount}`);
  ui.error.classList.remove('show');
  const preview = validatePreview(scene, variant);
  current = { scene, variant, ...preview };
  updateUi();
  if (reset) resetOrbit();
  else {
    const oldRatio = orbit.distance / Math.max(orbit.minDistance, 0.001);
    orbit.minDistance = Math.max(8, preview.focus[3] * 1.25);
    orbit.maxDistance = Math.max(orbit.minDistance + 1, Math.min(renderer.far * 0.82, preview.focus[3] * 8));
    orbit.distance = Math.max(orbit.minDistance, Math.min(orbit.maxDistance, orbit.minDistance * oldRatio));
    dirty = true;
  }
  if (updateUrl) syncUrl();
  return waitUntilReady();
}
gallery.setScene = (scene, variant = gallery.variant ?? 0) => selectPreview(scene, variant);

function cameraForOrbit() {
  const [cx, cy, cz] = current.focus;
  const ce = Math.cos(orbit.elevation);
  return {
    ex: cx + Math.sin(orbit.azimuth) * ce * orbit.distance,
    ey: cy + Math.sin(orbit.elevation) * orbit.distance,
    ez: cz + Math.cos(orbit.azimuth) * ce * orbit.distance,
    cx, cy, cz, ux: 0, uy: 1, uz: 0,
  };
}

function renderFrame() {
  if (!dirty || !current || gallery.state === 'fail') return;
  dirty = false;
  renderer.resize();
  const generation = selectionGeneration;
  renderer.frame(cameraForOrbit(), partView, 0, instView, current.count, {
    fov: 0.92,
    time: 17.0,
    sun: [0.44, 0.78, 0.36],
    sunI: 1.05,
    fog: [0.70, 0.82, 0.95],
    fogD: 0.00062,
    skipLo: -1,
    skipHi: -1,
    exposure: 1.10,
    bloomStrength: 0.38,
    waterAlpha: 0.82,
    hurt: 0,
    castles: [],
  });

  renderer.device.queue.onSubmittedWorkDone().then(() => {
    if (generation !== selectionGeneration || gallery.state !== 'loading') return;
    setStatus(
      'pass',
      `PASS — ${current.scene.label} · variant ${current.variant + 1}/${current.variantCount} · ${current.count} instances · ${current.hash}`,
    );
  }).catch(showFailure);
}

function animationLoop() {
  if (renderer?.lost && gallery.state !== 'fail') {
    showFailure(new Error(`GPU device lost: ${renderer.lost}`));
  }
  try { renderFrame(); } catch (error) { showFailure(error); }
  requestAnimationFrame(animationLoop);
}

function bindControls() {
  ui.scene.addEventListener('change', () => {
    selectPreview(Number(ui.scene.value), current?.variant ?? 0).catch(showFailure);
  });
  ui.variant.addEventListener('change', () => {
    selectPreview(current.scene.id, ui.variant.value, { reset: false }).catch(showFailure);
  });
  ui.variant.addEventListener('keydown', (event) => {
    if (event.key === 'Enter') {
      event.preventDefault();
      ui.variant.blur();
    }
  });
  const stepVariant = (delta) => {
    const count = current.variantCount;
    const variant = (current.variant + delta + count) % count;
    selectPreview(current.scene.id, variant, { reset: false }).catch(showFailure);
  };
  ui.variantPrev.addEventListener('click', () => stepVariant(-1));
  ui.variantNext.addEventListener('click', () => stepVariant(1));
  ui.resetView.addEventListener('click', resetOrbit);
  ui.canvas.addEventListener('dblclick', resetOrbit);

  const pointers = new Map();
  let dragStart = null;
  let pinchStart = null;
  const pointerDistance = () => {
    const p = [...pointers.values()];
    return p.length < 2 ? 0 : Math.hypot(p[0].x - p[1].x, p[0].y - p[1].y);
  };
  const restartGesture = () => {
    if (pointers.size === 1) {
      const [p] = pointers.values();
      dragStart = { x: p.x, y: p.y, azimuth: orbit.azimuth, elevation: orbit.elevation };
      pinchStart = null;
    } else if (pointers.size >= 2) {
      pinchStart = { span: pointerDistance(), distance: orbit.distance };
      dragStart = null;
    }
  };
  ui.canvas.addEventListener('pointerdown', (event) => {
    ui.canvas.setPointerCapture(event.pointerId);
    pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
    ui.canvas.classList.add('dragging');
    restartGesture();
  });
  ui.canvas.addEventListener('pointermove', (event) => {
    if (!pointers.has(event.pointerId)) return;
    pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
    if (pointers.size >= 2 && pinchStart) {
      const span = Math.max(pointerDistance(), 1);
      orbit.distance = Math.max(
        orbit.minDistance,
        Math.min(orbit.maxDistance, pinchStart.distance * pinchStart.span / span),
      );
      dirty = true;
    } else if (pointers.size === 1 && dragStart) {
      const [p] = pointers.values();
      orbit.azimuth = dragStart.azimuth - (p.x - dragStart.x) * 0.008;
      orbit.elevation = Math.max(-0.18, Math.min(1.28, dragStart.elevation + (p.y - dragStart.y) * 0.006));
      dirty = true;
    }
  });
  const releasePointer = (event) => {
    pointers.delete(event.pointerId);
    if (ui.canvas.hasPointerCapture(event.pointerId)) ui.canvas.releasePointerCapture(event.pointerId);
    ui.canvas.classList.toggle('dragging', pointers.size > 0);
    restartGesture();
  };
  ui.canvas.addEventListener('pointerup', releasePointer);
  ui.canvas.addEventListener('pointercancel', releasePointer);
  ui.canvas.addEventListener('wheel', (event) => {
    event.preventDefault();
    orbit.distance = Math.max(
      orbit.minDistance,
      Math.min(orbit.maxDistance, orbit.distance * Math.exp(event.deltaY * 0.0012)),
    );
    dirty = true;
  }, { passive: false });

  const resize = () => { renderer.resize(); dirty = true; };
  addEventListener('resize', resize);
  if (window.ResizeObserver) new ResizeObserver(resize).observe(ui.canvas);
}

async function boot() {
  populateScenes();
  const response = await fetch('./sim.wasm');
  if (!response.ok) throw new Error(`Could not load sim.wasm (HTTP ${response.status}).`);
  const { instance } = await WebAssembly.instantiate(await response.arrayBuffer(), {});
  sim = instance.exports;

  const required = [
    'previewSceneCount', 'previewVariantCount', 'previewScene', 'previewFocusPtr',
    'instPtr', 'instCount', 'instCapacity', 'instStride', 'partPtr', 'partCount',
    'partCapacity', 'partStride',
  ];
  const missing = required.filter((name) => typeof sim[name] !== 'function');
  if (missing.length) throw new Error(`sim.wasm is missing preview exports: ${missing.join(', ')}`);
  if (sim.previewSceneCount() !== CATALOG.length) {
    throw new Error(`Preview catalogue has ${CATALOG.length} entries but the core reports ${sim.previewSceneCount()}.`);
  }

  renderer = new Renderer(ui.canvas, sim);
  await renderer.init(sim.terrainWidth(), sim.cellSize(), 0);
  refreshViews();
  // Linear memory is zero-initialized without init(), yielding a neutral flat
  // background while previewScene owns only the model instance buffer.
  const heights = new Float32Array(sim.memory.buffer, sim.heightPtr(), sim.terrainWidth() ** 2);
  renderer.uploadHeights(heights, 0, sim.terrainWidth() - 1);

  renderer.device.addEventListener?.('uncapturederror', (event) => {
    event.preventDefault?.();
    showFailure(event.error || new Error('Uncaptured WebGPU validation error.'));
  });
  bindControls();

  const params = new URLSearchParams(location.search);
  const initialScene = sceneFrom(params.get('scene')) || CATALOG[0];
  const initialVariant = params.get('variant') ?? 0;
  selectPreview(initialScene.id, initialVariant, { updateUrl: true }).catch(showFailure);
  requestAnimationFrame(animationLoop);
}

boot().catch(showFailure);
