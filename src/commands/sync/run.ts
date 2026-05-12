/**
 * Unified sync run factory.
 *
 * Owns orchestration for sync flows. Frontends supply:
 *   - a `PackageResolver` (npm/crate/github)
 *   - hook bindings (UI surfaces: clack, parallel logUpdate, etc.)
 *
 * Hooks fire for every observable phase: resolve, fetch, index, base,
 * sections-cached, enhance, shipped, warn. Each payload carries `spec`
 * so a single hook bus handles concurrent specs.
 */

import type { Hookable } from 'hookable'
import type { OptimizeModel, SkillSection, StreamProgress } from '../../agent/index.ts'
import type { ResolveAttempt } from '../../sources/index.ts'
import type { LlmConfig } from '../llm-prompts.ts'
import type { ReadyState } from './phases.ts'
import type { PackageResolver } from './resolvers.ts'
import { createHooks } from 'hookable'
import pLimit from 'p-limit'
import { runBaseSync, runEnhancePhase } from './phases.ts'

export interface EnhanceDoneInfo {
  usage?: { totalTokens: number }
  cost?: number
  debugLogsDir?: string
  error?: string
  warnings?: string[]
}

export interface SyncHooks {
  'resolve:start': (info: { spec: string }) => void
  'resolve:progress': (info: { spec: string, message: string }) => void
  'resolve:done': (info: { spec: string, version: string, cached: boolean, force?: boolean }) => void
  'resolve:failed': (info: { spec: string, identityName: string, attempts: ResolveAttempt[] }) => void
  'dist:downloading': (info: { spec: string }) => void
  'fetch:start': (info: { spec: string }) => void
  'fetch:progress': (info: { spec: string, message: string }) => void
  'fetch:done': (info: { spec: string, parts: string[], cached: boolean }) => void
  'index:start': (info: { spec: string }) => void
  'index:progress': (info: { spec: string, message: string }) => void
  'index:done': (info: { spec: string }) => void
  'base:done': (info: { spec: string, skillDir: string, mode: 'add' | 'update' }) => void
  'sections:cached': (info: { spec: string }) => void
  'enhance:start': (info: { spec: string, modelLabel: string }) => void
  'enhance:progress': (info: { spec: string, progress: StreamProgress }) => void
  'enhance:done': (info: { spec: string } & EnhanceDoneInfo) => void
  'enhance:failed': (info: { spec: string, error: string, rateLimited: boolean }) => void
  'shipped:installed': (info: { spec: string, skillName: string, skillDir: string }) => void
  'warn': (info: { spec?: string, message: string }) => void
}

export const SYNC_HOOK_NAMES: ReadonlyArray<keyof SyncHooks> = [
  'resolve:start',
  'resolve:progress',
  'resolve:done',
  'resolve:failed',
  'dist:downloading',
  'fetch:start',
  'fetch:progress',
  'fetch:done',
  'index:start',
  'index:progress',
  'index:done',
  'base:done',
  'sections:cached',
  'enhance:start',
  'enhance:progress',
  'enhance:done',
  'enhance:failed',
  'shipped:installed',
  'warn',
]

export interface CreateSyncRunOptions {
  cwd: string
  resolver: PackageResolver
  agent: import('../../agent/index.ts').AgentType
  global: boolean
  mode?: 'add' | 'update'
  force?: boolean
  noSearch?: boolean
  name?: string
  from?: string
  debug?: boolean
  eject?: boolean | string
  defaultSections: SkillSection[]
  /**
   * LLM config resolver. Called once when the first ready state arrives
   * (sequential) or once for the whole batch (parallel) with the
   * aggregated update context. Return `null` to skip enhancement.
   */
  resolveLlmConfig?: (info: { ready: ReadonlyArray<{ spec: string, state: ReadyState }> }) => Promise<LlmConfig | null> | LlmConfig | null
  /**
   * Called when a merge-needed state is hit. Sequential frontends supply
   * a handler that regenerates SKILL.md; parallel frontends pass undefined
   * and the run reports the spec as an error.
   */
  onMergeNeeded?: (state: import('./phases.ts').MergeNeededState) => Promise<void>
}

export type RunOutcome
  = | { kind: 'shipped', spec: string }
    | { kind: 'unresolved', spec: string, identityName: string, attempts: ResolveAttempt[] }
    | { kind: 'merged', spec: string }
    | { kind: 'ready', spec: string, state: ReadyState }
    | { kind: 'enhanced', spec: string, state: ReadyState }
    | { kind: 'error', spec: string, reason: string }

export interface SyncRun {
  hooks: Hookable<SyncHooks>
  /** Phase 1 only: resolve + fetch + write base skill. Returns outcome (no enhance). */
  runBase: (spec: string) => Promise<RunOutcome>
  /** Phase 2: enhance via LLM and finalize. */
  runEnhance: (state: ReadyState, llmConfig: LlmConfig | null) => Promise<void>
  /** Full pipeline for a single spec: base + (per-spec llm prompt) + enhance. */
  run: (spec: string) => Promise<RunOutcome>
  /** Parallel base sync over many specs; returns per-spec outcomes. */
  runMany: (specs: string[], opts?: { concurrency?: number }) => Promise<RunOutcome[]>
}

export function createSyncRun(opts: CreateSyncRunOptions): SyncRun {
  const hooks = createHooks<SyncHooks>()

  async function runBase(spec: string): Promise<RunOutcome> {
    const result = await runBaseSync(
      spec,
      {
        agent: opts.agent,
        global: opts.global,
        mode: opts.mode,
        force: opts.force,
        noSearch: opts.noSearch,
        name: opts.name,
        from: opts.from,
        eject: opts.eject,
      },
      hooks,
      opts.resolver,
      opts.cwd,
      opts.defaultSections,
    )

    if (result.kind === 'shipped')
      return { kind: 'shipped', spec }
    if (result.kind === 'unresolved') {
      return {
        kind: 'unresolved',
        spec,
        identityName: result.unresolved.identityName,
        attempts: result.unresolved.attempts,
      }
    }
    if (result.kind === 'merge-needed') {
      if (opts.onMergeNeeded) {
        await opts.onMergeNeeded(result.state)
        return { kind: 'merged', spec }
      }
      return {
        kind: 'error',
        spec,
        reason: `Skill dir already holds ${result.state.existingLock.packageName} — run sequentially to merge`,
      }
    }
    return { kind: 'ready', spec, state: result.state }
  }

  async function runEnhance(state: ReadyState, llmConfig: LlmConfig | null): Promise<void> {
    await runEnhancePhase(
      state,
      llmConfig,
      {
        agent: opts.agent,
        global: opts.global,
        force: opts.force,
        debug: opts.debug,
        eject: opts.eject,
      },
      hooks,
      opts.cwd,
    )
  }

  async function run(spec: string): Promise<RunOutcome> {
    const base = await runBase(spec)
    if (base.kind !== 'ready')
      return base

    let llmConfig: LlmConfig | null = null
    if (!base.state.allSectionsCached && opts.resolveLlmConfig)
      llmConfig = await opts.resolveLlmConfig({ ready: [{ spec, state: base.state }] }) ?? null

    await runEnhance(base.state, llmConfig)
    return { kind: 'enhanced', spec, state: base.state }
  }

  async function runMany(specs: string[], runOpts?: { concurrency?: number }): Promise<RunOutcome[]> {
    const limit = pLimit(runOpts?.concurrency ?? 5)
    const results = await Promise.allSettled(specs.map(spec => limit(() => runBase(spec))))
    return results.map((r, i): RunOutcome =>
      r.status === 'fulfilled'
        ? r.value
        : { kind: 'error', spec: specs[i]!, reason: r.reason instanceof Error ? r.reason.message : String(r.reason) },
    )
  }

  return { hooks, runBase, runEnhance, run, runMany }
}

export type { OptimizeModel }
