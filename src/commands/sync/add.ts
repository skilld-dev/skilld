import type { AgentType, OptimizeModel } from '../../agent/index.ts'
import type { GitSkillSource } from '../../sources/git-skills.ts'
import * as p from '@clack/prompts'
import { defineCommand } from 'citty'
import { promptForAgent, resolveAgent } from '../../cli/agent-prompt.ts'
import { sharedArgs } from '../../cli/args.ts'
import { introLine } from '../../cli/intro.ts'
import { hasCompletedWizard } from '../../core/config.ts'
import { parseSkillInput } from '../../core/prefix.ts'
import { getProjectState } from '../../core/skills.ts'
import { syncGitSkills } from '../sync-git.ts'
import { syncCommand } from '../sync.ts'
import { runWizard } from '../wizard.ts'
import { exportPortablePrompts } from './portable.ts'

export const addCommandDef = defineCommand({
  meta: { name: 'add', description: 'Install skills (npm:<pkg>, crate:<name>, gh:<owner/repo>, @<curator>)' },
  args: {
    package: {
      type: 'positional',
      description: 'Package(s) to sync (space/comma-separated; npm:<pkg>, crate:<name>, or owner/repo)',
      required: true,
    },
    skill: {
      type: 'string',
      alias: 's',
      description: 'Select specific skills from a git repo (comma-separated)',
      valueHint: 'name',
    },
    ...sharedArgs,
  },
  async run({ args }) {
    const cwd = process.cwd()
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
      const packages = [...new Set(rawInputs.flatMap(s => s.split(/[,\s]+/)).map(s => s.trim()).filter(Boolean))]
      for (const pkg of packages)
        await exportPortablePrompts(pkg, { force: args.force, agent: 'none' })
      return
    }

    if (!hasCompletedWizard())
      await runWizard({ agent })

    const parsedSources = rawInputs.map(parseSkillInput)
    const gitSources: GitSkillSource[] = []
    const npmEntries: Array<{ name: string, spec: string }> = []
    const crateSpecs: string[] = []
    const unsupported: string[] = []

    for (const source of parsedSources) {
      switch (source.type) {
        case 'git':
          gitSources.push(source.source)
          break
        case 'npm':
          npmEntries.push({ name: source.package, spec: source.tag ? `${source.package}@${source.tag}` : source.package })
          break
        case 'crate':
          crateSpecs.push(source.version ? `crate:${source.package}@${source.version}` : `crate:${source.package}`)
          break
        case 'bare':
          p.log.warn(`Bare names are deprecated. Use \x1B[36mnpm:${source.package}\x1B[0m instead.`)
          npmEntries.push({ name: source.package, spec: source.tag ? `${source.package}@${source.tag}` : source.package })
          break
        case 'curator':
          unsupported.push(`@${source.handle} (curator)`)
          break
        case 'collection':
          unsupported.push(`@${source.handle}/${source.name} (collection)`)
          break
        default: {
          const _exhaustive: never = source
          throw new Error(`Unhandled SkillSource type: ${JSON.stringify(_exhaustive)}`)
        }
      }
    }

    if (unsupported.length > 0) {
      p.log.error(`Curator and collection installs are not yet available:\n  ${unsupported.join('\n  ')}\n\nFollow https://skilld.dev for launch updates.`)
      process.exitCode = 1
      if (gitSources.length === 0 && npmEntries.length === 0 && crateSpecs.length === 0)
        return
    }

    if (gitSources.length > 0) {
      for (const source of gitSources) {
        const skillFilter = args.skill ? args.skill.split(/[,\s]+/).map((s: string) => s.trim()).filter(Boolean) : undefined
        await syncGitSkills({ source, global: args.global, agent, yes: args.yes, model: args.model as OptimizeModel | undefined, force: args.force, debug: args.debug, skillFilter })
      }
    }

    if (npmEntries.length > 0) {
      const { syncRegistrySkill } = await import('../sync-registry.ts')
      const seen = new Set<string>()
      const dedupedEntries = npmEntries.filter((e) => {
        if (seen.has(e.name))
          return false
        seen.add(e.name)
        return true
      })

      const fallbackPackages: string[] = []
      for (const entry of dedupedEntries) {
        const result = await syncRegistrySkill({ packageName: entry.name, agent, cwd })
        if (result) {
          p.log.success(`Installed \x1B[36m${result.name}\x1B[0m from registry`)
        }
        else {
          fallbackPackages.push(entry.spec)
        }
      }

      if (fallbackPackages.length > 0) {
        const state = await getProjectState(cwd)
        p.intro(introLine({ state, agentId: agent || undefined }))
        await syncCommand(state, {
          packages: [...fallbackPackages, ...crateSpecs],
          global: args.global,
          agent,
          model: args.model as OptimizeModel | undefined,
          yes: args.yes,
          force: args.force,
          debug: args.debug,
        })
        return
      }
    }

    if (crateSpecs.length > 0) {
      const state = await getProjectState(cwd)
      p.intro(introLine({ state, agentId: agent || undefined }))
      await syncCommand(state, {
        packages: crateSpecs,
        global: args.global,
        agent,
        model: args.model as OptimizeModel | undefined,
        yes: args.yes,
        force: args.force,
        debug: args.debug,
      })
    }
  },
})
