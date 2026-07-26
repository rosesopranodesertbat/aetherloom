// Compiles the simulation to WebAssembly and produces the single-file build.
//   src/sim.ts  --asc-->  site/sim.wasm
//   site/*      -------->  standalone.html   (modules inlined, wasm as base64)
import fs from 'fs';
import { execSync } from 'child_process';

console.log('compiling src/sim.ts -> site/sim.wasm');
execSync([
  'npx asc src/sim.ts --outFile site/sim.wasm',
  '-O3 --shrinkLevel 0 --noAssert --runtime stub',
  '--use abort= --use Math=NativeMath --use Mathf=NativeMathf',
  '--initialMemory 64 --maximumMemory 256'
].join(' '), { stdio: 'inherit' });

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
