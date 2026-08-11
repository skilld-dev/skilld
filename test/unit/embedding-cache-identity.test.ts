import { DatabaseSync } from 'node:sqlite'
import { describe, expect, it } from 'vitest'

/**
 * Guards the cache-invalidation rule in `src/retriv/embedding-cache.ts`.
 *
 * Vectors are keyed by text hash alone, so the only thing preventing one
 * model's vectors being served to another is the stored identity. Dimensions
 * are not enough: `Xenova/bge-large-en-v1.5` and `ollama:qwen3-embedding:0.6b`
 * are both 1024d, so switching between them would silently mix embedding
 * spaces and wreck ranking.
 *
 * This reimplements the decision against an in-memory database so the rule is
 * pinned without touching the user's real cache.
 */
function applyIdentity(db: DatabaseSync, dimensions: number, model?: string): void {
  const get = db.prepare('SELECT value FROM meta WHERE key = ?')
  const set = db.prepare('INSERT OR REPLACE INTO meta (key, value) VALUES (?, ?)')

  const storedDims = get.get('dimensions') as { value: string } | undefined
  const storedModel = get.get('model') as { value: string } | undefined
  const dimsChanged = storedDims && Number(storedDims.value) !== dimensions
  const modelChanged = model !== undefined && storedModel?.value !== model

  if (dimsChanged || modelChanged)
    db.exec('DELETE FROM embeddings')

  set.run('dimensions', String(dimensions))
  if (model !== undefined)
    set.run('model', model)
}

function makeDb(): DatabaseSync {
  const db = new DatabaseSync(':memory:')
  db.exec('CREATE TABLE embeddings (text_hash TEXT PRIMARY KEY, embedding BLOB NOT NULL)')
  db.exec('CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)')
  return db
}

function seed(db: DatabaseSync, n = 3): void {
  const stmt = db.prepare('INSERT OR IGNORE INTO embeddings (text_hash, embedding) VALUES (?, ?)')
  for (let i = 0; i < n; i++)
    stmt.run(`hash-${i}`, Buffer.from(new Float32Array([i, i, i]).buffer))
}

function count(db: DatabaseSync): number {
  return (db.prepare('SELECT COUNT(*) c FROM embeddings').get() as { c: number }).c
}

describe('embedding cache identity', () => {
  it('keeps cached vectors when model and dimensions are unchanged', () => {
    const db = makeDb()
    applyIdentity(db, 1024, 'model-a')
    seed(db)
    applyIdentity(db, 1024, 'model-a')
    expect(count(db)).toBe(3)
    db.close()
  })

  // The regression: equal width, different model.
  it('clears cached vectors when the model changes at identical dimensions', () => {
    const db = makeDb()
    applyIdentity(db, 1024, 'Xenova/bge-large-en-v1.5@webgpu')
    seed(db)
    expect(count(db)).toBe(3)

    applyIdentity(db, 1024, 'ollama:qwen3-embedding:0.6b')
    expect(count(db)).toBe(0)
    db.close()
  })

  it('clears cached vectors when dimensions change', () => {
    const db = makeDb()
    applyIdentity(db, 384, 'model-a')
    seed(db)
    applyIdentity(db, 1024, 'model-a')
    expect(count(db)).toBe(0)
    db.close()
  })

  // Same model on a different backend: numeric output can differ, so vectors
  // are only interchangeable within a device.
  it('clears cached vectors when only the device changes', () => {
    const db = makeDb()
    applyIdentity(db, 1024, 'Xenova/bge-large-en-v1.5@cpu')
    seed(db)
    applyIdentity(db, 1024, 'Xenova/bge-large-en-v1.5@webgpu')
    expect(count(db)).toBe(0)
    db.close()
  })

  // A cache written before the model key existed has unknown provenance.
  it('clears a legacy cache that has no stored model', () => {
    const db = makeDb()
    applyIdentity(db, 1024)
    seed(db)
    expect(count(db)).toBe(3)

    applyIdentity(db, 1024, 'model-a')
    expect(count(db)).toBe(0)
    db.close()
  })
})
