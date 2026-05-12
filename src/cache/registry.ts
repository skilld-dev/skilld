/**
 * Registry-level cache operations and shared utilities. These don't bind to a
 * specific `(name, version)` or `(owner, repo)` tuple, so they live outside
 * the factories.
 */

import { existsSync, rmSync } from 'node:fs'
import { REPOS_DIR } from '../core/paths.ts'
import { classifyCachedDoc } from './internal/classify.ts'
import { getSkillReferenceDirs } from './internal/references.ts'
import {
  clearCache as clearStoredCache,
  ensureCacheDir,
  inferDocsTypeFromCache,
  listReferenceFiles,
  listCached as listStoredPackages,
} from './internal/storage.ts'
import { getCacheDir, getCacheKey, getVersionKey } from './internal/version.ts'

/** List all cached packages on disk */
export function listCachedPackages(): ReturnType<typeof listStoredPackages> {
  return listStoredPackages()
}

/** Clear cache for a specific package (returns true if anything was removed) */
export function clearCachedPackage(name: string, version: string): boolean {
  return clearStoredCache(name, version)
}

/** Clear every cached package + the repo-level cache. Returns count of cleared package dirs. */
export function clearAllCachedPackages(): number {
  const packages = listStoredPackages()
  for (const pkg of packages) {
    clearStoredCache(pkg.name, pkg.version)
  }
  if (existsSync(REPOS_DIR))
    rmSync(REPOS_DIR, { recursive: true })
  return packages.length
}

export {
  classifyCachedDoc,
  ensureCacheDir,
  getCacheDir,
  getCacheKey,
  getSkillReferenceDirs,
  getVersionKey,
  inferDocsTypeFromCache,
  listReferenceFiles,
}
