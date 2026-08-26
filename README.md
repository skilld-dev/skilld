<h1><a href="https://skilld.dev"><img src=".github/logos/logo-mark.svg" alt="" width="28" height="28" valign="middle"></a> skilld</h1>

[![npm version](https://img.shields.io/npm/v/skilld?color=yellow)](https://npmjs.com/package/skilld)
[![npm downloads](https://img.shields.io/npm/dm/skilld?color=yellow)](https://npm.chart.dev/skilld)
[![license](https://img.shields.io/npm/l/skilld?color=yellow)](https://github.com/skilld-dev/skilld/blob/main/LICENSE)

Search, install, and keep Agent Skills current.

skilld v3 has two products:

- The native `skilld` CLI manages Skills.
- The JavaScript [`skilld-harness`](./packages/harness) package runs visible Skill generation and review instructions.

The skilld CLI contains no Skill generation logic or Agent runtime.

## Install the skilld CLI

```sh
npm install --global skilld
```

The npm package selects a native executable for the current system.
It has no JavaScript CLI engine or JavaScript fallback.

Install the skilld-maintained Skill for your Agent:

```sh
skilld install skilld --global
```

Use `--agent` when you want an explicit Agent target:

```sh
skilld install skilld --global --agent codex
```

## Run a Skill without installing it

`skilld run` is the default way to use a Skill.
It prints SKILL.md so your Agent follows it now.

```sh
npx skilld run skilld:skilld-dev/skills/vue
```

A remote run writes no Skill files.
It creates no lockfile entry, Agent target, project file, or Skill cache.
The command retains no Skill files after it exits.

skilld names the supporting files a Skill carries and prints none of them.
Read one when the instructions call for it:

```sh
npx skilld run skilld:skilld-dev/skills/vue --revision <commit> --file references/api.md
```

Use the revision and file-read command from the initial output.
skilld never prints executable or binary files.
A Skill that must run its own script needs an install.

Install the Skill when you want it in every session:

```sh
skilld install skilld:skilld-dev/skills/vue
```

An install writes files. Ask the user first.

## Use the skilld CLI

```sh
# Find a Skill
skilld search vue

# Run a Skill for this session only
skilld run skilld:skilld-dev/skills/vue

# Read one supporting file that Skill carries
skilld run skilld:skilld-dev/skills/vue --file references/api.md

# Install a Skill in the current project
skilld install skilld:skilld-dev/skills/vue

# Inspect installed Skills
skilld list
skilld view vue

# Keep Skills current
skilld verify vue
skilld update vue

# Check update relations for an Agent or CI
skilld update --check --json

# Remove a Skill
skilld remove vue
```

Project installs update `.skills/skilld-lock.yaml` and the selected Agent targets.
Use `--global` for account level Agent targets.
Use `--mode copy` or `--mode symlink` to control target writes.

Run `skilld install` without a source to restore the lockfile state.

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

## Generate or review a Skill

Skill generation lives outside the skilld CLI.

Use these skilld-maintained Skills directly with your Agent:

- [`generate-package-skill`](./skills/generate-package-skill)
- [`generate-project-skill`](./skills/generate-project-skill)
- [`review-skill`](./skills/review-skill)
- [`skilld`](./skills/skilld)

Direct Skill runs remain user reviewed.
The instructions and changes stay visible to the user.

Use [`skilld-harness`](./packages/harness) when an application or CI needs strict output checks.
The Harness runs the same visible Skill files through an AI SDK Harness.

```sh
pnpm add skilld-harness @ai-sdk/harness ws zod
```

```ts
import { createSkillHarness } from 'skilld-harness'

const skillHarness = createSkillHarness({ harness, sandbox })

const result = await skillHarness.run({
  _tag: 'PackageSkill',
  source: { _tag: 'NpmPackage', spec: 'vue' },
  destination: { rootDir: '.agents/skills', name: 'vue' },
})
```

See the [`skilld-harness` guide](./packages/harness/README.md) for its full contract.

## Upgrade from v2

v3 does not import v2 configuration or lockfiles.
Back up v2 state before replacing the CLI.

Follow the [v2 to v3 migration guide](./docs/migrate-v2-to-v3.md).
It maps removed commands and explains rollback limits.

## v3 development

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
