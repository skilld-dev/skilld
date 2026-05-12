import type { AgentType, SkillSection } from '../../agent/index.ts'
import { existsSync, mkdirSync, writeFileSync } from 'node:fs'
import * as p from '@clack/prompts'
import { join, relative, resolve } from 'pathe'
import {
  buildAllSectionPrompts,
  computeSkillDirName,
  portabilizePrompt,
  SECTION_OUTPUT_FILES,
  writeGeneratedSkillMd,
} from '../../agent/index.ts'
import { ensureGitignore, ensureProjectFiles, installSkill, resolveBaseDir } from '../../agent/skill-installer.ts'
import { createReferenceCache, listReferenceFiles } from '../../cache/index.ts'
import { getActiveFeatures } from '../../core/config.ts'
import { timedSpinner, todayIsoDate } from '../../core/formatting.ts'
import { writeLock } from '../../core/lockfile.ts'
import { parseGitHubRepoSlug } from '../../core/url.ts'
import {
  fetchPkgDist,
  resolvePackageOrCrate,
} from '../../sources/index.ts'
import { DEFAULT_SECTIONS } from '../llm-prompts.ts'
import {
  fetchAndCacheResources,
  prepareSkillReferences,
} from './pipeline.ts'

export async function exportPortablePrompts(packageSpec: string, opts: {
  out?: string
  sections?: SkillSection[]
  force?: boolean
  agent?: AgentType | 'none'
}): Promise<void> {
  const sections = opts.sections ?? DEFAULT_SECTIONS

  const spin = timedSpinner()
  spin.start(`Resolving ${packageSpec}`)
  const cwd = process.cwd()

  const { packageName, localVersion, resolved } = await resolvePackageOrCrate(packageSpec, {
    cwd,
    onProgress: label => spin.message(`${packageSpec}: ${label}`),
  })

  if (!resolved) {
    spin.stop(`Could not find docs for: ${packageSpec}`)
    return
  }

  const version = localVersion || resolved.version || 'latest'
  const cache = createReferenceCache(packageName, version)
  const useCache = !opts.force && cache.has()

  if (!existsSync(join(cwd, 'node_modules', packageName))) {
    spin.message(`Downloading ${packageName}@${version} dist`)
    await fetchPkgDist(packageName, version)
  }

  spin.stop(`Resolved ${packageName}@${useCache ? cache.versionKey : version}`)
  cache.ensure()

  const skillDirName = computeSkillDirName(packageName)
  const features = getActiveFeatures()

  const agent: AgentType | null = opts.agent === 'none'
    ? null
    : opts.agent ?? (await import('../../agent/detect.ts').then(m => m.detectTargetAgent()))
  const baseDir = agent
    ? resolveBaseDir(cwd, agent, false)
    : join(cwd, '.claude', 'skills')
  const skillDir = opts.out ? resolve(cwd, opts.out) : join(baseDir, skillDirName)

  if (existsSync(skillDir) && !opts.force) {
    const existing = Object.values(SECTION_OUTPUT_FILES).filter(f => existsSync(join(skillDir, f)))
    if (existing.length > 0)
      p.log.warn(`Overwriting existing output files in ${relative(cwd, skillDir)}: ${existing.join(', ')}`)
  }
  mkdirSync(skillDir, { recursive: true })

  const resSpin = timedSpinner()
  resSpin.start('Fetching resources')
  const resources = await fetchAndCacheResources({
    packageName,
    resolved,
    version,
    useCache,
    features,
    onProgress: msg => resSpin.message(msg),
  })
  resSpin.stop('Resources ready')
  for (const w of resources.warnings)
    p.log.warn(`\x1B[33m${w}\x1B[0m`)

  const prepared = await prepareSkillReferences({
    packageName,
    version,
    cwd,
    skillDir,
    resources,
    features,
    baseDir: join(skillDir, '..'),
  })
  const { hasChangelog, shippedDocs, pkgFiles, relatedSkills } = prepared
  const docFiles = listReferenceFiles(skillDir)

  const prompts = buildAllSectionPrompts({
    packageName,
    skillDir,
    version,
    hasIssues: resources.hasIssues,
    hasDiscussions: resources.hasDiscussions,
    hasReleases: resources.hasReleases,
    hasChangelog,
    docFiles,
    docsType: resources.docsType,
    hasShippedDocs: shippedDocs,
    pkgFiles,
    features,
    sections,
  })

  cache.eject(skillDir, cwd, resources.docsType, { features, repoInfo: resources.repoInfo })
  cache.clearSkillInternal(skillDir)

  for (const [section, prompt] of prompts) {
    const portable = portabilizePrompt(prompt, section)
    writeFileSync(join(skillDir, `PROMPT_${section}.md`), portable)
  }

  writeGeneratedSkillMd(skillDir, {
    name: packageName,
    version,
    releasedAt: resolved.releasedAt,
    description: resolved.description,
    distTags: resolved.distTags,
    relatedSkills,
    hasIssues: resources.hasIssues,
    hasDiscussions: resources.hasDiscussions,
    hasReleases: resources.hasReleases,
    hasChangelog,
    docsType: resources.docsType,
    hasShippedDocs: shippedDocs,
    pkgFiles,
    repoUrl: resolved.repoUrl,
    features,
    eject: true,
  })

  const repoSlug = parseGitHubRepoSlug(resolved.repoUrl)
  if (agent) {
    const { shared } = installSkill({
      cwd,
      agent,
      global: false,
      baseDir,
      skillDirName,
      lock: {
        packageName,
        version,
        repo: repoSlug,
        source: resources.docSource,
        syncedAt: todayIsoDate(),
        generator: 'skilld',
      },
    })
    await ensureProjectFiles({ cwd, agent, global: false, shared })
  }
  else {
    writeLock(baseDir, skillDirName, {
      packageName,
      version,
      repo: repoSlug,
      source: resources.docSource,
      syncedAt: todayIsoDate(),
      generator: 'skilld',
    })
    await ensureGitignore('.claude/skills', cwd, false)
  }

  const relDir = relative(cwd, skillDir)
  const sectionList = [...prompts.keys()]
  p.log.success(`Skill installed to ${relDir}`)

  const promptFiles = sectionList.map(s => `PROMPT_${s}.md`).join(', ')
  const outputFileList = sectionList.map(s => SECTION_OUTPUT_FILES[s]).join(', ')
  p.log.info(`Have your agent enhance the skill. Give it this prompt:\n\x1B[2m\x1B[3m  Read each prompt file (${promptFiles}) in ${relDir}/, read the\n  referenced files, then write your output to the matching file (${outputFileList}).\n  When done, run: skilld assemble\x1B[0m`)
}
