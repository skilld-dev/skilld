import type { AgentType, OptimizeModel } from '../../agent/index.ts'
import { defineCommand } from 'citty'
import { promptForAgent, resolveAgent } from '../../cli/agent-prompt.ts'
import { sharedArgs } from '../../cli/args.ts'
import { hasCompletedWizard } from '../../core/config.ts'
import { parseSkillInput } from '../../core/prefix.ts'
import { COMMA_OR_WHITESPACE_RE } from '../../core/regex.ts'
import { runWizard } from '../wizard.ts'
import { installSkills } from './install-many.ts'
import { exportPortablePrompts } from './portable.ts'

export const addCommandDef = defineCommand({
  meta: { name: 'add', description: 'Install skills (npm:<pkg>, crate:<name>, gh:<owner/repo>, @<curator>)' },
  args: {
    'package': {
      type: 'positional',
      description: 'Package(s) to sync (space/comma-separated; npm:<pkg>, crate:<name>, or owner/repo)',
      required: true,
    },
    'skill': {
      type: 'string',
      alias: 's',
      description: 'Select specific skills from a git repo (comma-separated)',
      valueHint: 'name',
    },
    'allow-unsafe': {
      type: 'boolean',
      description: 'Install skills that fail the upstream audit gate',
    },
    ...sharedArgs,
  },
  async run({ args }) {
    let agent: AgentType | 'none' | null = resolveAgent(args.agent)
    if (!agent) {
      agent = await promptForAgent()
      if (!agent)
        return
    }

    const rawInputs = [...new Set(
      [args.package, ...((args as any)._ || [])]
        .map((s: string) => s.trim())
        .filter(Boolean),
    )]

    if (agent === 'none') {
      const packages = [...new Set(rawInputs.flatMap(s => s.split(COMMA_OR_WHITESPACE_RE)).map(s => s.trim()).filter(Boolean))]
      for (const pkg of packages)
        await exportPortablePrompts(pkg, { force: args.force, agent: 'none' })
      return
    }

    if (!hasCompletedWizard())
      await runWizard({ agent })

    const items = rawInputs.map(parseSkillInput)
    await installSkills(items, {
      agent,
      surface: 'cli:add',
      global: args.global,
      yes: args.yes,
      force: args.force,
      debug: args.debug,
      model: args.model as OptimizeModel | undefined,
      skillFilter: args.skill,
      allowUnsafe: args['allow-unsafe'],
    })
  },
})
