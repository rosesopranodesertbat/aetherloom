import assert from "node:assert/strict";
import test from "node:test";
import {
  MAX_RETAINED_ISLAND_BACKLOG_TICKS,
  commitAdvancedSimulation,
  planIslandTickBatch,
} from "../src/island-runtime.ts";

test("excess due ticks remain as backlog and drain without elapsed time", () => {
  const first = planIslandTickBatch(
    {
      lastWallClockMs: 1_000,
      tickRemainder: 0,
      backlogTicks: 0,
      activeUntilMs: 40_000,
    },
    1_500,
  );
  assert.deepEqual(first, {
    kind: "advance",
    ticks: 32,
    backlogTicks: 32,
    tickRemainder: 0,
    wasDormant: false,
  });

  const second = planIslandTickBatch(
    {
      lastWallClockMs: 1_500,
      tickRemainder: first.kind === "advance" ? first.tickRemainder : 0,
      backlogTicks: first.kind === "advance" ? first.backlogTicks : 0,
      activeUntilMs: 40_000,
    },
    1_500,
  );
  assert.deepEqual(second, {
    kind: "advance",
    ticks: 32,
    backlogTicks: 0,
    tickRemainder: 0,
    wasDormant: false,
  });
});

test("sub-tick time is retained exactly at 128 Hz", () => {
  const waiting = planIslandTickBatch(
    {
      lastWallClockMs: 100,
      tickRemainder: 0,
      backlogTicks: 0,
      activeUntilMs: 1_000,
    },
    101,
  );
  assert.deepEqual(waiting, { kind: "not_due", retryAfterMs: 7 });

  const due = planIslandTickBatch(
    {
      lastWallClockMs: 100,
      tickRemainder: 0,
      backlogTicks: 0,
      activeUntilMs: 1_000,
    },
    108,
  );
  assert.deepEqual(due, {
    kind: "advance",
    ticks: 1,
    backlogTicks: 0,
    tickRemainder: 24,
    wasDormant: false,
  });
});

test("dormancy does not erase already durable backlog", () => {
  const plan = planIslandTickBatch(
    {
      lastWallClockMs: 1_000,
      tickRemainder: 777,
      backlogTicks: 40,
      activeUntilMs: 1_100,
    },
    2_000,
  );
  assert.deepEqual(plan, {
    kind: "advance",
    ticks: 32,
    backlogTicks: 9,
    tickRemainder: 0,
    wasDormant: true,
  });
});

test("backlog overflow fails before consuming any ticks", () => {
  assert.throws(
    () =>
      planIslandTickBatch(
        {
          lastWallClockMs: 0,
          tickRemainder: 0,
          backlogTicks: MAX_RETAINED_ISLAND_BACKLOG_TICKS,
          activeUntilMs: 1_000,
        },
        1_000,
      ),
    (error: unknown) =>
      typeof error === "object" &&
      error !== null &&
      "code" in error &&
      error.code === "island_tick_backlog_overflow",
  );
});

test("failed durability invalidates memory before the error escapes", () => {
  let valid = true;
  assert.throws(
    () =>
      commitAdvancedSimulation(
        () => {
          throw new Error("simulated SQLite failure");
        },
        () => {
          valid = false;
        },
      ),
    /simulated SQLite failure/u,
  );
  assert.equal(valid, false);
});

test("successful durability keeps the loaded simulation reusable", () => {
  let invalidations = 0;
  const result = commitAdvancedSimulation(
    () => "committed",
    () => {
      invalidations += 1;
    },
  );
  assert.equal(result, "committed");
  assert.equal(invalidations, 0);
});
