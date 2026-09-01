<h1><a href="https://skilld.dev"><picture><source media="(prefers-color-scheme: dark)" srcset=".github/logos/logo-mark.svg"><img src=".github/logos/logo-mark-light.svg" alt="" width="28" height="28" valign="middle"></picture></a> skilld</h1>

[![npm version](https://img.shields.io/npm/v/skilld?color=yellow)](https://npmjs.com/package/skilld)
[![npm downloads](https://img.shields.io/npm/dm/skilld?color=yellow)](https://npm.chart.dev/skilld)
[![license](https://img.shields.io/npm/l/skilld?color=yellow)](https://github.com/skilld-dev/skilld/blob/main/LICENSE)

**Curated agent skills by humans.**

Search, run, install, and keep them current. One command, every Agent.

skilld is a curated registry of agent Skills real maintainers wrote in their own GitHub repositories, usable with one command in every Agent.

## Install the skilld CLI

```sh
npm install --global skilld
```

The npm package selects a native executable for the current system.
It has no JavaScript CLI engine or JavaScript fallback.

## Run a Skill without installing it

`skilld run` is the default way to use a Skill.
It prints `SKILL.md` to stdout and installs nothing.
Ask your Agent to run the command and follow the printed instructions.
If you run it yourself, pass the output to your Agent.

```sh
npx skilld run skilld:skilld-dev/skills/find-skill
```

A remote run writes no Skill files.
It creates no lockfile entry, Agent target, project file, or Skill cache.

skilld names the supporting files a Skill carries and prints none of them.
Read one when the instructions call for it:

```sh
npx skilld run skilld:skilld-dev/skills/find-skill --revision <commit> --file references/api.md
```

Use the revision and file-read command from the initial output.
skilld never prints executable or binary files.
A Skill that must run its own script needs an install.

## Install a Skill

Install a Skill when you want it in every session:

```sh
skilld install skilld:skilld-dev/skills/find-skill
```

An install writes files. Ask the user first.

Use `--agent` when you want an explicit Agent target:

```sh
skilld install skilld:skilld-dev/skills/find-skill --agent codex
```

Install the skilld-maintained `skilld` Skill so your Agent knows the CLI:

```sh
skilld install skilld --global
```

## Use the skilld CLI

```sh
# Find a Skill
skilld search find-skill

# Run a Skill for this session only
skilld run skilld:skilld-dev/skills/find-skill

# Read one supporting file that Skill carries
skilld run skilld:skilld-dev/skills/find-skill --revision <commit> --file references/api.md

# Install a Skill in the current project
skilld install skilld:skilld-dev/skills/find-skill

# Inspect installed Skills
skilld list
skilld view find-skill

# Keep Skills current
skilld verify find-skill
skilld update find-skill
skilld outdated

# Check update relations for an Agent or CI
skilld update --check --json

# Remove a Skill
skilld remove find-skill
```

Project installs update `.skills/skilld-lock.yaml` and the selected Agent targets.
Use `--global` for account level Agent targets.
Use `--mode copy` or `--mode symlink` to control target writes.

Run `skilld install` without a source to restore the lockfile state.

## Why skilld

- **Every Skill has a human author.** A maintainer wrote it in their own GitHub Repository, and skilld.dev lists it under their name.
- **Read the SKILL.md first.** `skilld run` prints the file before your Agent follows it, and `skilld view` links the source Repository.
- **Know when the source moved.** skilld records the source commit. `skilld update --check` reports each update relation, and `skilld outdated` lists what fell behind.
- **One command, every Agent.** `skilld install` detects your Agent targets. Claude Code, Codex, Cursor, Gemini CLI, and the rest get the same Skill.

skilld reads `SKILL.md` files in the Agent Skills format ([agentskills.io](https://agentskills.io)).
The CLI checks that `SKILL.md` sits at the Skill root and declares a name.

See how skilld compares with skills.sh and Context7: https://skilld.dev/vs/skills-sh

## Author a Skill <a id="generate-or-review-a-skill"></a>

Run the skilld-maintained `generate-package-skill` Skill, or the Harness, to bootstrap a draft Skill for a package you maintain.
Edit the draft, commit it to your Repository, and own it from there.
skilld lists it with your name and a link to the file.

The skilld-maintained Skills:

- [`skilld`](./skills/skilld): search, run, and install Skills with the CLI
- [`generate-package-skill`](./skills/generate-package-skill): draft a Skill for a package you maintain
- [`generate-project-skill`](./skills/generate-project-skill): draft a Skill from a project you maintain
- [`review-skill`](./skills/review-skill): review a Skill before you publish it

## Artifact delivery

The skilld CLI resolves remote Skills through the skilld.dev API.
GitHub remains the source of truth.

skilld.dev builds an immutable Artifact from an exact Git commit.
The CLI checks its digest, Artifact attestation, check results, and archive before installation.
The CLI stops pending Artifact creation after at most 60 seconds.

Private Repository delivery requires both:

- `skilld auth login` for a skilld.dev account
- Access through the skilld GitHub App installation

Private Artifact responses use short lived, one time grants.
The API does not expose private storage addresses.

### Direct mode

`--direct` fetches a public GitHub Repository without the skilld.dev API.
Explicit GitHub selectors use hosted Artifact delivery unless you add `--direct`.

```sh
skilld install github:skilld-dev/skilld/skills/skilld --direct --agent codex
```

The installed Skill receives the `unverified` source status.
The user reviews the Skill before use.

Direct mode never handles private Repositories.
It never falls back to skilld.dev.

## Source status

- `verified`: skilld checked a skilld.dev Artifact and its attestation.
- `local`: the Skill came from a local directory or a bundled skilld-maintained Skill.
- `unverified`: direct mode fetched the Skill from public GitHub.

`verified` describes provenance checks.
It does not endorse the instructions inside a Skill.
See [SECURITY.md](./SECURITY.md) for what skilld checks and how to report a problem.

## Account and configuration

```sh
skilld auth login
skilld auth status
skilld auth logout

skilld config get agent.targets
skilld config set agent.targets codex,claude-code
skilld config list
```

Native builds store account credentials in the operating system keychain.
The CLI does not store tokens in environment variables or plain text files.

## Harness

Use [`skilld-harness`](./packages/harness) when an application or CI needs strict output checks.
The Harness runs the same visible Skill files through an AI SDK Harness.
See the [`skilld-harness` guide](./packages/harness/README.md) for its full contract.

## Upgrade from v2

v3 does not import v2 configuration or lockfiles.
Back up v2 state before replacing the CLI.

Follow the [v2 to v3 migration guide](./docs/migrate-v2-to-v3.md).
It maps removed commands and explains rollback limits.

## Development <a id="v3-development"></a>

```sh
pnpm install
pnpm test:run
pnpm lint
pnpm typecheck
pnpm build
```

The Rust workspace owns the skilld CLI.
`packages/harness` owns generation and review execution.
`packages/protocol` owns the skilld.dev wire contract.
`skills` owns the visible skilld-maintained Skills.

The WASIp2 build remains an internal proof.
Published packages use native executables only.

## License

[MIT](./LICENSE)
