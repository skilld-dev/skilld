import type { FeaturesConfig } from '../../core/config.ts'
import type { CustomPrompt, SkillSection } from '../prompts/index.ts'
import type { AgentType } from '../types.ts'

/** Normalized event emitted by every CliAdapter from a single stream-json line. */
export type CliEvent
  = | { kind: 'noop' }
  /** Assistant text — delta (partial streaming) or full (turn-level). */
    | { kind: 'text', delta?: string, full?: string }
    /**
     * Tool invocation. `tool` is the adapter's raw name (e.g. 'Read', 'read_file'); look it up in
     * TOOL_NAMES for display verb + canonical role. `writeContent` carries inline file content
     * the LLM tried to write (fallback path when the host denies the Write tool).
     */
    | { kind: 'tool-call', tool: string, hint?: string, writeContent?: string }
    /** Stream finished cleanly. */
    | { kind: 'done', usage?: { input: number, output: number }, cost?: number, turns?: number }
    /** Stream finished with an error. */
    | { kind: 'error', message?: string }

/**
 * Canonical tool registry — one row per raw name across adapters. Drives display (verb) and
 *  semantic role (canonical) without per-CLI switching in the dispatcher.
 */
export const TOOL_NAMES: Record<string, { canonical: 'read' | 'search' | 'write' | 'list' | 'shell', verb: string }> = {
  // Claude Code
  Read: { canonical: 'read', verb: 'Reading' },
  Glob: { canonical: 'search', verb: 'Searching' },
  Grep: { canonical: 'search', verb: 'Searching' },
  Write: { canonical: 'write', verb: 'Writing' },
  Bash: { canonical: 'shell', verb: 'Running' },
  // Gemini
  read_file: { canonical: 'read', verb: 'Reading' },
  glob_tool: { canonical: 'search', verb: 'Searching' },
  write_file: { canonical: 'write', verb: 'Writing' },
  list_directory: { canonical: 'list', verb: 'Listing' },
  search_file_content: { canonical: 'search', verb: 'Searching' },
  run_shell_command: { canonical: 'shell', verb: 'Running' },
}

/**
 * Common keys adapters poke at to extract a useful hint (path, query, command) from a tool's
 *  argument object — claude, gemini, codex all use overlapping shapes.
 */
export function extractToolHint(input: Record<string, unknown> | undefined | null): string | undefined {
  if (!input)
    return undefined
  for (const k of ['file_path', 'path', 'dir_path', 'pattern', 'query', 'command'] as const) {
    const v = input[k]
    if (typeof v === 'string' && v)
      return v
  }
  return undefined
}

export type OptimizeModel
  = | 'opus'
    | 'sonnet'
    | 'haiku'
    | 'gemini-3.1-pro'
    | 'gemini-3-flash'
    | 'gpt-5.3-codex'
    | 'gpt-5.3-codex-spark'
    | 'gpt-5.2-codex'
    // pi-ai direct API models — dynamic from pi-ai's model registry
    | `pi:${string}`
    // Local Ollama models, one-shot completions — e.g. `ollama:qwen2.5:14b-instruct`
    | `ollama:${string}`

export interface ModelInfo {
  id: OptimizeModel
  name: string
  hint: string
  recommended?: boolean
  agentId: string
  agentName: string
  /** Grouping key for provider selection (e.g. 'claude-code', 'pi:anthropic') */
  provider: string
  /** Human-readable provider name */
  providerName: string
  /** Normalized vendor name for UI grouping (e.g. 'Anthropic') — merges CLI and API entries */
  vendorGroup: string
}

export interface StreamProgress {
  chunk: string
  type: 'reasoning' | 'text'
  text: string
  reasoning: string
  section?: SkillSection
}

export interface OptimizeDocsOptions {
  packageName: string
  skillDir: string
  model?: OptimizeModel
  version?: string
  hasGithub?: boolean
  hasReleases?: boolean
  hasChangelog?: string | false
  docFiles?: string[]
  docsType?: 'llms.txt' | 'readme' | 'docs'
  hasShippedDocs?: boolean
  onProgress?: (progress: StreamProgress) => void
  timeout?: number
  verbose?: boolean
  debug?: boolean
  noCache?: boolean
  /** Which sections to generate */
  sections?: SkillSection[]
  /** Custom instructions from the user */
  customPrompt?: CustomPrompt
  /** Resolved feature flags */
  features?: FeaturesConfig
  /** Key files from the package (e.g., dist/pkg.d.ts) */
  pkgFiles?: string[]
  /** Lines consumed by SKILL.md overhead (frontmatter + header + search + footer) */
  overheadLines?: number
}

export interface OptimizeResult {
  optimized: string
  wasOptimized: boolean
  error?: string
  warnings?: string[]
  reasoning?: string
  finishReason?: string
  usage?: { inputTokens: number, outputTokens: number, totalTokens: number }
  cost?: number
  debugLogsDir?: string
}

export interface SectionResult {
  section: SkillSection
  content: string
  wasOptimized: boolean
  error?: string
  warnings?: ValidationWarning[]
  usage?: { input: number, output: number }
  cost?: number
}

export interface ValidationWarning {
  section: string
  warning: string
}

/** Per-model config without redundant cli/agentId (those come from the CLI file) */
export interface CliModelEntry {
  /** Model flag passed to the CLI */
  model: string
  /** Human-readable model name */
  name: string
  /** Short description hint */
  hint: string
  /** Whether this is the recommended model for this CLI */
  recommended?: boolean
}

/** Full model config (assembled from CLI files + their models) */
export interface CliModelConfig extends CliModelEntry {
  cli: CliName
  agentId: AgentType
}

export type CliName = 'claude' | 'gemini' | 'codex'

/**
 * Per-CLI integration. Adding a new LLM CLI is one new file exporting one of these — no edits
 * to the dispatcher, the model table, or the provider name table.
 */
export interface CliAdapter {
  cli: CliName
  agentId: AgentType
  /** Human-readable LLM provider name (e.g. 'Anthropic', 'OpenAI'). */
  providerName: string
  /** Models this adapter can target, keyed by OptimizeModel id. */
  models: Record<string, CliModelEntry>
  /** Build argv for spawning the CLI process. */
  buildArgs: (model: string, skillDir: string, symlinkDirs: string[]) => string[]
  /** Parse one stream-json line into a normalized event. */
  parseEvent: (line: string) => CliEvent
}
