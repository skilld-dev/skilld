/**
 * Migration-guide generation — npm package → agent-digestible upgrade guide.
 *
 * Reuses skilld's data layer end to end:
 *   1. pick the largest version across dist-tags (incl. prereleases)
 *   2. resolve the source repo at that version
 *   3. fetch + cache release notes / CHANGELOG into the reference cache
 *   4. synthesise the guide via the configured LLM executor
 *
 * Produces a `GeneratedGuide` the caller persists (skilld.dev `npm-guides`).
 */

import type { OptimizeModel } from '../agent/index.ts'
import type { FeaturesConfig } from '../core/config.ts'
import type { VersionBuckets } from './bucket-pipeline.ts'
import type { Buckets, BucketType } from './buckets.ts'
import { createHash } from 'node:crypto'
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'pathe'
import { selectExecutor } from '../agent/clis/executors.ts'
import { CACHE_DIR, getCacheDir, getRepoCacheDir } from '../cache/index.ts'
import { fetchAndCacheResources } from '../commands/sync/pipeline.ts'
import { isSnapshotVersion, pickLatestTag, semverGt, semverMajor, semverValid } from '../core/semver.ts'
import { parseGitHubUrl, parsePackageSpec } from '../core/url.ts'
import { fetchNpmRegistryMeta } from '../sources/npm-registry.ts'
import { resolvePackageOrCrate } from '../sources/resolve-package.ts'
import { bucketReleases, formatBucketsForRunbook } from './bucket-pipeline.ts'
import { buildGuidePrompt } from './prompt.ts'
import { PIPELINE_VERSION, RUNBOOK_PROMPT_VERSION } from './version.ts'

type BucketCounts = Record<BucketType, number>
interface ReleaseInput { version: string, markdown: string }

/** Guide synthesis only needs releases; skip issues/discussions for speed. */
const GUIDE_FEATURES: FeaturesConfig = {
  search: false,
  issues: false,
  discussions: false,
  releases: true,
}

const DEFAULT_TIMEOUT = 240_000
/** Cap releases bucketed per guide — bounds the LLM fork on huge-history packages. */
const MAX_RELEASES = Number(process.env.GUIDE_MAX_RELEASES) || 30

export interface GenerateGuideOptions {
  cwd?: string
  model?: OptimizeModel
  /** Reuse cached references when present (default true). */
  useCache?: boolean
  timeout?: number
  /** Run the clean+extract distillation pass before synthesis (default true). */
  distill?: boolean
  /** Model for the cleanup/extract map-pass; defaults to `model`. Use a faster one. */
  distillModel?: OptimizeModel
  /**
   * Major version to scope the guide to (the v3→v4 jump). Defaults to the major
   * of the latest published version. Each major is its own content collection,
   * so older majors are generated as separate batched pages.
   */
  major?: number
  onProgress?: (message: string) => void
}

export interface GeneratedGuide {
  packageName: string
  /** URL-safe slug (scoped names keep their `/`; caller encodes for the path). */
  slug: string
  /** Largest version, the canonical target of the guide. */
  version: string
  tag: string
  prerelease: boolean
  /** Stable version migrated from, when distinct from `version`. */
  fromVersion?: string
  repoUrl?: string
  releasedAt?: string
  title: string
  markdown: string
  /** Per-type change counts from bucketing — powers the change8-style listing. */
  counts: BucketCounts
  /**
   * Per-version buckets (newest-first) within the major — powers the from-version
   *  selector + per-version sections on the page.
   */
  releaseBuckets: VersionBuckets[]
  /** Other published versions this canonical guide stands in for (301 sources). */
  supersedes: string[]
  /** Pipeline stamp (`b<rules>.p<prompt>`); mismatch ⇒ this guide is stale. */
  pipelineVersion: string
  model: OptimizeModel
  usage?: { input: number, output: number }
  cost?: number
}

export type GenerateGuideResult
  = | { ok: true, guide: GeneratedGuide }
    | { ok: false, error: string }

const MD_SUFFIX_RE = /\.md$/
const V_PREFIX_RE = /^v/
// First semver-looking token anywhere in a filename — recovers the version from
// monorepo/scoped release tags like `vquasar-v2.18.0` or `@scope/pkg@2.6.0`.
const SEMVER_IN_NAME_RE = /(\d+\.\d+\.\d+(?:-[0-9A-Z.-]+)?)/i
// Two-part `major.minor` with optional prerelease (e.g. TypeScript `6.0-beta`).
const SEMVER_2PART_RE = /\b(\d+)\.(\d+)(-[0-9A-Z.-]+)?\b/i

/**
 * Honest stub for a release with no actionable changes (no breaking changes or
 * code-affecting features). Without it, synthesis on raw release notes invents
 * breaking-change sections out of version-bump noise (e.g. quasar's per-release
 * `## v2.18.0` bumps). Surfaces the fix/improvement counts so the page still
 * reflects what shipped, while making clear no migration work is required.
 */
function buildNoChangesStub(packageName: string, version: string, counts: BucketCounts, repoUrl?: string): string {
  const tally = [
    counts.fixes ? `${counts.fixes} fix${counts.fixes === 1 ? '' : 'es'}` : '',
    counts.improvements ? `${counts.improvements} improvement${counts.improvements === 1 ? '' : 's'}` : '',
  ].filter(Boolean).join(' and ')
  const summary = tally
    ? `This release ships ${tally} but no breaking changes or new APIs, so upgrading requires no code changes.`
    : `No breaking changes or new APIs were detected in this release, so upgrading requires no code changes.`
  const ref = repoUrl ? `\n\nFor the full changelog, see ${repoUrl}/releases.` : ''
  return `# Migrating ${packageName} to ${version}

${summary}

## Upgrade steps

1. Update the dependency:
   \`\`\`bash
   npm install ${packageName}@${version}
   \`\`\`
2. Run your build and test suite to confirm nothing breaks.

If you depend on internal or undocumented APIs, review the upstream release notes before upgrading.${ref}`
}

/** Version parsed from a release filename, e.g. `v1.0.0-beta.8.md` → `1.0.0-beta.8`. */
function versionFromReleaseFile(file: string): string | null {
  const base = file.replace(MD_SUFFIX_RE, '')
  // Plain `v1.2.3.md` / `1.2.3.md`.
  const direct = base.replace(V_PREFIX_RE, '')
  if (semverValid(direct))
    return direct
  // Monorepo/scoped tag (`vquasar-v2.18.0`, `@scope/pkg@2.6.0`): pull the semver out.
  const m = base.match(SEMVER_IN_NAME_RE)
  if (m && semverValid(m[1]!))
    return m[1]!
  // Two-part major.minor tags, optionally prereleased (TypeScript's `v6.0-beta`,
  // `v6.0-rc`): pad the patch so they're valid semver and land in the right major.
  const two = base.match(SEMVER_2PART_RE)
  if (two) {
    const padded = `${two[1]}.${two[2]}.0${two[3] ?? ''}`
    if (semverValid(padded))
      return padded
  }
  return null
}

/**
 * Locate the cached `releases/` dir. Timeline releases write to the repo-level
 * cache (`~/.skilld/repos/<owner>/<repo>/releases/`); fall back to the
 * per-package dir for sources without a resolved repo (e.g. blog releases).
 */
function findReleasesDir(packageName: string, version: string, repoUrl?: string): string | null {
  const repo = repoUrl ? parseGitHubUrl(repoUrl) : null
  if (repo) {
    const dir = join(getRepoCacheDir(repo.owner, repo.repo), 'releases')
    if (existsSync(dir))
      return dir
  }
  const pkgDir = join(getCacheDir(packageName, version), 'releases')
  return existsSync(pkgDir) ? pkgDir : null
}

// CHANGELOG version heading: `## 1.2.3`, `### [1.2.3]`, `## v1.2.3 (2024…)`.
const CHANGELOG_VERSION_HEADING_RE = /^#{1,4}\s+\[?v?(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)/

/**
 * Split a CHANGELOG.md into per-version sections so the bucketer has structured
 * input. Used when a package ships no per-release files (or they're stale),
 * keeping synthesis on bucketed bullets rather than raw prose.
 */
function splitChangelog(markdown: string): ReleaseInput[] {
  const out: ReleaseInput[] = []
  let cur: { version: string, body: string[] } | null = null
  for (const line of markdown.split('\n')) {
    const m = line.match(CHANGELOG_VERSION_HEADING_RE)
    if (m && semverValid(m[1]!)) {
      if (cur)
        out.push({ version: cur.version, markdown: cur.body.join('\n') })
      cur = { version: m[1]!, body: [line] }
    }
    else if (cur) {
      cur.body.push(line)
    }
  }
  if (cur)
    out.push({ version: cur.version, markdown: cur.body.join('\n') })
  return out
}

/**
 * Read a major version's releases as individual notes (newest-first) for the
 * bucketing pipeline, which classifies each release on its own. A guide is
 * scoped to ONE major (the v3→v4 jump), so we keep only releases whose major
 * equals `targetMajor` — this is what keeps "Migrating to v9" from absorbing the
 * previous major's patch noise. Per-release files are preferred; when none
 * survive, the CHANGELOG is split into per-version sections as a fallback.
 */
function readReleaseList(
  packageName: string,
  version: string,
  repoUrl: string | undefined,
  targetMajor: number,
): ReleaseInput[] {
  const releasesDir = findReleasesDir(packageName, version, repoUrl)
  if (!releasesDir)
    return []

  const inMajor = (v: string) => semverMajor(v) === targetMajor
  // Cap within the major: an extremely active major could exceed this, but the
  // newest N releases of the major are the relevant upgrade surface.
  const files = readdirSync(releasesDir).filter(f => f.endsWith('.md'))
  const perRelease = files
    .filter(f => f !== '_INDEX.md' && f !== 'CHANGELOG.md')
    .map(f => ({ file: f, version: versionFromReleaseFile(f) }))
    .filter((e): e is { file: string, version: string } => e.version != null)
    .filter(e => inMajor(e.version))
    .sort((a, b) => (semverGt(a.version, b.version) ? -1 : 1))
    .slice(0, MAX_RELEASES)
    .map(e => ({ version: e.version, markdown: readFileSync(join(releasesDir, e.file), 'utf8') }))
  if (perRelease.length)
    return perRelease

  if (files.includes('CHANGELOG.md')) {
    return splitChangelog(readFileSync(join(releasesDir, 'CHANGELOG.md'), 'utf8'))
      .filter(e => inMajor(e.version))
      .sort((a, b) => (semverGt(a.version, b.version) ? -1 : 1))
      .slice(0, MAX_RELEASES)
  }
  return []
}

/**
 * The latest cached release of the previous major — the "migrating from" anchor
 * so a v9 guide reads "8.3.18 → 9.2.2". Undefined when the previous major isn't
 * in the cached window (e.g. a brand-new package with only one major).
 */
function previousMajorAnchor(
  packageName: string,
  version: string,
  repoUrl: string | undefined,
  targetMajor: number,
): string | undefined {
  const releasesDir = findReleasesDir(packageName, version, repoUrl)
  if (!releasesDir)
    return undefined
  return readdirSync(releasesDir)
    .filter(f => f.endsWith('.md') && f !== '_INDEX.md' && f !== 'CHANGELOG.md')
    .map(versionFromReleaseFile)
    .filter((v): v is string => v != null && semverMajor(v) === targetMajor - 1)
    .sort((a, b) => (semverGt(a, b) ? -1 : 1))[0]
}

interface PickedLike { version: string, tag: string, prerelease: boolean, releasedAt?: string }
interface MetaLike { distTags?: Record<string, { version: string }> }

/** Assemble the final `GeneratedGuide`, deriving the 301-source `supersedes` list. */
function buildGuideResult(
  packageName: string,
  picked: PickedLike,
  fromVersion: string | undefined,
  repoUrl: string | undefined,
  resolvedReleasedAt: string | undefined,
  counts: BucketCounts,
  releaseBuckets: VersionBuckets[],
  meta: MetaLike,
  model: OptimizeModel,
  markdown: string,
  usage?: { input: number, output: number },
  cost?: number,
): GeneratedGuide {
  // Clean, redirect-worthy prior versions only — drop per-commit snapshots and
  // dedupe. These are the versioned URLs that 301 to the canonical guide.
  const supersedes = Object.values(meta.distTags ?? {})
    .map(t => t.version)
    .filter(v => v !== picked.version && !isSnapshotVersion(v))
    .filter((v, i, all) => all.indexOf(v) === i)
    .sort((a, b) => (semverGt(a, b) ? -1 : 1))

  return {
    packageName,
    slug: packageName,
    version: picked.version,
    tag: picked.tag,
    prerelease: picked.prerelease,
    fromVersion,
    repoUrl,
    releasedAt: picked.releasedAt ?? resolvedReleasedAt,
    title: `Migrating ${packageName} to ${picked.version}`,
    markdown,
    counts,
    releaseBuckets,
    supersedes,
    pipelineVersion: PIPELINE_VERSION,
    model,
    usage,
    cost,
  }
}

export async function generateGuide(
  packageSpec: string,
  opts: GenerateGuideOptions = {},
): Promise<GenerateGuideResult> {
  const { cwd = process.cwd(), model = 'sonnet', useCache = true, timeout = DEFAULT_TIMEOUT, distill, distillModel, major, onProgress = () => {} } = opts
  const { name: packageName } = parsePackageSpec(packageSpec)

  onProgress(`Resolving dist-tags for ${packageName}`)
  const meta = await fetchNpmRegistryMeta(packageName, '')
  const picked = pickLatestTag(meta.distTags)
  if (!picked)
    return { ok: false, error: `No valid published version found for ${packageName}` }

  // Scope the guide to one major (default: the latest version's major). This is
  // the v3→v4 unit — keeping a guide from spilling across a major boundary.
  const targetMajor = major ?? semverMajor(picked.version)
  if (targetMajor == null)
    return { ok: false, error: `Could not determine major version for ${packageName}@${picked.version}` }

  onProgress(`Resolving ${packageName}@${picked.version} source`)
  const resolution = await resolvePackageOrCrate(`${packageName}@${picked.version}`, { cwd, onProgress })
  const resolved = resolution.resolved
  if (!resolved?.repoUrl)
    return { ok: false, error: `Could not resolve a source repo for ${packageName}` }

  onProgress(`Fetching release notes for ${packageName}@${picked.version}`)
  await fetchAndCacheResources({
    packageName,
    resolved,
    version: picked.version,
    useCache,
    features: GUIDE_FEATURES,
    onProgress,
  })

  const releases = readReleaseList(packageName, picked.version, resolved.repoUrl, targetMajor)
  // Synthesis runs only on bucketed bullets, never raw prose — without structured
  // release notes the model hallucinates (e.g. remark's CHANGELOG is just a "see
  // GitHub Releases" pointer). Refuse rather than publish an invented guide.
  if (!releases.length)
    return { ok: false, error: `No structured release notes found for ${packageName}@${picked.version} (major ${targetMajor})` }

  // "Migrating from" anchor = the previous major's last release, when cached.
  const fromVersion = previousMajorAnchor(packageName, picked.version, resolved.repoUrl, targetMajor)

  const executor = selectExecutor(model)
  if ('error' in executor)
    return { ok: false, error: executor.error }

  // The flagged-release re-bucketing is a simpler task than synthesis, so it can
  // run on a smaller, faster model (distillModel) while synthesis uses `model`.
  const distillExecutor = distillModel && distillModel !== model ? selectExecutor(distillModel) : executor
  if ('error' in distillExecutor)
    return { ok: false, error: distillExecutor.error }

  // The per-package cache dir may not exist (data can live in the repo cache),
  // so ensure it before any read/write of the bucket cache or skillDir.
  const cacheDir = getCacheDir(packageName, picked.version)
  mkdirSync(cacheDir, { recursive: true })

  // Build a one-shot completion helper around a given executor.
  const completeWith = (ex: typeof executor) => async (p: string): Promise<string> => {
    const r = await ex.run({
      section: 'custom',
      prompt: p,
      skillDir: cacheDir,
      skilldDir: CACHE_DIR,
      timeout,
      onProgress: prog => onProgress(prog.type),
    })
    return (r.text || r.writeContent || '').trim()
  }

  // Bucket the release notes: deterministic md4x parsing for the ~93% that are
  // structured, LLM re-bucketing only for prose-heavy flagged releases. The
  // merged buckets feed synthesis and the counts power the listing; cache both
  // so re-runs and prompt iteration skip the (expensive) LLM fork.
  const bucketCachePath = join(cacheDir, 'guide-buckets.json')
  const cached = useCache && existsSync(bucketCachePath)
    ? JSON.parse(readFileSync(bucketCachePath, 'utf8')) as { buckets: Buckets, counts: BucketCounts, perVersion?: VersionBuckets[] }
    : null
  let buckets: Buckets
  let counts: BucketCounts
  let perVersion: VersionBuckets[]
  // Cache invalidates when it predates per-version data (older cache shape).
  if (cached && cached.perVersion) {
    onProgress('Using cached buckets')
    buckets = cached.buckets
    counts = cached.counts
    perVersion = cached.perVersion
  }
  else {
    onProgress(`Bucketing ${releases.length} releases${distill === false ? ' (deterministic only)' : ''}`)
    const res = await bucketReleases(releases, {
      packageName,
      complete: distill === false ? undefined : completeWith(distillExecutor),
      onProgress,
    })
    buckets = res.buckets
    counts = res.counts
    perVersion = res.perVersion
    writeFileSync(bucketCachePath, JSON.stringify({ buckets, counts, perVersion }))
  }

  // Actionable guard: a runbook needs breaking changes or code-affecting
  // features to migrate against. Pure fix/improvement releases (and releases
  // that bucketed to nothing) get an honest stub instead of a synthesised
  // runbook — this is what kills the version-bump-noise hallucinations.
  const synthInput = formatBucketsForRunbook(buckets)
  if (!synthInput.trim()) {
    onProgress(`No actionable changes for ${packageName}@${picked.version}; emitting stub`)
    return {
      ok: true,
      guide: buildGuideResult(packageName, picked, fromVersion, resolved.repoUrl, resolved.releasedAt, counts, perVersion, meta, model, buildNoChangesStub(packageName, picked.version, counts, resolved.repoUrl)),
    }
  }

  // Runbook synthesis is the expensive LLM stage. Cache it keyed by the synthesis
  // input + prompt version, so bucketing/UI/windowing tweaks that don't change the
  // breaking+features input reuse the prose; only a prompt-version bump (or genuinely
  // changed inputs) re-synthesises. This is what makes future regens incremental.
  const runbookCachePath = join(cacheDir, 'guide-runbook.json')
  const synthHash = createHash('sha256').update(synthInput).digest('hex').slice(0, 16)
  const cachedRunbook = useCache && existsSync(runbookCachePath)
    ? JSON.parse(readFileSync(runbookCachePath, 'utf8')) as { hash: string, promptVersion: number, markdown: string }
    : null

  let markdown: string
  let usage: { input: number, output: number } | undefined
  let cost: number | undefined
  if (cachedRunbook && cachedRunbook.hash === synthHash && cachedRunbook.promptVersion === RUNBOOK_PROMPT_VERSION) {
    onProgress('Using cached runbook (synthesis input unchanged)')
    markdown = cachedRunbook.markdown
  }
  else {
    onProgress(`Synthesising guide with ${model}`)
    const prompt = buildGuidePrompt({
      packageName,
      version: picked.version,
      fromVersion,
      prerelease: picked.prerelease,
      repoUrl: resolved.repoUrl,
      material: synthInput,
      contextCounts: { fixes: counts.fixes, improvements: counts.improvements },
    })
    const out = await executor.run({
      section: 'custom',
      prompt,
      skillDir: getCacheDir(packageName, picked.version),
      skilldDir: CACHE_DIR,
      timeout,
      onProgress: p => onProgress(p.type),
    })
    markdown = (out.text || out.writeContent || '').trim()
    if (!markdown)
      return { ok: false, error: `LLM produced no output for ${packageName}${out.stderr ? `: ${out.stderr}` : ''}` }
    usage = out.usage
    cost = out.cost
    writeFileSync(runbookCachePath, JSON.stringify({ hash: synthHash, promptVersion: RUNBOOK_PROMPT_VERSION, markdown }))
  }

  return {
    ok: true,
    guide: buildGuideResult(packageName, picked, fromVersion, resolved.repoUrl, resolved.releasedAt, counts, perVersion, meta, model, markdown, usage, cost),
  }
}
