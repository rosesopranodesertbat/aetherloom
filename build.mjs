// Compiles the simulation to WebAssembly and produces the single-file build.
//   src/lib.rs  --cargo-->  site/sim.wasm
//   site/*      --------->  standalone.html   (modules inlined, wasm as base64)
import fs from 'fs';
import { execSync } from 'child_process';

const TARGET = 'wasm32-unknown-unknown';
const OUT = `target/${TARGET}/release/sim.wasm`;
// rustup installs to ~/.cargo/bin, which is not always on a non-login PATH
const env = { ...process.env, PATH: `${process.env.HOME}/.cargo/bin:${process.env.PATH}` };

if (!process.argv.includes('--no-wasm')) {
  console.log(`compiling src/lib.rs -> site/sim.wasm  (${TARGET})`);
  try {
    execSync(`cargo build --release --target ${TARGET}`, { stdio: 'inherit', env });
  } catch {
    console.error(
      '\ncargo failed. This needs a Rust toolchain with the wasm target:\n' +
      "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh\n" +
      `  rustup target add ${TARGET}\n` +
      'Pass --no-wasm to rebuild standalone.html from the existing site/sim.wasm.');
    process.exit(1);
  }
  fs.copyFileSync(OUT, 'site/sim.wasm');
}

// The module has to stay freestanding: game.js instantiates it with an empty
// import object, so one stray import would break instantiation outright.
const mod = new WebAssembly.Module(fs.readFileSync('site/sim.wasm'));
const imports = WebAssembly.Module.imports(mod);
if (imports.length) {
  console.error('sim.wasm is not freestanding, it imports:', imports);
  process.exit(1);
}

const strip = (f) => fs.readFileSync(f, 'utf8')
  .replace(/^import[^\n]*\n/gm, '')
  .replace(/^export (const|class|function|async function) /gm, '$1 ');

const b64 = fs.readFileSync('site/sim.wasm').toString('base64');
const bundle = `
// ---- engine ----------------------------------------------------------------
${strip('site/engine.js')}
// ---- game ------------------------------------------------------------------
${strip('site/game.js')}
// ---- boot: wasm inlined as base64 ------------------------------------------
const B64 = "${b64}";
const bin = atob(B64);
const bytes = new Uint8Array(bin.length);
for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
start(bytes);
`;

const html = fs.readFileSync('site/index.html', 'utf8')
  .replace(/<!--BOOT-->[\s\S]*<!--\/BOOT-->/, '<script type="module">\n' + bundle + '\n</script>');
fs.writeFileSync('standalone.html', html);

const kb = (n) => (n / 1024).toFixed(1) + ' KB';
console.log('\nsite/');
for (const f of fs.readdirSync('site').sort()) console.log('  ' + f.padEnd(16) + kb(fs.statSync('site/' + f).size));
console.log('standalone.html  ' + kb(fs.statSync('standalone.html').size));
