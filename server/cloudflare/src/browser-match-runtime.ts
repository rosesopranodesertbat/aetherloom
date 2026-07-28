import { ApiError } from "./util.ts";
import { BROWSER_MATCH_SPELL_SLOTS } from "./browser-match-types.ts";
import type {
  CoreEventSnapshot,
  CoreMatchSnapshot,
  CorePlayerSnapshot,
  CoreProjectileSnapshot,
} from "./browser-match-types.ts";

interface WorkerMatchExports extends WebAssembly.Exports {
  memory: WebAssembly.Memory;
  worker_match_abi_version: () => number;
  worker_match_authoritative_hz: () => number;
  worker_match_max_players: () => number;
  worker_match_init: (seedLow: number, seedHigh: number) => number;
  worker_match_add_human: (player: number, team: number) => number;
  worker_match_add_bot: (player: number, team: number) => number;
  worker_match_remove_player: (player: number) => number;
  worker_match_set_human: (player: number) => number;
  worker_match_set_bot: (player: number) => number;
  worker_match_reset_player: (player: number) => number;
  worker_match_clear_input: (player: number) => number;
  worker_match_submit_input: (
    player: number,
    moveX: number,
    moveY: number,
    moveVertical: number,
    lookYaw: number,
    lookPitch: number,
    actionFlags: number,
    requestedSpell: number,
  ) => number;
  worker_match_advance_tick: () => number;
  worker_match_snapshot_ptr: () => number;
  worker_match_snapshot_len: () => number;
}

const SNAPSHOT_MAGIC = 0x314d4c41;
const EXPECTED_WORKER_ABI = 3;
const HEADER_WORDS = 16;
const PLAYER_WORDS = 15 + BROWSER_MATCH_SPELL_SLOTS;
const PROJECTILE_WORDS = 15;
const EVENT_WORDS = 13;

export function selectSnapshotProjectiles(
  projectiles: readonly CoreProjectileSnapshot[],
  viewer: Pick<CorePlayerSnapshot, "playerId" | "xCm" | "yCm" | "zCm">,
  previouslySelected: ReadonlySet<string>,
  maximum: number,
): CoreProjectileSnapshot[] {
  if (
    !Number.isInteger(viewer.playerId) ||
    viewer.playerId < 0 ||
    ![viewer.xCm, viewer.yCm, viewer.zCm].every(Number.isInteger) ||
    !(previouslySelected instanceof Set) ||
    !Number.isInteger(maximum) ||
    maximum < 0
  ) {
    throw new Error("Browser match projectile selection is invalid.");
  }
  const distanceSquared = (projectile: CoreProjectileSnapshot): number => {
    const dx = projectile.xCm - viewer.xCm;
    const dy = projectile.yCm - viewer.yCm;
    const dz = projectile.zCm - viewer.zCm;
    return dx * dx + dy * dy + dz * dz;
  };
  const threatBucket = (projectile: CoreProjectileSnapshot): number => {
    const dx = projectile.xCm - viewer.xCm;
    const dy = projectile.yCm - viewer.yCm;
    const dz = projectile.zCm - viewer.zCm;
    const closing =
      dx * projectile.velocityXCmPerTick +
        dy * projectile.velocityYCmPerTick +
        dz * projectile.velocityZCmPerTick <
      0;
    const distance = distanceSquared(projectile);
    return distance <= 2_000 ** 2 || closing && distance <= 5_000 ** 2
      ? 0
      : 1;
  };
  return [...projectiles]
    .sort((left, right) => {
      const ownership =
        Number(right.ownerPlayerId === viewer.playerId) -
        Number(left.ownerPlayerId === viewer.playerId);
      if (ownership !== 0) return ownership;
      const threat = threatBucket(left) - threatBucket(right);
      if (threat !== 0) return threat;
      const continuity =
        Number(previouslySelected.has(right.entityKey)) -
        Number(previouslySelected.has(left.entityKey));
      if (continuity !== 0) return continuity;
      const proximity = distanceSquared(left) - distanceSquared(right);
      if (proximity !== 0) return proximity;
      const recency = right.lifetimeTicks - left.lifetimeTicks;
      if (recency !== 0) return recency;
      return left.entityKey < right.entityKey
        ? -1
        : Number(left.entityKey > right.entityKey);
    })
    .slice(0, maximum);
}

export class BrowserMatchSimulation {
  private readonly exports: WorkerMatchExports;

  constructor(module: WebAssembly.Module, seedLow: number, seedHigh: number) {
    const instance = new WebAssembly.Instance(module, {});
    this.exports = validateExports(instance.exports);
    if (
      this.exports.worker_match_abi_version() !== EXPECTED_WORKER_ABI ||
      this.exports.worker_match_authoritative_hz() !== 128 ||
      this.exports.worker_match_max_players() !== 8
    ) {
      throw new Error("Browser match Wasm exposes an unsupported ABI.");
    }
    this.check(this.exports.worker_match_init(seedLow, seedHigh), "initialize");
  }

  addHuman(slot: number, team: number): void {
    this.check(this.exports.worker_match_add_human(slot, team), "add player");
  }

  removePlayer(slot: number): void {
    this.check(this.exports.worker_match_remove_player(slot), "remove player");
  }

  setHuman(slot: number): void {
    this.check(this.exports.worker_match_set_human(slot), "restore human controller");
  }

  setBot(slot: number): void {
    this.check(this.exports.worker_match_set_bot(slot), "activate bot controller");
  }

  resetPlayer(slot: number): void {
    this.check(this.exports.worker_match_reset_player(slot), "reset player");
  }

  clearInput(slot: number): void {
    this.check(this.exports.worker_match_clear_input(slot), "clear player input");
  }

  submitInput(
    slot: number,
    moveX: number,
    moveY: number,
    moveVertical: number,
    yaw: number,
    pitch: number,
    cast: boolean,
    requestedSpell: number | null,
  ): void {
    this.check(
      this.exports.worker_match_submit_input(
        slot,
        moveX,
        moveY,
        moveVertical,
        yaw,
        pitch,
        cast ? 1 << 2 : 0,
        cast ? (requestedSpell ?? -1) : -1,
      ),
      "submit player input",
    );
  }

  advanceTick(): CoreMatchSnapshot {
    this.check(this.exports.worker_match_advance_tick(), "advance authoritative tick");
    return this.snapshot();
  }

  snapshot(): CoreMatchSnapshot {
    const pointer = this.exports.worker_match_snapshot_ptr();
    const length = this.exports.worker_match_snapshot_len();
    if (
      !Number.isInteger(pointer) ||
      !Number.isInteger(length) ||
      pointer < 0 ||
      length < HEADER_WORDS ||
      pointer % 4 !== 0 ||
      pointer + length * 4 > this.exports.memory.buffer.byteLength
    ) {
      throw new Error("Browser match Wasm returned an invalid snapshot buffer.");
    }
    const words = new Int32Array(this.exports.memory.buffer, pointer, length);
    if (
      words[0] !== SNAPSHOT_MAGIC ||
      words[1] !== EXPECTED_WORKER_ABI ||
      words[2] !== length ||
      words[5] !== 128 ||
      words[7] !== PLAYER_WORDS ||
      words[9] !== PROJECTILE_WORDS ||
      words[11] !== EVENT_WORDS
    ) {
      throw new Error("Browser match Wasm snapshot header is invalid.");
    }
    const playerCount = boundedCount(words[6], 8, "player");
    const projectileCount = boundedCount(words[8], 128, "projectile");
    const eventCount = boundedCount(words[10], 512, "event");
    const playerOffset = boundedOffset(words[12], playerCount, PLAYER_WORDS, length);
    const projectileOffset = boundedOffset(
      words[13],
      projectileCount,
      PROJECTILE_WORDS,
      length,
    );
    const eventOffset = boundedOffset(words[14], eventCount, EVENT_WORDS, length);
    if (
      playerOffset !== HEADER_WORDS ||
      projectileOffset !== playerOffset + playerCount * PLAYER_WORDS ||
      eventOffset !== projectileOffset + projectileCount * PROJECTILE_WORDS ||
      eventOffset + eventCount * EVENT_WORDS !== length
    ) {
      throw new Error("Browser match Wasm snapshot sections overlap or contain trailing data.");
    }

    const players: CorePlayerSnapshot[] = [];
    for (let index = 0; index < playerCount; index += 1) {
      const at = playerOffset + index * PLAYER_WORDS;
      const cooldownTicks: number[] = [];
      for (let spell = 0; spell < BROWSER_MATCH_SPELL_SLOTS; spell += 1) {
        cooldownTicks.push(
          unsigned16Word(words, at + 14 + spell, "spell cooldown"),
        );
      }
      players.push({
        playerId: requiredWord(words, at),
        controller: requiredWord(words, at + 1),
        teamId: requiredWord(words, at + 2),
        entityKey: entityKey(requiredWord(words, at + 3), requiredWord(words, at + 4)),
        xCm: requiredWord(words, at + 5),
        yCm: requiredWord(words, at + 6),
        zCm: requiredWord(words, at + 7),
        velocityXCmPerTick: requiredWord(words, at + 8),
        velocityYCmPerTick: requiredWord(words, at + 9),
        velocityZCmPerTick: requiredWord(words, at + 10),
        yaw: requiredWord(words, at + 11) & 0xffff,
        pitch: requiredWord(words, at + 12),
        health: requiredWord(words, at + 13),
        cooldownTicks,
        outcome: requiredWord(words, at + PLAYER_WORDS - 1),
      });
    }

    const projectiles: CoreProjectileSnapshot[] = [];
    for (let index = 0; index < projectileCount; index += 1) {
      const at = projectileOffset + index * PROJECTILE_WORDS;
      projectiles.push({
        entityKey: entityKey(requiredWord(words, at), requiredWord(words, at + 1)),
        ownerPlayerId: requiredWord(words, at + 2),
        teamId: requiredWord(words, at + 3),
        xCm: requiredWord(words, at + 4),
        yCm: requiredWord(words, at + 5),
        zCm: requiredWord(words, at + 6),
        velocityXCmPerTick: requiredWord(words, at + 7),
        velocityYCmPerTick: requiredWord(words, at + 8),
        velocityZCmPerTick: requiredWord(words, at + 9),
        yaw: requiredWord(words, at + 10) & 0xffff,
        pitch: requiredWord(words, at + 11),
        lifetimeTicks: requiredWord(words, at + 12),
        flags: requiredWord(words, at + 13),
      });
    }

    const events: CoreEventSnapshot[] = [];
    for (let index = 0; index < eventCount; index += 1) {
      const at = eventOffset + index * EVENT_WORDS;
      events.push({
        eventId: safeU64(
          requiredWord(words, at),
          requiredWord(words, at + 1),
          "event id",
        ),
        tick: safeU64(requiredWord(words, at + 2), requiredWord(words, at + 3), "event tick"),
        kind: requiredWord(words, at + 4),
        actorEntityKey: optionalEntityKey(
          requiredWord(words, at + 5),
          requiredWord(words, at + 6),
        ),
        targetEntityKey: optionalEntityKey(
          requiredWord(words, at + 7),
          requiredWord(words, at + 8),
        ),
        data: [
          requiredWord(words, at + 9),
          requiredWord(words, at + 10),
          requiredWord(words, at + 11),
          requiredWord(words, at + 12),
        ],
      });
    }

    return {
      tick: safeU64(requiredWord(words, 3), requiredWord(words, 4), "match tick"),
      players,
      projectiles,
      events,
    };
  }

  private check(status: number, operation: string): void {
    if (status !== 0) {
      throw new ApiError(
        409,
        "browser_match_core_rejected",
        `Authoritative browser match failed to ${operation}.`,
        { status },
      );
    }
  }
}

function validateExports(exports: WebAssembly.Exports): WorkerMatchExports {
  const functions = [
    "worker_match_abi_version",
    "worker_match_authoritative_hz",
    "worker_match_max_players",
    "worker_match_init",
    "worker_match_add_human",
    "worker_match_add_bot",
    "worker_match_remove_player",
    "worker_match_set_human",
    "worker_match_set_bot",
    "worker_match_reset_player",
    "worker_match_clear_input",
    "worker_match_submit_input",
    "worker_match_advance_tick",
    "worker_match_snapshot_ptr",
    "worker_match_snapshot_len",
  ] as const;
  if (!(exports.memory instanceof WebAssembly.Memory)) {
    throw new Error("Browser match Wasm must export linear memory.");
  }
  for (const name of functions) {
    if (typeof exports[name] !== "function") {
      throw new Error(`Browser match Wasm is missing ${name}.`);
    }
  }
  return exports as WorkerMatchExports;
}

function boundedCount(raw: number | undefined, maximum: number, label: string): number {
  if (raw === undefined || !Number.isInteger(raw) || raw < 0 || raw > maximum) {
    throw new Error(`Browser match Wasm ${label} count is invalid.`);
  }
  return raw;
}

function boundedOffset(
  raw: number | undefined,
  count: number,
  stride: number,
  length: number,
): number {
  if (
    raw === undefined ||
    !Number.isInteger(raw) ||
    raw < HEADER_WORDS ||
    raw + count * stride > length
  ) {
    throw new Error("Browser match Wasm snapshot offset is invalid.");
  }
  return raw;
}

function requiredWord(words: Int32Array, index: number): number {
  const value = words[index];
  if (value === undefined) throw new Error("Browser match Wasm snapshot is truncated.");
  return value;
}

function unsigned16Word(words: Int32Array, index: number, label: string): number {
  const value = requiredWord(words, index);
  if (!Number.isInteger(value) || value < 0 || value > 0xffff) {
    throw new Error(`Browser match Wasm ${label} is invalid.`);
  }
  return value;
}

function unsignedLow(value: number): number {
  return value >>> 0;
}

function safeU64(low: number, high: number, label: string): number {
  const value = unsignedLow(low) + unsignedLow(high) * 0x1_0000_0000;
  if (!Number.isSafeInteger(value)) {
    throw new Error(`Browser match ${label} exceeds JavaScript's safe range.`);
  }
  return value;
}

function entityKey(low: number, high: number): string {
  return `${unsignedLow(high)}:${unsignedLow(low)}`;
}

function optionalEntityKey(low: number, high: number): string | null {
  return low === 0 && high === 0 ? null : entityKey(low, high);
}
