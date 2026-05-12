/**
 * Install command - restore .skilld/ and SKILL.md from lockfile
 *
 * After cloning a repo, the .skilld/ symlinks are missing (gitignored).
 * If SKILL.md was deleted, a base version is regenerated from local metadata.
 * This command recreates them from the lockfile:
 *   .claude/skills/<skill>/.skilld/pkg -> node_modules/<pkg> (always)
 *   .claude/skills/<skill>/.skilld/docs -> ~/.skilld/references/<pkg>@<version>/docs (if external)
 *   .claude/skills/<skill>/SKILL.md -> regenerated from package.json + cache state
 */

import type { AgentType, CustomPrompt, SkillSection } from '../agent/index.ts'
import type { FeaturesConfig } from '../core/config.ts'
import type { SkillInfo } from '../core/lockfile.ts'
import { copyFileSync, existsSync, lstatSync, mkdirSync, readdirSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import * as p from '@clack/prompts'
import { defineCommand } from 'citty'
import { dirname, join } from 'pathe'
import { agents, createToolProgress, getModelLabel, linkSkillToAgents, optimizeDocs } from '../agent/index.ts'
import { writeGeneratedSkillMd, writeSkillMd } from '../agent/prompts/skill.ts'
import {
  hasShippedDocs as checkShippedDocs,
  classifyCachedDoc,
  createReferenceCache,
  ensureCacheDir,
  getCacheDir,
  getPackageDbPath,
  getPkgKeyFiles,
  getShippedSkills,
  inferDocsTypeFromCache,
  linkShippedSkill,
  listReferenceFiles,
  resolvePkgDir,
} from '../cache/index.ts'
import { promptForAgent, resolveAgent, sharedArgs } from '../cli-helpers.ts'
import { defaultFeatures, readConfig } from '../core/config.ts'
import { timedSpinner } from '../core/formatting.ts'
import { mergeLocks, parsePackageNames, parsePackages, readLock, syncLockfilesToDirs, writeLock } from '../core/lockfile.ts'
import { readPackageJsonSafe } from '../core/package-json.ts'
import { getSharedSkillsDir, skillInternalDir } from '../core/paths.ts'
import { toStoragePackageName } from '../core/prefix.ts'
import { sanitizeMarkdown } from '../core/sanitize.ts'
import { indexResources } from '../retriv/index-pipeline.ts'
import { createIndex, SearchDepsUnavailableError } from '../retriv/index.ts'
import { shutdownWorker } from '../retriv/pool.ts'
import { resolveContentDocs } from '../sources/content-resolver.ts'
import { fetchGitSkills } from '../sources/git-skills.ts'
import {
  parseGitHubUrl,
  resolveEntryFiles,
  resolvePackageDocs,
} from '../sources/index.ts'
import { selectLlmConfig, writePromptFiles } from './sync.ts'

export interface InstallOptions {
  global: boolean
  agent: AgentType
}

export async function installCommand(opts: InstallOptions): Promise<void> {
  const cwd = process.cwd()
  const agent = agents[opts.agent]
  const shared = !opts.global && getSharedSkillsDir(cwd)
  const skillsDir = opts.global
    ? join(homedir(), '.skilld', 'skills')
    : shared || join(cwd, agent.skillsDir)

  // Collect lockfiles from all agent skill dirs and merge
  // In shared mode, read from .skills/ only
  const allSkillsDirs = shared
    ? [shared]
    : Object.values(agents).map(t =>
        opts.global ? t.globalSkillsDir : join(cwd, t.skillsDir),
      )
  const allLocks = allSkillsDirs
    .map(dir => readLock(dir))
    .filter((l): l is NonNullable<typeof l> => !!l && Object.keys(l.skills).length > 0)

  if (allLocks.length === 0) {
    p.log.warn('No skilld-lock.yaml found. Run `skilld` to sync skills first.')
    return
  }

  const lock = mergeLocks(allLocks)

  const skills = Object.entries(lock.skills)
  const toRestore: Array<{ name: string, info: SkillInfo }> = []
  const features = readConfig().features ?? defaultFeatures

  // Find skills with missing/broken references symlinks
  for (const [name, info] of skills) {
    if (!info.version)
      continue

    // Shipped skills: the skill dir IS the symlink, no references/ subdir
    if (info.source === 'shipped') {
      const skillDir = join(skillsDir, name)
      if (!existsSync(skillDir)) {
        toRestore.push({ name, info })
      }
      continue
    }

    const skillDir = join(skillsDir, name)
    const referencesPath = skillInternalDir(skillDir)
    const skillMdPath = join(skillDir, 'SKILL.md')

    // Check skill dir, SKILL.md, and all internal .skilld/ references
    const needsRestore = !existsSync(skillDir)
      || !existsSync(skillMdPath)
      || !existsSync(referencesPath)
      || hasStaleReferences(referencesPath, toStoragePackageName(info.packageName || name), info.version!, features)

    if (needsRestore) {
      toRestore.push({ name, info })
    }
  }

  if (toRestore.length === 0) {
    p.log.success('All up to date')
    return
  }

  p.log.info(`Restoring ${toRestore.length} references`)
  ensureCacheDir()

  const allSkillNames = skills.map(([, info]) => info.packageName || '').filter(Boolean)
  const regenerated: Array<{ name: string, pkgName: string, version: string, skillDir: string, packages?: string }> = []

  for (const { name, info } of toRestore) {
    const version = info.version!
    const identityName = info.packageName || unsanitizeName(name, info.source)
    const pkgName = toStoragePackageName(identityName)

    // Shipped skills: re-link from node_modules or cached dist
    if (info.source === 'shipped') {
      const shipped = getShippedSkills(pkgName, cwd, version)
      const match = shipped.find(s => s.skillName === name)
      if (match) {
        linkShippedSkill(skillsDir, name, match.skillDir)
        p.log.success(`Linked ${name}`)
      }
      else {
        p.log.warn(`${name}: package ${pkgName} no longer ships this skill`)
      }
      continue
    }

    // Git-sourced skills: re-fetch from remote
    if (info.source === 'github' || info.source === 'gitlab' || info.source === 'local') {
      const source = {
        type: info.source as 'github' | 'gitlab' | 'local',
        ...(info.repo?.includes('/') ? { owner: info.repo.split('/')[0], repo: info.repo.split('/')[1] } : {}),
        skillPath: info.path,
        ref: info.ref,
        ...(info.source === 'local' ? { localPath: info.repo } : {}),
      }
      const result = await fetchGitSkills(source)
      const match = result.skills.find(s => s.name === name)
      if (match) {
        const skillDir = join(skillsDir, name)
        mkdirSync(skillDir, { recursive: true })
        writeSkillMd(skillDir, sanitizeMarkdown(match.content))
        for (const f of match.files) {
          const filePath = join(skillDir, f.path)
          mkdirSync(dirname(filePath), { recursive: true })
          writeFileSync(filePath, f.content)
        }
        p.log.success(`Restored ${name} from ${info.repo}`)
      }
      else {
        p.log.warn(`${name}: skill not found in ${info.repo}`)
      }
      continue
    }

    const skillDir = join(skillsDir, name)
    const cache = createReferenceCache(pkgName, version)
    const spin = timedSpinner()

    // Check if already in global cache - just create symlinks
    if (cache.has()) {
      spin.start(`Linking ${name}`)
      mkdirSync(skillDir, { recursive: true })
      const repoGh = info.repo ? parseGitHubUrl(`https://github.com/${info.repo}`) : null
      const docsType = inferDocsTypeFromCache(cache.dir, info.source)
      cache.linkInto(skillDir, cwd, docsType, {
        extraPackages: parsePackages(info.packages),
        features,
        repoInfo: repoGh ? { owner: repoGh.owner, repo: repoGh.repo } : undefined,
      })
      // Create search index from cached docs if missing
      if (features.search && !existsSync(getPackageDbPath(pkgName, version))) {
        spin.message(`Indexing ${name}`)
        const cached = cache.readDocs()
        const docsToIndex = cached.map(d => ({
          id: d.path,
          content: d.content,
          metadata: { package: pkgName, source: d.path, type: classifyCachedDoc(d.path).type },
        }))
        await indexResources({ packageName: pkgName, version, cwd, docsToIndex, features, onProgress: msg => spin.message(msg) })
      }
      if (!copyFromExistingAgent(skillDir, name, allSkillsDirs)) {
        if (regenerateBaseSkillMd(skillDir, pkgName, version, cwd, allSkillNames, info.source, info.packages))
          regenerated.push({ name, pkgName, version, skillDir, packages: info.packages })
      }
      spin.stop(`Linked ${name}`)
      continue
    }

    // Need to download to global cache first
    spin.start(`Downloading ${name}@${version}`)

    const resolved = await resolvePackageDocs(pkgName, { version })

    if (!resolved) {
      spin.stop(`Could not resolve: ${name}`)
      continue
    }

    const content = await resolveContentDocs({
      packageName: pkgName,
      resolved,
      version,
      onProgress: msg => spin.message(msg),
    })

    for (const warning of content.warnings)
      p.log.warn(`${name}: ${warning}`)

    if (content.docs.length > 0) {
      cache.write(content.docs)

      const repoGh = info.repo ? parseGitHubUrl(`https://github.com/${info.repo}`) : null
      const docsType = content.docsType
      cache.linkInto(skillDir, cwd, docsType, {
        extraPackages: parsePackages(info.packages),
        features,
        repoInfo: repoGh ? { owner: repoGh.owner, repo: repoGh.repo } : undefined,
      })

      if (features.search) {
        try {
          if (content.docsToIndex.length > 0) {
            await createIndex(content.docsToIndex, { dbPath: getPackageDbPath(pkgName, version) })
          }

          // Index package entry files (.d.ts / .js)
          const pkgDir = resolvePkgDir(pkgName, cwd, version)
          const entryFiles = pkgDir ? await resolveEntryFiles(pkgDir) : []
          if (entryFiles.length > 0) {
            await createIndex(entryFiles.map(e => ({
              id: e.path,
              content: e.content,
              metadata: { package: pkgName, source: `pkg/${e.path}`, type: e.type },
            })), { dbPath: getPackageDbPath(pkgName, version) })
          }
        }
        catch (err) {
          if (!(err instanceof SearchDepsUnavailableError))
            throw err
        }
      }

      if (!copyFromExistingAgent(skillDir, name, allSkillsDirs)) {
        if (regenerateBaseSkillMd(skillDir, pkgName, version, cwd, allSkillNames, info.source, info.packages))
          regenerated.push({ name, pkgName, version, skillDir, packages: info.packages })
      }
      spin.stop(`Downloaded and linked ${name}`)
    }
    else {
      spin.stop(`No docs found for ${name}`)
    }
  }

  // Offer LLM enhancement for regenerated SKILL.md files
  if (regenerated.length > 0 && !readConfig().skipLlm) {
    const names = regenerated.map(r => r.name).join(', ')
    const llmConfig = await selectLlmConfig(undefined, `Enhance SKILL.md for ${names}`)
    if (llmConfig?.promptOnly) {
      const features = readConfig().features ?? defaultFeatures
      for (const { pkgName, version, skillDir } of regenerated) {
        const globalCachePath = getCacheDir(pkgName, version)
        writePromptFiles({
          packageName: pkgName,
          version,
          skillDir,
          references: {
            docsType: 'docs',
            hasShippedDocs: false,
            pkgFiles: getPkgKeyFiles(pkgName, process.cwd(), version),
            hasIssues: existsSync(join(globalCachePath, 'issues')),
            hasDiscussions: existsSync(join(globalCachePath, 'discussions')),
            hasReleases: existsSync(join(globalCachePath, 'releases')),
            hasChangelog: false,
          },
          resolved: {},
          relatedSkills: [],
          features,
        }, {
          sections: llmConfig.sections,
          customPrompt: llmConfig.customPrompt,
        })
      }
    }
    else if (llmConfig) {
      p.log.step(getModelLabel(llmConfig.model))
      for (const { pkgName, version, skillDir, packages: pkgPackages } of regenerated) {
        await enhanceRegenerated(pkgName, version, skillDir, llmConfig.model, llmConfig.sections, llmConfig.customPrompt, pkgPackages)
      }
    }
  }

  // Write merged lockfile to target dir and sync to all other existing lockfiles
  for (const [name, info] of Object.entries(lock.skills))
    writeLock(skillsDir, name, info)

  // In shared mode: recreate per-agent symlinks, skip per-agent lockfile sync
  if (shared) {
    for (const [name] of skills)
      linkSkillToAgents(name, shared, cwd, opts.agent)
  }
  else {
    syncLockfilesToDirs(lock, allSkillsDirs.filter(d => d !== skillsDir))
  }

  await shutdownWorker()

  p.outro('Install complete')
}

/** Copy SKILL.md from another agent's skill dir if one exists */
function copyFromExistingAgent(skillDir: string, name: string, allSkillsDirs: string[]): boolean {
  const targetMd = join(skillDir, 'SKILL.md')
  if (existsSync(targetMd))
    return false
  for (const dir of allSkillsDirs) {
    if (dir === skillDir)
      continue
    const candidateMd = join(dir, name, 'SKILL.md')
    if (existsSync(candidateMd) && !lstatSync(candidateMd).isSymbolicLink()) {
      mkdirSync(skillDir, { recursive: true })
      copyFileSync(candidateMd, targetMd)
      return true
    }
  }
  return false
}

/** Try to recover original package name from sanitized name + source */
function unsanitizeName(sanitized: string, source?: string): string {
  if (source?.includes('ungh://')) {
    const match = source.match(/ungh:\/\/([^/]+)\/(.+)/)
    if (match)
      return `@${match[1]}/${match[2]}`
  }

  if (sanitized.startsWith('antfu-'))
    return `@antfu/${sanitized.slice(6)}`
  if (sanitized.startsWith('clack-'))
    return `@clack/${sanitized.slice(6)}`
  if (sanitized.startsWith('nuxt-'))
    return `@nuxt/${sanitized.slice(5)}`
  if (sanitized.startsWith('vue-'))
    return `@vue/${sanitized.slice(4)}`
  if (sanitized.startsWith('vueuse-'))
    return `@vueuse/${sanitized.slice(7)}`

  return sanitized
}

/** Run LLM enhancement on a regenerated SKILL.md */
async function enhanceRegenerated(
  pkgName: string,
  version: string,
  skillDir: string,
  model: Parameters<typeof optimizeDocs>[0]['model'],
  sections: SkillSection[],
  customPrompt?: CustomPrompt,
  packages?: string,
): Promise<void> {
  const llmLog = p.taskLog({ title: `Agent exploring ${pkgName}`, limit: 3 })

  const docFiles = listReferenceFiles(skillDir)
  const globalCachePath = getCacheDir(pkgName, version)
  const hasIssues = existsSync(join(globalCachePath, 'issues'))
  const hasDiscussions = existsSync(join(globalCachePath, 'discussions'))
  const hasGithub = hasIssues || hasDiscussions
  const hasReleases = existsSync(join(globalCachePath, 'releases'))

  const features = readConfig().features ?? defaultFeatures
  const { optimized, wasOptimized } = await optimizeDocs({
    packageName: pkgName,
    skillDir,
    model,
    version,
    hasGithub,
    hasReleases,
    docFiles,
    sections,
    customPrompt,
    features,
    pkgFiles: getPkgKeyFiles(pkgName, process.cwd(), version),
    onProgress: createToolProgress(llmLog),
  })

  if (wasOptimized) {
    llmLog.success('Generated best practices')
    // Re-read local metadata for the enhanced version
    const cwd = process.cwd()
    const pkgPath = resolvePkgDir(pkgName, cwd, version)
    let description: string | undefined
    if (pkgPath) {
      const pkgJsonPath = join(pkgPath, 'package.json')
      const pkgJsonResult = readPackageJsonSafe(pkgJsonPath)
      if (pkgJsonResult) {
        description = pkgJsonResult.parsed.description as string | undefined
      }
    }

    const docsType = inferDocsTypeFromCache(globalCachePath)

    // Derive dirName from the skill directory name
    const dirName = skillDir.split('/').pop()

    const allPackages = parsePackageNames(packages)
    writeGeneratedSkillMd(skillDir, {
      name: pkgName,
      version,
      description,
      body: optimized,
      relatedSkills: [],
      hasIssues,
      hasDiscussions,
      hasReleases,
      docsType,
      hasShippedDocs: checkShippedDocs(pkgName, cwd, version),
      pkgFiles: getPkgKeyFiles(pkgName, cwd, version),
      dirName,
      packages: allPackages.length > 1 ? allPackages : undefined,
      features,
    })
  }
  else {
    llmLog.message('Enhancement skipped')
  }
}

export const installCommandDef = defineCommand({
  meta: { name: 'install', description: 'Restore references from lockfile' },
  args: {
    global: sharedArgs.global,
    agent: sharedArgs.agent,
  },
  async run({ args }) {
    let agent = resolveAgent(args.agent)
    if (!agent || agent === 'none') {
      if (agent === 'none')
        return
      const picked = await promptForAgent()
      if (!picked || picked === 'none')
        return
      agent = picked
    }

    p.intro(`\x1B[1m\x1B[35mskilld\x1B[0m install`)
    return installCommand({ global: args.global, agent })
  },
})

/** Regenerate base SKILL.md from local metadata if missing */
function regenerateBaseSkillMd(
  skillDir: string,
  pkgName: string,
  version: string,
  cwd: string,
  allSkillNames: string[],
  source?: string,
  packages?: string,
): boolean {
  const skillMdPath = join(skillDir, 'SKILL.md')
  if (existsSync(skillMdPath))
    return false

  // Read description + deps from local package.json
  const pkgPath = resolvePkgDir(pkgName, cwd, version)
  let description: string | undefined
  if (pkgPath) {
    const pkgResult = readPackageJsonSafe(join(pkgPath, 'package.json'))
    if (pkgResult) {
      description = pkgResult.parsed.description as string | undefined
    }
  }

  // Infer docsType from source or cache
  const globalCachePath = getCacheDir(pkgName, version)
  const docsType = inferDocsTypeFromCache(globalCachePath, source)

  // Check cache dirs for issues/discussions/releases (only if feature enabled)
  const feat = readConfig().features ?? defaultFeatures
  const hasIssues = feat.issues && existsSync(join(globalCachePath, 'issues'))
  const hasDiscussions = feat.discussions && existsSync(join(globalCachePath, 'discussions'))
  const hasReleases = feat.releases && existsSync(join(globalCachePath, 'releases'))

  // Related skills from other lockfile entries
  const relatedSkills = allSkillNames.filter(n => n !== pkgName)

  // Derive dirName from the skill directory name (lockfile key)
  const dirName = skillDir.split('/').pop()

  // Build multi-package list from lockfile packages field
  const allPackages = parsePackageNames(packages)

  mkdirSync(skillDir, { recursive: true })
  writeGeneratedSkillMd(skillDir, {
    name: pkgName,
    version,
    description,
    relatedSkills,
    hasIssues,
    hasDiscussions,
    hasReleases,
    docsType,
    hasShippedDocs: checkShippedDocs(pkgName, cwd, version),
    pkgFiles: getPkgKeyFiles(pkgName, cwd, version),
    dirName,
    packages: allPackages.length > 1 ? allPackages : undefined,
    features: readConfig().features ?? defaultFeatures,
  })

  return true
}

/** Check if .skilld/ has broken symlinks or is missing expected references from global cache */
function hasStaleReferences(referencesPath: string, pkgName: string, version: string, features: FeaturesConfig): boolean {
  // Scan existing entries for broken symlinks
  for (const entry of readdirSync(referencesPath)) {
    const entryPath = join(referencesPath, entry)
    if (lstatSync(entryPath).isSymbolicLink() && !existsSync(entryPath))
      return true
  }

  // Check pkg link always expected
  if (!existsSync(join(referencesPath, 'pkg')))
    return true

  // Check expected links against global cache
  const globalCachePath = getCacheDir(pkgName, version)
  const expected: Array<[string, boolean]> = [
    ['docs', existsSync(join(globalCachePath, 'docs'))],
    ['issues', features.issues && existsSync(join(globalCachePath, 'issues'))],
    ['discussions', features.discussions && existsSync(join(globalCachePath, 'discussions'))],
    ['releases', features.releases && existsSync(join(globalCachePath, 'releases'))],
    ['sections', existsSync(join(globalCachePath, 'sections'))],
  ]

  for (const [name, shouldExist] of expected) {
    if (shouldExist && !existsSync(join(referencesPath, name)))
      return true
  }

  return false
}
