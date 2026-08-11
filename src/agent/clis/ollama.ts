/**
 * Ollama executor — one-shot local completions via Ollama's `/api/chat`.
 *
 * Unlike the CLI adapters (claude/codex/gemini) this is NOT an agentic loop:
 * no tools, no file exploration, no multi-turn transcript. The caller supplies
 * the full prompt and we return the single completion. That makes it free
 * (local) and cheap on tokens — a good fit for guide synthesis where all the
 * source material is already inlined into the prompt.
 *
 * Ollama has no first-class provider in pi-ai, and pi's OpenAI-compatible path
 * can't set Ollama's `num_ctx` option — the knob that prevents silent prompt
 * truncation below — so we talk to `/api/chat` directly.
 *
 * Model ids are `ollama:<name>`, e.g. `ollama:qwen2.5:14b-instruct`. The host
 * defaults to http://localhost:11434, override with `OLLAMA_HOST`.
 */

import type { SectionExecutor } from './runner.ts'
import type { OptimizeModel } from './types.ts'
import { ollamaHost } from '../../core/ollama-host.ts'

const OLLAMA_PREFIX = 'ollama:'

export function isOllamaModel(model: string): boolean {
  return model.startsWith(OLLAMA_PREFIX)
}

/** Parse `ollama:qwen2.5:14b-instruct` → `qwen2.5:14b-instruct`. */
export function parseOllamaModelId(model: string): string | null {
  return isOllamaModel(model) ? model.slice(OLLAMA_PREFIX.length) : null
}

interface OllamaChatChunk {
  message?: { content?: string }
  done?: boolean
  prompt_eval_count?: number
  eval_count?: number
  error?: string
}

export interface OllamaModelInfo {
  id: OptimizeModel
  name: string
  hint: string
}

interface OllamaTagsResponse {
  models?: Array<{
    name: string
    size?: number
    details?: { parameter_size?: string, quantization_level?: string }
  }>
}

interface OllamaShowResponse {
  capabilities?: string[]
}

/**
 * Whether a model can generate text. `/api/tags` can't tell chat models from
 * embedding-only ones (both report e.g. family `gemma3`), and an embedding
 * model 500s on `/api/chat` — so probe `/api/show` for capabilities. Fail open:
 * a probe error or an older Ollama that omits `capabilities` keeps the model.
 */
async function isCompletionModel(name: string): Promise<boolean> {
  const res = await fetch(`${ollamaHost()}/api/show`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ model: name }),
    signal: AbortSignal.timeout(1500),
  }).catch(() => null)
  if (!res?.ok)
    return true

  const data = await res.json().catch(() => null) as OllamaShowResponse | null
  const caps = data?.capabilities
  return !caps?.length || caps.includes('completion') || caps.includes('insert')
}

/**
 * Enumerate locally-pulled, text-capable Ollama models via `/api/tags`. Returns
 * `[]` when the daemon is unreachable (not installed / not running) — discovery
 * must never block or throw, it just contributes nothing to the model list.
 */
export async function getAvailableOllamaModels(): Promise<OllamaModelInfo[]> {
  const res = await fetch(`${ollamaHost()}/api/tags`, { signal: AbortSignal.timeout(1500) })
    .catch(() => null)
  if (!res?.ok)
    return []

  const data = await res.json().catch(() => null) as OllamaTagsResponse | null
  if (!data?.models?.length)
    return []

  const checked = await Promise.all(data.models.map(async (m): Promise<OllamaModelInfo | null> => {
    if (!(await isCompletionModel(m.name)))
      return null
    const params = m.details?.parameter_size
    const quant = m.details?.quantization_level
    const sizeGb = m.size ? `${(m.size / 1e9).toFixed(1)}GB` : undefined
    const detail = params ? `${params}${quant ? ` ${quant}` : ''}` : sizeGb
    return {
      id: `ollama:${m.name}`,
      name: m.name,
      hint: detail ? `local · ${detail}` : 'local',
    }
  }))

  return checked.filter((m): m is OllamaModelInfo => m !== null)
}

export function ollamaExecutor(model: OptimizeModel): SectionExecutor | { error: string } {
  const modelId = parseOllamaModelId(model)
  if (!modelId)
    return { error: `Not an Ollama model: ${model}` }

  return {
    cliCleanup: false,
    run: async ({ section, prompt, timeout, onProgress }) => {
      const ac = new AbortController()
      const timer = setTimeout(() => ac.abort(), timeout)
      onProgress?.({ chunk: '[ollama]', type: 'reasoning', text: '', reasoning: 'Generating locally…', section })

      const res = await fetch(`${ollamaHost()}/api/chat`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          model: modelId,
          messages: [{ role: 'user', content: prompt }],
          stream: true,
          // Ollama defaults num_ctx to ~4k, which silently truncates our
          // ~16k-token prompt (release notes) and starves synthesis. Size it to
          // hold the full material + output; override with OLLAMA_NUM_CTX.
          options: { temperature: 0.2, num_ctx: Number(process.env.OLLAMA_NUM_CTX) || 32768 },
        }),
        signal: ac.signal,
      }).catch((err: Error) => ({ ok: false, statusText: err.message, body: null } as unknown as Response))

      if (!res.ok || !res.body) {
        clearTimeout(timer)
        return {
          text: '',
          stderr: `Ollama request failed: ${res.statusText}. Is \`ollama serve\` running at ${ollamaHost()}?`,
          exitCode: 1,
        }
      }

      let text = ''
      let usage: { input: number, output: number } | undefined

      // Ollama streams NDJSON: one JSON object per line, the final one carrying
      // `done: true` plus token counts. Accumulate content and surface deltas.
      const drain = async (): Promise<void> => {
        const reader = res.body!.getReader()
        const decoder = new TextDecoder()
        let buffer = ''
        for (;;) {
          const { done, value } = await reader.read()
          if (done)
            break
          buffer += decoder.decode(value, { stream: true })
          const lines = buffer.split('\n')
          buffer = lines.pop() || ''
          for (const line of lines) {
            if (!line.trim())
              continue
            const chunk = JSON.parse(line) as OllamaChatChunk
            if (chunk.error)
              throw new Error(chunk.error)
            const delta = chunk.message?.content
            if (delta) {
              text += delta
              onProgress?.({ chunk: delta, type: 'text', text, reasoning: '', section })
            }
            if (chunk.done)
              usage = { input: chunk.prompt_eval_count ?? 0, output: chunk.eval_count ?? 0 }
          }
        }
      }

      const streamError = await drain().then(() => undefined).catch((err: Error) => err.message)
      clearTimeout(timer)

      if (streamError)
        return { text: '', stderr: `Ollama stream error: ${streamError}`, exitCode: 1 }
      if (!text.trim())
        return { text: '', stderr: 'Ollama returned no content', exitCode: 1 }

      return { text: text.trim(), usage, cost: 0 }
    },
  }
}
