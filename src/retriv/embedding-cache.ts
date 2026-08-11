import type { DatabaseSync } from 'node:sqlite'
import type { Embedding } from 'retriv'
import { rmSync } from 'node:fs'
import { join } from 'pathe'
import { CACHE_DIR } from '../cache/index.ts'

interface EmbeddingConfig {
  resolve: () => Promise<{ embedder: (texts: string[]) => Promise<Embedding[]>, dimensions: number, maxTokens?: number }>
}

const EMBEDDINGS_DB_PATH = join(CACHE_DIR, 'embeddings.db')

let _db: DatabaseSync | null = null

async function openDb(): Promise<DatabaseSync> {
  if (_db)
    return _db
  const { DatabaseSync: DB } = await import('node:sqlite')
  const db = new DB(EMBEDDINGS_DB_PATH)
  db.exec('PRAGMA journal_mode=WAL')
  db.exec('PRAGMA busy_timeout=5000')
  db.exec(`CREATE TABLE IF NOT EXISTS embeddings (text_hash TEXT PRIMARY KEY, embedding BLOB NOT NULL)`)
  db.exec(`CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)`)
  _db = db
  return db
}

function closeDb(): void {
  if (_db) {
    _db.close()
    _db = null
  }
}

function createSqliteStorage(db: DatabaseSync) {
  const getStmt = db.prepare('SELECT embedding FROM embeddings WHERE text_hash = ?')
  const setStmt = db.prepare('INSERT OR IGNORE INTO embeddings (text_hash, embedding) VALUES (?, ?)')

  return {
    get: (hash: string): Embedding | null => {
      const row = getStmt.get(hash) as { embedding: Buffer } | undefined
      if (!row)
        return null
      return new Float32Array(row.embedding.buffer, row.embedding.byteOffset, row.embedding.byteLength / 4)
    },
    set: (hash: string, embedding: Embedding): void => {
      const arr = embedding instanceof Float32Array ? embedding : new Float32Array(embedding)
      setStmt.run(hash, Buffer.from(arr.buffer, arr.byteOffset, arr.byteLength))
    },
  }
}

/**
 * Wrap an embedding provider with the on-disk vector cache.
 *
 * `model` identifies which embedder produced the cached vectors. Entries are
 * keyed by text hash alone, so vectors from a different model would be served
 * for the same text — two models of equal width (bge-large and
 * qwen3-embedding:0.6b are both 1024d) would silently mix embedding spaces and
 * destroy ranking. Dimensions alone cannot catch that; the model id can.
 */
export async function cachedEmbeddings(config: EmbeddingConfig, model?: string): Promise<EmbeddingConfig> {
  const { cachedEmbeddings: retrivCached } = await import('retriv/embeddings/cached')
  const db = await openDb()
  const storage = createSqliteStorage(db)

  const originalResolve = config.resolve
  const validatedConfig: EmbeddingConfig = {
    async resolve() {
      const resolved = await originalResolve()
      const getMetaStmt = db.prepare('SELECT value FROM meta WHERE key = ?')
      const setMetaStmt = db.prepare('INSERT OR REPLACE INTO meta (key, value) VALUES (?, ?)')

      const storedDims = getMetaStmt.get('dimensions') as { value: string } | undefined
      const storedModel = getMetaStmt.get('model') as { value: string } | undefined
      const dimsChanged = storedDims && Number(storedDims.value) !== resolved.dimensions
      // A cache written before this key existed has unknown provenance, so
      // treat a missing stored model as a mismatch once a model is supplied.
      const modelChanged = model !== undefined && storedModel?.value !== model

      if (dimsChanged || modelChanged)
        db.exec('DELETE FROM embeddings')

      setMetaStmt.run('dimensions', String(resolved.dimensions))
      if (model !== undefined)
        setMetaStmt.run('model', model)

      return resolved
    },
  }

  return retrivCached(validatedConfig, { storage })
}

export function clearEmbeddingCache(): void {
  closeDb()
  rmSync(EMBEDDINGS_DB_PATH, { force: true })
}
