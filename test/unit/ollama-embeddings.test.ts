import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  getAvailableOllamaEmbedModels,
  isOllamaEmbedModel,
  ollamaEmbeddings,
  stripOllamaPrefix,
} from '../../src/retriv/ollama-embeddings.ts'

const SHOW_EMBEDDING = {
  capabilities: ['embedding'],
  model_info: { 'qwen3.embedding_length': 1024, 'qwen3.context_length': 32768 },
}

function jsonResponse(body: unknown, ok = true, status = 200) {
  return {
    ok,
    status,
    json: async () => body,
  } as unknown as Response
}

/** Route mocked fetch by URL path so tests read as request/response pairs. */
function mockOllama(routes: Record<string, () => Response | Promise<Response>>) {
  return vi.fn(async (url: string | URL) => {
    const path = new URL(String(url)).pathname
    const handler = routes[path]
    if (!handler)
      throw new Error(`unexpected request to ${path}`)
    return handler()
  })
}

describe('ollama embed model ids', () => {
  it('detects the prefix', () => {
    expect(isOllamaEmbedModel('ollama:nomic-embed-text')).toBe(true)
    expect(isOllamaEmbedModel('bge-small-en-v1.5')).toBe(false)
  })

  // Model names contain colons (`qwen3-embedding:0.6b`), so only the leading
  // prefix may be stripped.
  it('strips only the leading prefix', () => {
    expect(stripOllamaPrefix('ollama:qwen3-embedding:0.6b')).toBe('qwen3-embedding:0.6b')
    expect(stripOllamaPrefix('qwen3-embedding:0.6b')).toBe('qwen3-embedding:0.6b')
  })
})

describe('ollamaEmbeddings', () => {
  const original = globalThis.fetch

  afterEach(() => {
    globalThis.fetch = original
    vi.restoreAllMocks()
  })

  it('reads dimensions and max tokens from /api/show without a probe embed', async () => {
    const fetchMock = mockOllama({ '/api/show': () => jsonResponse(SHOW_EMBEDDING) })
    globalThis.fetch = fetchMock as unknown as typeof fetch

    const resolved = await ollamaEmbeddings('ollama:qwen3-embedding:0.6b').resolve()

    expect(resolved.dimensions).toBe(1024)
    expect(resolved.maxTokens).toBe(32768)
    expect(fetchMock).toHaveBeenCalledTimes(1)
  })

  it('falls back to a probe embed when the model reports no embedding_length', async () => {
    globalThis.fetch = mockOllama({
      '/api/show': () => jsonResponse({ capabilities: ['embedding'], model_info: {} }),
      '/api/embed': () => jsonResponse({ embeddings: [[0, 1, 0]] }),
    }) as unknown as typeof fetch

    const resolved = await ollamaEmbeddings('ollama:mystery-model').resolve()
    expect(resolved.dimensions).toBe(3)
  })

  it('normalizes returned vectors to unit length', async () => {
    globalThis.fetch = mockOllama({
      '/api/show': () => jsonResponse({
        capabilities: ['embedding'],
        model_info: { 'test.embedding_length': 2 },
      }),
      // Deliberately unnormalized: the index scores by L2 distance and assumes
      // unit vectors, so magnitude must not leak into ranking.
      '/api/embed': () => jsonResponse({ embeddings: [[3, 4], [0, 10]] }),
    }) as unknown as typeof fetch

    const { embedder } = await ollamaEmbeddings('ollama:x').resolve()
    const [a, b] = await embedder(['one', 'two'])

    const norm = (v: ArrayLike<number>) => Math.sqrt(Array.from(v).reduce((s, x) => s + x * x, 0))
    expect(norm(a!)).toBeCloseTo(1, 6)
    expect(norm(b!)).toBeCloseTo(1, 6)
    // [3, 4] has magnitude 5, so it normalizes to [0.6, 0.8].
    expect(a![0]).toBeCloseTo(0.6, 6)
    expect(a![1]).toBeCloseTo(0.8, 6)
  })

  it('returns an empty array without calling the API for no input', async () => {
    const fetchMock = mockOllama({ '/api/show': () => jsonResponse(SHOW_EMBEDDING) })
    globalThis.fetch = fetchMock as unknown as typeof fetch

    const { embedder } = await ollamaEmbeddings('ollama:x').resolve()
    expect(await embedder([])).toEqual([])
    expect(fetchMock).toHaveBeenCalledTimes(1)
  })

  it('explains how to start Ollama when the daemon is unreachable', async () => {
    globalThis.fetch = vi.fn(async () => {
      throw new Error('ECONNREFUSED')
    }) as unknown as typeof fetch

    await expect(ollamaEmbeddings('ollama:x').resolve()).rejects.toThrow(/not reachable/i)
  })

  it('rejects a model that does not support embeddings', async () => {
    globalThis.fetch = mockOllama({
      '/api/show': () => jsonResponse({ capabilities: ['completion'], model_info: {} }),
    }) as unknown as typeof fetch

    await expect(ollamaEmbeddings('ollama:llama3').resolve()).rejects.toThrow(/does not support embeddings/i)
  })

  it('rejects a model whose embedding capability is missing', async () => {
    globalThis.fetch = mockOllama({
      '/api/show': () => jsonResponse({ model_info: { 'llama.embedding_length': 4096 } }),
    }) as unknown as typeof fetch

    await expect(ollamaEmbeddings('ollama:llama3').resolve()).rejects.toThrow(/does not support embeddings/i)
  })

  it('rejects a response with fewer vectors than inputs', async () => {
    globalThis.fetch = mockOllama({
      '/api/show': () => jsonResponse({ capabilities: ['embedding'], model_info: { 'test.embedding_length': 2 } }),
      '/api/embed': () => jsonResponse({ embeddings: [[1, 0]] }),
    }) as unknown as typeof fetch

    const { embedder } = await ollamaEmbeddings('ollama:x').resolve()
    await expect(embedder(['one', 'two'])).rejects.toThrow(/returned 1 embeddings for 2 inputs/i)
  })

  it('rejects a vector with the wrong dimensions', async () => {
    globalThis.fetch = mockOllama({
      '/api/show': () => jsonResponse({ capabilities: ['embedding'], model_info: { 'test.embedding_length': 3 } }),
      '/api/embed': () => jsonResponse({ embeddings: [[1, 0]] }),
    }) as unknown as typeof fetch

    const { embedder } = await ollamaEmbeddings('ollama:x').resolve()
    await expect(embedder(['one'])).rejects.toThrow(/returned 2 dimensions, expected 3/i)
  })

  it('rejects a zero vector', async () => {
    globalThis.fetch = mockOllama({
      '/api/show': () => jsonResponse({ capabilities: ['embedding'], model_info: { 'test.embedding_length': 2 } }),
      '/api/embed': () => jsonResponse({ embeddings: [[0, 0]] }),
    }) as unknown as typeof fetch

    const { embedder } = await ollamaEmbeddings('ollama:x').resolve()
    await expect(embedder(['one'])).rejects.toThrow(/zero vector/i)
  })

  it('rejects a vector whose magnitude overflows', async () => {
    globalThis.fetch = mockOllama({
      '/api/show': () => jsonResponse({ capabilities: ['embedding'], model_info: { 'test.embedding_length': 2 } }),
      '/api/embed': () => jsonResponse({ embeddings: [[Number.MAX_VALUE, Number.MAX_VALUE]] }),
    }) as unknown as typeof fetch

    const { embedder } = await ollamaEmbeddings('ollama:x').resolve()
    await expect(embedder(['one'])).rejects.toThrow(/invalid vector magnitude/i)
  })

  it('surfaces the Ollama error message when embedding fails', async () => {
    globalThis.fetch = mockOllama({
      '/api/show': () => jsonResponse(SHOW_EMBEDDING),
      '/api/embed': () => jsonResponse({ error: 'model "x" not found, try pulling it first' }, false, 404),
    }) as unknown as typeof fetch

    const { embedder } = await ollamaEmbeddings('ollama:x').resolve()
    await expect(embedder(['hi'])).rejects.toThrow(/try pulling it first/)
  })
})

describe('getAvailableOllamaEmbedModels', () => {
  const original = globalThis.fetch

  beforeEach(() => {
    delete process.env.OLLAMA_HOST
  })

  afterEach(() => {
    globalThis.fetch = original
    vi.restoreAllMocks()
  })

  // Discovery feeds a config menu; it must degrade to an empty list rather
  // than block or throw when Ollama is not installed.
  it('returns an empty list when the daemon is unreachable', async () => {
    globalThis.fetch = vi.fn(async () => {
      throw new Error('ECONNREFUSED')
    }) as unknown as typeof fetch

    await expect(getAvailableOllamaEmbedModels()).resolves.toEqual([])
  })

  it('keeps only models advertising the embedding capability', async () => {
    globalThis.fetch = vi.fn(async (url: string | URL, init?: RequestInit) => {
      const path = new URL(String(url)).pathname
      if (path === '/api/tags') {
        return jsonResponse({
          models: [
            { name: 'qwen3-embedding:0.6b', details: { parameter_size: '595M' } },
            { name: 'llama3:8b', details: { parameter_size: '8B' } },
          ],
        })
      }
      const body = JSON.parse(String(init?.body)) as { model: string }
      return jsonResponse(body.model.startsWith('qwen3')
        ? SHOW_EMBEDDING
        : { capabilities: ['completion'], model_info: {} })
    }) as unknown as typeof fetch

    const models = await getAvailableOllamaEmbedModels()

    expect(models).toHaveLength(1)
    expect(models[0]!.id).toBe('ollama:qwen3-embedding:0.6b')
    expect(models[0]!.dimensions).toBe(1024)
    expect(models[0]!.hint).toContain('1024d')
  })

  it('drops models whose capabilities cannot be confirmed', async () => {
    globalThis.fetch = vi.fn(async (url: string | URL) => {
      const path = new URL(String(url)).pathname
      if (path === '/api/tags')
        return jsonResponse({ models: [{ name: 'mystery' }] })
      return jsonResponse({}, false, 500)
    }) as unknown as typeof fetch

    await expect(getAvailableOllamaEmbedModels()).resolves.toEqual([])
  })
})
