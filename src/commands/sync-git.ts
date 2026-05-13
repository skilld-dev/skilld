/**
 * Git skill sync — install pre-authored skills from git repos, or generate
 * from repo docs when no pre-authored skills exist.
 */

import type { AgentType, OptimizeModel } from '../agent/index.ts'
import type { GitSkillSource } from '../sources/git-skills.ts'
import { mkdirSync, writeFileSync } from 'node:fs'
import { styleText } from 'node:util'
import * as p from '@clack/prompts'
import { dirname, join, relative } from 'pathe'
import { agents, writeSkillMd } from '../agent/index.ts'
import { installSkill } from '../agent/skill-installer.ts'
import { CACHE_DIR } from '../cache/index.ts'
import { readConfig } from '../core/config.ts'
import { timedSpinner, todayIsoDate } from '../core/formatting.ts'
import { sanitizeMarkdown } from '../core/sanitize.ts'
import { shutdownWorker } from '../retriv/pool.ts'
import { fetchGitSkills } from '../sources/git-skills.ts'
import { track } from '../telemetry.ts'
import { DEFAULT_SECTIONS, selectLlmConfig } from './llm-prompts.ts'
import { createGithubResolver } from './sync/resolvers.ts'
import { createSyncRun } from './sync/run.ts'
import { bindClackUi } from './sync/ui/clack.ts'

const STATIC_REGEX_1 = /-skilld$/

export interface GitSyncOptions {
  source: GitSkillSource
  global: boolean
  agent: AgentType
  yes: boolean
  model?: OptimizeModel
  force?: boolean
  debug?: boolean
  from?: string
  skillFilter?: string[]
}

export async function syncGitSkills(opts: GitSyncOptions): Promise<void> {
  const { source, agent, global: isGlobal, yes } = opts
  const cwd = process.cwd()
  const agentConfig = agents[agent]
  const baseDir = isGlobal
    ? join(CACHE_DIR, 'skills')
    : join(cwd, agentConfig.skillsDir)

  const label = source.type === 'local'
    ? source.localPath!
    : `${source.owner}/${source.repo}`

  const spin = timedSpinner()
  spin.start(`Fetching skills from ${label}`)

  const { skills } = await fetchGitSkills(source, msg => spin.message(msg))

  if (skills.length === 0) {
    if (source.type === 'github' && source.owner && source.repo) {
      spin.stop(`No pre-authored skills in ${label}, generating from repo docs...`)
      return syncGitHubRepo(opts)
    }
    spin.stop(`No skills found in ${label}`)
    return
  }

  spin.stop(`Found ${skills.length} skill(s) in ${label}`)

  let selected = skills

  if (opts.skillFilter?.length) {
    const filterSet = new Set(opts.skillFilter.map(s => s.toLowerCase().replace(STATIC_REGEX_1, '')))
    selected = skills.filter(s => filterSet.has(s.name.toLowerCase().replace(STATIC_REGEX_1, '')))
    if (selected.length === 0) {
      p.log.warn(`No skills matched: ${opts.skillFilter.join(', ')}`)
      p.log.message(`Available: ${skills.map(s => s.name).join(', ')}`)
      return
    }
  }
  else if (source.skillPath) {
    selected = skills
  }
  else if (skills.length > 1 && !yes) {
    const choices = await p.autocompleteMultiselect({
      message: `Select skills to install from ${label}`,
      options: skills.map(s => ({
        label: s.name.replace(STATIC_REGEX_1, ''),
        value: s.name,
        hint: s.description || s.path,
      })),
      initialValues: [],
    })

    if (p.isCancel(choices))
      return

    const selectedNames = new Set(choices)
    selected = skills.filter(s => selectedNames.has(s.name))
    if (selected.length === 0)
      return
  }

  mkdirSync(baseDir, { recursive: true })

  for (const skill of selected) {
    const skillDir = join(baseDir, skill.name)
    mkdirSync(skillDir, { recursive: true })

    writeSkillMd(skillDir, sanitizeMarkdown(skill.content))

    if (skill.files.length > 0) {
      for (const f of skill.files) {
        const filePath = join(skillDir, f.path)
        mkdirSync(dirname(filePath), { recursive: true })
        writeFileSync(filePath, f.content)
      }
    }

    const sourceType = source.type === 'local' ? 'local' : source.type
    installSkill({
      cwd,
      agent,
      global: isGlobal,
      baseDir,
      skillDirName: skill.name,
      lock: {
        source: sourceType,
        repo: source.type === 'local' ? source.localPath : `${source.owner}/${source.repo}`,
        path: skill.path || undefined,
        ref: source.ref || 'main',
        syncedAt: todayIsoDate(),
        generator: 'external',
      },
      skipLinkAgents: true,
    })
  }

  if (source.type !== 'local' && source.owner && source.repo) {
    track({
      event: 'install',
      surface: 'cli:add',
      sourceKind: 'gh',
      slug: `${source.owner}/${source.repo}`,
      agent,
    })
  }

  for (const skill of selected) {
    const skillRel = relative(cwd, join(baseDir, skill.name))
    const fileLines = ['SKILL.md', ...skill.files.map(f => f.path)]
      .map(f => `  ${styleText('gray', '└')} ${f}`)
      .join('\n')
    p.log.success(`Installed ${styleText('cyan', skill.name)} ${styleText('gray', `→ ${skillRel}`)}\n${fileLines}`)
  }
}

async function syncGitHubRepo(opts: GitSyncOptions): Promise<void> {
  const { source, agent, global: isGlobal, yes } = opts
  const owner = source.owner!
  const repo = source.repo!
  const cwd = process.cwd()
  const spec = `${owner}/${repo}`

  const run = createSyncRun({
    cwd,
    resolver: createGithubResolver(owner, repo),
    agent,
    global: isGlobal,
    force: opts.force,
    debug: opts.debug,
    from: opts.from,
    defaultSections: DEFAULT_SECTIONS,
  })
  bindClackUi(run.hooks, { cwd })

  const base = await run.runBase(spec)

  if (base.kind !== 'ready')
    return

  const { state } = base
  const globalConfig = readConfig()
  let llmConfig: import('./llm-prompts.ts').LlmConfig | null = null
  if (!state.allSectionsCached && !globalConfig.skipLlm && (!yes || opts.model))
    llmConfig = await selectLlmConfig(opts.model)

  await run.runEnhance(state, llmConfig)

  await shutdownWorker()

  track({
    event: 'install',
    surface: 'cli:add',
    sourceKind: 'gh',
    slug: spec,
    agent,
  })

  p.outro(`Synced ${spec} to ${relative(cwd, state.skillDir)}`)
}
