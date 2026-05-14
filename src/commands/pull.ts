/**
 * `skilld pull` — install skills from one of the user's collections.
 *
 * Headline authed command. Pulls the user's collections from skilld.dev,
 * optionally lets them pick one, fans out audit checks in parallel, and
 * presents a multiselect checklist with audit-status badges before handing
 * the selection to `installSkills` (surface = `cli:pull`).
 */

import type { SkillSource } from '../core/prefix.ts'
import type { AuditResult, AuditStatus, CollectionManifest, CollectionManifestItem, CollectionSummary } from '../registry/client.ts'
import { styleText } from 'node:util'
import * as p from '@clack/prompts'
import { defineCommand } from 'citty'
import { loadSession } from '../auth/store.ts'
import { autoResolveAgent } from '../cli/agent-prompt.ts'
import { sharedArgs } from '../cli/args.ts'
import { createRegistryClient } from '../registry/client.ts'
import { track } from '../telemetry.ts'
import { installSkills } from './sync/install-many.ts'

function manifestItemKey(item: CollectionManifestItem): string {
  if (item.kind === 'gh')
    return item.name ? `${item.owner}/${item.repo}/${item.name}` : `${item.owner}/${item.repo}`
  return item.package ?? `${item.kind}:unknown`
}

/**
 * Convert selected manifest items into `installSkills` inputs. Multiple gh
 * items in the same repo collapse to one `git` source carrying the union of
 * picked skill names as `skillFilter`, so a repo with N skills installs in
 * one `syncGitSkills` call instead of N redundant ones.
 */
function manifestToSources(items: CollectionManifestItem[]): Array<{ source: SkillSource, skillFilter?: string }> {
  const npm: Array<{ source: SkillSource, skillFilter?: string }> = []
  const crate: Array<{ source: SkillSource, skillFilter?: string }> = []
  const ghByRepo = new Map<string, { owner: string, repo: string, names: string[] }>()

  for (const item of items) {
    if (item.kind === 'npm' && item.package) {
      npm.push({ source: { type: 'npm', package: item.package } })
      continue
    }
    if (item.kind === 'crate' && item.package) {
      crate.push({ source: { type: 'crate', package: item.package } })
      continue
    }
    if (item.kind === 'gh' && item.owner && item.repo) {
      const key = `${item.owner}/${item.repo}`
      const group = ghByRepo.get(key) ?? { owner: item.owner, repo: item.repo, names: [] }
      if (item.name && !group.names.includes(item.name))
        group.names.push(item.name)
      ghByRepo.set(key, group)
    }
  }

  const gh: Array<{ source: SkillSource, skillFilter?: string }> = []
  for (const group of ghByRepo.values()) {
    gh.push({
      source: { type: 'git', source: { type: 'github', owner: group.owner, repo: group.repo } },
      skillFilter: group.names.length ? group.names.join(',') : undefined,
    })
  }

  return [...gh, ...npm, ...crate]
}

function badgeFor(status: AuditStatus, result: AuditResult): string {
  switch (status) {
    case 'pass':
      return styleText('green', '✓ audited')
    case 'warn':
      return styleText('yellow', `⚠ warn${result.summary ? `: ${result.summary}` : ''}`)
    case 'fail':
      return styleText('red', `✗ fail${result.summary ? `: ${result.summary}` : ''}`)
    case 'unaudited':
      return styleText('gray', '? unaudited')
  }
}

async function pickCollection(collections: CollectionSummary[], slug?: string): Promise<CollectionSummary | null> {
  if (slug) {
    const match = collections.find(c => c.slug === slug)
    if (!match) {
      p.log.error(`No collection with slug "${slug}". Available: ${collections.map(c => c.slug).join(', ')}`)
      return null
    }
    return match
  }
  if (collections.length === 0) {
    p.log.warn('You have no collections on skilld.dev. Create one at https://skilld.dev/me/collections')
    return null
  }
  if (collections.length === 1)
    return collections[0]!

  const choice = await p.select({
    message: 'Pick a collection',
    options: collections.map(c => ({ label: c.name, value: c.slug, hint: `${c.itemCount} skills` })),
  })
  if (p.isCancel(choice))
    return null
  return collections.find(c => c.slug === choice) ?? null
}

export const pullCommandDef = defineCommand({
  meta: { name: 'pull', description: 'Install skills from one of your collections' },
  args: {
    'collection': { type: 'string', description: 'Collection slug to pull', valueHint: 'slug' },
    'all': { type: 'boolean', description: 'Install every item without prompting' },
    'allow-unsafe': { type: 'boolean', description: 'Install items that fail the audit gate' },
    ...sharedArgs,
  },
  async run({ args }) {
    const agent = autoResolveAgent(args.agent)
    if (!agent) {
      p.log.error('No target agent detected.\n  Pass --agent <name> (claude-code, cursor, codex, …), or run `skilld config` to set a default.')
      process.exitCode = 1
      return
    }

    const session = await loadSession()
    if (!session) {
      p.log.error('Not logged in. Run `skilld login` first.')
      process.exitCode = 1
      return
    }

    const client = createRegistryClient({ session })
    const collections = await client.my.collections()
    const picked = await pickCollection(collections, args.collection)
    if (!picked)
      return

    const manifest = await client.fetchCollection(session.login, picked.slug) as CollectionManifest | null
    if (!manifest) {
      p.log.error(`Failed to load collection manifest for @${session.login}/${picked.slug}.`)
      process.exitCode = 1
      return
    }
    if (manifest.items.length === 0) {
      p.log.warn('Collection is empty.')
      return
    }

    const auditCache = new Map<string, AuditResult>()
    const auditByKey = new Map<string, AuditResult>()

    const spin = p.spinner()
    spin.start(`Auditing ${manifest.items.length} items`)
    await Promise.all(manifest.items.map(async (item) => {
      // Audit needs the full (owner, repo, name) tuple. gh manifest items
      // carry `name` directly; npm/crate items don't, so they're unaudited
      // here and re-evaluated inside `installSkills` after resolve.
      if (item.kind !== 'gh' || !item.owner || !item.repo || !item.name) {
        auditByKey.set(manifestItemKey(item), { status: 'unaudited', audits: [] })
        return
      }
      const result = await client.audit({ owner: item.owner, repo: item.repo, name: item.name })
      auditCache.set(`${item.owner}/${item.repo}/${item.name}`, result)
      auditByKey.set(manifestItemKey(item), result)
    }))
    spin.stop(`Audited ${manifest.items.length} items`)

    track({
      event: 'pull-checklist',
      surface: 'cli:pull',
      sourceKind: 'collection',
      slug: `${session.login}/${picked.slug}`,
      agent,
    })

    let selected: CollectionManifestItem[]
    if (args.all || args.yes) {
      selected = manifest.items.filter((item) => {
        const audit = auditByKey.get(manifestItemKey(item))
        return args['allow-unsafe'] || audit?.status !== 'fail'
      })
    }
    else {
      const choice = await p.multiselect({
        message: `Select skills from ${manifest.name}`,
        required: false,
        initialValues: manifest.items
          .filter(item => auditByKey.get(manifestItemKey(item))?.status !== 'fail')
          .map(manifestItemKey),
        options: manifest.items.map((item) => {
          const key = manifestItemKey(item)
          const audit = auditByKey.get(key) ?? { status: 'unaudited' as const, audits: [] }
          return {
            label: `${key} ${styleText('gray', `(${item.kind})`)}`,
            value: key,
            hint: badgeFor(audit.status, audit),
          }
        }),
      })
      if (p.isCancel(choice))
        return
      const chosen = new Set(choice as string[])
      selected = manifest.items.filter(item => chosen.has(manifestItemKey(item)))
    }

    const sources = manifestToSources(selected)
    if (sources.length === 0) {
      p.log.info('Nothing to install.')
      return
    }
    // Carry per-source `skillFilter` on the git variant of SkillSource so
    // multiple skills under one repo collapse into a single syncGitSkills call.
    const items: SkillSource[] = sources.map(({ source, skillFilter }) =>
      source.type === 'git' ? { ...source, skillFilter } : source,
    )

    const summary = await installSkills(items, {
      agent,
      surface: 'cli:pull',
      yes: args.yes,
      force: args.force,
      debug: args.debug,
      allowUnsafe: args['allow-unsafe'],
      auditCache,
    })

    p.outro(`${summary.installed} installed · ${summary.skipped} skipped · ${summary.failed} failed`)
  },
})
