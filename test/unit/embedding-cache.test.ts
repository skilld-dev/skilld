import { existsSync, mkdirSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'pathe'
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'

const TEST_DIR = join(tmpdir(), 'skilld-test-embedding-cache')

vi.mock('../../src/cache', () => ({
  CACHE_DIR: TEST_DIR,
}))

const { cachedEmbeddings, clearEmbeddingCache } = await import('../../src/retriv/embedding-cache')

function fakeEmbeddingConfig(dims = 4, embedder?: (texts: string[]) => Promise<Float32Array[]>) {
  const calls: string[][] = []
  const defaultEmbedder = async (texts: string[]) => {
    calls.push(texts)
    return texts.map(() => new Float32Array(dims).fill(1))
  }
  return {
    config: {
      resolve: async () => ({
        embedder: embedder ?? defaultEmbedder,
        dimensions: dims,
      }),
    },
    calls,
  }
}

describe('embedding-cache', () => {
  beforeAll(() => {
    mkdirSync(TEST_DIR, { recursive: true })
  })

  beforeEach(() => {
    clearEmbeddingCache()
  })

  afterAll(() => {
    rmSync(TEST_DIR, { recursive: true, force: true })
  })

  it('computes embeddings on first call (cache miss)', async () => {
    const { config, calls } = fakeEmbeddingConfig()
    const wrapped = await cachedEmbeddings(config, 'model-a@cpu')
    const { embedder } = await wrapped.resolve()

    const result = await embedder(['hello', 'world'])

    expect(result).toHaveLength(2)
    expect(calls).toHaveLength(1)
    expect(calls[0]).toEqual(['hello', 'world'])
  })

  it('serves cached embeddings on second call (cache hit)', async () => {
    const { config, calls } = fakeEmbeddingConfig()
    const wrapped = await cachedEmbeddings(config, 'model-a@cpu')
    const { embedder } = await wrapped.resolve()

    await embedder(['hello', 'world'])
    const result = await embedder(['hello', 'world'])

    expect(result).toHaveLength(2)
    // Only called real embedder once
    expect(calls).toHaveLength(1)
  })

  it('computes only missed texts on partial cache hit', async () => {
    const { config, calls } = fakeEmbeddingConfig()
    const wrapped = await cachedEmbeddings(config, 'model-a@cpu')
    const { embedder } = await wrapped.resolve()

    await embedder(['hello'])
    await embedder(['hello', 'world'])

    expect(calls).toHaveLength(2)
    expect(calls[0]).toEqual(['hello'])
    expect(calls[1]).toEqual(['world'])
  })

  it('returns correct embeddings for mixed hits/misses', async () => {
    let counter = 0
    const { config } = fakeEmbeddingConfig(2, async (texts) => {
      return texts.map(() => {
        counter++
        return new Float32Array([counter, counter * 10])
      })
    })
    const wrapped = await cachedEmbeddings(config, 'model-a@cpu')
    const { embedder } = await wrapped.resolve()

    await embedder(['a', 'b'])
    // 'a' → [1, 10], 'b' → [2, 20]

    const result = await embedder(['b', 'c', 'a'])
    // 'b' → cached [2, 20], 'c' → new [3, 30], 'a' → cached [1, 10]

    expect([...result[0] as Float32Array]).toEqual([2, 20])
    expect([...result[1] as Float32Array]).toEqual([3, 30])
    expect([...result[2] as Float32Array]).toEqual([1, 10])
  })

  it('does not serve cached vectors from another dimension', async () => {
    // First: populate with 4-dim embeddings
    const { config: config4, calls: calls4 } = fakeEmbeddingConfig(4)
    const wrapped4 = await cachedEmbeddings(config4, 'model-a@cpu')
    const { embedder: embedder4 } = await wrapped4.resolve()
    await embedder4(['hello'])
    expect(calls4).toHaveLength(1)

    // Second: resolve with 8-dim → should wipe, recompute
    const { config: config8, calls: calls8 } = fakeEmbeddingConfig(8)
    const wrapped8 = await cachedEmbeddings(config8, 'model-a@cpu')
    const { embedder: embedder8 } = await wrapped8.resolve()
    const result = await embedder8(['hello'])

    expect(calls8).toHaveLength(1)
    expect(calls8[0]).toEqual(['hello'])
    expect((result[0] as Float32Array).length).toBe(8)
  })

  it('does not serve cached vectors from another model', async () => {
    const { config, calls } = fakeEmbeddingConfig()
    const first = await cachedEmbeddings(config, 'model-a@cpu')
    await (await first.resolve()).embedder(['hello'])

    const second = await cachedEmbeddings(config, 'model-b@cpu')
    await (await second.resolve()).embedder(['hello'])

    expect(calls).toEqual([['hello'], ['hello']])
  })

  it('clearEmbeddingCache removes the db file', async () => {
    const { config } = fakeEmbeddingConfig()
    const wrapped = await cachedEmbeddings(config, 'model-a@cpu')
    const { embedder } = await wrapped.resolve()
    await embedder(['hello'])

    const dbPath = join(TEST_DIR, 'embeddings.db')
    expect(existsSync(dbPath)).toBe(true)

    clearEmbeddingCache()
    expect(existsSync(dbPath)).toBe(false)
  })

  it('persists cache across resolve() calls', async () => {
    const { config, calls } = fakeEmbeddingConfig()

    // First resolve + embed
    const wrapped1 = await cachedEmbeddings(config, 'model-a@cpu')
    const { embedder: e1 } = await wrapped1.resolve()
    await e1(['hello'])

    // Second resolve (simulates new process opening same DB)
    const wrapped2 = await cachedEmbeddings(config, 'model-a@cpu')
    const { embedder: e2 } = await wrapped2.resolve()
    await e2(['hello'])

    // Real embedder only called once across both resolves
    expect(calls).toHaveLength(1)
  })

  it('handles empty input', async () => {
    const { config, calls } = fakeEmbeddingConfig()
    const wrapped = await cachedEmbeddings(config, 'model-a@cpu')
    const { embedder } = await wrapped.resolve()

    const result = await embedder([])
    expect(result).toHaveLength(0)
    expect(calls).toHaveLength(0)
  })
})
