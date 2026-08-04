/**
 * Bucketing pipeline: deterministic first, LLM only for flagged releases.
 *
 * Each release is bucketed by the free md4x parser (see buckets.ts). Releases
 * whose coverage is too low (prose / custom format) are re-bucketed by the LLM:
 * the model rewrites the note into the canonical `## Breaking/Features/Fixes/
 * Improvements` sections, which we then run back through the SAME deterministic
 * parser — one parser, no bespoke LLM-output parsing. Across a representative
 * 53-package sample the deterministic path already covers ~93% of changes, so
 * the LLM fork fires on only a handful of repos.
 */

import type { Buckets, BucketType, HeadingRule } from './buckets.ts'
import { BUCKET_TYPES, bucketCounts, mergeBuckets, normalizeHeading, parseReleaseToBuckets } from './buckets.ts'

const BUCKET_HEADINGS: Record<BucketType, string> = {
  breaking: '## Breaking changes',
  features: '## New features',
  fixes: '## Fixes',
  improvements: '## Improvements',
}

/** Render the given bucket types as Markdown sections. */
function formatBuckets(buckets: Buckets, types: BucketType[]): string {
  return types
    .filter(type => buckets[type].length)
    .map(type => `${BUCKET_HEADINGS[type]}\n${buckets[type].map(item => `- ${item}`).join('\n')}`)
    .join('\n\n')
}

/** All four buckets — full reference rendering. */
export function formatBucketsAsMarkdown(buckets: Buckets): string {
  return formatBuckets(buckets, ['breaking', 'features', 'fixes', 'improvements'])
}

/**
 * Runbook input: only the actionable buckets (breaking changes + code-affecting
 * features). Fixes/improvements are passed to synthesis as counts, not steps.
 */
export function formatBucketsForRunbook(buckets: Buckets): string {
  return formatBuckets(buckets, ['breaking', 'features'])
}

export interface ReleaseInput {
  version: string
  markdown: string
}

/** One release's buckets — powers per-version sections + the from-version window. */
export interface VersionBuckets {
  version: string
  buckets: Buckets
  counts: Record<BucketType, number>
}

export interface BucketPipelineResult {
  buckets: Buckets
  counts: Record<BucketType, number>
  releases: number
  /** Per-version buckets, newest-first — for windowing/filtering in the UI. */
  perVersion: VersionBuckets[]
  /** Per-release LLM re-bucket calls (the expensive fork). */
  llmCalls: number
  /** Pattern-inference calls (0 or 1). */
  inferenceCalls: number
}

export interface BucketPipelineOptions {
  packageName: string
  /** Runs one LLM completion; omit to stay fully deterministic (no fork). */
  complete?: (prompt: string) => Promise<string>
  flagThreshold?: number
  concurrency?: number
  onProgress?: (message: string) => void
}

function LLM_BUCKET_PROMPT(pkg: string, md: string): string {
  return `Re-express the migration-relevant changes in this ${pkg} release note as Markdown under EXACTLY these headings (omit a heading that has no items):

## Breaking
## Features
## Fixes
## Improvements

Rules: one concise bullet per change; copy any code/diff the note shows; ignore contributors, links, and version-bump noise; do not invent changes. Output only the Markdown sections.

Release note:
${md}`
}

// Only attempt pattern inference when enough releases flag that one cheap
// learn-the-vocabulary call can beat many per-release calls.
const INFER_MIN_FLAGGED = 3
const BULLET_PREFIX_RE = /^[-*]\s*/
const REGEX_ESCAPE_RE = /[.*+?^${}()|[\]\\]/g

/**
 * Pattern inference: one LLM call that maps a repo's unrecognized section
 * headings to buckets, turning many per-release LLM calls into a single
 * learn-the-vocabulary call. Helps repos with consistent non-standard headings;
 * for genuinely freeform prose it simply learns little and we fall back to the
 * per-release fork. Returns repo-specific HeadingRules (anchored to the
 * normalized heading text), `ignore` headings are dropped.
 */
export async function inferHeadingRules(
  packageName: string,
  unmatchedHeadings: string[],
  complete: (prompt: string) => Promise<string>,
): Promise<HeadingRule[]> {
  const distinct = [...new Set(unmatchedHeadings.map(h => h.trim()).filter(Boolean))].slice(0, 40)
  if (!distinct.length)
    return []

  const prompt = `These are section headings from \`${packageName}\` release notes that an automated classifier could not categorize. Map EACH to exactly one of: breaking, features, fixes, improvements, ignore (use "ignore" for navigation/wrapper/noise headings).

Reply one per line, format: <heading> => <category>. No other text.

Headings:
${distinct.map(h => `- ${h}`).join('\n')}`

  const out = await complete(prompt).catch(() => '')
  const rules: HeadingRule[] = []
  for (const line of out.split('\n')) {
    const sep = line.lastIndexOf('=>')
    if (sep === -1)
      continue
    const bucket = line.slice(sep + 2).trim().toLowerCase()
    if (!BUCKET_TYPES.includes(bucket as BucketType))
      continue // skips "ignore" and any malformed value
    const norm = normalizeHeading(line.slice(0, sep).replace(BULLET_PREFIX_RE, ''))
    if (norm)
      rules.push([new RegExp(`^${norm.replace(REGEX_ESCAPE_RE, '\\$&')}$`), bucket as BucketType])
  }
  return rules
}

/** Bounded-concurrency map preserving order. */
async function pool<T, R>(items: T[], limit: number, fn: (item: T, index: number) => Promise<R>): Promise<R[]> {
  const out: R[] = Array.from({ length: items.length })
  let cursor = 0
  await Promise.all(Array.from({ length: Math.min(limit, items.length) }, async () => {
    while (cursor < items.length) {
      const i = cursor++
      out[i] = await fn(items[i]!, i)
    }
  }))
  return out
}

export async function bucketReleases(releases: ReleaseInput[], opts: BucketPipelineOptions): Promise<BucketPipelineResult> {
  const { packageName, complete, flagThreshold = 0.6, concurrency = 2, onProgress } = opts

  // Pass 1: deterministic.
  const parsed = await Promise.all(releases.map(r => parseReleaseToBuckets(r.version, r.markdown, flagThreshold)))
  let flaggedIdx = parsed.map((p, i) => (p.flagged ? i : -1)).filter(i => i >= 0)
  let inferenceCalls = 0

  // Pass 1.5: one cheap inference call learns the repo's heading vocabulary and
  // re-buckets the flagged releases deterministically — collapsing many
  // per-release calls into one when the repo just uses non-standard headings.
  if (complete && flaggedIdx.length >= INFER_MIN_FLAGGED) {
    const unmatched = flaggedIdx.flatMap(i => parsed[i]!.unmatchedHeadings)
    const learned = await inferHeadingRules(packageName, unmatched, complete)
    inferenceCalls = 1
    onProgress?.(`Inferred ${learned.length} heading rules from ${flaggedIdx.length} flagged releases`)
    if (learned.length) {
      await Promise.all(flaggedIdx.map(async (i) => {
        parsed[i] = await parseReleaseToBuckets(releases[i]!.version, releases[i]!.markdown, flagThreshold, learned)
      }))
      flaggedIdx = parsed.map((p, i) => (p.flagged ? i : -1)).filter(i => i >= 0)
    }
  }

  // Pass 2: per-release LLM re-buckets whatever still flags after inference.
  let llmCalls = 0
  if (complete && flaggedIdx.length) {
    let done = 0
    const rebucketed = await pool(flaggedIdx, concurrency, async (idx) => {
      const out = await complete(LLM_BUCKET_PROMPT(packageName, releases[idx]!.markdown))
      onProgress?.(`LLM bucketed ${++done}/${flaggedIdx.length} flagged releases`)
      // Re-parse the LLM's structured output; threshold 0 so it never re-flags.
      return parseReleaseToBuckets(releases[idx]!.version, out, 0)
    })
    flaggedIdx.forEach((idx, k) => {
      parsed[idx] = rebucketed[k]!
    })
    llmCalls = flaggedIdx.length
  }

  const buckets = mergeBuckets(parsed.map(p => p.buckets))
  const perVersion: VersionBuckets[] = parsed.map(p => ({
    version: p.version,
    buckets: p.buckets,
    counts: bucketCounts(p.buckets),
  }))
  return { buckets, counts: bucketCounts(buckets), releases: releases.length, perVersion, llmCalls, inferenceCalls }
}
