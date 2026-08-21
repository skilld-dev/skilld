# CLAUDE.md

Read `GLOSSARY.md` before changing public names, commands, errors, routes, or documentation.

## Commands

```sh
pnpm install
pnpm test:run
pnpm lint
pnpm typecheck
pnpm build
```

Use focused commands during development:

```sh
pnpm test:loader
pnpm --filter skilld-protocol test:run
pnpm --filter skilld-harness test:run
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Product boundary

The native `skilld` CLI searches, installs, lists, views, removes, updates, and verifies Skills.
It also manages account authentication and Agent target configuration.

The skilld CLI contains no Skill generation logic or Agent runtime.

`skilld-harness` runs visible skilld-maintained Skills for generation and review.
Agents can run the same Skill files directly without the Harness.

Direct runs remain user reviewed.
Harness and CI runs enforce strict output checks.

## Architecture

- `crates/skilld-core`: shared Rust domain types and Agent target rules
- `crates/skilld-command`: command parsing and local Skill operations
- `crates/skilld-auth`: PKCE, callback, refresh, logout, and keychain contracts
- `crates/skilld-native`: native operating system adapters and executable
- `crates/skilld-wasi`: internal WASIp2 proof
- `bin` and `loader`: minimal npm native executable selector
- `packages/cli-*`: system specific native npm packages
- `packages/harness`: JavaScript Harness package
- `packages/protocol`: skilld.dev API wire contract
- `skills`: visible skilld-maintained Skills

The npm package has no JavaScript CLI fallback.
The WASIp2 package remains private until write behavior reaches native parity.

## Artifact delivery

The skilld CLI uses the skilld.dev API for remote Skills.
GitHub remains the source of truth.

The CLI verifies exact statement bytes, signatures, digests, and archive structure.
Production verification must start from a compiled trusted root key.
If no root key exists, fail with `TRUSTED_ROOT_UNCONFIGURED`.

Private Repository delivery requires account authentication and a GitHub App installation.
Direct mode supports public GitHub Repositories only.
Direct mode records the `unverified` source status.

## Change rules

- Keep generation code outside every CLI crate and the root npm loader.
- Use tagged results for expected failures.
- Parse untrusted input once at its boundary.
- Pass clients, clocks, keys, and storage as explicit dependencies.
- Test exported behavior.
- Keep direct instructions visible in `skills`.
- Keep Harness enforcement in `packages/harness`.
- Never add a JavaScript CLI engine or fallback.

<!-- skilld -->
Before modifying code, evaluate each installed skill against the current task.
For each skill, determine YES or NO relevance and invoke all YES skills before proceeding.
<!-- /skilld -->
