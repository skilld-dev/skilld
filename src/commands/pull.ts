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
import { promptForAgent, resolveAgent } from '../cli/agent-prompt.ts'
import { sharedArgs } from '../cli/args.ts'
import { createRegistryClient } from '../registry/client.ts'
import { track } from '../telemetry.ts'
import { installSkills } from './sync/install-many.ts'

function manifestToSource(item: CollectionManifestItem): SkillSource | null {
  if (item.kind === 'npm' && item.package)
    return { type: 'npm', package: item.package }
  if (item.kind === 'crate' && item.package)
    return { type: 'crate', package: item.package }
  if (item.kind === 'gh' && item.owner && item.repo)
    return { type: 'git', source: { type: 'github', owner: item.owner, repo: item.repo } }
  return null
}

function manifestItemKey(item: CollectionManifestItem): string {
  if (item.kind === 'gh')
    return `${item.owner}/${item.repo}`
  return item.package ?? `${item.kind}:unknown`
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
    let agent = resolveAgent(args.agent)
    if (!agent)
      agent = await promptForAgent()
    if (!agent || agent === 'none') {
      p.log.error('`skilld pull` requires an agent target.')
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
      if (item.kind === 'crate') {
        auditByKey.set(manifestItemKey(item), { status: 'unaudited', audits: [] })
        return
      }
      const owner = item.owner ?? (item.package?.split('/')[0])
      const repo = item.repo ?? (item.package?.split('/')[1] ?? item.package ?? '')
      const name = item.package ?? `${item.owner}/${item.repo}`
      if (!owner || !repo || !name) {
        auditByKey.set(manifestItemKey(item), { status: 'unaudited', audits: [] })
        return
      }
      const result = await client.audit({ owner, repo, name })
      auditCache.set(`${owner}/${repo}/${name}`, result)
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

    const items = selected.map(manifestToSource).filter((s): s is SkillSource => s !== null)
    if (items.length === 0) {
      p.log.info('Nothing to install.')
      return
    }

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
