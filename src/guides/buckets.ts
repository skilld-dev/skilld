/**
 * Deterministic release-note bucketing.
 *
 * Most npm release notes are machine-generated (changelogen, release-please,
 * semantic-release, GitHub auto-notes) with predictable structure — section
 * headings (`### Features`, `### Bug Fixes`) or conventional-commit bullet
 * prefixes (`feat:`, `fix:`, `perf:`). We can bucket those for free with no LLM.
 *
 * Each release is parsed into change8-style buckets and given a `coverage`
 * score. Low coverage (prose-heavy / custom format, e.g. drizzle's 1.0 notes)
 * flags the release for LLM bucketing instead — that's the only place the model
 * is needed.
 */

import { parseAST } from 'md4x'

export type BucketType = 'breaking' | 'features' | 'fixes' | 'improvements'

export const BUCKET_TYPES: BucketType[] = ['breaking', 'features', 'fixes', 'improvements']

export type Buckets = Record<BucketType, string[]>

export function emptyBuckets(): Buckets {
  return { breaking: [], features: [], fixes: [], improvements: [] }
}

export interface ReleaseBuckets {
  version: string
  buckets: Buckets
  totalItems: number
  classifiedItems: number
  /** Fraction of bullet items the deterministic parser could classify. */
  coverage: number
  /** True when this release should be bucketed by the LLM instead. */
  flagged: boolean
  /** Heading texts that matched no bucket rule — candidates for new matchers. */
  unmatchedHeadings: string[]
}

// Heading text → bucket. Matched against lowercased, emoji/punctuation-stripped
// heading text via substring, so `🚀 Enhancements` and `Bug Fixes` both hit.
/** A heading-text matcher → bucket. Built-in rules + repo-learned (inferred) ones. */
export type HeadingRule = [RegExp, BucketType]

const HEADING_RULES: HeadingRule[] = [
  // changesets headings (`### Major/Minor/Patch Changes`) map by semver impact.
  [/major changes/, 'breaking'],
  [/minor changes/, 'features'],
  [/patch changes/, 'fixes'],
  [/breaking|removed|deprecat|upgrad|migrat/, 'breaking'],
  [/feat|enhancement|added|new|highlight|labs/, 'features'],
  [/fix|bug/, 'fixes'],
  [/perf|refactor|improvement|chore|doc|style|build|ci|dependenc|revert|test|maintenance/, 'improvements'],
]

// Wrapper/noise headings that carry no bucket signal. Treated as transparent:
// they don't reset the active bucket and aren't counted as unmatched. Matched
// against the normalized (lowercased, punctuation-stripped) heading, anchored
// so "breaking changes" still routes to a real bucket via HEADING_RULES first.
const TRANSPARENT_RE = /^(?:whats changed|contributors?|change ?log|changes|changed|packages|notes|view changes on github|full change ?log)$/
// Version-number section headers (CHANGELOG.md style: `## v1.2.3`, `## 3.0.0`).
const VERSION_HEADING_RE = /^v?\d+\.\d+/

// Conventional-commit prefix → bucket (when bullets carry no heading context).
const PREFIX_RULES: Array<[RegExp, BucketType]> = [
  [/^feat\b/, 'features'],
  [/^fix\b/, 'fixes'],
  [/^(perf|refactor|style|docs?|chore|build|ci|test|revert|deps?)\b/, 'improvements'],
]

const EMOJI_PUNCT_RE = /[^a-z\s]/g
// Conventional bullet: `feat(scope)!: desc` / `* fix: desc`
const CONVENTIONAL_RE = /^([a-z]+)(?:\([^)]*\))?(!)?:\s*/i
const BREAKING_BODY_RE = /breaking[\s-]?change/i
// Inline breaking markers common in monorepo changelogs (jest, babel, lerna):
// `[**BREAKING**]`, `[BREAKING]`, `**BREAKING**`, `[**BREAKING CHANGE**]`. These
// often sit under a `### Features` heading, so the bullet itself must flag it.
// Brackets/bold required so the word "breaking" in prose doesn't false-positive.
const BREAKING_MARKER_RE = /\[\s*(?:\*{1,2}\s*)?breaking(?:\s+changes?)?\s*(?:\*{1,2}\s*)?\]|\*\*\s*breaking(?:\s+changes?)?\s*\*\*/i

type HeadingDisposition = BucketType | 'transparent' | null

const HTML_ENTITY_RE = /&[a-z]+;|&#\d+;/g

/** Normalize a heading for matching: lowercase, drop entities + punctuation. */
export function normalizeHeading(text: string): string {
  return text.toLowerCase().replace(HTML_ENTITY_RE, ' ').replace(EMOJI_PUNCT_RE, ' ').replace(/\s+/g, ' ').trim()
}

function headingDisposition(text: string, extraRules: HeadingRule[]): HeadingDisposition {
  const lower = text.toLowerCase().trim()
  // Version headers must be tested before punctuation stripping turns `1.2` → `1 2`.
  if (VERSION_HEADING_RE.test(lower))
    return 'transparent'
  const norm = normalizeHeading(text)
  if (!norm)
    return 'transparent'
  // Repo-learned rules (pattern inference) win first, then the built-in rules,
  // so "breaking changes" / "3.0 migration" route correctly before transparent.
  for (const [re, bucket] of [...extraRules, ...HEADING_RULES]) {
    if (re.test(norm))
      return bucket
  }
  if (TRANSPARENT_RE.test(norm))
    return 'transparent'
  return null
}

function bucketForBullet(text: string): BucketType | null {
  // An explicit breaking marker wins regardless of conventional prefix or the
  // bullet's section heading (jest lists `[**BREAKING**]` items under Features).
  if (BREAKING_MARKER_RE.test(text))
    return 'breaking'
  const conv = text.match(CONVENTIONAL_RE)
  if (conv) {
    if (conv[2] === '!' || BREAKING_BODY_RE.test(text))
      return 'breaking'
    const prefix = `${conv[1]!.toLowerCase()}:`
    for (const [re, bucket] of PREFIX_RULES) {
      if (re.test(prefix))
        return bucket
    }
  }
  if (BREAKING_BODY_RE.test(text))
    return 'breaking'
  return null
}

// Leading-verb heuristic for freeform bullets (GitHub auto-notes PR titles like
// `Fix #123 …` / `Added support …`) that carry no conventional prefix. Lowest
// confidence, so it's only consulted after conventional prefix + heading.
const LEADING_VERB_RULES: Array<[RegExp, BucketType]> = [
  [/^(?:remove|removed|drop|dropped|deprecat|renamed?|breaking|delete|disallow)\b/, 'breaking'],
  [/^(?:add|added|adds|introduc\w*|implement\w*|support|new|create\w*|enable\w*|allow)\b/, 'features'],
  [/^(?:fix|fixed|fixes|resolve\w*|correct\w*|patch\w*|prevent\w*|handle\w*|ensure\w*|avoid\w*)\b/, 'fixes'],
  [/^(?:updat\w*|improv\w*|refactor\w*|bump|chore|perf\w*|optimi\w*|clean\w*|test|docs?|migrat\w*|upgrad\w*|tweak\w*|adjust\w*|revert\w*)\b/, 'improvements'],
]

function looseBucketForBullet(text: string): BucketType | null {
  const t = text.toLowerCase().trim()
  for (const [re, bucket] of LEADING_VERB_RULES) {
    if (re.test(t))
      return bucket
  }
  return null
}

/** Strip the conventional prefix so the surfaced bullet reads cleanly. */
function cleanBullet(text: string): string {
  return text.replace(CONVENTIONAL_RE, '').trim()
}

// The hyperscript tuple shape (`[tag, props, ...children]`) fights TS's tuple
// inference on `.slice`, so the walker operates on `any` with runtime checks.
/** Concatenate the visible text of a node, recursing into inline children. */
function nodeText(node: any): string {
  if (typeof node === 'string')
    return node
  if (Array.isArray(node))
    return node.slice(2).map(nodeText).join('')
  return ''
}

function isTag(node: any, ...tags: string[]): boolean {
  return Array.isArray(node) && tags.includes(node[0])
}

/**
 * Parse a release note into buckets via the md4x AST. Headings set the active
 * bucket; list items inherit it (or fall back to their own conventional-commit
 * prefix). Code blocks parse as `pre` nodes, so `-`/`+` diff lines never leak in
 * as bullets. Unclassified items lower `coverage`; low coverage flags the
 * release for LLM bucketing instead.
 */
export async function parseReleaseToBuckets(version: string, markdown: string, flagThreshold = 0.6, extraHeadingRules: HeadingRule[] = []): Promise<ReleaseBuckets> {
  const { nodes } = await parseAST(markdown)
  const buckets = emptyBuckets()
  const unmatchedHeadings: string[] = []
  let currentHeadingBucket: BucketType | null = null
  let total = 0
  let classified = 0
  let proseChars = 0

  const classifyItem = (text: string) => {
    if (!text.trim())
      return
    total++
    // A bullet's own conventional-commit prefix is more specific than its
    // section, so it wins (lets GitHub's "What's Changed" lists of `feat:`/`fix:`
    // bullets classify correctly); breaking always wins; bare bullets inherit
    // the active heading bucket.
    const own = bucketForBullet(text)
    // Priority: explicit breaking > conventional prefix > active heading >
    // leading-verb guess. The guess only fires for bare freeform bullets.
    const bucket = own === 'breaking' ? 'breaking' : (own ?? currentHeadingBucket ?? looseBucketForBullet(text))
    if (bucket) {
      buckets[bucket].push(cleanBullet(text.trim()))
      classified++
    }
  }

  const walk = (siblings: any[]) => {
    for (const node of siblings) {
      if (isTag(node, 'h1', 'h2', 'h3', 'h4', 'h5', 'h6')) {
        const text = nodeText(node).trim()
        const disp = headingDisposition(text, extraHeadingRules)
        if (disp === 'transparent')
          continue // keep the active bucket; carries no signal of its own
        currentHeadingBucket = disp
        if (!disp && text)
          unmatchedHeadings.push(text)
      }
      else if (isTag(node, 'ul', 'ol')) {
        for (const li of node.slice(2) as any[]) {
          if (!isTag(li, 'li'))
            continue
          const children = li.slice(2) as any[]
          const nested = children.filter(c => isTag(c, 'ul', 'ol'))
          const direct = children.filter(c => !isTag(c, 'ul', 'ol'))
          classifyItem(direct.map(nodeText).join(''))
          walk(nested) // sub-bullets, same heading context
        }
      }
      else if (isTag(node, 'p')) {
        proseChars += nodeText(node).length
      }
      // pre/code/blockquote/table etc: skip — no migration bullets there.
    }
  }
  walk(nodes)

  const coverage = total === 0 ? 0 : classified / total
  // Flag prose-heavy/custom releases (drizzle-style) for LLM bucketing: either
  // we classified too little, or there were no list items but real prose.
  const flagged = total === 0 ? proseChars > 200 : coverage < flagThreshold

  return { version, buckets, totalItems: total, classifiedItems: classified, coverage, flagged, unmatchedHeadings }
}

/** Merge many releases' buckets into one, deduping identical items. */
export function mergeBuckets(all: Buckets[]): Buckets {
  const merged = emptyBuckets()
  for (const type of BUCKET_TYPES) {
    const seen = new Set<string>()
    for (const b of all) {
      for (const item of b[type]) {
        const key = item.toLowerCase()
        if (!seen.has(key)) {
          seen.add(key)
          merged[type].push(item)
        }
      }
    }
  }
  return merged
}

export function bucketCounts(b: Buckets): Record<BucketType, number> {
  return { breaking: b.breaking.length, features: b.features.length, fixes: b.fixes.length, improvements: b.improvements.length }
}
