/**
 * CLI orchestrator — spawns per-CLI processes for skill generation
 * Each CLI (claude, gemini, codex) has its own buildArgs + parseLine in separate files
 */

import type { AgentType } from '../types.ts'
import type { CliAdapter, CliModelConfig, CliName, OptimizeModel } from './types.ts'
import { exec } from 'node:child_process'
import { promisify } from 'node:util'
import { isWindows } from 'std-env'
import { detectInstalledAgents } from '../detect.ts'
import { agents } from '../targets/index.ts'
import { adapter as claudeAdapter } from './claude.ts'
import { adapter as codexAdapter } from './codex.ts'
import { adapter as geminiAdapter } from './gemini.ts'
import { getAvailableOllamaModels, isOllamaModel, parseOllamaModelId } from './ollama.ts'
import { getAvailablePiAiModels, isPiAiModel, parsePiAiModelId } from './pi-ai.ts'

export { buildAllSectionPrompts, buildSectionPrompt, SECTION_MERGE_ORDER, SECTION_OUTPUT_FILES } from '../prompts/index.ts'
export type { CustomPrompt, SkillSection } from '../prompts/index.ts'
export { cleanSectionOutput } from './clean-output.ts'
export { createToolProgress } from './cli-progress.ts'
export type { CliModelConfig, CliName, ModelInfo, OptimizeDocsOptions, OptimizeModel, OptimizeResult, StreamProgress } from './types.ts'

// ── Per-CLI dispatch ─────────────────────────────────────────────────

/** Single source of truth: adding a new CLI adapter here is the only edit needed in this file. */
export const CLI_ADAPTERS: Record<CliName, CliAdapter> = {
  claude: claudeAdapter,
  gemini: geminiAdapter,
  codex: codexAdapter,
}

const CLI_PROVIDER_NAMES: Record<string, string> = Object.fromEntries(
  Object.values(CLI_ADAPTERS).map(a => [a.agentId, a.providerName]),
)

const PI_PROVIDER_NAMES: Record<string, string> = {
  'anthropic': 'Anthropic',
  'google': 'Google',
  'google-antigravity': 'Antigravity',
  'google-gemini-cli': 'Google Gemini',
  'google-vertex': 'Google Vertex',
  'openai': 'OpenAI',
  'openai-codex': 'OpenAI Codex',
  'github-copilot': 'GitHub Copilot',
  'groq': 'Groq',
  'mistral': 'Mistral',
  'xai': 'xAI',
}

// ── Assemble CLI_MODELS from per-CLI model definitions ───────────────

export const CLI_MODELS: Partial<Record<OptimizeModel, CliModelConfig>> = Object.fromEntries(
  Object.values(CLI_ADAPTERS).flatMap(adapter =>
    Object.entries(adapter.models).map(([id, entry]) => [
      id,
      { ...entry, cli: adapter.cli, agentId: adapter.agentId },
    ]),
  ),
)

// ── Model helpers ────────────────────────────────────────────────────

export function getModelName(id: OptimizeModel): string {
  if (isOllamaModel(id))
    return parseOllamaModelId(id) ?? id
  if (isPiAiModel(id)) {
    const parsed = parsePiAiModelId(id)
    return parsed?.modelId ?? id
  }
  return CLI_MODELS[id]?.name ?? id
}

export function getModelLabel(id: OptimizeModel): string {
  if (isOllamaModel(id))
    return `Ollama · ${parseOllamaModelId(id) ?? id}`
  if (isPiAiModel(id)) {
    const parsed = parsePiAiModelId(id)
    return parsed ? `${PI_PROVIDER_NAMES[parsed.provider] ?? parsed.provider} · ${parsed.modelId}` : id
  }
  const config = CLI_MODELS[id]
  if (!config)
    return id
  const providerName = CLI_PROVIDER_NAMES[config.agentId] ?? config.cli
  return `${providerName} · ${config.name}`
}

export async function getAvailableModels(): Promise<import('./types.ts').ModelInfo[]> {
  const execAsync = promisify(exec)
  const lookupCmd = isWindows ? 'where' : 'which'

  // Kick off Ollama discovery concurrently with the CLI lookups below; it has
  // its own short timeout and resolves to [] when the daemon is unreachable.
  const ollamaModelsPromise = getAvailableOllamaModels()

  const installedAgents = detectInstalledAgents()
  const agentsWithCli = installedAgents.filter(id => agents[id].cli)

  const cliChecks = await Promise.all(
    agentsWithCli.map(async (agentId) => {
      const cli = agents[agentId].cli!
      try {
        await execAsync(`${lookupCmd} ${cli}`)
        return agentId
      }
      catch { return null }
    }),
  )
  const availableAgentIds = new Set(cliChecks.filter((id): id is AgentType => id != null))

  const cliModels = (Object.entries(CLI_MODELS) as [OptimizeModel, CliModelConfig][])
    .filter(([_, config]) => availableAgentIds.has(config.agentId))
    .map(([id, config]) => {
      const providerName = CLI_PROVIDER_NAMES[config.agentId] ?? agents[config.agentId]?.displayName ?? config.agentId
      return {
        id,
        name: config.name,
        hint: config.hint,
        recommended: config.recommended,
        agentId: config.agentId,
        agentName: providerName,
        provider: config.agentId,
        providerName: `${providerName} (via ${config.cli} CLI)`,
        vendorGroup: providerName,
      }
    })

  // Append pi-ai direct API models (providers with auth configured)
  const piAiModels = getAvailablePiAiModels()
  const piAiEntries = piAiModels.map((m) => {
    const parsed = parsePiAiModelId(m.id)
    const piProvider = parsed?.provider ?? 'pi-ai'
    const displayName = PI_PROVIDER_NAMES[piProvider] ?? piProvider
    const authLabel = m.authSource === 'env' ? 'API' : 'OAuth'
    return {
      id: m.id as OptimizeModel,
      name: m.name,
      hint: m.hint,
      recommended: m.recommended,
      agentId: 'pi-ai',
      agentName: `pi-ai (${m.authSource})`,
      provider: `pi:${piProvider}:${m.authSource}`,
      providerName: `${displayName} (${authLabel})`,
      vendorGroup: displayName,
    }
  })

  // Append locally-pulled Ollama models (one-shot, no auth, free).
  const ollamaModels = await ollamaModelsPromise
  const ollamaEntries = ollamaModels.map(m => ({
    id: m.id,
    name: m.name,
    hint: m.hint,
    agentId: 'ollama',
    agentName: 'Ollama (local)',
    provider: 'ollama',
    providerName: 'Ollama (local)',
    vendorGroup: 'Ollama',
  }))

  return [...cliModels, ...piAiEntries, ...ollamaEntries]
}
