/**
 * Parallel sync orchestrator. Two phased waves:
 *   1. base sync per pkg (pLimited) — fetch, cache, write base SKILL.md
 *   2. LLM enhancement per pkg (pLimited)
 *
 * Both waves use `createSyncRun()` with a parallel UI binder. State map is
 * keyed by raw spec to match hook payloads.
 */

import type { AgentType, OptimizeModel } from '../agent/index.ts'
import type { ReadyState } from './sync/phases.ts'
import type { PackageState, ParallelRender } from './sync/ui/parallel.ts'
import * as p from '@clack/prompts'
import logUpdate from 'log-update'
import pLimit from 'p-limit'
import { getModelLabel } from '../agent/index.ts'
import { ensureProjectFiles } from '../agent/skill-installer.ts'
import { ensureCacheDir, getVersionKey } from '../cache/index.ts'
import { readConfig } from '../core/config.ts'
import { semverDiff } from '../core/semver.ts'
import { parsePackageSpec } from '../core/url.ts'
import { shutdownWorker } from '../retriv/pool.ts'
import { searchNpmPackages } from '../sources/index.ts'
import { DEFAULT_SECTIONS, resolveAutoModel, selectLlmConfig } from './llm-prompts.ts'
import { npmResolver } from './sync/resolvers.ts'
import { createSyncRun } from './sync/run.ts'
import { bindParallelUi, renderParallel } from './sync/ui/parallel.ts'

export interface ParallelSyncConfig {
  packages: string[]
  global: boolean
  agent: AgentType
  model?: OptimizeModel
  yes?: boolean
  force?: boolean
  debug?: boolean
  concurrency?: number
  mode?: 'add' | 'update'
}

const DIFF_RANK: Record<string, number> = {
  major: 5,
  premajor: 4,
  minor: 3,
  preminor: 2,
  patch: 1,
  prepatch: 1,
  prerelease: 0,
}

export async function syncPackagesParallel(config: ParallelSyncConfig): Promise<void> {
  const { packages, concurrency = 5 } = config
  const cwd = process.cwd()

  // State map keyed by display name (matches the original behavior where
  // parsePackageSpec(spec).name was used as the slot key).
  const states = new Map<string, PackageState>()
  const specToName = new Map<string, string>()
  for (const spec of packages) {
    const { name } = parsePackageSpec(spec)
    specToName.set(spec, name)
    states.set(spec, { name, status: 'pending', message: 'Waiting...' })
  }

  const render: ParallelRender = {
    states,
    verb: config.mode === 'update' ? 'Updating' : 'Syncing',
    total: packages.length,
  }

  ensureCacheDir()
  renderParallel(render)

  const run = createSyncRun({
    cwd,
    resolver: npmResolver,
    agent: config.agent,
    global: config.global,
    mode: config.mode,
    force: config.force,
    debug: config.debug,
    defaultSections: DEFAULT_SECTIONS,
  })
  bindParallelUi(run.hooks, render)

  // ── Wave 1: base sync per pkg ──
  const limit = pLimit(concurrency)
  const baseResults = await Promise.allSettled(
    packages.map(spec => limit(() => run.runBase(spec))),
  )

  logUpdate.done()

  const ready: Array<{ spec: string, state: ReadyState }> = []
  const shippedCount: string[] = []
  const errors: Array<{ spec: string, reason: string }> = []
  const aggregatedWarnings: string[] = []

  for (let i = 0; i < baseResults.length; i++) {
    const spec = packages[i]!
    const r = baseResults[i]!
    if (r.status === 'rejected') {
      const err = r.reason
      const reason = err instanceof Error ? err.message : String(err)
      const slot = states.get(spec)
      if (slot)
        slot.status = 'error'
      errors.push({ spec, reason })
      continue
    }
    const result = r.value
    if (result.kind === 'shipped') {
      shippedCount.push(spec)
      continue
    }
    if (result.kind === 'unresolved') {
      const npmAttempt = result.attempts.find(a => a.source === 'npm')
      let reason: string
      if (npmAttempt?.status === 'not-found') {
        const suggestions = await searchNpmPackages(result.identityName, 3)
        const hint = suggestions.length > 0 ? ` (try: ${suggestions.map(s => s.name).join(', ')})` : ''
        reason = (npmAttempt.message || 'Not on npm') + hint
      }
      else {
        const failed = result.attempts.filter(a => a.status !== 'success')
        reason = failed.map(a => a.message || a.source).join('; ') || 'No docs found'
      }
      const slot = states.get(spec)
      if (slot) {
        slot.status = 'error'
        slot.message = reason
      }
      errors.push({ spec, reason })
      continue
    }
    if (result.kind === 'error') {
      errors.push({ spec, reason: result.reason })
      continue
    }
    if (result.kind === 'ready')
      ready.push({ spec, state: result.state })
  }

  renderParallel(render)
  logUpdate.done()

  const pastVerb = config.mode === 'update' ? 'Updated' : 'Created'
  p.log.success(`${pastVerb} ${ready.length} base skills${shippedCount.length > 0 ? ` (${shippedCount.length} shipped)` : ''}`)

  for (const w of aggregatedWarnings)
    p.log.warn(`\x1B[33m${w}\x1B[0m`)
  for (const { spec, reason } of errors)
    p.log.error(`  ${spec}: ${reason}`)

  const cachedPkgs: string[] = []
  const uncached: typeof ready = []
  for (const r of ready) {
    if (r.state.allSectionsCached)
      cachedPkgs.push(r.spec)
    else
      uncached.push(r)
  }
  if (cachedPkgs.length > 0)
    p.log.success(`Applied cached SKILL.md sections for ${cachedPkgs.join(', ')}`)

  // ── Wave 2: LLM enhancement ──
  const globalConfig = readConfig()
  const resolvedModel = await resolveAutoModel(config.model, config.yes)
  const shouldAskLlm = uncached.length > 0 && !globalConfig.skipLlm && !(config.yes && !resolvedModel)

  if (shouldAskLlm) {
    const updateCtx = config.mode === 'update' ? aggregateUpdateCtx(uncached) : undefined
    const llmConfig = await selectLlmConfig(resolvedModel, undefined, updateCtx)

    if (llmConfig?.promptOnly) {
      for (const r of uncached) {
        const slot = states.get(r.spec)
        if (slot) {
          slot.status = 'done'
          slot.message = 'Prompts written'
        }
      }
      renderParallel(render)
      for (const r of uncached)
        await run.runEnhance(r.state, llmConfig)
    }
    else if (llmConfig) {
      p.log.step(getModelLabel(llmConfig.model))
      for (const r of uncached) {
        const displayName = specToName.get(r.spec) ?? r.spec
        states.set(r.spec, { name: displayName, status: 'pending', message: 'Waiting...', version: getVersionKey(r.state.version) })
      }
      renderParallel(render)

      const llmResults = await Promise.allSettled(
        uncached.map(r => limit(() => run.runEnhance(r.state, llmConfig))),
      )

      logUpdate.done()
      const llmSucceeded = llmResults.filter(x => x.status === 'fulfilled').length
      p.log.success(`Enhanced ${llmSucceeded}/${uncached.length} skills with LLM`)
    }
  }
  else {
    for (const r of ready)
      await run.runEnhance(r.state, null)
  }

  await ensureProjectFiles({ cwd, agent: config.agent, global: config.global })

  await shutdownWorker()

  p.outro(`${pastVerb} ${ready.length}/${packages.length} packages`)

  const { suggestPrepareHook } = await import('../cli/prepare-hook.ts')
  try {
    await suggestPrepareHook(cwd)
  }
  catch (err) {
    p.log.warn(`Failed to suggest prepare hook: ${err instanceof Error ? err.message : String(err)}`)
  }
}

function aggregateUpdateCtx(ready: Array<{ state: ReadyState }>): import('./llm-prompts.ts').UpdateContext {
  let maxDiff = ''
  let allEnhanced = true
  let anySyncedAt: string | undefined
  for (const r of ready) {
    const u = r.state.updateCtx
    if (!u?.wasEnhanced)
      allEnhanced = false
    if (u?.syncedAt && (!anySyncedAt || u.syncedAt < anySyncedAt))
      anySyncedAt = u.syncedAt
    if (u?.oldVersion && u.newVersion) {
      const diff = semverDiff(u.oldVersion, u.newVersion)
      if (diff && (DIFF_RANK[diff] ?? 0) > (DIFF_RANK[maxDiff] ?? -1))
        maxDiff = diff
    }
  }
  const first = ready[0]?.state.updateCtx
  return {
    oldVersion: ready.length === 1 ? first?.oldVersion : undefined,
    newVersion: ready.length === 1 ? first?.newVersion : undefined,
    syncedAt: anySyncedAt,
    wasEnhanced: allEnhanced,
    bumpType: maxDiff || undefined,
  }
}
