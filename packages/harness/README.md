# skilld-harness

Run visible skilld-maintained Skills through an AI SDK Harness.

The package checks output limits, paths, file types, and Skill frontmatter.
It then promotes generated Skills with an atomic directory rename.

## Install

```sh
pnpm add skilld-harness @ai-sdk/harness ws zod
```

Add one Harness adapter. The examples below use OpenCode:

```sh
pnpm add @ai-sdk/harness-opencode
```

Use Node 22 or newer.
The sandbox must provide POSIX `sh`, `rm`, `mkdir`, and GNU `find`.

## Run a Skill

```ts
import { createSkillHarness } from 'skilld-harness'

const skillHarness = createSkillHarness({
  harness,
  sandbox,
})

const result = await skillHarness.run({
  _tag: 'ProjectSkill',
  projectDir: process.cwd(),
  destination: {
    rootDir: '.skills',
    name: 'my-project',
  },
})
```

Every run returns a tagged `Ok` or `Err` value.
The input tag sets the `Ok` value: `PackageSkill` and `ProjectSkill` return a `GeneratedSkill`, and `ReviewSkill` returns a `SkillReview`.
An `Ok` value carries `warnings`.
They name each source file the Harness left out for size, and any cleanup problem after promotion.
An `Ok` value also carries `usage` (input, cached input, and output tokens) and `steps`, the number of model calls.
A token count is `undefined` when the adapter does not report it.
An `InvalidSkill` error lists each failed output check in `issues`.

The Harness checks the rules that the generation Skills state:

- The frontmatter contains only `name` and `description`.
- `SKILL.md` links every file under `references/`, by a Markdown link or an inline code path.
- A package Skill keeps `SKILL.md` under 500 lines and writes at most eight reference files.
- A project Skill points only at project files and gives at least one search command.

A run can take several minutes. Pass `onEvent` to follow the Agent:

```ts
import type { SkillRun } from 'skilld-harness'
import { createSkillHarness } from 'skilld-harness'

declare const skillHarness: ReturnType<typeof createSkillHarness>
declare const input: SkillRun

await skillHarness.run(input, {
  onEvent: (event) => {
    if (event._tag === 'ToolCall')
      console.log(`step ${event.step}: ${event.toolName}`)
    if (event._tag === 'StepFinish')
      console.log(`step ${event.step} used ${event.usage.outputTokens} output tokens`)
  },
})
```

The events are `StepStart`, `ToolCall`, and `StepFinish`. Steps count from 0.
Some adapters report tool calls only when their step ends.
If `onEvent` throws, the run continues, and the `Ok` value carries the error as a warning.

Pass `fetch` to `createSkillHarness` when the host owns HTTP access.
The default adapter uses the Node global fetch implementation.

## Local sandbox

`createSkillHarness` needs a sandbox. Import `skilld-harness/sandbox-local` to
run a Skill on the computer that starts it, with no hosted sandbox account.

```ts
import { createOpenCode } from '@ai-sdk/harness-opencode'
import { createSkillHarness } from 'skilld-harness'
import { createLocalSandbox } from 'skilld-harness/sandbox-local'

const skillHarness = createSkillHarness({
  harness: createOpenCode({
    provider: 'opencode-go',
    openCodeConfig: { agent: { general: { model: 'opencode-go/glm-5.3-flash' } } },
  }),
  sandbox: createLocalSandbox(),
})
```

The session runs in a new temporary directory and exposes one port on
`127.0.0.1` for bridge-backed Harness adapters. `destroy` kills every process
the session started and removes the directory.

Each session gets its own `HOME`, XDG directories, and `TMPDIR` under the
session root. Harness state, the adapter bootstrap, and installed Skills stay
inside the session, and `destroy` removes them. The Agent does not load your own
agent configuration, Skills, or MCP servers. Configure the adapter explicitly,
for example through `openCodeConfig`.

Processes receive a minimal environment: `PATH`, locale, terminal, and proxy
variables. Pass any other variable through the `env` option. The adapter reads
its credentials from your environment and hands them to the processes it starts.
For OpenCode Go, set `OPENCODE_API_KEY`. The adapter does not read the key that
`opencode auth login` stores.

Each session installs the adapter bootstrap again, because nothing persists
between sessions. With OpenCode, that is about 200 MB of downloads per run.

| Option | Default | Effect |
| --- | --- | --- |
| `root` | a new temporary directory | Session root directory. |
| `port` | a free port from the operating system | Bridge port. |
| `keepRoot` | `false` | Keep the session root after `destroy`. |
| `env` | none | More environment variables for every process. |

The adapter prints this warning on every local run:
"credential brokering does not work. Falling back to less secure credential forwarding."
It is expected. The local sandbox cannot rewrite requests, so the adapter
gives the raw API key to the processes it starts. If the Agent must not see the key,
use a hosted sandbox provider that supports request transformations.

The local sandbox needs POSIX `sh` at `/bin/sh` and GNU `find`, because the
Harness inventories output with `find -printf`. It runs on Linux. It does not
support macOS, whose `find` has no `-printf`, or Windows.

A Skill that carries a large reference tree will exceed the default output
policy, which stops at 64 files. Raise it on `createSkillHarness`:

```ts
createSkillHarness({ harness, sandbox, outputPolicy: { maxOutputFiles: 512 } })
```

**It applies no process isolation.** A separate `HOME` keeps state apart. It does
not stop a process from reading or writing any file your user account can reach.
Use it for your own Skills on your own computer or on a self-hosted runner. Use a
hosted sandbox provider when the Harness must contain what it runs.

A `LocalPackage` source copies the working tree, including uncommitted and
untracked files. A symbolic link anywhere in it stops the run, and the error
names the link. To use only committed files, pass a `git archive` export.

## Visible Skills

Import `skilld-harness/skills` to load the published Skill instructions.

```ts
import {
  loadSkilldMaintainedSkill,
  skilldMaintainedSkillNames,
} from 'skilld-harness/skills'

const names = await skilldMaintainedSkillNames()
const skill = await loadSkilldMaintainedSkill('generate-project-skill')
```

The same Skill files remain usable directly through an Agent.
Direct runs remain user reviewed.

## Develop against the local package

`package.json` exports `dist` files. Run `pnpm build` in `packages/harness`
before you link the package into another project. A stale `dist` fails on import.
