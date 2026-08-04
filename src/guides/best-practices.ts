/**
 * Best-practices artifact — the "how to use <pkg> correctly" skill, the second
 * pillar alongside migration guides (see [[project_npm_guides_pseo]]). Mirrors
 * the migration pipeline (resolve → fetch → synthesise → stamped/cached markdown)
 * but sources from the package's DOCS/README rather than release notes, and is
 * evergreen (one per package, scoped to the latest version, not per major).
 *
 * Output: an agent-digestible best-practices skill that doubles as the `/npm/<pkg>`
 * hub's "Best practices" section and SEO surface.
 */

import type { OptimizeModel } from '../agent/index.ts'
import type { FeaturesConfig } from '../core/config.ts'
import { createHash } from 'node:crypto'
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'pathe'
import { selectExecutor } from '../agent/clis/executors.ts'
import { CACHE_DIR, getCacheDir } from '../cache/index.ts'
import { fetchAndCacheResources } from '../commands/sync/pipeline.ts'
import { pickLatestTag } from '../core/semver.ts'
import { parsePackageSpec } from '../core/url.ts'
import { fetchNpmRegistryMeta } from '../sources/npm-registry.ts'
import { resolvePackageOrCrate } from '../sources/resolve-package.ts'

/** Bump when the best-practices synthesis prompt changes (stage stamp). */
export const BP_PROMPT_VERSION = 1
export const BP_PIPELINE_VERSION = `bp${BP_PROMPT_VERSION}`

const DEFAULT_TIMEOUT = 240_000
const MAX_MATERIAL_CHARS = Number(process.env.BP_MAX_MATERIAL_CHARS) || 90_000
/** Docs only — no releases/issues/discussions for the best-practices artifact. */
const BP_FEATURES: FeaturesConfig = { search: false, issues: false, discussions: false, releases: false }
const MD_RE = /\.md$/

export interface GenerateBestPracticesOptions {
  cwd?: string
  model?: OptimizeModel
  useCache?: boolean
  timeout?: number
  onProgress?: (message: string) => void
}

export interface GeneratedBestPractices {
  packageName: string
  slug: string
  version: string
  repoUrl?: string
  title: string
  markdown: string
  pipelineVersion: string
  model: OptimizeModel
  usage?: { input: number, output: number }
  cost?: number
}

export type GenerateBestPracticesResult
  = | { ok: true, skill: GeneratedBestPractices }
    | { ok: false, error: string }

/**
 * Read cached documentation material for synthesis, newest doc first. Pulls the
 * README, `docs/`, and `llms-docs/` markdown from the reference cache (skilld's
 * doc fetch already wrote these), capped so a huge doc set can't blow the context.
 */
function readDocMaterial(packageName: string, version: string): string {
  const base = getCacheDir(packageName, version)
  if (!existsSync(base))
    return ''
  // Priority order: README first (overview), then curated docs, then llms-docs.
  const candidates: string[] = []
  const readme = ['pkg/README.md', 'README.md', 'packages/docs/README.md', 'docs/README.md']
    .map(p => join(base, p))
    .find(existsSync)
  if (readme)
    candidates.push(readme)
  for (const sub of ['docs', 'llms-docs']) {
    const dir = join(base, sub)
    if (existsSync(dir)) {
      for (const f of readdirSync(dir)) {
        if (MD_RE.test(f) && f !== '_INDEX.md')
          candidates.push(join(dir, f))
      }
    }
  }
  const parts: string[] = []
  let budget = MAX_MATERIAL_CHARS
  for (const file of candidates) {
    if (budget <= 0)
      break
    const body = readFileSync(file, 'utf8').slice(0, budget)
    budget -= body.length
    parts.push(body)
  }
  return parts.join('\n\n---\n\n')
}

function buildBestPracticesPrompt(packageName: string, version: string, material: string): string {
  return `You are writing a concise BEST-PRACTICES skill for the npm package \`${packageName}\` (current version ${version}), for a coding agent that must use it correctly today. Base every claim ONLY on the documentation below — do not invent APIs.

Output Markdown with these sections (omit one only if the docs truly lack it):

## Setup
Install command and the minimum config to get running.

## Core API
The handful of APIs/exports an agent actually uses, each with a one-line purpose and a short code example copied or adapted from the docs.

## Idiomatic patterns
The recommended way to do the common tasks — the patterns the docs steer you toward.

## Common mistakes
Pitfalls, deprecated approaches, and gotchas the docs warn about. Concrete and specific.

## Minimal example
One complete, runnable snippet showing the typical usage.

Rules: imperative and dense; real code from the docs (no placeholders/invented APIs); no marketing prose, contributor lists, or links-only filler.

Documentation:
${material}`
}

export async function generateBestPractices(
  packageSpec: string,
  opts: GenerateBestPracticesOptions = {},
): Promise<GenerateBestPracticesResult> {
  const { cwd = process.cwd(), model = 'sonnet', useCache = true, timeout = DEFAULT_TIMEOUT, onProgress = () => {} } = opts
  const { name: packageName } = parsePackageSpec(packageSpec)

  onProgress(`Resolving dist-tags for ${packageName}`)
  const meta = await fetchNpmRegistryMeta(packageName, '')
  // Best practices describe the CURRENT STABLE release, so prefer the `latest`
  // dist-tag; only fall back to the largest tag (incl. prereleases) when there
  // is no stable release. (Prerelease/canary versions ship without docs.)
  const version = meta.distTags?.latest?.version ?? pickLatestTag(meta.distTags)?.version
  if (!version)
    return { ok: false, error: `No valid published version found for ${packageName}` }

  onProgress(`Resolving ${packageName}@${version} source`)
  const resolution = await resolvePackageOrCrate(`${packageName}@${version}`, { cwd, onProgress })
  const resolved = resolution.resolved
  if (!resolved?.repoUrl)
    return { ok: false, error: `Could not resolve a source repo for ${packageName}` }

  onProgress(`Fetching docs for ${packageName}@${version}`)
  await fetchAndCacheResources({ packageName, resolved, version, useCache, features: BP_FEATURES, onProgress })

  const material = readDocMaterial(packageName, version)
  if (material.trim().length < 200)
    return { ok: false, error: `No documentation found for ${packageName}@${version}` }

  const executor = selectExecutor(model)
  if ('error' in executor)
    return { ok: false, error: executor.error }

  const cacheDir = getCacheDir(packageName, version)
  mkdirSync(cacheDir, { recursive: true })

  // Synthesis cache: skip the LLM when the docs + prompt version are unchanged.
  const cachePath = join(cacheDir, 'best-practices.json')
  const hash = createHash('sha256').update(material).digest('hex').slice(0, 16)
  const cached = useCache && existsSync(cachePath)
    ? JSON.parse(readFileSync(cachePath, 'utf8')) as { hash: string, promptVersion: number, markdown: string }
    : null

  let markdown: string
  let usage: { input: number, output: number } | undefined
  let cost: number | undefined
  if (cached && cached.hash === hash && cached.promptVersion === BP_PROMPT_VERSION) {
    onProgress('Using cached best-practices')
    markdown = cached.markdown
  }
  else {
    onProgress(`Synthesising best-practices with ${model}`)
    const out = await executor.run({
      section: 'custom',
      prompt: buildBestPracticesPrompt(packageName, version, material),
      skillDir: cacheDir,
      skilldDir: CACHE_DIR,
      timeout,
      onProgress: p => onProgress(p.type),
    })
    markdown = (out.text || out.writeContent || '').trim()
    if (!markdown)
      return { ok: false, error: `LLM produced no output for ${packageName}${out.stderr ? `: ${out.stderr}` : ''}` }
    usage = out.usage
    cost = out.cost
    writeFileSync(cachePath, JSON.stringify({ hash, promptVersion: BP_PROMPT_VERSION, markdown }))
  }

  return {
    ok: true,
    skill: {
      packageName,
      slug: packageName,
      version,
      repoUrl: resolved.repoUrl,
      title: `${packageName} best practices`,
      markdown,
      pipelineVersion: BP_PIPELINE_VERSION,
      model,
      usage,
      cost,
    },
  }
}
