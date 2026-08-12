import { DatabaseSync } from 'node:sqlite'
import { existsSync } from 'node:fs'
import { DEFAULT_EMBEDDING_IDENTITY } from './models.ts'

const META_TABLE = 'skilld_meta'
const IDENTITY_KEY = 'embedding_identity'

export type IndexEmbeddingIdentityState
  = | { _tag: 'Current' }
    | { _tag: 'Missing' }
    | { _tag: 'Mismatch', current: string, stored: string }

function tableExists(db: DatabaseSync, name: string): boolean {
  return db.prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?").get(name) !== undefined
}

function hasIndexedDocuments(db: DatabaseSync): boolean {
  if (!tableExists(db, 'documents_meta'))
    return false
  const row = db.prepare('SELECT EXISTS(SELECT 1 FROM documents_meta) AS found').get() as { found: number }
  return row.found === 1
}

export function checkIndexEmbeddingIdentity(dbPath: string, current: string): IndexEmbeddingIdentityState {
  if (dbPath === ':memory:' || !existsSync(dbPath))
    return { _tag: 'Missing' }

  const db = new DatabaseSync(dbPath, { open: true, readOnly: true })
  try {
    const row = tableExists(db, META_TABLE)
      ? db.prepare(`SELECT value FROM ${META_TABLE} WHERE key = ?`).get(IDENTITY_KEY) as { value: string } | undefined
      : undefined
    const stored = row?.value ?? (hasIndexedDocuments(db) ? DEFAULT_EMBEDDING_IDENTITY : undefined)

    if (!stored)
      return { _tag: 'Missing' }
    if (stored !== current)
      return { _tag: 'Mismatch', current, stored }
    return { _tag: 'Current' }
  }
  finally {
    db.close()
  }
}

export function recordIndexEmbeddingIdentity(dbPath: string, identity: string): void {
  if (dbPath === ':memory:')
    return
  const db = new DatabaseSync(dbPath)
  try {
    db.exec(`CREATE TABLE IF NOT EXISTS ${META_TABLE} (key TEXT PRIMARY KEY, value TEXT NOT NULL)`)
    db.prepare(`INSERT OR REPLACE INTO ${META_TABLE} (key, value) VALUES (?, ?)`).run(IDENTITY_KEY, identity)
  }
  finally {
    db.close()
  }
}
