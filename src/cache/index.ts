/**
 * Cache module — global doc caching with symlinks.
 *
 * Public surface:
 *   - createReferenceCache(name, version)  → per-package factory
 *   - createRepoCache(owner, repo)         → per-repo factory
 *   - registry-level ops (list/clear/utilities)
 */

export { CACHE_DIR, getPackageDbPath, getRepoCacheDir, REFERENCES_DIR, REPOS_DIR } from '../core/paths.ts'
export type { ReferenceCache, ReferenceCacheEjectOpts, ReferenceCacheLinkOpts } from './reference-cache.ts'
export { createReferenceCache } from './reference-cache.ts'
export {
  classifyCachedDoc,
  clearAllCachedPackages,
  clearCachedPackage,
  ensureCacheDir,
  getCacheDir,
  getCacheKey,
  getSkillReferenceDirs,
  getVersionKey,
  inferDocsTypeFromCache,
  listCachedPackages,
  listReferenceFiles,
} from './registry.ts'
export type { RepoCache } from './repo-cache.ts'
export { createRepoCache } from './repo-cache.ts'
export type { CacheConfig, CachedDoc, CachedPackage, CachedReferencesResult, LoadCachedReferencesOptions } from './types.ts'
