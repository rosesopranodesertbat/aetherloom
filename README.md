# Aetherloom

A flying-carpet wizard duel that runs in a browser tab. Original game, built as an homage to Bullfrog's *Magic Carpet* (1994) and *Magic Carpet 2* (1995) — the terrain you can blow holes in, the mana your balloons have to physically carry home, and a rival wizard racing you for the same pool.

No original assets are used. Every texture, shape, sound and level is generated procedurally at runtime.

## Play it

**Fastest:** open `standalone.html` directly. One file, roughly 520 KB,
WASM embedded as base64, no server needed.

**As a site:**
```bash
node serve.mjs        # → http://localhost:8080
```
(ES modules and `fetch` need a real origin, so `site/index.html` won't work over `file://` — that's what `standalone.html` is for.)

The same server exposes a deterministic [model gallery](http://localhost:8080/models.html)
covering every creature, keep tier, scenery model, carpet, orb and projectile in
eight stable variants. A specific view can be linked as
`models.html?scene=dragon&variant=3`.

Requires **WebGPU**: Chrome/Edge 113+, Firefox 141+, Safari 18.2+.

## Controls

| | |
|---|---|
| `W` / `A` `D` | thrust, slide left and right |
| Mouse | steer (click the sky to capture the pointer, `Esc` releases) |
| `Space` / `Shift` | climb, dive |
| `X` | hold to slow down |
| Left click | cast the held spell |
| `1`–`9` `0` `-` `=` `[` | pick a spell · `Q`/`E` or wheel to cycle |
| `C` | first-person ↔ chase view (first person hides your own carpet) |
| `P` pause · `M` mute · `B` bloom · `Shift+R` restart realm |

## How it plays

Mana is the whole game, and you never carry it home yourself — your balloons do.

1. **Break things.** Wildlife and their nests drop **gold** mana orbs when they die. Nests are worth the most and keep spawning until you flatten them.
2. **Claim what drops.** Gold orbs belong to nobody. Cast **Claim** (`0`) near them and they turn **white** — yours. Possessing mana also permanently widens your own mana pool, which is the only way to afford the expensive spells later.
3. **Let the balloons work.** The hot-air balloons circling your keep are automated collectors. They only fetch orbs *you* have claimed, and they carry them back to the fortress. This is what fills it.
4. **Fill the fortress.** The realm is won when your fortress holds the target amount of mana. Your rival is filling theirs the same way.
5. **Upgrade to hold more.** A fortress can only store `240 x tier`. When the fill bar hits the pale tick, you are capped — fly home and cast **Fortress** (`[`) over your own keep to add a tier, new walls and towers included. Each tier costs more than the last.
6. **Fight the rival.** They run the same loop. Kill their balloons and their claimed mana never arrives. Crack the keep and it spills what it holds.

## Architecture

The repository now contains a platform-neutral 128 Hz multiplayer foundation
alongside the playable browser compatibility client. See
[`docs/multiplayer-architecture.md`](docs/multiplayer-architecture.md) for the
runtime topology, security boundaries, rollout gates, and the precise list of
external production integrations.

```
crates/aetherloom-protocol    versioned commands, snapshots, events, results
            │
crates/aetherloom-core        owned MatchState, checkpoints, replication
       ┌────┴──────────────┐
       │                   │
aetherloom-client     aetherloom-server + aetherloom-quic
prediction/input/     fixed scheduler, hosts, native QUIC I/O
platform ABI                 │
                    apps/aetherloom-dedicated
                     one authoritative match
                              │
                    server/cloudflare
              profiles, islands, matchmaking, settlement

src/lib.rs  ──cargo──► sim.wasm ──► site/game.js + site/engine.js
             offline/browser compatibility path, fixed at the same 128 Hz
```

### Current browser compatibility adapter

The current playable client still uses JavaScript for browser API calls:

| | lines |
|---|---|
| `engine.js` — WGSL shader source (not JavaScript at all) | 410 |
| `engine.js` — WebGPU API calls and pipeline descriptors | ~180 |
| `game.js` — DOM, events, Web Audio, canvas, pointer lock | ~200 |

This compatibility module remains freestanding and importless so
`standalone.html` still works. The portable client boundary is Rust-owned:
gameplay and presentation types do not depend on WebGPU, and web, desktop, and
vendor console renderers implement `RendererBackend` separately.

**The compatibility simulation is `no_std` on wasm and compiles to a
freestanding module with zero imports.** Terrain, creatures, AI, projectiles,
particles, spells, the mana economy and the win condition live in linear
memory as flat arrays. JS maps `memory.buffer` once and reads typed views.

Authoritative catch-up ticks do not rebuild presentation. `game.js` advances
exact 7.8125 ms ticks and extracts GPU-ready instance data once per rendered
frame.

### WASM ABI

Flat and C-like on purpose, so the core is replaceable:

```
init(seed: u32, level: i32)          authoritativeHz() → 128
advanceTick()                        extractFrame()
simulationTick()                     step(dt)  // fixed-tick compatibility
setInput(fwd, strafe, up, dyaw, dpitch, fire, brake)
cast(spell) / fireSelected() / selectSpell(i) / cycleSpell(dir)

heightPtr()  → f32[320*320]     dirtyLoRow()/dirtyHiRow()/clearDirty()
instPtr()    → f32[n*14]        instCount()
partPtr()    → f32[n*8]         partCount()
mapPtr()     → f32[n*4]         mapCount()
evtPtr()     → f32[n*4]         evtCount() / clearEvents()
statePtr()   → f32[128]         terrainWidth() / cellSize() / worldSize()
```

`step(dt)` retains the old shape for existing hosts, but deliberately ignores
`dt`; callers cannot alter authoritative time. Sound events remain queued
across ticks so catch-up simulation cannot
erase a cue before JavaScript reads it. The consumer calls `clearEvents()` only
after processing the current `evtPtr()` / `evtCount()` contents.

### Models

```
src/render/
  mod.rs       buffers, the frame, the shared body/detail/variation helpers
  mesh.rs      the ten prototype meshes and the conventions they obey
  scenery.rs   palms, boulders, burnt stumps
  castle.rs    keeps and the ruins they leave
  ground.rs    walkers: worm, troll, nest, villager, soldier
  flyer.rs     wasp, griffin, dragon, wraith, balloon, and the wing they share
  effects.rs   carpets, riders, mana orbs, projectiles, the fireball
  minimap.rs   the minimap raster
```

What actually stops assembled primitives reading as a pile of shapes, in
rough order of effect: per-instance orientation; per-entity variation; joints
that overlap by 15-30% rather than butting; silhouettes broken by a spine, a
strap or a snapped edge at every rim; left/right pairs that are never perfect
mirrors; a slightly different tint on every piece; and chains — a neck, a tail,
a trunk — that taper *and* curve *and* twist rather than stepping in a line.

### Rendering

- **Terrain** is a static 320×320 grid mesh displaced in the vertex shader from an `r32float` height texture, with normals from neighbour taps. Deforming the world means writing one dirty row-band per frame — the mesh never changes. `bytesPerRow` is 1280, so the 256-byte alignment rule is satisfied for free.
- **Pixels live on the edges, not on everything.** The scene renders at full resolution and stays smooth across open sky, water and grass. In the composite pass each screen-space block compares itself against its four neighbours; where that contrast is high — a silhouette, a shoreline, the line between grass and rock, the rim of a cloud — the block collapses to a single flat colour and the animated static, 4×4 Bayer dither and 20-level quantise fade in with it. Everywhere else keeps full precision. Blanket low-res pixelation puts the same chunk size on a blank sky as on a tree, which reads as a broken display rather than an art style. The 4px block is fixed — it is the art direction, not a setting.
- HDR `rgba16float` target, threshold → separable blur bloom at quarter res, ACES tonemap, vignette.
- **Draw range is deliberately short** (1600 units, on a 2560-unit map). Haze is exponential up close and then closed off hard by a `smoothstep` on `dist / far`, so the world always reaches full sky before the far plane and the clipped edge of the sea is never visible. The haze colour is `skyColor()` evaluated with the ray clamped to the horizon — a downward ray through thick air ends in bright air, not in the dark band the sky puts below the horizon — and the sky holds that same colour flat at and below the horizon, so the join is seamless wherever the world runs out.
- **Everything solid is instanced from ten procedural prototypes**, partitioned by shape each frame: box, sphere, cone, cylinder, frustum (a box tapering to 45%), wedge, **frond** (a leaf with its droop, taper, central rib and cut leaflets in the mesh), **boulder** (an irregular flat-shaded lump), **crenels** (a whole parapet ring — merlons, embrasures and walkway in one instance), and a flat disc used only for shadows. The last four exist because a palm frond drawn as five cuboids costs five instances and still reads as five cuboids; drawn as one `Frond` it costs one and reads as a leaf. Cost is per instance, not per triangle, so an intricate mesh is always cheaper than the boxes that approximate it.
- **Instances carry pitch and roll, not just yaw.** Yaw alone leaves every piece of every model square to the world axes, which is most of why parts assembled from prototypes read as a stack of blocks rather than as a creature. Drooping fronds, splayed limbs, canted roofs and banked wings all need the other two angles. Local +Z is forward, +X is the model's own right; positive pitch tips the nose down, positive roll raises the right side; applied roll, then pitch, then yaw.
- **Nothing is allowed to come out the same twice.** Every creature hashes its slot index, every scenery item hashes its own, and every proportion, count, angle, tint and optional feature comes off that hash. A grove where each tree carries the same seven fronds at the same seven bearings reads as wallpaper however well one tree is built — and the same is true of a keep's garrison. The hash is an integer mixer, not the usual `fract(sin(x) * 43758.5)`: at f32 precision that lands on a couple of hundred distinct values and correlates strongly between neighbouring slots, and neighbouring slots are exactly what a clump is made of.
- **Models thin out with distance.** Creatures, scenery and keeps each resolve a detail tier from range and drop trim first, then limbs, then everything but the silhouette. A troll is sixty pieces close up and five across the bay. The ceilings are 55 instances per creature at full detail (90 for the dragon), 18 reduced, 5 distant; 26/10/2 for a palm, 6/3/1 for a boulder, 183 for a max-visual-tier keep. Economic fortress tiers can keep rising, while model complexity stops growing after visual tier six. Several models now sit *exactly* on their limit, so adding a piece means taking one away. Scenery is submitted last and to its own cap, so when a frame does run out it is distant trees that stop rather than the creature about to eat you.
- **Shading is stepped, which is what gives shadows a pixel edge.** A smooth lambert ramp has no contrast boundary anywhere along it, so the edge pass never finds one and shadows stay airbrushed. The diffuse term is snapped to eight levels and pulled 58% of the way toward them, so shaded faces and hillsides get hard terminators the pixel pass can bite on; the steps ease back to smooth past 260 units so distant slopes don't band into stripes.
- **Ground shadows** are flat discs laid under every caster, depth-tested but never depth-written, multiplied into the scene rather than painted over it. Their falloff is snapped to a grid in the disc's own space, so the rim breaks into steps instead of fading out. A blob broadens and fades with the caster's altitude — flying over a flat sea of terrain, it is the only read you get on your own height, which is why the player's shadow is pushed *outside* the instance range that first-person view drops.
- **Scenery is drawn last, to a budget, nearest tier first.** Palms and boulders outnumber everything else by two orders of magnitude, and a hillside of trees must never crowd the creature about to eat you out of the instance buffer. Each item is built from up to twenty pieces close up and two at range; when the budget runs out it is the far tier that stops, which is the one place it isn't noticed.
- Particles are additive camera-facing billboards with a radial falloff computed in the fragment shader — no textures anywhere in the project.
- Water samples the height texture for depth-based colour, shoreline foam and specular.
- Audio is synthesised from oscillators and one noise buffer; the sim emits positioned event codes and JS turns them into sound.
- **The HUD is icons, not words.** Every symbol — life, mana, the two keeps on the claim bar, all thirteen spells — is an 8×8 pixel grid in `game.js`, one character per pixel, expanded to inline SVG with `shape-rendering="crispEdges"` and runs of equal pixels merged into single rects. Spell names live in the tooltip. There is no screen border and no permanent text readout.
- **Scenery is clumped, not scattered.** Placement draws a couple of hundred seed points with their own spreads and then draws around them, biased toward the middle, with a fifth left loose. Uniform noise puts an even grey rash on every hillside; clumps give you palm groves and boulder fields with clear ground between them. Boulders are overlapping squashed ellipsoids with one angular slab, their sizes and tint driven off each item's own stored spin so no two match — a single grey box reads as a crate, not as stone.
- Castles level their own pad at placement: flat right out past the plinth, then a smoothstep skirt down to the hill. Anything that just samples the ground at its centre point lifts off the downhill side of a slope, so scenery and nests are sunk below their sample instead.

## The simulation core

`src/lib.rs` and its modules use `no_std` on `wasm32-unknown-unknown` and build
as both a `cdylib` and an `rlib`. The wasm path has no allocator, panic runtime,
or host interface. `libm` supplies transcendentals and `spin` provides the
compatibility ABI lock. `build.mjs` fails if the output gains an import.

```
src/
  lib.rs        crate attributes, the world singleton, and the exported ABI
  types.rs      creature kinds, spells, factions, shapes, blips, cues
  world.rs      state layout, the RNG, and shared operations
  terrain.rs    heightmap sampling, noise, deformation, realm generation
  session.rs    realm setup and the fixed-tick update order
  spells.rs     casting: one function per spell
  creatures.rs  creature AI, projectiles, orbs, particles
  wizards.rs    player flight and the rival AI
  render/       instance and particle buffers, meshes, every model, minimap
```

### No unsafe

The crate is `#![deny(unsafe_code)]` and contains **no unsafe blocks at all**. State lives in a `spin::Mutex<World>`; each exported function takes the lock once and passes `&mut World` down, so every line of logic is ordinary safe Rust. The only exemptions are the `#[no_mangle]` attributes on the ABI itself, which modern Rust classes as unsafe because an unmangled symbol can collide at link time — unavoidable for a module that has to expose named exports.

`World::new()` is deliberately all zeros. A single non-zero field anywhere in it moves two megabytes of arrays out of `.bss` and into the wasm as initialised data — that mistake took the module from 94 KB to 1.8 MB. The RNG therefore starts at zero and repairs itself on first use.

### Things that will bite

- **Presentation is not authoritative.** `advanceTick()` never calls
  `build_frame`; `extractFrame()` does. Generation, AI, combat, environment,
  and cosmetic RNG streams are independent, so changing a particle cannot
  alter replay state.
- **Wrapping arithmetic.** The value-noise hash multiplies deliberately overflow. Rust panics on overflow in debug where the original wrapped silently, so those sites use `wrapping_mul`/`wrapping_add`. Without them the same seed grows different terrain.
- **Random-stream order is load-bearing inside each subsystem.** Random draws
  in AI conditions stay inside those conditions, and authoritative hashes
  exclude the cosmetic stream.

```rust
static WORLD: spin::Mutex<World> = spin::Mutex::new(World::new());

#[allow(unsafe_code)] #[no_mangle]
pub extern "C" fn advanceTick() {
    WORLD.lock().advance_tick(1.0 / 128.0);
}
```

## Build

```bash
rustup target add wasm32-unknown-unknown
npm run build      # cargo → site/sim.wasm, then inlines site/ into standalone.html
npm test           # workspace tests, fresh wasm, balance, renderer and control-plane checks
```

`npm run build -- --no-wasm` skips cargo and rebuilds `standalone.html` from the existing `site/sim.wasm`. The extra `--` is required so npm forwards the flag to `build.mjs`. There are no npm dependencies — node is only used for the bundler, the static server and the test harnesses.

`npm test` first rebuilds `site/sim.wasm` from the current Rust sources, then runs:

- **The Rust workspace suites** cover command/protocol fuzz boundaries,
  deterministic replay/checkpoint restore, stable entity generations,
  prediction/reconciliation, transport impairment, and the 128-player
  performance-gate harness.
- **`model-preview-test.mjs`** renders all 30 gallery scenes in all eight variants directly through the WASM preview ABI. It rejects non-finite or out-of-bounds transforms, illegal shapes, invalid sizes and colours, instance-capacity exhaustion, nondeterministic output, and changes to the reviewed byte-level signature baseline.
- **`balance-sim-test.mjs`** checks event-queue lifetime, the realm-eight capacity boundary, and the park-on-castle exploit before driving the WASM core with a scripted bot for thousands of simulated seconds across several seeds and difficulty levels. Win rates remain diagnostic, while invalid outcomes, non-finite state, and broken fortress-capacity invariants fail the test.
- **`headless-render-test.mjs`** runs the real game loop against a WebGPU stub that *validates arguments* the way the driver would — buffer overflows, non-4-aligned writes, `bytesPerRow` alignment, reads past the end of source arrays. Any validation error fails the process. It also checks every prototype mesh fits its arrays and that no index dangles: the mesh builders drop vertices silently when a shape outgrows its capacity, so an overflowing prototype renders with holes and nothing says why.
- **The Cloudflare validator** checks bindings, schemas, migrations, security
  contracts, and the intentionally explicit unbound-island state. CI also
  installs and strictly type-checks the Worker package before publishing.

## Deploy

The deployed result is static files with no runtime server build or special
headers. Deployment CI still compiles and tests the source before publishing.

**GitHub Pages** — `.github/workflows/pages.yml` installs the Rust WASM target,
runs the fresh-build test suite, and publishes the resulting `site/` on every
push to `main`, including `models.html` and `models.js`. Set Settings → Pages →
Source to **GitHub Actions** once, then:
```bash
git push
```
(A *branch* deploy will not work here: GitHub only offers `/` or `/docs` as the folder, and the playable files live in `site/`.)

**Cloudflare Pages / Netlify**
```bash
npx wrangler pages deploy site --project-name aetherloom
netlify deploy --prod --dir site
```

**Vercel** — `vercel --prod` from inside `site/`.

WASM is fetched with `fetch` + `arrayBuffer`, not `instantiateStreaming`, so hosts that serve `.wasm` with the wrong MIME type work anyway.

## What's simulated

A 2560-unit archipelago on a 320x320 heightmap · deformable terrain (craters, raised volcanoes, travelling quake ripples) · procedural archipelago generation per realm · carpet flight with banking and terrain following · 12 spells unlocking across realms · 10 creature types with state-machine AI including burrowers, swarms, ranged throwers, flyers and dragons · villagers and soldiers garrisoning every keep, leashed to it, the villagers fleeing anything hostile · dragons that cruise at altitude and swim through the air on a travelling body wave · spawning nests, placed clear of your own keep so the opening minute is exploration rather than a dogfight on the doorstep · rival wizard AI with gather/bank/duel/flee behaviours · mana-ferrying balloons · castles with tiers, HP and destruction that spills their stored mana · **palm fires**: a firebolt sets a tree alight, it burns for seven seconds, spreads to its neighbours and leaves a charred stump · out-of-combat and at-castle regeneration · positioned audio events.

## Licence

Original code, MIT. *Magic Carpet* is a trademark of Electronic Arts; this project is an independent homage and contains none of its code or assets.
