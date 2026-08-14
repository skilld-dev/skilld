import type { ChunkEntity, Document, IndexConfig, IndexPhase, IndexProgress, SearchFilter, SearchOptions, SearchResult, SearchSnippet } from './types.ts'
import { existsSync } from 'node:fs'
import { readConfig } from '../core/config.ts'
import { stripFrontmatter } from '../core/markdown.ts'
import { hasIndexEmbeddingIdentity, removeStaleIndex, resolveEmbeddingIdentity } from './index-identity.ts'
import { resolveEmbedDevice, resolveEmbedModel } from './models.ts'
import { isOllamaEmbedModel, ollamaEmbeddings } from './ollama-embeddings.ts'

export type { ChunkEntity, Document, IndexConfig, IndexPhase, IndexProgress, SearchFilter, SearchOptions, SearchResult, SearchSnippet }

type RetrivInstance = Awaited<ReturnType<typeof getDb>>

export class SearchDepsUnavailableError extends Error {
  constructor(cause: unknown, message?: string) {
    super(message ?? 'Search dependencies unavailable (sqlite-vec or retriv not installed). Search indexing skipped.')
    this.name = 'SearchDepsUnavailableError'
    this.cause = cause
  }
}

let _fts5Available: boolean | null = null

/**
 * Probe whether SQLite FTS5 module is available.
 * Windows Node.js binaries often ship without FTS5 compiled in.
 */
function checkFts5(): boolean {
  if (_fts5Available !== null)
    return _fts5Available
  const nodeSqlite = globalThis.process?.getBuiltinModule?.('node:sqlite') as typeof import('node:sqlite') | undefined
  if (!nodeSqlite) {
    _fts5Available = false
    return false
  }
  const db = new nodeSqlite.DatabaseSync(':memory:')
  try {
    db.exec('CREATE VIRTUAL TABLE _fts5_probe USING fts5(content)')
    db.exec('DROP TABLE _fts5_probe')
    _fts5Available = true
  }
  catch {
    _fts5Available = false
  }
  finally {
    db.close()
  }
  return _fts5Available
}

// Dynamic imports: retriv/chunkers/auto eagerly loads typescript which may not be installed (e.g. npx)
async function openDb(config: Pick<IndexConfig, 'dbPath'>, identity: string) {
  if (!checkFts5())
    throw new SearchDepsUnavailableError(new Error('FTS5 module not available'), 'SQLite FTS5 module not available. Search indexing skipped. On Windows, run from WSL where FTS5 is included.')

  let createRetriv, autoChunker, sqliteMod, sqliteVec, transformersJs, cachedEmbeddings
  try {
    ;([
      { createRetriv },
      { autoChunker },
      sqliteMod,
      sqliteVec,
      { transformersJs },
      { cachedEmbeddings },
    ] = await Promise.all([
      import('retriv'),
      import('retriv/chunkers/auto'),
      import('retriv/db/sqlite'),
      import('sqlite-vec'),
      import('retriv/embeddings/transformers-js'),
      import('./embedding-cache.ts'),
    ]))
  }
  catch (err: any) {
    if (err?.code === 'ERR_MODULE_NOT_FOUND')
      throw new SearchDepsUnavailableError(err)
    throw err
  }
  const userConfig = readConfig()
  const embedModel = resolveEmbedModel(userConfig.embedModel)
  const device = resolveEmbedDevice(userConfig.embedDevice)
  // Cache identity pairs model with device: cached vectors are only valid for
  // the embedder that produced them, and backends can differ numerically.
  // Ollama runs the model in its own process, so `device` does not apply there.
  const isOllama = isOllamaEmbedModel(embedModel)
  const embeddings = await cachedEmbeddings(
    isOllama
      ? ollamaEmbeddings(embedModel)
      : transformersJs({
          model: embedModel,
          // Omitted when `auto` so transformers.js keeps its own device resolution.
          ...(device ? { device } : {}),
        }),
    identity,
  )
  return createRetriv({
    driver: sqliteMod.default({
      path: config.dbPath,
      embeddings,
      sqliteVec,
    }),
    chunking: autoChunker(),
  })
}

export async function getDb(config: Pick<IndexConfig, 'dbPath'>) {
  const identity = resolveEmbeddingIdentity()
  if (existsSync(config.dbPath) && !hasIndexEmbeddingIdentity(config.dbPath, identity)) {
    throw new Error(
      'Search index uses different embedding settings. Run `skilld update` to rebuild it.',
    )
  }
  return openDb(config, identity)
}

export async function getIndexDb(
  config: Pick<IndexConfig, 'dbPath'>,
  identity: string = resolveEmbeddingIdentity(),
) {
  removeStaleIndex(config.dbPath, identity)
  return openDb(config, identity)
}

/**
 * Index documents in a background worker thread.
 * Falls back to direct indexing if worker fails to spawn.
 */
export async function createIndex(
  documents: Document[],
  config: IndexConfig & { removeIds?: string[] },
): Promise<void> {
  // Dynamic import justified: search/searchSnippets shouldn't pull in worker_threads
  const { createIndexInWorker } = await import('./pool.ts')
  return createIndexInWorker(documents, config)
}

/**
 * List all raw document IDs in an existing index.
 * Returns chunk IDs (e.g. "doc-id#chunk-0") for chunked docs.
 * Queries sqlite directly to bypass createRetriv's parent-ID deduplication,
 * so callers can use these IDs for exact removal and parent-ID grouping.
 */
export async function listIndexIds(
  config: Pick<IndexConfig, 'dbPath'>,
): Promise<string[]> {
  const nodeSqlite = globalThis.process?.getBuiltinModule?.('node:sqlite') as typeof import('node:sqlite') | undefined
  if (!nodeSqlite)
    return []
  const db = new nodeSqlite.DatabaseSync(config.dbPath, { open: true, readOnly: true })
  try {
    const rows = db.prepare('SELECT id FROM documents_meta').all() as Array<{ id: string }>
    return rows.map(r => r.id)
  }
  finally {
    db.close()
  }
}

export async function search(
  query: string,
  config: IndexConfig,
  options: SearchOptions = {},
): Promise<SearchResult[]> {
  const { limit = 10, filter } = options
  const db = await getDb(config)
  const results = await db.search(query, { limit, filter, returnContent: true, returnMetadata: true, returnMeta: true })
  await db.close?.()

  return results.map(r => ({
    id: r.id,
    content: r.content ?? '',
    score: r.score,
    metadata: r.metadata ?? {},
    highlights: r._meta?.highlights ?? [],
    lineRange: r._chunk?.lineRange,
    entities: r._chunk?.entities,
    scope: r._chunk?.scope,
  }))
}

/**
 * Search and return formatted snippets
 */
export async function searchSnippets(
  query: string,
  config: IndexConfig,
  options: SearchOptions = {},
): Promise<SearchSnippet[]> {
  const results = await search(query, config, options)
  return toSnippets(results)
}

function toSnippets(results: SearchResult[]): SearchSnippet[] {
  return results.map((r) => {
    const content = stripFrontmatter(r.content)
    const source = r.metadata.source || r.id
    const lines = content.split('\n').length

    return {
      package: r.metadata.package || 'unknown',
      source,
      referenceRoot: typeof r.metadata.referenceRoot === 'string' ? r.metadata.referenceRoot : undefined,
      lineStart: r.lineRange?.[0] ?? 1,
      lineEnd: r.lineRange?.[1] ?? lines,
      content,
      score: r.score,
      highlights: r.highlights,
      entities: r.entities,
      scope: r.scope,
    }
  })
}

// ── Pooled DB access for interactive search ──

export async function openPool(dbPaths: string[]): Promise<Map<string, RetrivInstance>> {
  const pool = new Map<string, RetrivInstance>()
  await Promise.all(dbPaths.map(async (dbPath) => {
    const db = await getDb({ dbPath })
    pool.set(dbPath, db)
  }))
  return pool
}

export async function searchPooled(
  query: string,
  pool: Map<string, RetrivInstance>,
  options: SearchOptions = {},
): Promise<SearchSnippet[]> {
  const { limit = 10, filter } = options
  const fetchLimit = limit * 2 // Over-fetch to compensate for dedup
  const allResults = await Promise.all(
    Array.from(pool.values(), async (db) => {
      const results = await db.search(query, { limit: fetchLimit, filter, returnContent: true, returnMetadata: true, returnMeta: true })
      return results.map(r => ({
        id: r.id,
        content: r.content ?? '',
        score: r.score,
        metadata: r.metadata ?? {},
        highlights: r._meta?.highlights ?? [],
        lineRange: r._chunk?.lineRange as [number, number] | undefined,
        entities: r._chunk?.entities,
        scope: r._chunk?.scope,
      }))
    }),
  )
  // Deduplicate by source+lineRange (overlapping chunks from same doc)
  const seen = new Set<string>()
  const merged = allResults.flat()
    .sort((a, b) => b.score - a.score)
    .filter((r) => {
      const lr = r.lineRange
      const key = `${r.metadata.source || r.id}:${lr?.[0]}-${lr?.[1]}`
      if (seen.has(key))
        return false
      seen.add(key)
      return true
    })
    .slice(0, limit)
  return toSnippets(merged)
}

export async function closePool(pool: Map<string, RetrivInstance>): Promise<void> {
  await Promise.all(Array.from(pool.values(), db => db.close?.()))
  pool.clear()
}
