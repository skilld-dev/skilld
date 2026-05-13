/**
 * pi-ai section runner — drives the agentic tool-use loop for a single SKILL.md
 * section. Streams reasoning + text to onProgress; tools are sandboxed to .skilld/.
 */

import type { AssistantMessage, Message, ToolCall } from '@earendil-works/pi-ai'
import type { SkillSection } from '../prompts/index.ts'
import type { StreamProgress } from './types.ts'
import { getModel, streamSimple } from '@earendil-works/pi-ai'
import { skillInternalDir } from '../../core/paths.ts'
import { resolveApiKey } from './pi-ai-auth.ts'
import { parsePiAiModelId } from './pi-ai-models.ts'
import { executeTool, MAX_TOOL_TURNS, TOOLS } from './pi-ai-tools.ts'

export interface PiAiSectionOptions {
  section: SkillSection
  prompt: string
  skillDir: string
  model: string
  onProgress?: (progress: StreamProgress) => void
  signal?: AbortSignal
}

export interface PiAiSectionResult {
  text: string
  /** The raw prompt sent to the model. */
  fullPrompt: string
  usage?: { input: number, output: number }
  cost?: number
}

const SYSTEM_PROMPT = 'You are a technical documentation expert generating SKILL.md sections for AI agent skills. Follow the format instructions exactly. Use the provided tools to explore reference files in ./.skilld/ before writing your output.'

/** Optimize a single section using pi-ai agentic API with tool use. */
export async function optimizeSectionPiAi(opts: PiAiSectionOptions): Promise<PiAiSectionResult> {
  const parsed = parsePiAiModelId(opts.model)
  if (!parsed)
    throw new Error(`Invalid pi-ai model ID: ${opts.model}. Expected format: pi:provider/model-id`)

  const model = getModel(parsed.provider as any, parsed.modelId as any)
  const apiKey = await resolveApiKey(parsed.provider)
  const skilldDir = skillInternalDir(opts.skillDir)

  const fullPrompt = opts.prompt

  opts.onProgress?.({ chunk: '[starting...]', type: 'reasoning', text: '', reasoning: '', section: opts.section })

  const messages: Message[] = [{
    role: 'user' as const,
    content: [{ type: 'text' as const, text: fullPrompt }],
    timestamp: Date.now(),
  }]

  let text = ''
  let completed = false
  let totalUsage: { input: number, output: number } | undefined
  let totalCost: number | undefined
  let lastWriteContent = ''

  for (let turn = 0; turn < MAX_TOOL_TURNS; turn++) {
    if (opts.signal?.aborted)
      throw new Error('pi-ai request timed out')

    const eventStream = streamSimple(model, {
      systemPrompt: SYSTEM_PROMPT,
      messages,
      tools: TOOLS,
    }, {
      reasoning: turn === 0 ? 'medium' : undefined,
      maxTokens: 16_384,
      ...(apiKey ? { apiKey } : {}),
    })

    let assistantMessage: AssistantMessage | undefined
    let turnText = ''

    for await (const event of eventStream) {
      if (opts.signal?.aborted)
        throw new Error('pi-ai request timed out')

      switch (event.type) {
        case 'text_delta':
          turnText += event.delta
          opts.onProgress?.({ chunk: event.delta, type: 'text', text: turnText, reasoning: '', section: opts.section })
          break
        case 'toolcall_end': {
          const tc = event.toolCall
          const hint = tc.name === 'Read' || tc.name === 'Write'
            ? `[${tc.name}: ${tc.arguments.path}]`
            : tc.name === 'Bash'
              ? `[${tc.name}: ${tc.arguments.command}]`
              : `[${tc.name}: ${tc.arguments.pattern}]`
          opts.onProgress?.({ chunk: hint, type: 'reasoning', text: '', reasoning: hint, section: opts.section })
          break
        }
        case 'done':
          assistantMessage = event.message
          break
        case 'error':
          throw new Error(event.error?.errorMessage ?? 'pi-ai stream error')
      }
    }

    if (!assistantMessage)
      throw new Error('pi-ai stream ended without a message')

    if (assistantMessage.usage) {
      if (totalUsage) {
        totalUsage.input += assistantMessage.usage.input
        totalUsage.output += assistantMessage.usage.output
      }
      else {
        totalUsage = { input: assistantMessage.usage.input, output: assistantMessage.usage.output }
      }
      totalCost = (totalCost ?? 0) + (assistantMessage.usage.cost?.total ?? 0)
    }

    messages.push(assistantMessage)

    const toolCalls = assistantMessage.content.filter((c): c is ToolCall => c.type === 'toolCall')
    if (toolCalls.length === 0) {
      text = turnText
      completed = true
      break
    }

    for (const tc of toolCalls) {
      const result = executeTool(tc, skilldDir)
      if (tc.name === 'Write')
        lastWriteContent = String(tc.arguments.content)
      messages.push({
        role: 'toolResult' as const,
        toolCallId: tc.id,
        toolName: tc.name,
        content: [{ type: 'text' as const, text: result }],
        isError: result.startsWith('Error:'),
        timestamp: Date.now(),
      })
    }
  }

  if (!completed)
    throw new Error(`pi-ai exceeded ${MAX_TOOL_TURNS} tool turns without completing`)

  // Prefer text output, fall back to last Write content.
  const finalText = text || lastWriteContent

  return { text: finalText, fullPrompt, usage: totalUsage, cost: totalCost }
}
