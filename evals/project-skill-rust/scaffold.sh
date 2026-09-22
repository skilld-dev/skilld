#!/usr/bin/env bash
set -e
mkdir -p src/store tests target/debug
cat > Cargo.toml <<'TOML'
[package]
name = "ledgerd"
version = "0.2.0"
edition = "2021"
description = "Append-only ledger daemon."

[[bin]]
name = "ledgerd"
path = "src/main.rs"
TOML
printf '# ledgerd\n\nAppend-only ledger daemon.\n' > README.md
printf 'mod store;\n\nfn main() {\n    store::open();\n}\n' > src/main.rs
printf 'pub mod journal;\n\npub fn open() {}\n' > src/store/mod.rs
printf 'pub fn append(entry: &str) -> usize {\n    entry.len()\n}\n' > src/store/journal.rs
printf '#[test]\nfn appends_an_entry() {\n    assert_eq!(ledgerd::store::journal::append("a"), 1);\n}\n' > tests/journal.rs
printf 'binary\n' > target/debug/ledgerd
