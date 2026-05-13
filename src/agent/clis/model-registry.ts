/**
 * Model registry: sources display names from @earendil-works/pi-ai's auto-generated
 * model list, so CLI adapters don't have to hand-maintain "Opus 4.6" / "GPT-5.3" labels.
 *
 * Each adapter declares the CLI flag value it passes (e.g. `opus`, `gpt-5.3-codex`)
 * plus an id pattern; we resolve the latest matching pi-ai entry for the human label.
 */

import type { KnownProvider, Model } from '@earendil-works/pi-ai'
import type { CliModelEntry } from './types.ts'
import { getModels } from '@earendil-works/pi-ai'

const STATIC_REGEX_1 = /-\d{8}$/
const STATIC_REGEX_2 = /-\d{4}-\d{2}-\d{2}$/
const STATIC_REGEX_3 = /-preview/

/** Strip dated aliases (claude-opus-4-5-20251101) and preview tags. */
function isStableId(id: string): boolean {
  if (STATIC_REGEX_1.test(id))
    return false
  if (STATIC_REGEX_2.test(id))
    return false
  if (STATIC_REGEX_3.test(id))
    return false
  return true
}

/**
 * Numeric version comparator on the trailing version chunks of an id
 * (e.g. claude-opus-4-7 > claude-opus-4-6, gpt-5.3 > gpt-5.2).
 */
function compareVersions(a: string, b: string): number {
  const versionOf = (id: string): number[] => {
    const nums = id.match(/\d+(?:\.\d+)?/g) ?? []
    return nums.flatMap(n => n.split('.').map(Number))
  }
  const av = versionOf(a)
  const bv = versionOf(b)
  for (let i = 0; i < Math.max(av.length, bv.length); i++) {
    const x = av[i] ?? 0
    const y = bv[i] ?? 0
    if (x !== y)
      return y - x
  }
  return 0
}

interface ResolveOpts {
  provider: KnownProvider
  /** Prefix the id must start with (e.g. `claude-opus-`, `gpt-5.`, `gemini-`). */
  prefix: string
  /** Optional substring filter (e.g. `codex`, `flash`). */
  contains?: string
  /** Skip ids containing any of these substrings. */
  exclude?: string[]
}

function resolve(opts: ResolveOpts): Model<any> | undefined {
  const { provider, prefix, contains, exclude } = opts
  const matches = getModels(provider)
    .filter(m => isStableId(m.id))
    .filter(m => m.id.startsWith(prefix))
    .filter(m => !contains || m.id.includes(contains))
    .filter(m => !exclude?.some(e => m.id.includes(e)))
    .sort((a, b) => compareVersions(a.id, b.id))
  return matches[0]
}

/**
 * Build a CliModelEntry by looking up the latest matching pi-ai model and using its
 * display name. `model` is the literal value passed to the CLI (alias or id).
 */
export function buildModelEntry(opts: ResolveOpts & {
  /** Value passed to the CLI's --model flag. Defaults to the resolved pi-ai id. */
  model?: string
  /** Override for the display name. Defaults to pi-ai's `name`. */
  name?: string
  /** Post-process pi-ai's name (e.g. strip "Claude " prefix). */
  nameTransform?: (name: string) => string
  hint: string
  recommended?: boolean
}): CliModelEntry {
  const found = resolve(opts)
  const piName = found?.name
  const transformed = piName && opts.nameTransform ? opts.nameTransform(piName) : piName
  const name = opts.name ?? transformed ?? opts.model ?? opts.prefix
  const model = opts.model ?? found?.id ?? opts.prefix
  return { model, name, hint: opts.hint, recommended: opts.recommended }
}

/** Build a Record<id, CliModelEntry> from a list of entry specs, keyed by their `model` field. */
export function buildModels(entries: Array<Parameters<typeof buildModelEntry>[0]>): Record<string, CliModelEntry> {
  return Object.fromEntries(entries.map((e) => {
    const entry = buildModelEntry(e)
    return [entry.model, entry]
  }))
}
