import { getModelDimensions, resolveModelForPreset } from 'retriv/embeddings/model-info'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { DEFAULT_EMBED_DEVICE, DEFAULT_EMBED_MODEL, EMBED_DEVICES, EMBED_MODELS, getEmbedDeviceInfo, getEmbedModelInfo, resolveEmbedDevice, resolveEmbedModel } from '../../src/retriv/models.ts'

describe('resolveEmbedModel', () => {
  let original: string | undefined

  beforeEach(() => {
    original = process.env.SKILLD_EMBED_MODEL
    delete process.env.SKILLD_EMBED_MODEL
  })

  afterEach(() => {
    if (original === undefined)
      delete process.env.SKILLD_EMBED_MODEL
    else
      process.env.SKILLD_EMBED_MODEL = original
  })

  it('falls back to the default when nothing is configured', () => {
    expect(resolveEmbedModel(undefined)).toBe(DEFAULT_EMBED_MODEL)
  })

  it('uses the configured model', () => {
    expect(resolveEmbedModel('bge-base-en-v1.5')).toBe('bge-base-en-v1.5')
  })

  it('lets the env var override configured and default', () => {
    process.env.SKILLD_EMBED_MODEL = 'bge-m3'
    expect(resolveEmbedModel('bge-base-en-v1.5')).toBe('bge-m3')
    expect(resolveEmbedModel(undefined)).toBe('bge-m3')
  })

  it('ignores a blank env var', () => {
    process.env.SKILLD_EMBED_MODEL = '   '
    expect(resolveEmbedModel('bge-base-en-v1.5')).toBe('bge-base-en-v1.5')
  })
})

describe('embed model registry', () => {
  it('includes the default model', () => {
    expect(EMBED_MODELS.map(m => m.id)).toContain(DEFAULT_EMBED_MODEL)
  })

  it('has no duplicate ids', () => {
    const ids = EMBED_MODELS.map(m => m.id)
    expect(new Set(ids).size).toBe(ids.length)
  })

  it('looks up known models and rejects unknown ones', () => {
    expect(getEmbedModelInfo(DEFAULT_EMBED_MODEL)?.dimensions).toBe(384)
    expect(getEmbedModelInfo('not-a-model')).toBeUndefined()
  })

  // retriv's bare `bge-large-en-v1.5` preset maps to
  // `onnx-community/bge-large-en-v1.5`, whose weights return 401. We pin the
  // Xenova repo instead, so the bare id must never creep back in.
  it('avoids the bge-large preset that resolves to unavailable weights', () => {
    const ids = EMBED_MODELS.map(m => m.id)
    expect(ids).not.toContain('bge-large-en-v1.5')
    expect(ids).toContain('Xenova/bge-large-en-v1.5')
  })

  // Guards against drift: every id must resolve to a real transformers.js repo
  // and our declared width must match retriv's registry, since the declared
  // width is what warns users about rebuilding indexes.
  it('matches retriv model resolution and dimensions', () => {
    for (const model of EMBED_MODELS) {
      const resolved = resolveModelForPreset(model.id, 'transformers.js')
      expect(resolved, `${model.id} should resolve`).toBeTruthy()
      expect(resolved, `${model.id} should map to a namespaced repo`).toContain('/')
      expect(getModelDimensions(model.id), `${model.id} dimensions`).toBe(model.dimensions)
    }
  })
})

describe('resolveEmbedDevice', () => {
  let original: string | undefined

  beforeEach(() => {
    original = process.env.SKILLD_EMBED_DEVICE
    delete process.env.SKILLD_EMBED_DEVICE
  })

  afterEach(() => {
    if (original === undefined)
      delete process.env.SKILLD_EMBED_DEVICE
    else
      process.env.SKILLD_EMBED_DEVICE = original
  })

  // `auto` must resolve to undefined so the option is omitted entirely and
  // transformers.js keeps its own device resolution.
  it('returns undefined for auto so the option is omitted', () => {
    expect(resolveEmbedDevice(undefined)).toBeUndefined()
    expect(resolveEmbedDevice(DEFAULT_EMBED_DEVICE)).toBeUndefined()
  })

  it('returns the configured device', () => {
    expect(resolveEmbedDevice('webgpu')).toBe('webgpu')
  })

  it('lets the env var override configured and default', () => {
    process.env.SKILLD_EMBED_DEVICE = 'cpu'
    expect(resolveEmbedDevice('webgpu')).toBe('cpu')
    expect(resolveEmbedDevice(undefined)).toBe('cpu')
  })

  it('ignores a blank env var', () => {
    process.env.SKILLD_EMBED_DEVICE = '   '
    expect(resolveEmbedDevice('webgpu')).toBe('webgpu')
  })

  it('treats an env var of auto as unset', () => {
    process.env.SKILLD_EMBED_DEVICE = 'auto'
    expect(resolveEmbedDevice('webgpu')).toBeUndefined()
  })

  it('rejects an unknown configured device', () => {
    expect(() => resolveEmbedDevice('quantum')).toThrow(/unknown embedding device/i)
  })

  it('rejects an unknown device from the environment', () => {
    process.env.SKILLD_EMBED_DEVICE = 'quantum'
    expect(() => resolveEmbedDevice('cpu')).toThrow(/unknown embedding device/i)
  })
})

describe('embed device registry', () => {
  it('includes the default device', () => {
    expect(EMBED_DEVICES.map(d => d.id)).toContain(DEFAULT_EMBED_DEVICE)
  })

  it('has no duplicate ids', () => {
    const ids = EMBED_DEVICES.map(d => d.id)
    expect(new Set(ids).size).toBe(ids.length)
  })

  it('looks up known devices and rejects unknown ones', () => {
    expect(getEmbedDeviceInfo('webgpu')?.label).toBe('GPU (WebGPU)')
    expect(getEmbedDeviceInfo('not-a-device')).toBeUndefined()
  })
})
