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
