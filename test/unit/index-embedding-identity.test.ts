import { DatabaseSync } from 'node:sqlite'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'pathe'
import { afterEach, describe, expect, it } from 'vitest'
import { EmbeddingIndexMismatchError, getDb } from '../../src/retriv/index.ts'
import { checkIndexEmbeddingIdentity, recordIndexEmbeddingIdentity } from '../../src/retriv/index-embedding-identity.ts'

const dirs: string[] = []

function dbPath(): string {
  const dir = mkdtempSync(join(tmpdir(), 'skilld-index-identity-'))
  dirs.push(dir)
  return join(dir, 'search.db')
}

afterEach(() => {
  for (const dir of dirs.splice(0))
    rmSync(dir, { recursive: true, force: true })
})

describe('index embedding identity', () => {
  it('rejects an equal-width model change', () => {
    const path = dbPath()
    recordIndexEmbeddingIdentity(path, 'Xenova/bge-large-en-v1.5@auto')

    expect(checkIndexEmbeddingIdentity(path, 'Xenova/bge-m3@auto')).toEqual({
      _tag: 'Mismatch',
      current: 'Xenova/bge-m3@auto',
      stored: 'Xenova/bge-large-en-v1.5@auto',
    })
  })

  it('blocks opening an index with a different model', async () => {
    const path = dbPath()
    recordIndexEmbeddingIdentity(path, 'Xenova/bge-large-en-v1.5@auto')
    const originalModel = process.env.SKILLD_EMBED_MODEL
    const originalDevice = process.env.SKILLD_EMBED_DEVICE
    process.env.SKILLD_EMBED_MODEL = 'bge-m3'
    process.env.SKILLD_EMBED_DEVICE = 'auto'

    try {
      await expect(getDb({ dbPath: path })).rejects.toBeInstanceOf(EmbeddingIndexMismatchError)
    }
    finally {
      if (originalModel === undefined)
        delete process.env.SKILLD_EMBED_MODEL
      else
        process.env.SKILLD_EMBED_MODEL = originalModel
      if (originalDevice === undefined)
        delete process.env.SKILLD_EMBED_DEVICE
      else
        process.env.SKILLD_EMBED_DEVICE = originalDevice
    }
  })

  it('treats existing pre-identity indexes as the old default', () => {
    const path = dbPath()
    const db = new DatabaseSync(path)
    db.exec('CREATE TABLE documents_meta (id TEXT PRIMARY KEY)')
    db.prepare('INSERT INTO documents_meta (id) VALUES (?)').run('doc')
    db.close()

    expect(checkIndexEmbeddingIdentity(path, 'Xenova/bge-m3@auto')).toEqual({
      _tag: 'Mismatch',
      current: 'Xenova/bge-m3@auto',
      stored: 'Xenova/bge-small-en-v1.5@auto',
    })
  })

  it('accepts the identity recorded for a new index', () => {
    const path = dbPath()
    recordIndexEmbeddingIdentity(path, 'Xenova/bge-m3@webgpu')

    expect(checkIndexEmbeddingIdentity(path, 'Xenova/bge-m3@webgpu')).toEqual({
      _tag: 'Current',
    })
  })
})
