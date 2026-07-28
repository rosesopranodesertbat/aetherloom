import assert from "node:assert/strict";
import test from "node:test";

import {
  advanceBrowserTickSchedule,
  shouldConsumeBrowserInput,
} from "../src/browser-match-types.ts";

test("browser input consumption stays on persistent 64 Hz tick parity", () => {
  let authoritativeTick = 0;
  const consumedAt: number[] = [];
  for (const batchSize of [5, 3, 5]) {
    for (let index = 0; index < batchSize; index += 1) {
      if (shouldConsumeBrowserInput(authoritativeTick)) {
        consumedAt.push(authoritativeTick);
      }
      authoritativeTick += 1;
    }
  }
  assert.deepEqual(consumedAt, [0, 2, 4, 6, 8, 10, 12]);
});

test("tick credit includes loop processing time between pulse starts", () => {
  const first = advanceBrowserTickSchedule(
    { tickCredit: 0, lastPulseAtMs: 1_000 },
    1_032,
  );
  assert.equal(first.ticks, 4);
  assert.ok(Math.abs(first.tickCredit - 0.096) < 1e-9);

  // The next pulse starts 38 ms later: a 32 ms timer plus 6 ms spent on the
  // prior simulation/snapshot pass. That processing time must remain credited.
  const second = advanceBrowserTickSchedule(first, 1_070);
  assert.equal(second.ticks, 4);
  assert.ok(Math.abs(second.tickCredit - 0.96) < 1e-9);
});

test("snapshot batches never bisect a two-tick browser input sample", () => {
  let schedule = { tickCredit: 0, lastPulseAtMs: 2_000 };
  for (const nowMs of [2_040, 2_071, 2_104, 2_141, 2_174, 2_209]) {
    const batch = advanceBrowserTickSchedule(schedule, nowMs);
    assert.equal(batch.ticks % 2, 0);
    schedule = batch;
  }
  const oddCandidate = advanceBrowserTickSchedule(
    { tickCredit: 0, lastPulseAtMs: 3_000 },
    3_040,
  );
  assert.equal(oddCandidate.ticks, 4);
  assert.ok(Math.abs(oddCandidate.tickCredit - 1.12) < 1e-9);
});
