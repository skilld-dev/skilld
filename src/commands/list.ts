import { defineCommand } from 'citty'
import { sharedArgs } from '../cli/args.ts'
import { formatSource, timeAgo } from '../core/formatting.ts'
import { getProjectState, iterateSkills } from '../core/skills.ts'

export interface ListOptions {
  global?: boolean
  json?: boolean
  outdated?: boolean
}

interface ListEntry {
  name: string
  version: string
  source: string
  synced: string
  latest?: string
}

export async function listCommand(opts: ListOptions = {}): Promise<void> {
  if (opts.outdated) {
    const state = await getProjectState()
    const entries: ListEntry[] = state.outdated.map(skill => ({
      name: skill.name,
      version: skill.info?.version || '',
      latest: skill.latestVersion || '',
      source: formatSource(skill.info?.source),
      synced: timeAgo(skill.info?.syncedAt),
    }))

    if (opts.json) {
      process.stdout.write(`${JSON.stringify(entries)}\n`)
      return
    }

    if (entries.length === 0) {
      process.stdout.write('All skills are up to date\n')
      return
    }

    const nameW = Math.max(...entries.map(e => e.name.length))
    const verW = Math.max(...entries.map(e => e.version.length))
    const latW = Math.max(...entries.map(e => (e.latest || '').length))
    const srcW = Math.max(...entries.map(e => e.source.length))

    for (const e of entries) {
      const line = [
        e.name.padEnd(nameW),
        `${e.version.padEnd(verW)}  →  ${(e.latest || '').padEnd(latW)}`,
        e.source.padEnd(srcW),
        e.synced,
      ].join('  ')
      process.stdout.write(`${line}\n`)
    }
    return
  }

  const scope = opts.global ? 'global' : 'all'
  const skills = [...iterateSkills({ scope })]

  // Deduplicate by package identity
  const seen = new Set<string>()
  const entries: ListEntry[] = []

  for (const skill of skills) {
    const key = skill.info?.packageName || skill.name
    if (seen.has(key))
      continue
    seen.add(key)
    entries.push({
      name: skill.name,
      version: skill.info?.version || '',
      source: formatSource(skill.info?.source),
      synced: timeAgo(skill.info?.syncedAt),
    })
  }

  if (opts.json) {
    process.stdout.write(`${JSON.stringify(entries)}\n`)
    return
  }

  if (entries.length === 0) {
    process.stdout.write('No skills installed\n')
    return
  }

  // Column widths
  const nameW = Math.max(...entries.map(e => e.name.length))
  const verW = Math.max(...entries.map(e => e.version.length))
  const srcW = Math.max(...entries.map(e => e.source.length))

  for (const e of entries) {
    const line = [
      e.name.padEnd(nameW),
      e.version.padEnd(verW),
      e.source.padEnd(srcW),
      e.synced,
    ].join('  ')
    process.stdout.write(`${line}\n`)
  }
}

export const listCommandDef = defineCommand({
  meta: { name: 'list', description: 'List installed skills' },
  args: {
    global: sharedArgs.global,
    json: {
      type: 'boolean' as const,
      description: 'Output as JSON',
      default: false,
    },
    outdated: {
      type: 'boolean' as const,
      alias: 'o',
      description: 'Show only outdated skills',
      default: false,
    },
  },
  run({ args }) {
    return listCommand({ global: args.global, json: args.json, outdated: args.outdated })
  },
})
