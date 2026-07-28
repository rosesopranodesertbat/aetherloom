import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const cargoBin = path.join(os.homedir(), '.cargo', 'bin');
const env = {
  ...process.env,
  PATH: `${cargoBin}${path.delimiter}${process.env.PATH ?? ''}`,
  RUSTFLAGS: [process.env.RUSTFLAGS, '-Dwarnings'].filter(Boolean).join(' '),
};
const result = spawnSync('cargo', ['test', '--workspace', '--all-targets'], {
  cwd: process.cwd(),
  env,
  stdio: 'inherit',
});

if (result.error) {
  console.error(`could not run cargo test: ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status ?? 1);
