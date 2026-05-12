/**
 * Content cascade: resolve in-memory docs for a package given its already-resolved URLs.
 *
 * Pure: no fs writes. The caller owns persistence; a single cache-write seam.
 *
 * Cascade order (first-wins for primary, with one supplementary case):
 *   1. Versioned git docs (docs/**\/*.md at the package's git tag)
 *      - if "shallow" and an llms.txt exists, discard and fall through
 *      - on success, also fetch llms.txt as supplementary reference
 *   2. Crawl URL (registry-configured, e.g. motion-v)
 *   3. llms.txt (linked docs, with optional docsUrl crawl augmentation)
 *   4. Crawl docsUrl as a fallback when no doc files found above
 *   5. README
 */

import type { ResolvedPackage } from './types.ts'
import { join } from 'pathe'
import { parseGitHubUrl } from '../core/url.ts'
import {
  downloadLlmsDocs,
  fetchCrawledDocs,
  fetchGitDocs,
  fetchGitHubRaw,
  fetchLlmsTxt,
  fetchReadmeContent,
  filterFrameworkDocs,
  isShallowGitDocs,
  normalizeLlmsLinks,
  toCrawlPattern,
} from './index.ts'

export interface CachedDoc {
  path: string
  content: string
}

export interface IndexDoc {
  id: string
  content: string
  metadata: Record<string, any>
}

export interface ResolvedContent {
  /** Docs to persist to the package cache (path is cache-relative) */
  docs: CachedDoc[]
  /** Docs to feed the embedding index (id may differ from cache path) */
  docsToIndex: IndexDoc[]
  /** Human-readable origin (URL or label) of the chosen primary source */
  docSource: string
  /** Which kind of source won */
  docsType: 'docs' | 'llms.txt' | 'readme'
  /** Non-fatal issues to surface to the user */
  warnings: string[]
}

export interface ResolveContentOptions {
  packageName: string
  resolved: ResolvedPackage
  version: string
  onProgress: (message: string) => void
}

export async function resolveContentDocs(opts: ResolveContentOptions): Promise<ResolvedContent> {
  const { packageName, resolved, version, onProgress } = opts

  const docs: CachedDoc[] = []
  const docsToIndex: IndexDoc[] = []
  const warnings: string[] = []
  let docSource: string = resolved.readmeUrl || 'readme'
  let docsType: 'docs' | 'llms.txt' | 'readme' = 'readme'

  const isFrameworkDoc = (path: string) => filterFrameworkDocs([path], packageName).length > 0

  // 1. Versioned git docs
  if (resolved.gitDocsUrl && resolved.repoUrl) {
    const gh = parseGitHubUrl(resolved.repoUrl)
    if (gh) {
      const r = await tryGitDocs({
        owner: gh.owner,
        repo: gh.repo,
        version,
        packageName,
        repoUrl: resolved.repoUrl,
        llmsUrl: resolved.llmsUrl,
        docsUrl: resolved.docsUrl,
        onProgress,
      })
      if (r) {
        docs.push(...r.docs)
        docsToIndex.push(...r.docsToIndex)
        if (r.warning)
          warnings.push(r.warning)
        if (r.kind === 'git') {
          docSource = r.docSource
          docsType = 'docs'
        }
      }
    }
  }

  // 2. Registry-configured crawl URL
  if (resolved.crawlUrl && docs.length === 0) {
    onProgress('Crawling website')
    const crawled = await fetchCrawledDocs(resolved.crawlUrl, onProgress).catch((err) => {
      warnings.push(`Crawl failed for ${resolved.crawlUrl}: ${err?.message || err}`)
      return []
    })
    if (crawled.length === 0) {
      warnings.push(`Crawl returned 0 docs from ${resolved.crawlUrl}`)
    }
    for (const doc of crawled) {
      if (!isFrameworkDoc(doc.path))
        continue
      docs.push(doc)
      docsToIndex.push({
        id: doc.path,
        content: doc.content,
        metadata: { package: packageName, source: doc.path, type: 'doc' },
      })
    }
    if (docs.length > 0) {
      docSource = resolved.crawlUrl
      docsType = 'docs'
    }
  }

  // 3. llms.txt
  if (resolved.llmsUrl && docs.length === 0) {
    onProgress('Fetching llms.txt')
    const llmsContent = await fetchLlmsTxt(resolved.llmsUrl)
    if (llmsContent) {
      docSource = resolved.llmsUrl
      docsType = 'llms.txt'
      const baseUrl = resolved.docsUrl || new URL(resolved.llmsUrl).origin
      docs.push({ path: 'llms.txt', content: normalizeLlmsLinks(llmsContent.raw, baseUrl) })

      if (llmsContent.links.length > 0) {
        onProgress(`Downloading ${llmsContent.links.length} linked docs`)
        const linked = await downloadLlmsDocs(llmsContent, baseUrl, (_url, done, total) => {
          onProgress(`Downloading linked doc ${done + 1}/${total}`)
        })
        for (const doc of linked) {
          if (!isFrameworkDoc(doc.url))
            continue
          const localPath = doc.url.startsWith('/') ? doc.url.slice(1) : doc.url
          const cachePath = join('docs', ...localPath.split('/'))
          docs.push({ path: cachePath, content: doc.content })
          docsToIndex.push({
            id: doc.url,
            content: doc.content,
            metadata: { package: packageName, source: cachePath, type: 'doc' },
          })
        }
        if (linked.length > 0)
          docsType = 'docs'
      }
    }
  }

  // 4. Crawl docsUrl as fallback when no actual doc files found yet
  if (resolved.docsUrl && !docs.some(d => d.path.startsWith('docs/'))) {
    const crawlPattern = resolved.crawlUrl || toCrawlPattern(resolved.docsUrl)
    onProgress('Crawling docs site')
    const maxPages = resolved.crawlUrl ? 200 : 400
    const crawled = await fetchCrawledDocs(crawlPattern, onProgress, maxPages).catch((err) => {
      warnings.push(`Crawl failed for ${crawlPattern}: ${err?.message || err}`)
      return []
    })
    let added = 0
    for (const doc of crawled) {
      if (!isFrameworkDoc(doc.path))
        continue
      docs.push(doc)
      docsToIndex.push({
        id: doc.path,
        content: doc.content,
        metadata: { package: packageName, source: doc.path, type: 'doc' },
      })
      added++
    }
    if (added > 0) {
      docSource = crawlPattern
      docsType = 'docs'
    }
  }

  // 5. README fallback
  if (resolved.readmeUrl && docs.length === 0) {
    onProgress('Fetching README')
    const content = await fetchReadmeContent(resolved.readmeUrl)
    if (content) {
      docs.push({ path: 'docs/README.md', content })
      docsToIndex.push({
        id: 'README.md',
        content,
        metadata: { package: packageName, source: 'docs/README.md', type: 'doc' },
      })
    }
  }

  return { docs, docsToIndex, docSource, docsType, warnings }
}

interface GitDocsAttempt {
  kind: 'git' | 'discarded'
  docs: CachedDoc[]
  docsToIndex: IndexDoc[]
  docSource: string
  warning?: string
}

async function tryGitDocs(opts: {
  owner: string
  repo: string
  version: string
  packageName: string
  repoUrl: string
  llmsUrl?: string
  docsUrl?: string
  onProgress: (msg: string) => void
}): Promise<GitDocsAttempt | null> {
  const { owner, repo, version, packageName, repoUrl, llmsUrl, docsUrl, onProgress } = opts
  onProgress('Fetching git docs')
  const gitDocs = await fetchGitDocs(owner, repo, version, packageName)
  if (!gitDocs || gitDocs.files.length === 0)
    return null

  let warning: string | undefined
  if (gitDocs.fallback)
    warning = `Docs fetched from ${gitDocs.ref} branch (no tag found for v${version})`

  const BATCH_SIZE = 20
  const results: Array<{ file: string, content: string } | null> = []
  for (let i = 0; i < gitDocs.files.length; i += BATCH_SIZE) {
    const batch = gitDocs.files.slice(i, i + BATCH_SIZE)
    onProgress(`Downloading docs ${Math.min(i + BATCH_SIZE, gitDocs.files.length)}/${gitDocs.files.length} from ${gitDocs.ref}`)
    const batchResults = await Promise.all(
      batch.map(async (file) => {
        const url = `${gitDocs.baseUrl}/${file}`
        const content = await fetchGitHubRaw(url)
        return content ? { file, content } : null
      }),
    )
    results.push(...batchResults)
  }

  const docs: CachedDoc[] = []
  const docsToIndex: IndexDoc[] = []
  for (const r of results) {
    if (!r)
      continue
    const stripped = gitDocs.docsPrefix ? r.file.replace(gitDocs.docsPrefix, '') : r.file
    const cachePath = stripped.startsWith('docs/') ? stripped : `docs/${stripped}`
    docs.push({ path: cachePath, content: r.content })
    docsToIndex.push({
      id: cachePath,
      content: r.content,
      metadata: { package: packageName, source: cachePath, type: 'doc' },
    })
  }

  // Shallow git-docs: if too few docs and llms.txt exists, discard and fall through
  if (isShallowGitDocs(docs.length) && llmsUrl) {
    onProgress(`Shallow git-docs (${docs.length} files), trying llms.txt`)
    return { kind: 'discarded', docs: [], docsToIndex: [], docSource: '', warning }
  }

  // Always cache llms.txt alongside good git-docs as supplementary reference
  if (llmsUrl) {
    onProgress('Caching supplementary llms.txt')
    const llmsContent = await fetchLlmsTxt(llmsUrl)
    if (llmsContent) {
      const baseUrl = docsUrl || new URL(llmsUrl).origin
      docs.push({ path: 'llms.txt', content: normalizeLlmsLinks(llmsContent.raw, baseUrl) })
      if (llmsContent.links.length > 0) {
        onProgress(`Downloading ${llmsContent.links.length} supplementary docs`)
        const supplementary = await downloadLlmsDocs(llmsContent, baseUrl, (_url, done, total) => {
          onProgress(`Downloading supplementary doc ${done + 1}/${total}`)
        })
        for (const doc of supplementary) {
          if (filterFrameworkDocs([doc.url], packageName).length === 0)
            continue
          const localPath = doc.url.startsWith('/') ? doc.url.slice(1) : doc.url
          docs.push({ path: join('llms-docs', ...localPath.split('/')), content: doc.content })
        }
      }
    }
  }

  return {
    kind: 'git',
    docs,
    docsToIndex,
    docSource: `${repoUrl}/tree/${gitDocs.ref}/docs`,
    warning,
  }
}
