<h1 align="center">
<a href="https://skilld.dev"><picture>
<source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/skilld-dev/skilld/main/.github/logos/logo.svg">
<img alt="skilld" src="https://raw.githubusercontent.com/skilld-dev/skilld/main/.github/logos/logo-light.svg" width="220">
</picture></a>
</h1>

<p align="center">
Open source, privacy friendly Agent Skills, curated by humans.<br>Search, run, install, and keep them current.
</p>

<p align="center">
<a href="https://npmjs.com/package/skilld"><img alt="npm version" src="https://img.shields.io/npm/v/skilld?style=flat&labelColor=1c1917&color=e11d48"></a>
<a href="https://npm.chart.dev/skilld"><img alt="npm downloads" src="https://img.shields.io/npm/dm/skilld?style=flat&labelColor=1c1917&color=e11d48"></a>
<a href="https://github.com/skilld-dev/skilld/blob/main/LICENSE"><img alt="license" src="https://img.shields.io/github/license/skilld-dev/skilld?style=flat&labelColor=1c1917&color=e11d48"></a>
</p>

<div align="center">
<table>
<tbody>
<tr>
<td align="center">
<sub>Made possible by my <a href="https://github.com/sponsors/harlan-zw">Sponsor Program 💖</a><br> Follow me <a href="https://twitter.com/harlan_zw">@harlan_zw</a> 🐦 • Join <a href="https://discord.gg/275MBUBvgP">Discord</a> for help</sub>
</td>
</tr>
</tbody>
</table>
</div>

## Features

- 📖 **Try a Skill without installing it.** `skilld run` prints `SKILL.md` to stdout and writes no file. Your Agent follows it for this session only.
- 🎯 **One install, 19 Agent targets.** `skilld install` detects the Agents you use and writes the same Skill to each. Claude Code, Codex, Cursor, Gemini CLI, Zed, and 14 more.
- 🦀 **One native binary, no runtime to install.** Startup is under a millisecond. Install it with `curl` and you need no Node.js at all.
- 🔏 **Every install is pinned to a commit.** The lockfile records the exact source commit. skilld checks the Artifact digest and attestation before it writes a file. `skilld outdated` reports when the source moved.
- 🔍 **No telemetry.** skilld sends no analytics. Account credentials go to your operating system keychain, never a plain text file.

## What is skilld?

skilld is a curated registry of agent Skills.
Real maintainers write them in their own GitHub repositories.
One command loads one into any Agent.

A Skill is a directory with a `SKILL.md` file in the [Agent Skills](https://agentskills.io) format.
Most Skill directories rank by install count, or ship a generated doc dump.
skilld names the author, links the exact file, and records the commit it came from.
Read it before your Agent follows it.

The `skilld` CLI searches, runs, installs, updates, verifies, and removes Skills.
It contains no Skill generation logic and no Agent runtime.
Skill authoring lives in visible [skilld-maintained Skills](#author-a-skill) and the optional [Harness](#harness).

## Get Started

Your Agent runs skilld, not you. Teach it the CLI once:

```sh
npm install --global skilld
skilld install skilld --global
```

Then ask for what you need, in your own words:

> Find a skilld Skill for Vue and use it

Your Agent searches the registry, reads the descriptions, and loads the Skill for that session.
It installs nothing unless you ask it to.

Find a Skill yourself on [skilld.dev](https://skilld.dev/skills).
[Trending Skills](https://skilld.dev/skills/trending) shows what is moving this month.
Every Skill page credits its author and links the source file on GitHub.
[Curators](https://skilld.dev/community) publish named collections that `skilld add` installs in one command.

### Run it yourself

You need no install and no project files:

```sh
npx skilld search vue
npx skilld run antfu/skills/vue
```

`skilld run` prints `SKILL.md` to stdout and writes no file.
Pass the output to your Agent.

Keep a Skill when you want it in every session:

```sh
npx skilld install antfu/skills/vue
```

An install writes files. If an Agent runs the install, it asks you first.

### Run or install?

| | `skilld run` | `skilld install` |
| --- | --- | --- |
| Use it for | The task in front of you | Every session in this project |
| Files written | None | `.skills`, the lockfile, and Agent targets |
| Cleanup | None | `skilld remove` |
| Updates | Loads the source each time | `skilld update` |
| Skill scripts | Never printed or executed | On disk for the Skill to use |

Start with `skilld run`. Install when you reach for the same Skill again.

### Agent targets

Use `--agent` to name a target. Repeat it for several, or use `--agent all`.
`-g` is the short form of `--global`.

```sh
skilld install skilld -g --agent codex
skilld install skilld -g --agent kiro --agent zed
skilld install skilld -g --agent all
```

### Selectors

`OWNER/REPOSITORY/SKILL` names one Skill in the registry.
`OWNER/REPOSITORY` names every Skill in one Repository.
`skilld search` prints the selector for each result.
skilld 3.0 printed `skilld:OWNER/REPOSITORY/SKILL` and `gh:OWNER/REPOSITORY`. The CLI still accepts both.
`skilld-dev/skills` is the Repository; `find-skill` is the Skill directory inside it.

### Supporting files

skilld names the supporting files a Skill carries and prints none of them.
Read one when the instructions call for it:

```sh
skilld run skilld-dev/skills/find-skill --revision <commit> --file agents/openai.yaml
```

Use the revision and file-read command from the initial output.
skilld never prints executable or binary files.
A Skill that must run its own script needs an install.

## Install without Node.js

macOS and Linux:

```sh
curl -fsSL https://github.com/skilld-dev/skilld/releases/latest/download/install.sh | sh
```

Windows PowerShell:

```powershell
irm https://github.com/skilld-dev/skilld/releases/latest/download/install.ps1 | iex
```

The script installs one native binary to `~/.skilld/bin`.
That install upgrades itself in the background after it verifies the signed release manifest.
Restart skilld to use the new version.
An npm install prints the upgrade command instead.
Set `SKILLD_NO_UPGRADE=1` to turn off upgrade checks.
See [SECURITY.md](./SECURITY.md#how-the-cli-upgrades-itself) for each check.

The npm package selects the same native executable for your system.
It carries no JavaScript engine and no fallback.

## Commands

```sh
# Find a Skill
skilld search <query>

# Run a Skill for this session only (start here)
skilld run <selector>

# Read one supporting file that Skill carries
skilld run <selector> --revision <commit> --file <path>

# Install a Skill in the current project, or restore the lockfile
skilld install <selector>
skilld install

# List every Skill a Repository, curator, or collection names
skilld run anthropics/skills
skilld run @harlan-zw
skilld run @harlan-zw/agent-workflow-stack

# Install every Skill one of those refs names
skilld add @harlan-zw/agent-workflow-stack

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

Run a skilld-maintained Skill, or the Harness, to bootstrap a draft Skill you own.
Use `generate-package-skill` for a package you maintain.
Use `generate-project-skill` for the project you work in.
Edit the draft, commit it to your Repository, and own it from there.
skilld lists it with your name and a link to the file.

```sh
skilld run skilld-dev/skilld/generate-project-skill
skilld run skilld-dev/skilld/generate-package-skill
```

A project Skill carries the search commands your Agent repeats against real files.
It replaces the v2 local documentation index.

The skilld-maintained Skills:

- [`skilld`](./skills/skilld): search, run, and install Skills with the CLI
- [`generate-package-skill`](./skills/generate-package-skill): draft a Skill for a package you maintain
- [`generate-project-skill`](./skills/generate-project-skill): draft a Skill from a project you maintain
- [`review-skill`](./skills/review-skill): review a Skill before you publish it

Direct Skill runs stay user reviewed.
The instructions and changes stay visible to you.

`skilld run` with a Repository, curator, or collection ref prints an index.
The index has one line per Skill with its run command. It loads no Skill.
`skilld add` installs the Skills the same ref names. It accepts the `skilld install` flags.
A terminal asks which Skills to install. `--all` installs every one without asking.
Any other context, such as an Agent, a pipe, or CI, installs every one.
A Repository that skilld.dev does not list yet falls back to its public GitHub tree.
Those Skills install through direct mode and record the `unverified` source status.

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

## Privacy

The skilld CLI sends no telemetry or analytics.
It makes network requests only for these reasons:

- `skilld search`, `skilld run`, and `skilld install` of a hosted Skill call the skilld.dev API.
- If you signed in, those requests carry your account token.
- `skilld auth login` sends your computer hostname to name the sign-in. Only you see it on skilld.dev.
- `--direct` fetches public Skills from GitHub.
- `skilld update` compares commits through skilld.dev or the GitHub API.
- At a terminal, the CLI checks GitHub Releases or the npm registry for a new version once a day.

The upgrade check never runs in CI or inside an Agent.
Set `SKILLD_NO_UPGRADE=1` to turn it off.

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
