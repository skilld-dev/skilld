/**
 * Cache types
 */

import type { IndexDoc } from '../sources/content-resolver.ts'

export interface CacheConfig {
  /** Package name */
  name: string
  /** Package version (full semver) */
  version: string
}

export interface CachedPackage {
  name: string
  version: string
  dir: string
}

export interface CachedDoc {
  path: string
  content: string
}

export interface CachedReferencesResult {
  /** Docs to feed the embedding index (empty if db already exists) */
  docsToIndex: IndexDoc[]
  /** Resolved doc-source label (URL or git path) */
  docSource: string
  /** Detected docs type from cache layout */
  docsType: 'docs' | 'llms.txt' | 'readme'
  /** _INDEX.md to write if missing (backfill for older caches) */
  backfillIndex?: { path: string, content: string }
}

export interface LoadCachedReferencesOptions {
  packageName: string
  version: string
  repoUrl?: string
  llmsUrl?: string
  readmeUrl?: string
  onProgress: (message: string) => void
  /** Caller supplies index generator to avoid the cache module pulling sources */
  generateDocsIndex: (docs: Array<{ path: string, content: string }>) => string | null
}
