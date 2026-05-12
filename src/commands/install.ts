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

import type { AgentType } from '../agent/index.ts'
import type { SkillContext } from '../agent/skill-builder.ts'
import type { FeaturesConfig } from '../core/config.ts'
import type { SkillInfo } from '../core/lockfile.ts'
import type { ResolvedPackage } from '../sources/index.ts'
import { copyFileSync, existsSync, lstatSync, mkdirSync, readdirSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import * as p from '@clack/prompts'
import { defineCommand } from 'citty'
import { dirname, join } from 'pathe'
import { agents, getModelLabel, linkSkillToAgents } from '../agent/index.ts'
import { writeSkillMd } from '../agent/prompts/skill.ts'
import { enhanceSkillWithLLM, writeBaseSkill, writePromptFiles } from '../agent/skill-builder.ts'
import { createReferenceCache, ensureCacheDir, getCacheDir } from '../cache/index.ts'
import { promptForAgent, resolveAgent } from '../cli/agent-prompt.ts'
import { sharedArgs } from '../cli/args.ts'
import { defaultFeatures, readConfig } from '../core/config.ts'
import { timedSpinner } from '../core/formatting.ts'
import { mergeLocks, parsePackageNames, parsePackages, readLock, syncLockfilesToDirs, writeLock } from '../core/lockfile.ts'
import { readPackageJsonSafe } from '../core/package-json.ts'
import { getSharedSkillsDir, skillInternalDir } from '../core/paths.ts'
import { toStoragePackageName } from '../core/prefix.ts'
import {
  getShippedSkills,
  linkShippedSkill,
  resolvePkgDir,
} from '../core/prepare.ts'
import { sanitizeMarkdown } from '../core/sanitize.ts'
import { shutdownWorker } from '../retriv/pool.ts'
import { fetchGitSkills } from '../sources/git-skills.ts'
import { resolvePackageDocs } from '../sources/index.ts'
import { selectLlmConfig } from './llm-prompts.ts'
import { buildSkillContext, fetchAndCacheResources, prepareSkillReferences } from './sync/pipeline.ts'

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

  const regenerated: Array<{ name: string, ctx: SkillContext }> = []

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
    const wasCacheHit = cache.has()
    spin.start(wasCacheHit ? `Linking ${name}` : `Downloading ${name}@${version}`)
    mkdirSync(skillDir, { recursive: true })

    const resolved: ResolvedPackage | null = wasCacheHit
      ? synthesizeResolved(identityName, version, info, cwd)
      : await resolvePackageDocs(pkgName, { version })

    if (!resolved) {
      spin.stop(`Could not resolve: ${name}`)
      continue
    }

    const resources = await fetchAndCacheResources({
      packageName: pkgName,
      resolved,
      version,
      useCache: wasCacheHit,
      features,
      onProgress: msg => spin.message(msg),
    })

    for (const w of resources.warnings)
      p.log.warn(`${name}: ${w}`)

    if (!cache.has()) {
      spin.stop(`No docs found for ${name}`)
      continue
    }

    const prepared = await prepareSkillReferences({
      packageName: pkgName,
      version,
      cwd,
      skillDir,
      resources,
      features,
      baseDir: skillsDir,
      extraPackages: parsePackages(info.packages),
      onIndexProgress: msg => spin.message(msg),
    })

    const ctx = buildSkillContext({
      packageName: identityName,
      cachePackageName: pkgName,
      version,
      skillDir,
      skillDirName: name,
      resources,
      prepared,
      resolved,
      packages: parsePackageNames(info.packages),
      features,
    })

    if (!copyFromExistingAgent(skillDir, name, allSkillsDirs)) {
      if (!existsSync(join(skillDir, 'SKILL.md'))) {
        writeBaseSkill(ctx)
        regenerated.push({ name, ctx })
      }
    }
    spin.stop(wasCacheHit ? `Linked ${name}` : `Downloaded and linked ${name}`)
  }

  // Offer LLM enhancement for regenerated SKILL.md files
  if (regenerated.length > 0 && !readConfig().skipLlm) {
    const names = regenerated.map(r => r.name).join(', ')
    const llmConfig = await selectLlmConfig(undefined, `Enhance SKILL.md for ${names}`)
    if (llmConfig?.promptOnly) {
      for (const { ctx } of regenerated)
        writePromptFiles(ctx, { sections: llmConfig.sections, customPrompt: llmConfig.customPrompt })
    }
    else if (llmConfig) {
      p.log.step(getModelLabel(llmConfig.model))
      for (const { ctx } of regenerated) {
        await enhanceSkillWithLLM(ctx, {
          model: llmConfig.model,
          sections: llmConfig.sections,
          customPrompt: llmConfig.customPrompt,
        })
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

/** Build a minimal ResolvedPackage from lockfile state for cache-hit restoration. */
function synthesizeResolved(identityName: string, version: string, info: SkillInfo, cwd: string): ResolvedPackage {
  const repoUrl = info.repo?.includes('/') ? `https://github.com/${info.repo}` : undefined
  const pkgPath = resolvePkgDir(toStoragePackageName(identityName), cwd, version)
  const description = pkgPath
    ? readPackageJsonSafe(join(pkgPath, 'package.json'))?.parsed.description as string | undefined
    : undefined
  return { name: identityName, version, repoUrl, description }
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
