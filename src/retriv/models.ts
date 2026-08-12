/**
 * Local embedding models available to the search index.
 *
 * Every model here runs offline through transformers.js. It needs no API key or
 * network after the initial download. Larger models retrieve more accurately
 * but cost more time and memory to index with.
 *
 * Dimensions are fixed per model and sqlite-vec columns are fixed-width, so
 * switching model invalidates existing indexes. Rebuild with
 * `skilld update`.
 */
export interface EmbedModelInfo {
  /** Model id passed to retriv (resolved to a Hugging Face repo internally) */
  id: string
  label: string
  /** Vector width, which determines index layout */
  dimensions: number
  hint: string
}

export const DEFAULT_EMBED_MODEL = 'bge-small-en-v1.5'

export const EMBED_MODELS: readonly EmbedModelInfo[] = [
  {
    id: 'bge-small-en-v1.5',
    label: 'BGE small (English)',
    dimensions: 384,
    hint: 'fastest to index, smallest download',
  },
  {
    id: 'bge-base-en-v1.5',
    label: 'BGE base (English)',
    dimensions: 768,
    hint: 'balanced accuracy and speed',
  },
  {
    // Pinned to the full repo id on purpose: retriv's `bge-large-en-v1.5`
    // preset maps to `onnx-community/bge-large-en-v1.5`, which returns 401.
    // The Xenova repo carries the same weights and resolves correctly.
    id: 'Xenova/bge-large-en-v1.5',
    label: 'BGE large (English)',
    dimensions: 1024,
    hint: 'most accurate English retrieval, slowest to index',
  },
  {
    id: 'bge-m3',
    label: 'BGE m3 (multilingual)',
    dimensions: 1024,
    hint: 'multilingual, 8192-token context',
  },
]

export function getEmbedModelInfo(id: string): EmbedModelInfo | undefined {
  return EMBED_MODELS.find(m => m.id === id)
}

/**
 * Resolve the embedding model to index and query with.
 *
 * `SKILLD_EMBED_MODEL` wins so a single run can be overridden without touching
 * saved config; otherwise the configured value, otherwise the default.
 */
export function resolveEmbedModel(configured?: string): string {
  const fromEnv = process.env.SKILLD_EMBED_MODEL?.trim()
  if (fromEnv)
    return fromEnv
  return configured || DEFAULT_EMBED_MODEL
}

/**
 * Execution device for the embedding model.
 *
 * `auto` means "let transformers.js decide", which resolves to CPU under Node.
 * Everything else is opt-in because the fastest backend is hardware-specific:
 * on an Apple M5 Max `webgpu` measured 2.6-2.9x faster than CPU across every
 * bge size, while `coreml` measured 3-8x slower (it falls back to CPU for
 * unsupported ops and pays for graph partitioning).
 */
export interface EmbedDeviceInfo {
  id: EmbedDeviceSetting
  label: string
  hint: string
}

export const DEFAULT_EMBED_DEVICE = 'auto'
export type EmbedDevice = 'cpu' | 'webgpu' | 'coreml'
export type EmbedDeviceSetting = typeof DEFAULT_EMBED_DEVICE | EmbedDevice

export const EMBED_DEVICES: readonly EmbedDeviceInfo[] = [
  {
    id: 'auto',
    label: 'Auto',
    hint: 'let transformers.js choose; CPU under Node',
  },
  {
    id: 'cpu',
    label: 'CPU',
    hint: 'always available, predictable',
  },
  {
    id: 'webgpu',
    label: 'GPU (WebGPU)',
    hint: 'fastest on Apple Silicon in testing; verify on your hardware',
  },
  {
    id: 'coreml',
    label: 'CoreML',
    hint: 'Apple Neural Engine; measured slower than CPU for these models',
  },
]

export function getEmbedDeviceInfo(id: string): EmbedDeviceInfo | undefined {
  return EMBED_DEVICES.find(d => d.id === id)
}

/**
 * Resolve the execution device. Returns `undefined` for `auto` so the option
 * is omitted entirely and transformers.js keeps its own default resolution.
 */
export function resolveEmbedDevice(configured?: string): EmbedDevice | undefined {
  const fromEnv = process.env.SKILLD_EMBED_DEVICE?.trim()
  const value = fromEnv || configured || DEFAULT_EMBED_DEVICE
  const device = getEmbedDeviceInfo(value)
  if (!device)
    throw new Error(`Unknown embedding device "${value}". Run \`skilld config\` to choose a supported device.`)
  return device.id === DEFAULT_EMBED_DEVICE ? undefined : device.id
}
