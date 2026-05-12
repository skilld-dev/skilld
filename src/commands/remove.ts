import type { AgentType } from '../agent/index.ts'
import type { ProjectState, SkillEntry } from '../core/skills.ts'
import { existsSync, rmSync } from 'node:fs'
import * as p from '@clack/prompts'
import { defineCommand } from 'citty'
import { unlinkSkillFromAgents } from '../agent/index.ts'
import { promptForAgent, resolveAgent } from '../cli/agent-prompt.ts'
import { sharedArgs } from '../cli/args.ts'
import { isInteractive } from '../cli/env.ts'
import { getInstalledGenerators, introLine } from '../cli/intro.ts'
import { readConfig } from '../core/config.ts'
import { removeLockEntry } from '../core/lockfile.ts'
import { getSharedSkillsDir } from '../core/paths.ts'
import { resolveSkillName } from '../core/prefix.ts'
import { getProjectState, getSkillsDir, iterateSkills } from '../core/skills.ts'

export interface RemoveOptions {
  packages?: string[]
  global: boolean
  agent: AgentType
  yes: boolean
}

export async function removeCommand(state: ProjectState, opts: RemoveOptions): Promise<void> {
  // Get skills from the appropriate scope
  const scope = opts.global ? 'global' : 'local'
  const allSkills = [...iterateSkills({ scope })]

  // Non-interactive without packages → error
  if (!isInteractive() && !opts.packages) {
    console.error('Error: `skilld remove` requires package names in non-interactive mode.\n  Usage: skilld remove <package...>')
    process.exit(1)
  }

  // Get skills to choose from
  const skills = opts.packages
    ? allSkills.filter(s => opts.packages!.includes(s.name))
    : await pickSkillsToRemove(allSkills, scope)

  if (!skills || skills.length === 0) {
    p.log.info('No skills selected')
    return
  }

  // Confirm deletion (skip in non-interactive)
  if (!opts.yes && isInteractive()) {
    const confirmed = await p.confirm({
      message: `Remove ${skills.length} skill(s)? ${skills.map(s => s.name).join(', ')}`,
    })

    if (p.isCancel(confirmed) || !confirmed) {
      p.cancel('Cancelled')
      return
    }
  }

  // Delete each skill
  const cwd = process.cwd()
  const shared = getSharedSkillsDir(cwd)
  for (const skill of skills) {
    const skillsDir = getSkillsDir(skill.agent, skill.scope)

    if (existsSync(skill.dir)) {
      rmSync(skill.dir, { recursive: true, force: true })
      removeLockEntry(skillsDir, skill.name)
      // Clean up per-agent symlinks when removing from shared dir
      if (shared && skill.scope === 'local')
        unlinkSkillFromAgents(skill.name, cwd, opts.agent)
      p.log.success(`Removed ${skill.name}`)
    }
    else {
      p.log.warn(`${skill.name} not found`)
    }
  }

  p.outro(`Removed ${skills.length} skill(s)`)
}

async function pickSkillsToRemove(skills: SkillEntry[], scope: 'local' | 'global'): Promise<SkillEntry[] | null> {
  if (skills.length === 0) {
    p.log.warn(`No ${scope} skills installed`)
    return null
  }

  const options = skills.map(skill => ({
    label: skill.name,
    value: skill.name,
    hint: skill.info?.version ? `@${skill.info.version}` : undefined,
  }))

  const selected = await p.multiselect({
    message: 'Select skills to remove',
    options,
    required: false,
  })

  if (p.isCancel(selected)) {
    p.cancel('Cancelled')
    return null
  }

  const selectedSet = new Set(selected as string[])
  return skills.filter(s => selectedSet.has(s.name))
}

export const removeCommandDef = defineCommand({
  meta: { name: 'remove', description: 'Remove installed skills' },
  args: {
    package: {
      type: 'positional',
      description: 'Package(s) to remove (space-separated)',
      required: false,
    },
    ...sharedArgs,
  },
  async run({ args }) {
    const cwd = process.cwd()
    let agent = resolveAgent(args.agent)
    if (!agent || agent === 'none') {
      if (agent === 'none')
        return
      const picked = await promptForAgent()
      if (!picked || picked === 'none')
        return
      agent = picked
    }

    const state = await getProjectState(cwd)
    const generators = getInstalledGenerators()
    const config = readConfig()
    const scope = args.global ? 'global' : 'project'
    const intro = { state, generators, modelId: config.model, agentId: agent || config.agent || undefined }
    p.intro(`${introLine(intro)} · remove (${scope})`)

    // Collect packages from positional args (strip npm:/gh: prefixes)
    const packages = args.package
      ? [...new Set(
          [args.package, ...((args as any)._ || [])]
            .map((s: string) => s.trim())
            .filter(Boolean)
            .map((s) => {
              const name = resolveSkillName(s)
              if (!name) {
                p.log.warn(`Cannot remove \x1B[36m${s}\x1B[0m: curator/collection inputs are not addressable here.`)
                return null
              }
              return name
            })
            .filter((s): s is string => s !== null),
        )]
      : undefined

    return removeCommand(state, {
      packages,
      global: args.global,
      agent,
      yes: args.yes,
    })
  },
})
