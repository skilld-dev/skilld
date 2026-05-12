/**
 * Internal phase functions: `runBaseSync` (resolve → fetch → cache → write base
 * SKILL.md) and `runEnhancePhase` (LLM optimization → finalize). Both emit
 * hookable events instead of calling a UI surface directly.
 */

import type { Hookable } from 'hookable'
import type { AgentType, SkillSection } from '../../agent/index.ts'
import type { SkillContext } from '../../agent/skill-builder.ts'
import type { SkillInfo } from '../../core/lockfile.ts'
import type { ResolveAttempt, ResolvedPackage } from '../../sources/index.ts'
import type { LlmConfig, UpdateContext } from '../llm-prompts.ts'
import type { PackageResolver } from './resolvers.ts'
import type { SyncHooks } from './run.ts'
import { existsSync, mkdirSync, readFileSync } from 'node:fs'
import { join, relative, resolve as resolvePath } from 'pathe'
import { computeSkillDirName, getModelLabel, linkSkillToAgents, sanitizeName } from '../../agent/index.ts'
import { applyCachedSections, runSkillEnhancement, writeBaseSkill, writePromptFiles } from '../../agent/skill-builder.ts'
import { ensureProjectFiles, handleShippedSkills, installSkill, linkShippedToAgents, resolveBaseDir } from '../../agent/skill-installer.ts'
import { createReferenceCache } from '../../cache/index.ts'
import { getActiveFeatures } from '../../core/config.ts'
import { todayIsoDate } from '../../core/formatting.ts'
import { findSkillDirByPackage, parsePackageNames, readLock } from '../../core/lockfile.ts'
import { parseFrontmatter } from '../../core/markdown.ts'
import { getSharedSkillsDir } from '../../core/paths.ts'
import { parseGitHubRepoSlug } from '../../core/url.ts'
import { fetchPkgDist, isPrerelease } from '../../sources/index.ts'
import { buildSkillContext, fetchAndCacheResources, prepareSkillReferences } from './pipeline.ts'

const RATE_LIMIT_RE = /\b429\b|rate.?limit|exhausted.*capacity|quota.*reset/i

/** Spec resolved into the facts every runner step needs. */
export interface ResolvedSpec {
  identityName: string
  storageName: string
  version: string
  resolved: ResolvedPackage
  kind: 'npm' | 'crate' | 'github'
  requestedTag?: string
  localVersion?: string
}

/** Resolution failure: caller decides whether to suggest, retry, or abort. */
export interface UnresolvedSpec {
  identityName: string
  shipped?: { skillName: string, skillDir: string }[]
  attempts: ResolveAttempt[]
  registryVersion?: string
}

export type ResolverResult = ResolvedSpec | UnresolvedSpec

export interface ResolverOpts {
  cwd: string
  agent: AgentType
  global: boolean
  onProgress: (msg: string) => void
}

/** Result of `runBaseSync`. */
export type BaseSyncResult
  = | { kind: 'shipped' }
    | { kind: 'unresolved', unresolved: UnresolvedSpec }
    | { kind: 'merge-needed', state: MergeNeededState }
    | { kind: 'ready', state: ReadyState }

export interface MergeNeededState {
  identityName: string
  storageName: string
  version: string
  resolved: ResolvedPackage
  baseDir: string
  skillDir: string
  skillDirName: string
  existingLock: SkillInfo
}

export interface ReadyState {
  ctx: SkillContext
  skillDir: string
  skillDirName: string
  baseDir: string
  updateCtx?: UpdateContext
  allSectionsCached: boolean
  identityName: string
  storageName: string
  version: string
  docsType: 'llms.txt' | 'readme' | 'docs'
  repoInfo?: { owner: string, repo: string }
}

export interface RunBaseConfig {
  agent: AgentType
  global: boolean
  mode?: 'add' | 'update'
  force?: boolean
  noSearch?: boolean
  name?: string
  from?: string
  eject?: boolean | string
}

/** Phase 1: resolve → fetch → cache → install → write base SKILL.md. */
export async function runBaseSync(
  spec: string,
  config: RunBaseConfig,
  hooks: Hookable<SyncHooks>,
  resolver: PackageResolver,
  cwd: string,
  defaultSections: SkillSection[],
): Promise<BaseSyncResult> {
  await hooks.callHook('resolve:start', { spec })

  const resolverResult = await resolver(spec, {
    cwd,
    agent: config.agent,
    global: config.global,
    onProgress: msg => hooks.callHook('resolve:progress', { spec, message: msg }),
  })

  if (!('resolved' in resolverResult)) {
    if (resolverResult.shipped && resolverResult.shipped.length > 0) {
      for (const s of resolverResult.shipped)
        await hooks.callHook('shipped:installed', { spec, skillName: s.skillName, skillDir: s.skillDir })
      return { kind: 'shipped' }
    }
    await hooks.callHook('resolve:failed', { spec, identityName: resolverResult.identityName, attempts: resolverResult.attempts })
    return { kind: 'unresolved', unresolved: resolverResult }
  }

  const { identityName, storageName, version, resolved, kind, requestedTag, localVersion } = resolverResult
  const cache = createReferenceCache(storageName, version)

  if (config.force)
    cache.clearForce()

  const useCache = cache.has()

  if (kind !== 'crate' && !existsSync(join(cwd, 'node_modules', identityName))) {
    await hooks.callHook('dist:downloading', { spec })
    await fetchPkgDist(identityName, version)
  }

  if (kind !== 'crate') {
    const shipped = handleShippedSkills(identityName, version, cwd, config.agent, config.global)
    if (shipped) {
      linkShippedToAgents(shipped.shipped, cwd, config.agent, config.global)
      for (const s of shipped.shipped)
        await hooks.callHook('shipped:installed', { spec, skillName: s.skillName, skillDir: s.skillDir })
      return { kind: 'shipped' }
    }
  }

  await hooks.callHook('resolve:done', { spec, version, cached: useCache, force: config.force })

  if (kind === 'npm' && !localVersion && !requestedTag && !isPrerelease(version)) {
    const nextTag = resolved.distTags?.next ?? resolved.distTags?.beta ?? resolved.distTags?.alpha
    if (nextTag && (!resolved.releasedAt || !nextTag.releasedAt || nextTag.releasedAt > resolved.releasedAt))
      await hooks.callHook('warn', { spec, message: `No local dependency found — using latest stable (${version}). Prerelease ${nextTag.version} available: skilld add ${identityName}@beta` })
  }

  cache.ensure()

  const isEject = !!config.eject
  const baseDir = resolveBaseDir(cwd, config.agent, config.global)
  let skillDirName = config.name ? sanitizeName(config.name) : computeSkillDirName(storageName)
  if (config.mode === 'update' && !config.name && !isEject) {
    const lock = readLock(baseDir)
    const found = lock ? findSkillDirByPackage(lock, identityName) : null
    if (found)
      skillDirName = found
  }
  const skillDir = isEject
    ? typeof config.eject === 'string'
      ? join(resolvePath(cwd, config.eject), skillDirName)
      : join(cwd, 'skills', skillDirName)
    : join(baseDir, skillDirName)
  mkdirSync(skillDir, { recursive: true })

  const existingLock = isEject ? undefined : readLock(baseDir)?.skills[skillDirName]
  if (existingLock && existingLock.packageName && existingLock.packageName !== identityName) {
    return {
      kind: 'merge-needed',
      state: {
        identityName,
        storageName,
        version,
        resolved,
        baseDir,
        skillDir,
        skillDirName,
        existingLock,
      },
    }
  }
  const updateCtx: UpdateContext | undefined = config.mode === 'update' && existingLock
    ? {
        oldVersion: existingLock.version,
        newVersion: version,
        syncedAt: existingLock.syncedAt,
        wasEnhanced: (() => {
          const skillMdPath = join(skillDir, 'SKILL.md')
          if (!existsSync(skillMdPath))
            return false
          const fm = parseFrontmatter(readFileSync(skillMdPath, 'utf-8'))
          return !!fm.generated_by
        })(),
      }
    : undefined

  const features = getActiveFeatures(config.noSearch ? { search: false } : undefined)

  await hooks.callHook('fetch:start', { spec })
  const resources = await fetchAndCacheResources({
    packageName: storageName,
    resolved,
    version,
    useCache,
    features,
    from: config.from,
    onProgress: msg => hooks.callHook('fetch:progress', { spec, message: msg }),
  })
  const parts: string[] = []
  if (resources.docsToIndex.length > 0) {
    const docCount = resources.docsToIndex.filter(d => d.metadata?.type === 'doc').length
    if (docCount > 0)
      parts.push(`${docCount} docs`)
  }
  if (resources.hasIssues)
    parts.push('issues')
  if (resources.hasDiscussions)
    parts.push('discussions')
  if (resources.hasReleases)
    parts.push('releases')
  await hooks.callHook('fetch:done', { spec, parts, cached: resources.usedCache })
  for (const w of resources.warnings)
    await hooks.callHook('warn', { spec, message: w })

  if (features.search)
    await hooks.callHook('index:start', { spec })
  const prepared = await prepareSkillReferences({
    packageName: storageName,
    version,
    cwd,
    skillDir,
    resources,
    features,
    baseDir,
    onIndexProgress: msg => hooks.callHook('index:progress', { spec, message: msg }),
  })
  if (features.search)
    await hooks.callHook('index:done', { spec })

  if (!isEject) {
    const repoSlug = parseGitHubRepoSlug(resolved.repoUrl)
    cache.linkPkgNamed(skillDir, cwd)
    const lock: SkillInfo = {
      packageName: identityName,
      version,
      repo: repoSlug,
      source: resources.docSource,
      syncedAt: todayIsoDate(),
      generator: 'skilld',
    }
    installSkill({
      cwd,
      agent: config.agent,
      global: config.global,
      baseDir,
      skillDirName,
      lock,
      dedupePackageName: identityName,
      skipLinkAgents: true,
    })
  }

  const updatedLock = isEject ? undefined : readLock(baseDir)?.skills[skillDirName]
  const allPackages = parsePackageNames(updatedLock?.packages)

  const ctx = buildSkillContext({
    packageName: identityName,
    cachePackageName: storageName,
    version,
    skillDir,
    skillDirName,
    resources,
    prepared,
    resolved,
    packages: allPackages,
    features,
  })

  const baseSkillMd = writeBaseSkill(ctx, { eject: isEject })
  ctx.overheadLines = baseSkillMd.split('\n').length
  await hooks.callHook('base:done', { spec, skillDir: relative(cwd, skillDir), mode: config.mode === 'update' ? 'update' : 'add' })

  const allSectionsCached = !config.force && applyCachedSections(ctx, defaultSections, { eject: isEject })
  if (allSectionsCached)
    await hooks.callHook('sections:cached', { spec })

  return {
    kind: 'ready',
    state: {
      ctx,
      skillDir,
      skillDirName,
      baseDir,
      updateCtx,
      allSectionsCached,
      identityName,
      storageName,
      version,
      docsType: resources.docsType,
      repoInfo: resources.repoInfo,
    },
  }
}

export interface RunEnhanceConfig {
  agent: AgentType
  global: boolean
  force?: boolean
  debug?: boolean
  eject?: boolean | string
}

/** Phase 2: enhance with LLM (or write prompt files), then finalize. */
export async function runEnhancePhase(
  state: ReadyState,
  llmConfig: LlmConfig | null,
  config: RunEnhanceConfig,
  hooks: Hookable<SyncHooks>,
  cwd: string,
): Promise<void> {
  const isEject = !!config.eject
  const spec = state.identityName

  if (llmConfig?.promptOnly) {
    writePromptFiles(
      { ...state.ctx, packageName: state.ctx.cachePackageName ?? state.ctx.packageName, cachePackageName: undefined },
      { sections: llmConfig.sections, customPrompt: llmConfig.customPrompt },
    )
  }
  else if (llmConfig) {
    await enhanceWithHooks(state.ctx, llmConfig, { ...config, eject: isEject }, hooks, spec)
  }

  if (isEject) {
    const cache = createReferenceCache(state.storageName, state.version)
    if (!config.debug)
      cache.clearSkillInternal(state.skillDir)
    cache.eject(state.skillDir, cwd, state.docsType, {
      features: state.ctx.features ?? getActiveFeatures(),
      repoInfo: state.repoInfo,
    })
    return
  }

  const shared: string | false = config.global ? false : (getSharedSkillsDir(cwd) ?? false)
  if (shared)
    linkSkillToAgents(state.skillDirName, shared, cwd, config.agent)

  await ensureProjectFiles({ cwd, agent: config.agent, global: config.global, shared })
}

async function enhanceWithHooks(
  ctx: SkillContext,
  llmConfig: LlmConfig,
  config: RunEnhanceConfig,
  hooks: Hookable<SyncHooks>,
  spec: string,
): Promise<void> {
  await hooks.callHook('enhance:start', { spec, modelLabel: getModelLabel(llmConfig.model) })
  const result = await runSkillEnhancement(
    ctx,
    {
      model: llmConfig.model,
      force: config.force,
      debug: config.debug,
      sections: llmConfig.sections,
      customPrompt: llmConfig.customPrompt,
      eject: !!config.eject,
    },
    progress => hooks.callHook('enhance:progress', { spec, progress }),
  )

  if (result.wasOptimized) {
    await hooks.callHook('enhance:done', {
      spec,
      usage: result.usage ? { totalTokens: result.usage.totalTokens } : undefined,
      cost: result.cost,
      debugLogsDir: result.debugLogsDir,
      error: result.error,
      warnings: result.warnings,
    })
  }
  else {
    await hooks.callHook('enhance:failed', {
      spec,
      error: result.error ?? '',
      rateLimited: !!result.error && RATE_LIMIT_RE.test(result.error),
    })
  }
}

export type { CustomPrompt, OptimizeModel, SkillSection } from '../../agent/index.ts'
