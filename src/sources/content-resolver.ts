/**
 * Content cascade: resolve in-memory docs for a package given its already-resolved URLs.
 *
 * Pure: no fs writes. The caller owns persistence; a single cache-write seam.
 *
 * Cascade order is defined by `defaultContentSteps` in
 * `./resolvers/content/index.ts`. Each step is a `StepResolver<ContentCtx>`
 * with its own `canResolve` predicate; see step files for the full ladder.
 */

import type { ContentCtx } from './resolvers/content/index.ts'
import type { ResolvedPackage } from './types.ts'
import { filterFrameworkDocs } from './github.ts'
import { walkSteps } from './resolvers/cascade.ts'
import { defaultContentSteps } from './resolvers/content/index.ts'

export interface CachedDoc {
  path: string
  content: string
}

export interface IndexDoc {
  id: string
  content: string
  metadata: Record<string, any>
}

export interface ResolvedContent {
  /** Docs to persist to the package cache (path is cache-relative) */
  docs: CachedDoc[]
  /** Docs to feed the embedding index (id may differ from cache path) */
  docsToIndex: IndexDoc[]
  /** Human-readable origin (URL or label) of the chosen primary source */
  docSource: string
  /** Which kind of source won */
  docsType: 'docs' | 'llms.txt' | 'readme'
  /** Non-fatal issues to surface to the user */
  warnings: string[]
}

export interface ResolveContentOptions {
  packageName: string
  resolved: ResolvedPackage
  version: string
  onProgress: (message: string) => void
}

export async function resolveContentDocs(opts: ResolveContentOptions): Promise<ResolvedContent> {
  const { packageName, resolved, version, onProgress } = opts

  const ctx: ContentCtx = {
    packageName,
    resolved,
    version,
    onProgress,
    isFrameworkDoc: path => filterFrameworkDocs([path], packageName).length > 0,
    docs: [],
    docsToIndex: [],
    warnings: [],
    docSource: resolved.readmeUrl || 'readme',
    docsType: 'readme',
  }

  await walkSteps(defaultContentSteps, ctx)

  return {
    docs: ctx.docs,
    docsToIndex: ctx.docsToIndex,
    docSource: ctx.docSource,
    docsType: ctx.docsType,
    warnings: ctx.warnings,
  }
}
