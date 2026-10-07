// Reads eval cases in the `claude plugin eval` format, so one case serves both
// runners. Every grader field here exists in that format's strict schema.
import { existsSync, readdirSync, readFileSync } from 'node:fs'
import { basename, join } from 'node:path'
import { parse } from 'yaml'

/** `with-only` graders show that a Skill fired. They are reported, not scored. */
export type Arm = 'both' | 'with-only'

export type RegexTarget
  = | { _tag: 'last_message' }
    | { _tag: 'trace' }
    | { _tag: 'file', path: string }

export type Grader
  = | { _tag: 'regex', name: string, target: RegexTarget, pattern: string, flags: string, match: 'contains' | 'not_contains', weight: number, arm: Arm }
    | { _tag: 'tool_used', name: string, tool: string, inputMatch: string | null, min: number, max: number | null, weight: number, arm: Arm }
    | { _tag: 'file_exists', name: string, path: string, exists: boolean, weight: number, arm: Arm }
    | { _tag: 'llm', name: string, criteria: string, focus: 'last_message' | 'trace', weight: number, arm: Arm }
    | { _tag: 'unsupported', name: string, reason: string }

export interface EvalCase {
  name: string
  description: string
  dir: string
  prompt: string
  maxTurns: number
  scaffoldScript: string | null
  graders: Grader[]
  /** `opencode.json` beside `case.yaml`: extra opencode config, such as an MCP server. */
  opencodeConfig: Record<string, unknown> | null
}

export type ParseResult<T> = { _tag: 'Ok', value: T } | { _tag: 'Err', message: string }

function record(value: unknown): Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value) ? value as Record<string, unknown> : {}
}

function text(value: unknown, fallback: string): string {
  return typeof value === 'string' ? value : fallback
}

function count(value: unknown, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback
}

function armOf(value: unknown): Arm {
  return value === 'with-only' ? 'with-only' : 'both'
}

/** Splits `---\nfrontmatter\n---\nbody`. A file without frontmatter is all body. */
export function splitFrontmatter(source: string): { frontmatter: Record<string, unknown>, body: string } {
  const match = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/.exec(source)
  if (!match)
    return { frontmatter: {}, body: source.trim() }
  return { frontmatter: record(parse(match[1]!)), body: match[2]!.trim() }
}

function regexTarget(value: unknown): RegexTarget | null {
  if (value === undefined || value === 'last_message')
    return { _tag: 'last_message' }
  if (value === 'trace')
    return { _tag: 'trace' }
  const source = record(value)
  if (source.source === 'file' && typeof source.path === 'string')
    return { _tag: 'file', path: source.path }
  return null
}

export function parseGrader(name: string, source: string): Grader {
  const { frontmatter: f, body } = splitFrontmatter(source)
  const weight = count(f.weight, 1)
  const arm = armOf(f.arm)
  switch (f.type) {
    case 'regex': {
      const target = regexTarget(f.target)
      const match = f.match ?? 'contains'
      if (!target)
        return { _tag: 'unsupported', name, reason: `regex target ${JSON.stringify(f.target)} is not supported` }
      if (match !== 'contains' && match !== 'not_contains')
        return { _tag: 'unsupported', name, reason: `regex match ${JSON.stringify(match)} is not supported` }
      return { _tag: 'regex', name, target, pattern: text(f.pattern, ''), flags: text(f.flags, ''), match, weight, arm }
    }
    case 'tool_used': {
      const max = typeof f.max === 'number' ? f.max : null
      // The format defaults `min` to 1. A grader with only `max` asks for an upper bound.
      const min = count(f.min, max === null ? 1 : 0)
      return { _tag: 'tool_used', name, tool: text(f.tool, ''), inputMatch: typeof f.input_match === 'string' ? f.input_match : null, min, max, weight, arm }
    }
    case 'file_exists':
      return { _tag: 'file_exists', name, path: text(f.path, ''), exists: f.exists !== false, weight, arm }
    case 'llm':
      return { _tag: 'llm', name, criteria: text(f.criteria, body), focus: f.focus === 'trace' ? 'trace' : 'last_message', weight, arm }
    default:
      return { _tag: 'unsupported', name, reason: `grader type ${JSON.stringify(f.type)} is not supported` }
  }
}

export function loadCase(dir: string): ParseResult<EvalCase> {
  const file = join(dir, 'case.yaml')
  const doc = record(parse(readFileSync(file, 'utf8')))
  const execution = record(doc.execution)
  const context = record(doc.context)
  const prompt = text(execution.prompt, '').trim()
  if (!prompt)
    return { _tag: 'Err', message: `${file} has no execution.prompt` }

  const gradersDir = join(dir, 'graders')
  const graders = existsSync(gradersDir)
    ? readdirSync(gradersDir).filter(f => f.endsWith('.md')).sort().map(f => parseGrader(basename(f, '.md'), readFileSync(join(gradersDir, f), 'utf8')))
    : []
  const configFile = join(dir, 'opencode.json')
  const scaffold = typeof context.scaffold_script === 'string' ? join(dir, context.scaffold_script) : null

  return {
    _tag: 'Ok',
    value: {
      name: text(doc.name, basename(dir)),
      description: text(doc.description, ''),
      dir,
      prompt,
      maxTurns: count(execution.max_turns, 30),
      scaffoldScript: scaffold,
      graders,
      opencodeConfig: existsSync(configFile) ? record(JSON.parse(readFileSync(configFile, 'utf8'))) : null,
    },
  }
}

/** Every directory below `root` that holds a `case.yaml`. */
export function findCaseDirs(root: string): string[] {
  if (!existsSync(root))
    return []
  return readdirSync(root, { withFileTypes: true })
    .filter(entry => entry.isDirectory() && existsSync(join(root, entry.name, 'case.yaml')))
    .map(entry => join(root, entry.name))
    .sort()
}
