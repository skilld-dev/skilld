/**
 * Interactive @clack/prompts UI for LLM model + section selection.
 *
 * Three entry points:
 *   - `selectModel`         — pick a model (or use configured) for one-off use
 *   - `selectSkillSections` — pick which SKILL.md sections to enhance
 *   - `selectLlmConfig`     — combined flow: model + sections + prompt-only
 *                             option, with update-context hints
 *
 * All return `null` on cancel/no-models/no-tty. Updates `config.model` when
 * the user picks a non-default model.
 */

import type { CustomPrompt, OptimizeModel, SkillSection } from '../agent/index.ts'
import * as p from '@clack/prompts'
import { getAvailableModels, getModelName } from '../agent/index.ts'
import { maxItems, maxLines } from '../agent/prompts/optional/budget.ts'
import { isInteractive } from '../cli/env.ts'
import { NO_MODELS_MESSAGE, pickModel } from '../cli/model-picker.ts'
import { readConfig, updateConfig } from '../core/config.ts'
import { semverDiff } from '../core/semver.ts'

/** Default sections when model is pre-set (non-interactive) */
export const DEFAULT_SECTIONS: SkillSection[] = ['best-practices', 'api-changes']

/**
 * Resolve the model to use when `-y` is passed without `-m`. Returns the
 * caller-supplied override, then the configured model, then the recommended
 * available model. Returns `undefined` when no model can be auto-picked
 * (skipLlm set, no `-y`, or no models installed).
 */
export async function resolveAutoModel(
  override: OptimizeModel | undefined,
  yes: boolean | undefined,
): Promise<OptimizeModel | undefined> {
  if (override)
    return override
  const config = readConfig()
  if (!yes || config.skipLlm)
    return undefined
  if (config.model)
    return config.model
  const available = await getAvailableModels()
  const auto = available.find(m => m.recommended)?.id ?? available[0]?.id
  return auto as OptimizeModel | undefined
}

/** Select LLM model for SKILL.md generation (independent of target agent) */
export async function selectModel(skipPrompt: boolean): Promise<OptimizeModel | null> {
  const config = readConfig()
  const available = await getAvailableModels()

  if (available.length === 0) {
    p.log.warn(NO_MODELS_MESSAGE)
    return null
  }

  if (skipPrompt) {
    if (config.model && available.some(m => m.id === config.model))
      return config.model
    if (config.model)
      p.log.warn(`Configured model \x1B[36m${config.model}\x1B[0m is unavailable — using auto-selected fallback`)
    return available.find(m => m.recommended)?.id ?? available[0]!.id
  }

  const choice = await pickModel(available)
  if (!choice)
    return null

  updateConfig({ model: choice as OptimizeModel })
  return choice as OptimizeModel
}

export async function selectSkillSections(message = 'Enhance SKILL.md'): Promise<{ sections: SkillSection[], customPrompt?: CustomPrompt, cancelled: boolean }> {
  p.log.info('Budgets adapt to package release density.')
  const selected = await p.multiselect({
    message,
    options: [
      { label: 'API changes', value: 'api-changes' as SkillSection, hint: 'new/deprecated APIs from version history' },
      { label: 'Best practices', value: 'best-practices' as SkillSection, hint: 'gotchas, pitfalls, patterns' },
      { label: 'Custom section', value: 'custom' as SkillSection, hint: 'add your own section' },
    ],
    initialValues: DEFAULT_SECTIONS,
    required: false,
  })

  if (p.isCancel(selected))
    return { sections: [], cancelled: true }

  const sections = selected as SkillSection[]
  if (sections.length === 0)
    return { sections: [], cancelled: false }

  if (sections.length > 1) {
    const n = sections.length
    const budgetLines: string[] = []
    for (const s of sections) {
      switch (s) {
        case 'api-changes':
          budgetLines.push(`  API changes     ${maxItems(6, 12, n)}–${maxItems(6, Math.round(12 * 1.6), n)} items (adapts to release churn)`)
          break
        case 'best-practices':
          budgetLines.push(`  Best practices  ${maxItems(4, 10, n)}–${maxItems(4, Math.round(10 * 1.3), n)} items`)
          break
        case 'custom':
          budgetLines.push(`  Custom          ≤${maxLines(50, 80, n)} lines`)
          break
      }
    }
    p.log.info(`Budget (${n} sections):\n${budgetLines.join('\n')}`)
  }

  let customPrompt: CustomPrompt | undefined
  if (sections.includes('custom')) {
    const heading = await p.text({
      message: 'Section heading',
      placeholder: 'e.g. "Migration from v2" or "SSR Patterns"',
    })
    if (p.isCancel(heading))
      return { sections: [], cancelled: true }

    const body = await p.text({
      message: 'Instructions for this section',
      placeholder: 'e.g. "Document breaking changes and migration steps from v2 to v3"',
    })
    if (p.isCancel(body))
      return { sections: [], cancelled: true }

    customPrompt = { heading: heading as string, body: body as string }
  }

  return { sections, customPrompt, cancelled: false }
}

export interface LlmConfig {
  model: OptimizeModel
  sections: SkillSection[]
  customPrompt?: CustomPrompt
  promptOnly?: boolean
}

/** Context about the existing skill when running an update (not a fresh add). */
export interface UpdateContext {
  oldVersion?: string
  newVersion?: string
  syncedAt?: string
  /** Whether the existing SKILL.md was LLM-enhanced (has generated_by in frontmatter). */
  wasEnhanced: boolean
  /** Pre-computed bump type (used by parallel sync to pass the max across packages). */
  bumpType?: string
}

/**
 * Resolve sections + model for LLM enhancement.
 * If presetModel is provided, uses DEFAULT_SECTIONS without prompting.
 * Returns null if cancelled or no sections/model selected.
 */
export async function selectLlmConfig(presetModel?: OptimizeModel, message?: string, updateCtx?: UpdateContext): Promise<LlmConfig | null> {
  if (presetModel) {
    const available = await getAvailableModels()
    if (available.some(m => m.id === presetModel))
      return { model: presetModel, sections: DEFAULT_SECTIONS }
    if (!isInteractive())
      return null
  }

  if (!isInteractive())
    return null

  const config = readConfig()
  const available = await getAvailableModels()

  if (available.length === 0) {
    p.log.warn(NO_MODELS_MESSAGE)
    return null
  }

  let defaultModel: OptimizeModel
  if (config.model && available.some(m => m.id === config.model)) {
    defaultModel = config.model
  }
  else {
    if (config.model)
      p.log.warn(`Configured model \x1B[36m${config.model}\x1B[0m is unavailable — using auto-selected fallback`)
    defaultModel = (available.find(m => m.recommended)?.id ?? available[0]!.id) as OptimizeModel
  }

  const defaultModelName = getModelName(defaultModel)
  const defaultModelInfo = available.find(m => m.id === defaultModel)
  const providerHint = defaultModelInfo?.providerName ?? ''
  const sourceHint = config.model === defaultModel ? 'configured' : 'recommended'
  const defaultHint = providerHint ? `${providerHint} · ${sourceHint}` : sourceHint

  let enhanceMessage = message ? `${message}?` : 'Enhance SKILL.md?'
  let defaultToSkip = false
  if (updateCtx) {
    const diff = updateCtx.bumpType
      ?? (updateCtx.oldVersion && updateCtx.newVersion ? semverDiff(updateCtx.oldVersion, updateCtx.newVersion) : null)
    const isSmallBump = diff === 'patch' || diff === 'prerelease' || diff === 'prepatch' || diff === 'preminor' || diff === 'premajor'

    const ageParts: string[] = []
    if (diff)
      ageParts.push(diff)
    if (updateCtx.syncedAt) {
      const syncedAtMs = new Date(updateCtx.syncedAt).getTime()
      if (Number.isFinite(syncedAtMs)) {
        const days = Math.floor((Date.now() - syncedAtMs) / 86_400_000)
        ageParts.push(days === 0 ? 'today' : days === 1 ? '1d ago' : `${days}d ago`)
      }
    }
    if (updateCtx.wasEnhanced)
      ageParts.push('LLM-enhanced')

    const versionHint = updateCtx.oldVersion && updateCtx.newVersion
      ? `${updateCtx.oldVersion} → ${updateCtx.newVersion}`
      : null
    const hint = [versionHint, ...ageParts].filter(Boolean).join(' · ')
    if (hint)
      enhanceMessage = `Enhance SKILL.md? \x1B[90m(${hint})\x1B[0m`

    if (updateCtx.wasEnhanced && isSmallBump)
      defaultToSkip = true
  }

  const choice = await p.select({
    message: enhanceMessage,
    options: [
      { label: defaultModelName, value: 'default' as const, hint: defaultHint },
      { label: 'Different model', value: 'pick' as const, hint: 'choose another enhancement model' },
      { label: 'Prompt only', value: 'prompt' as const, hint: 'write prompts for manual use' },
      { label: 'Skip', value: 'skip' as const, hint: 'base skill with docs, issues, and types' },
    ],
    ...(defaultToSkip ? { initialValue: 'skip' as const } : {}),
  })

  if (p.isCancel(choice))
    return null

  if (choice === 'skip')
    return null

  if (choice === 'prompt') {
    const { sections, customPrompt, cancelled } = await selectSkillSections(
      message ? `${message} (prompt only)` : 'Select sections for prompt generation',
    )
    if (cancelled || sections.length === 0)
      return null
    return { model: defaultModel, sections, customPrompt, promptOnly: true }
  }

  let model: OptimizeModel
  if (choice === 'pick') {
    const picked = await pickModel(available)
    if (!picked)
      return null
    updateConfig({ model: picked as OptimizeModel })
    model = picked as OptimizeModel
  }
  else {
    model = defaultModel
  }
  if (!model)
    return null

  const modelName = getModelName(model)
  const { sections, customPrompt, cancelled } = await selectSkillSections(
    message ? `${message} (${modelName})` : `Enhance SKILL.md with ${modelName}`,
  )

  if (cancelled || sections.length === 0)
    return null

  return { model, sections, customPrompt }
}
