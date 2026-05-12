/**
 * Shared types for the content resolution cascade.
 * Each step mutates the same `ContentCtx`.
 */

import type { CachedDoc, IndexDoc, ResolvedContent } from '../../content-resolver.ts'
import type { ResolvedPackage } from '../../types.ts'

export interface ContentCtx {
  packageName: string
  resolved: ResolvedPackage
  version: string
  onProgress: (message: string) => void
  /** Source-path filter: drop docs that don't look like framework-relevant content. */
  isFrameworkDoc: (path: string) => boolean
  /** Accumulator: docs to persist (cache-relative paths). */
  docs: CachedDoc[]
  /** Accumulator: docs to feed the embedding index. */
  docsToIndex: IndexDoc[]
  /** Accumulator: non-fatal issues to surface to the user. */
  warnings: string[]
  /** Human-readable origin of the chosen primary source. */
  docSource: string
  /** Which kind of source has won so far. */
  docsType: 'docs' | 'llms.txt' | 'readme'
}

export type ContentResult = ResolvedContent
