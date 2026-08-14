/**
 * Shared CLI helpers used by subcommand definitions and the main CLI entry.
 * Extracted to avoid circular deps between cli.ts and commands/*.ts.
 */

import type { AgentType, OptimizeModel } from './agent/index.ts'
import type { ProjectState } from './core/skills.ts'
import * as p from '@clack/prompts'
import { parseTree } from 'jsonc-parser'
import { join } from 'pathe'
import { detectCurrentAgent } from 'unagent/env'
import { agents, detectInstalledAgents, detectProjectAgents, detectTargetAgent, getAgentVersion, getModelName } from './agent/index.ts'
import { readConfig, updateConfig } from './core/config.ts'
import { editJsonProperty, patchPackageJson, readPackageJsonSafe } from './core/package-json.ts'
import { version } from './version.ts'

export type { AgentType, OptimizeModel }

export interface IntroOptions {
  state: ProjectState
  /** Installed CLIs that can serve as enhancement models */
  generators?: Array<{ name: string, version: string }>
  /** Configured enhancement model ID */
  modelId?: string
  /** Resolved target agent ID */
  agentId?: string
}

export const sharedArgs = {
  global: {
    type: 'boolean' as const,
    alias: 'g',
    description: 'Install globally to ~/<agent>/skills',
    default: false,
  },
  agent: {
    type: 'enum' as const,
    options: Object.keys(agents),
    alias: 'a',
    description: 'Target agent — where skills are installed',
  },
  model: {
    type: 'string' as const,
    alias: 'm',
    description: 'Enhancement model for SKILL.md generation',
    valueHint: 'id',
  },
  yes: {
    type: 'boolean' as const,
    alias: 'y',
    description: 'Skip prompts, use defaults',
    default: false,
  },
  force: {
    type: 'boolean' as const,
    alias: 'f',
    description: 'Ignore all caches, re-fetch docs and regenerate',
    default: false,
  },
  debug: {
    type: 'boolean' as const,
    description: 'Save raw enhancement output to logs/ for each section',
    default: false,
  },
}

// ── Menu loop utility ─────────────────────────────────────────────────

/** Thrown when a clack prompt is cancelled inside a menuLoop handler */
export class MenuCancel extends Error { override name = 'MenuCancel' }

/** Assert a clack prompt result is not cancelled. Throws MenuCancel if cancelled. */
export function guard<T>(value: T | symbol): T {
  if (p.isCancel(value))
    throw new MenuCancel()
  return value as T
}

export interface MenuOption {
  label: string
  value: string
  hint?: string
}

/**
 * Run a select menu in a loop with automatic back-navigation.
 *
 * - Cancel (Escape) at the menu itself → exits (returns)
 * - Cancel inside a handler (via guard()) → caught, loops back to menu
 * - Handler returns truthy → exits (returns)
 * - Handler returns void/false → loops back to menu
 *
 * Options are rebuilt each iteration so hints stay fresh after changes.
 */
export async function menuLoop(opts: {
  message: string
  options: () => MenuOption[] | Promise<MenuOption[]>
  onSelect: (value: string) => Promise<boolean | void>
  initialValue?: string | (() => string | undefined)
  /** Use fuzzy-searchable autocomplete instead of static select */
  searchable?: boolean
}): Promise<void> {
  while (true) {
    const options = await opts.options()
    const initial = typeof opts.initialValue === 'function' ? opts.initialValue() : opts.initialValue
    const choice = opts.searchable
      ? await p.autocomplete({ message: opts.message, options, ...(initial != null ? { initialValue: initial } : {}) })
      : await p.select({ message: opts.message, options, ...(initial != null ? { initialValue: initial } : {}) })
    if (p.isCancel(choice))
      return
    try {
      if (await opts.onSelect(choice as string))
        return
    }
    catch (err) {
      if (err instanceof MenuCancel)
        continue
      throw err
    }
  }
}

/** Check if we're running inside an AI coding agent */
export function isRunningInsideAgent(): boolean {
  return !!detectCurrentAgent()
}

/** Check if the current environment supports interactive prompts */
export function isInteractive(): boolean {
  if (isRunningInsideAgent())
    return false
  if (process.env.CI)
    return false
  if (!process.stdout.isTTY)
    return false
  return true
}

/** Exit with error if interactive terminal is required but unavailable */
export function requireInteractive(command: string): void {
  if (!isInteractive()) {
    console.error(`Error: \`skilld ${command}\` requires an interactive terminal`)
    process.exit(1)
  }
}

/** Resolve agent from flags/cwd/config. cwd is source of truth over config. */
export function resolveAgent(agentFlag?: string): AgentType | 'none' | null {
  if (process.env.SKILLD_NO_AGENT)
    return null
  return (agentFlag as AgentType | undefined)
    ?? detectTargetAgent()
    ?? (readConfig().agent as AgentType | undefined)
    ?? null
}

let _warnedNoAgent = false
function warnNoAgent(): void {
  if (_warnedNoAgent)
    return
  _warnedNoAgent = true
  p.log.warn('No target agent detected — falling back to prompt-only mode.\n  Use --agent <name> to specify, or run `skilld config` to set a default.')
}

/** Prompt user to pick an agent when auto-detection fails */
export async function promptForAgent(): Promise<AgentType | 'none' | null> {
  const noAgent = !!process.env.SKILLD_NO_AGENT
  const installed = noAgent ? [] : detectInstalledAgents()
  const projectMatches = noAgent ? [] : detectProjectAgents()

  // Non-interactive: auto-select sole installed agent or fall back to prompt-only
  if (!isInteractive()) {
    if (installed.length === 1) {
      updateConfig({ agent: installed[0] })
      return installed[0]!
    }
    warnNoAgent()
    return 'none'
  }

  // Brief context before asking about agents
  p.log.info(
    `Skilld generates reference cards from package docs so your AI agent\n`
    + `  always has accurate APIs for your exact dependency versions.`,
  )

  // Build options: prefer project-matched agents, then installed, then all
  const candidateIds = projectMatches.length > 0
    ? projectMatches
    : installed.length > 0
      ? installed
      : Object.keys(agents) as AgentType[]

  // Agents that also read .claude/skills/
  const sharedAgents = new Set(
    Object.entries(agents)
      .filter(([, a]) => a.additionalSkillsDirs.some(d => d.includes('.claude/skills')))
      .map(([id]) => id),
  )

  // Group: agents that share skills vs agents with their own directory
  const sharedIds = candidateIds.filter(id => id === 'claude-code' || sharedAgents.has(id))
  const isolatedIds = candidateIds.filter(id => id !== 'claude-code' && !sharedAgents.has(id))

  const options: Array<{ label: string, value: AgentType | 'none', hint?: string }> = []

  // Show shared-compatible agents first
  if (sharedIds.length > 0 && isolatedIds.length > 0) {
    for (const id of sharedIds) {
      const a = agents[id]
      const hint = id === 'claude-code'
        ? `skills shared with ${sharedIds.length - 1} other agents`
        : `skills shared with Claude Code and others`
      options.push({ label: a.displayName, value: id as AgentType, hint })
    }
  }

  // Agents with isolated skill dirs
  const isolatedAgentIds = new Set(
    Object.entries(agents)
      .filter(([, a]) => a.additionalSkillsDirs.length === 0)
      .map(([id]) => id),
  )

  for (const id of (sharedIds.length > 0 && isolatedIds.length > 0 ? isolatedIds : candidateIds)) {
    if (options.some(o => o.value === id))
      continue
    const a = agents[id]
    const hint = sharedAgents.has(id) && id !== 'claude-code'
      ? 'skills shared with Claude Code and others'
      : isolatedAgentIds.has(id)
        ? 'skills only visible to this agent'
        : undefined
    options.push({ label: a.displayName, value: id as AgentType, hint })
  }

  options.push({ label: 'No agent', value: 'none', hint: 'export as standalone files for any AI' })

  if (!_warnedNoAgent) {
    _warnedNoAgent = true
    const hint = projectMatches.length > 1
      ? `Multiple agent directories found: ${projectMatches.map(t => agents[t].displayName).join(', ')}`
      : installed.length > 0
        ? `Found ${installed.map(t => agents[t].displayName).join(', ')} but couldn't determine which to use`
        : 'No agents auto-detected'
    const crossNote = sharedIds.length > 1
      ? `\n  \x1B[90mTip: Picking Claude Code shares skills with ${sharedIds.filter(id => id !== 'claude-code').map(id => agents[id].displayName).join(', ')} automatically.\x1B[0m`
      : ''
    p.log.warn(`${hint}\n  Pick the agent you actively code with.${crossNote}`)
  }

  const choice = await p.select({
    message: 'Which AI coding agent do you use?',
    options,
  })

  if (p.isCancel(choice))
    return null

  if (choice === 'none')
    return 'none'

  // Save as default so they don't get asked again
  updateConfig({ agent: choice })
  p.log.success(`Target agent set to ${agents[choice].displayName}`)
  return choice
}

/** Get installed LLM generators with working CLIs (verified via --version) */
export function getInstalledGenerators(): Array<{ name: string, version: string }> {
  const installed = detectInstalledAgents()
  return installed
    .filter(id => agents[id].cli)
    .map((id) => {
      const ver = getAgentVersion(id)
      return ver ? { name: agents[id].displayName, version: ver } : null
    })
    .filter((a): a is { name: string, version: string } => a !== null)
}

export function relativeTime(date: Date): string {
  const now = Date.now()
  const diff = now - date.getTime()
  const mins = Math.floor(diff / 60000)
  const hours = Math.floor(diff / 3600000)
  const days = Math.floor(diff / 86400000)
  if (mins < 1)
    return 'just now'
  if (mins < 60)
    return `${mins}m ago`
  if (hours < 24)
    return `${hours}h ago`
  return `${days}d ago`
}

export function getLastSynced(state: ProjectState): string | null {
  let latest: Date | null = null
  for (const skill of state.skills) {
    if (skill.info?.syncedAt) {
      const d = new Date(skill.info.syncedAt)
      if (!latest || d > latest)
        latest = d
    }
  }
  return latest ? relativeTime(latest) : null
}

export function introLine({ state, generators, modelId, agentId }: IntroOptions): string {
  const name = '\x1B[1m\x1B[35mskilld\x1B[0m'
  const ver = `\x1B[90mv${version}\x1B[0m`
  const lastSynced = getLastSynced(state)
  const synced = lastSynced ? ` · \x1B[90msynced ${lastSynced}\x1B[0m` : ''

  // Status line: enhancement model → target agent
  const parts: string[] = []
  if (modelId)
    parts.push(getModelName(modelId as any))
  else if (generators?.length)
    parts.push(generators.map(g => `${g.name} v${g.version}`).join(', '))
  if (agentId && agents[agentId as AgentType])
    parts.push(agents[agentId as AgentType].displayName)
  const statusLine = parts.length > 0
    ? `\n\x1B[90m↳ ${parts.join(' → ')}\x1B[0m`
    : ''

  return `${name} ${ver}${synced}${statusLine}`
}

export function formatStatus(synced: number, outdated: number): string {
  const parts: string[] = []
  if (synced > 0)
    parts.push(`\x1B[32m${synced} synced\x1B[0m`)
  if (outdated > 0)
    parts.push(`\x1B[33m${outdated} outdated\x1B[0m`)
  return `Skills: ${parts.join(' · ')}`
}

// ── Shared UI constants ───────────────────────────────────────────────

export const OAUTH_NOTE
  = 'Use an existing subscription (Claude Pro, ChatGPT Plus, Gemini)\n'
    + 'without an API key. You authenticate directly with the provider\n'
    + 'in your browser - no data leaves your machine.\n'
    + '\n'
    + 'A refresh token is stored locally at ~/.skilld/pi-ai-auth.json\n'
    + 'and used to call the provider API directly from your computer.\n'
    + '\x1B[90mOAuth handled by pi-ai, an open-source local client library:\n'
    + 'https://github.com/badlogic/pi-mono\x1B[0m'

export const NO_MODELS_MESSAGE = 'No enhancement models detected.\n'
  + '  \x1B[90mSkills work fine without this - you get raw docs, issues, and types.\n'
  + '  Enhancement compresses them into a concise cheat sheet with gotchas.\x1B[0m\n'
  + '\n'
  + '  To connect a model (optional):\n'
  + '  1. Connect a subscription via OAuth below (Claude Pro, ChatGPT Plus, Copilot, Gemini)\n'
  + '  2. Set an env var: ANTHROPIC_API_KEY, GEMINI_API_KEY, or OPENAI_API_KEY\n'
  + '  3. Install a CLI tool: \x1B[36mclaude\x1B[0m, \x1B[36mgemini\x1B[0m, or \x1B[36mcodex\x1B[0m (restart wizard after)'

/** Group models by vendor for provider→model selection. Uses vendorGroup to merge CLI and API entries under one heading. */
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
  /** Extra options prepended (e.g. Auto, Connect OAuth) */
  before?: Array<{ label: string, value: string, hint?: string }>
  /** Extra options appended (e.g. Skip) */
  after?: Array<{ label: string, value: string, hint?: string }>
}

/**
 * Smart provider→model picker. Skips the provider step when there's only 1 provider.
 * Returns the selected model value, or a sentinel string from before/after options.
 */
export async function pickModel<T extends { provider: string, providerName: string, name: string, id: string, hint: string, recommended?: boolean }>(
  models: T[],
  opts: ModelPickerOptions = {},
): Promise<string | null> {
  const byProvider = groupModelsByProvider(models)
  const before = opts.before ?? []
  const after = opts.after ?? []

  // Single provider → skip provider step, show models directly
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

  // Multiple providers or has before options - two-step
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

  // Check if it's a sentinel from before/after
  const providerStr = providerChoice as string
  if (before.some(o => o.value === providerStr) || after.some(o => o.value === providerStr))
    return providerStr

  // Drill into provider's models
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

/**
 * Check if the prepare hook is already installed in package.json.
 */
export function hasPrepareHook(cwd: string = process.cwd()): boolean {
  const pkg = readPackageJsonSafe(join(cwd, 'package.json'))
  if (!pkg)
    return true // no package.json means nothing to suggest
  const existing = (pkg.parsed.scripts as Record<string, unknown> | undefined)?.prepare
  return typeof existing === 'string' && existing.includes('skilld')
}

/**
 * Prompt to add `skilld prepare` to package.json "prepare" script.
 * In non-interactive environments, falls back to an info log.
 * Returns true if the hook was added or already present.
 */
export async function suggestPrepareHook(cwd: string = process.cwd()): Promise<boolean> {
  const pkgJsonPath = join(cwd, 'package.json')
  const pkg = readPackageJsonSafe(pkgJsonPath)
  if (!pkg)
    return false

  const rawExisting = (pkg.parsed.scripts as Record<string, unknown> | undefined)?.prepare
  const existing: string | undefined = typeof rawExisting === 'string' ? rawExisting : undefined

  if (existing?.includes('skilld'))
    return true

  const prepareCmd = buildPrepareScript(existing, cwd)

  if (!isInteractive()) {
    p.log.info(
      `\x1B[90mAdd to package.json scripts:\n`
      + `  \x1B[36m"prepare": "${prepareCmd}"\x1B[0m\n`
      + `  \x1B[90mRestores references and shipped skills on install.\x1B[0m`,
    )
    return false
  }

  const confirmed = await p.confirm({
    message: `Add \x1B[36m"prepare": "${prepareCmd}"\x1B[0m to package.json?`,
    initialValue: true,
  })
  if (p.isCancel(confirmed) || !confirmed)
    return false

  patchPackageJson(pkgJsonPath, (content) => {
    const tree = parseTree(content)
    const hasScripts = tree?.children?.some(c =>
      c.type === 'property' && c.children?.[0]?.value === 'scripts',
    )

    let patched = content
    if (!hasScripts)
      patched = editJsonProperty(patched, ['scripts'], {})

    return editJsonProperty(patched, ['scripts', 'prepare'], prepareCmd)
  })
  p.log.success('Added \x1B[36mskilld prepare\x1B[0m to package.json')
  return true
}

/**
 * Build the full prepare script value, safely appending to any existing command.
 */
export function buildPrepareScript(existing: string | undefined, cwd: string = process.cwd()): string {
  const bin = isSkilldInstalled(cwd) ? 'skilld' : 'npx skilld'
  const cmd = `${bin} prepare || true`
  if (!existing || !existing.trim())
    return cmd

  const trimmed = existing.trim()

  // Strip trailing && or ; that would leave a dangling operator
  const cleaned = trimmed.replace(/[&|;]+\s*$/, '').trim()
  if (!cleaned)
    return cmd

  return `${cleaned} && (${cmd})`
}

/**
 * Check if skilld is listed as a dependency (dev or regular) in the project's package.json.
 */
function isSkilldInstalled(cwd: string): boolean {
  const pkg = readPackageJsonSafe(join(cwd, 'package.json'))
  if (!pkg)
    return false
  const deps = pkg.parsed as Record<string, any>
  return !!(deps.dependencies?.skilld || deps.devDependencies?.skilld)
}

export function getRepoHint(name: string, cwd: string): string | undefined {
  const result = readPackageJsonSafe(join(cwd, 'node_modules', name, 'package.json'))
  if (!result)
    return undefined
  const pkg = result.parsed as Record<string, any>
  const url = typeof pkg.repository === 'string'
    ? pkg.repository
    : pkg.repository?.url
  if (!url)
    return undefined
  return url
    .replace(/^git\+/, '')
    .replace(/\.git$/, '')
    .replace(/^git:\/\//, 'https://')
    .replace(/^ssh:\/\/git@github\.com/, 'https://github.com')
    .replace(/^https?:\/\/(www\.)?github\.com\//, '')
}
