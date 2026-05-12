import type { SkillContext } from '../../agent/skill-builder.ts'
import type { FeaturesConfig } from '../../core/config.ts'
import type { IndexDoc } from '../../sources/content-resolver.ts'
import type { ResolvedPackage, ResolveStep } from '../../sources/index.ts'
import { existsSync, readdirSync } from 'node:fs'
import { join } from 'pathe'
import { createReferenceCache } from '../../cache/index.ts'
import { defaultFeatures, readConfig } from '../../core/config.ts'
import { buildPackageDirMap, readLock } from '../../core/lockfile.ts'
import { indexResources } from '../../retriv/index-pipeline.ts'
import { resolveContentDocs } from '../../sources/content-resolver.ts'
import {
  fetchNpmPackage,
  generateDocsIndex,
} from '../../sources/index.ts'
import { resolveTimelineReferences } from '../../sources/timeline-resolver.ts'

export type { IndexDoc } from '../../sources/content-resolver.ts'

export const RESOLVE_STEP_LABELS: Record<ResolveStep, string> = {
  'npm': 'npm registry',
  'github-docs': 'GitHub docs',
  'github-meta': 'GitHub meta',
  'github-search': 'GitHub search',
  'readme': 'README',
  'llms.txt': 'llms.txt',
  'crawl': 'website crawl',
  'local': 'node_modules',
}

export async function findRelatedSkills(packageName: string, skillsDir: string): Promise<string[]> {
  const related: string[] = []

  const npmInfo = await fetchNpmPackage(packageName)
  if (!npmInfo?.dependencies)
    return related

  const deps = new Set(Object.keys(npmInfo.dependencies))

  if (!existsSync(skillsDir))
    return related

  const lock = readLock(skillsDir)
  const pkgToDirName = lock ? buildPackageDirMap(lock) : new Map<string, string>()
  const installedSet = new Set(readdirSync(skillsDir))

  for (const dep of deps) {
    const dirName = pkgToDirName.get(dep)
    if (dirName && installedSet.has(dirName))
      related.push(dirName)
  }

  return related.slice(0, 5)
}

/** Detect CHANGELOG.md in a package directory or cached releases */
export function detectChangelog(pkgDir: string | null, cacheDir?: string): string | false {
  if (pkgDir) {
    const found = ['CHANGELOG.md', 'changelog.md'].find(f => existsSync(join(pkgDir, f)))
    if (found)
      return `pkg/${found}`
  }
  if (cacheDir && existsSync(join(cacheDir, 'releases', 'CHANGELOG.md')))
    return 'releases/CHANGELOG.md'
  return false
}

export interface FetchResult {
  docSource: string
  docsType: 'llms.txt' | 'readme' | 'docs'
  docsToIndex: IndexDoc[]
  hasIssues: boolean
  hasDiscussions: boolean
  hasReleases: boolean
  warnings: string[]
  repoInfo?: { owner: string, repo: string }
  usedCache: boolean
}

/** Fetch and cache all resources for a package */
export async function fetchAndCacheResources(opts: {
  packageName: string
  resolved: ResolvedPackage
  version: string
  useCache: boolean
  features?: FeaturesConfig
  from?: string
  onProgress: (message: string) => void
}): Promise<FetchResult> {
  const { packageName, resolved, version, onProgress } = opts
  const features = opts.features ?? readConfig().features ?? defaultFeatures
  const cache = createReferenceCache(packageName, version)

  const cacheInvalidated = opts.useCache
    && resolved.crawlUrl
    && cache.detectDocs(resolved.repoUrl, resolved.llmsUrl).docsType === 'readme'
  const useCache = opts.useCache && !cacheInvalidated
  let docSource: string = resolved.readmeUrl || 'readme'
  let docsType: 'llms.txt' | 'readme' | 'docs' = 'readme'
  const docsToIndex: IndexDoc[] = []
  const warnings: string[] = []
  if (cacheInvalidated)
    warnings.push(`Retrying crawl for ${resolved.crawlUrl} (previous attempt only cached README)`)

  if (!useCache) {
    const content = await resolveContentDocs({ packageName, resolved, version, onProgress })
    docSource = content.docSource
    docsType = content.docsType
    docsToIndex.push(...content.docsToIndex)
    warnings.push(...content.warnings)
    if (content.docs.length > 0) {
      cache.write(content.docs)
      if (docsType !== 'readme' && content.docs.filter(d => d.path.startsWith('docs/') && d.path.endsWith('.md')).length > 1) {
        const docsIndex = generateDocsIndex(content.docs)
        if (docsIndex)
          cache.write([{ path: 'docs/_INDEX.md', content: docsIndex }])
      }
    }
  }
  else {
    const cached = cache.load({
      repoUrl: resolved.repoUrl,
      llmsUrl: resolved.llmsUrl,
      readmeUrl: resolved.readmeUrl,
      onProgress,
      generateDocsIndex,
    })
    docsType = cached.docsType
    docSource = cached.docSource
    docsToIndex.push(...cached.docsToIndex)
    if (cached.backfillIndex)
      cache.write([cached.backfillIndex])
  }

  const timeline = await resolveTimelineReferences({
    packageName,
    resolved,
    version,
    features,
    from: opts.from,
    onProgress,
  })
  docsToIndex.push(...timeline.docsToIndex)

  return {
    docSource,
    docsType,
    docsToIndex,
    hasIssues: timeline.hasIssues,
    hasDiscussions: timeline.hasDiscussions,
    hasReleases: timeline.hasReleases,
    warnings,
    repoInfo: timeline.repoInfo,
    usedCache: useCache,
  }
}

export interface PreparedSkill {
  hasChangelog: string | false
  shippedDocs: boolean
  pkgFiles: string[]
  relatedSkills: string[]
}

export async function prepareSkillReferences(opts: {
  packageName: string
  version: string
  cwd: string
  skillDir: string
  resources: FetchResult
  features: FeaturesConfig
  baseDir?: string
  onIndexProgress?: (msg: string) => void
}): Promise<PreparedSkill> {
  const { packageName, version, cwd, skillDir, resources, features, baseDir, onIndexProgress } = opts
  const cache = createReferenceCache(packageName, version)

  cache.linkInto(skillDir, cwd, resources.docsType, { features, repoInfo: resources.repoInfo })

  if (features.search) {
    await indexResources({
      packageName,
      version,
      cwd,
      docsToIndex: resources.docsToIndex,
      features,
      onProgress: onIndexProgress ?? (() => {}),
    })
  }

  const pkgDir = cache.pkgDir(cwd)
  const hasChangelog = detectChangelog(pkgDir, cache.dir)
  const shippedDocs = cache.hasShipped(cwd)
  const pkgFiles = cache.keyFiles(cwd)
  const relatedSkills = baseDir ? await findRelatedSkills(packageName, baseDir) : []

  return { hasChangelog, shippedDocs, pkgFiles, relatedSkills }
}

export { resolveLocalDep } from '../../sources/local-dep.ts'

export interface BuildSkillContextOpts {
  packageName: string
  cachePackageName?: string
  version: string
  skillDir: string
  skillDirName: string
  resources: FetchResult
  prepared: PreparedSkill
  resolved: ResolvedPackage
  packages?: Array<{ name: string }>
  features: FeaturesConfig
  overheadLines?: number
}

export function buildSkillContext(opts: BuildSkillContextOpts): SkillContext {
  const { packages, cachePackageName, packageName } = opts
  return {
    packageName,
    ...(cachePackageName && cachePackageName !== packageName ? { cachePackageName } : {}),
    version: opts.version,
    skillDir: opts.skillDir,
    dirName: opts.skillDirName,
    references: {
      docsType: opts.resources.docsType,
      hasShippedDocs: opts.prepared.shippedDocs,
      pkgFiles: opts.prepared.pkgFiles,
      hasIssues: opts.resources.hasIssues,
      hasDiscussions: opts.resources.hasDiscussions,
      hasReleases: opts.resources.hasReleases,
      hasChangelog: opts.prepared.hasChangelog,
    },
    resolved: opts.resolved,
    relatedSkills: opts.prepared.relatedSkills,
    packages: packages && packages.length > 1 ? packages : undefined,
    features: opts.features,
    overheadLines: opts.overheadLines,
  }
}
