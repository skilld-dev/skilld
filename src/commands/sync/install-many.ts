/**
 * Install many skills from a parsed source list. Routes each `SkillSource`
 * to the right pipeline (git, npm registry → npm doc fallback, crate) and
 * collects per-item outcomes for telemetry and `pull` summaries.
 */

import type { AgentType, OptimizeModel } from '../../agent/index.ts'
import type { SkillSource } from '../../core/prefix.ts'
import type { AuditResult, RegistryClient } from '../../registry/client.ts'
import type { GitSkillSource } from '../../sources/git-skills.ts'
import { styleText } from 'node:util'
import * as p from '@clack/prompts'
import { introLine } from '../../cli/intro.ts'
import { COMMA_OR_WHITESPACE_RE } from '../../core/regex.ts'
import { getProjectState } from '../../core/skills.ts'
import { createRegistryClient, gateInstall } from '../../registry/client.ts'
import { track } from '../../telemetry.ts'
import { syncGitSkills } from '../sync-git.ts'
import { syncCommand } from '../sync.ts'
import { syncRegistrySkill } from './registry.ts'

export type InstallSurface = 'cli:add' | 'cli:pull' | 'cli:prepare' | 'cli:update' | 'cli:wizard'

export interface InstallOpts {
  agent: AgentType
  surface: InstallSurface
  global?: boolean
  yes?: boolean
  force?: boolean
  debug?: boolean
  model?: OptimizeModel
  skillFilter?: string
  /** Allow installs that fail the upstream audit gate (Step 3 wiring). */
  allowUnsafe?: boolean
  /** Caller-supplied audit cache; pull populates this with pre-fetched results. */
  auditCache?: Map<string, AuditResult>
}

export interface InstallSummary {
  installed: number
  skipped: number
  failed: number
}

const RECEIPTS_URL = 'https://skilld.dev/gh'

async function getAudit(
  client: RegistryClient,
  cache: Map<string, AuditResult>,
  owner: string,
  repo: string,
  name: string,
): Promise<AuditResult> {
  const key = `${owner}/${repo}/${name}`
  const cached = cache.get(key)
  if (cached)
    return cached
  const result = await client.audit({ owner, repo, name })
  cache.set(key, result)
  return result
}

function logAuditWarn(slug: string, result: AuditResult): void {
  const parts = [
    result.riskLevel && `risk: ${result.riskLevel}`,
    result.summary,
    result.audits.filter(a => a.status === 'warn').map(a => a.category).join(','),
  ].filter(Boolean).join(' · ')
  p.log.warn(`${styleText('yellow', '⚠')} ${slug} ${styleText('gray', parts)}`)
}

function logAuditFail(slug: string, result: AuditResult, owner: string, repo: string, name: string): void {
  const detail = result.audits.filter(a => a.status === 'fail').map(a => a.summary || a.category).join('; ')
  p.log.error(`${styleText('red', '✗')} ${slug} blocked: ${detail || 'audit failed'}\n  Receipts: ${RECEIPTS_URL}/${owner}/${repo}/${name}`)
}

export async function installSkills(items: SkillSource[], opts: InstallOpts): Promise<InstallSummary> {
  const cwd = process.cwd()
  const summary: InstallSummary = { installed: 0, skipped: 0, failed: 0 }
  const client = createRegistryClient()
  const auditCache = opts.auditCache ?? new Map<string, AuditResult>()

  const gitSources: GitSkillSource[] = []
  const npmEntries: Array<{ name: string, spec: string }> = []
  const crateSpecs: string[] = []
  const unsupported: string[] = []

  for (const source of items) {
    switch (source.type) {
      case 'git':
        gitSources.push(source.source)
        break
      case 'npm':
        npmEntries.push({ name: source.package, spec: source.tag ? `${source.package}@${source.tag}` : source.package })
        break
      case 'crate':
        crateSpecs.push(source.version ? `crate:${source.package}@${source.version}` : `crate:${source.package}`)
        break
      case 'bare':
        p.log.warn(`Bare names are deprecated. Use ${styleText('cyan', `npm:${source.package}`)} instead.`)
        npmEntries.push({ name: source.package, spec: source.tag ? `${source.package}@${source.tag}` : source.package })
        break
      case 'curator':
        unsupported.push(`@${source.handle} (curator)`)
        break
      case 'collection':
        unsupported.push(`@${source.handle}/${source.name} (collection)`)
        break
      default: {
        const _exhaustive: never = source
        throw new Error(`Unhandled SkillSource type: ${JSON.stringify(_exhaustive)}`)
      }
    }
  }

  if (unsupported.length > 0) {
    p.log.error(`Curator and collection installs are not yet available:\n  ${unsupported.join('\n  ')}\n\nFollow https://skilld.dev for launch updates.`)
    summary.skipped += unsupported.length
    process.exitCode = 1
    if (gitSources.length === 0 && npmEntries.length === 0 && crateSpecs.length === 0)
      return summary
  }

  for (const source of gitSources) {
    const skillFilter = opts.skillFilter
      ? opts.skillFilter.split(COMMA_OR_WHITESPACE_RE).map(s => s.trim()).filter(Boolean)
      : undefined
    await syncGitSkills({
      source,
      global: !!opts.global,
      agent: opts.agent,
      yes: !!opts.yes,
      model: opts.model,
      force: opts.force,
      debug: opts.debug,
      skillFilter,
    })
      .then(() => { summary.installed += 1 })
      .catch((err) => {
        summary.failed += 1
        p.log.error(`Failed to install ${source.type === 'local' ? source.localPath : `${source.owner}/${source.repo}`}: ${err instanceof Error ? err.message : String(err)}`)
      })
  }

  if (npmEntries.length > 0) {
    const seen = new Set<string>()
    const dedupedEntries = npmEntries.filter((e) => {
      if (seen.has(e.name))
        return false
      seen.add(e.name)
      return true
    })

    const fallbackPackages: string[] = []
    for (const entry of dedupedEntries) {
      const resolved = await client.resolveSkill(entry.name).catch(() => null)
      if (!resolved) {
        fallbackPackages.push(entry.spec)
        continue
      }

      const [auditOwner, auditRepo] = resolved.repo.split('/')
      const audit = await getAudit(client, auditCache, auditOwner!, auditRepo!, resolved.name)
      const decision = gateInstall(audit, { allowUnsafe: opts.allowUnsafe, yes: opts.yes, sourceKind: 'npm' })

      const slug = `${resolved.repo}/${resolved.name}`
      if (audit.status === 'warn') {
        logAuditWarn(slug, audit)
        track({ event: 'audit-warn', surface: opts.surface, sourceKind: 'npm', slug, agent: opts.agent })
      }
      if (audit.status === 'fail') {
        logAuditFail(slug, audit, auditOwner!, auditRepo!, resolved.name)
        track({ event: 'audit-fail', surface: opts.surface, sourceKind: 'npm', slug, agent: opts.agent })
      }
      if (decision === 'skip') {
        track({ event: 'audit-blocked', surface: opts.surface, sourceKind: 'npm', slug, agent: opts.agent })
        summary.skipped += 1
        continue
      }

      const result = await syncRegistrySkill({ packageName: entry.name, agent: opts.agent, cwd, prefetched: resolved, surface: opts.surface })
        .catch((err) => {
          summary.failed += 1
          p.log.error(`Failed to install ${entry.name}: ${err instanceof Error ? err.message : String(err)}`)
          return null
        })
      if (result) {
        p.log.success(`Installed ${styleText('cyan', result.name)} from registry`)
        summary.installed += 1
      }
      else if (result === null) {
        fallbackPackages.push(entry.spec)
      }
    }

    if (fallbackPackages.length > 0) {
      const state = await getProjectState(cwd)
      p.intro(introLine({ state, agentId: opts.agent }))
      await syncCommand(state, {
        packages: [...fallbackPackages, ...crateSpecs],
        global: !!opts.global,
        agent: opts.agent,
        model: opts.model,
        yes: !!opts.yes,
        force: opts.force,
        debug: opts.debug,
      })
      summary.installed += fallbackPackages.length + crateSpecs.length
      return summary
    }
  }

  if (crateSpecs.length > 0) {
    const state = await getProjectState(cwd)
    p.intro(introLine({ state, agentId: opts.agent }))
    await syncCommand(state, {
      packages: crateSpecs,
      global: !!opts.global,
      agent: opts.agent,
      model: opts.model,
      yes: !!opts.yes,
      force: opts.force,
      debug: opts.debug,
    })
    summary.installed += crateSpecs.length
  }

  return summary
}
