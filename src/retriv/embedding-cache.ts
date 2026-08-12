import type { DatabaseSync } from 'node:sqlite'
import type { Embedding } from 'retriv'
import { createHash } from 'node:crypto'
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
  _db = db
  return db
}

function closeDb(): void {
  if (_db) {
    _db.close()
    _db = null
  }
}

function createSqliteStorage(db: DatabaseSync, getNamespace: () => string) {
  const getStmt = db.prepare('SELECT embedding FROM embeddings WHERE text_hash = ?')
  const setStmt = db.prepare('INSERT OR IGNORE INTO embeddings (text_hash, embedding) VALUES (?, ?)')
  const key = (hash: string) => `${getNamespace()}:${hash}`

  return {
    get: (hash: string): Embedding | null => {
      const row = getStmt.get(key(hash)) as { embedding: Buffer } | undefined
      if (!row)
        return null
      return new Float32Array(row.embedding.buffer, row.embedding.byteOffset, row.embedding.byteLength / 4)
    },
    set: (hash: string, embedding: Embedding): void => {
      const arr = embedding instanceof Float32Array ? embedding : new Float32Array(embedding)
      setStmt.run(key(hash), Buffer.from(arr.buffer, arr.byteOffset, arr.byteLength))
    },
  }
}

/**
 * Wrap an embedding provider with the on-disk vector cache.
 *
 * `identity` identifies the model and execution backend. It namespaces every
 * text hash, so concurrent providers cannot read or overwrite each other's
 * vectors. Dimensions join the namespace after the provider resolves.
 */
export async function cachedEmbeddings(config: EmbeddingConfig, identity: string): Promise<EmbeddingConfig> {
  const { cachedEmbeddings: retrivCached } = await import('retriv/embeddings/cached')
  const db = await openDb()
  let namespace: string | undefined
  const storage = createSqliteStorage(db, () => {
    if (!namespace)
      throw new Error('Embedding cache used before its provider resolved')
    return namespace
  })

  const originalResolve = config.resolve
  const namespacedConfig: EmbeddingConfig = {
    async resolve() {
      const resolved = await originalResolve()
      namespace = createHash('sha256')
        .update(`${identity}\0${resolved.dimensions}`)
        .digest('hex')
      return resolved
    },
  }

  return retrivCached(namespacedConfig, { storage })
}

export function clearEmbeddingCache(): void {
  closeDb()
  rmSync(EMBEDDINGS_DB_PATH, { force: true })
}
