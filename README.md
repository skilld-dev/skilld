<h1>skilld</h1>

[![npm version](https://img.shields.io/npm/v/skilld?color=yellow)](https://npmjs.com/package/skilld)
[![npm downloads](https://img.shields.io/npm/dm/skilld?color=yellow)](https://npm.chart.dev/skilld)
[![license](https://img.shields.io/github/license/skilld-dev/skilld?color=yellow)](https://github.com/skilld-dev/skilld/blob/main/LICENSE)

> 🪶 Curated agent skills by humans. Search, run, install, and keep them current. One command, every Agent.

<a href="https://skilld.dev"><picture><source media="(prefers-color-scheme: dark)" srcset=".github/logos/logo-mark.svg"><img src=".github/logos/logo-mark-light.svg" alt="skilld logo" width="120"></picture></a>

<p align="center">
<table>
<tbody>
<td align="center">
<sub>Made possible by my <a href="https://github.com/sponsors/harlan-zw">Sponsor Program 💖</a><br> Follow me <a href="https://twitter.com/harlan_zw">@harlan_zw</a> 🐦 • Join <a href="https://discord.gg/275MBUBvgP">Discord</a> for help</sub><br>
</td>
</tbody>
</table>
</p>

## Features

- 👤 **Every Skill has a human author.** A maintainer wrote it in their own GitHub Repository. skilld.dev lists it under their name with a link to the source file.
- 📖 **Read before you run.** `skilld run` prints `SKILL.md` and installs nothing. Your Agent follows it for this session only.
- 🔁 **Know when the source moved.** skilld records the source commit. `skilld update --check` reports each update relation, and `skilld outdated` lists what fell behind.
- 🎯 **One command, every Agent.** `skilld install` detects your Agent targets. Claude Code, Codex, Cursor, Gemini CLI, and the rest get the same Skill.
- 🔏 **Verified delivery.** skilld.dev builds an immutable Artifact from an exact commit. The CLI checks its digest and attestation before it writes a file.
- 🦀 **Native binary, no runtime.** The npm package selects a native executable for your system. No JavaScript engine, no fallback.

## What is skilld?

skilld is a curated registry of agent Skills that real maintainers wrote in their own GitHub repositories, usable with one command in every Agent.

A Skill is a directory with a `SKILL.md` file in the [Agent Skills](https://agentskills.io) format.
Install-count leaderboards and generated doc dumps tell you a Skill is popular.
skilld tells you who wrote it, links the exact file, and tracks the commit it came from, so you can read it before your Agent follows it.

The `skilld` CLI searches, runs, installs, updates, verifies, and removes Skills.
It contains no Skill generation logic and no Agent runtime.
Skill authoring lives in visible [skilld-maintained Skills](#author-a-skill) and the optional [Harness](#harness).

See how skilld compares with skills.sh and Context7: [skilld.dev/vs/skills-sh](https://skilld.dev/vs/skills-sh).

## Get Started

Install the CLI:

```sh
npm install --global skilld
```

Find a Skill, then run it for this session:

```sh
skilld search find-skill
skilld run skilld:skilld-dev/skills/find-skill
```

`skilld run` prints `SKILL.md` to stdout and writes no file.
Ask your Agent to run the command and follow the printed instructions.
If you run it yourself, pass the output to your Agent.

Install the Skill when you want it in every session:

```sh
skilld install skilld:skilld-dev/skills/find-skill
```

An install writes files. If an Agent runs the install, it asks you first.

Teach your Agent the CLI with the skilld-maintained `skilld` Skill:

```sh
skilld install skilld --global
```

Use `--agent` when you want an explicit Agent target.
Repeat it for several targets. Use `--agent all` for every known target.
`-g` is the short form of `--global`.

```sh
skilld install skilld --global --agent codex
skilld install skilld -g --agent kiro --agent zed
skilld install skilld -g --agent all
```

### Selectors

`skilld:OWNER/REPOSITORY/SKILL` names one Skill in the registry.
`skilld search` prints the selector for each result.
`skilld-dev/skills` is the Repository; `find-skill` is the Skill directory inside it.

### Supporting files

skilld names the supporting files a Skill carries and prints none of them.
Read one when the instructions call for it:

```sh
skilld run skilld:skilld-dev/skills/find-skill --revision <commit> --file references/api.md
```

Use the revision and file-read command from the initial output.
skilld never prints executable or binary files.
A Skill that must run its own script needs an install.

## Commands

```sh
# Find a Skill
skilld search <query>

# Run a Skill for this session only
skilld run <selector>

# Read one supporting file that Skill carries
skilld run <selector> --revision <commit> --file <path>

# Install a Skill in the current project, or restore the lockfile
skilld install <selector>
skilld install

# Inspect installed Skills
skilld list
skilld view <skill>

# Keep Skills current
skilld outdated
skilld update --check --json
skilld update <skill>
skilld verify <skill>

# Remove a Skill
skilld remove <skill>
```

Project installs update `.skills/skilld-lock.yaml` and the selected Agent targets.
Use `--global` (or `-g`) for account level Agent targets.
Use `--agent <agent>` to name a target; repeat it for several, or use `--agent all`.
Use `--mode copy` or `--mode symlink` to control target writes.
Use `--json` with `search`, `run`, and `update --check` for stable output.

Run `skilld install --help` for every Agent target value.

## Author a Skill

Run the skilld-maintained `generate-package-skill` Skill, or the Harness, to bootstrap a draft Skill for a package you maintain.
Edit the draft, commit it to your Repository, and own it from there.
skilld lists it with your name and a link to the file.

The skilld-maintained Skills:

- [`skilld`](./skills/skilld): search, run, and install Skills with the CLI
- [`generate-package-skill`](./skills/generate-package-skill): draft a Skill for a package you maintain
- [`generate-project-skill`](./skills/generate-project-skill): draft a Skill from a project you maintain
- [`review-skill`](./skills/review-skill): review a Skill before you publish it

Direct Skill runs stay user reviewed.
The instructions and changes stay visible to you.

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
Review the Skill before use.

Direct mode never handles private Repositories.
It never falls back to skilld.dev.

### Source status

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

## Development

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
