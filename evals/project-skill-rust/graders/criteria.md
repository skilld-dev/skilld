---
type: llm
weight: 2
focus: last_message
---

The run drafted a project Skill for the `ledgerd` Rust workspace.

A passing run:

- Reads `Cargo.toml` for the project name and entry point. It never claims a
  `package.json`, which this project does not have.
- Names `src/main.rs` as the binary entry point, as `[[bin]]` declares.
- Names only paths the project contains: `Cargo.toml`, `README.md`,
  `src/main.rs`, `src/store/mod.rs`, `src/store/journal.rs`, `tests/journal.rs`,
  `target/debug/ledgerd`. Any other source path is invented.
- Gives at least one search command the Agent can repeat, scoped to a real
  directory such as `src`.
- Treats `target/` as generated output rather than source to read.

Naming a path the project lacks passes when the Skill says it is missing. A
report such as "there is no `src/lib.rs`" is a fact about the project.
Presenting a path that does not exist as a real file is what fails.

It fails if it invents a path, claims a manifest the project does not have,
names the wrong entry point, or gives no repeatable search command.
