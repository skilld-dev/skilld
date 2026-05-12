/**
 * Shared pipeline runner for e2e tests.
 *
 * Extracted so both sync.test.ts and crosscheck.ts can use it.
 */

import type { ResolveAttempt, ResolvedPackage } from '../../src/sources'
import { existsSync, readdirSync, statSync } from 'node:fs'
import pLimit from 'p-limit'
import { join } from 'pathe'
import { computeSkillDirName } from '../../src/agent'
import {
  createReferenceCache,
  ensureCacheDir,
  getCacheDir,
  getPackageDbPath,
} from '../../src/cache'
import { parseGitHubUrl } from '../../src/core/url'
import { createIndexDirect } from '../../src/retriv'
import {
  downloadLlmsDocs,
  fetchGitDocs,
  fetchLlmsTxt,
  fetchReadmeContent,
  isShallowGitDocs,
  normalizeLlmsLinks,
  resolvePackageDocsWithAttempts,
} from '../../src/sources'

// ── Types ──────────────────────────────────────────────────────────

export interface PipelineResult {
  resolved: ResolvedPackage
  attempts: ResolveAttempt[]
  version: string
  docsType: 'llms.txt' | 'readme' | 'docs'
  cachedDocsCount: number
  cachedFiles: string[]
  skillMd: string
}

// ── Helpers ─────────────────────────────────────────────────────────

/** List all doc files (.md, .txt) in cache dir as relative paths */
export function listDocFiles(dir: string): string[] {
  if (!existsSync(dir))
    return []
  const files: string[] = []
  function walk(d: string, prefix = '') {
    for (const entry of readdirSync(d, { withFileTypes: true })) {
      const rel = prefix ? `${prefix}/${entry.name}` : entry.name
      if (entry.isDirectory())
        walk(join(d, entry.name), rel)
      else if (entry.name.endsWith('.md') || entry.name.endsWith('.mdx') || entry.name.endsWith('.txt'))
        files.push(rel)
    }
  }
  walk(dir)
  return files.sort()
}

export function parseFrontmatter(content: string): Record<string, string> {
  const match = content.match(/^---\n([\s\S]*?)\n---/)
  if (!match)
    return {}
  const result: Record<string, string> = {}
  for (const line of match[1]!.split('\n')) {
    const idx = line.indexOf(':')
    if (idx > 0) {
      const key = line.slice(0, idx).trim()
      const value = line.slice(idx + 1).trim().replace(/^["']|["']$/g, '')
      result[key] = value
    }
  }
  return result
}

// Serialize indexing: concurrent ONNX model loads cause silent failures
const indexLimit = pLimit(1)

/** Empty search.db (schema only, no docs) is exactly 61440 bytes */
export function hasValidSearchDb(dbPath: string): boolean {
  if (!existsSync(dbPath))
    return false
  return statSync(dbPath).size > 61440
}

// ── Pipeline ────────────────────────────────────────────────────────

/**
 * Run the full sync pipeline for a package (minus LLM).
 * Uses real cache — idempotent across runs.
 */
export async function runPipeline(name: string): Promise<PipelineResult> {
  ensureCacheDir()
  const { package: resolved, attempts } = await resolvePackageDocsWithAttempts(name)
  if (!resolved) {
    throw new Error(
      `Failed to resolve: ${name}\n${attempts.map(a => `  ${a.source}: ${a.status} ${a.message || ''}`).join('\n')}`,
    )
  }

  const version = resolved.version || 'latest'

  let docsType: 'llms.txt' | 'readme' | 'docs' = 'readme'
  let cachedDocsCount: number
  let cachedFiles: string[]

  const cache = createReferenceCache(name, version)
  const cacheDir = getCacheDir(name, version)
  const cachedDocFiles = cache.has() ? listDocFiles(cacheDir) : []
  // Consider cached if we have docs (not just changelogs)
  // Cache valid when docs use normalized docs/ prefix or llms.txt-only.
  // Stale caches (src/, packages/, www/ prefixes) need refetch.
  const hasStaleFiles = cachedDocFiles.some(f =>
    f.includes('/') && !f.startsWith('docs/') && !f.startsWith('llms-docs/'),
  )
  const hasCachedDocs = !hasStaleFiles && cachedDocFiles.some(f =>
    f.startsWith('docs/') || f === 'llms.txt',
  )

  if (hasCachedDocs) {
    cachedFiles = cachedDocFiles
    cachedDocsCount = cachedFiles.length

    if (existsSync(join(cacheDir, 'llms.txt'))) {
      docsType = 'llms.txt'
    }
    if (cachedDocFiles.some(f =>
      f.startsWith('docs/') && !f.includes('README'),
    )) { docsType = 'docs' }

    // Search index for cached docs was built during the original sync run.
    // Don't rebuild here — too slow for large doc sets (nuxt 708, astro 394).
    // Search tests use skip guards when no valid index exists.
  }
  else {
    const cachedDocs: Array<{ path: string, content: string }> = []
    const docsToIndex: Array<{ id: string, content: string, metadata: Record<string, any> }> = []

    // Try git docs
    if (resolved.gitDocsUrl && resolved.repoUrl) {
      const gh = parseGitHubUrl(resolved.repoUrl)
      if (gh) {
        const gitDocs = await fetchGitDocs(gh.owner, gh.repo, version, name)
        if (gitDocs?.files.length) {
          const BATCH = 20
          for (let i = 0; i < gitDocs.files.length; i += BATCH) {
            const batch = gitDocs.files.slice(i, i + BATCH)
            const results = await Promise.all(
              batch.map(async (file) => {
                const url = `${gitDocs.baseUrl}/${file}`
                const res = await fetch(url, { headers: { 'User-Agent': 'skilld/1.0' } }).catch(() => null)
                if (!res?.ok)
                  return null
                return { file, content: await res.text() }
              }),
            )
            for (const r of results) {
              if (r) {
                // Normalize paths same as commands/sync/pipeline.ts: strip docsPrefix, ensure docs/ prefix
                const stripped = gitDocs.docsPrefix ? r.file.replace(gitDocs.docsPrefix, '') : r.file
                const cachePath = stripped.startsWith('docs/') ? stripped : `docs/${stripped}`
                cachedDocs.push({ path: cachePath, content: r.content })
                docsToIndex.push({ id: cachePath, content: r.content, metadata: { package: name, source: cachePath } })
              }
            }
          }
          if (cachedDocs.length > 0) {
            // Shallow git-docs: if < threshold and llms.txt exists, discard and fall through
            if (isShallowGitDocs(cachedDocs.length) && resolved.llmsUrl) {
              cachedDocs.length = 0
              docsToIndex.length = 0
            }
            else {
              docsType = 'docs'

              // Always cache llms.txt alongside good git-docs as supplementary reference
              if (resolved.llmsUrl) {
                const llmsContent = await fetchLlmsTxt(resolved.llmsUrl)
                if (llmsContent) {
                  const baseUrl = resolved.docsUrl || new URL(resolved.llmsUrl).origin
                  cachedDocs.push({ path: 'llms.txt', content: normalizeLlmsLinks(llmsContent.raw, baseUrl) })
                  if (llmsContent.links.length > 0) {
                    const docs = await downloadLlmsDocs(llmsContent, baseUrl)
                    for (const doc of docs) {
                      const localPath = doc.url.startsWith('/') ? doc.url.slice(1) : doc.url
                      cachedDocs.push({ path: `llms-docs/${localPath}`, content: doc.content })
                    }
                  }
                }
              }
            }
          }
        }
      }
    }

    // Try llms.txt
    if (resolved.llmsUrl && cachedDocs.length === 0) {
      const llmsContent = await fetchLlmsTxt(resolved.llmsUrl)
      if (llmsContent) {
        const baseUrl = resolved.docsUrl || new URL(resolved.llmsUrl).origin
        cachedDocs.push({ path: 'llms.txt', content: normalizeLlmsLinks(llmsContent.raw, baseUrl) })
        docsType = 'llms.txt'

        if (llmsContent.links.length > 0) {
          const docs = await downloadLlmsDocs(llmsContent, baseUrl)
          for (const doc of docs) {
            const localPath = doc.url.startsWith('/') ? doc.url.slice(1) : doc.url
            cachedDocs.push({ path: `docs/${localPath}`, content: doc.content })
            docsToIndex.push({ id: doc.url, content: doc.content, metadata: { package: name, source: `docs/${localPath}` } })
          }
          if (docs.length > 0)
            docsType = 'docs'
        }
      }
    }

    // Fallback README
    if (resolved.readmeUrl && cachedDocs.length === 0) {
      const content = await fetchReadmeContent(resolved.readmeUrl)
      if (content) {
        cachedDocs.push({ path: 'docs/README.md', content })
        docsToIndex.push({ id: 'README.md', content, metadata: { package: name, source: 'docs/README.md' } })
      }
    }

    if (cachedDocs.length > 0) {
      cache.write(cachedDocs)
    }

    const dbPath = getPackageDbPath(name, version)
    if (docsToIndex.length > 0 && !hasValidSearchDb(dbPath)) {
      await indexLimit(async () => {
        if (hasValidSearchDb(dbPath))
          return
        await createIndexDirect(docsToIndex, { dbPath })
      })
    }

    const cacheDir = getCacheDir(name, version)
    cachedFiles = listDocFiles(cacheDir)
    cachedDocsCount = cachedFiles.length
  }

  // Generate SKILL.md frontmatter (pure, same as sync command)
  const skillDirName = computeSkillDirName(name)
  const description = `Using code importing from "${name}". Researching or debugging ${name}.`

  const fmLines = [
    '---',
    `name: ${skillDirName}`,
    `description: ${description}`,
  ]
  if (version) {
    fmLines.push('metadata:')
    fmLines.push(`  version: "${version}"`)
  }
  fmLines.push('---', '')

  return { resolved, attempts, version, docsType, cachedDocsCount, cachedFiles, skillMd: fmLines.join('\n') }
}
