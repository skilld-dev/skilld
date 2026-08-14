import type { SearchSnippet } from '../retriv/index.ts'
import { styleText } from 'node:util'
import * as p from '@clack/prompts'

const STATIC_REGEX_1 = /https?:\/\/github\.com\//
const STATIC_REGEX_2 = /^#+\s*$/

export function todayIsoDate(): string {
  return new Date().toISOString().split('T')[0]!
}

export function timeAgo(iso?: string): string {
  if (!iso)
    return ''
  const diff = Date.now() - new Date(iso).getTime()
  const days = Math.floor(diff / 86400000)
  if (days <= 0)
    return 'today'
  if (days === 1)
    return '1d ago'
  if (days < 7)
    return `${days}d ago`
  if (days < 30)
    return `${Math.floor(days / 7)}w ago`
  return `${Math.floor(days / 30)}mo ago`
}

export function formatSource(source?: string): string {
  if (!source)
    return ''
  if (source === 'shipped')
    return 'shipped'
  if (source.includes('llms.txt'))
    return 'llms.txt'
  if (source.includes('github.com'))
    return source.replace(STATIC_REGEX_1, '')
  return source
}

export function formatDuration(ms: number): string {
  if (ms < 1000)
    return `${Math.round(ms)}ms`
  return `${(ms / 1000).toFixed(1)}s`
}

/** Spinner wrapper that appends elapsed time on stop (ms when sub-second) */
export function timedSpinner() {
  const spin = p.spinner()
  let startedAt = 0
  return {
    start(msg: string) {
      startedAt = Date.now()
      spin.start(msg)
    },
    message(msg: string) {
      spin.message(msg)
    },
    stop(msg: string) {
      const elapsed = startedAt ? formatDuration(Date.now() - startedAt) : ''
      spin.stop(elapsed ? `${msg} ${styleText('gray', `[${elapsed}]`)}` : msg)
    },
  }
}

export function highlightTerms(content: string, terms: string[]): string {
  if (terms.length === 0)
    return content
  // Sort by length desc to match longer terms first
  const sorted = terms.toSorted((a, b) => b.length - a.length)
  const pattern = new RegExp(`(${sorted.map(t => t.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|')})`, 'gi')
  return content.replace(pattern, (m: string) => styleText('yellow', m))
}

/** Format a normalized score (0-100) with color */
export function scoreLabel(pct: number): string {
  const color = pct >= 70 ? 'green' : pct >= 40 ? 'yellow' : 'gray'
  return styleText(color, `${pct}%`)
}

/** Normalize raw cosine similarity scores to 0-100 relative to the best match */
export function normalizeScores(results: SearchSnippet[]): Map<SearchSnippet, number> {
  const map = new Map<SearchSnippet, number>()
  const max = results.reduce((m, r) => Math.max(m, r.score), 0)
  for (const r of results)
    map.set(r, max > 0 ? Math.round((r.score / max) * 100) : 0)
  return map
}

function snippetRefPath(r: SearchSnippet): string {
  const root = r.referenceRoot || `.claude/skills/${r.package}/.skilld`
  return `${root}/${r.source}`
}

export function formatSnippet(r: SearchSnippet, versions?: Map<string, string>, pct?: number): string {
  const refPath = snippetRefPath(r)
  const lineRange = r.lineStart === r.lineEnd ? `L${r.lineStart}` : `L${r.lineStart}-${r.lineEnd}`
  const score = pct != null ? scoreLabel(pct) : styleText('gray', r.score.toFixed(2))
  const version = versions?.get(r.package)
  const pkgLabel = version ? `${r.package}@${version}` : r.package

  const scopeStr = r.scope?.length ? `${r.scope.map(e => e.name).join('.')} → ` : ''
  const entityStr = r.entities?.map(e => e.signature || `${e.type} ${e.name}`).join(', ')
  const highlighted = highlightTerms(r.content, r.highlights)

  return [
    `${pkgLabel} ${score}${entityStr ? `  ${styleText('cyan', `${scopeStr}${entityStr}`)}` : ''}`,
    styleText('gray', `${refPath}:${lineRange}`),
    `  ${highlighted.replace(/\n/g, '\n  ')}`,
  ].join('\n')
}

/** Compact 2-line format for interactive search list */
export function formatCompactSnippet(r: SearchSnippet, cols: number): { title: string, path: string, preview: string } {
  const entityStr = r.entities?.length
    ? r.entities.map(e => e.signature || e.name).join(', ')
    : ''
  const scopeStr = r.scope?.length ? `${r.scope.map(e => e.name).join('.')} → ` : ''
  const title = entityStr ? `${scopeStr}${entityStr}` : r.source.split('/').pop() || r.source

  const refPath = snippetRefPath(r)
  const lineRange = r.lineStart === r.lineEnd ? `L${r.lineStart}` : `L${r.lineStart}-${r.lineEnd}`
  const path = `${refPath}:${lineRange}`

  // First meaningful line as preview (skip empty, frontmatter delimiters, headings-only)
  const maxPreview = cols - 6
  const firstLine = r.content.split('\n').find(l => l.trim() && l.trim() !== '---' && !STATIC_REGEX_2.test(l.trim())) || ''
  const preview = firstLine.length > maxPreview ? `${firstLine.slice(0, maxPreview - 1)}…` : firstLine

  return { title, path, preview }
}
