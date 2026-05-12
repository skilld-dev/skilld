/**
 * Search-index pipeline: builds and incrementally updates the per-package
 * sqlite-vec database.
 *
 * Wraps `createIndex` / `listIndexIds` with the higher-level concerns of
 * adding `pkg/` entry files, capping doc count, computing diffs against the
 * existing index, and gracefully degrading when native deps are unavailable.
 */

import type { FeaturesConfig } from '../core/config.ts'
import type { IndexDoc } from '../sources/content-resolver.ts'
import { existsSync } from 'node:fs'
import { getPackageDbPath } from '../cache/index.ts'
import { defaultFeatures, readConfig } from '../core/config.ts'
import { resolvePkgDir } from '../core/prepare.ts'
import { resolveEntryFiles } from '../sources/index.ts'
import { createIndex, listIndexIds, SearchDepsUnavailableError } from './index.ts'

/** Max docs sent to the embedding pipeline to prevent oversized indexes */
export const MAX_INDEX_DOCS = 250

/**
 * Extract the parent document ID from a chunk ID.
 * Chunk IDs have the form "docId#chunk-N"; non-chunk IDs return as-is.
 */
function parentDocId(id: string): string {
  const idx = id.indexOf('#chunk-')
  return idx === -1 ? id : id.slice(0, idx)
}

/** Cap and sort docs by type priority, mutates and truncates allDocs in place */
function capDocs(allDocs: IndexDoc[], max: number, onProgress: (msg: string) => void): void {
  if (allDocs.length <= max)
    return
  const TYPE_PRIORITY: Record<string, number> = { doc: 0, issue: 1, discussion: 2, release: 3, source: 4, types: 5 }
  allDocs.sort((a, b) => {
    const ta = TYPE_PRIORITY[a.metadata?.type || 'doc'] ?? 3
    const tb = TYPE_PRIORITY[b.metadata?.type || 'doc'] ?? 3
    if (ta !== tb)
      return ta - tb
    return a.id.localeCompare(b.id)
  })
  onProgress(`Indexing capped at ${max}/${allDocs.length} docs (prioritized by type)`)
  allDocs.length = max
}

export interface IndexResourcesOptions {
  packageName: string
  version: string
  cwd: string
  docsToIndex: IndexDoc[]
  features?: FeaturesConfig
  onProgress: (message: string) => void
}

/** Index all resources into the search database, with incremental support */
export async function indexResources(opts: IndexResourcesOptions): Promise<void> {
  const { packageName, version, cwd, onProgress } = opts
  const features = opts.features ?? readConfig().features ?? defaultFeatures

  if (!features.search)
    return

  const dbPath = getPackageDbPath(packageName, version)
  const dbExists = existsSync(dbPath)

  const allDocs = [...opts.docsToIndex]

  // Add entry files
  const pkgDir = resolvePkgDir(packageName, cwd, version)
  if (features.search && pkgDir) {
    onProgress('Scanning exports')
    const entryFiles = await resolveEntryFiles(pkgDir)
    for (const e of entryFiles) {
      allDocs.push({
        id: e.path,
        content: e.content,
        metadata: { package: packageName, source: `pkg/${e.path}`, type: e.type },
      })
    }
  }

  if (allDocs.length === 0)
    return

  capDocs(allDocs, MAX_INDEX_DOCS, onProgress)

  // Full build when no existing DB
  if (!dbExists) {
    onProgress(`Building search index (${allDocs.length} docs)`)
    try {
      await createIndex(allDocs, {
        dbPath,
        onProgress: ({ phase, current, total }) => {
          if (phase === 'storing') {
            const d = allDocs[current - 1]
            const type = d?.metadata?.type === 'source' || d?.metadata?.type === 'types' ? 'code' : (d?.metadata?.type || 'doc')
            onProgress(`Storing ${type} (${current}/${total})`)
          }
          else if (phase === 'embedding') {
            onProgress(`Creating embeddings (${current}/${total})`)
          }
        },
      })
    }
    catch (err) {
      if (err instanceof SearchDepsUnavailableError)
        onProgress('Search indexing skipped (native deps unavailable)')
      else
        throw err
    }
    return
  }

  // Incremental update: diff incoming docs against existing index
  let existingIds: string[]
  try {
    existingIds = await listIndexIds({ dbPath })
  }
  catch (err) {
    if (err instanceof SearchDepsUnavailableError) {
      onProgress('Search indexing skipped (native deps unavailable)')
      return
    }
    throw err
  }

  const existingParentIds = new Set(existingIds.map(parentDocId))
  const incomingIds = new Set(allDocs.map(d => d.id))

  const newDocs = allDocs.filter(d => !existingParentIds.has(d.id))
  const removeIds = existingIds.filter(id => !incomingIds.has(parentDocId(id)))

  if (newDocs.length === 0 && removeIds.length === 0) {
    onProgress('Search index up to date')
    return
  }

  const parts: string[] = []
  if (newDocs.length > 0)
    parts.push(`+${newDocs.length} new`)
  if (removeIds.length > 0)
    parts.push(`-${removeIds.length} stale`)
  onProgress(`Updating search index (${parts.join(', ')})`)

  try {
    await createIndex(newDocs, {
      dbPath,
      removeIds,
      onProgress: ({ phase, current, total }) => {
        if (phase === 'storing') {
          const d = newDocs[current - 1]
          const type = d?.metadata?.type === 'source' || d?.metadata?.type === 'types' ? 'code' : (d?.metadata?.type || 'doc')
          onProgress(`Storing ${type} (${current}/${total})`)
        }
        else if (phase === 'embedding') {
          onProgress(`Creating embeddings (${current}/${total})`)
        }
      },
    })
  }
  catch (err) {
    if (err instanceof SearchDepsUnavailableError)
      onProgress('Search indexing skipped (native deps unavailable)')
    else
      throw err
  }
}
