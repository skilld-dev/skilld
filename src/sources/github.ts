/**
 * GitHub repo resolution: search, metadata, README, source files, full ResolvedPackage build.
 *
 * Tag/version logic lives in `./github-tags.ts`; doc discovery + fetch in `./github-docs.ts`.
 * Re-exports the doc-side public surface for back-compat with existing importers.
 */

import type { ResolvedPackage } from './types.ts'
import { spawnSync } from 'node:child_process'
import { existsSync as fsExistsSync, readFileSync as fsReadFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { extractBranchHint, parseGitHubUrl } from '../core/url.ts'
import { getGitHubToken, ghApi, isKnownPrivateRepo, markRepoPrivate } from './github-common.ts'
import { fetchGitDocs } from './github-docs.ts'
import { fetchUnghReleases, findGitTag } from './github-tags.ts'
import { isGhAvailable } from './issues.ts'
import { fetchLlmsUrl } from './llms.ts'
import { getDocOverride } from './package-registry.ts'
import { $fetch, fetchGitHubRaw, fetchText } from './utils.ts'

// Re-export doc-side public surface (callers used to import these from here)
export type { GitDocsResult } from './github-docs.ts'
export { fetchGitDocs, filterFrameworkDocs, isShallowGitDocs, MIN_GIT_DOCS, validateGitDocsWithLlms } from './github-docs.ts'

/**
 * Verify a GitHub repo is the source for an npm package by checking package.json name field.
 * Checks root first, then common monorepo paths (packages/{shortName}, packages/{name}).
 */
async function verifyNpmRepo(owner: string, repo: string, packageName: string): Promise<boolean> {
  const base = `https://raw.githubusercontent.com/${owner}/${repo}/HEAD`
  const shortName = packageName.replace(/^@.*\//, '')
  const paths = [
    'package.json',
    `packages/${shortName}/package.json`,
    `packages/${packageName.replace(/^@/, '').replace('/', '-')}/package.json`,
  ]
  for (const path of paths) {
    const text = await fetchGitHubRaw(`${base}/${path}`)
    if (!text)
      continue
    try {
      const pkg = JSON.parse(text) as { name?: string }
      if (pkg.name === packageName)
        return true
    }
    catch {}
  }
  return false
}

export async function searchGitHubRepo(packageName: string): Promise<string | null> {
  // Try ungh heuristic first — check if repo name matches package name
  const shortName = packageName.replace(/^@.*\//, '')
  for (const candidate of [packageName.replace(/^@/, '').replace('/', '/'), shortName]) {
    if (!candidate.includes('/')) {
      const unghRes = await $fetch.raw(`https://ungh.cc/repos/${shortName}/${shortName}`).catch(() => null)
      if (unghRes?.ok)
        return `https://github.com/${shortName}/${shortName}`
      continue
    }
    const unghRes = await $fetch.raw(`https://ungh.cc/repos/${candidate}`).catch(() => null)
    if (unghRes?.ok)
      return `https://github.com/${candidate}`
  }

  // Try gh CLI — strip @ to avoid GitHub search syntax issues
  const searchTerm = packageName.replace(/^@/, '')
  if (isGhAvailable()) {
    try {
      const { stdout: json } = spawnSync('gh', ['search', 'repos', searchTerm, '--json', 'fullName', '--limit', '5'], {
        encoding: 'utf-8',
        timeout: 15_000,
      })
      if (!json)
        throw new Error('no output')
      const repos = JSON.parse(json) as Array<{ fullName: string }>
      const match = repos.find(r =>
        r.fullName.toLowerCase().endsWith(`/${packageName.toLowerCase()}`)
        || r.fullName.toLowerCase().endsWith(`/${shortName.toLowerCase()}`),
      )
      if (match)
        return `https://github.com/${match.fullName}`
      for (const candidate of repos) {
        const gh = parseGitHubUrl(`https://github.com/${candidate.fullName}`)
        if (gh && await verifyNpmRepo(gh.owner, gh.repo, packageName))
          return `https://github.com/${candidate.fullName}`
      }
    }
    catch {
      // fall through to REST API
    }
  }

  // Fallback: GitHub REST search API (no auth needed, but rate-limited)
  const query = encodeURIComponent(`${searchTerm} in:name`)
  const data = await $fetch<{ items?: Array<{ full_name: string }> }>(
    `https://api.github.com/search/repositories?q=${query}&per_page=5`,
  ).catch(() => null)
  if (!data?.items?.length)
    return null

  const match = data.items.find(r =>
    r.full_name.toLowerCase().endsWith(`/${packageName.toLowerCase()}`)
    || r.full_name.toLowerCase().endsWith(`/${shortName.toLowerCase()}`),
  )
  if (match)
    return `https://github.com/${match.full_name}`

  for (const candidate of data.items) {
    const gh = parseGitHubUrl(`https://github.com/${candidate.full_name}`)
    if (gh && await verifyNpmRepo(gh.owner, gh.repo, packageName))
      return `https://github.com/${candidate.full_name}`
  }

  return null
}

/**
 * Fetch GitHub repo metadata to get website URL.
 * Pass packageName to check doc overrides first (avoids API call).
 */
export async function fetchGitHubRepoMeta(owner: string, repo: string, packageName?: string): Promise<{ homepage?: string } | null> {
  const override = packageName ? getDocOverride(packageName) : undefined
  if (override?.homepage)
    return { homepage: override.homepage }

  const data = await ghApi<{ homepage?: string }>(`repos/${owner}/${repo}`)
    ?? await $fetch<{ homepage?: string }>(`https://api.github.com/repos/${owner}/${repo}`).catch(() => null)
  return data?.homepage ? { homepage: data.homepage } : null
}

/** Resolve README URL for a GitHub repo, returns ungh:// pseudo-URL or raw URL */
export async function fetchReadme(owner: string, repo: string, subdir?: string, ref?: string): Promise<string | null> {
  const branch = ref || 'main'

  if (!isKnownPrivateRepo(owner, repo)) {
    const unghUrl = subdir
      ? `https://ungh.cc/repos/${owner}/${repo}/files/${branch}/${subdir}/README.md`
      : `https://ungh.cc/repos/${owner}/${repo}/readme${ref ? `?ref=${ref}` : ''}`

    const unghRes = await $fetch.raw(unghUrl).catch(() => null)

    if (unghRes?.ok) {
      return `ungh://${owner}/${repo}${subdir ? `/${subdir}` : ''}${ref ? `@${ref}` : ''}`
    }
  }

  // Fallback to raw.githubusercontent.com — use GET instead of HEAD
  // because raw.githubusercontent.com sometimes returns HTML on HEAD for valid URLs
  const basePath = subdir ? `${subdir}/` : ''
  const branches = ref ? [ref] : ['main', 'master']
  const token = isKnownPrivateRepo(owner, repo) ? getGitHubToken() : null
  const authHeaders: HeadersInit = token ? { Authorization: `token ${token}` } : {}
  for (const b of branches) {
    for (const filename of ['README.md', 'Readme.md', 'readme.md']) {
      const readmeUrl = `https://raw.githubusercontent.com/${owner}/${repo}/${b}/${basePath}${filename}`
      const res = await $fetch.raw(readmeUrl, { headers: authHeaders }).catch(() => null)
      if (res?.ok)
        return readmeUrl
    }
  }

  // Last resort: GitHub API (handles private repos via token auth)
  const refParam = ref ? `?ref=${ref}` : ''
  const endpoint = subdir
    ? `repos/${owner}/${repo}/contents/${subdir}/README.md${refParam}`
    : `repos/${owner}/${repo}/readme${refParam}`
  const apiData = await ghApi<{ download_url?: string }>(endpoint)
  if (apiData?.download_url) {
    markRepoPrivate(owner, repo)
    return apiData.download_url
  }

  return null
}

export interface GitSourceResult {
  /** URL pattern for fetching source */
  baseUrl: string
  /** Git ref (tag) used */
  ref: string
  /** List of source file paths relative to repo root */
  files: string[]
}

const SOURCE_EXTENSIONS = new Set([
  '.ts',
  '.tsx',
  '.mts',
  '.cts',
  '.js',
  '.jsx',
  '.mjs',
  '.cjs',
  '.vue',
  '.svelte',
  '.astro',
])

const EXCLUDE_PATTERNS = [
  /\.test\./,
  /\.spec\./,
  /\.d\.ts$/,
  /__tests__/,
  /__mocks__/,
  /\.config\./,
  /fixtures?\//,
]

function filterSourceFiles(files: string[]): string[] {
  return files.filter((path) => {
    if (!path.startsWith('src/'))
      return false

    const ext = path.slice(path.lastIndexOf('.'))
    if (!SOURCE_EXTENSIONS.has(ext))
      return false
    if (EXCLUDE_PATTERNS.some(p => p.test(path)))
      return false

    return true
  })
}

/** Fetch source files from GitHub repo's src/ folder */
export async function fetchGitSource(owner: string, repo: string, version: string, packageName?: string, repoUrl?: string): Promise<GitSourceResult | null> {
  const branchHint = repoUrl ? extractBranchHint(repoUrl) : undefined
  const tag = await findGitTag(owner, repo, version, packageName, branchHint)
  if (!tag)
    return null

  const files = filterSourceFiles(tag.files)
  if (files.length === 0)
    return null

  return {
    baseUrl: `https://raw.githubusercontent.com/${owner}/${repo}/${tag.ref}`,
    ref: tag.ref,
    files,
  }
}

/** Fetch README content from ungh:// pseudo-URL, file:// URL, or regular URL */
export async function fetchReadmeContent(url: string): Promise<string | null> {
  if (url.startsWith('file://')) {
    const filePath = fileURLToPath(url)
    if (!fsExistsSync(filePath))
      return null
    return fsReadFileSync(filePath, 'utf-8')
  }

  if (url.startsWith('ungh://')) {
    let path = url.replace('ungh://', '')
    let ref = 'main'

    const atIdx = path.lastIndexOf('@')
    if (atIdx !== -1) {
      ref = path.slice(atIdx + 1)
      path = path.slice(0, atIdx)
    }

    const parts = path.split('/')
    const owner = parts[0]
    const repo = parts[1]
    const subdir = parts.slice(2).join('/')

    const unghUrl = subdir
      ? `https://ungh.cc/repos/${owner}/${repo}/files/${ref}/${subdir}/README.md`
      : `https://ungh.cc/repos/${owner}/${repo}/readme?ref=${ref}`

    const text = await $fetch(unghUrl, { responseType: 'text' }).catch(() => null)
    if (!text)
      return null

    try {
      const json = JSON.parse(text) as { markdown?: string, file?: { contents?: string } }
      return json.markdown || json.file?.contents || null
    }
    catch {
      return text
    }
  }

  if (url.includes('raw.githubusercontent.com'))
    return fetchGitHubRaw(url)

  return fetchText(url)
}

/**
 * Resolve a GitHub repo into a ResolvedPackage (no npm registry needed).
 * Fetches repo meta, latest release version, git docs, README, and llms.txt.
 */
export async function resolveGitHubRepo(
  owner: string,
  repo: string,
  onProgress?: (msg: string) => void,
): Promise<ResolvedPackage | null> {
  onProgress?.('Fetching repo metadata')

  const repoUrl = `https://github.com/${owner}/${repo}`
  const meta = await ghApi<{ homepage?: string, description?: string }>(`repos/${owner}/${repo}`)
    ?? await $fetch<{ homepage?: string, description?: string }>(`https://api.github.com/repos/${owner}/${repo}`).catch(() => null)
  const homepage = meta?.homepage || undefined
  const description = meta?.description || undefined

  onProgress?.('Fetching latest release')
  const releases = await fetchUnghReleases(owner, repo)

  let version = 'main'
  let releasedAt: string | undefined
  const latestRelease = releases[0]
  if (latestRelease) {
    version = latestRelease.tag.replace(/^v/, '')
    releasedAt = latestRelease.publishedAt
  }

  onProgress?.('Resolving docs')
  const gitDocs = await fetchGitDocs(owner, repo, version)
  const gitDocsUrl = gitDocs ? `${repoUrl}/tree/${gitDocs.ref}/docs` : undefined
  const gitRef = gitDocs?.ref

  onProgress?.('Fetching README')
  const readmeUrl = await fetchReadme(owner, repo)

  let llmsUrl: string | undefined
  if (homepage) {
    onProgress?.('Checking llms.txt')
    llmsUrl = await fetchLlmsUrl(homepage).catch(() => null) ?? undefined
  }

  if (!gitDocsUrl && !readmeUrl && !llmsUrl)
    return null

  return {
    name: repo,
    version: latestRelease ? version : undefined,
    releasedAt,
    description,
    repoUrl,
    docsUrl: homepage,
    gitDocsUrl,
    gitRef,
    gitDocsFallback: gitDocs?.fallback,
    readmeUrl: readmeUrl ?? undefined,
    llmsUrl,
  }
}
