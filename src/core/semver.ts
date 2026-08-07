/**
 * Thin semver wrappers that pin `loose: true` at every callsite.
 * Centralized so the loose flag stays consistent across the project.
 */

import { difference, isGreater, normalize } from 'verkit'

/** Returns the cleaned version if valid semver, null otherwise. */
export function semverValid(v: string): string | null {
  return normalize(v, { loose: true })
}

/** Compare two semver strings: returns true if a > b. Handles prereleases. */
export function semverGt(a: string, b: string): boolean {
  return isGreater(a, b, { loose: true })
}

/** Returns the semver diff type between two versions, or null if equal/invalid. */
export function semverDiff(a: string, b: string): string | null {
  return difference(a, b)
}

export interface DistTagEntry {
  version: string
  releasedAt?: string
}

export interface PickedTag {
  tag: string
  version: string
  releasedAt?: string
  prerelease: boolean
}

const SNAPSHOT_REGEX = /-[0-9a-f]{7,40}$/i

export function pickLatestTag(distTags?: Record<string, DistTagEntry>): PickedTag | null {
  if (!distTags)
    return null

  const candidates: PickedTag[] = []

  for (const [tag, entry] of Object.entries(distTags)) {
    if (!entry?.version)
      continue
    const cleaned = semverValid(entry.version)
    if (!cleaned)
      continue

    const prerelease = cleaned.includes('-')
    candidates.push({
      tag,
      version: cleaned,
      releasedAt: entry.releasedAt,
      prerelease,
    })
  }

  if (candidates.length === 0)
    return null

  const nonSnapshots = candidates.filter(c => !SNAPSHOT_REGEX.test(c.version))
  const pool = nonSnapshots.length > 0 ? nonSnapshots : candidates

  pool.sort((a, b) => {
    if (a.version === b.version) {
      if (a.tag === 'latest')
        return -1
      if (b.tag === 'latest')
        return 1
      return 0
    }
    return semverGt(a.version, b.version) ? -1 : 1
  })

  return pool[0] ?? null
}

