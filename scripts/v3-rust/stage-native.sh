#!/usr/bin/env bash

set -euo pipefail

repository_root=$(git rev-parse --show-toplevel)
cargo build --manifest-path "$repository_root/Cargo.toml" --release -p skilld-native
mkdir -p "$repository_root/packages/cli-linux-x64-gnu/bin"
cp "$repository_root/target/release/skilld" "$repository_root/packages/cli-linux-x64-gnu/bin/skilld"
chmod 755 "$repository_root/packages/cli-linux-x64-gnu/bin/skilld"
