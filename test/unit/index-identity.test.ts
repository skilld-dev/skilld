import { mkdirSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { DatabaseSync } from 'node:sqlite'
import { join } from 'pathe'
import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  hasIndexEmbeddingIdentity,
  readIndexEmbeddingIdentity,
  removeStaleIndex,
  resolveEmbeddingIdentity,
  writeIndexEmbeddingIdentity,
} from '../../src/retriv/index-identity.ts'

const TEST_DIR = join(tmpdir(), 'skilld-test-index-identity')
const DB_PATH = join(TEST_DIR, 'search.db')

afterEach(() => {
  vi.unstubAllEnvs()
  rmSync(TEST_DIR, { recursive: true, force: true })
})

function createIndexFile(): void {
  mkdirSync(TEST_DIR, { recursive: true })
  new DatabaseSync(DB_PATH).close()
}

describe('search index embedding identity', () => {
  it('resolves transformer presets before identifying the provider', () => {
    vi.stubEnv('SKILLD_EMBED_MODEL', '')
    vi.stubEnv('SKILLD_EMBED_DEVICE', '')
    expect(resolveEmbeddingIdentity({
      embedModel: 'bge-small-en-v1.5',
      embedDevice: 'webgpu',
    })).toBe('v1:Xenova/bge-small-en-v1.5@webgpu')
  })

  it('persists the model identity in the search database', () => {
    createIndexFile()
    writeIndexEmbeddingIdentity(DB_PATH, 'model-a@cpu')

    expect(readIndexEmbeddingIdentity(DB_PATH)).toBe('model-a@cpu')
    expect(hasIndexEmbeddingIdentity(DB_PATH, 'model-a@cpu')).toBe(true)
  })

  it('removes an index built with different embedding settings', () => {
    createIndexFile()
    writeIndexEmbeddingIdentity(DB_PATH, 'model-a@cpu')

    expect(removeStaleIndex(DB_PATH, 'model-b@cpu')).toBe(true)
    expect(readIndexEmbeddingIdentity(DB_PATH)).toBeUndefined()
  })
})
