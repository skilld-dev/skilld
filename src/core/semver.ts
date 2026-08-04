/**
 * Thin semver wrappers that pin `loose: true` at every callsite.
 * Centralized so the loose flag stays consistent across the project.
 */

import { difference, getMajor, getPrerelease, isGreater, normalize } from 'verkit'

export interface DistTagVersion {
  version: string
  releasedAt?: string
}

export interface PickedTag {
  /** dist-tag name, e.g. 'latest', 'beta', 'rc', 'next'. */
  tag: string
  version: string
  releasedAt?: string
  /** True when the picked version is a prerelease (beta/rc/etc). */
  prerelease: boolean
}

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

/** Major version number (e.g. `9.2.2` → 9, `1.0.0-rc.3` → 1), or null if invalid. */
export function semverMajor(v: string): number | null {
  const clean = semverValid(v)
  return clean ? getMajor(clean) : null
}

/** True if `v` carries a prerelease component (e.g. 1.0.0-beta.8). */
export function semverIsPrerelease(v: string): boolean {
  const clean = semverValid(v)
  return clean ? (getPrerelease(clean)?.length ?? 0) > 0 : false
}

/** Trailing git short-hash, optionally `g`-prefixed: `-5d5b77c`, `-gabc1234`. */
const SNAPSHOT_RE = /-g?[0-9a-f]{7,40}$/i

/**
 * True for per-commit snapshot publishes (e.g. drizzle's `1.0.0-rc.4-5d5b77c`).
 * These get published under throwaway CI branch dist-tags and should not be the
 * canonical version a guide targets.
 */
export function isSnapshotVersion(v: string): boolean {
  return SNAPSHOT_RE.test(v)
}

/**
 * Pick the largest version across all npm dist-tags, including prereleases.
 *
 * npm publishes prereleases under tags like `beta`/`rc`/`next` while `latest`
 * stays on the last stable. semver ranks `1.0.0-beta.8 > 0.44.7`, so the max
 * naturally surfaces the bleeding-edge release we want to index (the drizzle
 * `1.0.0-beta.8` case). Ties resolve to the `latest` tag, then alphabetically
 * for determinism. Invalid versions are skipped; returns null if none valid.
 */
export function pickLatestTag(distTags: Record<string, DistTagVersion> | undefined): PickedTag | null {
  if (!distTags)
    return null

  const valid = Object.entries(distTags)
    .filter(([, info]) => info?.version && semverValid(info.version))

  if (!valid.length)
    return null

  // Prefer clean releases; fall back to snapshots only if that's all there is.
  const clean = valid.filter(([, info]) => !isSnapshotVersion(info.version))
  const candidates = clean.length ? clean : valid

  let [bestTag, bestInfo] = candidates[0]!
  for (const [tag, info] of candidates.slice(1)) {
    if (semverGt(info.version, bestInfo.version)
      || (info.version === bestInfo.version && tag === 'latest')) {
      bestTag = tag
      bestInfo = info
    }
  }

  return {
    tag: bestTag,
    version: bestInfo.version,
    releasedAt: bestInfo.releasedAt,
    prerelease: semverIsPrerelease(bestInfo.version),
  }
}
