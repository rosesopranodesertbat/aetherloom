import assert from "node:assert/strict";
import test from "node:test";

import {
  activateBeforeConfirm,
  checkpointObjectKey,
  commitSequentially,
  compensateAll,
  dispatchIdentityValue,
  dispatchRecoveryAction,
  materializeRoster,
  rosterIdentityValue,
} from "../src/dispatch-invariants.ts";
import {
  authorizedResultSigner,
  resultEvidenceObjectKey,
  resultTicketHosts,
} from "../src/ticket-invariants.ts";

test("checkpoint objects are content-addressed within an account namespace", () => {
  assert.equal(
    checkpointObjectKey("account-a", "sha-a"),
    checkpointObjectKey("account-a", "sha-a"),
  );
  assert.notEqual(
    checkpointObjectKey("account-a", "sha-a"),
    checkpointObjectKey("account-a", "sha-b"),
  );
  assert.notEqual(
    checkpointObjectKey("account-a", "sha-a"),
    checkpointObjectKey("account-b", "sha-a"),
  );
});

test("dispatch identity binds every immutable request field", () => {
  const dispatch = {
    shard: "weur:solo-extraction:controller:mmr-5:shard-1",
    matchId: "match-1",
    region: "weur",
    playlist: "solo-extraction",
    inputPool: "controller",
    buildHash: "build-1",
    targetPlayers: 2,
    allowBots: false,
  };
  assert.equal(dispatchIdentityValue(dispatch), dispatchIdentityValue({ ...dispatch }));
  const mutations = {
    shard: "weur:solo-extraction:controller:mmr-5:shard-2",
    matchId: "match-2",
    region: "eeur",
    playlist: "squad-extraction",
    inputPool: "mouse-keyboard",
    buildHash: "build-2",
    targetPlayers: 3,
    allowBots: true,
  } as const;
  for (const [field, value] of Object.entries(mutations)) {
    assert.notEqual(
      dispatchIdentityValue(dispatch),
      dispatchIdentityValue({ ...dispatch, [field]: value }),
      `${field} must be bound to the match id`,
    );
  }
});

test("roster materialization validates counts, duplicates and exact reservation maps", () => {
  const first = materializeRoster(
    [
      {
        accountIds: ["a", "b"],
        reservationIds: { a: "ra", b: "rb" },
      },
    ],
    2,
    "squad-extraction",
  );
  const reorderedMap = materializeRoster(
    [
      {
        accountIds: ["a", "b"],
        reservationIds: { b: "rb", a: "ra" },
      },
    ],
    2,
    "squad-extraction",
  );
  assert.equal(rosterIdentityValue(first), rosterIdentityValue(reorderedMap));
  for (const changed of [
    [{ ...first[0], accountId: "c" }, first[1]],
    [{ ...first[0], reservationId: "other" }, first[1]],
    [{ ...first[0], playerSlot: 1 }, first[1]],
    [{ ...first[0], teamId: 1 }, first[1]],
  ]) {
    assert.notEqual(rosterIdentityValue(first), rosterIdentityValue(changed));
  }
  assert.throws(
    () =>
      materializeRoster(
        [
          { accountIds: ["a"], reservationIds: { a: "ra" } },
          { accountIds: ["a"], reservationIds: { a: "rb" } },
        ],
        2,
        "squad-extraction",
      ),
    /duplicate/u,
  );
  assert.throws(
    () => materializeRoster([{ accountIds: ["a"], reservationIds: { a: "ra", b: "rb" } }], 1, "solo-extraction"),
    /exactly one reservation/u,
  );
  assert.throws(
    () => materializeRoster([{ accountIds: ["a"], reservationIds: { a: "ra" } }], 2, "solo-extraction"),
    /player count/u,
  );
  assert.throws(
    () =>
      materializeRoster(
        Array.from({ length: 33 }, (_, index) => ({
          accountIds: [`a${index}x`, `b${index}x`, `c${index}x`, `d${index}x`],
          reservationIds: {
            [`a${index}x`]: `ra${index}`,
            [`b${index}x`]: `rb${index}`,
            [`c${index}x`]: `rc${index}`,
            [`d${index}x`]: `rd${index}`,
          },
        })),
        132,
        "squad-extraction",
      ),
    /player count/u,
  );
});

test("commit sequencing records every attempted remote mutation", async () => {
  const started: number[] = [];
  const attempted: number[] = [];
  let inFlight = 0;
  let maximumInFlight = 0;
  await assert.rejects(
    commitSequentially([1, 2, 3], attempted, async (value) => {
      started.push(value);
      inFlight += 1;
      maximumInFlight = Math.max(maximumInFlight, inFlight);
      await Promise.resolve();
      inFlight -= 1;
      if (value === 2) throw new Error("commit acknowledgement lost");
    }),
    /acknowledgement lost/u,
  );
  assert.deepEqual(started, [1, 2]);
  assert.deepEqual(attempted, [1, 2]);
  assert.equal(maximumInFlight, 1);
});

test("compensation observes sync throws and waits for every rollback", async () => {
  let releaseDeferred!: () => void;
  const deferred = new Promise<void>((resolve) => {
    releaseDeferred = resolve;
  });
  const events: string[] = [];
  const pending = compensateAll([1, 2, 3], async (value) => {
    events.push(`start-${value}`);
    if (value === 1) throw new Error("rollback 1");
    if (value === 2) await deferred;
    events.push(`end-${value}`);
  });
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.deepEqual(events.sort(), ["end-3", "start-1", "start-2", "start-3"]);
  let settled = false;
  void pending.then(() => {
    settled = true;
  });
  await Promise.resolve();
  assert.equal(settled, false);
  releaseDeferred();
  const failures = await pending;
  assert.deepEqual(events.sort(), ["end-2", "end-3", "start-1", "start-2", "start-3"]);
  assert.equal(failures.length, 1);

  const syncFailures = await compensateAll([1], () => {
    throw new Error("sync rollback");
  });
  assert.equal(syncFailures.length, 1);
});

test("allocation activation completes before confirmation starts", async () => {
  let releaseActivation!: () => void;
  const activation = new Promise<void>((resolve) => {
    releaseActivation = resolve;
  });
  const events: string[] = [];
  const pending = activateBeforeConfirm(
    async () => {
      events.push("activation-start");
      await activation;
      events.push("activation-end");
    },
    async () => {
      events.push("confirm");
    },
  );
  await Promise.resolve();
  assert.deepEqual(events, ["activation-start"]);
  releaseActivation();
  await pending;
  assert.deepEqual(events, ["activation-start", "activation-end", "confirm"]);

  let confirmCalled = false;
  await assert.rejects(
    activateBeforeConfirm(
      async () => {
        throw new Error("activation failed");
      },
      async () => {
        confirmCalled = true;
      },
    ),
    /activation failed/u,
  );
  assert.equal(confirmCalled, false);
  await assert.rejects(
    activateBeforeConfirm(
      async () => undefined,
      async () => {
        throw new Error("confirmation failed");
      },
    ),
    /confirmation failed/u,
  );
});

test("dispatch recovery aborts expired journals but preserves confirmed receipts", () => {
  const now = 1_000_000;
  assert.equal(
    dispatchRecoveryAction("staged", now + 60_000, [now + 30_000], now),
    "resume",
  );
  assert.equal(
    dispatchRecoveryAction("active-pending", now + 60_000, [now + 4_999], now),
    "abort",
  );
  assert.equal(
    dispatchRecoveryAction("staged", now + 4_999, [now + 60_000], now),
    "abort",
  );
  assert.equal(dispatchRecoveryAction("aborting", now + 60_000, [now + 60_000], now), "abort");
  assert.equal(dispatchRecoveryAction("active-confirmed", now - 1, [], now), "done");
});

test("result signer keys bind one verified subject to its allocation host", () => {
  const hosts = '{"result-host-a":"host-a","result-host-b":"host-b"}';
  assert.deepEqual(resultTicketHosts(hosts), {
    "result-host-a": "host-a",
    "result-host-b": "host-b",
  });
  assert.equal(authorizedResultSigner(hosts, "result-host-a", "host-a"), "host-a");
  assert.equal(authorizedResultSigner(hosts, "result-host-a", "host-b"), null);
  assert.equal(authorizedResultSigner(hosts, "service-v1", "host-a"), null);
  assert.throws(() => resultTicketHosts("{}"), /between one and eight/u);
  assert.throws(
    () => resultTicketHosts('{"result-host-a":"../../host"}'),
    /invalid key id or host id/u,
  );
  assert.equal(
    resultEvidenceObjectKey("match-a", "settlement-a", "sha-a"),
    "match.result.match-a.settlement-a.sha-a.json",
  );
});
