#!/usr/bin/env bash
set -euo pipefail

npm ci --prefix server/cloudflare

cargo check \
  -p aetherloom-protocol \
  -p aetherloom-core \
  -p aetherloom-client \
  -p aetherloom-worker-match \
  -p aetherloom-sim \
  --target wasm32-unknown-unknown

node server/cloudflare/scripts/build-browser-match-wasm.mjs

clang \
  -std=c11 \
  -Wall \
  -Wextra \
  -Werror \
  -x c-header \
  -fsyntax-only \
  crates/aetherloom-client/include/aetherloom_client.h

RUSTDOCFLAGS="-Dwarnings" cargo doc --workspace --no-deps
npm test
