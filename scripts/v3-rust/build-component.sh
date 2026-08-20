#!/usr/bin/env bash

set -euo pipefail

repository_root=$(git rev-parse --show-toplevel)
component_package="$repository_root/packages/cli-wasm32-wasi"
component_output="$component_package/component"

cargo build --manifest-path "$repository_root/Cargo.toml" --release --target wasm32-wasip2 -p skilld-wasi
npm install --prefix "$component_package" --ignore-scripts --no-audit --no-fund
mkdir -p "$component_output"
npm exec --yes --package=@bytecodealliance/jco@1.30.0 -- jco transpile \
  "$repository_root/target/wasm32-wasip2/release/skilld_wasi.wasm" \
  --name skilld-wasi \
  --map 'skilld:host/*@3.0.0=./host-imports.mjs#*' \
  --out-dir "$component_output"
cp "$component_package/host-imports.mjs" "$component_output/host-imports.mjs"
