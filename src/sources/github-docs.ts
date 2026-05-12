/**
 * GitHub versioned doc discovery, fetch, and validation.
 *
 * Owns: doc-dir scoring heuristic, monorepo prefix discovery, framework-specific
 * filtering, llms.txt cross-validation. This is where "wrong docs picked"
 * regressions live; isolating it from repo/source/readme concerns localizes the
 * bug surface.
 */

import type { LlmsLink } from './types.ts'
import { mapInsert } from '../core/map.ts'
import { extractBranchHint } from '../core/url.ts'
import { findGitTag, listFilesAtRef } from './github-tags.ts'
import { getDocOverride } from './package-registry.ts'

/** Minimum git-doc file count to prefer over llms.txt */
export const MIN_GIT_DOCS = 5

/** True when git-docs exist but are too few to be useful (< MIN_GIT_DOCS) */
export const isShallowGitDocs = (n: number) => n > 0 && n < MIN_GIT_DOCS

export interface GitDocsResult {
  /** URL pattern for fetching docs (use with ref) */
  baseUrl: string
  /** Git ref (tag) used */
  ref: string
  /** List of doc file paths relative to repo root */
  files: string[]
  /** Prefix to strip when normalizing paths to docs/ (e.g. 'apps/evalite-docs/src/content/') for nested monorepo docs */
  docsPrefix?: string
  /** Full repo file tree — only set when discoverDocFiles() heuristic was used (not standard docs/ prefix) */
  allFiles?: string[]
  /** True when ref is a branch (main/master) rather than a version-specific tag */
  fallback?: boolean
}

/** Filter file paths by prefix and md/mdx extension */
function filterDocFiles(files: string[], pathPrefix: string): string[] {
  return files.filter(f => f.startsWith(pathPrefix) && /\.(?:md|mdx)$/.test(f))
}

const FRAMEWORK_NAMES = new Set(['vue', 'react', 'solid', 'angular', 'svelte', 'preact', 'lit', 'qwik'])

/**
 * Filter out docs for other frameworks when the package targets a specific one.
 * e.g. @tanstack/vue-query → keep vue + shared docs, exclude react/solid/angular
 * Uses word-boundary matching to catch all path conventions:
 *   framework/react/, 0.react/, api/ai-react.md, react-native.mdx, etc.
 */
export function filterFrameworkDocs(files: string[], packageName?: string): string[] {
  if (!packageName)
    return files
  const shortName = packageName.replace(/^@.*\//, '')
  const targetFramework = [...FRAMEWORK_NAMES].find(fw => shortName.includes(fw))
  if (!targetFramework)
    return files

  const otherFrameworks = [...FRAMEWORK_NAMES].filter(fw => fw !== targetFramework)
  const excludePattern = new RegExp(`\\b(?:${otherFrameworks.join('|')})\\b`)
  return files.filter(f => !excludePattern.test(f))
}

// ── Doc-dir discovery heuristic ──────────────────────────────────────

/** Known noise paths to exclude from doc discovery */
const NOISE_PATTERNS = [
  /^\.changeset\//,
  /CHANGELOG\.md$/i,
  /CONTRIBUTING\.md$/i,
  /^\.github\//,
]

/** Directories to exclude from "best directory" heuristic */
const EXCLUDE_DIRS = new Set([
  'test',
  'tests',
  '__tests__',
  'fixtures',
  'fixture',
  'examples',
  'example',
  'node_modules',
  '.git',
  'dist',
  'build',
  'coverage',
  'e2e',
  'spec',
  'mocks',
  '__mocks__',
])

/** Directory names that suggest documentation */
const DOC_DIR_BONUS = new Set([
  'docs',
  'documentation',
  'pages',
  'content',
  'website',
  'guide',
  'guides',
  'wiki',
  'manual',
  'api',
])

interface DiscoveredDocs {
  files: string[]
  /** Prefix before 'docs/' to strip when normalizing (e.g. 'apps/evalite-docs/src/content/') */
  prefix: string
}

function hasExcludedDir(path: string): boolean {
  const parts = path.split('/')
  return parts.some(p => EXCLUDE_DIRS.has(p.toLowerCase()))
}

function getPathDepth(path: string): number {
  return path.split('/').filter(Boolean).length
}

function hasDocDirBonus(path: string): boolean {
  const parts = path.split('/')
  return parts.some(p => DOC_DIR_BONUS.has(p.toLowerCase()))
}

/**
 * Score a directory for doc likelihood.
 * Higher = better. Formula: count * nameBonus / depth
 */
function scoreDocDir(dir: string, fileCount: number): number {
  const depth = getPathDepth(dir) || 1
  const nameBonus = hasDocDirBonus(dir) ? 1.5 : 1
  return (fileCount * nameBonus) / depth
}

/**
 * Discover doc files in non-standard locations.
 * First tries to scope to sub-package dir in monorepos.
 * Then looks for clusters of md/mdx files in paths containing /docs/.
 * Falls back to finding the directory with the most markdown files (≥5).
 */
function discoverDocFiles(allFiles: string[], packageName?: string): DiscoveredDocs | null {
  const mdFiles = allFiles
    .filter(f => /\.(?:md|mdx)$/.test(f))
    .filter(f => !NOISE_PATTERNS.some(p => p.test(f)))
    .filter(f => f.includes('/'))

  // Strategy 0: Scope to sub-package in monorepos
  if (packageName?.includes('/')) {
    const shortName = packageName.split('/').pop()!.toLowerCase()
    const subPkgPrefix = `packages/${shortName}/`
    const subPkgFiles = mdFiles.filter(f => f.startsWith(subPkgPrefix))
    if (subPkgFiles.length >= 3)
      return { files: subPkgFiles, prefix: subPkgPrefix }
  }

  // Strategy 1: Look for /docs/ clusters (existing behavior)
  const docsGroups = new Map<string, string[]>()

  for (const file of mdFiles) {
    const docsIdx = file.lastIndexOf('/docs/')
    if (docsIdx === -1)
      continue

    const prefix = file.slice(0, docsIdx + '/docs/'.length)
    mapInsert(docsGroups, prefix, () => []).push(file)
  }

  if (docsGroups.size > 0) {
    const largest = [...docsGroups.entries()].sort((a, b) => b[1].length - a[1].length)[0]!
    if (largest[1].length >= 3) {
      const fullPrefix = largest[0]
      const docsIdx = fullPrefix.lastIndexOf('docs/')
      const stripPrefix = docsIdx > 0 ? fullPrefix.slice(0, docsIdx) : ''
      return { files: largest[1], prefix: stripPrefix }
    }
  }

  // Strategy 2: Find best directory by file count (for non-standard structures)
  const dirGroups = new Map<string, string[]>()

  for (const file of mdFiles) {
    if (hasExcludedDir(file))
      continue

    const lastSlash = file.lastIndexOf('/')
    if (lastSlash === -1)
      continue

    const dir = file.slice(0, lastSlash + 1)
    mapInsert(dirGroups, dir, () => []).push(file)
  }

  if (dirGroups.size === 0)
    return null

  const scored = Array.from(dirGroups.entries(), ([dir, files]) => ({ dir, files, score: scoreDocDir(dir, files.length) }))
    .filter(d => d.files.length >= 5)
    .sort((a, b) => b.score - a.score)

  if (scored.length === 0)
    return null

  const best = scored[0]!
  return { files: best.files, prefix: best.dir }
}

/** List markdown files in a folder at a specific git ref */
async function listDocsAtRef(owner: string, repo: string, ref: string, pathPrefix = 'docs/'): Promise<string[]> {
  const files = await listFilesAtRef(owner, repo, ref)
  return filterDocFiles(files, pathPrefix)
}

// ── Public fetch ─────────────────────────────────────────────────────

/**
 * Fetch versioned docs from GitHub repo's docs/ folder.
 * Pass packageName to check doc overrides (e.g. vue -> vuejs/docs).
 */
export async function fetchGitDocs(owner: string, repo: string, version: string, packageName?: string, repoUrl?: string): Promise<GitDocsResult | null> {
  const override = packageName ? getDocOverride(packageName) : undefined
  if (override) {
    const ref = override.ref || 'main'
    const fallback = !override.ref
    const files = await listDocsAtRef(override.owner, override.repo, ref, `${override.path}/`)
    if (files.length === 0)
      return null
    return {
      baseUrl: `https://raw.githubusercontent.com/${override.owner}/${override.repo}/${ref}`,
      ref,
      files,
      fallback,
      docsPrefix: `${override.path}/` !== 'docs/' ? `${override.path}/` : undefined,
    }
  }

  const branchHint = repoUrl ? extractBranchHint(repoUrl) : undefined
  const tag = await findGitTag(owner, repo, version, packageName, branchHint)
  if (!tag)
    return null

  let docs = filterDocFiles(tag.files, 'docs/')
  let docsPrefix: string | undefined
  let allFiles: string[] | undefined

  if (docs.length === 0) {
    const discovered = discoverDocFiles(tag.files, packageName)
    if (discovered) {
      docs = discovered.files
      docsPrefix = discovered.prefix || undefined
      allFiles = tag.files
    }
  }

  docs = filterFrameworkDocs(docs, packageName)

  if (docs.length === 0)
    return null

  return {
    baseUrl: `https://raw.githubusercontent.com/${owner}/${repo}/${tag.ref}`,
    ref: tag.ref,
    files: docs,
    docsPrefix,
    allFiles,
    fallback: tag.fallback,
  }
}

// ── llms.txt cross-validation ────────────────────────────────────────

/** Strip file extension (.md, .mdx) and leading slash from a path */
function normalizePath(p: string): string {
  return p.replace(/^\//, '').replace(/\.(?:md|mdx)$/, '')
}

/**
 * Validate that discovered git docs are relevant by cross-referencing llms.txt links
 * against the repo file tree. Uses extensionless suffix matching to handle monorepo nesting.
 *
 * Returns { isValid, matchRatio } where isValid = matchRatio >= 0.3
 */
export function validateGitDocsWithLlms(
  llmsLinks: LlmsLink[],
  repoFiles: string[],
): { isValid: boolean, matchRatio: number } {
  if (llmsLinks.length === 0)
    return { isValid: true, matchRatio: 1 }

  const sample = llmsLinks.slice(0, 10)

  const normalizedLinks = sample.map((link) => {
    let path = link.url
    if (path.startsWith('http')) {
      try {
        path = new URL(path).pathname
      }
      catch { /* keep as-is */ }
    }
    return normalizePath(path)
  })

  const repoNormalized = new Set(repoFiles.map(normalizePath))

  let matches = 0
  for (const linkPath of normalizedLinks) {
    for (const repoPath of repoNormalized) {
      if (repoPath === linkPath || repoPath.endsWith(`/${linkPath}`)) {
        matches++
        break
      }
    }
  }

  const matchRatio = matches / sample.length
  return { isValid: matchRatio >= 0.3, matchRatio }
}
