import type { AgentType, OptimizeModel } from '../agent/index.ts'
import type { ProjectState } from '../core/skills.ts'
import type { ResolveAttempt } from '../sources/index.ts'
import type { RunBaseConfig } from './sync-runner.ts'
import * as p from '@clack/prompts'
import { relative } from 'pathe'
import { detectImportedPackages } from '../agent/index.ts'
import { suggestPrepareHook } from '../cli/prepare-hook.ts'
import { readConfig } from '../core/config.ts'
import { timedSpinner } from '../core/formatting.ts'
import { isCrateSpec } from '../core/prefix.ts'
import { shutdownWorker } from '../retriv/pool.ts'
import { searchNpmPackages } from '../sources/index.ts'
import { DEFAULT_SECTIONS, resolveAutoModel, selectLlmConfig } from './llm-prompts.ts'
import { handleMerge } from './sync-merge.ts'
import { syncPackagesParallel } from './sync-parallel.ts'
import { npmResolver } from './sync-resolvers.ts'
import { runBaseSync, runEnhancePhase } from './sync-runner.ts'
import { createClackUi } from './sync-ui-clack.ts'

const RESOLVE_SOURCE_LABELS: Record<string, string> = {
  'npm': 'npm registry',
  'github-docs': 'GitHub versioned docs',
  'github-meta': 'GitHub metadata',
  'github-search': 'GitHub search',
  'readme': 'README fallback',
  'llms.txt': 'llms.txt convention',
  'crawl': 'website crawl',
  'local': 'local node_modules',
}

function showResolveAttempts(attempts: ResolveAttempt[]): void {
  if (attempts.length === 0)
    return

  p.log.message('\x1B[90mDoc resolution:\x1B[0m')
  for (const attempt of attempts) {
    const icon = attempt.status === 'success' ? '\x1B[32m✓\x1B[0m' : '\x1B[90m✗\x1B[0m'
    const label = RESOLVE_SOURCE_LABELS[attempt.source] ?? attempt.source
    const source = `\x1B[90m${label}\x1B[0m`
    const msg = attempt.message ? ` \x1B[90m— ${attempt.message}\x1B[0m` : ''
    p.log.message(`  ${icon} ${source}${msg}`)
  }
}

export interface SyncOptions {
  packages?: string[]
  global: boolean
  agent: AgentType
  model?: OptimizeModel
  yes: boolean
  force?: boolean
  debug?: boolean
  mode?: 'add' | 'update'
  /** Eject mode: copy references as real files instead of symlinking */
  eject?: boolean | string
  /** Override the computed skill directory name */
  name?: string
  /** Lower-bound date for release/issue/discussion collection (ISO date, e.g. "2025-07-01") */
  from?: string
  /** Skip search index / embeddings generation */
  noSearch?: boolean
}

export async function syncCommand(state: ProjectState, opts: SyncOptions): Promise<void> {
  // If packages specified, sync those
  if (opts.packages && opts.packages.length > 0) {
    const crateSpecs = opts.packages.filter(isCrateSpec)
    const npmSpecs = opts.packages.filter(p => !isCrateSpec(p))

    // npm packages: parallel if >1, serial if 1
    if (npmSpecs.length > 1) {
      await syncPackagesParallel({
        packages: npmSpecs,
        global: opts.global,
        agent: opts.agent,
        model: opts.model,
        yes: opts.yes,
        force: opts.force,
        debug: opts.debug,
        mode: opts.mode,
      })
    }
    else if (npmSpecs.length === 1) {
      await syncSinglePackage(npmSpecs[0]!, opts)
    }

    // Crates: serialize (respect crates.io rate limits)
    for (const spec of crateSpecs)
      await syncSinglePackage(spec, opts)

    return
  }

  // Otherwise show picker, pre-selecting missing/outdated
  const packages = await interactivePicker(state)
  if (!packages || packages.length === 0) {
    p.outro('No packages selected')
    return
  }

  // Use parallel sync for multiple packages
  if (packages.length > 1) {
    return syncPackagesParallel({
      packages,
      global: opts.global,
      agent: opts.agent,
      model: opts.model,
      yes: opts.yes,
      force: opts.force,
      debug: opts.debug,
      mode: opts.mode,
    })
  }

  // Single package - use original flow
  await syncSinglePackage(packages[0]!, opts)
}

async function interactivePicker(state: ProjectState): Promise<string[] | null> {
  const spin = timedSpinner()
  spin.start('Detecting imports...')

  const cwd = process.cwd()
  const { packages: detected, error } = await detectImportedPackages(cwd)
  const declaredMap = state.deps

  if (error || detected.length === 0) {
    spin.stop(error ? `Detection failed: ${error}` : 'No imports detected')
    if (declaredMap.size === 0) {
      p.log.warn('No dependencies found')
      return null
    }
    // Fallback to package.json
    return pickFromList(Array.from(declaredMap.entries(), ([name, version]) => ({
      name,
      version: maskPatch(version),
      count: 0,
      inPkgJson: true,
    })), state)
  }

  spin.stop(`Loaded ${detected.length} project skills`)

  const packages = detected.map(pkg => ({
    name: pkg.name,
    version: declaredMap.get(pkg.name),
    count: pkg.count,
    inPkgJson: declaredMap.has(pkg.name),
  }))

  return pickFromList(packages, state)
}

function maskPatch(version: string | undefined): string | undefined {
  if (!version)
    return undefined
  const parts = version.split('.')
  if (parts.length >= 3) {
    parts[2] = 'x'
    return parts.slice(0, 3).join('.')
  }
  return version
}

async function pickFromList(
  packages: Array<{ name: string, version?: string, count: number, inPkgJson: boolean }>,
  state: ProjectState,
): Promise<string[] | null> {
  // Pre-select missing and outdated
  const missingSet = new Set(state.missing)
  const outdatedSet = new Set(state.outdated.map(s => s.name))

  const options = packages.map(pkg => ({
    label: pkg.inPkgJson ? `${pkg.name} ★` : pkg.name,
    value: pkg.name,
    hint: [
      maskPatch(pkg.version),
      pkg.count > 0 ? `${pkg.count} imports` : null,
    ].filter(Boolean).join(' · ') || undefined,
  }))

  const initialValues = packages
    .filter(pkg => missingSet.has(pkg.name) || outdatedSet.has(pkg.name))
    .map(pkg => pkg.name)

  const selected = await p.multiselect({
    message: 'Select packages to sync',
    options,
    required: false,
    initialValues,
  })

  if (p.isCancel(selected)) {
    p.cancel('Cancelled')
    return null
  }

  return selected as string[]
}

interface SyncConfig {
  global: boolean
  agent: AgentType
  model?: OptimizeModel
  yes: boolean
  force?: boolean
  debug?: boolean
  mode?: 'add' | 'update'
  eject?: boolean | string
  name?: string
  from?: string
  noSearch?: boolean
}

/**
 * Sequential sync via the unified runner. Handles npm/crate, add/update mode,
 * eject mode, merge, shipped skills, and "did-you-mean" suggestions.
 */
async function runSimpleSync(packageSpec: string, config: SyncConfig): Promise<void> {
  const cwd = process.cwd()
  const ui = createClackUi({ cwd })
  const isEject = !!config.eject

  const baseConfig: RunBaseConfig = {
    agent: config.agent,
    global: config.global,
    mode: config.mode,
    force: config.force,
    noSearch: config.noSearch,
    name: config.name,
    from: config.from,
    eject: config.eject,
  }

  const result = await runBaseSync(packageSpec, baseConfig, ui, npmResolver, cwd, DEFAULT_SECTIONS)

  if (result.kind === 'shipped') {
    p.outro(`Synced ${packageSpec}`)
    return
  }

  if (result.kind === 'unresolved') {
    const { unresolved } = result
    // Suggestion picker: only meaningful for npm specs (not crates).
    if (!isCrateSpec(packageSpec)) {
      const suggestions = await searchNpmPackages(unresolved.identityName)
      if (suggestions.length > 0) {
        showResolveAttempts(unresolved.attempts)
        const selected = await p.select({
          message: 'Did you mean one of these?',
          options: [
            ...suggestions.map(s => ({ label: s.name, value: s.name, hint: s.description })),
            { label: 'None of these', value: '_none_' as const },
          ],
        })
        if (!p.isCancel(selected) && selected !== '_none_')
          return syncSinglePackage(selected as string, config)
        return
      }
    }
    showResolveAttempts(unresolved.attempts)
    return
  }

  if (result.kind === 'merge-needed') {
    await handleMerge(result.state, { agent: config.agent, global: config.global }, cwd)
    return
  }

  // result.kind === 'ready'
  const { state } = result
  const globalConfig = readConfig()
  const resolvedModel = await resolveAutoModel(config.model, config.yes)

  let llmConfig: import('./llm-prompts.ts').LlmConfig | null = null
  if (!state.allSectionsCached && !globalConfig.skipLlm && !(config.yes && !resolvedModel))
    llmConfig = await selectLlmConfig(resolvedModel, undefined, state.updateCtx)

  await runEnhancePhase(
    state,
    llmConfig,
    { agent: config.agent, global: config.global, force: config.force, debug: config.debug, eject: config.eject },
    ui,
    cwd,
  )

  await shutdownWorker()
  const ejectMsg = isEject ? ' (ejected)' : ''
  const relDir = relative(cwd, state.skillDir)
  p.outro(config.mode === 'update'
    ? `Updated ${state.identityName}${ejectMsg}`
    : `Synced ${state.identityName} → ${relDir}${ejectMsg}`)

  try {
    await suggestPrepareHook(cwd)
  }
  catch (err) {
    p.log.warn(`Failed to suggest prepare hook: ${err instanceof Error ? err.message : String(err)}`)
  }
}

async function syncSinglePackage(packageSpec: string, config: SyncConfig): Promise<void> {
  if (isCrateSpec(packageSpec) && !packageSpec.slice('crate:'.length).trim()) {
    p.log.error('Invalid crate spec. Use format: crate:<name>')
    return
  }
  return runSimpleSync(packageSpec, config)
}
