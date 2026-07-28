import { generateKeyPairSync, randomBytes } from "node:crypto";
import { writeFile } from "node:fs/promises";

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

function keyId(value, label) {
  if (!/^[A-Za-z0-9][A-Za-z0-9_.-]{1,127}$/u.test(value)) {
    fail(`${label} is invalid`);
  }
  return value;
}

const output = option("--output");
const joinKid = keyId(option("--join-kid"), "join key id");
const playerKid = keyId(option("--player-kid"), "player key id");
const serviceKid = keyId(option("--service-kid"), "service key id");
const resultKid = keyId(option("--result-kid"), "result key id");

const { privateKey, publicKey } = generateKeyPairSync("ed25519");
const privateDer = privateKey.export({ type: "pkcs8", format: "der" });
const publicDer = publicKey.export({ type: "spki", format: "der" });
const publicPrefix = Buffer.from("302a300506032b6570032100", "hex");
if (
  publicDer.byteLength !== publicPrefix.byteLength + 32 ||
  !publicDer.subarray(0, publicPrefix.byteLength).equals(publicPrefix)
) {
  fail("Node returned an unexpected Ed25519 public-key encoding");
}
const rawPublicKey = publicDer.subarray(publicPrefix.byteLength);

const encodedRandomKey = () => randomBytes(32).toString("base64url");
const secretBindings = {
  PLAYER_TICKET_KEYS_JSON: JSON.stringify({ [playerKid]: encodedRandomKey() }),
  JOIN_TICKET_SIGNING_KEYS_JSON: JSON.stringify({
    [joinKid]: privateDer.toString("base64url"),
  }),
  SERVICE_TICKET_KEYS_JSON: JSON.stringify({ [serviceKid]: encodedRandomKey() }),
  RESULT_TICKET_KEYS_JSON: JSON.stringify({ [resultKid]: encodedRandomKey() }),
};

await writeFile(output, `${JSON.stringify(secretBindings, null, 2)}\n`, {
  flag: "wx",
  mode: 0o600,
});

const publicConfiguration = {
  JOIN_TICKET_PUBLIC_KEYS_JSON: JSON.stringify({
    [joinKid]: rawPublicKey.toString("base64url"),
  }),
  ACTIVE_JOIN_TICKET_KID: joinKid,
  ACTIVE_SERVICE_TICKET_KID: serviceKid,
  resultTicketKid: resultKid,
};
console.log(JSON.stringify(publicConfiguration, null, 2));
console.error(`private Worker secret bundle written with mode 0600 to ${output}`);
