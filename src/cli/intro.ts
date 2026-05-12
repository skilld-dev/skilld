import type { AgentType } from '../agent/index.ts'
import type { ProjectState } from '../core/skills.ts'
import { join } from 'pathe'
import { agents, detectInstalledAgents, getAgentVersion, getModelName } from '../agent/index.ts'
import { readPackageJsonSafe } from '../core/package-json.ts'
import { version } from '../version.ts'

export interface IntroOptions {
  state: ProjectState
  generators?: Array<{ name: string, version: string }>
  modelId?: string
  agentId?: string
}

export function getInstalledGenerators(): Array<{ name: string, version: string }> {
  const installed = detectInstalledAgents()
  return installed
    .filter(id => agents[id].cli)
    .map((id) => {
      const ver = getAgentVersion(id)
      return ver ? { name: agents[id].displayName, version: ver } : null
    })
    .filter((a): a is { name: string, version: string } => a !== null)
}

export function relativeTime(date: Date): string {
  const now = Date.now()
  const diff = now - date.getTime()
  const mins = Math.floor(diff / 60000)
  const hours = Math.floor(diff / 3600000)
  const days = Math.floor(diff / 86400000)
  if (mins < 1)
    return 'just now'
  if (mins < 60)
    return `${mins}m ago`
  if (hours < 24)
    return `${hours}h ago`
  return `${days}d ago`
}

export function getLastSynced(state: ProjectState): string | null {
  let latest: Date | null = null
  for (const skill of state.skills) {
    if (skill.info?.syncedAt) {
      const d = new Date(skill.info.syncedAt)
      if (!latest || d > latest)
        latest = d
    }
  }
  return latest ? relativeTime(latest) : null
}

export function introLine({ state, generators, modelId, agentId }: IntroOptions): string {
  const name = '\x1B[1m\x1B[35mskilld\x1B[0m'
  const ver = `\x1B[90mv${version}\x1B[0m`
  const lastSynced = getLastSynced(state)
  const synced = lastSynced ? ` · \x1B[90msynced ${lastSynced}\x1B[0m` : ''

  const parts: string[] = []
  if (modelId)
    parts.push(getModelName(modelId as any))
  else if (generators?.length)
    parts.push(generators.map(g => `${g.name} v${g.version}`).join(', '))
  if (agentId && agents[agentId as AgentType])
    parts.push(agents[agentId as AgentType].displayName)
  const statusLine = parts.length > 0
    ? `\n\x1B[90m↳ ${parts.join(' → ')}\x1B[0m`
    : ''

  return `${name} ${ver}${synced}${statusLine}`
}

export function formatStatus(synced: number, outdated: number): string {
  const parts: string[] = []
  if (synced > 0)
    parts.push(`\x1B[32m${synced} synced\x1B[0m`)
  if (outdated > 0)
    parts.push(`\x1B[33m${outdated} outdated\x1B[0m`)
  return `Skills: ${parts.join(' · ')}`
}

export function getRepoHint(name: string, cwd: string): string | undefined {
  const result = readPackageJsonSafe(join(cwd, 'node_modules', name, 'package.json'))
  if (!result)
    return undefined
  const pkg = result.parsed as Record<string, any>
  const url = typeof pkg.repository === 'string'
    ? pkg.repository
    : pkg.repository?.url
  if (!url)
    return undefined
  return url
    .replace(/^git\+/, '')
    .replace(/\.git$/, '')
    .replace(/^git:\/\//, 'https://')
    .replace(/^ssh:\/\/git@github\.com/, 'https://github.com')
    .replace(/^https?:\/\/(www\.)?github\.com\//, '')
}
