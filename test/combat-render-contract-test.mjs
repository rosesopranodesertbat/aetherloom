import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const TEST_DIR = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(TEST_DIR, '..');
const SRC = path.join(ROOT, 'src');

function rustFiles(directory) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const target = path.join(directory, entry.name);
    if (entry.isDirectory()) return rustFiles(target);
    return entry.isFile() && entry.name.endsWith('.rs') ? [target] : [];
  });
}

function read(relativePath) {
  return fs.readFileSync(path.join(ROOT, relativePath), 'utf8');
}

/**
 * Return the brace-delimited item beginning at `openBrace`.
 *
 * Rust comments and strings can contain braces, so a plain brace count is not
 * quite enough. This small scanner only needs to understand those lexical
 * forms; it intentionally does not try to parse Rust.
 */
function braceBlock(source, openBrace) {
  assert.equal(source[openBrace], '{', 'braceBlock must start on an opening brace');
  let depth = 0;
  let state = 'code';

  for (let index = openBrace; index < source.length; index++) {
    const current = source[index];
    const next = source[index + 1];

    if (state === 'line-comment') {
      if (current === '\n') state = 'code';
      continue;
    }
    if (state === 'block-comment') {
      if (current === '*' && next === '/') {
        state = 'code';
        index++;
      }
      continue;
    }
    if (state === 'string') {
      if (current === '\\') {
        index++;
      } else if (current === '"') {
        state = 'code';
      }
      continue;
    }
    if (state === 'char') {
      if (current === '\\') {
        index++;
      } else if (current === "'") {
        state = 'code';
      }
      continue;
    }

    if (current === '/' && next === '/') {
      state = 'line-comment';
      index++;
    } else if (current === '/' && next === '*') {
      state = 'block-comment';
      index++;
    } else if (current === '"') {
      state = 'string';
    } else if (current === "'") {
      state = 'char';
    } else if (current === '{') {
      depth++;
    } else if (current === '}') {
      depth--;
      if (depth === 0) return source.slice(openBrace, index + 1);
    }
  }

  assert.fail('unterminated Rust brace-delimited item');
}

function namedRustItem(source, declaration) {
  const declarationIndex = source.search(declaration);
  assert.notEqual(declarationIndex, -1, `missing Rust item matching ${declaration}`);
  const openBrace = source.indexOf('{', declarationIndex);
  assert.notEqual(openBrace, -1, `missing body for Rust item matching ${declaration}`);
  return braceBlock(source, openBrace);
}

function projectileArm(functionBody, kind) {
  const arm = new RegExp(
    String.raw`(?:ProjectileKind::[A-Za-z0-9_]+\s*(?:\|\s*)?)*ProjectileKind::${kind}` +
      String.raw`(?:\s*\|\s*ProjectileKind::[A-Za-z0-9_]+)*\s*=>`,
  );
  const match = arm.exec(functionBody);
  assert.ok(match, `detonate_projectile is missing an explicit ${kind} arm`);

  const expressionStart = match.index + match[0].length;
  const firstToken = functionBody.slice(expressionStart).search(/\S/);
  assert.notEqual(firstToken, -1, `${kind} arm has no expression`);
  const start = expressionStart + firstToken;
  if (functionBody[start] === '{') return braceBlock(functionBody, start);

  const end = functionBody.indexOf(',', start);
  assert.notEqual(end, -1, `${kind} arm expression is not comma-terminated`);
  return functionBody.slice(start, end);
}

const rustSources = rustFiles(SRC).map((filename) => ({
  filename,
  source: fs.readFileSync(filename, 'utf8'),
}));
const allRust = rustSources.map(({ source }) => source).join('\n');

// Enemy damage is represented by impact geometry/particles. Keeping a timer in
// every creature slot and threading it into every body renderer made unrelated
// models flash red and left a surprisingly broad state/render coupling.
assert.doesNotMatch(
  allRust,
  /\bhurt_flash\b/,
  'per-creature hurt_flash state was reintroduced',
);

const renderSource = rustSources
  .filter(({ filename }) => filename.includes(`${path.sep}render${path.sep}`))
  .map(({ source }) => source)
  .join('\n');
assert.doesNotMatch(
  renderSource,
  /\bbody\.hurt\b/,
  'creature renderers still depend on the removed hit-flash path',
);
const renderModule = read('src/render/mod.rs');
const creatureBody = namedRustItem(renderModule, /\bstruct\s+CreatureBody\b/);
assert.doesNotMatch(
  creatureBody,
  /\bhurt\s*:/,
  'CreatureBody still carries hit-flash render state',
);

// Both fiery projectile kinds should select one purpose-built impact effect.
// Discover the helper from the match arms instead of freezing its exact name.
const creaturesSource = read('src/creatures.rs');
const detonate = namedRustItem(creaturesSource, /\bfn\s+detonate_projectile\s*\(/);
const fireboltArm = projectileArm(detonate, 'Firebolt');
const dragonFireArm = projectileArm(detonate, 'DragonFire');
const dedicatedImpactCall = /\bself\.(spawn_[a-z0-9_]*fire[a-z0-9_]*impact[a-z0-9_]*)\s*\(/g;
const fireboltHelpers = [...fireboltArm.matchAll(dedicatedImpactCall)].map((match) => match[1]);
const dragonHelpers = [...dragonFireArm.matchAll(dedicatedImpactCall)].map((match) => match[1]);
assert.ok(fireboltHelpers.length > 0, 'Firebolt does not dispatch to a dedicated fire impact effect');
assert.ok(dragonHelpers.length > 0, 'DragonFire does not dispatch to a dedicated fire impact effect');
assert.equal(
  fireboltHelpers[0],
  dragonHelpers[0],
  'Firebolt and DragonFire should share the dedicated fire impact effect',
);
assert.notEqual(
  fireboltHelpers[0],
  'spawn_impact',
  'fiery projectiles must not fall back to the generic creature-hit effect',
);

const impactDefinition = new RegExp(String.raw`\bfn\s+${fireboltHelpers[0]}\s*\(`);
const impactOwners = rustSources.filter(({ source }) => impactDefinition.test(source));
assert.equal(impactOwners.length, 1, `${fireboltHelpers[0]} must have exactly one definition`);
const fireImpact = namedRustItem(impactOwners[0].source, impactDefinition);
assert.match(
  fireImpact,
  /\b(?:spawn_particle|spawn_burst)\s*\(/,
  `${fireboltHelpers[0]} does not emit an impact effect`,
);

// Fiery projectiles share the layered in-flight renderer, while the creature
// bolt keeps its separate shard model.
const effectsSource = read('src/render/effects.rs');
const drawProjectiles = namedRustItem(effectsSource, /\bfn\s+draw_projectiles\s*\(/);
for (const kind of ['Firebolt', 'Meteor', 'DragonFire']) {
  assert.match(
    projectileArm(drawProjectiles, kind),
    /\bself\.draw_fireball\s*\(\s*at\s*,\s*velocity\s*,\s*size\s*,\s*colour\s*,\s*index\s*\)/,
    `${kind} no longer uses the shared in-flight fireball renderer with its style inputs`,
  );
}
assert.match(
  projectileArm(drawProjectiles, 'CreatureBolt'),
  /\bself\.draw_bolt\s*\(/,
  'CreatureBolt should retain its distinct shard renderer',
);

// Check the fireball's stable visual contract rather than its complete source
// or preview bytes: a directional, deterministic stack of body/wake/tongue
// layers, with every repeated layer capped by a small compile-time constant.
const fireball = namedRustItem(effectsSource, /\bfn\s+draw_fireball\s*\(/);
assert.match(
  fireball,
  /\blength3\s*\(\s*velocity\[0\]\s*,\s*velocity\[1\]\s*,\s*velocity\[2\]\s*\)/,
  'fireball flight axis is no longer derived from velocity',
);
assert.ok(
  (fireball.match(/velocity\[[0-2]\]\s*\/\s*speed/g) ?? []).length >= 3,
  'fireball flight axis is not normalized',
);
assert.match(
  fireball,
  /\[\s*0\.0\s*,\s*1\.0\s*,\s*0\.0\s*\]/,
  'a stalled fireball needs a stable +Y fallback axis',
);
assert.match(fireball, /\bself\.session\.elapsed\b/, 'fireball animation lost its shared clock');
assert.match(fireball, /\bindex\s+as\s+f32\b/, 'fireballs no longer have per-projectile phase');
assert.doesNotMatch(fireball, /\b(?:self\.)?rng\b/, 'fireball geometry must be deterministic');
assert.match(fireball, /\bShape::Sphere\b/, 'fireball lost its layered body/core geometry');
assert.match(fireball, /\bShape::Cone\b/, 'fireball lost its directional flame tongues');
assert.ok(
  (fireball.match(/\bself\.push_(?:instance|oriented)\s*\(/g) ?? []).length >= 4,
  'fireball no longer has distinct wake, body, tongue, and core layers',
);
assert.match(fireball, /\bsize\b/, 'fireball geometry no longer responds to projectile size');
assert.match(fireball, /\bwarm\[[0-2]\]/, 'fireball layers no longer use projectile colour');

const fixedCounts = new Map(
  [...fireball.matchAll(/\bconst\s+([A-Z][A-Z0-9_]*)\s*:\s*i32\s*=\s*(\d+)\s*;/g)]
    .map((match) => [match[1], Number(match[2])]),
);
const repeatedLayers = [
  ...fireball.matchAll(/\bfor\s+[a-z_][a-z0-9_]*\s+in\s+0\.\.([A-Z][A-Z0-9_]*)\b/g),
].map((match) => match[1]);
assert.ok(repeatedLayers.length >= 2, 'fireball needs multiple repeated visual layers');
for (const countName of repeatedLayers) {
  assert.ok(fixedCounts.has(countName), `${countName} must be a compile-time fireball layer bound`);
  assert.ok(
    fixedCounts.get(countName) > 0 && fixedCounts.get(countName) <= 8,
    `${countName} exceeds the fixed per-layer instance budget`,
  );
}
assert.doesNotMatch(
  fireball,
  /\bwhile[^\n{]*\{|\bloop[ \t]*\{/,
  'fireball rendering must not contain an unbounded instance loop',
);

console.log('combat/render contracts: hit flash removed, fire impacts dedicated, fireballs layered');
