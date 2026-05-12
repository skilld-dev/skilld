/**
 * Sequential clack-based UI binder. Subscribes a fresh spinner/taskLog set
 * to the sync hook bus. Used by single-package sync flows.
 */

import type { Hookable } from 'hookable'
import type { SyncHooks } from '../run.ts'
import * as p from '@clack/prompts'
import { relative } from 'pathe'
import { timedSpinner } from '../../../core/formatting.ts'

export interface ClackUiOptions {
  cwd: string
}

export function bindClackUi(hooks: Hookable<SyncHooks>, { cwd }: ClackUiOptions): void {
  let spinner: ReturnType<typeof timedSpinner> | null = null
  let resourceSpinner: ReturnType<typeof timedSpinner> | null = null
  let indexSpinner: ReturnType<typeof timedSpinner> | null = null
  let llmLog: ReturnType<typeof p.taskLog> | null = null
  let currentSpec = ''

  hooks.hook('resolve:start', ({ spec }) => {
    currentSpec = spec
    spinner = timedSpinner()
    spinner.start(`Resolving ${spec}`)
  })
  hooks.hook('resolve:progress', ({ message }) => {
    spinner?.message(message)
  })
  hooks.hook('resolve:done', ({ version, cached, force }) => {
    const suffix = force ? ' (force)' : cached ? ' (cached)' : ''
    spinner?.stop(`Resolved ${currentSpec}@${version}${suffix}`)
    spinner = null
  })
  hooks.hook('resolve:failed', ({ identityName }) => {
    spinner?.stop(`Could not find docs for: ${identityName}`)
    spinner = null
  })
  hooks.hook('dist:downloading', () => {
    spinner?.message('Downloading dist')
  })
  hooks.hook('fetch:start', () => {
    resourceSpinner = timedSpinner()
    resourceSpinner.start('Finding resources')
  })
  hooks.hook('fetch:progress', ({ message }) => {
    resourceSpinner?.message(message)
  })
  hooks.hook('fetch:done', ({ parts, cached }) => {
    const summary = parts.length > 0 ? parts.join(', ') : 'resources'
    resourceSpinner?.stop(cached ? `Loaded ${summary} (cached)` : `Fetched ${summary}`)
    resourceSpinner = null
  })
  hooks.hook('index:start', () => {
    indexSpinner = timedSpinner()
    indexSpinner.start('Creating search index')
  })
  hooks.hook('index:progress', ({ message }) => {
    indexSpinner?.message(message)
  })
  hooks.hook('index:done', () => {
    indexSpinner?.stop('Search index ready')
    indexSpinner = null
  })
  hooks.hook('warn', ({ message }) => {
    p.log.warn(`\x1B[33m${message}\x1B[0m`)
  })
  hooks.hook('base:done', ({ skillDir, mode }) => {
    p.log.success(mode === 'update' ? `Updated skill: ${skillDir}` : `Created base skill: ${skillDir}`)
  })
  hooks.hook('sections:cached', () => {
    p.log.success('Applied cached SKILL.md sections')
  })
  hooks.hook('enhance:start', ({ spec, modelLabel }) => {
    currentSpec = spec
    p.log.step(modelLabel)
    llmLog = p.taskLog({ title: `Agent exploring ${spec}`, limit: 3 })
  })
  hooks.hook('enhance:progress', ({ progress }) => {
    if (!llmLog)
      return
    const sectionPrefix = progress.section ? `[${progress.section}] ` : ''
    const line = `${sectionPrefix}${progress.chunk}`
    llmLog.message(line)
  })
  hooks.hook('enhance:done', (info) => {
    if (!llmLog)
      return
    const parts: string[] = []
    if (info.usage)
      parts.push(`${Math.round(info.usage.totalTokens / 1000)}k tokens`)
    if (info.cost)
      parts.push(`$${info.cost.toFixed(2)}`)
    const suffix = parts.length > 0 ? ` (${parts.join(', ')})` : ''
    llmLog.success(`Generated best practices${suffix}`)
    llmLog = null
    if (info.debugLogsDir)
      p.log.info(`Debug logs: ${relative(cwd, info.debugLogsDir)}`)
    if (info.error)
      p.log.warn(`\x1B[33mPartial failure: ${info.error}\x1B[0m`)
    if (info.warnings) {
      for (const w of info.warnings)
        p.log.warn(`\x1B[33m${w}\x1B[0m`)
    }
  })
  hooks.hook('enhance:failed', ({ error, rateLimited }) => {
    if (!llmLog)
      return
    if (rateLimited)
      llmLog.error(`Rate limited by LLM provider. Try again shortly or use a different model via \`skilld config\``)
    else
      llmLog.error(`Enhancement failed${error ? `: ${error}` : ''}`)
    llmLog = null
  })
  hooks.hook('shipped:installed', ({ skillName, skillDir }) => {
    p.log.success(`Using published SKILL.md: ${skillName} → ${relative(cwd, skillDir)}`)
  })
}
