/**
 * llms.txt fetching and parsing
 */

import type { FetchedDoc, LlmsContent, LlmsLink } from './types.ts'
import pLimit from 'p-limit'
import { extractLinks } from '../core/markdown.ts'
import { isSafeUrl } from '../core/url.ts'
import { fetchText, verifyUrl } from './utils.ts'

/**
 * Check for llms.txt at a docs URL, returns the llms.txt URL if found
 */
export async function fetchLlmsUrl(docsUrl: string): Promise<string | null> {
  const origin = new URL(docsUrl).origin
  const llmsUrl = `${origin}/llms.txt`
  if (await verifyUrl(llmsUrl))
    return llmsUrl
  return null
}

/**
 * Fetch and parse llms.txt content
 */
export async function fetchLlmsTxt(url: string): Promise<LlmsContent | null> {
  const content = await fetchText(url)
  if (!content || content.length < 50)
    return null

  return {
    raw: content,
    links: parseMarkdownLinks(content),
  }
}

/**
 * Parse markdown links from llms.txt to get .md file paths
 */
export function parseMarkdownLinks(content: string): LlmsLink[] {
  return extractLinks(content).filter(l => l.url.endsWith('.md'))
}

export async function downloadLlmsDocs(
  llmsContent: LlmsContent,
  baseUrl: string,
  onProgress?: (url: string, index: number, total: number) => void,
): Promise<FetchedDoc[]> {
  const limit = pLimit(5)
  let completed = 0

  const results = await Promise.all(
    llmsContent.links.map(link => limit(async () => {
      const url = link.url.startsWith('http')
        ? link.url
        : `${baseUrl.replace(/\/$/, '')}${link.url.startsWith('/') ? '' : '/'}${link.url}`

      if (!isSafeUrl(url))
        return null

      const content = await fetchText(url)
      onProgress?.(link.url, ++completed, llmsContent.links.length)
      if (content && content.length > 100) {
        // Normalize full URLs to just the pathname for cache paths
        const docUrl = link.url.startsWith('http') ? new URL(link.url).pathname : link.url
        return { url: docUrl, title: link.title, content } as FetchedDoc
      }
      return null
    })),
  )

  return results.filter((d): d is FetchedDoc => d !== null)
}

/**
 * Normalize llms.txt links to relative paths for local access
 * Handles: absolute URLs, root-relative paths, and relative paths
 */
export function normalizeLlmsLinks(content: string, baseUrl?: string): string {
  let normalized = content

  // Handle absolute URLs: https://example.com/docs/foo.md → ./docs/foo.md
  if (baseUrl) {
    const base = baseUrl.replace(/\/$/, '')
    const escaped = base.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
    normalized = normalized.replace(
      new RegExp(`\\]\\(${escaped}(/[^)]+\\.md)\\)`, 'g'),
      '](./docs$1)',
    )
  }

  // Handle root-relative paths: /foo.md → ./docs/foo.md
  normalized = normalized.replace(/\]\(\/([^)]+\.md)\)/g, '](./docs/$1)')

  return normalized
}

/**
 * Extract sections from llms-full.txt by URL patterns
 * Format: ---\nurl: /path.md\n---\n<content>\n\n---\nurl: ...
 */
export function extractSections(content: string, patterns: string[]): string | null {
  const sections: string[] = []
  const parts = content.split(/\n---\n/)

  for (const part of parts) {
    const urlMatch = part.match(/^url: *(\S.*)$/m)
    if (!urlMatch)
      continue

    const url = urlMatch[1]!
    if (patterns.some(p => url.includes(p))) {
      const contentStart = part.indexOf('\n', part.indexOf('url:'))
      if (contentStart > -1) {
        sections.push(part.slice(contentStart + 1))
      }
    }
  }

  if (sections.length === 0)
    return null
  return sections.join('\n\n---\n\n')
}
