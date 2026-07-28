import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { signJoinTicket, verifyJoinTicket } from "../src/join-tickets.ts";
import type { JoinTicketClaims } from "../src/types.ts";

interface Fixture {
  privateKeyPkcs8Base64Url: string;
  publicKeyRawBase64Url: string;
  claims: JoinTicketClaims;
  token: string;
}

const fixture = JSON.parse(
  await readFile(new URL("../fixtures/join-ticket-v1.json", import.meta.url), "utf8"),
) as Fixture;
const privateKeys = JSON.stringify({
  "test-ed25519-v1": fixture.privateKeyPkcs8Base64Url,
});
const publicKeys = JSON.stringify({
  "test-ed25519-v1": fixture.publicKeyRawBase64Url,
});
const expectations = {
  issuer: "aetherloom-control-plane",
  audience: "aetherloom-match",
  nowSeconds: 2_000_000_001,
  matchId: "11111111111111111111111111111111",
  matchEpoch: 42,
  buildHash: "22222222222222222222222222222222",
  inputPool: "controller" as const,
};

test("Cloudflare Ed25519 signer reproduces the Rust contract vector", async () => {
  const signed = await signJoinTicket(
    fixture.claims,
    privateKeys,
    "test-ed25519-v1",
  );
  assert.equal(signed, fixture.token);
  assert.deepEqual(await verifyJoinTicket(signed, publicKeys, expectations), fixture.claims);
});

test("join verifier binds target match, epoch, build and input pool", async () => {
  for (const changed of [
    { ...expectations, matchId: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" },
    { ...expectations, matchEpoch: 43 },
    { ...expectations, buildHash: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" },
    { ...expectations, inputPool: "mouse-keyboard" as const },
  ]) {
    await assert.rejects(
      verifyJoinTicket(fixture.token, publicKeys, changed),
      /target match/u,
    );
  }
});

test("join signer and verifier reject extensions, noncanonical ids and tampering", async () => {
  await assert.rejects(
    signJoinTicket(
      { ...fixture.claims, admin: true } as JoinTicketClaims,
      privateKeys,
      "test-ed25519-v1",
    ),
    /exact v1 fields/u,
  );
  await assert.rejects(
    signJoinTicket(
      {
        ...fixture.claims,
        match_id: "1111111111111111111111111111111A",
      },
      privateKeys,
      "test-ed25519-v1",
    ),
    /lowercase hexadecimal/u,
  );
  const tampered = `${fixture.token.slice(0, -1)}${
    fixture.token.endsWith("A") ? "B" : "A"
  }`;
  await assert.rejects(
    verifyJoinTicket(tampered, publicKeys, expectations),
    /signature is invalid/u,
  );
});
