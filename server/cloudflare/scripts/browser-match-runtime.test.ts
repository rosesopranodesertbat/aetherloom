import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { BrowserMatchSimulation } from "../src/browser-match-runtime.ts";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const bytes = await readFile(join(root, "generated", "aetherloom-worker-match.wasm"));
const module = new WebAssembly.Module(bytes);

test("browser-match Wasm advances the real fixed-spawn authoritative core", () => {
  const match = new BrowserMatchSimulation(module, 7, 0);
  match.addHuman(0, 0);
  match.addHuman(1, 1);
  const initial = match.snapshot();
  assert.deepEqual(
    initial.players.map(({ playerId, xCm, zCm, health }) => ({
      playerId,
      xCm,
      zCm,
      health,
    })),
    [
      { playerId: 0, xCm: -250, zCm: 0, health: 100 },
      { playerId: 1, xCm: 250, zCm: 0, health: 100 },
    ],
  );

  match.submitInput(0, 2_047, 0, 0, false);
  const first = match.advanceTick();
  const second = match.advanceTick();
  const stale = match.advanceTick();
  assert.equal(first.tick, 1);
  assert.equal(first.players[0]?.xCm, -242);
  assert.equal(second.players[0]?.xCm, -234);
  assert.equal(
    stale.players[0]?.xCm,
    -234,
    "an open socket without a fresh frame must auto-neutralize after two ticks",
  );
});

test("browser-match Wasm exposes authoritative projectile damage events", () => {
  const match = new BrowserMatchSimulation(module, 11, 0);
  match.addHuman(0, 0);
  match.addHuman(1, 1);
  match.submitInput(0, 0, 0, 0, true);

  let damaged = false;
  for (let tick = 0; tick < 32; tick += 1) {
    const snapshot = match.advanceTick();
    if (snapshot.events.some((event) => event.kind === 4)) {
      damaged = true;
      assert.ok((snapshot.players[1]?.health ?? 100) < 100);
      break;
    }
    match.submitInput(0, 0, 0, 0, false);
  }
  assert.equal(damaged, true);
});
