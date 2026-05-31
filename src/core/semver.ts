/**
 * Thin semver wrappers that pin `loose: true` at every callsite.
 * Centralized so the loose flag stays consistent across the project.
 */

import { diff as _diff, gt as _gt, major as _major, prerelease as _prerelease, valid as _valid } from 'semver'

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
  return _valid(v, true)
}

/** Compare two semver strings: returns true if a > b. Handles prereleases. */
export function semverGt(a: string, b: string): boolean {
  return _gt(a, b, true)
}

/** Returns the semver diff type between two versions, or null if equal/invalid. */
export function semverDiff(a: string, b: string): string | null {
  return _diff(a, b)
}

/** Major version number (e.g. `9.2.2` → 9, `1.0.0-rc.3` → 1), or null if invalid. */
export function semverMajor(v: string): number | null {
  const clean = _valid(v, true)
  return clean ? _major(clean) : null
}

/** True if `v` carries a prerelease component (e.g. 1.0.0-beta.8). */
export function semverIsPrerelease(v: string): boolean {
  return !!_prerelease(v, true)
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
