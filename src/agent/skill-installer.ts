/**
 * SkillInstaller: persists a built skill (lockfile, agent linking, project
 * registration) and ensures supporting project files (.gitignore, agent
 * instruction file).
 *
 * Public entry points:
 *   - `installSkill`             — finalize a built skill: lockfile, dedupe,
 *                                  agent linking, project registration
 *   - `ensureProjectFiles`       — once-per-session: gitignore + agent
 *                                  instructions
 *   - `handleShippedSkills`      — link skills shipped in node_modules
 *   - `resolveBaseDir`           — agent's per-project or global skills dir
 */

import type { SkillInfo } from '../core/lockfile.ts'
import type { AgentType } from './index.ts'
import { appendFileSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import * as p from '@clack/prompts'
import { join, relative } from 'pathe'
import { isInteractive } from '../cli/env.ts'
import { registerProject } from '../core/config.ts'
import { todayIsoDate } from '../core/formatting.ts'
import { findSkillDirsByPackage, readLock, removeLockEntry, writeLock } from '../core/lockfile.ts'
import { getSharedSkillsDir, SHARED_SKILLS_DIR } from '../core/paths.ts'
import { getShippedSkills, linkShippedSkill } from '../core/prepare.ts'
import { agents } from './index.ts'
import { linkSkillToAgents } from './install.ts'

export interface HandleShippedResult {
  shipped: Array<{ skillName: string, skillDir: string }>
  baseDir: string
}

/** Link shipped skills, write lock entries, register project. Returns result or null if no shipped skills. */
export function handleShippedSkills(
  packageName: string,
  version: string,
  cwd: string,
  agent: AgentType,
  global: boolean,
): HandleShippedResult | null {
  const shippedSkills = getShippedSkills(packageName, cwd, version)
  if (shippedSkills.length === 0)
    return null

  const baseDir = resolveBaseDir(cwd, agent, global)
  mkdirSync(baseDir, { recursive: true })

  for (const shipped of shippedSkills) {
    linkShippedSkill(baseDir, shipped.skillName, shipped.skillDir)
    writeLock(baseDir, shipped.skillName, {
      packageName,
      version,
      source: 'shipped',
      syncedAt: todayIsoDate(),
      generator: 'skilld',
    })
  }

  if (!global)
    registerProject(cwd)

  return { shipped: shippedSkills, baseDir }
}

/** Resolve the base skills directory for an agent */
export function resolveBaseDir(cwd: string, agent: AgentType, global: boolean): string {
  if (global) {
    const agentConfig = agents[agent]
    return agentConfig.globalSkillsDir
  }
  const shared = getSharedSkillsDir(cwd)
  if (shared)
    return shared
  const agentConfig = agents[agent]
  return join(cwd, agentConfig.skillsDir)
}

export interface InstallSkillOptions {
  cwd: string
  agent: AgentType
  global: boolean
  /** Pre-resolved base skills dir (use `resolveBaseDir`). */
  baseDir: string
  skillDirName: string
  /** Lockfile entry to write. */
  lock: SkillInfo
  /**
   * If set, remove other lockfile entries that reference this package name
   * (and delete their on-disk skill dirs). Used when a sync renames or merges
   * a skill so stale entries don't linger.
   */
  dedupePackageName?: string
  /** Skip linking the shared dir to per-agent dirs (caller does it later). */
  skipLinkAgents?: boolean
}

export interface InstallSkillResult {
  /** Shared skills dir if active, otherwise false. */
  shared: string | false
}

/**
 * Persist a built skill: write its lockfile entry, dedupe stale entries,
 * link the shared dir into each detected agent, and register the project.
 *
 * Idempotent. SKILL.md and reference files must already be on disk.
 */
export function installSkill(opts: InstallSkillOptions): InstallSkillResult {
  const { cwd, agent, global, baseDir, skillDirName, lock, dedupePackageName, skipLinkAgents } = opts

  writeLock(baseDir, skillDirName, lock)

  if (dedupePackageName) {
    const current = readLock(baseDir)
    if (current) {
      for (const stale of findSkillDirsByPackage(current, dedupePackageName, skillDirName)) {
        removeLockEntry(baseDir, stale)
        const staleDir = join(baseDir, stale)
        if (existsSync(staleDir))
          rmSync(staleDir, { recursive: true })
      }
    }
  }

  const shared: string | false = global ? false : (getSharedSkillsDir(cwd) ?? false)
  if (shared && !skipLinkAgents)
    linkSkillToAgents(skillDirName, shared, cwd, agent)

  if (!global)
    registerProject(cwd)

  return { shared }
}

export interface EnsureProjectFilesOptions {
  cwd: string
  agent: AgentType
  global: boolean
  /**
   * Pre-computed shared dir from a prior `installSkill` call. Optional;
   * recomputed if omitted. Pass `false` to force per-agent dir.
   */
  shared?: string | false
}

/**
 * Once-per-session project file maintenance: ensures `.gitignore` has
 * `.skilld` and the agent's instruction file activates skills.
 *
 * Skipped entirely for global installs.
 */
export async function ensureProjectFiles(opts: EnsureProjectFilesOptions): Promise<void> {
  const { cwd, agent, global } = opts
  if (global)
    return

  const shared = opts.shared ?? getSharedSkillsDir(cwd)
  const skillsDir = shared ? SHARED_SKILLS_DIR : agents[agent].skillsDir
  await ensureGitignore(skillsDir, cwd, false)
  await ensureAgentInstructions(agent, cwd, false)
}

/**
 * Link shipped (in-package) skills into the per-agent dirs after
 * `handleShippedSkills` populated the shared/base dir.
 */
export function linkShippedToAgents(
  shipped: Array<{ skillName: string }>,
  cwd: string,
  agent: AgentType,
  global: boolean,
): void {
  if (global)
    return
  const shared = getSharedSkillsDir(cwd)
  if (!shared)
    return
  for (const s of shipped)
    linkSkillToAgents(s.skillName, shared, cwd, agent)
}

/**
 * Check if .gitignore has `.skilld` entry.
 * If missing, prompt to add it. Skipped for global installs.
 */
export async function ensureGitignore(skillsDir: string, cwd: string, isGlobal: boolean): Promise<void> {
  if (isGlobal)
    return

  const gitignorePath = join(cwd, '.gitignore')
  const pattern = '.skilld'

  if (existsSync(gitignorePath)) {
    const content = readFileSync(gitignorePath, 'utf-8')
    if (content.split('\n').some(line => line.trim() === pattern))
      return
  }

  if (!isInteractive()) {
    const entry = `\n# Skilld references (recreated by \`skilld install\`)\n${pattern}\n`
    if (existsSync(gitignorePath)) {
      const existing = readFileSync(gitignorePath, 'utf-8')
      const separator = existing.endsWith('\n') ? '' : '\n'
      appendFileSync(gitignorePath, `${separator}${entry}`)
    }
    else {
      writeFileSync(gitignorePath, entry)
    }
    return
  }

  const relSkillsDir = relative(cwd, skillsDir) || '.'
  p.log.info(
    `\x1B[1mGit guidance:\x1B[0m\n`
    + `  \x1B[32m✓\x1B[0m Commit: \x1B[36m${relSkillsDir}/*/SKILL.md\x1B[0m\n`
    + `  \x1B[32m✓\x1B[0m Commit: \x1B[36m${relSkillsDir}/skilld-lock.yaml\x1B[0m\n`
    + `  \x1B[31m✗\x1B[0m Ignore: \x1B[36m${pattern}\x1B[0m \x1B[90m(recreated by \`skilld install\`)\x1B[0m`,
  )

  const add = await p.confirm({
    message: `Add \`${pattern}\` to .gitignore?`,
    initialValue: true,
  })

  if (p.isCancel(add) || !add)
    return

  const entry = `\n# Skilld references (recreated by \`skilld install\`)\n${pattern}\n`
  if (existsSync(gitignorePath)) {
    const existing = readFileSync(gitignorePath, 'utf-8')
    const separator = existing.endsWith('\n') ? '' : '\n'
    appendFileSync(gitignorePath, `${separator}${entry}`)
  }
  else {
    writeFileSync(gitignorePath, entry)
  }

  p.log.success('Updated .gitignore')
}

export const SKILLD_MARKER_START = '<!-- skilld -->'
export const SKILLD_MARKER_END = '<!-- /skilld -->'

const DEFAULT_SKILL_HINT = 'Before modifying code, evaluate each installed skill against the current task.\nFor each skill, determine YES/NO relevance and invoke all YES skills before proceeding.'

function getSkillInstructions(agent: AgentType): string {
  const hint = agents[agent].skillActivationHint || DEFAULT_SKILL_HINT
  return `${SKILLD_MARKER_START}\n${hint}\n${SKILLD_MARKER_END}`
}

function getMdcSkillInstructions(agent: AgentType): string {
  const hint = agents[agent].skillActivationHint || DEFAULT_SKILL_HINT
  return `---\ndescription: "Activates installed skilld skills before code changes"\nalwaysApply: true\n---\n\n${hint}`
}

/**
 * Check if agent instruction file has skilld skill-activation snippet.
 * If missing, prompt to add it. Skipped for global installs or agents without an instructionFile.
 */
export async function ensureAgentInstructions(agent: AgentType, cwd: string, isGlobal: boolean): Promise<void> {
  if (isGlobal)
    return

  const agentConfig = agents[agent]
  if (!agentConfig.instructionFile)
    return

  const filePath = join(cwd, agentConfig.instructionFile)
  const isMdc = agentConfig.instructionFile.endsWith('.mdc')

  if (isMdc) {
    if (existsSync(filePath))
      return

    const content = `${getMdcSkillInstructions(agent)}\n`

    if (!isInteractive()) {
      mkdirSync(join(filePath, '..'), { recursive: true })
      writeFileSync(filePath, content)
      return
    }

    p.note(
      `This tells your agent to check installed skills before making\n`
      + `code changes. Without it, skills are available but may not\n`
      + `activate automatically.\n`
      + `\n`
      + `\x1B[90m${getMdcSkillInstructions(agent)}\x1B[0m`,
      `Create ${agentConfig.instructionFile}`,
    )

    const add = await p.confirm({
      message: `Create ${agentConfig.instructionFile} with skill activation instructions?`,
      initialValue: true,
    })

    if (p.isCancel(add) || !add)
      return

    mkdirSync(join(filePath, '..'), { recursive: true })
    writeFileSync(filePath, content)
    p.log.success(`Created ${agentConfig.instructionFile}`)
    return
  }

  if (existsSync(filePath)) {
    const content = readFileSync(filePath, 'utf-8')
    if (content.includes(SKILLD_MARKER_START))
      return
  }

  if (!isInteractive()) {
    if (existsSync(filePath)) {
      const existing = readFileSync(filePath, 'utf-8')
      const separator = existing.endsWith('\n') ? '' : '\n'
      appendFileSync(filePath, `${separator}\n${getSkillInstructions(agent)}\n`)
    }
    else {
      writeFileSync(filePath, `${getSkillInstructions(agent)}\n`)
    }
    return
  }

  const fileExists = existsSync(filePath)
  const action = fileExists ? 'Append to' : 'Create'
  p.note(
    `This tells your agent to check installed skills before making\n`
    + `code changes. Without it, skills are available but may not\n`
    + `activate automatically.\n`
    + `\n`
    + `\x1B[90m${getSkillInstructions(agent).replace(/\n/g, '\n')}\x1B[0m`,
    `${action} ${agentConfig.instructionFile}`,
  )

  const add = await p.confirm({
    message: `${action} ${agentConfig.instructionFile} with skill activation instructions?`,
    initialValue: true,
  })

  if (p.isCancel(add) || !add)
    return

  if (existsSync(filePath)) {
    const existing = readFileSync(filePath, 'utf-8')
    const separator = existing.endsWith('\n') ? '' : '\n'
    appendFileSync(filePath, `${separator}\n${getSkillInstructions(agent)}\n`)
  }
  else {
    writeFileSync(filePath, `${getSkillInstructions(agent)}\n`)
  }

  p.log.success(`Updated ${agentConfig.instructionFile}`)
}
