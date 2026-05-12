/**
 * Crosscheck — runs all matrix packages and produces a static comparison table.
 *
 * No assertions, no throws on mismatch. Pure data collection + formatting.
 *
 * Usage:
 *   npx tsx test/e2e/crosscheck.ts           # print to stdout
 *   npx tsx test/e2e/crosscheck.ts --md      # markdown table
 *   npx tsx test/e2e/crosscheck.ts --json    # raw JSON
 */

import type { PackageSpec } from './matrix'
import type { PipelineResult } from './pipeline'
import { existsSync } from 'node:fs'
import { getPackageDbPath } from '../../src/cache'
import { getShippedSkills } from '../../src/core/prepare'
import { search } from '../../src/retriv'
import { PACKAGES } from './matrix'
import { parseFrontmatter, runPipeline } from './pipeline'

// ── Types ──────────────────────────────────────────────────────────

export interface CrosscheckRow {
  name: string
  status: 'ok' | 'error'
  error?: string

  // resolution
  npm: boolean
  repo: boolean
  docsUrl: boolean
  gitDocs: boolean
  llmsTxt: boolean
  readme: boolean
  shipped: boolean

  // cache
  docsType: string | null
  cachedDocs: number

  // search
  searchDb: boolean
  searchHits: number | null

  // skill
  skillValid: boolean
  globs: string | null

  // version info
  version: string | null
  releasedAt: string | null
}

// ── Collect ─────────────────────────────────────────────────────────

async function collectRow(spec: PackageSpec): Promise<CrosscheckRow> {
  const row: CrosscheckRow = {
    name: spec.name,
    status: 'ok',
    npm: false,
    repo: false,
    docsUrl: false,
    gitDocs: false,
    llmsTxt: false,
    readme: false,
    shipped: false,
    docsType: null,
    cachedDocs: 0,
    searchDb: false,
    searchHits: null,
    skillValid: false,
    globs: null,
    version: null,
    releasedAt: null,
  }

  let result: PipelineResult
  try {
    result = await runPipeline(spec.name)
  }
  catch (err) {
    row.status = 'error'
    row.error = (err as Error).message
    return row
  }

  const r = result.resolved

  // resolution
  row.npm = !!r.name
  row.repo = !!r.repoUrl
  row.docsUrl = !!r.docsUrl
  row.gitDocs = !!r.gitDocsUrl
  row.llmsTxt = !!r.llmsUrl
  row.readme = !!r.readmeUrl

  // shipped
  if (spec.expectShipped) {
    const shipped = getShippedSkills(spec.name, process.cwd())
    row.shipped = shipped.length > 0
  }

  // cache
  row.docsType = result.docsType
  row.cachedDocs = result.cachedDocsCount

  // search
  const dbPath = getPackageDbPath(spec.name, result.version)
  row.searchDb = existsSync(dbPath)
  if (row.searchDb && spec.searchQuery) {
    const hits = await search(spec.searchQuery.query, { dbPath, limit: 5 }).catch(() => [])
    row.searchHits = hits.length
  }

  // skill
  const fm = parseFrontmatter(result.skillMd)
  row.skillValid = !!fm.name && !!fm.description
  row.globs = null
  row.version = result.version
  row.releasedAt = fm.releasedAt || null

  return row
}

export async function crosscheck(packages: PackageSpec[] = PACKAGES): Promise<CrosscheckRow[]> {
  const settled = await Promise.allSettled(packages.map(collectRow))
  return settled.map((s, i) =>
    s.status === 'fulfilled'
      ? s.value
      : { name: packages[i]!.name, status: 'error' as const, error: (s.reason as Error).message, npm: false, repo: false, docsUrl: false, gitDocs: false, llmsTxt: false, readme: false, shipped: false, docsType: null, cachedDocs: 0, searchDb: false, searchHits: null, skillValid: false, globs: null, version: null, releasedAt: null },
  )
}

// ── Format ──────────────────────────────────────────────────────────

const B = (v: boolean) => v ? '✓' : '-'
const N = (v: number | null) => v === null ? '-' : String(v)

export function formatTable(rows: CrosscheckRow[]): string {
  const headers = ['Package', 'npm', 'repo', 'docs', 'git', 'llms', 'readme', 'shipped', 'type', 'cached', 'search', 'skill', 'globs', 'version']

  const data = rows.map(r => [
    r.status === 'error' ? `${r.name} ✗` : r.name,
    B(r.npm),
    B(r.repo),
    B(r.docsUrl),
    B(r.gitDocs),
    B(r.llmsTxt),
    B(r.readme),
    B(r.shipped),
    r.docsType || '-',
    N(r.cachedDocs),
    r.searchHits !== null ? N(r.searchHits) : B(r.searchDb),
    B(r.skillValid),
    r.globs || '-',
    r.version || '-',
  ])

  // compute column widths
  const widths = headers.map((h, i) =>
    Math.max(h.length, ...data.map(row => row[i]!.length)),
  )

  const pad = (s: string, w: number) => s + ' '.repeat(w - s.length)
  const sep = widths.map(w => '-'.repeat(w))

  const lines = [
    headers.map((h, i) => pad(h, widths[i]!)).join('  '),
    sep.join('  '),
    ...data.map(row => row.map((cell, i) => pad(cell, widths[i]!)).join('  ')),
  ]

  return lines.join('\n')
}

export function formatMarkdown(rows: CrosscheckRow[]): string {
  const headers = ['Package', 'npm', 'repo', 'docs', 'git', 'llms', 'readme', 'shipped', 'type', 'cached', 'search', 'skill', 'globs', 'version']

  const data = rows.map(r => [
    r.status === 'error' ? `\`${r.name}\` ✗` : `\`${r.name}\``,
    B(r.npm),
    B(r.repo),
    B(r.docsUrl),
    B(r.gitDocs),
    B(r.llmsTxt),
    B(r.readme),
    B(r.shipped),
    r.docsType || '-',
    N(r.cachedDocs),
    r.searchHits !== null ? N(r.searchHits) : B(r.searchDb),
    B(r.skillValid),
    r.globs ? `\`${r.globs}\`` : '-',
    r.version ? `\`${r.version}\`` : '-',
  ])

  const row = (cells: string[]) => `| ${cells.join(' | ')} |`
  const divider = `| ${headers.map(h => '-'.repeat(h.length)).join(' | ')} |`

  return [row(headers), divider, ...data.map(row)].join('\n')
}

export function formatJson(rows: CrosscheckRow[]): string {
  return JSON.stringify(rows, null, 2)
}

// ── CLI ─────────────────────────────────────────────────────────────

async function main() {
  const flag = process.argv[2]
  console.log(`Running crosscheck for ${PACKAGES.length} packages...\n`)

  const rows = await crosscheck()

  if (flag === '--json')
    console.log(formatJson(rows))
  else if (flag === '--md')
    console.log(formatMarkdown(rows))
  else
    console.log(formatTable(rows))

  const errors = rows.filter(r => r.status === 'error')
  if (errors.length) {
    console.log(`\n${errors.length} package(s) failed:`)
    for (const e of errors)
      console.log(`  ${e.name}: ${e.error}`)
  }
}

// Run if executed directly
const isMain = process.argv[1]?.endsWith('crosscheck.ts') || process.argv[1]?.endsWith('crosscheck')
if (isMain)
  main()
