/**
 * Generate pipeline — full SKILL.md generation + installation to target agents.
 *
 * Extends the sync pipeline with LLM generation:
 *   docs cached → LLM generates sections → assembles SKILL.md → installs to agent dir
 */

import type { OptimizeModel, SkillSection } from '../../src/agent/clis'
import type { AgentType } from '../../src/agent/types'
import type { PipelineResult } from '../e2e/pipeline'
import { execSync } from 'node:child_process'
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'pathe'
import { generateSkillMd, getModelLabel, optimizeDocs, sanitizeName } from '../../src/agent'
import { agents } from '../../src/agent/registry'
import {
  ensureCacheDir,
  getCacheDir,
  listReferenceFiles,
} from '../../src/cache'
import { linkAllReferences } from '../../src/cache/internal/references'
import { detectChangelog } from '../../src/commands/sync/pipeline'
import { getPkgKeyFiles, hasShippedDocs, resolvePkgDir } from '../../src/core/prepare'
import { runPipeline } from '../e2e/pipeline'
import { GENERATE_SECTIONS } from './generate-matrix'

// ── Types ───────────────────────────────────────────────────────────

export interface GenerateResult {
  /** Package name */
  package: string
  /** Generator model used */
  generator: OptimizeModel
  /** Sync pipeline result (doc resolution + caching) */
  sync: PipelineResult
  /** Full assembled SKILL.md content */
  skillMd: string
  /** Whether LLM optimization succeeded */
  wasOptimized: boolean
  /** LLM-generated body (sections merged) */
  optimizedBody?: string
  /** Token usage */
  usage?: { inputTokens: number, outputTokens: number, totalTokens: number }
  /** Cost in USD */
  cost?: number
  /** Error message if generation failed */
  error?: string
  /** Validation warnings from section outputs */
  warnings?: string[]
}

export interface InstallResult {
  targetAgent: AgentType
  /** Temporary skill directory */
  skillDir: string
  /** Path to SKILL.md */
  skillMdPath: string
  /** Whether SKILL.md was written successfully */
  exists: boolean
}

// ── CLI availability ────────────────────────────────────────────────

const CLI_MODELS_MAP: Partial<Record<OptimizeModel, string>> = {
  'opus': 'claude',
  'sonnet': 'claude',
  'haiku': 'claude',
  'gemini-3.1-pro': 'gemini',
  'gemini-3-flash': 'gemini',
  'gpt-5.3-codex': 'codex',
  'gpt-5.3-codex-spark': 'codex',
  'gpt-5.2-codex': 'codex',
  'gpt-5.1-codex-max': 'codex',
  'gpt-5.2': 'codex',
  'gpt-5.1-codex-mini': 'codex',
}

/** Check if the CLI for a given model is installed */
export function isGeneratorAvailable(model: OptimizeModel): boolean {
  const cli = CLI_MODELS_MAP[model]
  if (!cli)
    return false
  try {
    execSync(`which ${cli}`, { stdio: 'ignore' })
    return true
  }
  catch { return false }
}

// ── Pipeline ────────────────────────────────────────────────────────

/**
 * Ensure docs are cached for a package.
 * Runs the sync pipeline (resolution + caching) if not already cached.
 * Returns the pipeline result.
 */
export async function ensureDocs(packageName: string): Promise<PipelineResult> {
  ensureCacheDir()
  return runPipeline(packageName)
}

/**
 * Run LLM generation for a package using a specific model.
 * Assumes docs are already cached (call ensureDocs first).
 */
export async function runGenerate(
  packageName: string,
  generator: OptimizeModel,
  syncResult: PipelineResult,
): Promise<GenerateResult> {
  const version = syncResult.version
  const cwd = process.cwd()

  // Set up a temporary skill dir for generation (mimics real sync flow)
  const tmpBase = join(tmpdir(), 'skilld-generate-test', `${sanitizeName(packageName)}-${generator}`)
  const skillDir = join(tmpBase, sanitizeName(packageName))
  mkdirSync(join(skillDir, '.skilld'), { recursive: true })

  // Link references so the LLM can access docs
  linkAllReferences(skillDir, packageName, cwd, version, syncResult.docsType)

  const docFiles = listReferenceFiles(skillDir)
  const pkgDir = resolvePkgDir(packageName, cwd, version)
  const hasChangelog = detectChangelog(pkgDir, getCacheDir(packageName, version))
  const shippedDocs = hasShippedDocs(packageName, cwd, version)
  const hasGithub = existsSync(`${getCacheDir(packageName, version)}/issues`)
  const hasReleases = existsSync(`${getCacheDir(packageName, version)}/releases`)

  // Run LLM generation
  const { optimized, wasOptimized, usage, cost, error, warnings } = await optimizeDocs({
    packageName,
    skillDir,
    model: generator,
    version,
    hasGithub,
    hasReleases,
    hasChangelog,
    docFiles,
    docsType: syncResult.docsType,
    hasShippedDocs: shippedDocs,
    noCache: true, // Always regenerate in tests
    debug: true,
    sections: [...GENERATE_SECTIONS] as SkillSection[],
    timeout: 300_000, // 5 min per section
  })

  // Assemble full SKILL.md
  const pkgFiles = getPkgKeyFiles(packageName, cwd, version)
  const skillMd = generateSkillMd({
    name: packageName,
    version,
    releasedAt: syncResult.resolved.releasedAt,
    description: syncResult.resolved.description,
    dependencies: syncResult.resolved.dependencies,
    distTags: syncResult.resolved.distTags,
    body: wasOptimized ? optimized : undefined,
    relatedSkills: [],
    hasIssues: hasGithub,
    hasDiscussions: hasGithub,
    hasReleases,
    hasChangelog,
    docsType: syncResult.docsType,
    hasShippedDocs: shippedDocs,
    pkgFiles,
    generatedBy: wasOptimized ? getModelLabel(generator) : undefined,
  })

  return {
    package: packageName,
    generator,
    sync: syncResult,
    skillMd,
    wasOptimized,
    optimizedBody: wasOptimized ? optimized : undefined,
    usage,
    cost,
    error,
    warnings,
  }
}

/**
 * Install a generated SKILL.md to a target agent's skill directory (temp dir).
 * Returns the install result for validation.
 */
export function installToAgent(
  skillMd: string,
  packageName: string,
  targetAgent: AgentType,
  generator: OptimizeModel,
): InstallResult {
  const agent = agents[targetAgent]
  const tmpBase = join(tmpdir(), 'skilld-generate-test', `install-${generator}`)
  const baseDir = join(tmpBase, agent.skillsDir)
  const skillDir = join(baseDir, sanitizeName(packageName))
  const skilldDir = join(skillDir, '.skilld')

  mkdirSync(skilldDir, { recursive: true })

  const skillMdPath = join(skillDir, 'SKILL.md')
  writeFileSync(skillMdPath, skillMd)

  return {
    targetAgent,
    skillDir,
    skillMdPath,
    exists: existsSync(skillMdPath),
  }
}

// ── Artifacts ────────────────────────────────────────────────────────

const ARTIFACTS_DIR = join(import.meta.dirname, '../../.artifacts/generate')

export function writeGenerateArtifact(result: GenerateResult): void {
  const safePkg = result.package.replace(/\//g, '__')
  const dir = join(ARTIFACTS_DIR, safePkg, result.generator)
  mkdirSync(dir, { recursive: true })

  writeFileSync(join(dir, 'SKILL.md'), result.skillMd)
  writeFileSync(join(dir, 'result.json'), JSON.stringify({
    package: result.package,
    generator: result.generator,
    wasOptimized: result.wasOptimized,
    error: result.error,
    warnings: result.warnings,
    usage: result.usage,
    cost: result.cost,
    docsType: result.sync.docsType,
    cachedDocsCount: result.sync.cachedDocsCount,
  }, null, 2))

  if (result.optimizedBody)
    writeFileSync(join(dir, 'body.md'), result.optimizedBody)
}

export function writeInstallArtifact(
  result: GenerateResult,
  install: InstallResult,
): void {
  const safePkg = result.package.replace(/\//g, '__')
  const dir = join(ARTIFACTS_DIR, safePkg, result.generator, install.targetAgent)
  mkdirSync(dir, { recursive: true })

  writeFileSync(join(dir, 'install.json'), JSON.stringify({
    targetAgent: install.targetAgent,
    skillDir: install.skillDir,
    skillMdPath: install.skillMdPath,
    exists: install.exists,
  }, null, 2))

  if (install.exists)
    writeFileSync(join(dir, 'SKILL.md'), readFileSync(install.skillMdPath, 'utf-8'))
}
