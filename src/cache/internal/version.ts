/**
 * Version utilities
 */

import { resolve } from 'pathe'
import { REFERENCES_DIR } from '../../core/paths.ts'

/** Validate npm package name (scoped or unscoped) */
const VALID_PKG_NAME = /^(?:@[a-z0-9][-a-z0-9._]*\/)?[a-z0-9][-a-z0-9._]*$/

/** Validate version string (semver-ish, no path separators) */
const VALID_VERSION = /^[a-z0-9][-\w.+]*$/i

/**
 * Get exact version key for cache keying
 */
export function getVersionKey(version: string): string {
  return version
}

/**
 * Get cache key for a package: name@version
 */
export function getCacheKey(name: string, version: string): string {
  return `${name}@${getVersionKey(version)}`
}

/**
 * Get path to cached package references.
 * Validates name/version to prevent path traversal.
 */
export function getCacheDir(name: string, version: string): string {
  if (!VALID_PKG_NAME.test(name))
    throw new Error(`Invalid package name: ${name}`)
  if (!VALID_VERSION.test(version))
    throw new Error(`Invalid version: ${version}`)

  const dir = resolve(REFERENCES_DIR, getCacheKey(name, version))
  if (!dir.startsWith(REFERENCES_DIR))
    throw new Error(`Path traversal detected: ${dir}`)
  return dir
}
