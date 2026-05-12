import * as p from '@clack/prompts'

export const OAUTH_NOTE
  = '\x1B[33m⚠\x1B[0m  OAuth providers are disabled.\n'
    + '\n'
    + 'Consumer subscription OAuth impersonates official CLI clients and\n'
    + 'violates provider Terms of Service, risking account bans.\n'
    + '\n'
    + 'Use API keys or native CLI tools instead:\n'
    + '  \x1B[36mANTHROPIC_API_KEY\x1B[0m / \x1B[36mclaude\x1B[0m CLI\n'
    + '  \x1B[36mOPENAI_API_KEY\x1B[0m   / \x1B[36mcodex\x1B[0m CLI\n'
    + '  \x1B[36mGEMINI_API_KEY\x1B[0m   / \x1B[36mgemini\x1B[0m CLI'

export const NO_MODELS_MESSAGE = 'No enhancement models detected.\n'
  + '  \x1B[90mSkills work fine without this, you get raw docs, issues, and types.\n'
  + '  Enhancement compresses them into a concise cheat sheet with gotchas.\x1B[0m\n'
  + '\n'
  + '  To connect a model (optional):\n'
  + '  1. Set an env var: ANTHROPIC_API_KEY, GEMINI_API_KEY, or OPENAI_API_KEY\n'
  + '  2. Install a CLI tool: \x1B[36mclaude\x1B[0m, \x1B[36mgemini\x1B[0m, or \x1B[36mcodex\x1B[0m (restart wizard after)'

export function groupModelsByProvider<T extends { provider: string, providerName: string, vendorGroup?: string }>(models: T[]): Map<string, { name: string, models: T[] }> {
  const byVendor = new Map<string, { name: string, models: T[] }>()
  for (const m of models) {
    const key = m.vendorGroup ?? m.provider
    if (!byVendor.has(key))
      byVendor.set(key, { name: key, models: [] })
    byVendor.get(key)!.models.push(m)
  }
  return byVendor
}

export interface ModelPickerOptions {
  before?: Array<{ label: string, value: string, hint?: string }>
  after?: Array<{ label: string, value: string, hint?: string }>
}

export async function pickModel<T extends { provider: string, providerName: string, name: string, id: string, hint: string, recommended?: boolean }>(
  models: T[],
  opts: ModelPickerOptions = {},
): Promise<string | null> {
  const byProvider = groupModelsByProvider(models)
  const before = opts.before ?? []
  const after = opts.after ?? []

  if (byProvider.size === 1 && before.length === 0) {
    const [, group] = [...byProvider.entries()][0]!
    const choice = await p.select({
      message: `${group.name}`,
      options: [
        ...group.models.map(m => ({
          label: m.recommended ? `${m.name} (recommended - fast and cheap)` : m.name,
          value: m.id,
          hint: m.hint,
        })),
        ...after,
      ],
    })
    return p.isCancel(choice) ? null : choice as string
  }

  const providerChoice = await p.select({
    message: 'Select provider',
    options: [
      ...before,
      ...Array.from(byProvider.entries(), ([key, { name, models: ms }]) => ({
        label: name,
        value: key,
        hint: `${ms.length} models`,
      })),
      ...after,
    ],
  })

  if (p.isCancel(providerChoice))
    return null

  const providerStr = providerChoice as string
  if (before.some(o => o.value === providerStr) || after.some(o => o.value === providerStr))
    return providerStr

  const group = byProvider.get(providerStr)!
  const modelChoice = await p.select({
    message: `Select model (${group.name})`,
    options: group.models.map(m => ({
      label: m.recommended ? `${m.name} (recommended - fast and cheap)` : m.name,
      value: m.id,
      hint: m.hint,
    })),
  })

  return p.isCancel(modelChoice) ? null : modelChoice as string
}
