/**
 * Registry-based skill installation: fetch SKILL.md → write → lockfile → link.
 * No doc resolution, no LLM, no caching. Fast path.
 */

import type { AgentType } from '../../agent/index.ts'
import type { RegistrySkill } from '../../registry/client.ts'
import { mkdirSync } from 'node:fs'
import { join } from 'pathe'
import { writeSkillMd } from '../../agent/prompts/skill.ts'
import { installSkill, resolveBaseDir } from '../../agent/skill-installer.ts'
import { SHARED_SKILLS_DIR } from '../../core/paths.ts'
import { fetchRegistrySkill } from '../../registry/client.ts'

export interface SyncRegistryOptions {
  packageName: string
  agent: AgentType
  global?: boolean
  cwd?: string
}

export async function syncRegistrySkill(opts: SyncRegistryOptions): Promise<RegistrySkill | null> {
  const { packageName, agent, cwd = process.cwd() } = opts

  const skill = await fetchRegistrySkill(packageName)
  if (!skill)
    return null

  const sharedDir = join(cwd, SHARED_SKILLS_DIR)
  const skillDir = join(sharedDir, skill.name)
  mkdirSync(skillDir, { recursive: true })
  writeSkillMd(skillDir, skill.content)

  const baseDir = resolveBaseDir(cwd, agent, false)
  mkdirSync(baseDir, { recursive: true })
  installSkill({
    cwd,
    agent,
    global: false,
    baseDir,
    skillDirName: skill.name,
    lock: {
      packageName: skill.packageName,
      version: skill.updatedAt,
      repo: skill.repo,
      source: 'registry',
      syncedAt: new Date().toISOString().slice(0, 10),
      generator: 'curator',
    },
  })

  return skill
}
