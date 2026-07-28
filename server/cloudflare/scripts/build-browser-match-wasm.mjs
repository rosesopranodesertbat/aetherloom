import { execFile } from "node:child_process";
import { existsSync } from "node:fs";
import { chmod, copyFile, mkdir } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const exec = promisify(execFile);
const cloudflareRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(cloudflareRoot, "..", "..");
const homeCargo = join(homedir(), ".cargo", "bin", "cargo");
const cargo = process.env.CARGO || (existsSync(homeCargo) ? homeCargo : "cargo");
const source = join(
  repositoryRoot,
  "target",
  "wasm32-unknown-unknown",
  "release",
  "aetherloom_worker_match.wasm",
);
const output = join(
  cloudflareRoot,
  "generated",
  "aetherloom-worker-match.wasm",
);

await exec(
  cargo,
  [
    "build",
    "--release",
    "--target",
    "wasm32-unknown-unknown",
    "-p",
    "aetherloom-worker-match",
  ],
  { cwd: repositoryRoot },
);
await mkdir(dirname(output), { recursive: true });
await copyFile(source, output);
await chmod(output, 0o644);
console.log(`built ${output}`);
