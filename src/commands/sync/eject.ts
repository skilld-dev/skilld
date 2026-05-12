import type { AgentType, OptimizeModel } from '../../agent/index.ts'
import * as p from '@clack/prompts'
import { defineCommand } from 'citty'
import { resolveAgent } from '../../cli/agent-prompt.ts'
import { sharedArgs } from '../../cli/args.ts'
import { introLine } from '../../cli/intro.ts'
import { hasCompletedWizard } from '../../core/config.ts'
import { getProjectState } from '../../core/skills.ts'
import { syncCommand } from '../sync.ts'
import { runWizard } from '../wizard.ts'

export const ejectCommandDef = defineCommand({
  meta: { name: 'eject', description: 'Eject skill with references as real files (portable, no symlinks)' },
  args: {
    package: {
      type: 'positional',
      description: 'Package to eject',
      required: true,
    },
    name: {
      type: 'string',
      alias: 'n',
      description: 'Custom skill directory name (default: derived from package)',
    },
    out: {
      type: 'string',
      alias: 'o',
      description: 'Output directory path override',
    },
    from: {
      type: 'string',
      description: 'Collect releases/issues/discussions from this date onward (YYYY-MM-DD)',
    },
    search: {
      type: 'boolean',
      description: 'Build search index / embeddings (use --no-search to skip)',
      default: true,
    },
    ...sharedArgs,
  },
  async run({ args }) {
    const cwd = process.cwd()
    const resolved = resolveAgent(args.agent)
    const agent: AgentType = resolved && resolved !== 'none' ? resolved : 'claude-code'

    if (!hasCompletedWizard())
      await runWizard({ agent })

    const state = await getProjectState(cwd)
    p.intro(introLine({ state, agentId: agent || undefined }))
    return syncCommand(state, {
      packages: [args.package],
      global: args.global,
      agent,
      model: args.model as OptimizeModel | undefined,
      yes: args.yes,
      force: args.force,
      debug: args.debug,
      eject: args.out || true,
      name: args.name,
      from: args.from,
      noSearch: !args.search,
    })
  },
})
