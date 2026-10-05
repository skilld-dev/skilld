/**
 * Skill behaviors: what a Skill's files ask an Agent to do.
 *
 * The skilld CLI and skilld.dev match Skill files against the same fixed rules
 * in `rules/skill-behaviors.json`. A match names the behavior and where it
 * appears. No match proves nothing: text patterns miss obfuscated code.
 *
 * The Rust matcher in `crates/skilld-core/src/behavior.rs` follows the same
 * semantics. The cases in `contracts/fixtures/skill-behaviors/cases.json` bind
 * both implementations.
 */

import { z } from 'zod'
import rawRules from '../rules/skill-behaviors.json' with { type: 'json' }

/** The locations one behavior keeps. `total` still counts every match. */
export const MAX_BEHAVIOR_LOCATIONS = 5

export type BehaviorTier = 'ask' | 'show'

/** One Skill file. Leave `text` out when the content is binary or was not read. */
export interface BehaviorFile {
  path: string
  text?: string
  executable?: boolean
}

/** One place a behavior appears. A rule that reads no lines has no line. */
export interface BehaviorLocation {
  path: string
  line: number | null
}

/** One behavior a Skill's files matched. */
export interface Behavior {
  id: string
  tier: BehaviorTier
  label: string
  /** The first matches, at most {@link MAX_BEHAVIOR_LOCATIONS}. */
  locations: BehaviorLocation[]
  /** Every matching line, or every matching file for a rule without lines. */
  total: number
}

const pattern = z.string().min(1).refine(value => !/[A-Z]/.test(value), 'patterns are lowercase')

const ruleSchema = z.strictObject({
  id: z.string().regex(/^[a-z-]+$/),
  tier: z.enum(['ask', 'show']),
  label: z.string().min(1),
  commands: z.array(z.array(pattern).min(1)).default([]),
  substrings: z.array(pattern).default([]),
  fences: z.array(pattern).default([]),
  tools: z.array(pattern).default([]),
  codepoints: z.array(z.tuple([z.string(), z.string()])).default([]),
  executable: z.boolean().default(false),
  extensions: z.array(pattern).default([]),
})

const documentSchema = z.strictObject({
  version: z.literal(1),
  matching: z.array(z.string()),
  behaviors: z.array(ruleSchema),
}).refine(
  document => new Set(document.behaviors.map(rule => rule.id)).size === document.behaviors.length,
  'behavior ids are unique',
)

/** One behavior rule, with its codepoint ranges parsed from hexadecimal. */
export type BehaviorRule = z.infer<typeof ruleSchema> & { ranges: Array<[number, number]> }

let parsed: BehaviorRule[] | undefined

/** Every behavior rule, in the order skilld reports behaviors. */
export function behaviorRules(): readonly BehaviorRule[] {
  parsed ??= documentSchema.parse(rawRules).behaviors.map(rule => ({
    ...rule,
    ranges: rule.codepoints.map(([start, end]) => {
      const range: [number, number] = [Number.parseInt(start, 16), Number.parseInt(end, 16)]
      if (range.some(Number.isNaN))
        throw new Error(`behavior ${rule.id} has a codepoint range that is not hexadecimal`)
      return range
    }),
  }))
  return parsed
}

/**
 * Match a Skill's files against every behavior rule.
 *
 * Behaviors come back in rule order. A behavior with no match is absent.
 */
export function detectBehaviors(files: readonly BehaviorFile[]): Behavior[] {
  const rules = behaviorRules()
  const found = rules.map(() => ({ locations: [] as BehaviorLocation[], total: 0 }))
  const record = (index: number, path: string, line: number | null): void => {
    const entry = found[index]!
    entry.total++
    if (entry.locations.length < MAX_BEHAVIOR_LOCATIONS)
      entry.locations.push({ path, line })
  }
  for (const file of files) {
    rules.forEach((rule, index) => {
      if (fileMatches(rule, file))
        record(index, file.path, null)
    })
    if (file.text !== undefined)
      scanText(rules, file.path, file.text, record)
  }
  return rules.flatMap((rule, index) => {
    const { locations, total } = found[index]!
    return total > 0 ? [{ id: rule.id, tier: rule.tier, label: rule.label, locations, total }] : []
  })
}

function fileMatches(rule: BehaviorRule, file: BehaviorFile): boolean {
  if (rule.executable && file.executable)
    return true
  const path = asciiLower(file.path)
  return rule.extensions.some(extension => path.endsWith(extension))
}

interface LineFacts {
  /** Code text: a fenced line, inline code spans, or a whole non-Markdown line. */
  code: string[]
  /** The first info string word of a fence this line opens. */
  fence?: string
  /** Tool names an `allowed-tools` entry on this line declares. */
  tools: string[]
}

interface Fence {
  marker: string
  length: number
}

function scanText(
  rules: readonly BehaviorRule[],
  path: string,
  raw: string,
  record: (index: number, path: string, line: number | null) => void,
): void {
  const text = raw.startsWith('\uFEFF') ? raw.slice(1) : raw
  const markdown = isMarkdown(path)
  const lines = rustLines(text)
  const frontmatter = path === 'SKILL.md' ? frontmatterEnd(lines) : 0
  const fence: { open?: Fence } = {}
  const list = { open: false }
  lines.forEach((line, index) => {
    const number = index + 1
    const facts: LineFacts = { code: [], tools: [] }
    if (number <= frontmatter)
      facts.tools = frontmatterTools(line, list)
    else if (markdown)
      markdownLine(line, fence, facts)
    else
      facts.code.push(line)
    const code = facts.code.map(asciiLower)
    rules.forEach((rule, ruleIndex) => {
      if (lineMatches(rule, line, code, facts))
        record(ruleIndex, path, number)
    })
  })
}

function lineMatches(rule: BehaviorRule, line: string, code: string[], facts: LineFacts): boolean {
  if (rule.ranges.length > 0) {
    for (const character of line) {
      const value = character.codePointAt(0)!
      if (rule.ranges.some(([start, end]) => value >= start && value <= end))
        return true
    }
  }
  if (facts.fence !== undefined && rule.fences.includes(facts.fence))
    return true
  if (facts.tools.some(tool => rule.tools.some(pattern => toolMatches(pattern, tool))))
    return true
  return code.some(segment =>
    rule.substrings.some(substring => segment.includes(substring))
    || rule.commands.some(tokens => commandMatches(segment, tokens)),
  )
}

function toolMatches(pattern: string, tool: string): boolean {
  return pattern.endsWith('*') ? tool.startsWith(pattern.slice(0, -1)) : tool === pattern
}

function isMarkdown(path: string): boolean {
  const lowered = asciiLower(path)
  return ['.md', '.mdx', '.markdown'].some(extension => lowered.endsWith(extension))
}

/** The line number that closes the SKILL.md frontmatter, or 0 without one. */
function frontmatterEnd(lines: string[]): number {
  if (lines.length === 0 || trimEnd(lines[0]!) !== '---')
    return 0
  const close = lines.findIndex((line, index) => index > 0 && trimEnd(line) === '---')
  return close < 0 ? 0 : close + 1
}

/**
 * The tool names one frontmatter line declares under `allowed-tools`.
 *
 * The key takes a string of names split by commas or spaces, a flow list, or a
 * block list on the lines that follow.
 */
function frontmatterTools(line: string, list: { open: boolean }): string[] {
  if (line.startsWith('allowed-tools:')) {
    let value = trim(line.slice('allowed-tools:'.length))
    list.open = value === ''
    if (value.startsWith('[') && value.endsWith(']') && value.length >= 2)
      value = value.slice(1, -1)
    return splitTools(value)
  }
  if (list.open) {
    const trimmed = trimStart(line)
    if (trimmed.startsWith('- '))
      return splitTools(trimmed.slice(2))
    if (!startsWithWhitespace(line) || trimmed === '')
      list.open = false
  }
  return []
}

function splitTools(value: string): string[] {
  const tools: string[] = []
  let current = ''
  let depth = 0
  const push = (): void => {
    const entry = trim(current).replace(/^['"]+|['"]+$/g, '')
    const name = trim(entry.split('(')[0] ?? '')
    if (name)
      tools.push(asciiLower(name))
    current = ''
  }
  for (const character of value) {
    if (character === '(') {
      depth++
      current += character
    }
    else if (character === ')') {
      depth = Math.max(0, depth - 1)
      current += character
    }
    else if ((character === ',' || character === ' ' || character === '\t') && depth === 0) {
      push()
    }
    else {
      current += character
    }
  }
  push()
  return tools
}

function markdownLine(line: string, fence: { open?: Fence }, facts: LineFacts): void {
  const trimmed = trimStart(line)
  if (fence.open) {
    const run = leadingRun(trimmed, fence.open.marker)
    if (run >= fence.open.length && trim(trimmed.slice(run)) === '')
      fence.open = undefined
    else
      facts.code.push(line)
    return
  }
  for (const marker of ['`', '~']) {
    const run = leadingRun(trimmed, marker)
    if (run >= 3) {
      const info = trim(trimmed.slice(run))
      const language = asciiLower(info.replace(/^\{+/, '').split(/[{},\t\n\v\f\r \u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]/)[0] ?? '')
      if (language)
        facts.fence = language
      fence.open = { marker, length: run }
      return
    }
  }
  facts.code.push(...line.split('`').filter((_, index) => index % 2 === 1))
}

function leadingRun(value: string, marker: string): number {
  let run = 0
  while (value[run] === marker)
    run++
  return run
}

/** Characters that delimit a shell token on their own. */
const SELF_DELIMITING = new Set(['|', '&', ';', '(', ')', '<', '>', '\'', '"', '`'])

function commandMatches(line: string, tokens: readonly string[]): boolean {
  let from = 0
  for (const token of tokens) {
    const end = findToken(line, token, from)
    if (end === undefined)
      return false
    from = end
  }
  return true
}

/** The end of the first bounded `token` at or after `from`. */
function findToken(line: string, token: string, from: number): number | undefined {
  const first = String.fromCodePoint(token.codePointAt(0)!)
  const last = [...token].at(-1)!
  let start = from
  for (;;) {
    const at = line.indexOf(token, start)
    if (at < 0)
      return undefined
    const end = at + token.length
    if (leftBounded(line, at, first) && rightBounded(line, end, last))
      return end
    start = at + first.length
  }
}

function leftBounded(line: string, at: number, first: string): boolean {
  if (SELF_DELIMITING.has(first))
    return true
  // Half of a surrogate pair is neither whitespace nor a delimiter, so one code unit decides.
  const before = line[at - 1]
  return before === undefined || isWhitespace(before) || SELF_DELIMITING.has(before) || before === '/' || before === '='
}

function rightBounded(line: string, end: number, last: string): boolean {
  if (SELF_DELIMITING.has(last))
    return true
  const after = line.slice(end).codePointAt(0)
  if (after === undefined)
    return true
  const character = String.fromCodePoint(after)
  return isWhitespace(character) || SELF_DELIMITING.has(character)
}

// Rust's `char::is_whitespace`, which differs from JavaScript's `\s` on U+0085 and U+FEFF.
const WHITESPACE = /^[\t\n\v\f\r \u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]$/u

function isWhitespace(character: string): boolean {
  return WHITESPACE.test(character)
}

function startsWithWhitespace(value: string): boolean {
  const first = value.codePointAt(0)
  return first !== undefined && isWhitespace(String.fromCodePoint(first))
}

function trimStart(value: string): string {
  const characters = [...value]
  let index = 0
  while (index < characters.length && isWhitespace(characters[index]!))
    index++
  return characters.slice(index).join('')
}

function trimEnd(value: string): string {
  const characters = [...value]
  let end = characters.length
  while (end > 0 && isWhitespace(characters[end - 1]!))
    end--
  return characters.slice(0, end).join('')
}

function trim(value: string): string {
  return trimEnd(trimStart(value))
}

/** Lowercase ASCII letters only, so every index still points at the same character. */
function asciiLower(value: string): string {
  return value.replace(/[A-Z]/g, letter => letter.toLowerCase())
}

/**
 * Rust's `str::lines`: split on `\n`, drop a `\r` before it, and add no empty
 * line after a final newline.
 */
function rustLines(text: string): string[] {
  if (text === '')
    return []
  const parts = text.split('\n')
  const lines = parts.map((part, index) => index < parts.length - 1 && part.endsWith('\r') ? part.slice(0, -1) : part)
  if (text.endsWith('\n'))
    lines.pop()
  return lines
}
