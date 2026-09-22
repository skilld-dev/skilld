# skilld-harness

Run visible skilld-maintained Skills through an AI SDK Harness.

The package checks output limits, paths, file types, and Skill frontmatter.
It then promotes generated Skills with an atomic directory rename.

## Install

```sh
pnpm add skilld-harness @ai-sdk/harness ws zod
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
An installed Skill can include cleanup warnings after promotion.

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

| Option | Default | Effect |
| --- | --- | --- |
| `root` | a new temporary directory | Session root directory. |
| `port` | a free port from the operating system | Bridge port. |
| `keepRoot` | `false` | Keep the session root after `destroy`. |

The local sandbox needs POSIX `sh` at `/bin/sh` and GNU `find`, because the
Harness inventories output with `find -printf`. It runs on Linux. It does not
support macOS, whose `find` has no `-printf`, or Windows.

A Skill that carries a large reference tree will exceed the default output
policy, which stops at 64 files. Raise it on `createSkillHarness`:

```ts
createSkillHarness({ harness, sandbox, outputPolicy: { maxOutputFiles: 512 } })
```

**It applies no isolation.** Every process reaches the whole computer and the
caller's environment. Use it for your own Skills on your own computer or on a
self-hosted runner. Use a hosted sandbox provider when the Harness must contain
what it runs.

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
