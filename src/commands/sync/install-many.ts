/**
 * Install many skills from a parsed source list. Routes each `SkillSource`
 * to the right pipeline (git, npm registry → npm doc fallback, crate) and
 * collects per-item outcomes for telemetry and `pull` summaries.
 */

import type { AgentType, OptimizeModel } from '../../agent/index.ts'
import type { SkillSource } from '../../core/prefix.ts'
import type { AuditResult, RegistryClient, RepositoryRef } from '../../registry/client.ts'
import type { GitSkillSource } from '../../sources/git-skills.ts'
import { styleText } from 'node:util'
import * as p from '@clack/prompts'
import { introLine } from '../../cli/intro.ts'
import { COMMA_OR_WHITESPACE_RE } from '../../core/regex.ts'
import { getProjectState } from '../../core/skills.ts'
import { createRegistryClient, gateInstall } from '../../registry/client.ts'
import { manifestToSources } from '../../registry/collections.ts'
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
  repositories: RepositoryRef[]
}

const RECEIPTS_URL = 'https://skilld.dev/gh'

function repositoryKey(repo: RepositoryRef): string {
  return `${repo.owner}/${repo.repo}`
}

function addRepository(repositories: RepositoryRef[], repository: RepositoryRef): void {
  if (!repositories.some(repo => repositoryKey(repo) === repositoryKey(repository)))
    repositories.push(repository)
}

function manifestItemsToSources(items: Parameters<typeof manifestToSources>[0]): SkillSource[] {
  return manifestToSources(items).map(({ source, skillFilter }) =>
    source.type === 'git' ? { ...source, skillFilter } : source,
  )
}

type MissingCollectionAction
  = | { _tag: 'Fail' }
    | { _tag: 'UseNpm', package: string }

export async function expandPublicSources(
  items: SkillSource[],
  client: Pick<RegistryClient, 'fetchCollection' | 'fetchCurator'>,
  yes: boolean,
): Promise<{ items: SkillSource[], skipped: number, failed: number }> {
  const expanded: SkillSource[] = []
  let skipped = 0
  let failed = 0

  const loadCollection = async (
    handle: string,
    name: string,
    confirm: boolean,
    onMissing: MissingCollectionAction,
  ): Promise<void> => {
    let transportFailed = false
    const manifest = await client.fetchCollection(handle, name).catch((error) => {
      p.log.error(`Failed to load @${handle}/${name}: ${error instanceof Error ? error.message : String(error)}`)
      transportFailed = true
      failed += 1
      return null
    })
    if (!manifest) {
      if (!transportFailed && onMissing._tag === 'UseNpm') {
        expanded.push({ type: 'bare', package: onMissing.package })
      }
      else if (!transportFailed) {
        p.log.error(`Collection @${handle}/${name} was not found.`)
        failed += 1
      }
      return
    }
    if (manifest.items.length === 0) {
      p.log.info(`Collection @${handle}/${name} is empty.`)
      skipped += 1
      return
    }
    if (manifest.preamble)
      p.note(manifest.preamble, manifest.name)
    if (confirm && !yes) {
      const accepted = await p.confirm({ message: `Install ${manifest.items.length} skills from @${handle}/${name}?` })
      if (p.isCancel(accepted) || !accepted) {
        skipped += 1
        return
      }
    }
    expanded.push(...manifestItemsToSources(manifest.items))
  }

  for (const source of items) {
    if (source.type === 'collection-or-npm') {
      await loadCollection(source.handle, source.name, true, { _tag: 'UseNpm', package: source.package })
      continue
    }
    if (source.type !== 'curator') {
      expanded.push(source)
      continue
    }

    let transportFailed = false
    const curator = await client.fetchCurator(source.handle).catch((error) => {
      p.log.error(`Failed to load @${source.handle}: ${error instanceof Error ? error.message : String(error)}`)
      transportFailed = true
      failed += 1
      return null
    })
    if (!curator) {
      if (!transportFailed) {
        p.log.error(`Curator @${source.handle} was not found.`)
        failed += 1
      }
      continue
    }
    if (curator.collections.length === 0) {
      p.log.info(`Curator @${source.handle} has no collections.`)
      skipped += 1
      continue
    }

    let slugs = curator.collections.map(collection => collection.slug)
    if (!yes) {
      const selection = await p.multiselect({
        message: `Select collections from @${source.handle}`,
        required: false,
        options: curator.collections.map(collection => ({
          label: collection.name,
          value: collection.slug,
          hint: `${collection.itemCount} skills`,
        })),
      })
      if (p.isCancel(selection)) {
        skipped += 1
        continue
      }
      slugs = selection as string[]
    }
    for (const slug of slugs)
      await loadCollection(source.handle, slug, false, { _tag: 'Fail' })
  }

  return { items: expanded, skipped, failed }
}

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
  const result = await client.audit({ owner, repo, name }).catch((error) => {
    p.log.warn(`Audit unavailable for ${key}: ${error instanceof Error ? error.message : String(error)}`)
    return { status: 'unaudited' as const, audits: [] }
  })
  cache.set(key, result)
  return result
}

function logAuditWarn(slug: string, result: AuditResult): void {
  const parts = [
    result.riskLevel && `risk: ${result.riskLevel}`,
    result.summary,
    result.audits.filter(a => a.status === 'warn').map(a => a.slug).join(','),
  ].filter(Boolean).join(' · ')
  p.log.warn(`${styleText('yellow', '⚠')} ${slug} ${styleText('gray', parts)}`)
}

function logAuditFail(slug: string, result: AuditResult, owner: string, repo: string, name: string): void {
  const detail = result.audits.filter(a => a.status === 'fail').map(a => a.summary || a.slug).join('; ')
  p.log.error(`${styleText('red', '✗')} ${slug} blocked: ${detail || 'audit failed'}\n  Receipts: ${RECEIPTS_URL}/${owner}/${repo}/${name}`)
}

export async function installSkills(items: SkillSource[], opts: InstallOpts): Promise<InstallSummary> {
  const cwd = process.cwd()
  const summary: InstallSummary = { installed: 0, skipped: 0, failed: 0, repositories: [] }
  const client = createRegistryClient()
  const auditCache = opts.auditCache ?? new Map<string, AuditResult>()
  const publicSources = await expandPublicSources(items, client, !!opts.yes)
  summary.skipped += publicSources.skipped
  summary.failed += publicSources.failed

  const gitSources: Array<{ source: GitSkillSource, skillFilter?: string }> = []
  const npmEntries: Array<{ name: string, spec: string }> = []
  const crateSpecs: string[] = []

  for (const source of publicSources.items) {
    switch (source.type) {
      case 'git':
        gitSources.push({ source: source.source, skillFilter: source.skillFilter })
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
      case 'collection-or-npm':
        throw new Error(`Unexpanded public source: ${JSON.stringify(source)}`)
      default: {
        const _exhaustive: never = source
        throw new Error(`Unhandled SkillSource type: ${JSON.stringify(_exhaustive)}`)
      }
    }
  }

  for (const { source, skillFilter: perSourceFilter } of gitSources) {
    // Per-source filter (from a pull manifest) wins over the global flag.
    const filterRaw = perSourceFilter ?? opts.skillFilter
    const skillFilter = filterRaw
      ? filterRaw.split(COMMA_OR_WHITESPACE_RE).map(s => s.trim()).filter(Boolean)
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
      .then(() => {
        summary.installed += 1
        if (source.type === 'github' && source.owner && source.repo)
          addRepository(summary.repositories, { owner: source.owner, repo: source.repo })
      })
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
      const resolved = await client.resolveSkill(entry.name).catch((error) => {
        p.log.warn(`Registry unavailable for ${entry.name}: ${error instanceof Error ? error.message : String(error)}. Using package documentation.`)
        return null
      })
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
        addRepository(summary.repositories, { owner: auditOwner!, repo: auditRepo! })
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
