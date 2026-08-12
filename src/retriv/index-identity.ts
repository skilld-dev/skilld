import type { SkilldConfig } from '../core/config.ts'
import { createHash } from 'node:crypto'
import { existsSync, rmSync } from 'node:fs'
import { readConfig } from '../core/config.ts'
import { ollamaHost } from '../core/ollama-host.ts'
import { resolveEmbedDevice, resolveEmbedModel } from './models.ts'
import { isOllamaEmbedModel } from './ollama-embeddings.ts'

const META_TABLE = 'skilld_meta'
const IDENTITY_KEY = 'embedding_identity'
const IDENTITY_VERSION = 'v1'

export function resolveEmbeddingIdentity(
  config: Pick<SkilldConfig, 'embedModel' | 'embedDevice'> = readConfig(),
): string {
  const model = resolveEmbedModel(config.embedModel)
  if (!isOllamaEmbedModel(model))
    return `${IDENTITY_VERSION}:${model}@${resolveEmbedDevice(config.embedDevice) ?? 'auto'}`

  const host = createHash('sha256').update(ollamaHost()).digest('hex').slice(0, 16)
  return `${IDENTITY_VERSION}:${model}@host:${host}`
}

export function readIndexEmbeddingIdentity(dbPath: string): string | undefined {
  if (!existsSync(dbPath))
    return undefined
  const nodeSqlite = globalThis.process?.getBuiltinModule?.('node:sqlite') as typeof import('node:sqlite') | undefined
  if (!nodeSqlite)
    return undefined

  const db = new nodeSqlite.DatabaseSync(dbPath, { open: true, readOnly: true })
  try {
    const table = db.prepare("SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?").get(META_TABLE)
    if (!table)
      return undefined
    const row = db.prepare(`SELECT value FROM ${META_TABLE} WHERE key = ?`).get(IDENTITY_KEY) as { value: string } | undefined
    return row?.value
  }
  finally {
    db.close()
  }
}

export function hasIndexEmbeddingIdentity(dbPath: string, identity: string): boolean {
  return readIndexEmbeddingIdentity(dbPath) === identity
}

export function removeStaleIndex(dbPath: string, identity: string): boolean {
  if (!existsSync(dbPath) || hasIndexEmbeddingIdentity(dbPath, identity))
    return false
  for (const path of [dbPath, `${dbPath}-shm`, `${dbPath}-wal`])
    rmSync(path, { force: true })
  return true
}

export function writeIndexEmbeddingIdentity(dbPath: string, identity: string): void {
  const nodeSqlite = globalThis.process?.getBuiltinModule?.('node:sqlite') as typeof import('node:sqlite') | undefined
  if (!nodeSqlite)
    throw new Error('SQLite is unavailable, so the search index identity cannot be saved')
  const db = new nodeSqlite.DatabaseSync(dbPath)
  try {
    db.exec(`CREATE TABLE IF NOT EXISTS ${META_TABLE} (key TEXT PRIMARY KEY, value TEXT NOT NULL)`)
    db.prepare(`INSERT OR REPLACE INTO ${META_TABLE} (key, value) VALUES (?, ?)`).run(IDENTITY_KEY, identity)
  }
  finally {
    db.close()
  }
}
