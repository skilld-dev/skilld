/**
 * Section executors — concrete `SectionExecutor` impls and model→executor selection.
 *
 * Two adapters today:
 * - `cliExecutor` wraps `spawnCliAndStream` (claude/codex/gemini subprocess)
 * - `piAiExecutor` wraps `optimizeSectionPiAi` (in-process agent loop)
 *
 * Adding a new executor (e.g. raw Anthropic SDK) is a new factory plus a branch in
 * `selectExecutor`. The lifecycle in `llm-enhancer.optimizeSection` does not change.
 */

import type { SectionExecutor } from './runner.ts'
import type { OptimizeModel } from './types.ts'
import { getSkillReferenceDirs } from '../../cache/index.ts'
import { CLI_ADAPTERS, CLI_MODELS } from './index.ts'
import { isOllamaModel, ollamaExecutor } from './ollama.ts'
import { getAvailablePiAiModels, isPiAiModel, optimizeSectionPiAi } from './pi-ai.ts'
import { createPiAiModels } from './pi-ai-auth.ts'
import { spawnCliAndStream } from './runner.ts'

function cliExecutor(model: OptimizeModel): SectionExecutor | { error: string } {
  const cliConfig = CLI_MODELS[model]
  if (!cliConfig)
    return { error: `No CLI mapping for model: ${model}` }
  const adapter = CLI_ADAPTERS[cliConfig.cli]
  return {
    cliCleanup: true,
    run: ({ section, prompt, skillDir, skilldDir, timeout, debug, onProgress }) => spawnCliAndStream({
      adapter,
      cliModel: cliConfig.model,
      prompt,
      skillDir,
      skilldDir,
      symlinkDirs: getSkillReferenceDirs(skillDir),
      timeout,
      debug,
      section,
      onProgress,
    }),
  }
}

async function piAiExecutor(model: OptimizeModel): Promise<SectionExecutor | { error: string }> {
  const models = createPiAiModels()
  const available = new Set((await getAvailablePiAiModels(models)).map(m => m.id as OptimizeModel))
  if (!available.has(model))
    return { error: `Pi model unavailable or not authenticated: ${model}` }

  return {
    cliCleanup: false,
    run: async ({ section, prompt, skillDir, timeout, onProgress }) => {
      const ac = new AbortController()
      const timer = setTimeout(() => ac.abort(), timeout)
      try {
        const result = await optimizeSectionPiAi({ section, prompt, skillDir, model, onProgress, signal: ac.signal, models })
        return { text: result.text.trim(), usage: result.usage, cost: result.cost }
      }
      catch (err) {
        return { text: '', stderr: (err as Error).message, exitCode: 1 }
      }
      finally {
        clearTimeout(timer)
      }
    },
  }
}

/** Resolve `model` to an executor, or an error if the model is unavailable/unmapped. */
export async function selectExecutor(model: OptimizeModel): Promise<SectionExecutor | { error: string }> {
  if (isOllamaModel(model))
    return ollamaExecutor(model)
  return isPiAiModel(model) ? piAiExecutor(model) : cliExecutor(model)
}
