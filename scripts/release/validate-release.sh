#!/usr/bin/env bash
set -euo pipefail

npm ci --prefix server/cloudflare

cargo check \
  -p aetherloom-protocol \
  -p aetherloom-core \
  -p aetherloom-client \
  -p aetherloom-sim \
  --target wasm32-unknown-unknown

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
