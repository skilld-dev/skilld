/**
 * URL resolution cascade: package name → ResolvedPackage with discovered URLs.
 *
 * Walks npm registry → repository.url → versioned git docs → repo metadata homepage →
 * GitHub search fallback → llms.txt discovery → local node_modules README.
 * Each step pushes a ResolveAttempt for caller-facing diagnostics.
 */

import type { ResolveAttempt, ResolvedPackage, ResolveResult } from './types.ts'
import { existsSync, readdirSync } from 'node:fs'
import { pathToFileURL } from 'node:url'
import { join } from 'pathe'
import { fetchGitDocs, fetchGitHubRepoMeta, fetchReadme, searchGitHubRepo, validateGitDocsWithLlms } from './github.ts'
import { fetchLlmsTxt, fetchLlmsUrl } from './llms.ts'
import { fetchNpmPackage, fetchNpmRegistryMeta } from './npm-registry.ts'
import { getCrawlUrl } from './package-registry.ts'
import { isGitHubRepoUrl, isUselessDocsUrl, normalizeRepoUrl, parseGitHubUrl } from './utils.ts'

export type ResolveStep = 'npm' | 'github-docs' | 'github-meta' | 'github-search' | 'readme' | 'llms.txt' | 'crawl' | 'local'

export interface ResolveOptions {
  /** User's installed version - used to fetch versioned git docs */
  version?: string
  /** Current working directory - for local readme fallback */
  cwd?: string
  /** Progress callback - called before each resolution step */
  onProgress?: (step: ResolveStep) => void
}

async function resolveGitHub(
  gh: { owner: string, repo: string },
  targetVersion: string | undefined,
  pkg: { name: string },
  result: ResolvedPackage,
  attempts: ResolveAttempt[],
  onProgress?: (step: ResolveStep) => void,
  opts?: { rawRepoUrl?: string, subdir?: string },
): Promise<string[] | undefined> {
  let allFiles: string[] | undefined

  if (targetVersion) {
    onProgress?.('github-docs')
    const gitDocs = await fetchGitDocs(gh.owner, gh.repo, targetVersion, pkg.name, opts?.rawRepoUrl)
    if (gitDocs) {
      result.gitDocsUrl = gitDocs.baseUrl
      result.gitRef = gitDocs.ref
      result.gitDocsFallback = gitDocs.fallback
      allFiles = gitDocs.allFiles
      attempts.push({
        source: 'github-docs',
        url: gitDocs.baseUrl,
        status: 'success',
        message: gitDocs.fallback
          ? `Found ${gitDocs.files.length} docs at ${gitDocs.ref} (no tag for v${targetVersion})`
          : `Found ${gitDocs.files.length} docs at ${gitDocs.ref}`,
      })
    }
    else {
      attempts.push({
        source: 'github-docs',
        url: `${result.repoUrl}/tree/v${targetVersion}/docs`,
        status: 'not-found',
        message: 'No docs/ folder found at version tag',
      })
    }
  }

  if (!result.docsUrl) {
    onProgress?.('github-meta')
    const repoMeta = await fetchGitHubRepoMeta(gh.owner, gh.repo, pkg.name)
    if (repoMeta?.homepage && !isUselessDocsUrl(repoMeta.homepage)) {
      result.docsUrl = repoMeta.homepage
      attempts.push({
        source: 'github-meta',
        url: result.repoUrl!,
        status: 'success',
        message: `Found homepage: ${repoMeta.homepage}`,
      })
    }
    else {
      attempts.push({
        source: 'github-meta',
        url: result.repoUrl!,
        status: 'not-found',
        message: 'No homepage in repo metadata',
      })
    }
  }

  onProgress?.('readme')
  const readmeUrl = await fetchReadme(gh.owner, gh.repo, opts?.subdir, result.gitRef)
  if (readmeUrl) {
    result.readmeUrl = readmeUrl
    attempts.push({
      source: 'readme',
      url: readmeUrl,
      status: 'success',
    })
  }
  else {
    attempts.push({
      source: 'readme',
      url: `${result.repoUrl}/README.md`,
      status: 'not-found',
      message: 'No README found',
    })
  }

  return allFiles
}

export async function resolvePackageDocs(packageName: string, options: ResolveOptions = {}): Promise<ResolvedPackage | null> {
  const result = await resolvePackageDocsWithAttempts(packageName, options)
  return result.package
}

export async function resolvePackageDocsWithAttempts(packageName: string, options: ResolveOptions = {}): Promise<ResolveResult> {
  const attempts: ResolveAttempt[] = []
  const { onProgress } = options

  onProgress?.('npm')
  const pkg = await fetchNpmPackage(packageName)
  if (!pkg) {
    attempts.push({
      source: 'npm',
      url: `https://registry.npmjs.org/${packageName}/latest`,
      status: 'not-found',
      message: 'Package not found on npm registry',
    })
    return { package: null, attempts }
  }

  attempts.push({
    source: 'npm',
    url: `https://registry.npmjs.org/${packageName}/latest`,
    status: 'success',
    message: `Found ${pkg.name}@${pkg.version}`,
  })

  const registryMeta = pkg.version
    ? await fetchNpmRegistryMeta(packageName, pkg.version)
    : {}

  const result: ResolvedPackage = {
    name: pkg.name,
    version: pkg.version,
    releasedAt: registryMeta.releasedAt,
    description: pkg.description,
    dependencies: pkg.dependencies,
    distTags: registryMeta.distTags,
  }

  let gitDocsAllFiles: string[] | undefined

  let subdir: string | undefined
  let rawRepoUrl: string | undefined
  if (typeof pkg.repository === 'object' && pkg.repository?.url) {
    rawRepoUrl = pkg.repository.url
    const normalized = normalizeRepoUrl(rawRepoUrl)
    if (!normalized.includes('://') && normalized.includes('/') && !normalized.includes(':'))
      result.repoUrl = `https://github.com/${normalized}`
    else
      result.repoUrl = normalized
    subdir = pkg.repository.directory
  }
  else if (typeof pkg.repository === 'string') {
    if (pkg.repository.includes('://')) {
      const gh = parseGitHubUrl(pkg.repository)
      if (gh)
        result.repoUrl = `https://github.com/${gh.owner}/${gh.repo}`
    }
    else {
      const repo = pkg.repository.replace(/^github:/, '')
      if (repo.includes('/') && !repo.includes(':'))
        result.repoUrl = `https://github.com/${repo}`
    }
  }

  if (pkg.homepage && !isGitHubRepoUrl(pkg.homepage) && !isUselessDocsUrl(pkg.homepage)) {
    result.docsUrl = pkg.homepage
  }

  if (result.repoUrl?.includes('github.com')) {
    const gh = parseGitHubUrl(result.repoUrl)
    if (gh) {
      const targetVersion = options.version || pkg.version
      gitDocsAllFiles = await resolveGitHub(gh, targetVersion, pkg, result, attempts, onProgress, { rawRepoUrl, subdir })
    }
  }
  else if (!result.repoUrl) {
    onProgress?.('github-search')
    const searchedUrl = await searchGitHubRepo(pkg.name)
    if (searchedUrl) {
      result.repoUrl = searchedUrl
      attempts.push({
        source: 'github-search',
        url: searchedUrl,
        status: 'success',
        message: `Found via GitHub search: ${searchedUrl}`,
      })

      const gh = parseGitHubUrl(searchedUrl)
      if (gh) {
        const targetVersion = options.version || pkg.version
        gitDocsAllFiles = await resolveGitHub(gh, targetVersion, pkg, result, attempts, onProgress)
      }
    }
    else {
      attempts.push({
        source: 'github-search',
        status: 'not-found',
        message: 'No repository URL in package.json and GitHub search found no match',
      })
    }
  }

  const crawlUrl = getCrawlUrl(packageName)
  if (crawlUrl) {
    result.crawlUrl = crawlUrl
  }

  if (result.docsUrl) {
    onProgress?.('llms.txt')
    const llmsUrl = await fetchLlmsUrl(result.docsUrl)
    if (llmsUrl) {
      result.llmsUrl = llmsUrl
      attempts.push({
        source: 'llms.txt',
        url: llmsUrl,
        status: 'success',
      })
    }
    else {
      attempts.push({
        source: 'llms.txt',
        url: `${new URL(result.docsUrl).origin}/llms.txt`,
        status: 'not-found',
        message: 'No llms.txt at docs URL',
      })
    }
  }

  if (result.gitDocsUrl && result.llmsUrl && gitDocsAllFiles) {
    const llmsContent = await fetchLlmsTxt(result.llmsUrl)
    if (llmsContent && llmsContent.links.length > 0) {
      const validation = validateGitDocsWithLlms(llmsContent.links, gitDocsAllFiles)
      if (!validation.isValid) {
        attempts.push({
          source: 'github-docs',
          url: result.gitDocsUrl,
          status: 'not-found',
          message: `Heuristic git docs don't match llms.txt links (${Math.round(validation.matchRatio * 100)}% match), preferring llms.txt`,
        })
        result.gitDocsUrl = undefined
        result.gitRef = undefined
      }
    }
  }

  if (!result.docsUrl && !result.llmsUrl && !result.readmeUrl && !result.gitDocsUrl && options.cwd) {
    onProgress?.('local')
    const pkgDir = join(options.cwd, 'node_modules', packageName)
    const readmeFile = existsSync(pkgDir) && readdirSync(pkgDir).find(f => /^readme\.md$/i.test(f))
    if (readmeFile) {
      const readmePath = join(pkgDir, readmeFile)
      result.readmeUrl = pathToFileURL(readmePath).href
      attempts.push({
        source: 'readme',
        url: readmePath,
        status: 'success',
        message: 'Found local readme in node_modules',
      })
    }
  }

  if (!result.docsUrl && !result.llmsUrl && !result.readmeUrl && !result.gitDocsUrl) {
    return { package: null, attempts, registryVersion: pkg.version }
  }

  return { package: result, attempts, registryVersion: pkg.version }
}
