import type { AgentType } from '../agent/index.ts'
import type { readLock } from '../core/index.ts'
import type { SearchFilter } from '../retriv/index.ts'
import { existsSync, readdirSync } from 'node:fs'
import * as p from '@clack/prompts'
import { join } from 'pathe'
import { getPackageDbPath, REFERENCES_DIR } from '../cache/index.ts'
import { toStoragePackageName } from '../core/prefix.ts'
import { readProjectLock } from '../core/skills.ts'

const STATIC_REGEX_1 = /[-_/]+/
const STATIC_REGEX_2 = /^(issues?|docs?|releases?):(.+)$/i

/** Collect search.db paths for packages installed in the current project (from skilld-lock.yaml) */
export function findPackageDbs(packageFilter?: string, agentTypes?: AgentType[]): string[] {
  const cwd = process.cwd()
  const lock = readProjectLock(cwd, agentTypes)
  if (!lock)
    return []
  return filterLockDbs(lock, packageFilter)
}

/** Build package name → version map from the project lockfile */
export function getPackageVersions(cwd: string = process.cwd(), agentTypes?: AgentType[]): Map<string, string> {
  const lock = readProjectLock(cwd, agentTypes)
  const map = new Map<string, string>()
  if (!lock)
    return map
  for (const s of Object.values(lock.skills)) {
    if (s.packageName && s.version)
      map.set(s.packageName, s.version)
  }
  return map
}

/** List installed packages with versions from the project lockfile */
export function listLockPackages(cwd: string = process.cwd(), agentTypes?: AgentType[]): string[] {
  const lock = readProjectLock(cwd, agentTypes)
  if (!lock)
    return []
  const seen = new Map<string, string>()
  for (const s of Object.values(lock.skills)) {
    if (s.packageName && s.version)
      seen.set(s.packageName, s.version)
  }
  return Array.from(seen, ([name, version]) => `${name}@${version}`)
}

function filterLockDbs(lock: ReturnType<typeof readLock>, packageFilter?: string): string[] {
  if (!lock)
    return []
  const tokenize = (s: string) => s.toLowerCase().replace(/@/g, '').split(STATIC_REGEX_1).filter(Boolean)

  return Object.values(lock.skills)
    .filter((info) => {
      if (!info.packageName || !info.version)
        return false
      if (!packageFilter)
        return true
      const filterTokens = tokenize(packageFilter)
      const nameTokens = tokenize(info.packageName)
      return filterTokens.every(ft => nameTokens.some(nt => nt.includes(ft) || ft.includes(nt)))
    })
    .map((info) => {
      const storageName = toStoragePackageName(info.packageName!)
      const exact = getPackageDbPath(storageName, info.version!)
      if (existsSync(exact))
        return exact
      const fallback = findAnyPackageDb(storageName)
      if (fallback)
        p.log.warn(`Using cached search index for ${info.packageName} (v${info.version} not indexed). Run \`skilld update ${info.packageName}\` to re-index.`)
      return fallback
    })
    .filter((db): db is string => !!db)
}

/** Find any search.db for a package when exact version cache is missing */
function findAnyPackageDb(name: string): string | null {
  if (!existsSync(REFERENCES_DIR))
    return null

  const prefix = `${name}@`

  if (name.startsWith('@')) {
    const [scope, pkg] = name.split('/')
    const scopeDir = join(REFERENCES_DIR, scope!)
    if (!existsSync(scopeDir))
      return null
    const scopePrefix = `${pkg}@`
    for (const entry of readdirSync(scopeDir)) {
      if (entry.startsWith(scopePrefix)) {
        const db = join(scopeDir, entry, 'search.db')
        if (existsSync(db))
          return db
      }
    }
    return null
  }

  for (const entry of readdirSync(REFERENCES_DIR)) {
    if (entry.startsWith(prefix)) {
      const db = join(REFERENCES_DIR, entry, 'search.db')
      if (existsSync(db))
        return db
    }
  }
  return null
}

/** Parse filter prefix (e.g., "issues:bug" -> filter by type=issue, query="bug") */
export function parseFilterPrefix(rawQuery: string): { query: string, filter?: SearchFilter } {
  const prefixMatch = rawQuery.match(STATIC_REGEX_2)
  if (!prefixMatch)
    return { query: rawQuery }

  const prefix = prefixMatch[1]!.toLowerCase()
  const query = prefixMatch[2]!
  if (prefix.startsWith('issue'))
    return { query, filter: { type: 'issue' } }
  if (prefix.startsWith('release'))
    return { query, filter: { type: 'release' } }
  return { query, filter: { type: { $in: ['doc', 'docs'] } } }
}
