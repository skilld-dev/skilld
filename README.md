<h1 align="center">
<a href="https://skilld.dev"><picture>
<source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/skilld-dev/skilld/main/.github/logos/logo.svg">
<img alt="skilld" src="https://raw.githubusercontent.com/skilld-dev/skilld/main/.github/logos/logo-light.svg" width="260">
</picture></a>
</h1>

<p align="center">
Find, preview, run, and watch Agent Skills.<br>The open-source CLI behind <a href="https://skilld.dev">skilld.dev</a>.
</p>

<p align="center">
<a href="https://npmjs.com/package/skilld"><img alt="npm version" src="https://img.shields.io/npm/v/skilld?style=flat&labelColor=1c1917&color=e11d48"></a>
<a href="https://npm.chart.dev/skilld"><img alt="npm downloads" src="https://img.shields.io/npm/dm/skilld?style=flat&labelColor=1c1917&color=e11d48"></a>
<a href="https://github.com/skilld-dev/skilld/blob/main/LICENSE"><img alt="license" src="https://img.shields.io/github/license/skilld-dev/skilld?style=flat&labelColor=1c1917&color=e11d48"></a>
<a href="https://skilld.dev/gh/skilld-dev/skilld/skilld"><picture>
<source media="(prefers-color-scheme: dark)" srcset="https://skilld.dev/b/skilld-dev/skilld/skilld?theme=dark">
<source media="(prefers-color-scheme: light)" srcset="https://skilld.dev/b/skilld-dev/skilld/skilld?theme=light">
<img alt="Agent skill on skilld.dev" src="https://skilld.dev/b/skilld-dev/skilld/skilld?theme=light">
</picture></a>
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

## What is skilld?

[skilld.dev](https://skilld.dev) is a curated registry of Agent Skills.
Real maintainers write them in their own GitHub Repositories.
skilld.dev and the open-source `skilld` CLI cover four steps:

| Step | What you do | Where |
| --- | --- | --- |
| 🔥 **Find** | See the Skills devs talk about this week. | [Trending Skills](https://skilld.dev/skills/trending), `skilld trending`, `skilld search` |
| 🧑‍🎨 **Preview** | See what a Skill makes before you run it. | [Skill demos](https://skilld.dev/skills/demos) |
| 🏃‍♂️ **Run** | Run a Skill once, fork it, or install it. | `skilld run`, `skilld install` |
| 👀 **Watch** | Get a digest when the Skills you use change. | `skilld watch`, `skilld changes` |

A Skill is a directory with a `SKILL.md` file in the [Agent Skills](https://agentskills.io) format.
Most Skill directories rank by install count, or ship a generated doc dump.
skilld names the author, links the exact file, and records the commit it came from.
Read it before your Agent follows it.

The `skilld` CLI searches, runs, installs, updates, verifies, and removes Skills.
It contains no Skill generation logic and no Agent runtime.
Skill authoring lives in visible [skilld-maintained Skills](#author-a-skill) and the optional [Harness](#harness).

## Features

- 📖 **Run a Skill without installing it.** `skilld run` prints `SKILL.md` to stdout and writes no file. Your Agent follows it for this session only.
- 🎯 **One install, 73 Agent targets.** `skilld install` detects the Agents you use and writes the same Skill to each: Claude Code, Codex, Cursor, Gemini CLI, Zed, and 68 more.
- 🦀 **One native binary, no runtime.** It starts in under a millisecond. The `curl` install needs no Node.js.
- 🔏 **Every install is pinned to a commit.** The lockfile records the exact source commit. skilld checks the Artifact digest and attestation before it writes a file. `skilld outdated` reports when the source moved.
- 🛡️ **No telemetry.** The CLI sends no analytics. Account credentials go to your operating system keychain, never to a plain text file.

## Get started

Every command below works through `npx` with no install.
To install the CLI, see [Install the CLI](#install-the-cli).

### Find a Skill

[Trending Skills](https://skilld.dev/skills/trending) ranks Skills by how many separate devs talked about each one this week.
Search the registry from a terminal:

```sh
npx skilld search vue
npx skilld trending
```

Every Skill page credits its author and links the source file on GitHub.
[Curators](https://skilld.dev/community) publish collections of the Skills they use.

### Preview it

Some Skills have demos.
Each demo is one recorded run: the prompt, and what the Agent built with the Skill.
Browse [every demo](https://skilld.dev/skills/demos), or open the Demo panel on a Skill page.

### Run it

Give your Agent a Skill URL. There is nothing to install:

> Use this Skill: https://skilld.dev/gh/antfu/skills/vue

Your Agent reads the page and follows it for that session.
Nothing is written to your project.
Add `.md` to any Skill URL for the raw `SKILL.md`.

From a terminal, `skilld run` does the same:

```sh
npx skilld run antfu/skills/vue
```

It prints `SKILL.md` to stdout and writes no file.
Pass the output to your Agent.

To change a Skill, ask your Agent to fork it:

> Fork this Skill: https://skilld.dev/gh/antfu/skills/vue

A fork creates an editable local Skill with its original author and license.

To keep a Skill in every session of one project, install it:

```sh
npx skilld install antfu/skills/vue
```

An install writes files. If an Agent runs the install, it asks you first.

| | `skilld run` | `skilld install` |
| --- | --- | --- |
| Use it for | The task in front of you | Every session in this project |
| Files written | None | `.skills`, the lockfile, and Agent targets |
| Cleanup | None | `skilld remove` |
| Updates | Loads the source each time | `skilld update` |
| Skill scripts | Never printed or executed | On disk for the Skill to use |

Start with `skilld run`. Install when you reach for the same Skill again.

### Watch for changes

Sign in, then watch the Repositories and collections you use:

```sh
npx skilld auth login
npx skilld watch antfu/skills
```

Each month, the digest email lists what changed. If nothing changed, skilld.dev sends nothing.
`skilld changes` prints the same changes from the last 30 days.
`skilld outdated` and `skilld update` keep your installed Skills current.

## Install the CLI

macOS and Linux:

```sh
curl -fsSL https://skilld.dev/install.sh | sh
```

Windows PowerShell:

```powershell
irm https://skilld.dev/install.ps1 | iex
```

npm, which needs Node.js:

```sh
npm install --global skilld
```

The script installs one native binary to `~/.skilld/bin`.
It adds that directory to PATH: in your shell profile on macOS and Linux, in your user PATH on Windows.
On macOS and Linux, set `SKILLD_NO_MODIFY_PATH=1` to leave your shell profile alone.

skilld checks for CLI upgrades in the background once a day.
If a newer release is cached, the next terminal invocation offers to upgrade.
Choose `Upgrade now`, `Not now`, or `Don't remind me for this version`.
The script install verifies the signed release manifest before replacing the executable.
Restart skilld to use the new version.
Recognized global npm, pnpm, Yarn, and Bun installs run their package manager after you confirm.
Local and transient installs show upgrade guidance instead.
After an upgrade, run your command again.
Set `SKILLD_NO_UPGRADE=1` to turn off upgrade checks.
See [SECURITY.md](./SECURITY.md#how-the-cli-upgrades-itself) for each check.

The npm package selects the same native executable for your system.
It carries no JavaScript engine and no fallback.

### Teach your Agent skilld

Install the [`skilld` Skill](https://skilld.dev/gh/skilld-dev/skilld/skilld) once for every project:

```sh
skilld install skilld --global
```

Your Agent then searches the registry and loads Skills on its own.
Ask for what you need, in your own words:

> Find a skilld Skill for Vue and use it

No terminal? Paste this into your Agent:

> Read https://skilld.dev/agent.md and follow it to set up skilld for me.

### Claude Code plugin

The plugin installs the skilld-maintained Skills and adds the skilld.dev MCP server.
Run these in Claude Code:

```sh
/plugin marketplace add skilld-dev/skilld
/plugin install skilld@skilld
```

The MCP server searches the registry and returns run and install commands. It never runs a Skill.
For ChatGPT, Claude, Codex, Cursor, and VS Code, see [skilld.dev/developers](https://skilld.dev/developers).

### Agent targets

Use `--agent` to name a target. Repeat it for several, or use `--agent all`.
`-g` is the short form of `--global`.

```sh
skilld install skilld -g --agent codex
skilld install skilld -g --agent kiro --agent zed
skilld install skilld -g --agent all
```

Some targets share a directory with an earlier target, such as `warp` and `pi` with `.agents/skills`.
Detection skips them, so name them with `--agent`.
Run `skilld install --help` for every Agent target value.

## How runs and installs work

### Selectors

`OWNER/REPOSITORY/SKILL` names one Skill in the registry.
`OWNER/REPOSITORY` names every Skill in one Repository.
`skilld search` prints the selector for each result.
In `skilld-dev/skills/find-skill`, `skilld-dev/skills` is the Repository and `find-skill` is the Skill directory inside it.
skilld 3.0 printed `skilld:OWNER/REPOSITORY/SKILL` and `gh:OWNER/REPOSITORY`. The CLI still accepts both.

### Repositories, curators, and collections

`skilld run` with a Repository, curator, or collection ref prints an index.
The index has one line per Skill with its run command. It loads no Skill.

`skilld add` installs the Skills the same ref names. It accepts the `skilld install` flags.
A normal terminal asks which Skills to install when several are listed.
Agents, pipes, CI, and `--plain` require `--all` when several Skills are listed.
One listed Skill installs without a picker. `add` does not support JSON output.

```sh
skilld add antfu/skills --all --agent codex
```

Success lists the installed agent paths, exact source commit, and source status.
Read the instructions before your agent uses a Skill.

A Repository that skilld.dev does not list yet falls back to its public GitHub tree.
Listing does not request registry indexing or wait for an index job.
The listed GitHub paths still use hosted delivery by default.
If delivery fails, skilld reports the failure and an explicit direct installation command where possible.
It never switches source paths automatically.

To choose direct installation for a public Repository:

```sh
skilld add antfu/skills --all --direct --agent codex
```

This command lists and reads files from GitHub without skilld.dev.
It records `unverified` and checks no Artifact attestation.
Behavior approval still applies. Curator and collection refs require hosted delivery.

`skilld add ./PATH` installs one local Skill. The path must contain its `SKILL.md`.
It does not search a local Repository for several Skills.

### Supporting files

skilld names the supporting files a Skill carries and prints none of them.
Read one when the instructions call for it:

```sh
skilld run skilld-dev/skills/find-skill --revision <commit> --file agents/openai.yaml
```

Use the revision and file-read command from the initial output.
skilld never prints executable or binary files.
A Skill that must run its own script needs an install.

### Skill behaviors

Every run lists what the Skill files ask an Agent to do, with the file and line.
skilld finds behaviors with the fixed text patterns in `packages/protocol/rules/skill-behaviors.json`.
Markdown that forbids a command does not count as that behavior: "Don't read: `id_rsa`", a table row marked **rejected**, or a list under an "Anti-patterns" heading.
Patterns miss obfuscated code, so an empty list proves nothing.

Five behaviors stop a remote `run`, `install`, or `add` until you approve them:

| id | Skill behavior |
| --- | --- |
| `remote-code` | Runs code downloaded from the network |
| `privilege` | Runs commands as root |
| `credentials` | Reads credential files or tokens |
| `destructive` | Deletes system or home directories |
| `hidden-text` | Contains invisible characters |

A terminal asks you directly.
An Agent, a pipe, or CI gets `BEHAVIOR_CONFIRMATION_REQUIRED` and the exact approval command:

```sh
skilld run 'github:OWNER/REPOSITORY/SKILL#commit:COMMIT' --allow remote-code
```

The command pins the commit that skilld checked.
A local or bundled Skill lists its behaviors and never stops.

A Skill that skilld.dev delivered can carry a model reading for each match:
what a language model read the line as, such as a quoted example in a security guide, and why.
The message adds it after the match, as `SKILL.md:7 (model reading: quoted example. REASON)`.
A model reading is no guarantee and changes no approval.

`install`, `add`, and `update` stop the same way and write nothing.
Run the same command again with the `--allow` ids the message names.
`add` installs every other Skill the ref names.
`update` stops only when the new version adds an ask behavior the installed copy lacks.
A lockfile restore and `skilld sync` install the commits already recorded, so they never stop.

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

# Sweep Skill files and review cleanup actions
skilld doctor
skilld doctor ./apps --json
skilld doctor --exclude '**/archive/**'

# Remove a Skill
skilld remove <skill>
```

Project installs update `.skills/skilld-lock.yaml` and the selected Agent targets.
Use `--global` (or `-g`) for account-level Agent targets.
Use `--mode copy` or `--mode symlink` to control target writes.

### Review Skill files

`skilld doctor` scans your home directory and opens a terminal UI.
Pass roots to narrow the scan. Use `--json` or `--plain` for a read-only report.
The scan includes hidden `.claude/skills` directories and lists `CLAUDE.md` and `.claude/commands` files separately.

Default exclude globs prune dependencies, builds, caches, plugin staging, sessions, backups, and fixtures before reading their children.
Git worktrees are skipped beneath scan roots.
Use `--include-excluded` or `--include-worktrees` when you need those files.
Repeat `--exclude GLOB` to add exclusions. Git internals remain excluded.
Source and plugin files retain their owners.

Press Tab to show skills.sh installs separately. Choose a Skill, then press `m` to migrate or `d` to remove.
Review affected Agent targets before pressing Enter. Escape cancels.
Migration keeps the scope and copy or link mode of each observed target.
It verifies source contents, preserves recorded branches, and requests approval for Skill behaviors.
When the recorded Git tree matches the installed files, migration can also replace them with the displayed source commit.
If provenance cannot establish the replacement, migration stops before changing files.

Removal backs up the selected targets and removes only their skills.sh lockfile entry.
Original targets, lock metadata, and `recovery.json` remain under `.skilld-doctor-backups` beside the skilld store.
Unknown Agent targets can also be removed after review.
Declared skilld installs, plugin files, and source directories require their existing owner's workflow.

`--check-sources` checks up to 20 source candidates through skilld.dev.
An exact directory match includes supporting files and executable modes.
A name match alone never establishes provenance.
The report retains scan problems. Exit code 1 means some files could not be checked.
Historical commit searches and project-to-global deduplication are outside this command's current actions.

### Read the registry

These commands read skilld.dev and need no account:

```sh
# One Skill, Repository, curator, or collection, with its provenance
skilld view vercel-labs/agent-skills/web-design-guidelines
skilld view vercel-labs/agent-skills
skilld view @harlan-zw
skilld view @harlan-zw/design-engineering-essentials

# Browse by owner, tag, and order
skilld browse --owner vercel-labs --sort likes

# What devs talk about this week, and why each Skill trends
skilld trending

# Tracks, and the Skills of one track
skilld tracks
skilld tracks design

# The curators who publish collections
skilld curators

# Ask skilld.dev to index a Repository, then wait for its Skills
skilld index vercel-labs/agent-skills
```

`skilld view` with a bare name still shows an installed Skill.
A name with `/` or `@` reads the registry instead.

### Use your account

These commands act for your skilld.dev account. Run `skilld auth login` first.

```sh
# Settings: email, digest, weekly, likes-public, repository-indexing
skilld account
skilld account set digest off

# Like Skills. A like also watches the Repository for your digest.
skilld like vercel-labs/agent-skills/web-design-guidelines
skilld unlike vercel-labs/agent-skills/web-design-guidelines
skilld likes
skilld likes @harlan-zw

# Watch Repositories and collections, then read what changed
skilld watch vercel-labs/agent-skills
skilld watch @harlan-zw/design-engineering-essentials
skilld unwatch vercel-labs/agent-skills
skilld watches
skilld changes --since 2026-09-01

# Your GitHub stars that hold Skills
skilld stars import
skilld stars

# Build a collection
skilld collection create picks --title "My picks"
skilld collection add @you/picks vercel-labs/agent-skills/web-design-guidelines --reason "Catches UI mistakes"
skilld collection remove @you/picks vercel-labs/agent-skills/web-design-guidelines

# Index or unpublish your own Repositories
skilld account scan
skilld account unpublish you/skills

# Tokens for CI
skilld tokens
skilld tokens create --label "CI deploy" --ttl-days 90
skilld tokens revoke 412
```

`skilld tokens create` prints the token once. skilld.dev never shows it again.
Account deletion needs skilld.dev in a browser.

### JSON output

Use `--json` for stable output with `search`, `run`, `update --check`, `view` of a registry ref, and every command in the two sections above.
The `data` field of each answer is the skilld.dev API answer as skilld.dev sent it.
A command whose API answer has no body returns `"data": null`.

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
- [`update-package-skill`](./skills/update-package-skill): update that Skill after a release, testing only what changed
- [`generate-project-skill`](./skills/generate-project-skill): draft a Skill from a project you maintain
- [`review-skill`](./skills/review-skill): review a Skill before you publish it

When you run these Skills directly, you see every instruction and review every change.

## Artifact delivery

The skilld CLI resolves remote Skills through the skilld.dev API.
GitHub remains the source of truth.

skilld.dev builds an immutable Artifact from an exact Git commit.
The CLI checks its digest, Artifact attestation, check results, and archive before installation.
The CLI stops pending Artifact creation after at most 60 seconds.

Private Repository delivery requires both:

- `skilld auth login` for a skilld.dev account
- Access through the skilld GitHub App installation

Private Artifact responses use short-lived, one-time grants.
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

A script or a CI job without a browser can send a skilld token instead.
Create one at skilld.dev/me/cli-tokens/new or with `skilld tokens create`, then set `SKILLD_TOKEN`.
The CLI reads it for that run only, and never stores or refreshes it.

### Environment variables

| Variable | Effect |
| --- | --- |
| `SKILLD_DATA_DIR` | The directory for global Skills and configuration. The default is `~/.skilld`, or `%LOCALAPPDATA%\skilld` on Windows. |
| `SKILLD_API_URL` | The skilld.dev origin the CLI talks to. The default is `https://skilld.dev`. |
| `SKILLD_TOKEN` | A skilld token to send instead of the stored sign-in. For scripts and CI. |
| `SKILLD_NO_UPGRADE` | Set to `1` to turn off upgrade checks. |
| `SKILLD_NO_WEEKLY` | Set to `1` to turn off the note about the weekly email. |

Use `SKILLD_API_URL` to test a local or preview site:

```sh
SKILLD_API_URL=http://localhost:3000 skilld search vue
```

It accepts an HTTPS origin, or an HTTP origin on `localhost` or `127.0.0.1`.
Search, Skill delivery, sign-in, and account commands all use that origin.
Each origin keeps its own sign-in, so a skilld.dev token never goes to another origin.

## Privacy

The skilld CLI sends no telemetry or analytics.
It makes network requests only for these reasons:

- `skilld search`, `skilld run`, and `skilld install` of a hosted Skill call the skilld.dev API.
- If you signed in, those requests carry your account token.
- The registry commands, such as `skilld browse` and `skilld trending`, call the skilld.dev API without your token.
- The account commands, such as `skilld like` and `skilld watch`, call the skilld.dev API with your token.
- `skilld auth login` sends your computer hostname to name the sign-in. Only you see it on skilld.dev.
- `--direct` fetches public Skills from GitHub.
- `skilld update` compares commits through skilld.dev or the GitHub API.
- At a terminal, the CLI checks GitHub Releases or the npm registry for a new version once a day.

The upgrade check never runs in CI or inside an Agent.
Set `SKILLD_NO_UPGRADE=1` to turn it off.

If you are signed out, a terminal shows a short note about the weekly email.
It prints to stderr at most three times, a week apart, and sends nothing.
It never prints in CI, inside an Agent, or when output is piped.
Set `SKILLD_NO_WEEKLY=1` to turn it off.

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
`packages/protocol` owns the Artifact delivery wire contract.
`packages/sdk` owns the public API contract and publishes as `skilld-sdk`.
`skills` owns the visible skilld-maintained Skills.

The WASIp2 build remains an internal proof.
Published packages use native executables only.

## License

[MIT](./LICENSE)
