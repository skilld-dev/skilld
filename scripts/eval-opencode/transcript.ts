// Turns the JSON event stream of `opencode run --format json` into the facts
// graders and the friction report read. Pure: no file or process access.

export interface ToolCall {
  /** opencode's tool name: `bash`, `skill`, or `<server>_<tool>` for MCP. */
  tool: string
  input: Record<string, unknown>
  output: string
  status: string
  /** Shell exit code. Null for a tool that is not a shell. */
  exit: number | null
  error: string | null
}

export interface Transcript {
  lastMessage: string
  toolCalls: ToolCall[]
  /** Every text part and tool call in order, for `trace` graders and the LLM judge. */
  trace: string
  tokens: number
  cost: number
  /** Lines the stream holds that are not JSON events, such as a crash message. */
  stray: string[]
}

export interface SkilldError {
  code: string
  message: string
  nextStep: string | null
}

export interface Friction {
  failedCommands: { command: string, exit: number | null, outputTail: string }[]
  skilldErrors: SkilldError[]
  nextSteps: string[]
  repeatedCommands: { command: string, times: number }[]
  toolErrors: { tool: string, error: string }[]
}

function record(value: unknown): Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value) ? value as Record<string, unknown> : {}
}

function tail(value: string, max = 300): string {
  return value.length > max ? `...${value.slice(-max)}` : value
}

export function parseTranscript(jsonl: string): Transcript {
  const toolCalls: ToolCall[] = []
  const trace: string[] = []
  const stray: string[] = []
  // Text parts of the current step. The last step's text is the closing message.
  let stepText: string[] = []
  let lastText: string[] = []
  let tokens = 0
  let cost = 0

  for (const line of jsonl.split('\n')) {
    if (!line.trim())
      continue
    let event: Record<string, unknown>
    try {
      event = record(JSON.parse(line))
    }
    catch {
      stray.push(line)
      continue
    }
    const part = record(event.part)
    switch (event.type) {
      case 'step_start':
        stepText = []
        break
      case 'text': {
        const value = typeof part.text === 'string' ? part.text : ''
        stepText.push(value)
        lastText = stepText
        trace.push(value)
        break
      }
      case 'tool_use': {
        const state = record(part.state)
        const metadata = record(state.metadata)
        const call: ToolCall = {
          tool: typeof part.tool === 'string' ? part.tool : 'unknown',
          input: record(state.input),
          output: typeof state.output === 'string' ? state.output : '',
          status: typeof state.status === 'string' ? state.status : 'unknown',
          exit: typeof metadata.exit === 'number' ? metadata.exit : null,
          error: typeof state.error === 'string' ? state.error : null,
        }
        toolCalls.push(call)
        trace.push(`[tool ${call.tool}] ${JSON.stringify(call.input)}\n${tail(call.error ?? call.output, 2000)}`)
        break
      }
      case 'step_finish': {
        tokens += typeof record(part.tokens).total === 'number' ? record(part.tokens).total as number : 0
        cost += typeof part.cost === 'number' ? part.cost : 0
        break
      }
      default:
        trace.push(`[${String(event.type)}] ${JSON.stringify(part).slice(0, 500)}`)
    }
  }

  return { lastMessage: lastText.join('\n').trim(), toolCalls, trace: trace.join('\n\n'), tokens, cost, stray }
}

/** The command a shell call ran. Graders match shell input against this string. */
export function commandOf(call: ToolCall): string | null {
  return call.tool === 'bash' && typeof call.input.command === 'string' ? call.input.command : null
}

const NEXT_STEP = /^Next step:\s*(.+)$/gm

/** skilld prints one JSON document per command with `--json`. A failure has `_tag: "OperationError"`. */
export function skilldErrorsIn(output: string): SkilldError[] {
  const errors: SkilldError[] = []
  for (const line of output.split('\n')) {
    if (!line.includes('"OperationError"'))
      continue
    try {
      const error = record(record(JSON.parse(line)).error)
      errors.push({
        code: typeof error.code === 'string' ? error.code : 'UNKNOWN',
        message: typeof error.message === 'string' ? error.message : '',
        nextStep: typeof error.nextStep === 'string' ? error.nextStep : null,
      })
    }
    catch {
      // A line that only mentions the tag is not a skilld answer. The failed
      // command list still records the exit code.
    }
  }
  return errors
}

export function friction(transcript: Transcript): Friction {
  const failedCommands: Friction['failedCommands'] = []
  const skilldErrors: SkilldError[] = []
  const nextSteps: string[] = []
  const toolErrors: Friction['toolErrors'] = []
  const runs = new Map<string, number>()

  for (const call of transcript.toolCalls) {
    const command = commandOf(call)
    if (command !== null) {
      runs.set(command, (runs.get(command) ?? 0) + 1)
      if ((call.exit !== null && call.exit !== 0) || call.status === 'error')
        failedCommands.push({ command, exit: call.exit, outputTail: tail(call.error ?? call.output) })
      skilldErrors.push(...skilldErrorsIn(call.output))
      nextSteps.push(...[...call.output.matchAll(NEXT_STEP)].map(m => m[1]!.trim()))
    }
    else if (call.status === 'error') {
      toolErrors.push({ tool: call.tool, error: tail(call.error ?? call.output) })
    }
  }

  const repeatedCommands = [...runs].filter(([, times]) => times > 1).map(([command, times]) => ({ command, times }))
  return { failedCommands, skilldErrors, nextSteps, repeatedCommands, toolErrors }
}
