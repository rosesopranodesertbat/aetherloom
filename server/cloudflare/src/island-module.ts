// Build integration seam for the future platform-neutral Rust island core.
//
// Once that artifact exports the ABI validated in island-runtime.ts, replace
// this with a static CompiledWasm import:
//
//   import module from "../generated/island-sim.wasm";
//   export const islandSimulationModule: WebAssembly.Module | undefined = module;
//
// Wrangler uploads imported `.wasm` files as CompiledWasm modules. Keeping the
// value undefined today makes the incomplete capability explicit and allows
// every other control-plane route to compile and run safely.
export const islandSimulationModule: WebAssembly.Module | undefined = undefined;

