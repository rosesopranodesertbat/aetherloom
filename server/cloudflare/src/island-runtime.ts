import { ApiError } from "./util.ts";

export const ISLAND_TICK_HZ = 128;
export const MAX_TICKS_PER_COMMAND_BATCH = 32;
export const ISLAND_IDLE_AFTER_MS = 30_000;
export const MAX_ISLAND_COMMAND_BYTES = 4_096;
// This is a fail-closed storage bound, not a truncation threshold. Reaching it
// rejects the delivery without moving the durable wall-clock anchor, so no
// authoritative tick is silently discarded.
export const MAX_RETAINED_ISLAND_BACKLOG_TICKS = 1_000_000;

export interface IslandTickClock {
  lastWallClockMs: number;
  tickRemainder: number;
  backlogTicks: number;
  activeUntilMs: number | null;
}

export type IslandTickPlan =
  | {
      kind: "not_due";
      retryAfterMs: number;
    }
  | {
      kind: "advance";
      ticks: number;
      backlogTicks: number;
      tickRemainder: number;
      wasDormant: boolean;
    };

export interface IslandAdvance {
  events: Uint8Array;
}

export interface HeadlessIslandSimulation {
  readonly available: boolean;
  restore(checkpoint: ArrayBuffer | null): Promise<void>;
  advance(ticks: number, commands: Uint8Array): Promise<IslandAdvance>;
  checkpoint(): Promise<ArrayBuffer>;
}

/**
 * Computes one bounded simulation batch without consuming excess elapsed time.
 * Whole ticks not executed in this delivery remain as durable backlog; the
 * sub-tick numerator remains in `tickRemainder`.
 */
export function planIslandTickBatch(clock: IslandTickClock, nowMs: number): IslandTickPlan {
  assertSafeClockInteger(clock.lastWallClockMs, "lastWallClockMs", 0);
  assertSafeClockInteger(clock.tickRemainder, "tickRemainder", 0, 999);
  assertSafeClockInteger(
    clock.backlogTicks,
    "backlogTicks",
    0,
    MAX_RETAINED_ISLAND_BACKLOG_TICKS,
  );
  assertSafeClockInteger(nowMs, "nowMs", 0);
  if (
    clock.activeUntilMs !== null &&
    (!Number.isSafeInteger(clock.activeUntilMs) || clock.activeUntilMs < 0)
  ) {
    throw new Error("activeUntilMs must be a non-negative safe integer or null.");
  }

  const wasDormant = clock.activeUntilMs === null || clock.activeUntilMs < nowMs;
  const elapsedMs = wasDormant ? 0 : Math.max(0, nowMs - clock.lastWallClockMs);
  const numerator = wasDormant
    ? 0
    : elapsedMs * ISLAND_TICK_HZ + clock.tickRemainder;
  if (!Number.isSafeInteger(numerator)) {
    throw new ApiError(
      503,
      "island_tick_clock_overflow",
      "Island tick clock exceeded its safe integer range.",
    );
  }

  const newlyDueTicks = wasDormant ? 1 : Math.floor(numerator / 1_000);
  const totalDueTicks = clock.backlogTicks + newlyDueTicks;
  if (!Number.isSafeInteger(totalDueTicks)) {
    throw new ApiError(
      503,
      "island_tick_clock_overflow",
      "Island tick backlog exceeded its safe integer range.",
    );
  }
  if (totalDueTicks < 1) {
    return {
      kind: "not_due",
      retryAfterMs: Math.max(1, Math.ceil((1_000 - numerator) / ISLAND_TICK_HZ)),
    };
  }

  const ticks = Math.min(MAX_TICKS_PER_COMMAND_BATCH, totalDueTicks);
  const backlogTicks = totalDueTicks - ticks;
  if (backlogTicks > MAX_RETAINED_ISLAND_BACKLOG_TICKS) {
    throw new ApiError(
      503,
      "island_tick_backlog_overflow",
      "Island tick backlog exceeded its durable safety bound; no ticks were consumed.",
      {
        maximumBacklogTicks: MAX_RETAINED_ISLAND_BACKLOG_TICKS,
      },
    );
  }
  return {
    kind: "advance",
    ticks,
    backlogTicks,
    tickRemainder: wasDormant ? 0 : numerator % 1_000,
    wasDormant,
  };
}

/**
 * The simulation mutates before SQLite can journal the resulting batch. If
 * SQLite fails or returns an ambiguous commit result, the in-memory simulation
 * must never be reused: the next operation restores the last checkpoint and
 * replays only committed journal rows.
 */
export function commitAdvancedSimulation<T>(
  commit: () => T,
  invalidateInMemorySimulation: () => void,
): T {
  let committed = false;
  try {
    const result = commit();
    committed = true;
    return result;
  } finally {
    if (!committed) invalidateInMemorySimulation();
  }
}

interface IslandWasmExports extends WebAssembly.Exports {
  memory: WebAssembly.Memory;
  islandInputPtr: () => number;
  islandInputCapacity: () => number;
  islandRestore: (byteLength: number) => number;
  islandAdvanceTicks: (ticks: number, commandByteLength: number) => number;
  islandEventsPtr: () => number;
  islandEventsLen: () => number;
  islandCheckpointPtr: () => number;
  islandCheckpointLen: () => number;
}

class MissingIslandSimulation implements HeadlessIslandSimulation {
  readonly available = false;

  restore(): Promise<void> {
    return Promise.reject(this.error());
  }

  advance(): Promise<IslandAdvance> {
    return Promise.reject(this.error());
  }

  checkpoint(): Promise<ArrayBuffer> {
    return Promise.reject(this.error());
  }

  private error(): ApiError {
    return new ApiError(
      501,
      "island_runtime_not_bound",
      "The headless island Wasm adapter is defined but no compatible artifact is bound.",
    );
  }
}

export class HeadlessWasmIslandSimulation implements HeadlessIslandSimulation {
  readonly available = true;
  private readonly exports: IslandWasmExports;

  constructor(module: WebAssembly.Module) {
    const instance = new WebAssembly.Instance(module, {});
    this.exports = validateExports(instance.exports);
  }

  async restore(checkpoint: ArrayBuffer | null): Promise<void> {
    const bytes = checkpoint === null ? new Uint8Array() : new Uint8Array(checkpoint);
    this.writeInput(bytes);
    if (this.exports.islandRestore(bytes.byteLength) !== 0) {
      throw new ApiError(409, "island_restore_failed", "Headless island core rejected its checkpoint.");
    }
  }

  async advance(ticks: number, commands: Uint8Array): Promise<IslandAdvance> {
    if (!Number.isInteger(ticks) || ticks < 1 || ticks > MAX_TICKS_PER_COMMAND_BATCH) {
      throw new Error(`Island tick batch must be 1-${MAX_TICKS_PER_COMMAND_BATCH}.`);
    }
    this.writeInput(commands);
    if (this.exports.islandAdvanceTicks(ticks, commands.byteLength) !== 0) {
      throw new ApiError(409, "island_command_rejected", "Headless island core rejected the command batch.");
    }
    return {
      events: this.readOutput(this.exports.islandEventsPtr(), this.exports.islandEventsLen()),
    };
  }

  async checkpoint(): Promise<ArrayBuffer> {
    const bytes = this.readOutput(
      this.exports.islandCheckpointPtr(),
      this.exports.islandCheckpointLen(),
    );
    return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer;
  }

  private writeInput(bytes: Uint8Array): void {
    const pointer = this.exports.islandInputPtr();
    const capacity = this.exports.islandInputCapacity();
    if (
      !Number.isInteger(pointer) ||
      !Number.isInteger(capacity) ||
      pointer < 0 ||
      capacity < bytes.byteLength ||
      pointer + bytes.byteLength > this.exports.memory.buffer.byteLength
    ) {
      throw new ApiError(500, "island_abi_violation", "Headless island input buffer is invalid.");
    }
    new Uint8Array(this.exports.memory.buffer, pointer, bytes.byteLength).set(bytes);
  }

  private readOutput(pointer: number, byteLength: number): Uint8Array {
    if (
      !Number.isInteger(pointer) ||
      !Number.isInteger(byteLength) ||
      pointer < 0 ||
      byteLength < 0 ||
      pointer + byteLength > this.exports.memory.buffer.byteLength
    ) {
      throw new ApiError(500, "island_abi_violation", "Headless island output buffer is invalid.");
    }
    return new Uint8Array(
      new Uint8Array(this.exports.memory.buffer, pointer, byteLength),
    );
  }
}

export function createIslandSimulation(
  module: WebAssembly.Module | undefined,
): HeadlessIslandSimulation {
  return module === undefined ? new MissingIslandSimulation() : new HeadlessWasmIslandSimulation(module);
}

function validateExports(exports: WebAssembly.Exports): IslandWasmExports {
  const requiredFunctions = [
    "islandInputPtr",
    "islandInputCapacity",
    "islandRestore",
    "islandAdvanceTicks",
    "islandEventsPtr",
    "islandEventsLen",
    "islandCheckpointPtr",
    "islandCheckpointLen",
  ];
  if (!(exports.memory instanceof WebAssembly.Memory)) {
    throw new Error("Headless island Wasm must export memory.");
  }
  for (const name of requiredFunctions) {
    if (typeof exports[name] !== "function") {
      throw new Error(`Headless island Wasm is missing export ${name}.`);
    }
  }
  return exports as IslandWasmExports;
}

function assertSafeClockInteger(
  value: number,
  name: string,
  minimum: number,
  maximum = Number.MAX_SAFE_INTEGER,
): void {
  if (!Number.isSafeInteger(value) || value < minimum || value > maximum) {
    throw new Error(`${name} must be a safe integer in the range ${minimum}-${maximum}.`);
  }
}
