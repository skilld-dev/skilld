import type { AgentType } from '../agent/index.ts'
import type { SkillInfo } from './lockfile.ts'
import type { ShippedSkill } from './prepare.ts'
import { existsSync, readdirSync } from 'node:fs'
import { join } from 'pathe'
import { agents } from '../agent/index.ts'
import { readLocalDependencies } from '../sources/index.ts'
import { parsePackages, parseSkillFrontmatter, readLock } from './lockfile.ts'
import { getSharedSkillsDir, LOCK_FILENAME, skillInternalFile } from './paths.ts'
import { getShippedSkills } from './prepare.ts'
import { semverGt, semverValid } from './semver.ts'

export interface SkillEntry {
  name: string
  dir: string
  agent: AgentType
  info: SkillInfo | null
  scope: 'local' | 'global'
  /** Original package name from package.json (e.g., @scope/pkg) */
  packageName?: string
  /** Latest version from package.json deps */
  latestVersion?: string
}

export interface AvailableShippedSkill {
  /** npm package that ships the skill */
  packageName: string
  skills: ShippedSkill[]
}

export interface ProjectState {
  skills: SkillEntry[]
  deps: Map<string, string>
  missing: string[]
  outdated: SkillEntry[]
  synced: SkillEntry[]
  /** Skills in lockfile but not matched to any local dep */
  unmatched: SkillEntry[]
  /** Dependencies that ship skills not yet installed */
  shipped: AvailableShippedSkill[]
}

export interface IterateSkillsOptions {
  scope?: 'local' | 'global' | 'all'
  agents?: AgentType[]
  cwd?: string
}

export function* iterateSkills(opts: IterateSkillsOptions = {}): Generator<SkillEntry> {
  const { scope = 'all', cwd = process.cwd() } = opts
  const agentTypes = opts.agents ?? (Object.keys(agents) as AgentType[])

  // When shared dir exists, read local skills from there (avoid duplicates from agent symlinks)
  const sharedDir = getSharedSkillsDir(cwd)
  let yieldedLocal = false

  if (sharedDir && (scope === 'local' || scope === 'all')) {
    yieldedLocal = true
    const lock = readLock(sharedDir)
    const entries = readdirSync(sharedDir).filter(f => !f.startsWith('.') && f !== LOCK_FILENAME)
    // Use first detected agent as the representative
    const firstAgent = agentTypes[0] ?? (Object.keys(agents) as AgentType[])[0]!
    for (const name of entries) {
      const dir = join(sharedDir, name)
      if (lock?.skills[name]) {
        yield { name, dir, agent: firstAgent, info: lock.skills[name], scope: 'local' }
      }
      else {
        const info = parseSkillFrontmatter(skillInternalFile(dir))
        if (info?.generator === 'skilld') {
          yield { name, dir, agent: firstAgent, info, scope: 'local' }
        }
      }
    }
  }

  for (const agentType of agentTypes) {
    const agent = agents[agentType]

    // Local skills (skip if already yielded from shared dir)
    if (!yieldedLocal && (scope === 'local' || scope === 'all')) {
      const localDir = join(cwd, agent.skillsDir)
      if (existsSync(localDir)) {
        const lock = readLock(localDir)
        const entries = readdirSync(localDir).filter(f => !f.startsWith('.') && f !== LOCK_FILENAME)
        for (const name of entries) {
          const dir = join(localDir, name)
          // Only track skills in lockfile OR with generator: "skilld"
          if (lock?.skills[name]) {
            yield { name, dir, agent: agentType, info: lock.skills[name], scope: 'local' }
          }
          else {
            const info = parseSkillFrontmatter(skillInternalFile(dir))
            if (info?.generator === 'skilld') {
              yield { name, dir, agent: agentType, info, scope: 'local' }
            }
          }
        }
      }
    }

    // Global skills
    if ((scope === 'global' || scope === 'all') && agent.globalSkillsDir) {
      const globalDir = agent.globalSkillsDir
      if (existsSync(globalDir)) {
        const lock = readLock(globalDir)
        const entries = readdirSync(globalDir).filter(f => !f.startsWith('.') && f !== LOCK_FILENAME)
        for (const name of entries) {
          const dir = join(globalDir, name)
          // Only track skills in lockfile OR with generator: "skilld"
          if (lock?.skills[name]) {
            yield { name, dir, agent: agentType, info: lock.skills[name], scope: 'global' }
          }
          else {
            const info = parseSkillFrontmatter(skillInternalFile(dir))
            if (info?.generator === 'skilld') {
              yield { name, dir, agent: agentType, info, scope: 'global' }
            }
          }
        }
      }
    }
  }
}

export function isOutdated(skill: SkillEntry, depVersion: string): boolean {
  if (!skill.info?.version)
    return true

  const depClean = depVersion.replace(/^[\^~>=<]+/, '')

  // Non-semver versions (e.g. '*' from catalog:/workspace: specifiers) can't be compared
  if (!semverValid(depClean))
    return false

  return semverGt(depClean, skill.info.version)
}

export async function getProjectState(cwd: string = process.cwd()): Promise<ProjectState> {
  const skills = [...iterateSkills({ scope: 'local', cwd })]

  // Get package.json deps
  const localDeps = await readLocalDependencies(cwd).catch(() => [])
  const deps = new Map(localDeps.map(d => [d.name, d.version]))

  // Build unified lookup: packageName -> best skill entry
  // When multiple skills claim the same package, prefer the one with the newer version
  const skillByPkgName = new Map<string, SkillEntry>()
  const setBestSkill = (key: string, s: SkillEntry) => {
    const existing = skillByPkgName.get(key)
    if (!existing) {
      skillByPkgName.set(key, s)
      return
    }
    // Prefer the skill with the newer version (more recently synced)
    if (s.info?.version && existing.info?.version && semverValid(s.info.version) && semverValid(existing.info.version) && semverGt(s.info.version, existing.info.version))
      skillByPkgName.set(key, s)
  }
  for (const s of skills) {
    if (s.info?.packageName)
      setBestSkill(s.info.packageName, s)
    for (const pkg of parsePackages(s.info?.packages))
      setBestSkill(pkg.name, s)
  }

  // Also build name-based lookups, but defer to pkgName map for conflicts
  const skillByName = new Map(skills.map(s => [s.name, s]))

  const missing: string[] = []
  const outdated: SkillEntry[] = []
  const synced: SkillEntry[] = []
  const matchedSkillNames = new Set<string>()

  for (const [pkgName, version] of deps) {
    // Normalize package name (e.g., @scope/pkg -> scope-pkg)
    const normalizedName = pkgName.replace(/^@/, '').replace(/\//g, '-')
    // Prefer packageName-based lookup (handles duplicates correctly), fall back to name-based
    const skill = skillByPkgName.get(pkgName) || skillByName.get(`${normalizedName}-skilld`) || skillByName.get(normalizedName) || skillByName.get(pkgName)

    if (!skill) {
      missing.push(pkgName)
    }
    else {
      matchedSkillNames.add(skill.name)
      if (isOutdated(skill, version)) {
        outdated.push({ ...skill, packageName: pkgName, latestVersion: version })
      }
      else {
        synced.push({ ...skill, packageName: pkgName, latestVersion: version })
      }
    }
  }

  // Skills in lockfile but not matched to any local dep
  const unmatched = skills.filter(s => !matchedSkillNames.has(s.name))

  // Discover dependencies that ship skills not yet installed
  const installedSkillNames = new Set(skills.map(s => s.name))
  const shipped: AvailableShippedSkill[] = []
  for (const pkgName of deps.keys()) {
    const pkgShipped = getShippedSkills(pkgName, cwd)
    const uninstalled = pkgShipped.filter(s => !installedSkillNames.has(s.skillName))
    if (uninstalled.length > 0)
      shipped.push({ packageName: pkgName, skills: uninstalled })
  }

  return { skills, deps, missing, outdated, synced, unmatched, shipped }
}

export function getSkillsDir(agent: AgentType, scope: 'local' | 'global', cwd: string = process.cwd()): string {
  const agentConfig = agents[agent]
  if (scope === 'global') {
    if (!agentConfig.globalSkillsDir) {
      throw new Error(`Agent ${agent} does not support global skills`)
    }
    return agentConfig.globalSkillsDir
  }
  return getSharedSkillsDir(cwd) || join(cwd, agentConfig.skillsDir)
}
