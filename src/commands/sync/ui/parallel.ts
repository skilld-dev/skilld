/**
 * Parallel logUpdate UI binder. Subscribes to a shared state map (keyed by
 * spec) and re-renders the full table on each event.
 */

import type { Hookable } from 'hookable'
import type { SyncHooks } from '../run.ts'
import logUpdate from 'log-update'
import { formatDuration } from '../../../core/formatting.ts'

export type PackageStatus = 'pending' | 'resolving' | 'downloading' | 'embedding' | 'exploring' | 'thinking' | 'generating' | 'done' | 'error'

export interface PackageState {
  name: string
  status: PackageStatus
  message: string
  version?: string
  streamPreview?: string
  startedAt?: number
  completedAt?: number
}

const STATUS_ICONS: Record<PackageStatus, string> = {
  pending: '○',
  resolving: '◐',
  downloading: '◒',
  embedding: '◓',
  exploring: '◔',
  thinking: '◔',
  generating: '◑',
  done: '✓',
  error: '✗',
}

const STATUS_COLORS: Record<PackageStatus, string> = {
  pending: '\x1B[90m',
  resolving: '\x1B[36m',
  downloading: '\x1B[36m',
  embedding: '\x1B[36m',
  exploring: '\x1B[34m',
  thinking: '\x1B[35m',
  generating: '\x1B[33m',
  done: '\x1B[32m',
  error: '\x1B[31m',
}

export interface ParallelRender {
  states: Map<string, PackageState>
  verb: string
  total: number
}

export function renderParallel(r: ParallelRender): void {
  const maxNameLen = Math.max(...[...r.states.keys()].map(n => n.length), 20)
  const lines = [...r.states.values()].map((s) => {
    const icon = STATUS_ICONS[s.status]
    const color = STATUS_COLORS[s.status]
    const reset = '\x1B[0m'
    const dim = '\x1B[90m'
    const name = s.name.padEnd(maxNameLen)
    const version = s.version ? `${dim}${s.version}${reset} ` : ''
    const elapsed = (s.status === 'done' || s.status === 'error') && s.startedAt && s.completedAt
      ? ` ${dim}(${formatDuration(s.completedAt - s.startedAt)})${reset}`
      : ''
    const preview = s.streamPreview ? ` ${dim}${s.streamPreview}${reset}` : ''
    return `  ${color}${icon}${reset} ${name} ${version}${s.message}${elapsed}${preview}`
  })

  const doneCount = [...r.states.values()].filter(s => s.status === 'done').length
  const errorCount = [...r.states.values()].filter(s => s.status === 'error').length
  const header = `\x1B[1m${r.verb} ${r.total} packages\x1B[0m (${doneCount} done${errorCount > 0 ? `, ${errorCount} failed` : ''})\n`

  logUpdate(header + lines.join('\n'))
}

/**
 * Subscribe the parallel renderer to the hook bus. Each event mutates the
 * slot for `spec` and triggers a full re-render.
 */
export function bindParallelUi(hooks: Hookable<SyncHooks>, render: ParallelRender): void {
  function update(spec: string, status: PackageStatus, message: string, ver?: string): void {
    const state = render.states.get(spec)
    if (!state)
      return
    if (!state.startedAt && status !== 'pending')
      state.startedAt = performance.now()
    if ((status === 'done' || status === 'error') && !state.completedAt)
      state.completedAt = performance.now()
    state.status = status
    state.message = message
    state.streamPreview = undefined
    if (ver)
      state.version = ver
    renderParallel(render)
  }

  hooks.hook('resolve:start', ({ spec }) => update(spec, 'resolving', 'Resolving...'))
  hooks.hook('resolve:progress', ({ spec, message }) => update(spec, 'resolving', message))
  hooks.hook('resolve:done', ({ spec, version, cached, force }) => {
    update(spec, 'downloading', cached ? 'Using cache' : force ? 'Re-fetching docs...' : 'Fetching docs...', version)
  })
  hooks.hook('resolve:failed', () => {
    // No-op: frontend sets error with reason via outcome
  })
  hooks.hook('dist:downloading', ({ spec }) => update(spec, 'downloading', 'Downloading dist...'))
  hooks.hook('fetch:start', () => {
    // Already in 'downloading'
  })
  hooks.hook('fetch:progress', ({ spec, message }) => update(spec, 'downloading', message))
  hooks.hook('fetch:done', ({ spec }) => update(spec, 'downloading', 'Linking references...'))
  hooks.hook('index:start', ({ spec }) => update(spec, 'embedding', 'Indexing docs'))
  hooks.hook('index:progress', ({ spec, message }) => update(spec, 'embedding', message))
  hooks.hook('index:done', () => {
    // Stay embedding until base:done
  })
  hooks.hook('warn', () => {
    // Warnings surfaced after parallel render completes
  })
  hooks.hook('base:done', ({ spec, mode }) => {
    update(spec, 'done', mode === 'update' ? 'Skill updated' : 'Base skill created')
  })
  hooks.hook('sections:cached', () => {
    // Frontend logs aggregate "Applied cached..." line
  })
  hooks.hook('enhance:start', ({ spec, modelLabel }) => update(spec, 'generating', modelLabel))
  hooks.hook('enhance:progress', ({ spec, progress }) => {
    const isReasoning = progress.type === 'reasoning'
    const status: PackageStatus = isReasoning ? 'exploring' : 'generating'
    const sectionPrefix = progress.section ? `[${progress.section}] ` : ''
    update(spec, status, `${sectionPrefix}${progress.chunk}`)
  })
  hooks.hook('enhance:done', ({ spec }) => update(spec, 'done', 'Skill optimized'))
  hooks.hook('enhance:failed', ({ spec, error }) => update(spec, 'error', error))
  hooks.hook('shipped:installed', ({ spec }) => update(spec, 'done', 'Published SKILL.md'))
}
