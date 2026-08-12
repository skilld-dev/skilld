import type { EmbeddingConfig } from 'retriv'
import { rm } from 'node:fs/promises'
import { resolve } from 'pathe'
import { getModelDimensions, getModelMaxTokens, resolveModelForPreset } from 'retriv/embeddings/model-info'
import type { RuntimeEmbedDevice } from './models.ts'

export interface TransformersEmbeddingOptions {
  model: string
  device?: RuntimeEmbedDevice
}

/** Transformers.js provider with explicit device support. */
export function transformersEmbeddings(options: TransformersEmbeddingOptions): EmbeddingConfig {
  const model = resolveModelForPreset(options.model, 'transformers.js')
  let cached: Awaited<ReturnType<EmbeddingConfig['resolve']>> | undefined

  return {
    async resolve() {
      if (cached)
        return cached

      // Search is optional, so keep the model runtime outside the main CLI bundle.
      const { env, pipeline } = await import('@huggingface/transformers')
      const load = () => pipeline('feature-extraction', model, {
        dtype: 'fp32',
        ...(options.device ? { device: options.device } : {}),
      })
      const extractor = await load().catch(async (error) => {
        const corrupted = error instanceof Error
          && (error.message.includes('Protobuf parsing failed') || String(error.cause).includes('Protobuf parsing failed'))
        if (!corrupted || !env.cacheDir)
          throw error
        const cacheRoot = resolve(env.cacheDir)
        const modelCache = resolve(cacheRoot, model)
        if (!modelCache.startsWith(`${cacheRoot}/`))
          throw error
        await rm(modelCache, { recursive: true, force: true })
        console.warn(`[skilld] Cleared corrupted model cache for ${model}, retrying...`)
        return load()
      })

      const dimensions = getModelDimensions(model)
      if (!dimensions)
        throw new Error(`Unknown dimensions for model ${model}.`)

      const embedder = async (texts: string[]) => {
        const output = await extractor(texts, { pooling: 'mean', normalize: true })
        const data = output.data as Float32Array
        return Array.from(
          { length: texts.length },
          (_, index) => data.slice(index * dimensions, (index + 1) * dimensions),
        )
      }

      cached = {
        embedder,
        dimensions,
        maxTokens: getModelMaxTokens(model),
      }
      return cached
    },
  }
}
