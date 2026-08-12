/**
 * Ollama-backed embeddings for the search index.
 *
 * Talks to `/api/embed` directly rather than going through retriv's own Ollama
 * provider, which requires the `ai` SDK and `ollama-ai-provider-v2`. skilld
 * already speaks to Ollama over plain `fetch` for completions, so this keeps
 * the dependency footprint unchanged.
 */
import type { Embedding } from 'retriv'
import { ollamaHost } from '../core/ollama-host.ts'

/** Models are addressed as `ollama:<name>`, matching the enhancement-model syntax. */
export const OLLAMA_PREFIX = 'ollama:'

/** Documents sent per `/api/embed` call. Keeps payloads and timeouts bounded. */
const BATCH_SIZE = 64

/** Embedding runs are slow on large models; discovery stays snappy separately. */
const EMBED_TIMEOUT_MS = 120_000
const DISCOVERY_TIMEOUT_MS = 1500

export function isOllamaEmbedModel(id: string): boolean {
  return id.startsWith(OLLAMA_PREFIX)
}

export function stripOllamaPrefix(id: string): string {
  return id.startsWith(OLLAMA_PREFIX) ? id.slice(OLLAMA_PREFIX.length) : id
}

interface OllamaShowResponse {
  capabilities?: string[]
  model_info?: Record<string, unknown>
}

interface OllamaTagsResponse {
  models?: Array<{
    name: string
    size?: number
    details?: { parameter_size?: string, quantization_level?: string }
  }>
}

/**
 * Pull a value out of `model_info`, whose keys are architecture-prefixed
 * (`qwen3.embedding_length`, `gemma3.context_length`, …).
 */
function readModelInfo(info: Record<string, unknown> | undefined, suffix: string): number | undefined {
  if (!info)
    return undefined
  for (const [key, value] of Object.entries(info)) {
    if (key.endsWith(suffix) && typeof value === 'number')
      return value
  }
  return undefined
}

async function showModel(name: string, timeout: number): Promise<OllamaShowResponse | null> {
  const res = await fetch(`${ollamaHost()}/api/show`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ model: name }),
    signal: AbortSignal.timeout(timeout),
  }).catch(() => null)
  if (!res?.ok)
    return null
  return await res.json().catch(() => null) as OllamaShowResponse | null
}

export interface OllamaEmbedModelInfo {
  /** Prefixed id, e.g. `ollama:qwen3-embedding:0.6b` */
  id: string
  name: string
  dimensions?: number
  hint: string
}

/**
 * Locally-pulled Ollama models that advertise the `embedding` capability.
 *
 * Returns `[]` when the daemon is unreachable. Discovery must never block or
 * throw, it just contributes nothing to the picker.
 */
export async function getAvailableOllamaEmbedModels(): Promise<OllamaEmbedModelInfo[]> {
  const res = await fetch(`${ollamaHost()}/api/tags`, { signal: AbortSignal.timeout(DISCOVERY_TIMEOUT_MS) })
    .catch(() => null)
  if (!res?.ok)
    return []

  const data = await res.json().catch(() => null) as OllamaTagsResponse | null
  if (!data?.models?.length)
    return []

  const checked = await Promise.all(data.models.map(async (m): Promise<OllamaEmbedModelInfo | null> => {
    const info = await showModel(m.name, DISCOVERY_TIMEOUT_MS)
    // Unlike completions we cannot fail open: a chat model returns an error
    // from /api/embed, so an unconfirmed model would break indexing later.
    if (!info?.capabilities?.includes('embedding'))
      return null

    const dimensions = readModelInfo(info.model_info, '.embedding_length')
    const params = m.details?.parameter_size
    const detail = [dimensions ? `${dimensions}d` : null, params].filter(Boolean).join(' · ')
    return {
      id: `${OLLAMA_PREFIX}${m.name}`,
      name: m.name,
      dimensions,
      hint: detail ? `local · ${detail}` : 'local',
    }
  }))

  return checked.filter((m): m is OllamaEmbedModelInfo => m !== null)
}

function l2Normalize(vector: unknown, expectedDimensions?: number): Float32Array {
  if (!Array.isArray(vector) || !vector.every(value => typeof value === 'number' && Number.isFinite(value)))
    throw new Error('Ollama returned an invalid embedding vector')
  if (expectedDimensions !== undefined && vector.length !== expectedDimensions) {
    throw new Error(
      `Ollama returned ${vector.length} dimensions, expected ${expectedDimensions}`,
    )
  }
  let sum = 0
  for (const value of vector)
    sum += value * value
  const norm = Math.sqrt(sum)
  const out = new Float32Array(vector.length)
  if (norm === 0)
    throw new Error('Ollama returned a zero vector')
  if (!Number.isFinite(norm))
    throw new Error('Ollama returned an invalid vector magnitude')
  for (let i = 0; i < vector.length; i++)
    out[i] = vector[i]! / norm
  return out
}

async function embedBatch(model: string, input: string[], expectedDimensions?: number): Promise<Embedding[]> {
  const res = await fetch(`${ollamaHost()}/api/embed`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ model, input }),
    signal: AbortSignal.timeout(EMBED_TIMEOUT_MS),
  }).catch((err) => {
    throw new Error(`Could not reach Ollama at ${ollamaHost()}: ${err instanceof Error ? err.message : String(err)}`)
  })

  const data = await res.json().catch(() => null) as { embeddings?: unknown[], error?: string } | null
  if (!res.ok || data?.error) {
    const message = data?.error || `HTTP ${res.status}`
    throw new Error(`Ollama embedding failed for ${model}: ${message}`)
  }
  if (!Array.isArray(data?.embeddings) || data.embeddings.length === 0)
    throw new Error(`Ollama returned no embeddings for ${model}`)
  if (data.embeddings.length !== input.length) {
    throw new Error(
      `Ollama returned ${data.embeddings.length} embeddings for ${input.length} inputs`,
    )
  }

  return data.embeddings.map(vector => l2Normalize(vector, expectedDimensions))
}

/**
 * Build a retriv `EmbeddingConfig` backed by Ollama.
 *
 * `id` may be prefixed or bare. Dimensions come from `/api/show` when the model
 * reports them, otherwise from a probe embedding.
 */
export function ollamaEmbeddings(id: string): {
  resolve: () => Promise<{ embedder: (texts: string[]) => Promise<Embedding[]>, dimensions: number, maxTokens?: number }>
} {
  const model = stripOllamaPrefix(id)
  let cached: { embedder: (texts: string[]) => Promise<Embedding[]>, dimensions: number, maxTokens?: number } | null = null

  return {
    async resolve() {
      if (cached)
        return cached

      const info = await showModel(model, DISCOVERY_TIMEOUT_MS)
      if (!info) {
        throw new Error(
          `Ollama is not reachable at ${ollamaHost()}. Start it with \`ollama serve\`, `
          + `or pick a built-in model with \`skilld config\`.`,
        )
      }
      if (!info.capabilities?.includes('embedding')) {
        throw new Error(
          `Ollama model "${model}" does not support embeddings. `
          + `Pull an embedding model, for example \`ollama pull qwen3-embedding\`.`,
        )
      }

      let dimensions = readModelInfo(info.model_info, '.embedding_length')
      if (!dimensions) {
        const [probe] = await embedBatch(model, ['dimension probe'])
        dimensions = probe?.length
      }
      if (!dimensions)
        throw new Error(`Could not determine embedding dimensions for Ollama model "${model}"`)

      const maxTokens = readModelInfo(info.model_info, '.context_length')

      const embedder = async (texts: string[]): Promise<Embedding[]> => {
        if (texts.length === 0)
          return []
        const out: Embedding[] = []
        for (let i = 0; i < texts.length; i += BATCH_SIZE) {
          const batch = await embedBatch(model, texts.slice(i, i + BATCH_SIZE), dimensions)
          out.push(...batch)
        }
        return out
      }

      cached = { embedder, dimensions, maxTokens }
      return cached
    },
  }
}
