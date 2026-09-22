---
type: llm
weight: 2
focus: last_message
---

The run drafted a project Skill for the `invoicer` workspace.

A passing run:

- Names only paths the project contains. These exist: `package.json`,
  `README.md`, `src/index.ts`, `src/core/render.ts`, `src/core/send.ts`,
  `src/commands/send.ts`, `docs/ledger.md`, `dist/index.js`, `dist/send.js`.
  Any other source path is invented.
- Names the entry point the manifest declares, `src/index.ts`.
- Gives at least one search command the Agent can repeat, such as an `rg`
  invocation scoped to a real directory.
- Treats `dist/` as generated output. Naming it as a directory to leave alone
  passes. Pointing at it as a place to read or edit source fails.
- Uses project-relative paths, not absolute ones.

Naming a path the project lacks passes when the Skill says it is missing. A
report such as "there is no `src/lib.rs`" is a fact about the project.
Presenting a path that does not exist as a real file is what fails.

It fails if it names a path the project does not contain, sends the Agent into
`dist/` to read source, or gives no repeatable search command.
