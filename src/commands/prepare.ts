/**
 * Prepare command — lightweight hook for package.json "prepare" script.
 *
 * Designed to run on every `pnpm install` / `npm install`. Blocking, fast, no LLM calls.
 * 1. Restore broken symlinks from lockfile (like `install` but skips doc fetching)
 * 2. Auto-install shipped skills from deps (just symlinks + lockfile writes)
 * 3. Report outdated skills count and suggest `skilld update`
 */

import { existsSync, mkdirSync } from 'node:fs'
import * as p from '@clack/prompts'
import { defineCommand } from 'citty'
import { join } from 'pathe'
import { agents, linkSkillToAgents } from '../agent/index.ts'
import { resolveAgent } from '../cli/agent-prompt.ts'
import { todayIsoDate } from '../core/formatting.ts'
import { readLock, writeLock } from '../core/lockfile.ts'
import { getSharedSkillsDir } from '../core/paths.ts'
import { getShippedSkills, linkShippedSkill, restorePkgSymlink } from '../core/prepare.ts'
import { getProjectState } from '../core/skills.ts'

export const prepareCommandDef = defineCommand({
  meta: { name: 'prepare', description: 'Restore references and sync shipped skills (for package.json hooks)' },
  args: {
    agent: {
      type: 'enum' as const,
      options: Object.keys(agents),
      alias: 'a',
      description: 'Target agent',
    },
  },
  async run({ args }) {
    const cwd = process.cwd()

    const agent = resolveAgent(args.agent)
    if (!agent || agent === 'none')
      return

    const agentConfig = agents[agent]
    const shared = getSharedSkillsDir(cwd)
    const skillsDir = shared || join(cwd, agentConfig.skillsDir)

    // ── Fast path: read primary lockfile, check all skills intact ──

    const lock = readLock(skillsDir)
    if (lock && Object.keys(lock.skills).length > 0) {
      let allIntact = true

      for (const [name, info] of Object.entries(lock.skills)) {
        if (!info.version)
          continue

        const skillDir = join(skillsDir, name)
        if (existsSync(skillDir)) {
          // Skill dir exists; for non-shipped, also check .skilld/pkg symlink
          if (info.source !== 'shipped')
            restorePkgSymlink(skillsDir, name, info, cwd)
          continue
        }

        // Skill dir missing, needs restore
        allIntact = false

        if (info.source === 'shipped') {
          const pkgName = info.packageName || name
          const shipped = getShippedSkills(pkgName, cwd, info.version)
          const match = shipped.find(s => s.skillName === name)
          if (match)
            linkShippedSkill(skillsDir, name, match.skillDir)
        }
      }

      // If all skills intact, skip expensive getProjectState entirely
      if (allIntact)
        return
    }

    // ── Slow path: discover new shipped skills + report outdated ──

    const state = await getProjectState(cwd)
    let shippedCount = 0

    if (state.shipped.length > 0) {
      mkdirSync(skillsDir, { recursive: true })

      for (const entry of state.shipped) {
        const version = state.deps.get(entry.packageName)?.replace(/^[\^~>=<]+/, '') || '0.0.0'

        for (const skill of entry.skills) {
          linkShippedSkill(skillsDir, skill.skillName, skill.skillDir)
          writeLock(skillsDir, skill.skillName, {
            packageName: entry.packageName,
            version,
            source: 'shipped',
            syncedAt: todayIsoDate(),
            generator: 'skilld',
          })

          if (shared)
            linkSkillToAgents(skill.skillName, shared, cwd, agent)

          shippedCount++
        }
      }

      if (shippedCount > 0)
        p.log.success(`Installed ${shippedCount} shipped skill${shippedCount > 1 ? 's' : ''}`)
    }

    if (state.outdated.length > 0) {
      const n = state.outdated.length
      p.log.info(`${n} package${n > 1 ? 's' : ''} ha${n > 1 ? 've' : 's'} new features and/or breaking changes. Run \`skilld update\` to sync.`)
    }
  },
})
