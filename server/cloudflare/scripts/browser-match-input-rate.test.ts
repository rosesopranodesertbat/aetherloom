import assert from "node:assert/strict";
import test from "node:test";

import {
  BROWSER_MATCH_INPUT_REJECTION_LIMIT,
  BROWSER_MATCH_INPUT_TOKEN_CAPACITY,
  consumeBrowserInputToken,
  isBrowserInputComponent,
  type InputRateBudget,
} from "../src/browser-match-types.ts";

function full(at = 0): InputRateBudget {
  return {
    tokens: BROWSER_MATCH_INPUT_TOKEN_CAPACITY,
    updatedAtMs: at,
    rejectedMessages: 0,
  };
}

test("the compact input contract reserves signed byte minus 128", () => {
  assert.equal(isBrowserInputComponent(-128), false);
  assert.equal(isBrowserInputComponent(-127), true);
  assert.equal(isBrowserInputComponent(127), true);
  assert.equal(isBrowserInputComponent(128), false);
});

test("a paced 64 Hz input stream retains headroom indefinitely", () => {
  let budget = full();
  for (let index = 1; index <= 64 * 60; index += 1) {
    const decision = consumeBrowserInputToken(budget, index * (1_000 / 64));
    assert.equal(decision.accepted, true);
    assert.equal(decision.shouldClose, false);
    budget = decision;
  }
  assert.ok(budget.tokens > BROWSER_MATCH_INPUT_TOKEN_CAPACITY / 2);
});

test("delivery batching can spend the burst budget without a policy close", () => {
  let budget = full(1_000);
  for (let index = 0; index < BROWSER_MATCH_INPUT_TOKEN_CAPACITY + 80; index += 1) {
    const decision = consumeBrowserInputToken(budget, 1_000);
    assert.equal(decision.shouldClose, false);
    budget = decision;
  }
  assert.equal(budget.rejectedMessages, 80);

  const recovered = consumeBrowserInputToken(budget, 3_000);
  assert.equal(recovered.accepted, true);
  assert.equal(recovered.rejectedMessages, 0);
});

test("a sustained flood eventually closes after bounded coalescing", () => {
  let budget = full();
  let closed = false;
  for (
    let index = 0;
    index < BROWSER_MATCH_INPUT_TOKEN_CAPACITY + BROWSER_MATCH_INPUT_REJECTION_LIMIT;
    index += 1
  ) {
    const decision = consumeBrowserInputToken(budget, 0);
    budget = decision;
    if (decision.shouldClose) {
      closed = true;
      break;
    }
  }
  assert.equal(closed, true);
  assert.equal(budget.rejectedMessages, BROWSER_MATCH_INPUT_REJECTION_LIMIT);
});

test("a continuously paced overload cannot cancel its own rejection strikes", () => {
  let budget = full();
  let closedAtMs: number | null = null;
  for (let index = 1; index <= 256 * 10; index += 1) {
    const nowMs = index * (1_000 / 256);
    const decision = consumeBrowserInputToken(budget, nowMs);
    budget = decision;
    if (decision.shouldClose) {
      closedAtMs = nowMs;
      break;
    }
  }
  assert.notEqual(closedAtMs, null);
  assert.ok((closedAtMs ?? Number.POSITIVE_INFINITY) < 10_000);
});
