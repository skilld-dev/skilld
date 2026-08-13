import type { AgentType, OptimizeModel } from '../../agent/index.ts'
import * as p from '@clack/prompts'
import { defineCommand } from 'citty'
import { autoResolveAgent } from '../../cli/agent-prompt.ts'
import { agentOrNoneArg, sharedArgs } from '../../cli/args.ts'
import { hasCompletedWizard } from '../../core/config.ts'
import { parseSkillInput } from '../../core/prefix.ts'
import { COMMA_OR_WHITESPACE_RE } from '../../core/regex.ts'
import { watchRepositories } from '../watch.ts'
import { runWizard } from '../wizard.ts'
import { installSkills } from './install-many.ts'
import { exportPortablePrompts } from './portable.ts'

export const addCommandDef = defineCommand({
  meta: { name: 'add', description: 'Install skills from packages, repositories, curators, or collections' },
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
    'watch': {
      type: 'boolean',
      description: 'Watch installed repositories for changes',
    },
    ...sharedArgs,
    // This command supports portable exports.
    'agent': agentOrNoneArg,
  },
  async run({ args }) {
    const rawInputs = [...new Set(
      [args.package, ...((args as any)._ || [])]
        .map((s: string) => s.trim())
        .filter(Boolean),
    )]
    const items = rawInputs.map(parseSkillInput)

    // --agent none → portable export (no installed-agent target needed).
    if (args.agent === 'none') {
      if (items.some(item => item.type === 'curator')) {
        p.log.error('Curator installs require a target agent.')
        process.exitCode = 1
        return
      }
      const packages = [...new Set(rawInputs.flatMap(s => s.split(COMMA_OR_WHITESPACE_RE)).map(s => s.trim()).filter(Boolean))]
      for (const pkg of packages)
        await exportPortablePrompts(pkg, { force: args.force, agent: 'none' })
      return
    }

    const agent: AgentType | null = autoResolveAgent(args.agent)
    if (!agent) {
      p.log.error('No target agent detected.\n  Pass --agent <name> (claude-code, cursor, codex, …) or run `skilld config` to set a default.\n  Use --agent none for portable export.')
      process.exitCode = 1
      return
    }

    if (!hasCompletedWizard())
      await runWizard({ agent })

    const summary = await installSkills(items, {
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
    if (args.watch) {
      if (summary.repositories.length === 0) {
        p.log.warn('No GitHub repositories were resolved to watch.')
        return
      }
      await watchRepositories(summary.repositories).then((result) => {
        if (result._tag === 'AuthRequired') {
          p.log.error('Skills installed. Run `skilld login`, then `skilld watch`.')
          process.exitCode = 1
          return
        }
        p.log.success(`Watching ${summary.repositories.length} repositories. ${result.inserted} added.`)
      }).catch((error) => {
        p.log.error(`Skills installed, but watch failed: ${error instanceof Error ? error.message : String(error)}`)
        process.exitCode = 1
      })
    }
  },
})
