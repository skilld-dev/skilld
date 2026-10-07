// Scores one run against its graders. Pure: the caller supplies file reads
// and the LLM judge's verdicts, so every rule here is unit-testable.
import type { Grader } from './case.ts'
import type { Transcript } from './transcript.ts'
import { commandOf } from './transcript.ts'

export type GraderOutcome
  = | { _tag: 'Scored', score: number, detail: string }
    | { _tag: 'Skipped', reason: string }

export interface GradedGrader {
  name: string
  weight: number
  /** A `with-only` grader shows the Skill fired. It never counts toward the score. */
  indicator: boolean
  outcome: GraderOutcome
}

export interface JudgeVerdict {
  pass: boolean
  reason: string
}

export interface GradeContext {
  transcript: Transcript
  readProjectFile: (path: string) => string | null
  /** Verdicts keyed by grader name. A missing verdict skips that LLM grader. */
  verdicts: ReadonlyMap<string, JudgeVerdict>
  arm: 'with' | 'without'
}

const CLAUDE_TOOLS: Readonly<Record<string, string>> = {
  Bash: 'bash',
  Skill: 'skill',
  Read: 'read',
  Write: 'write',
  Edit: 'edit',
  Glob: 'glob',
  Grep: 'grep',
  WebFetch: 'webfetch',
}

/** Cases name tools as Claude Code does. `mcp__skilld__search_skills` is `skilld_search_skills` in opencode. */
export function opencodeToolName(name: string): string {
  const mcp = /^mcp__(.+?)__(.+)$/.exec(name)
  if (mcp)
    return `${mcp[1]}_${mcp[2]}`
  return CLAUDE_TOOLS[name] ?? name.toLowerCase()
}

function scored(pass: boolean, detail: string): GraderOutcome {
  return { _tag: 'Scored', score: pass ? 1 : 0, detail }
}

function compile(pattern: string, flags: string): RegExp | string {
  try {
    return new RegExp(pattern, flags)
  }
  catch (error) {
    return error instanceof Error ? error.message : String(error)
  }
}

function gradeOne(grader: Grader, ctx: GradeContext): GraderOutcome {
  switch (grader._tag) {
    case 'unsupported':
      return { _tag: 'Skipped', reason: grader.reason }
    case 'regex': {
      const regex = compile(grader.pattern, grader.flags)
      if (typeof regex === 'string')
        return { _tag: 'Skipped', reason: `invalid pattern: ${regex}` }
      const subject = grader.target._tag === 'last_message'
        ? ctx.transcript.lastMessage
        : grader.target._tag === 'trace' ? ctx.transcript.trace : ctx.readProjectFile(grader.target.path)
      if (subject === null)
        return scored(grader.match === 'not_contains', 'file is missing')
      const found = regex.test(subject)
      return scored(grader.match === 'contains' ? found : !found, found ? 'pattern found' : 'pattern absent')
    }
    case 'tool_used': {
      const tool = opencodeToolName(grader.tool)
      const inputMatch = grader.inputMatch === null ? null : compile(grader.inputMatch, '')
      if (typeof inputMatch === 'string')
        return { _tag: 'Skipped', reason: `invalid input_match: ${inputMatch}` }
      const calls = ctx.transcript.toolCalls.filter((call) => {
        if (call.tool !== tool)
          return false
        if (!inputMatch)
          return true
        return inputMatch.test(commandOf(call) ?? JSON.stringify(call.input))
      }).length
      const pass = calls >= grader.min && (grader.max === null || calls <= grader.max)
      return scored(pass, `${calls} matching ${tool} call${calls === 1 ? '' : 's'}`)
    }
    case 'file_exists': {
      const exists = ctx.readProjectFile(grader.path) !== null
      return scored(exists === grader.exists, exists ? 'file exists' : 'file is missing')
    }
    case 'llm': {
      const verdict = ctx.verdicts.get(grader.name)
      if (!verdict)
        return { _tag: 'Skipped', reason: 'no judge verdict' }
      return scored(verdict.pass, verdict.reason)
    }
  }
}

export function gradeRun(graders: readonly Grader[], ctx: GradeContext): GradedGrader[] {
  return graders
    .filter(grader => grader._tag === 'unsupported' || grader.arm === 'both' || ctx.arm === 'with')
    .map(grader => ({
      name: grader.name,
      weight: grader._tag === 'unsupported' ? 0 : grader.weight,
      indicator: grader._tag !== 'unsupported' && grader.arm === 'with-only',
      outcome: gradeOne(grader, ctx),
    }))
}

/** Weighted mean of scored, non-indicator graders. Null when nothing was scored. */
export function runScore(graded: readonly GradedGrader[]): number | null {
  let total = 0
  let weight = 0
  for (const g of graded) {
    if (g.indicator || g.outcome._tag !== 'Scored')
      continue
    total += g.outcome.score * g.weight
    weight += g.weight
  }
  return weight === 0 ? null : total / weight
}

/** Reads the judge's reply. It must hold one JSON object with a boolean `pass`. */
export function parseVerdict(reply: string): JudgeVerdict | null {
  const match = /\{[\s\S]*\}/.exec(reply)
  if (!match)
    return null
  try {
    const value = JSON.parse(match[0]) as Record<string, unknown>
    if (typeof value.pass !== 'boolean')
      return null
    return { pass: value.pass, reason: typeof value.reason === 'string' ? value.reason : '' }
  }
  catch {
    return null
  }
}
