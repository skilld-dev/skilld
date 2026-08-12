import { describe, expect, it, vi } from 'vitest'

const { pipeline } = vi.hoisted(() => ({
  pipeline: vi.fn(async () => async (texts: string[]) => ({
    data: new Float32Array(texts.length * 384),
  })),
}))

vi.mock('@huggingface/transformers', () => ({
  env: {},
  pipeline,
}))

import { transformersEmbeddings } from '../../src/retriv/transformers-embeddings.ts'

describe('transformersEmbeddings', () => {
  it('runs the model on the selected device', async () => {
    const embeddings = transformersEmbeddings({
      model: 'bge-small-en-v1.5',
      device: 'webgpu',
    })

    const { embedder } = await embeddings.resolve()
    const result = await embedder(['one', 'two'])

    expect(pipeline).toHaveBeenCalledWith(
      'feature-extraction',
      'Xenova/bge-small-en-v1.5',
      { device: 'webgpu', dtype: 'fp32' },
    )
    expect(result).toHaveLength(2)
    expect(result[0]).toHaveLength(384)
  })
})
