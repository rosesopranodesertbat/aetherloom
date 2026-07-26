# Aetherloom

A flying-carpet wizard duel that runs in a browser tab. Original game, built as an homage to Bullfrog's *Magic Carpet* (1994) and *Magic Carpet 2* (1995) — the terrain you can blow holes in, the mana you have to physically carry home, and a rival wizard racing you for the same pool.

No original assets are used. Every texture, shape, sound and level is generated procedurally at runtime.

## Play it

**Fastest:** open `standalone.html` directly. One file, ~180 KB, WASM embedded as base64, no server needed.

**As a site:**
```bash
node serve.mjs        # → http://localhost:8080
```
(ES modules and `fetch` need a real origin, so `site/index.html` won't work over `file://` — that's what `standalone.html` is for.)

Requires **WebGPU**: Chrome/Edge 113+, Firefox 141+, Safari 18.2+.

## Controls

| | |
|---|---|
| `W` / `A` `D` | thrust, slide left and right |
| Mouse | steer (click the sky to capture the pointer, `Esc` releases) |
| `Space` / `Shift` | climb, dive |
| `X` | hold to slow down |
| Left click | cast the held spell |
| `1`–`9` `0` `-` `=` | pick a spell · `Q`/`E` or wheel to cycle |
| `C` | first-person ↔ chase view (first person hides your own carpet) |
| `V` | edge pixel size — 4 · 3 · 6 px · off |
| `P` pause · `M` mute · `B` bloom · `Shift+R` restart realm |

## How it plays

Mana is the whole game. It doesn't come out of thin air — it comes out of things you kill.

1. **Break things.** Wildlife and their nests drop mana when they die. Nests are worth the most and keep spawning until you flatten them.
2. **Fly low to collect.** Loose mana is drawn to a passing carpet within ~110 units, absorbed at ~42. Cruise high and you'll fly straight over it.
3. **Carry it home.** Only mana you're *carrying* counts. Fly over your keep and everything above your reserve is banked — banked mana is what wins the realm, raises your castle tier, and makes your spell fuel regenerate faster.
4. **Spend or bank.** Your castle only tops you back up to the reserve floor, so every big spell is paid for out of the mana you were going to bank. That's the tension.
5. **Fight the rival.** They run the same loop, and they're a fair bit faster at it as realms get harder. Their balloons ferry mana home for them. Kill the balloons. Crack the keep and it spills three quarters of everything they've banked.

Claim half the realm before they do. Twelve spells unlock across the realms; from realm 5 there are two rivals.

## Architecture

Two halves that barely talk to each other, which is the point.

```
src/sim.ts   ──asc──►  sim.wasm      the entire simulation
                          │           (no imports, no JS calls, no GC)
                          │  linear memory, read directly as typed arrays
                          ▼
site/engine.js         WebGPU renderer + WGSL  ─────►  screen
site/game.js           input, HUD, audio, level flow
```

**The simulation is a freestanding WASM module with zero imports.** Terrain, creatures, AI, projectiles, particles, spells, the mana economy and the win condition all live in linear memory as flat `f32` arrays. JS never marshals anything: it maps `memory.buffer` once and reads through `Float32Array` views.

The sim doesn't return "entities" — it writes **GPU-ready instance data**. Each frame it emits a packed instance array (position, scale, colour, yaw, glow, shape) that goes almost straight into `writeBuffer`. Measured cost: **~81 µs per 60 Hz step** (~12,000 steps/sec of headroom), so the simulation uses well under 1% of a frame.

### WASM ABI

Flat and C-like on purpose, so the core is replaceable:

```
init(seed: u32, level: i32)          step(dt: f32)
setInput(fwd, strafe, up, dyaw, dpitch, fire, brake)
cast(spell) / fireSelected() / selectSpell(i) / cycleSpell(dir)

heightPtr()  → f32[256*256]     dirtyLoRow()/dirtyHiRow()/clearDirty()
instPtr()    → f32[n*12]        instCount()
partPtr()    → f32[n*8]         partCount()
mapPtr()     → f32[n*4]         mapCount()
evtPtr()     → f32[n*4]         evtCount()
statePtr()   → f32[128]         terrainWidth() / cellSize() / worldSize()
```

### Rendering

- **Terrain** is a static 256×256 grid mesh displaced in the vertex shader from an `r32float` height texture, with normals from neighbour taps. Deforming the world means writing one dirty row-band per frame — the mesh never changes. `bytesPerRow` is 1024, so the 256-byte alignment rule is satisfied for free.
- **Pixels live on the edges, not on everything.** The scene renders at full resolution and stays smooth across open sky, water and grass. In the composite pass each screen-space block compares itself against its four neighbours; where that contrast is high — a silhouette, a shoreline, the line between grass and rock, the rim of a cloud — the block collapses to a single flat colour and the animated static, 4×4 Bayer dither and 20-level quantise fade in with it. Everywhere else keeps full precision. Blanket low-res pixelation puts the same chunk size on a blank sky as on a tree, which reads as a broken display rather than an art style. `V` cycles 4 / 3 / 6 px / off.
- HDR `rgba16float` target, threshold → separable blur bloom at quarter res, ACES tonemap, vignette.
- **Draw range is deliberately short** (1300 units, on a 1024-unit map). Haze is exponential up close and then closed off hard by a `smoothstep` on `dist / far`, so the world always reaches full sky before the far plane and the clipped edge of the sea is never visible. The haze colour is `skyColor()` evaluated with the ray clamped to the horizon — a downward ray through thick air ends in bright air, not in the dark band the sky puts below the horizon — and the sky holds that same colour flat at and below the horizon, so the join is seamless wherever the world runs out.
- Everything solid is instanced from three procedural prototypes (box, sphere, cone) partitioned by shape each frame; creatures are assembled from a few parts each.
- Particles are additive camera-facing billboards with a radial falloff computed in the fragment shader — no textures anywhere in the project.
- Water samples the height texture for depth-based colour, shoreline foam and specular.
- Audio is synthesised from oscillators and one noise buffer; the sim emits positioned event codes and JS turns them into sound.

## A note on Rust

The brief asked for Rust. I built this in a sandbox where `static.rust-lang.org` and `sh.rustup.rs` both return HTTP 403, and Ubuntu's packaged `rustc` ships no `wasm32-unknown-unknown` standard library — so no Rust toolchain could be obtained or built. Rather than hand you untested Rust that I could never compile, I wrote the core in **AssemblyScript**, which compiles through Binaryen to real WebAssembly. The output is genuine `wasm32`: zero imports, native math, `-O3`, 84 KB.

The ABI above is deliberately Rust-shaped. Porting is mechanical rather than a rewrite:

```rust
static mut HEIGHT: [f32; 256 * 256] = [0.0; 256 * 256];

#[no_mangle] pub extern "C" fn height_ptr() -> *const f32 { unsafe { HEIGHT.as_ptr() } }
#[no_mangle] pub extern "C" fn step(dt: f32) { /* ... */ }
```

Build with `cargo build --release --target wasm32-unknown-unknown` (no `wasm-bindgen` needed — the interface is pointers and scalars), point `game.js` at the new `.wasm`, and nothing else changes. The struct-of-arrays layout, the dirty-row protocol and the instance packing are all already in the shape Rust would want.

## Build

```bash
npm install
npm run build      # compiles src/sim.ts → wasm, writes dist/ + standalone.html
npm test           # headless sim balance sweep + validating render harness
```

`npm test` runs two things worth knowing about:

- **`balance-sim-test.mjs`** drives the WASM core with a scripted bot for thousands of simulated seconds across several seeds and difficulty levels, and reports who won. This is how the economy was tuned — it caught a faction-ID bug where player spells couldn't damage wild creatures at all, and an exploit where parking on your own castle won the level.
- **`headless-render-test.mjs`** runs the real game loop against a WebGPU stub that *validates arguments* the way the driver would — buffer overflows, non-4-aligned writes, `bytesPerRow` alignment, reads past the end of source arrays. Useful because it catches renderer bugs without a GPU.

## Deploy

Static files, no build step on the server, no special headers.

**GitHub Pages** — `.github/workflows/pages.yml` publishes `site/` on every push to `main`. Set Settings → Pages → Source to **GitHub Actions** once, then:
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

Deformable heightmap terrain (craters, raised volcanoes, travelling quake ripples) · procedural archipelago generation per realm · carpet flight with banking and terrain following · 12 spells unlocking across realms · 8 creature types with state-machine AI including burrowers, swarms, ranged throwers, flyers and dragons · spawning nests · rival wizard AI with gather/bank/duel/flee behaviours · mana-ferrying balloons · castles with tiers, HP and destruction that spills their stored mana · out-of-combat and at-castle regeneration · positioned audio events.

## Licence

Original code, MIT. *Magic Carpet* is a trademark of Electronic Arts; this project is an independent homage and contains none of its code or assets.
