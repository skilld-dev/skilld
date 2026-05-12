/**
 * Parallel sync orchestrator. Two phased waves:
 *   1. base sync per pkg (pLimited) — fetch, cache, write base SKILL.md
 *   2. LLM enhancement per pkg (pLimited) — single combined `selectLlmConfig`
 *      prompt drives all pkgs in this wave
 *
 * Both waves use `runBaseSync` / `runEnhancePhase` from sync-runner; the
 * frontend just supplies a parallel `SyncUi` and orchestrates the pLimit.
 */

import type { AgentType, OptimizeModel } from '../agent/index.ts'
import type { ReadyState, RunBaseConfig } from './sync-runner.ts'
import type { PackageState, ParallelRender } from './sync-ui-parallel.ts'
import * as p from '@clack/prompts'
import logUpdate from 'log-update'
import pLimit from 'p-limit'
import { getModelLabel } from '../agent/index.ts'
import { ensureProjectFiles } from '../agent/skill-installer.ts'
import { ensureCacheDir, getVersionKey } from '../cache/index.ts'
import { readConfig } from '../core/config.ts'
import { semverDiff } from '../core/semver.ts'
import { shutdownWorker } from '../retriv/pool.ts'
import { parsePackageSpec, searchNpmPackages } from '../sources/index.ts'
import { DEFAULT_SECTIONS, resolveAutoModel, selectLlmConfig } from './llm-prompts.ts'
import { npmResolver } from './sync-resolvers.ts'
import { runBaseSync, runEnhancePhase } from './sync-runner.ts'
import { createParallelUi, renderParallel } from './sync-ui-parallel.ts'

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

/** Bump-type ordering for combining update contexts across packages. */
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

  const states = new Map<string, PackageState>()
  for (const spec of packages)
    states.set(spec, { name: spec, status: 'pending', message: 'Waiting...' })

  const render: ParallelRender = {
    states,
    verb: config.mode === 'update' ? 'Updating' : 'Syncing',
    total: packages.length,
  }

  ensureCacheDir()
  renderParallel(render)

  const limit = pLimit(concurrency)
  const baseConfig: RunBaseConfig = {
    agent: config.agent,
    global: config.global,
    mode: config.mode,
    force: config.force,
  }

  // ── Wave 1: base sync per pkg ──
  const baseResults = await Promise.allSettled(
    packages.map(spec =>
      limit(async () => {
        const { name } = parsePackageSpec(spec)
        const ui = createParallelUi(name, render)
        return runBaseSync(spec, baseConfig, ui, npmResolver, cwd, DEFAULT_SECTIONS)
      }),
    ),
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
      const npmAttempt = result.unresolved.attempts.find(a => a.source === 'npm')
      let reason: string
      if (npmAttempt?.status === 'not-found') {
        const suggestions = await searchNpmPackages(result.unresolved.identityName, 3)
        const hint = suggestions.length > 0 ? ` (try: ${suggestions.map(s => s.name).join(', ')})` : ''
        reason = (npmAttempt.message || 'Not on npm') + hint
      }
      else {
        const failed = result.unresolved.attempts.filter(a => a.status !== 'success')
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
    if (result.kind === 'merge-needed') {
      // Merge requires interactive context; surface and skip.
      errors.push({ spec, reason: `Skill dir already holds ${result.state.existingLock.packageName} — run sequentially to merge` })
      continue
    }
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

  // Pre-cached pkgs skip the LLM ask. `runBaseSync` already wrote the cached
  // SKILL.md and surfaced `allSectionsCached` on the ReadyState.
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
      // Reset slots so the prompt-only path renders cleanly per pkg.
      for (const r of uncached) {
        const slot = states.get(r.spec)
        if (slot) {
          slot.status = 'done'
          slot.message = 'Prompts written'
        }
      }
      renderParallel(render)
      // Phase 2 in promptOnly mode is a synchronous file-writes loop — no
      // need for parallelism. Reuse the same enhance phase via runEnhancePhase
      // (which dispatches to writePromptFiles when promptOnly is set).
      for (const r of uncached) {
        const ui = createParallelUi(r.spec, render, getVersionKey(r.state.version))
        await runEnhancePhase(r.state, llmConfig, {
          agent: config.agent,
          global: config.global,
          force: config.force,
          debug: config.debug,
        }, ui, cwd)
      }
    }
    else if (llmConfig) {
      p.log.step(getModelLabel(llmConfig.model))
      // Reset slots for the LLM phase so progress restarts visually.
      for (const r of uncached) {
        states.set(r.spec, { name: r.spec, status: 'pending', message: 'Waiting...', version: getVersionKey(r.state.version) })
      }
      renderParallel(render)

      const llmResults = await Promise.allSettled(
        uncached.map(r =>
          limit(async () => {
            const ui = createParallelUi(r.spec, render, getVersionKey(r.state.version))
            await runEnhancePhase(r.state, llmConfig, {
              agent: config.agent,
              global: config.global,
              force: config.force,
              debug: config.debug,
            }, ui, cwd)
          }),
        ),
      )

      logUpdate.done()
      const llmSucceeded = llmResults.filter(x => x.status === 'fulfilled').length
      p.log.success(`Enhanced ${llmSucceeded}/${uncached.length} skills with LLM`)
    }
  }
  else {
    // No LLM ask but we still need to link agents + ensure project files for
    // each ready pkg. `runEnhancePhase(state, null, ...)` does exactly that.
    for (const r of ready) {
      const ui = createParallelUi(r.spec, render, getVersionKey(r.state.version))
      await runEnhancePhase(r.state, null, {
        agent: config.agent,
        global: config.global,
        force: config.force,
        debug: config.debug,
      }, ui, cwd)
    }
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

/**
 * Combine per-package update contexts into a single aggregate for the LLM
 * config prompt. Picks the highest-rank bump, the earliest syncedAt, and
 * `allEnhanced` only when every pkg was previously LLM-enhanced.
 */
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
