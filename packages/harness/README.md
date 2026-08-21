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
