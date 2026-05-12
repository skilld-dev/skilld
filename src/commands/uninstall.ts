import type { AgentType } from '../agent/index.ts'
import { existsSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import * as p from '@clack/prompts'
import { defineCommand } from 'citty'
import { join } from 'pathe'
import { agents } from '../agent/index.ts'
import { SKILLD_MARKER_END, SKILLD_MARKER_START } from '../agent/skill-installer.ts'
import { CACHE_DIR } from '../cache/index.ts'
import { sharedArgs } from '../cli/args.ts'
import { isInteractive } from '../cli/env.ts'
import { getRegisteredProjects, unregisterProject } from '../core/config.ts'
import { readLock } from '../core/lockfile.ts'
import { mapInsert } from '../core/map.ts'
import { SHARED_SKILLS_DIR } from '../core/paths.ts'

/**
 * Remove the skilld marker block from an agent's instruction file.
 * For .mdc files (dedicated skilld files), delete the entire file.
 * Also cleans up legacy .cursorrules markers for backwards compat.
 */
function removeAgentInstructions(agent: AgentType, projectPath: string): boolean {
  const agentConfig = agents[agent]
  if (!agentConfig.instructionFile)
    return false

  let removed = false

  // Handle current instruction file
  const filePath = join(projectPath, agentConfig.instructionFile)
  if (agentConfig.instructionFile.endsWith('.mdc')) {
    // MDC files are dedicated skilld files - just delete
    if (existsSync(filePath)) {
      rmSync(filePath)
      removed = true
    }
    // Also clean up legacy .cursorrules markers (cursor-specific)
    if (agent === 'cursor')
      removed = removeMarkerBlock(join(projectPath, '.cursorrules')) || removed
  }
  else if (existsSync(filePath)) {
    removed = removeMarkerBlock(filePath)
  }

  return removed
}

function removeMarkerBlock(filePath: string): boolean {
  if (!existsSync(filePath))
    return false

  const content = readFileSync(filePath, 'utf-8')
  const startIdx = content.indexOf(SKILLD_MARKER_START)
  if (startIdx === -1)
    return false

  const endIdx = content.indexOf(SKILLD_MARKER_END, startIdx)
  if (endIdx === -1)
    return false

  // Remove marker block plus surrounding blank lines
  const before = content.slice(0, startIdx).replace(/\n+$/, '')
  const after = content.slice(endIdx + SKILLD_MARKER_END.length).replace(/^\n+/, '')
  const updated = before + (before && after ? '\n' : '') + after

  if (updated.trim() === '') {
    rmSync(filePath)
  }
  else {
    writeFileSync(filePath, updated.endsWith('\n') ? updated : `${updated}\n`)
  }
  return true
}

export interface UninstallOptions {
  scope?: 'project' | 'all'
  agent?: AgentType
  yes: boolean
}

/**
 * Uninstall skilld skills by scope:
 * - project: Remove project skills (cwd)
 * - all: All registered projects + global skills + cache
 */
export async function uninstallCommand(opts: UninstallOptions): Promise<void> {
  let scope = opts.scope
  const registeredProjects = getRegisteredProjects()

  // Prompt for scope if not provided
  if (!scope) {
    if (!isInteractive()) {
      scope = 'project'
    }
    else {
      const allHint = registeredProjects.length > 0
        ? `${registeredProjects.length} projects + global + cache`
        : 'global skills + cache'

      const selected = await p.select({
        message: 'What do you want to uninstall?',
        options: [
          { label: 'This project', value: 'project', hint: 'current project only' },
          { label: 'Everything', value: 'all', hint: allHint },
        ],
      })

      if (p.isCancel(selected)) {
        p.cancel('Cancelled')
        return
      }
      scope = selected as 'project' | 'all'
    }
  }

  interface RemoveItem { label: string, path: string, version?: string }
  const toRemove: RemoveItem[] = []
  const seenPaths = new Set<string>()
  const projectsToUnregister: string[] = []
  const agentFilter = opts.agent ? [opts.agent] : undefined

  const addToRemove = (label: string, path: string, version?: string) => {
    if (seenPaths.has(path))
      return
    seenPaths.add(path)
    toRemove.push({ label, path, version })
  }

  // Helper to add skills from a lockfile
  const addSkillsFromLock = (skillsDir: string, label: string): string[] => {
    const trackedNames: string[] = []
    const lock = readLock(skillsDir)

    if (lock?.skills) {
      for (const [skillName, info] of Object.entries(lock.skills)) {
        trackedNames.push(skillName)
        const skillDir = join(skillsDir, skillName)
        if (existsSync(skillDir)) {
          const version = info.version ? `${info.version.split('.').slice(0, 2).join('.')}.x` : undefined
          addToRemove(`${label}: ${skillName}`, skillDir, version)
        }
      }

      // Also add the lockfile itself
      const lockPath = join(skillsDir, 'skilld-lock.yaml')
      if (existsSync(lockPath)) {
        addToRemove(`${label}: skilld-lock.yaml`, lockPath)
      }
    }

    return trackedNames
  }

  // Helper to find untracked skills in a directory
  const findUntrackedSkills = (skillsDir: string, trackedNames: string[]): string[] => {
    if (!existsSync(skillsDir))
      return []
    const tracked = new Set(trackedNames)
    return readdirSync(skillsDir)
      .filter(f => !f.startsWith('.') && f !== 'skilld-lock.yaml' && !tracked.has(f))
  }

  // Track untracked skills per directory (dedupe by path)
  const untrackedByDir = new Map<string, { label: string, skills: string[] }>()
  const processedDirs = new Set<string>()

  // Helper to process a skills directory (with deduping)
  const processSkillsDir = (skillsDir: string, label: string) => {
    if (processedDirs.has(skillsDir))
      return
    processedDirs.add(skillsDir)

    const tracked = addSkillsFromLock(skillsDir, label)
    const untracked = findUntrackedSkills(skillsDir, tracked)
    if (untracked.length > 0) {
      untrackedByDir.set(skillsDir, { label, skills: untracked })
    }
  }

  // Project skills
  if (scope === 'project') {
    // Shared dir
    const sharedDir = join(process.cwd(), SHARED_SKILLS_DIR)
    if (existsSync(sharedDir))
      processSkillsDir(sharedDir, 'project (.skills)')
    for (const [name, agent] of Object.entries(agents)) {
      if (agentFilter && !agentFilter.includes(name as AgentType))
        continue
      processSkillsDir(join(process.cwd(), agent.skillsDir), 'project')
    }
    projectsToUnregister.push(process.cwd())
  }

  // All registered projects + global
  if (scope === 'all') {
    const projectPaths = registeredProjects.length > 0 ? registeredProjects : [process.cwd()]

    // Show which projects will be affected
    if (registeredProjects.length > 0) {
      p.log.info('Projects to uninstall from:')
      for (const proj of projectPaths) {
        p.log.message(`  ${proj}`)
      }
    }

    // Project skills from lockfiles
    for (const projectPath of projectPaths) {
      if (!existsSync(projectPath))
        continue

      const shortPath = projectPath.replace(process.env.HOME || '', '~')

      // Shared dir
      const sharedDir = join(projectPath, SHARED_SKILLS_DIR)
      if (existsSync(sharedDir))
        processSkillsDir(sharedDir, `${shortPath} (.skills)`)

      for (const [name, agent] of Object.entries(agents)) {
        if (agentFilter && !agentFilter.includes(name as AgentType))
          continue
        processSkillsDir(join(projectPath, agent.skillsDir), shortPath)
      }

      projectsToUnregister.push(projectPath)
    }

    // Global skills from lockfiles
    for (const [name, agent] of Object.entries(agents)) {
      if (agentFilter && !agentFilter.includes(name as AgentType))
        continue
      if (!agent.globalSkillsDir)
        continue
      processSkillsDir(agent.globalSkillsDir, 'user')
    }

    // Cache directory
    if (existsSync(CACHE_DIR)) {
      addToRemove('~/.skilld cache', CACHE_DIR)
    }
  }

  // Warn about untracked skills that will remain (grouped by label, deduped)
  if (untrackedByDir.size > 0) {
    const groupedUntracked = new Map<string, Set<string>>()
    for (const [_dir, { label, skills }] of untrackedByDir) {
      const set = mapInsert(groupedUntracked, label, () => new Set())
      for (const s of skills) set.add(s)
    }

    const totalUntracked = [...groupedUntracked.values()].reduce((sum, s) => sum + s.size, 0)
    p.log.warn(`${totalUntracked} untracked skill(s) will remain (not managed by skilld):`)
    for (const [label, skills] of groupedUntracked) {
      p.log.message(`  ${label}: ${[...skills].join(', ')}`)
    }
  }

  if (toRemove.length === 0) {
    p.log.info('Nothing to uninstall')
    return
  }

  // Group by prefix for display
  const groups = new Map<string, Array<{ name: string, version?: string }>>()
  for (const item of toRemove) {
    const [prefix, name] = item.label.includes(': ')
      ? item.label.split(': ', 2)
      : ['other', item.label]
    mapInsert(groups, prefix!, () => []).push({ name: name!, version: item.version })
  }

  const formatGroup = (items: Array<{ name: string, version?: string }>) =>
    items.map(i => i.version ? `${i.name}@${i.version}` : i.name).join(', ')

  p.log.info(`Will remove ${toRemove.length} items:`)
  for (const [prefix, items] of groups) {
    p.log.message(`  ${prefix}: ${formatGroup(items)}`)
  }

  if (!opts.yes && isInteractive()) {
    const confirmed = await p.confirm({
      message: 'Proceed with uninstall?',
    })

    if (p.isCancel(confirmed) || !confirmed) {
      p.cancel('Cancelled')
      return
    }
  }

  // Remove all items
  for (const item of toRemove) {
    rmSync(item.path, { recursive: true, force: true })
  }

  // Show grouped removal summary
  for (const [prefix, items] of groups) {
    p.log.success(`Removed ${prefix}: ${formatGroup(items)}`)
  }

  // Remove skilld instructions from agent instruction files
  const agentTypes = agentFilter || (Object.keys(agents) as AgentType[])
  for (const proj of projectsToUnregister) {
    for (const agent of agentTypes) {
      if (removeAgentInstructions(agent, proj)) {
        const file = agents[agent].instructionFile!
        p.log.success(`Cleaned ${file}`)
      }
    }
  }

  // Unregister projects from config (skip if cache dir was removed — config is gone)
  if (scope !== 'all') {
    for (const proj of projectsToUnregister) {
      unregisterProject(proj)
    }
  }

  p.outro('skilld uninstalled')
}

export const uninstallCommandDef = defineCommand({
  meta: { name: 'uninstall', description: 'Remove skilld data' },
  args: {
    ...sharedArgs,
  },
  async run({ args }) {
    p.intro(`\x1B[1m\x1B[35mskilld\x1B[0m uninstall`)
    return uninstallCommand({
      scope: args.global ? 'all' : undefined,
      agent: args.agent as AgentType | undefined,
      yes: args.yes,
    })
  },
})
