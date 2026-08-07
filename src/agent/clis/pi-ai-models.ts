/**
 * pi-ai model identification + dynamic model enumeration.
 *
 * Models are sourced from pi-ai's registry; legacy generations and tiny
 * context windows are filtered out. A "recommended" model is auto-picked
 * per provider for ergonomic defaults.
 */

import type { Models } from '@earendil-works/pi-ai'
import { createPiAiModels } from './pi-ai-auth.ts'

export function isPiAiModel(model: string): boolean {
  return model.startsWith('pi:')
}

/** Parse a `pi:provider/model-id` string → `{ provider, modelId }`. */
export function parsePiAiModelId(model: string): { provider: string, modelId: string } | null {
  if (!model.startsWith('pi:'))
    return null
  const rest = model.slice(3)
  const slashIdx = rest.indexOf('/')
  if (slashIdx === -1)
    return null
  return { provider: rest.slice(0, slashIdx), modelId: rest.slice(slashIdx + 1) }
}

const MIN_CONTEXT_WINDOW = 32_000

/** Old generations that clutter the model list. */
const LEGACY_MODEL_PATTERNS = [
  // Anthropic: claude 3.x family
  /^claude-3-/,
  /^claude-3\.5-/,
  /^claude-3\.7-/,
  // OpenAI: pre-gpt-5
  /^gpt-4(?!\.\d)/, // gpt-4, gpt-4-turbo, gpt-4o but not gpt-4.1
  /^o1/,
  /^o3-mini/,
  // Google: old gemini generations + non-text models
  /^gemini-1\./,
  /^gemini-2\.0/,
  /^gemini-live-/,
  // Preview snapshots with date suffixes
  /-preview-\d{2}-\d{2,4}$/,
  // Dated model snapshots
  /-\d{8}$/,
]

function isLegacyModel(modelId: string): boolean {
  return LEGACY_MODEL_PATTERNS.some(p => p.test(modelId))
}

/** Cheapest reliable option per provider, picked for auto-selection. */
const RECOMMENDED_MODELS: Record<string, RegExp> = {
  anthropic: /haiku/,
  google: /flash/,
  openai: /gpt-4\.1-mini/,
}

export interface PiAiModelInfo {
  /** Full model ID: `pi:provider/model-id`. */
  id: string
  name: string
  hint: string
  authSource: 'api-key' | 'oauth'
  recommended: boolean
}

export async function getAvailablePiAiModels(models: Models = createPiAiModels()): Promise<PiAiModelInfo[]> {
  const modelsByProvider = Map.groupBy(await models.getAvailable(), model => model.provider)
  const available: PiAiModelInfo[] = []

  for (const [provider, providerModels] of modelsByProvider) {
    const auth = await models.checkAuth(provider)
    if (!auth)
      continue
    const authSource = auth.type === 'oauth' ? 'oauth' : 'api-key'

    const recPattern = RECOMMENDED_MODELS[provider]
    const recModelId = recPattern
      ? providerModels.find(model => !isLegacyModel(model.id) && recPattern.test(model.id))?.id
      : undefined

    for (const model of providerModels) {
      if (model.contextWindow && model.contextWindow < MIN_CONTEXT_WINDOW)
        continue
      if (isLegacyModel(model.id))
        continue

      const id = `pi:${provider}/${model.id}`
      const ctx = model.contextWindow ? ` · ${Math.round(model.contextWindow / 1000)}k ctx` : ''
      const cost = model.cost?.input ? ` · $${model.cost.input}/Mtok` : ''

      available.push({
        id,
        name: model.name || model.id,
        hint: `${authSource === 'oauth' ? 'OAuth' : 'API key'}${ctx}${cost}`,
        authSource,
        recommended: model.id === recModelId,
      })
    }
  }

  return available
}
