import { createHash } from "node:crypto";
import { lstat, mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { dirname, relative, resolve, sep } from "node:path";

function fail(message) {
  throw new Error(message);
}

function option(name) {
  const index = process.argv.indexOf(name);
  if (index === -1 || index + 1 >= process.argv.length) {
    fail(`missing ${name}`);
  }
  return process.argv[index + 1];
}

function validateCommit(commit) {
  if (!/^[0-9a-f]{40}$/u.test(commit)) {
    fail("commit must be a full lowercase 40-character Git SHA");
  }
  return commit;
}

async function collectFiles(root, current = root) {
  const entries = await readdir(current, { withFileTypes: true });
  const files = [];
  for (const entry of entries.sort((left, right) => left.name.localeCompare(right.name))) {
    const path = resolve(current, entry.name);
    if (entry.isSymbolicLink()) {
      fail(`release directory contains a symbolic link: ${relative(root, path)}`);
    }
    if (entry.isDirectory()) {
      files.push(...(await collectFiles(root, path)));
    } else if (entry.isFile()) {
      files.push(path);
    } else {
      fail(`release directory contains an unsupported entry: ${relative(root, path)}`);
    }
  }
  return files;
}

async function describeDirectory(directory) {
  const root = resolve(directory);
  const metadata = await lstat(root);
  if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
    fail(`${directory} must be a real directory`);
  }

  const result = {};
  for (const path of await collectFiles(root)) {
    const bytes = await readFile(path);
    const name = relative(root, path).split(sep).join("/");
    result[name] = {
      sha256: createHash("sha256").update(bytes).digest("hex"),
      size: bytes.byteLength,
    };
  }
  if (Object.keys(result).length === 0) {
    fail("release directory is empty");
  }
  return result;
}

function contentBuildId(commit, files) {
  const canonical = JSON.stringify({ version: 1, commit, files });
  const id = createHash("sha256").update(canonical).digest("hex").slice(0, 32);
  if (/^0{32}$/u.test(id)) {
    fail("derived content build id must not be zero");
  }
  return id;
}

async function createManifest() {
  const site = option("--site");
  const output = resolve(option("--output"));
  const commit = validateCommit(option("--commit"));
  const files = await describeDirectory(site);
  const manifest = {
    version: 1,
    commit,
    contentBuildId: contentBuildId(commit, files),
    files,
  };
  await mkdir(dirname(output), { recursive: true });
  await writeFile(output, `${JSON.stringify(manifest, null, 2)}\n`, { flag: "w" });
  console.log(
    `recorded ${Object.keys(manifest.files).length} release files for ${commit} (${manifest.contentBuildId})`,
  );
}

async function readManifest(manifestPath) {
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  if (
    manifest === null ||
    typeof manifest !== "object" ||
    manifest.version !== 1 ||
    !/^[0-9a-f]{40}$/u.test(manifest.commit) ||
    !/^[0-9a-f]{32}$/u.test(manifest.contentBuildId) ||
    /^0{32}$/u.test(manifest.contentBuildId) ||
    manifest.files === null ||
    typeof manifest.files !== "object" ||
    Array.isArray(manifest.files)
  ) {
    fail("release manifest metadata is invalid");
  }
  if (manifest.contentBuildId !== contentBuildId(manifest.commit, manifest.files)) {
    fail("release manifest content build id is invalid");
  }
  return manifest;
}

async function verifyManifest() {
  const site = option("--site");
  const manifestPath = option("--manifest");
  const expectedCommit = validateCommit(option("--commit"));
  const manifest = await readManifest(manifestPath);
  if (manifest.commit !== expectedCommit) {
    fail("release manifest does not match the requested commit");
  }

  const actual = await describeDirectory(site);
  const expectedNames = Object.keys(manifest.files).sort();
  const actualNames = Object.keys(actual).sort();
  if (JSON.stringify(expectedNames) !== JSON.stringify(actualNames)) {
    fail("release artifact file list does not match its manifest");
  }
  for (const name of expectedNames) {
    const expected = manifest.files[name];
    if (
      expected === null ||
      typeof expected !== "object" ||
      expected.sha256 !== actual[name].sha256 ||
      expected.size !== actual[name].size
    ) {
      fail(`release artifact hash mismatch: ${name}`);
    }
  }
  console.log(
    `verified ${actualNames.length} release files for ${expectedCommit} (${manifest.contentBuildId})`,
  );
}

async function printBuildId() {
  const manifest = await readManifest(option("--manifest"));
  process.stdout.write(manifest.contentBuildId);
}

const command = process.argv[2];
if (command === "create") {
  await createManifest();
} else if (command === "verify") {
  await verifyManifest();
} else if (command === "build-id") {
  await printBuildId();
} else {
  fail("usage: release-artifact.mjs <create|verify|build-id> [options]");
}
