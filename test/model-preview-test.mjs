import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import fs from 'node:fs';

const SCENES = [
  'sand-worm', 'wasp', 'troll', 'griffin', 'nest', 'wraith', 'balloon', 'dragon',
  'villager', 'soldier', 'castle-tier-1', 'castle-tier-2', 'castle-tier-3',
  'castle-tier-4', 'castle-tier-5', 'castle-tier-6', 'castle-ruin', 'palm',
  'boulder', 'burnt-stump', 'player-carpet', 'rival-carpet', 'unclaimed-mana-orb',
  'player-mana-orb', 'rival-mana-orb', 'firebolt', 'meteor', 'creature-bolt',
  'dragon-fire', 'fireball-impact',
];
const VARIANTS = 8;
const FIREBALL_IMPACT_SCENE = 29;
const SIGNATURE_PATH = new URL('./model-preview-signatures.json', import.meta.url);
const printSignatures = process.argv.includes('--print-signatures');

const wasm = fs.readFileSync('site/sim.wasm');
const sim = new WebAssembly.Instance(new WebAssembly.Module(wasm), {}).exports;
assert.ok(fs.existsSync('site/models.html'), 'model gallery HTML is missing from the deployable site');
const galleryHtml = fs.readFileSync('site/models.html', 'utf8');
const gallerySource = fs.readFileSync('site/models.js', 'utf8');
const gallerySlugs = [...gallerySource.matchAll(/\bslug:\s*'([^']+)'/g)].map((match) => match[1]);
assert.deepEqual(gallerySlugs, SCENES, 'model gallery catalog/order differs from the preview ABI');
assert.match(
  gallerySource,
  /\blabel:\s*'Fireball Impact'\s*,\s*slug:\s*'fireball-impact'/,
  'scene 29 is missing its Fireball Impact gallery label',
);
assert.match(galleryHtml, /\bid="particleCount"/, 'model gallery does not display particle count');
assert.equal(sim.previewSceneCount(), SCENES.length, 'preview scene catalog changed');
assert.equal(sim.instStride(), 14, 'preview validator expects the documented 14-float instance ABI');
assert.equal(sim.partStride(), 8, 'preview validator expects the documented 8-float particle ABI');
assert.equal(SCENES[FIREBALL_IMPACT_SCENE], 'fireball-impact', 'fireball impact scene ABI moved');
sim.buildMeshes();

const capacity = sim.instCapacity();
const stride = sim.instStride();
const particleCapacity = sim.partCapacity();
const particleStride = sim.partStride();
const shapeCount = sim.shapeCount();
const worldSize = sim.worldSize();
const pairSignatures = [];
const variantContentHashes = SCENES.map(() => new Set());
const impactParticleHashes = new Set();

function copyBytes(pointer, byteLength) {
  assert.ok(pointer >= 0 && pointer + byteLength <= sim.memory.buffer.byteLength,
    `wasm view ${pointer}+${byteLength} exceeds linear memory`);
  return Buffer.from(new Uint8Array(sim.memory.buffer, pointer, byteLength));
}

function snapshot(scene, variant) {
  const returned = sim.previewScene(scene, variant);
  const count = sim.instCount();
  assert.equal(returned, count, `${SCENES[scene]} variant ${variant}: return/count mismatch`);
  assert.ok(count > 0, `${SCENES[scene]} variant ${variant}: empty preview`);
  assert.ok(count < capacity,
    `${SCENES[scene]} variant ${variant}: filled the instance buffer and may be truncated`);
  const particleCount = sim.partCount();
  if (scene === FIREBALL_IMPACT_SCENE) {
    assert.ok(particleCount > 0, `${SCENES[scene]} variant ${variant}: missing impact particles`);
  } else {
    assert.equal(particleCount, 0, `${SCENES[scene]} variant ${variant}: unexpected particles`);
  }
  assert.ok(particleCount < particleCapacity,
    `${SCENES[scene]} variant ${variant}: filled the particle buffer and may be truncated`);

  const instanceBytes = copyBytes(sim.instPtr(), count * stride * 4);
  const particleBytes = copyBytes(sim.partPtr(), particleCount * particleStride * 4);
  const focusBytes = copyBytes(sim.previewFocusPtr(), 4 * 4);
  return { count, particleCount, instanceBytes, particleBytes, focusBytes };
}

function validateSnapshot(scene, variant, shot) {
  const label = `${SCENES[scene]} variant ${variant}`;
  const values = new Float32Array(shot.instanceBytes.buffer.slice(
    shot.instanceBytes.byteOffset,
    shot.instanceBytes.byteOffset + shot.instanceBytes.byteLength,
  ));
  const focus = new Float32Array(shot.focusBytes.buffer.slice(
    shot.focusBytes.byteOffset,
    shot.focusBytes.byteOffset + shot.focusBytes.byteLength,
  ));
  const particles = new Float32Array(shot.particleBytes.buffer.slice(
    shot.particleBytes.byteOffset,
    shot.particleBytes.byteOffset + shot.particleBytes.byteLength,
  ));

  for (const value of values) assert.ok(Number.isFinite(value), `${label}: non-finite instance value`);
  for (const value of particles) assert.ok(Number.isFinite(value), `${label}: non-finite particle value`);
  for (const value of focus) assert.ok(Number.isFinite(value), `${label}: non-finite focus value`);
  assert.ok(focus[3] > 0 && focus[3] <= worldSize, `${label}: invalid focus radius ${focus[3]}`);
  for (let axis = 0; axis < 3; axis++) {
    assert.ok(Math.abs(focus[axis]) <= worldSize * 2,
      `${label}: focus axis ${axis} is out of bounds (${focus[axis]})`);
  }

  const lower = [Infinity, Infinity, Infinity];
  const upper = [-Infinity, -Infinity, -Infinity];
  for (let i = 0; i < shot.count; i++) {
    const base = i * stride;
    const halfDiagonal = 0.5 * Math.hypot(
      values[base + 3],
      values[base + 4],
      values[base + 5],
    );
    for (let axis = 0; axis < 3; axis++) {
      const position = values[base + axis];
      const size = values[base + 3 + axis];
      assert.ok(Math.abs(position) <= worldSize * 2,
        `${label}: instance ${i} position axis ${axis} is out of bounds (${position})`);
      assert.ok(size > 0 && size <= worldSize,
        `${label}: instance ${i} size axis ${axis} is invalid (${size})`);
      lower[axis] = Math.min(lower[axis], position - halfDiagonal);
      upper[axis] = Math.max(upper[axis], position + halfDiagonal);
    }
    for (let channel = 0; channel < 3; channel++) {
      const colour = values[base + 6 + channel];
      assert.ok(colour >= 0 && colour <= 2,
        `${label}: instance ${i} colour channel ${channel} is invalid (${colour})`);
    }
    for (let axis = 0; axis < 3; axis++) {
      const rotation = values[base + 9 + axis];
      assert.ok(Math.abs(rotation) <= Math.PI * 8,
        `${label}: instance ${i} rotation axis ${axis} is invalid (${rotation})`);
    }
    const glow = values[base + 12];
    const shape = values[base + 13];
    assert.ok(glow >= 0 && glow <= 2, `${label}: instance ${i} glow is invalid (${glow})`);
    assert.ok(Number.isInteger(shape) && shape >= 0 && shape < shapeCount,
      `${label}: instance ${i} has illegal shape ${shape}`);
  }
  for (let i = 0; i < shot.particleCount; i++) {
    const base = i * particleStride;
    for (let axis = 0; axis < 3; axis++) {
      const position = particles[base + axis];
      assert.ok(Math.abs(position) <= worldSize * 2,
        `${label}: particle ${i} position axis ${axis} is out of bounds (${position})`);
    }
    const size = particles[base + 3];
    assert.ok(size > 0 && size <= worldSize,
      `${label}: particle ${i} size is invalid (${size})`);
    // Particle vertices form a camera-facing square from -size..size on both
    // axes, so its rotation-independent bounding radius is size * sqrt(2).
    const extent = size * Math.SQRT2;
    for (let axis = 0; axis < 3; axis++) {
      lower[axis] = Math.min(lower[axis], particles[base + axis] - extent);
      upper[axis] = Math.max(upper[axis], particles[base + axis] + extent);
    }
    for (let channel = 0; channel < 3; channel++) {
      const colour = particles[base + 4 + channel];
      assert.ok(colour >= 0 && colour <= 2,
        `${label}: particle ${i} colour channel ${channel} is invalid (${colour})`);
    }
    const alpha = particles[base + 7];
    assert.ok(alpha > 0 && alpha <= 1,
      `${label}: particle ${i} alpha is invalid (${alpha})`);
  }
  if (shot.particleCount > 0) {
    const positions = new Set();
    const sizes = new Set();
    const colours = new Set();
    for (let i = 0; i < shot.particleCount; i++) {
      const base = i * particleStride;
      positions.add(`${particles[base]},${particles[base + 1]},${particles[base + 2]}`);
      sizes.add(String(particles[base + 3]));
      colours.add(`${particles[base + 4]},${particles[base + 5]},${particles[base + 6]}`);
    }
    assert.ok(positions.size >= 4, `${label}: impact particles lack spatial spread`);
    assert.ok(sizes.size >= 2, `${label}: impact particles lack size layering`);
    assert.ok(colours.size >= 2, `${label}: impact particles lack colour layering`);
  }

  const expectedCentre = lower.map((value, axis) => (value + upper[axis]) * 0.5);
  for (let axis = 0; axis < 3; axis++) {
    assert.ok(Math.abs(focus[axis] - expectedCentre[axis]) <= 0.01,
      `${label}: focus centre axis ${axis} does not match emitted bounds`);
  }
  let requiredRadius = 0;
  for (let i = 0; i < shot.count; i++) {
    const base = i * stride;
    const halfDiagonal = 0.5 * Math.hypot(
      values[base + 3],
      values[base + 4],
      values[base + 5],
    );
    requiredRadius = Math.max(requiredRadius,
      Math.hypot(
        values[base] - focus[0],
        values[base + 1] - focus[1],
        values[base + 2] - focus[2],
      ) + halfDiagonal);
  }
  for (let i = 0; i < shot.particleCount; i++) {
    const base = i * particleStride;
    requiredRadius = Math.max(requiredRadius,
      Math.hypot(
        particles[base] - focus[0],
        particles[base + 1] - focus[1],
        particles[base + 2] - focus[2],
      ) + particles[base + 3] * Math.SQRT2);
  }
  const expectedRadius = requiredRadius * 1.05;
  const radiusTolerance = Math.max(0.01, expectedRadius * 0.0001);
  assert.ok(Math.abs(focus[3] - expectedRadius) <= radiusTolerance,
    `${label}: focus radius ${focus[3]} does not conservatively frame emitted bounds (${expectedRadius})`);
}

function signature(scene, variant, shot) {
  const header = Buffer.alloc(12);
  header.writeUInt32LE(scene, 0);
  header.writeUInt32LE(variant, 4);
  header.writeUInt32LE(shot.count, 8);
  const hash = createHash('sha256')
    .update(header)
    .update(shot.instanceBytes);
  if (shot.particleCount > 0) hash.update(shot.particleBytes);
  return hash.update(shot.focusBytes).digest('hex');
}

for (let scene = 0; scene < SCENES.length; scene++) {
  const expectedVariants = scene === 25 ? 24 : VARIANTS;
  assert.equal(sim.previewVariantCount(scene), expectedVariants,
    `${SCENES[scene]} must expose ${expectedVariants} variants`);
  for (let variant = 0; variant < VARIANTS; variant++) {
    const first = snapshot(scene, variant);
    validateSnapshot(scene, variant, first);
    const second = snapshot(scene, variant);
    assert.equal(second.count, first.count,
      `${SCENES[scene]} variant ${variant}: nondeterministic instance count`);
    assert.equal(second.particleCount, first.particleCount,
      `${SCENES[scene]} variant ${variant}: nondeterministic particle count`);
    assert.ok(second.instanceBytes.equals(first.instanceBytes),
      `${SCENES[scene]} variant ${variant}: nondeterministic instance bytes`);
    assert.ok(second.particleBytes.equals(first.particleBytes),
      `${SCENES[scene]} variant ${variant}: nondeterministic particle bytes`);
    assert.ok(second.focusBytes.equals(first.focusBytes),
      `${SCENES[scene]} variant ${variant}: nondeterministic focus`);
    const contentDigest = createHash('sha256').update(first.instanceBytes);
    if (first.particleCount > 0) contentDigest.update(first.particleBytes);
    const contentHash = contentDigest.update(first.focusBytes).digest('hex');
    assert.ok(!variantContentHashes[scene].has(contentHash),
      `${SCENES[scene]} variant ${variant} duplicates an earlier variant`);
    variantContentHashes[scene].add(contentHash);
    if (first.particleCount > 0) {
      const particleHash = createHash('sha256').update(first.particleBytes).digest('hex');
      assert.ok(!impactParticleHashes.has(particleHash),
        `${SCENES[scene]} variant ${variant} duplicates an earlier particle effect`);
      impactParticleHashes.add(particleHash);
    }
    pairSignatures.push([first.count, signature(scene, variant, first)]);
  }
}
for (let variant = VARIANTS; variant < 24; variant++) {
  const first = snapshot(25, variant);
  validateSnapshot(25, variant, first);
  const second = snapshot(25, variant);
  assert.ok(second.instanceBytes.equals(first.instanceBytes),
    `firebolt variant ${variant}: nondeterministic instance bytes`);
  assert.ok(second.focusBytes.equals(first.focusBytes),
    `firebolt variant ${variant}: nondeterministic focus`);
  const contentHash = createHash('sha256')
    .update(first.instanceBytes)
    .update(first.focusBytes)
    .digest('hex');
  assert.ok(!variantContentHashes[25].has(contentHash),
    `firebolt variant ${variant} duplicates an earlier animation phase`);
  variantContentHashes[25].add(contentHash);
}
assert.equal(variantContentHashes[25].size, 24,
  'firebolt must expose 24 distinct deterministic animation phases');
assert.equal(impactParticleHashes.size, VARIANTS,
  'fireball-impact must expose one distinct particle effect per variant');

assert.equal(sim.previewVariantCount(-1), 0, 'negative scene must have no variants');
assert.equal(sim.previewVariantCount(SCENES.length), 0, 'past-end scene must have no variants');
for (const [scene, variant] of [[-1, 0], [SCENES.length, 0], [0, -1], [0, VARIANTS]]) {
  assert.equal(sim.previewScene(scene, variant), 0, `invalid preview ${scene}:${variant} must fail`);
  assert.equal(sim.instCount(), 0, `invalid preview ${scene}:${variant} must clear instances`);
  assert.equal(sim.partCount(), 0, `invalid preview ${scene}:${variant} must clear particles`);
  assert.deepEqual(
    Array.from(new Float32Array(sim.memory.buffer, sim.previewFocusPtr(), 4)),
    [0, 0, 0, 0],
    `invalid preview ${scene}:${variant} must clear focus`,
  );
}
assert.equal(sim.previewScene(25, 24), 0, 'past-end firebolt animation phase must fail');

const observed = SCENES.map((_, scene) => {
  const pairs = pairSignatures.slice(scene * VARIANTS, (scene + 1) * VARIANTS);
  const counts = pairs.map(([count]) => count);
  const digest = createHash('sha256').update(JSON.stringify(pairs)).digest('hex');
  return [counts, digest];
});

if (printSignatures) {
  console.log(JSON.stringify({ version: 1, scenes: SCENES, variants: VARIANTS, entries: observed }, null, 2));
} else {
  const expected = JSON.parse(fs.readFileSync(SIGNATURE_PATH, 'utf8'));
  assert.equal(expected.version, 1, 'unsupported model signature version');
  assert.deepEqual(expected.scenes, SCENES, 'model signature catalog changed');
  assert.equal(expected.variants, VARIANTS, 'model signature variant count changed');
  assert.equal(expected.entries.length, SCENES.length, 'model signature scene count changed');
  for (let scene = 0; scene < SCENES.length; scene++) {
    assert.deepEqual(observed[scene], expected.entries[scene],
      `${SCENES[scene]} preview signature changed; inspect the gallery and intentionally refresh the baseline`);
  }
  console.log(`model previews: ${SCENES.length} scenes x ${VARIANTS} variants, all deterministic and valid`);
}
