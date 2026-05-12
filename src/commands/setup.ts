import type { AgentType } from '../agent/index.ts'
import { defineCommand } from 'citty'
import { resolveAgent } from '../cli/agent-prompt.ts'
import { sharedArgs } from '../cli/args.ts'
import { runWizard } from './wizard.ts'

export const setupCommandDef = defineCommand({
  meta: {
    name: 'setup',
    description: 'Re-run the setup wizard to configure features and model',
  },
  args: {
    agent: sharedArgs.agent,
  },
  async run({ args }) {
    const agent = resolveAgent(args.agent)
    await runWizard({
      agent: agent && agent !== 'none' ? agent as AgentType : undefined,
    })
  },
})
