/**
 * LLM enhancer — drives CLI adapters (and pi-ai) to generate SKILL.md sections.
 *
 * Owns the section-level lifecycle: cache lookup (references-dir + prompt-hash),
 * parallel spawn with stagger, rate-limit-aware retry, and merge-order assembly.
 * Per-CLI concerns (argv, stream parsing, model registry) live in `./clis/`.
 */

import type { SectionExecutor } from './clis/runner.ts'
import type { OptimizeDocsOptions, OptimizeResult, SectionResult, StreamProgress } from './clis/types.ts'
import type { SkillSection } from './prompts/index.ts'
import { existsSync, lstatSync, mkdirSync, readdirSync } from 'node:fs'
import { setTimeout as delay } from 'node:timers/promises'
import { join } from 'pathe'
import { createReferenceCache } from '../cache/index.ts'
import { skillInternalDir, skillLogDir } from '../core/paths.ts'
import { getCached, setCache } from './clis/cli-cache.ts'
import { selectExecutor } from './clis/executors.ts'
import { finalizeSection, prepareSection } from './clis/runner.ts'
import { buildAllSectionPrompts, SECTION_MERGE_ORDER, SECTION_OUTPUT_FILES, wrapSection } from './prompts/index.ts'

const STATIC_REGEX_1 = /\b429\b/
const STATIC_REGEX_2 = /rate.?limit/i
const STATIC_REGEX_3 = /exhausted.*capacity/i
const STATIC_REGEX_4 = /quota.*reset/i
const STATIC_REGEX_5 = /reset\s+after\s+(\d+)s/i

// ── Per-section run ──────────────────────────────────────────────────

interface OptimizeSectionOptions {
  section: SkillSection
  prompt: string
  outputFile: string
  skillDir: string
  executor: SectionExecutor
  onProgress?: (progress: StreamProgress) => void
  timeout: number
  debug?: boolean
  preExistingFiles: Set<string>
}

/** prepareSection → executor.run → finalizeSection. One linear flow per section. */
async function optimizeSection(opts: OptimizeSectionOptions): Promise<SectionResult> {
  const { section, prompt, outputFile, skillDir, executor, onProgress, timeout, debug, preExistingFiles } = opts
  const skilldDir = skillInternalDir(skillDir)
  const outputPath = join(skilldDir, outputFile)

  prepareSection({ section, prompt, outputPath, skilldDir })

  const raw = await executor.run({ section, prompt, skillDir, skilldDir, timeout, debug, onProgress })

  return finalizeSection({
    section,
    raw,
    outputFile,
    outputPath,
    skilldDir,
    debug: !!debug,
    cliCleanup: executor.cliCleanup ? { preExistingFiles } : undefined,
  })
}

// ── Main orchestrator ────────────────────────────────────────────────

export async function optimizeDocs(opts: OptimizeDocsOptions): Promise<OptimizeResult> {
  const { packageName, skillDir, model = 'sonnet', version, hasGithub, hasReleases, hasChangelog, docFiles, docsType, hasShippedDocs, onProgress, timeout = 180000, debug, noCache, sections, customPrompt, features, pkgFiles, overheadLines } = opts
  const cache = createReferenceCache(packageName, version)

  const selectedSections = sections ?? ['api-changes', 'best-practices'] as SkillSection[]

  const sectionPrompts = buildAllSectionPrompts({
    packageName,
    skillDir,
    version,
    hasIssues: hasGithub,
    hasDiscussions: hasGithub,
    hasReleases,
    hasChangelog,
    docFiles,
    docsType,
    hasShippedDocs,
    customPrompt,
    features,
    pkgFiles,
    overheadLines,
    sections: selectedSections,
  })

  if (sectionPrompts.size === 0) {
    return { optimized: '', wasOptimized: false, error: 'No valid sections to generate' }
  }

  const executorOrError = await selectExecutor(model)
  if ('error' in executorOrError)
    return { optimized: '', wasOptimized: false, error: executorOrError.error }
  const executor = executorOrError

  // Check per-section cache: references dir first (version-keyed), then LLM cache (prompt-hashed)
  const cachedResults: SectionResult[] = []
  const uncachedSections: Array<{ section: SkillSection, prompt: string }> = []

  for (const [section, prompt] of sectionPrompts) {
    if (!noCache) {
      if (version) {
        const outputFile = SECTION_OUTPUT_FILES[section]
        const refCached = cache.readSection(outputFile)
        if (refCached) {
          onProgress?.({ chunk: `[${section}: cached]`, type: 'text', text: refCached, reasoning: '', section })
          cachedResults.push({ section, content: refCached, wasOptimized: true })
          continue
        }
      }

      const cached = getCached(prompt, model, section)
      if (cached) {
        onProgress?.({ chunk: `[${section}: cached]`, type: 'text', text: cached, reasoning: '', section })
        cachedResults.push({ section, content: cached, wasOptimized: true })
        continue
      }
    }
    uncachedSections.push({ section, prompt })
  }

  const skilldDir = skillInternalDir(skillDir)
  mkdirSync(skilldDir, { recursive: true })

  // Pre-flight: warn about broken symlinks in .skilld/ (avoids wasting tokens on missing refs)
  for (const entry of readdirSync(skilldDir)) {
    const entryPath = join(skilldDir, entry)
    try {
      if (lstatSync(entryPath).isSymbolicLink() && !existsSync(entryPath))
        onProgress?.({ chunk: `[warn: broken symlink .skilld/${entry}]`, type: 'reasoning', text: '', reasoning: '' })
    }
    catch {}
  }

  const preExistingFiles = new Set(readdirSync(skilldDir))

  // Spawn uncached sections with staggered starts to avoid rate-limit collisions
  const STAGGER_MS = 3000
  const spawnResults = uncachedSections.length > 0
    ? await Promise.allSettled(
        uncachedSections.map(({ section, prompt }, i) => {
          const outputFile = SECTION_OUTPUT_FILES[section]
          const run = () => optimizeSection({
            section,
            prompt,
            outputFile,
            skillDir,
            executor,
            onProgress,
            timeout,
            debug,
            preExistingFiles,
          })
          if (i === 0)
            return run()
          return delay(i * STAGGER_MS).then(run)
        }),
      )
    : []

  const allResults: SectionResult[] = [...cachedResults]
  let totalUsage: { input: number, output: number } | undefined
  let totalCost = 0
  const retryQueue: Array<{ index: number, section: SkillSection, prompt: string }> = []

  for (let i = 0; i < spawnResults.length; i++) {
    const r = spawnResults[i]!
    const { section, prompt } = uncachedSections[i]!
    if (r.status === 'fulfilled' && r.value.wasOptimized) {
      allResults.push(r.value)
      if (r.value.usage) {
        totalUsage = totalUsage ?? { input: 0, output: 0 }
        totalUsage.input += r.value.usage.input
        totalUsage.output += r.value.usage.output
      }
      if (r.value.cost != null)
        totalCost += r.value.cost
      if (!noCache)
        setCache(prompt, model, section, r.value.content)
    }
    else {
      retryQueue.push({ index: i, section, prompt })
    }
  }

  // Retry failed sections (sequential, with rate-limit aware backoff)
  for (const { index, section, prompt } of retryQueue) {
    const prevError = getRetryError(spawnResults[index]!)
    const rateLimitDelay = parseRateLimitDelay(prevError)

    if (rateLimitDelay != null) {
      const waitSec = Math.max(rateLimitDelay, 5)
      onProgress?.({ chunk: `[${section}] Rate limited, waiting ${waitSec}s...`, type: 'reasoning', text: '', reasoning: '', section })
      await delay(waitSec * 1000)
    }
    else {
      onProgress?.({ chunk: `[${section}: retrying...]`, type: 'reasoning', text: '', reasoning: '', section })
      await delay(STAGGER_MS)
    }

    const result = await optimizeSection({
      section,
      prompt,
      outputFile: SECTION_OUTPUT_FILES[section],
      skillDir,
      executor,
      onProgress,
      timeout,
      debug,
      preExistingFiles,
    }).catch((err: Error) => ({ section, content: '', wasOptimized: false, error: err.message }) as SectionResult)

    allResults.push(result)
    if (result.wasOptimized && !noCache)
      setCache(prompt, model, section, result.content)
    if (result.usage) {
      totalUsage = totalUsage ?? { input: 0, output: 0 }
      totalUsage.input += result.usage.input
      totalUsage.output += result.usage.output
    }
    if (result.cost != null)
      totalCost += result.cost
  }

  // Write successful sections to global references dir for cross-project reuse
  if (version) {
    const sectionFiles = allResults
      .filter(r => r.wasOptimized && r.content)
      .map(r => ({ file: SECTION_OUTPUT_FILES[r.section], content: r.content }))
    if (sectionFiles.length > 0) {
      cache.writeSections(sectionFiles)
    }
  }

  // Merge results in SECTION_MERGE_ORDER, wrapped with comment markers
  const mergedParts: string[] = []
  for (const section of SECTION_MERGE_ORDER) {
    const result = allResults.find(r => r.section === section)
    if (result?.wasOptimized && result.content) {
      mergedParts.push(wrapSection(section, result.content))
    }
  }

  const optimized = mergedParts.join('\n\n')
  const wasOptimized = mergedParts.length > 0

  const usageResult = totalUsage
    ? { inputTokens: totalUsage.input, outputTokens: totalUsage.output, totalTokens: totalUsage.input + totalUsage.output }
    : undefined

  const errors = allResults.filter(r => r.error).map(r => `${r.section}: ${r.error}`)
  const warnings = allResults.flatMap(r => r.warnings ?? []).map(w => `${w.section}: ${w.warning}`)

  const debugLogsDir = debug && uncachedSections.length > 0
    ? skillLogDir(skillDir)
    : undefined

  return {
    optimized,
    wasOptimized,
    error: errors.length > 0 ? errors.join('; ') : undefined,
    warnings: warnings.length > 0 ? warnings : undefined,
    finishReason: wasOptimized ? 'stop' : 'error',
    usage: usageResult,
    cost: totalCost || undefined,
    debugLogsDir,
  }
}

// ── Helpers ──────────────────────────────────────────────────────────

function isRateLimitError(error: string | undefined): boolean {
  if (!error)
    return false
  return STATIC_REGEX_1.test(error)
    || STATIC_REGEX_2.test(error)
    || STATIC_REGEX_3.test(error)
    || STATIC_REGEX_4.test(error)
}

function parseRateLimitDelay(error: string | undefined): number | undefined {
  if (!error || !isRateLimitError(error))
    return undefined
  const match = error.match(STATIC_REGEX_5)
  return match ? Number(match[1]) : 10
}

function getRetryError(result: PromiseSettledResult<SectionResult>): string | undefined {
  if (result.status === 'rejected')
    return String(result.reason)
  return result.value.error
}
