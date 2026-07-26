// ============================================================================
// AETHERLOOM — WebGPU renderer + glue. The simulation lives in WASM (sim.wasm);
// this file only draws it and feeds it input.
// ============================================================================
export const WGSL = /* wgsl */`
struct Uni {
  vp       : mat4x4<f32>,
  ivp      : mat4x4<f32>,
  cam      : vec4<f32>,   // xyz, time
  sun      : vec4<f32>,   // dir xyz, intensity
  fog      : vec4<f32>,   // rgb, density
  misc     : vec4<f32>,   // terrainW, cell, seaLevel, aspect
  misc2    : vec4<f32>,   // exposure, bloomStrength, waterAlpha, hurt
  castle0  : vec4<f32>,   // x, z, radius, faction
  castle1  : vec4<f32>,
  castle2  : vec4<f32>,
  grade    : vec4<f32>,   // paletteLevels, grain, dither, drawRange
};
@group(0) @binding(0) var<uniform> U : Uni;

// ---------------------------------------------------------------- helpers
// Dave Hoskins' hash12. The usual fract(dot(p, vec2(127.1, 311.7)) * 43758.5)
// form needs f64 to work: its intermediate lands around 1e8, which blows past
// the 24-bit f32 mantissa, so on the GPU it degenerates to almost a constant
// and every fbm2 built on it comes out flat. This variant stays under ~2e4.
fn hash21(p: vec2<f32>) -> f32 {
  var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
  p3 += dot(p3, vec3<f32>(p3.y, p3.z, p3.x) + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}
fn vnoise(p: vec2<f32>) -> f32 {
  let i = floor(p); let f = fract(p);
  let u = f * f * (3.0 - 2.0 * f);
  let a = hash21(i);
  let b = hash21(i + vec2<f32>(1.0, 0.0));
  let c = hash21(i + vec2<f32>(0.0, 1.0));
  let d = hash21(i + vec2<f32>(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}
fn fbm2(p0: vec2<f32>) -> f32 {
  var s = 0.0; var a = 0.5; var p = p0;
  for (var i = 0; i < 5; i = i + 1) { s = s + a * vnoise(p); p = p * 2.02; a = a * 0.5; }
  return s;
}
fn skyColor(dir: vec3<f32>) -> vec3<f32> {
  let up = clamp(dir.y, -1.0, 1.0);
  // Clear summer midday: strong blue overhead washing to pale blue haze.
  let horizon = vec3<f32>(0.70, 0.84, 0.97);
  let zenith  = vec3<f32>(0.10, 0.34, 0.86);
  let below   = vec3<f32>(0.58, 0.73, 0.90);
  var col = mix(horizon, zenith, pow(clamp(up, 0.0, 1.0), 0.62));
  // Held flat at and below the horizon: with a short draw range you see this
  // band wherever the world runs out, and it has to read as more haze, not a
  // void. applyFog converges on exactly this colour, so the join is invisible.
  col = mix(below, col, clamp(max(up, 0.0) * 5.0 + 0.66, 0.0, 1.0));
  let sd = max(dot(dir, normalize(U.sun.xyz)), 0.0);
  col += vec3<f32>(1.0, 0.95, 0.80) * pow(sd, 220.0) * 6.0;      // sun disc
  col += vec3<f32>(1.0, 0.90, 0.66) * pow(sd, 8.0) * 0.24;        // glow
  // Drifting fair-weather cloud. Kept sparse and high-contrast: broad soft
  // coverage just milks the blue out instead of reading as cloud.
  if (dir.y > 0.02) {
    // Clamped rather than the true plane projection: near the horizon that
    // would run off to infinity and alias into fizz the edge pass would then
    // happily chop into blocks.
    let uv = dir.xz / max(dir.y, 0.18) * 1.7 + vec2<f32>(U.cam.w * 0.012, 0.0);
    // Narrow band: fbm2 is dominated by its first octave, so a wide threshold
    // gives a smooth wash rather than anything that reads as a cloud.
    let c = smoothstep(0.58, 0.70, fbm2(uv));
    // Gone by the time that clamp bites, or the clamped projection smears the
    // cloud field into vertical streaks across the horizon.
    col = mix(col, vec3<f32>(1.10, 1.08, 1.04), c * 0.95 * smoothstep(0.17, 0.46, dir.y));
  }
  return col;
}
// Haze that closes off completely at the draw range, so the far plane never
// shows an edge — the world just dissolves into the sky the way it used to.
fn applyFog(col: vec3<f32>, dist: f32, dir: vec3<f32>) -> vec3<f32> {
  let f = 1.0 - exp(-dist * U.fog.w);
  let edge = smoothstep(0.45, 1.0, dist / max(U.grade.w, 1.0));
  let a = clamp(max(f, edge), 0.0, 1.0);
  // Haze is lit like the horizon, never like the dark band skyColor puts below
  // it — a downward ray through thick air still ends in bright air. Clamping to
  // y >= 0 also makes the far edge meet the sky pass seamlessly at the horizon.
  let haze = skyColor(normalize(vec3<f32>(dir.x, max(dir.y, 0.0), dir.z)));
  // near haze keeps the warm fog tint; at full range it is pure horizon air
  return mix(col, mix(U.fog.rgb, haze, 0.55 + 0.45 * a), a);
}
fn territory(wp: vec2<f32>) -> vec3<f32> {
  var tint = vec3<f32>(0.0);
  var cs = array<vec4<f32>, 3>(U.castle0, U.castle1, U.castle2);
  for (var i = 0; i < 3; i = i + 1) {
    let c = cs[i];
    if (c.z <= 0.0) { continue; }
    let d = length(wp - c.xy) / c.z;
    if (d < 1.0) {
      let w = pow(1.0 - d, 2.0) * 0.30;
      if (c.w < 0.5) { tint += vec3<f32>(0.10, 0.26, 0.72) * w; }
      else           { tint += vec3<f32>(0.72, 0.13, 0.10) * w; }
    }
  }
  return tint;
}

// Lighting is stepped rather than smooth, which is what gives shaded faces and
// terrain shadows a hard boundary for the edge pass to find and blockify. A
// smooth ramp has no contrast edge anywhere along it, so it never pixelates.
// Steps are eased back with distance so far hillsides do not band into stripes.
fn terraceLight(v: f32, dist: f32) -> f32 {
  let steps = 8.0;
  let stepped = floor(v * steps + 0.5) / steps;
  // Pulled part of the way to the steps, not all: full quantisation turns a
  // whole hillside into one flat slab of shade, and banding a curved surface
  // into halves reads as a fault rather than a shadow.
  let terraced = mix(v, stepped, 0.58);
  return mix(terraced, v, smoothstep(260.0, 900.0, dist));
}

// ---------------------------------------------------------------- sky pass
struct FSOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn fsTri(@builtin(vertex_index) vi: u32) -> FSOut {
  var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -3.0), vec2<f32>(-1.0, 1.0), vec2<f32>(3.0, 1.0));
  var o: FSOut;
  o.pos = vec4<f32>(p[vi], 1.0, 1.0);
  // NDC is y-up, texture v is y-down: flip so uv.y = 0 is the top row.
  o.uv = vec2<f32>(p[vi].x, -p[vi].y) * 0.5 + 0.5;
  return o;
}
@fragment fn skyFS(i: FSOut) -> @location(0) vec4<f32> {
  let ndc = vec4<f32>(i.uv.x * 2.0 - 1.0, (1.0 - i.uv.y) * 2.0 - 1.0, 1.0, 1.0);
  let w = U.ivp * ndc;
  let dir = normalize(w.xyz / w.w - U.cam.xyz);
  return vec4<f32>(skyColor(dir), 1.0);
}

// ---------------------------------------------------------------- terrain
@group(1) @binding(0) var hmap : texture_2d<f32>;
fn hAt(ij: vec2<i32>) -> f32 {
  let w = i32(U.misc.x);
  let c = clamp(ij, vec2<i32>(0, 0), vec2<i32>(w - 1, w - 1));
  return textureLoad(hmap, c, 0).r;
}
struct TOut {
  @builtin(position) pos : vec4<f32>,
  @location(0) wp   : vec3<f32>,
  @location(1) nrm  : vec3<f32>,
  @location(2) hgt  : f32,
};
@vertex fn terrVS(@location(0) grid: vec2<f32>) -> TOut {
  let cell = U.misc.y;
  let ij = vec2<i32>(i32(grid.x), i32(grid.y));
  let h = hAt(ij);
  let hl = hAt(ij + vec2<i32>(-1, 0)); let hr = hAt(ij + vec2<i32>(1, 0));
  let hd = hAt(ij + vec2<i32>(0, -1)); let hu = hAt(ij + vec2<i32>(0, 1));
  var o: TOut;
  o.wp = vec3<f32>(grid.x * cell, h, grid.y * cell);
  o.nrm = normalize(vec3<f32>(hl - hr, 2.0 * cell, hd - hu));
  o.hgt = h;
  o.pos = U.vp * vec4<f32>(o.wp, 1.0);
  return o;
}
@fragment fn terrFS(i: TOut) -> @location(0) vec4<f32> {
  let n = normalize(i.nrm);
  let slope = 1.0 - clamp(n.y, 0.0, 1.0);
  let h = i.hgt;
  let p = i.wp.xz;

  // Four bands of noise, each doing a different job. All are deliberately low
  // frequency: with a working hash, a 0.6-scale term is sub-pixel at range and
  // aliases into fizz the edge pass would block.
  let region  = fbm2(p * 0.0038);            // which biome this ground leans to
  let soil    = fbm2(p * 0.011);             // meadow / scrub / bare-dirt drift
  let grain   = fbm2(p * 0.055) * 0.15 + vnoise(p * 0.22) * 0.05;
  let mottle  = vnoise(p * 0.09);            // clumpy tonal blotches

  let seabed = vec3<f32>(0.20, 0.26, 0.24);
  let sand   = vec3<f32>(0.80, 0.68, 0.42);
  let dirt   = vec3<f32>(0.44, 0.34, 0.22);
  let grass  = vec3<f32>(0.26, 0.44, 0.19);
  let dry    = vec3<f32>(0.47, 0.49, 0.22);
  let moss   = vec3<f32>(0.17, 0.34, 0.20);
  let rock   = vec3<f32>(0.40, 0.37, 0.34);
  let slate  = vec3<f32>(0.33, 0.31, 0.33);
  let snow   = vec3<f32>(0.90, 0.89, 0.84);

  // The green is never one green: it swings between lush, dry and mossy on a
  // very long wavelength, so no two hillsides read the same.
  var ground = mix(grass, dry, smoothstep(-0.10, 0.34, region));
  ground = mix(ground, moss, smoothstep(0.06, -0.30, region));
  // scrubby patches and bare earth inside each region
  ground = mix(ground, dirt, smoothstep(0.16, 0.42, soil) * 0.42);
  ground = mix(ground, dry,  smoothstep(-0.14, -0.40, soil) * 0.35);

  // Stone alternates between warm rock and cold slate in wide strata, which
  // gives the peaks visible bedding rather than one flat grey.
  let strata = sin(h * 0.09 + region * 4.0) * 0.5 + 0.5;
  let stone = mix(rock, slate, strata);

  var base = mix(seabed, sand, smoothstep(-14.0, 1.5, h));
  base = mix(base, ground, smoothstep(3.0, 16.0, h));
  base = mix(base, stone,  smoothstep(52.0, 104.0, h));
  base = mix(base, snow,   smoothstep(126.0, 176.0, h + mottle * 22.0));
  // anything steep sheds its soil and shows the rock underneath
  base = mix(base, stone, smoothstep(0.26, 0.66, slope) * 0.85);
  base *= (0.82 + grain * 1.35);
  base *= (0.90 + mottle * 0.21);
  base += territory(p);

  let toCam = i.wp - U.cam.xyz;
  let dist = length(toCam);
  let L = normalize(U.sun.xyz);
  let ndl = terraceLight(max(dot(n, L), 0.0), dist);
  let sky = 0.38 + 0.28 * clamp(n.y, 0.0, 1.0);
  var col = base * (sky * vec3<f32>(0.52, 0.62, 0.86) + ndl * vec3<f32>(1.24, 1.06, 0.80) * U.sun.w);
  // damp shoreline
  col *= mix(0.68, 1.0, smoothstep(-2.5, 3.0, h));

  col = applyFog(col, dist, normalize(toCam));
  return vec4<f32>(col, 1.0);
}

// ---------------------------------------------------------------- water
struct WOut { @builtin(position) pos: vec4<f32>, @location(0) wp: vec3<f32> };
@vertex fn waterVS(@location(0) g: vec2<f32>) -> WOut {
  var o: WOut;
  o.wp = vec3<f32>(g.x, U.misc.z, g.y);
  o.pos = U.vp * vec4<f32>(o.wp, 1.0);
  return o;
}
@fragment fn waterFS(i: WOut) -> @location(0) vec4<f32> {
  let t = U.cam.w;
  let p = i.wp.xz;
  let toCam = U.cam.xyz - i.wp;
  let dist = length(toCam);
  let w1 = sin(p.x * 0.055 + t * 1.1) * cos(p.y * 0.047 - t * 0.9);
  let w2 = sin(p.x * 0.017 - t * 0.6) * cos(p.y * 0.021 + t * 0.5);
  // Chop, faded out with range so it never goes sub-pixel and aliases. Two
  // smooth sinusoids alone read as an oil slick: their iso-contours show up as
  // rings once a tight specular lands on them.
  let near = 1.0 - smoothstep(80.0, 520.0, dist);
  let w3 = (vnoise(p * 0.10 + vec2<f32>(t * 0.30, t * -0.20)) - 0.5) * near;
  let w4 = (vnoise(p * 0.31 + vec2<f32>(t * -0.45, t * 0.36)) - 0.5) * near * 0.5;
  let ripple = vec3<f32>(w1 * 0.09 + w2 * 0.05 + w3 * 0.34 + w4 * 0.30, 1.0,
                         w2 * 0.09 + w1 * 0.05 + w4 * 0.34 - w3 * 0.30);
  let n = normalize(ripple);
  let V = toCam / dist;
  let L = normalize(U.sun.xyz);
  let fres = pow(1.0 - clamp(dot(n, V), 0.0, 1.0), 4.0);
  let refl = skyColor(reflect(-V, n));
  // sea bed showing through shallow water
  let ij = vec2<i32>(i32(floor(p.x / U.misc.y)), i32(floor(p.y / U.misc.y)));
  let W = i32(U.misc.x) - 1;
  var depth = 1.0;
  if (ij.x >= 0 && ij.y >= 0 && ij.x <= W && ij.y <= W) {
    let bed = textureLoad(hmap, ij, 0).r;
    depth = clamp((U.misc.z - bed) / 26.0, 0.0, 1.0);
  }
  let deep = vec3<f32>(0.02, 0.16, 0.42);
  let shallow = vec3<f32>(0.13, 0.58, 0.66);
  var col = mix(shallow, deep, depth);
  // Capped well below a mirror: at grazing angles a full sky reflection makes
  // the sea read as more sky and the horizon vanishes into the haze.
  col = mix(col, refl, clamp(fres * 0.62 + 0.06, 0.0, 1.0));
  let H = normalize(L + V);
  // Broad sheen, not a pinpoint: a tight exponent over a smooth normal field
  // traces thin contour lines the edge pass then outlines.
  col += vec3<f32>(1.0, 0.95, 0.78) * pow(max(dot(n, H), 0.0), 42.0) * 0.85;
  let foam = smoothstep(0.10, 0.0, depth) * (0.5 + 0.5 * sin(p.x * 0.4 + p.y * 0.33 + t * 3.0));
  col += vec3<f32>(0.9, 0.92, 0.88) * foam * 0.30;
  col = applyFog(col, dist, -V);
  let alpha = mix(0.62, 0.94, depth);
  return vec4<f32>(col, alpha * U.misc2.z);
}

// -------------------------------------------------------------- shadows
// A flat disc laid on the ground under each caster, darkening whatever it
// covers. The falloff is snapped to a grid in the disc's own space, so the rim
// breaks into steps: a shadow with a pixel edge, rather than an airbrushed
// blob that the edge pass would leave perfectly smooth.
struct ShOut {
  @builtin(position) pos   : vec4<f32>,
  @location(0) disc  : vec2<f32>,   // -1..1 across the disc
  @location(1) str   : f32,
  @location(2) wp    : vec3<f32>,
};
@vertex fn shadowVS(
  @location(0) vpos : vec3<f32>,
  @location(1) vnrm : vec3<f32>,
  @location(2) ipos : vec3<f32>,
  @location(3) iscl : vec3<f32>,
  @location(4) icol : vec3<f32>,
  @location(5) irot : vec3<f32>,
  @location(6) iext : vec2<f32>
) -> ShOut {
  var o: ShOut;
  o.disc = vpos.xz;
  o.str = icol.x;
  o.wp = ipos + vec3<f32>(vpos.x * iscl.x * 0.5, 0.0, vpos.z * iscl.z * 0.5);
  o.pos = U.vp * vec4<f32>(o.wp, 1.0);
  return o;
}
@fragment fn shadowFS(i: ShOut) -> @location(0) vec4<f32> {
  let steps = 6.0;
  let snapped = floor(i.disc * steps + 0.5) / steps;
  let r = length(snapped);
  if (r > 1.0) { discard; }
  // wide falloff: a hard-edged disc reads as a hole cut in the ground
  var a = i.str * (1.0 - smoothstep(0.08, 1.0, r));
  // gone by the time the haze has taken over, so shadows never float in fog
  a *= 1.0 - smoothstep(U.grade.w * 0.45, U.grade.w * 0.95, length(i.wp - U.cam.xyz));
  if (a <= 0.004) { discard; }
  return vec4<f32>(0.0, 0.0, 0.0, a);
}

// ---------------------------------------------------------------- solids
struct SOut {
  @builtin(position) pos : vec4<f32>,
  @location(0) wp   : vec3<f32>,
  @location(1) nrm  : vec3<f32>,
  @location(2) col  : vec3<f32>,
  @location(3) glow : f32,
};
// Yaw about Y, then pitch about X, then roll about Z, in the instance's own
// space. Yaw alone leaves every piece of every model square to the world axes,
// which is most of why parts assembled from prototypes read as a stack of
// blocks rather than as a creature.
fn instanceBasis(rot: vec3<f32>) -> mat3x3<f32> {
  let cy = cos(rot.x); let sy = sin(rot.x);
  let cp = cos(rot.y); let sp = sin(rot.y);
  let cr = cos(rot.z); let sr = sin(rot.z);
  // note the yaw convention: screen-right is (-cos, 0, sin), so the Y rotation
  // is transposed relative to the textbook form. Changing it silently mirrors
  // every model in the game.
  let yawM   = mat3x3<f32>(vec3<f32>( cy, 0.0, -sy), vec3<f32>(0.0, 1.0, 0.0), vec3<f32>( sy, 0.0,  cy));
  let pitchM = mat3x3<f32>(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0,  cp,  sp), vec3<f32>(0.0, -sp,  cp));
  let rollM  = mat3x3<f32>(vec3<f32>( cr,  sr, 0.0), vec3<f32>(-sr,  cr, 0.0), vec3<f32>(0.0, 0.0, 1.0));
  return yawM * pitchM * rollM;
}
@vertex fn solidVS(
  @location(0) vpos : vec3<f32>,
  @location(1) vnrm : vec3<f32>,
  @location(2) ipos : vec3<f32>,
  @location(3) iscl : vec3<f32>,
  @location(4) icol : vec3<f32>,
  @location(5) irot : vec3<f32>,  // yaw, pitch, roll
  @location(6) iext : vec2<f32>   // glow, shape
) -> SOut {
  let basis = instanceBasis(irot);
  // inverse-scale in object space *before* rotating, or non-uniform scale skews the normal
  let ns = vnrm / max(iscl, vec3<f32>(0.001));
  var o: SOut;
  o.wp = ipos + basis * (vpos * iscl * 0.5);
  o.nrm = normalize(basis * ns);
  o.col = icol;
  o.glow = iext.x;
  o.pos = U.vp * vec4<f32>(o.wp, 1.0);
  return o;
}
@fragment fn solidFS(i: SOut) -> @location(0) vec4<f32> {
  let n = normalize(i.nrm);
  let L = normalize(U.sun.xyz);
  let toCam = U.cam.xyz - i.wp;
  let V = normalize(toCam);
  let ndl = terraceLight(max(dot(n, L), 0.0), length(toCam));
  // Sky fill never reaches zero: an unlit face is in shade, not in a cave.
  let sky = 0.44 + 0.28 * clamp(n.y, 0.0, 1.0);
  var col = i.col * (sky * vec3<f32>(0.50, 0.60, 0.86) + ndl * vec3<f32>(1.22, 1.06, 0.82) * U.sun.w);
  let rim = pow(1.0 - clamp(dot(n, V), 0.0, 1.0), 2.5);
  col += i.col * rim * 0.30;
  col = mix(col, i.col * 2.9 + vec3<f32>(0.25), i.glow);
  col = applyFog(col, length(toCam), -V);
  return vec4<f32>(col, 1.0);
}

// ---------------------------------------------------------------- particles
struct POut {
  @builtin(position) pos : vec4<f32>,
  @location(0) uv  : vec2<f32>,
  @location(1) col : vec3<f32>,
  @location(2) a   : f32,
};
@vertex fn partVS(
  @builtin(vertex_index) vi : u32,
  @location(0) ipos : vec3<f32>,
  @location(1) isz  : f32,
  @location(2) icol : vec3<f32>,
  @location(3) ia   : f32
) -> POut {
  var q = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
    vec2<f32>(-1.0,  1.0), vec2<f32>(1.0, -1.0), vec2<f32>( 1.0, 1.0));
  let c = q[vi];
  // camera-facing basis
  let fwd = normalize(U.cam.xyz - ipos);
  var rgt = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), fwd));
  if (abs(fwd.y) > 0.995) { rgt = vec3<f32>(1.0, 0.0, 0.0); }
  let upv = cross(fwd, rgt);
  let wp = ipos + (rgt * c.x + upv * c.y) * isz;
  var o: POut;
  o.pos = U.vp * vec4<f32>(wp, 1.0);
  o.uv = c;
  o.col = icol;
  o.a = ia;
  return o;
}
@fragment fn partFS(i: POut) -> @location(0) vec4<f32> {
  let d = length(i.uv);
  if (d > 1.0) { discard; }
  let g = pow(1.0 - d, 2.2);
  return vec4<f32>(i.col * g * i.a * 2.1, g * i.a);
}

// ---------------------------------------------------------------- post
@group(1) @binding(3) var samp : sampler;
@group(1) @binding(1) var tex  : texture_2d<f32>;
@group(1) @binding(2) var tex2 : texture_2d<f32>;
@fragment fn brightFS(i: FSOut) -> @location(0) vec4<f32> {
  let c = textureSample(tex, samp, i.uv).rgb;
  let l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
  let k = max(l - 1.05, 0.0) / max(l, 0.0001);
  return vec4<f32>(c * k, 1.0);
}
@fragment fn blurHFS(i: FSOut) -> @location(0) vec4<f32> {
  let px = 1.0 / f32(textureDimensions(tex).x);
  var s = vec3<f32>(0.0);
  s += textureSample(tex, samp, i.uv + vec2<f32>(-4.0 * px, 0.0)).rgb * 0.05;
  s += textureSample(tex, samp, i.uv + vec2<f32>(-3.0 * px, 0.0)).rgb * 0.09;
  s += textureSample(tex, samp, i.uv + vec2<f32>(-2.0 * px, 0.0)).rgb * 0.12;
  s += textureSample(tex, samp, i.uv + vec2<f32>(-1.0 * px, 0.0)).rgb * 0.15;
  s += textureSample(tex, samp, i.uv).rgb * 0.18;
  s += textureSample(tex, samp, i.uv + vec2<f32>( 1.0 * px, 0.0)).rgb * 0.15;
  s += textureSample(tex, samp, i.uv + vec2<f32>( 2.0 * px, 0.0)).rgb * 0.12;
  s += textureSample(tex, samp, i.uv + vec2<f32>( 3.0 * px, 0.0)).rgb * 0.09;
  s += textureSample(tex, samp, i.uv + vec2<f32>( 4.0 * px, 0.0)).rgb * 0.05;
  return vec4<f32>(s, 1.0);
}
@fragment fn blurVFS(i: FSOut) -> @location(0) vec4<f32> {
  let px = 1.0 / f32(textureDimensions(tex).y);
  var s = vec3<f32>(0.0);
  s += textureSample(tex, samp, i.uv + vec2<f32>(0.0, -4.0 * px)).rgb * 0.05;
  s += textureSample(tex, samp, i.uv + vec2<f32>(0.0, -3.0 * px)).rgb * 0.09;
  s += textureSample(tex, samp, i.uv + vec2<f32>(0.0, -2.0 * px)).rgb * 0.12;
  s += textureSample(tex, samp, i.uv + vec2<f32>(0.0, -1.0 * px)).rgb * 0.15;
  s += textureSample(tex, samp, i.uv).rgb * 0.18;
  s += textureSample(tex, samp, i.uv + vec2<f32>(0.0,  1.0 * px)).rgb * 0.15;
  s += textureSample(tex, samp, i.uv + vec2<f32>(0.0,  2.0 * px)).rgb * 0.12;
  s += textureSample(tex, samp, i.uv + vec2<f32>(0.0,  3.0 * px)).rgb * 0.09;
  s += textureSample(tex, samp, i.uv + vec2<f32>(0.0,  4.0 * px)).rgb * 0.05;
  return vec4<f32>(s, 1.0);
}
fn aces(x: vec3<f32>) -> vec3<f32> {
  let a = 2.51; let b = 0.03; let c = 2.43; let d = 0.59; let e = 0.14;
  return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}
// 4x4 ordered (Bayer) threshold in [0,1) — the classic VGA dither cell.
fn bayer4(p: vec2<i32>) -> f32 {
  var m = array<f32, 16>(
     0.0,  8.0,  2.0, 10.0,
    12.0,  4.0, 14.0,  6.0,
     3.0, 11.0,  1.0,  9.0,
    15.0,  7.0, 13.0,  5.0);
  return m[(p.y & 3) * 4 + (p.x & 3)] / 16.0;
}
fn tone(scene: vec3<f32>, bloom: vec3<f32>) -> vec3<f32> {
  let x = aces((scene + bloom) * U.misc2.x);
  return pow(x, vec3<f32>(0.95, 0.99, 1.06));   // parchment-warm grade
}
@fragment fn compFS(i: FSOut) -> @location(0) vec4<f32> {
  // The scene is rendered at full resolution and stays that way across flat
  // surfaces. Chunky pixels are applied only where the picture actually
  // changes — silhouettes, shorelines, the line between grass and rock — which
  // is where the period look lives. Open sky and open water stay smooth.
  let dim = vec2<f32>(textureDimensions(tex));
  let B = max(U.grade.z, 1.0);
  let cell = floor(i.uv * dim / B);
  let bUV = (cell + 0.5) * B / dim;      // block centre: constant across a block
  let stp = B / dim;

  let blS = textureSample(tex2, samp, i.uv).rgb * U.misc2.y;
  let blB = textureSample(tex2, samp, bUV).rgb * U.misc2.y;
  let sharp  = tone(textureSample(tex, samp, i.uv).rgb, blS);
  let blocky = tone(textureSample(tex, samp, bUV).rgb, blB);
  // Contrast against the four neighbouring blocks. Every term here is constant
  // within a block, so the result snaps to the grid rather than smearing.
  let nl = tone(textureSample(tex, samp, bUV - vec2<f32>(stp.x, 0.0)).rgb, blB);
  let nr = tone(textureSample(tex, samp, bUV + vec2<f32>(stp.x, 0.0)).rgb, blB);
  let nd = tone(textureSample(tex, samp, bUV - vec2<f32>(0.0, stp.y)).rgb, blB);
  let nu = tone(textureSample(tex, samp, bUV + vec2<f32>(0.0, stp.y)).rgb, blB);
  var e = max(max(distance(blocky, nl), distance(blocky, nr)),
              max(distance(blocky, nd), distance(blocky, nu)));
  let k = smoothstep(0.085, 0.26, e);

  var c = mix(sharp, blocky, k);
  let q = i.uv - 0.5;
  c *= 1.0 - dot(q, q) * 0.42;
  // damage flash toward madder red
  c = mix(c, vec3<f32>(0.62, 0.09, 0.07), clamp(U.misc2.w, 0.0, 1.0) * 0.55);

  // Static and the hard palette step ride the same edge mask, so flat areas
  // keep full precision and never fizz.
  let bi = vec2<i32>(cell);
  let n = hash21(vec2<f32>(bi) + vec2<f32>(fract(U.cam.w * 7.3) * 311.0, fract(U.cam.w * 5.1) * 197.0));
  c += (n - 0.5) * U.grade.y * k;
  let L = mix(255.0, max(U.grade.x, 2.0), k);
  c = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
  c = floor(c * L + bayer4(bi)) / L;
  return vec4<f32>(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
`;

// Matrix math and the mesh prototypes now live in the Rust core (src/lib.rs).
// They are pure arithmetic with no browser surface, so there was no reason for
// them to be here. What remains in this file is the WebGPU API itself, which
// wasm cannot reach without an import bridge.


// ============================ renderer ======================================
// Instance and particle layout is read from the core at construction — see
// `this.maxInst` / `this.instFloats` below. Nothing here may hard-code it.
const PART_STRIDE = 32;
// Prototype meshes, in the order the core numbers them: cuboid, sphere, cone,
// then the ground-shadow disc, which is drawn by its own pass.
const SHADOW_SHAPE = 3;

export class Renderer {
  constructor(canvas, sim) {
    this.canvas = canvas; this.sim = sim; this.ok = false; this.bloom = true;
    this.block = 4;        // edge pixel size in CSS px — part of the look, fixed
    this.levels = 20;      // palette steps per channel, at edges only
    this.grain = 0.05;     // animated static, at edges only
    this.far = 1600;       // draw range; fog closes off completely by here
  }

  async init(terrainW, cellSize, seaLevel) {
    if (!navigator.gpu) throw new Error('This browser has no WebGPU support.');
    const adapter = await navigator.gpu.requestAdapter({ powerPreference: 'high-performance' });
    if (!adapter) throw new Error('No GPU adapter available. On Linux try enabling Vulkan; in Chrome check chrome://gpu.');
    this.device = await adapter.requestDevice();
    this.device.lost.then((i) => { this.lost = i.reason || 'unknown'; });
    this.adapterInfo = adapter.info ? `${adapter.info.vendor || ''} ${adapter.info.architecture || ''}`.trim() : '';
    const d = this.device;
    this.ctx = this.canvas.getContext('webgpu');
    this.fmt = navigator.gpu.getPreferredCanvasFormat();
    this.ctx.configure({ device: d, format: this.fmt, alphaMode: 'opaque' });
    this.TW = terrainW; this.cell = cellSize; this.sea = seaLevel;
    // No MSAA: the scene buffer is deliberately low-res, and smoothed edges
    // inside it would just soften the pixels we are trying to show.
    this.sampleCount = 1;

    // ---- uniforms
    // Uni = 2 mat4 (128B) + 9 vec4 (144B) = 272B
    this.uni = new Float32Array(68);
    this.uniBuf = d.createBuffer({ size: 272, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });

    // ---- meshes
    // Prototypes are generated in the core and read straight out of its
    // linear memory — nothing is built or copied on the JS side.
    this.sim.buildMeshes();
    // Prototype count, instance capacity and instance stride all come from the
    // core. Duplicating any of them here is how a frame ends up half-uploaded.
    this.shapeIds = Array.from({ length: this.sim.shapeCount() }, (_, i) => i);
    this.maxInst = this.sim.instCapacity();
    this.instFloats = this.sim.instStride();
    this.instStride = this.instFloats * 4;
    const mem = this.sim.memory.buffer;
    const mk = (s) => {
      const verts = new Float32Array(mem, this.sim.meshVertPtr(s), this.sim.meshVertFloats(s));
      const idx = new Uint16Array(mem, this.sim.meshIdxPtr(s), this.sim.meshIdxCount(s));
      const vb = d.createBuffer({ size: verts.byteLength, usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST });
      d.queue.writeBuffer(vb, 0, verts);
      const ibSize = Math.ceil(idx.byteLength / 4) * 4;
      const ib = d.createBuffer({ size: ibSize, usage: GPUBufferUsage.INDEX | GPUBufferUsage.COPY_DST });
      d.queue.writeBuffer(ib, 0, idx);
      return { vb, ib, count: idx.length };
    };
    this.shapes = this.shapeIds.map(mk);
    // 32 floats: view-projection, then its inverse
    this.camM = new Float32Array(mem, this.sim.camera(1.0, 1.0, 1, 2, 0, 0, 0, 0, 0, 1, 0, 1, 0), 32);

    // ---- terrain grid
    const W = terrainW;
    const gv = new Float32Array(W * W * 2);
    for (let j = 0; j < W; j++) for (let i = 0; i < W; i++) { const o = (j * W + i) * 2; gv[o] = i; gv[o + 1] = j; }
    this.gridVB = d.createBuffer({ size: gv.byteLength, usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST });
    d.queue.writeBuffer(this.gridVB, 0, gv);
    const gi = new Uint32Array((W - 1) * (W - 1) * 6);
    let k = 0;
    for (let j = 0; j < W - 1; j++) for (let i = 0; i < W - 1; i++) {
      const a = j * W + i, b = a + W;
      gi[k++] = a; gi[k++] = b; gi[k++] = a + 1;
      gi[k++] = a + 1; gi[k++] = b; gi[k++] = b + 1;
    }
    this.gridIB = d.createBuffer({ size: gi.byteLength, usage: GPUBufferUsage.INDEX | GPUBufferUsage.COPY_DST });
    d.queue.writeBuffer(this.gridIB, 0, gi);
    this.gridCount = gi.length;

    // ---- water plane (extends past the map so the horizon is ocean)
    const world = W * cellSize, ov = 2600;
    const wq = new Float32Array([-ov, -ov, world + ov, -ov, -ov, world + ov, -ov, world + ov, world + ov, -ov, world + ov, world + ov]);
    this.waterVB = d.createBuffer({ size: wq.byteLength, usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST });
    d.queue.writeBuffer(this.waterVB, 0, wq);

    // ---- height texture (r32float, read with textureLoad in the vertex stage)
    this.hTex = d.createTexture({
      size: [W, W], format: 'r32float',
      usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST
    });

    // ---- instance / particle buffers
    this.instBuf = this.shapeIds.map(() => d.createBuffer({ size: this.maxInst * this.instStride, usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST }));
    this.instData = this.shapeIds.map(() => new Float32Array(this.maxInst * this.instFloats));
    this.partBuf = d.createBuffer({ size: this.sim.partCapacity() * PART_STRIDE, usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST });

    // ---- bind group layouts
    const bglU = d.createBindGroupLayout({ entries: [
      { binding: 0, visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT, buffer: { type: 'uniform' } }] });
    const bglH = d.createBindGroupLayout({ entries: [
      { binding: 0, visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT, texture: { sampleType: 'unfilterable-float' } }] });
    const bglP = d.createBindGroupLayout({ entries: [
      { binding: 3, visibility: GPUShaderStage.FRAGMENT, sampler: { type: 'filtering' } },
      { binding: 1, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: 'float' } },
      { binding: 2, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: 'float' } }] });
    this.bglP = bglP;
    this.bgU = d.createBindGroup({ layout: bglU, entries: [{ binding: 0, resource: { buffer: this.uniBuf } }] });
    this.bgH = d.createBindGroup({ layout: bglH, entries: [{ binding: 0, resource: this.hTex.createView() }] });
    this.samp = d.createSampler({ magFilter: 'linear', minFilter: 'linear', addressModeU: 'clamp-to-edge', addressModeV: 'clamp-to-edge' });

    const mod = d.createShaderModule({ code: WGSL });
    this.mod = mod;
    const HDR = 'rgba16float';
    const dss = (write, cmp) => ({ format: 'depth24plus', depthWriteEnabled: write, depthCompare: cmp });
    const pl = (layouts) => d.createPipelineLayout({ bindGroupLayouts: layouts });

    // ---- main-pass pipelines (MSAA)
    this.pSky = d.createRenderPipeline({
      layout: pl([bglU]), vertex: { module: mod, entryPoint: 'fsTri' },
      fragment: { module: mod, entryPoint: 'skyFS', targets: [{ format: HDR }] },
      primitive: { topology: 'triangle-list' },
      depthStencil: dss(false, 'always'), multisample: { count: this.sampleCount }
    });
    this.pTerr = d.createRenderPipeline({
      layout: pl([bglU, bglH]),
      vertex: { module: mod, entryPoint: 'terrVS', buffers: [
        { arrayStride: 8, attributes: [{ shaderLocation: 0, offset: 0, format: 'float32x2' }] }] },
      fragment: { module: mod, entryPoint: 'terrFS', targets: [{ format: HDR }] },
      primitive: { topology: 'triangle-list', cullMode: 'none' },
      depthStencil: dss(true, 'less'), multisample: { count: this.sampleCount }
    });
    this.pSolid = d.createRenderPipeline({
      layout: pl([bglU]),
      vertex: { module: mod, entryPoint: 'solidVS', buffers: [
        { arrayStride: 24, attributes: [
          { shaderLocation: 0, offset: 0, format: 'float32x3' },
          { shaderLocation: 1, offset: 12, format: 'float32x3' }] },
        { arrayStride: this.instStride, stepMode: 'instance', attributes: [
          { shaderLocation: 2, offset: 0, format: 'float32x3' },
          { shaderLocation: 3, offset: 12, format: 'float32x3' },
          { shaderLocation: 4, offset: 24, format: 'float32x3' },
          { shaderLocation: 5, offset: 36, format: 'float32x3' },
          { shaderLocation: 6, offset: 48, format: 'float32x2' }] }] },
      fragment: { module: mod, entryPoint: 'solidFS', targets: [{ format: HDR }] },
      primitive: { topology: 'triangle-list', cullMode: 'back' },
      depthStencil: dss(true, 'less'), multisample: { count: this.sampleCount }
    });
    // Depth-tested so hills occlude it, but never written: a shadow is paint
    // on the ground, not an object. `zero / one-minus-src-alpha` multiplies the
    // scene down instead of laying grey over it, so dark ground stays dark.
    this.pShadow = d.createRenderPipeline({
      layout: pl([bglU]),
      vertex: { module: mod, entryPoint: 'shadowVS', buffers: [
        { arrayStride: 24, attributes: [
          { shaderLocation: 0, offset: 0, format: 'float32x3' },
          { shaderLocation: 1, offset: 12, format: 'float32x3' }] },
        { arrayStride: this.instStride, stepMode: 'instance', attributes: [
          { shaderLocation: 2, offset: 0, format: 'float32x3' },
          { shaderLocation: 3, offset: 12, format: 'float32x3' },
          { shaderLocation: 4, offset: 24, format: 'float32x3' },
          { shaderLocation: 5, offset: 36, format: 'float32x3' },
          { shaderLocation: 6, offset: 48, format: 'float32x2' }] }] },
      fragment: { module: mod, entryPoint: 'shadowFS', targets: [{ format: HDR, blend: {
        color: { srcFactor: 'zero', dstFactor: 'one-minus-src-alpha' },
        alpha: { srcFactor: 'zero', dstFactor: 'one' } } }] },
      primitive: { topology: 'triangle-list', cullMode: 'none' },
      depthStencil: dss(false, 'less-equal'), multisample: { count: this.sampleCount }
    });
    this.pWater = d.createRenderPipeline({
      layout: pl([bglU, bglH]),
      vertex: { module: mod, entryPoint: 'waterVS', buffers: [
        { arrayStride: 8, attributes: [{ shaderLocation: 0, offset: 0, format: 'float32x2' }] }] },
      fragment: { module: mod, entryPoint: 'waterFS', targets: [{ format: HDR, blend: {
        color: { srcFactor: 'src-alpha', dstFactor: 'one-minus-src-alpha' },
        alpha: { srcFactor: 'one', dstFactor: 'one-minus-src-alpha' } } }] },
      primitive: { topology: 'triangle-list', cullMode: 'none' },
      depthStencil: dss(false, 'less'), multisample: { count: this.sampleCount }
    });
    this.pPart = d.createRenderPipeline({
      layout: pl([bglU]),
      vertex: { module: mod, entryPoint: 'partVS', buffers: [
        { arrayStride: PART_STRIDE, stepMode: 'instance', attributes: [
          { shaderLocation: 0, offset: 0, format: 'float32x3' },
          { shaderLocation: 1, offset: 12, format: 'float32' },
          { shaderLocation: 2, offset: 16, format: 'float32x3' },
          { shaderLocation: 3, offset: 28, format: 'float32' }] }] },
      fragment: { module: mod, entryPoint: 'partFS', targets: [{ format: HDR, blend: {
        color: { srcFactor: 'one', dstFactor: 'one' },
        alpha: { srcFactor: 'one', dstFactor: 'one' } } }] },
      primitive: { topology: 'triangle-list' },
      depthStencil: dss(false, 'less'), multisample: { count: this.sampleCount }
    });

    // ---- post pipelines
    const post = (entry, fmt) => d.createRenderPipeline({
      layout: pl([bglU, bglP]), vertex: { module: mod, entryPoint: 'fsTri' },
      fragment: { module: mod, entryPoint: entry, targets: [{ format: fmt }] },
      primitive: { topology: 'triangle-list' }
    });
    this.pBright = post('brightFS', HDR);
    this.pBlurH = post('blurHFS', HDR);
    this.pBlurV = post('blurVFS', HDR);
    this.pComp = post('compFS', this.fmt);

    this.resize();
    this.ok = true;
    return this;
  }

  resize() {
    const d = this.device, dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = Math.max(2, Math.floor(this.canvas.clientWidth * dpr));
    const h = Math.max(2, Math.floor(this.canvas.clientHeight * dpr));
    this.dpr = dpr;
    if (this.w === w && this.h === h) return;
    this.cw = w; this.ch = h; this.w = w; this.h = h;
    this.canvas.width = w; this.canvas.height = h;
    for (const t of [this.hdrTex, this.depthTex, this.bloomA, this.bloomB]) t && t.destroy();
    const HDR = 'rgba16float';
    this.hdrTex = d.createTexture({ size: [w, h], format: HDR, usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING });
    this.depthTex = d.createTexture({ size: [w, h], format: 'depth24plus', usage: GPUTextureUsage.RENDER_ATTACHMENT });
    const bw = Math.max(1, w >> 2), bh = Math.max(1, h >> 2);
    this.bloomA = d.createTexture({ size: [bw, bh], format: HDR, usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING });
    this.bloomB = d.createTexture({ size: [bw, bh], format: HDR, usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING });
    const mkBG = (a, b) => d.createBindGroup({ layout: this.bglP, entries: [
      { binding: 3, resource: this.samp },
      { binding: 1, resource: a.createView() },
      { binding: 2, resource: b.createView() }] });
    this.bgBright = mkBG(this.hdrTex, this.hdrTex);
    this.bgBlurH = mkBG(this.bloomA, this.bloomA);
    this.bgBlurV = mkBG(this.bloomB, this.bloomB);
    this.bgComp = mkBG(this.hdrTex, this.bloomA);
  }

  uploadHeights(f32, lo, hi) {
    if (hi < lo) return;
    const W = this.TW;
    const rows = hi - lo + 1;
    this.device.queue.writeTexture(
      { texture: this.hTex, origin: { x: 0, y: lo, z: 0 } },
      f32, { offset: lo * W * 4, bytesPerRow: W * 4, rowsPerImage: rows },
      { width: W, height: rows, depthOrArrayLayers: 1 });
  }

  // Instances arrive as one flat wasm array; split by shape id (field 11).
  // [skipLo, skipHi) is dropped entirely — the player's own carpet, in first
  // person, where it would otherwise sit across the bottom of the screen.
  partition(src, n, skipLo, skipHi) {
    const c = this.shapeIds.map(() => 0), dst = this.instData, FLOATS = this.instFloats;
    for (let i = 0; i < n; i++) {
      if (i >= skipLo && i < skipHi) continue;
      const o = i * FLOATS;
      let s = src[o + FLOATS - 1] | 0; if (s < 0 || s >= dst.length) s = 0;
      const k = c[s]; if (k >= this.maxInst) continue;
      const t = dst[s], p = k * FLOATS;
      for (let f = 0; f < FLOATS; f++) t[p + f] = src[o + f];
      c[s]++;
    }
    for (let s = 0; s < c.length; s++) if (c[s]) this.device.queue.writeBuffer(this.instBuf[s], 0, this.instData[s], 0, c[s] * FLOATS);
    return c;
  }

  frame(cam, parts, partN, instSrc, instN, opts) {
    if (!this.ok || this.lost) return;
    const d = this.device;
    const aspect = this.cw / this.ch;
    this.sim.camera(opts.fov, aspect, 1.2, this.far,
      cam.ex, cam.ey, cam.ez, cam.cx, cam.cy, cam.cz, cam.ux, cam.uy, cam.uz);
    const u = this.uni;
    u.set(this.camM, 0);
    u[32] = cam.ex; u[33] = cam.ey; u[34] = cam.ez; u[35] = opts.time;
    u[36] = opts.sun[0]; u[37] = opts.sun[1]; u[38] = opts.sun[2]; u[39] = opts.sunI;
    u[40] = opts.fog[0]; u[41] = opts.fog[1]; u[42] = opts.fog[2]; u[43] = opts.fogD;
    u[44] = this.TW; u[45] = this.cell; u[46] = this.sea; u[47] = aspect;
    u[48] = opts.exposure; u[49] = this.bloom ? opts.bloomStrength : 0; u[50] = opts.waterAlpha; u[51] = opts.hurt;
    for (let c = 0; c < 3; c++) {
      const s4 = 52 + c * 4, src = opts.castles[c];
      if (src) { u[s4] = src[0]; u[s4 + 1] = src[1]; u[s4 + 2] = src[2]; u[s4 + 3] = src[3]; }
      else { u[s4] = 0; u[s4 + 1] = 0; u[s4 + 2] = 0; u[s4 + 3] = 0; }
    }
    u[64] = this.levels; u[65] = this.grain;
    u[66] = this.block * this.dpr; u[67] = this.far;
    d.queue.writeBuffer(this.uniBuf, 0, u.buffer, 0, 272);

    const counts = this.partition(instSrc, instN, opts.skipLo | 0, opts.skipHi | 0);
    if (partN > 0) d.queue.writeBuffer(this.partBuf, 0, parts, 0, partN * 8);

    const enc = d.createCommandEncoder();
    const pass = enc.beginRenderPass({
      colorAttachments: [{ view: this.hdrTex.createView(),
        clearValue: { r: 0, g: 0, b: 0, a: 1 }, loadOp: 'clear', storeOp: 'store' }],
      depthStencilAttachment: { view: this.depthTex.createView(), depthClearValue: 1, depthLoadOp: 'clear', depthStoreOp: 'store' }
    });
    pass.setBindGroup(0, this.bgU);
    pass.setPipeline(this.pSky); pass.draw(3);
    pass.setPipeline(this.pTerr); pass.setBindGroup(1, this.bgH);
    pass.setVertexBuffer(0, this.gridVB); pass.setIndexBuffer(this.gridIB, 'uint32');
    pass.drawIndexed(this.gridCount);
    pass.setPipeline(this.pSolid);
    for (let s = 0; s < counts.length; s++) {
      if (s === SHADOW_SHAPE || !counts[s]) continue;
      const sh = this.shapes[s];
      pass.setVertexBuffer(0, sh.vb); pass.setVertexBuffer(1, this.instBuf[s]);
      pass.setIndexBuffer(sh.ib, 'uint16');
      pass.drawIndexed(sh.count, counts[s]);
    }
    // after the solids, so a caster never darkens itself
    if (counts[SHADOW_SHAPE]) {
      const sh = this.shapes[SHADOW_SHAPE];
      pass.setPipeline(this.pShadow);
      pass.setVertexBuffer(0, sh.vb); pass.setVertexBuffer(1, this.instBuf[SHADOW_SHAPE]);
      pass.setIndexBuffer(sh.ib, 'uint16');
      pass.drawIndexed(sh.count, counts[SHADOW_SHAPE]);
    }
    pass.setPipeline(this.pWater); pass.setBindGroup(1, this.bgH);
    pass.setVertexBuffer(0, this.waterVB); pass.draw(6);
    if (partN > 0) {
      pass.setPipeline(this.pPart); pass.setVertexBuffer(0, this.partBuf); pass.draw(6, partN);
    }
    pass.end();

    const fs = (view, bg, pipe) => {
      const p = enc.beginRenderPass({ colorAttachments: [{ view, clearValue: { r: 0, g: 0, b: 0, a: 1 }, loadOp: 'clear', storeOp: 'store' }] });
      p.setPipeline(pipe); p.setBindGroup(0, this.bgU); p.setBindGroup(1, bg); p.draw(3); p.end();
    };
    if (this.bloom) {
      fs(this.bloomA.createView(), this.bgBright, this.pBright);
      fs(this.bloomB.createView(), this.bgBlurH, this.pBlurH);
      fs(this.bloomA.createView(), this.bgBlurV, this.pBlurV);
    }
    fs(this.ctx.getCurrentTexture().createView(), this.bgComp, this.pComp);
    d.queue.submit([enc.finish()]);
  }
}
