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
  const prose = openProse()
  lines.forEach((line, index) => {
    const number = index + 1
    const facts: LineFacts = { code: [], tools: [] }
    if (number <= frontmatter)
      facts.tools = frontmatterTools(line, list)
    else if (markdown)
      markdownLine(line, fence, prose, facts)
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

function markdownLine(line: string, fence: { open?: Fence }, prose: Prose, facts: LineFacts): void {
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
      Object.assign(prose, openProse())
      return
    }
  }
  proseLine(line, prose, facts)
}

/**
 * Markdown prose state that carries from one line to the next.
 *
 * A Skill names what the Agent must not touch, as in "Don't read: `id_rsa`".
 * A code span that a prohibition governs is not code the Skill asks for.
 * Fenced blocks and other files always count: they hold commands.
 */
interface Prose {
  /** The open sentence starts with a prohibition. */
  negated: boolean
  /** The open sentence has not reached its first word. */
  opening: boolean
  /** The last line was a prohibition that ends with a colon. */
  leadIn: boolean
  /** The indent of the list items a prohibition lead-in governs. */
  list: number | undefined
}

/** One-word prohibitions. */
const PROHIBITIONS = new Set(['dont', 'don\'t', 'never', 'avoid', 'mustn\'t', 'shouldn\'t'])

/** Words that negate the code span right after them, as in "no `curl | bash`". */
const NEGATIONS = new Set(['no', 'not', 'never', 'avoid', 'dont', 'don\'t'])

/** Words that prohibit when `not` or `never` follows. */
const MODALS = new Set(['do', 'must', 'should'])

/** A prohibition of one of these words asks for the action, as in "don't forget to run". */
const REQUESTS = new Set(['forget', 'hesitate', 'skip', 'miss', 'omit', 'worry', 'panic', 'mind'])

/** A sentence with one of these words names an exception or a condition, so its code still counts. */
const EXCEPTIONS = new Set(['unless', 'except', 'without', 'instead', 'but', 'only', 'if', 'when', 'whenever', 'while'])

function openProse(): Prose {
  return { negated: false, opening: true, leadIn: false, list: undefined }
}

function proseLine(line: string, prose: Prose, facts: LineFacts): void {
  const characters = [...line]
  // Blockquote markers belong to the indent, so a quoted list still reads as a list.
  let indent = 0
  while (indent < characters.length && (isWhitespace(characters[indent]!) || characters[indent] === '>'))
    indent++
  if (indent === characters.length) {
    // A blank line ends the sentence. A lead-in and its list continue past it.
    prose.negated = false
    prose.opening = true
    return
  }
  const heading = indent <= 3 && headingMarker(characters, indent)
  const table = characters[indent] === '|'
  const item = heading || table ? 0 : listMarker(characters, indent)
  if (heading || table) {
    prose.list = undefined
    prose.leadIn = false
    prose.negated = false
    prose.opening = true
  }
  else if (item > 0) {
    if (prose.list !== undefined && indent < prose.list)
      prose.list = undefined
    if (prose.leadIn)
      prose.list = Math.min(prose.list ?? indent, indent)
    prose.leadIn = false
    prose.negated = false
    prose.opening = true
  }
  else {
    // A line that does not indent past the list ends it.
    if (prose.list !== undefined && indent <= prose.list)
      prose.list = undefined
    prose.leadIn = false
  }
  const governed = prose.list !== undefined
  const segments = line.split('`')
  const spans: Array<{ text: string, sentence: number, negated: boolean }> = []
  const exceptions = [false]
  let lastProse: string[] = []
  // The word right before a code span, with nothing but whitespace or emphasis between.
  let before = ''
  segments.forEach((segment, index) => {
    if (index % 2 === 1) {
      prose.opening = false
      spans.push({ text: segment, sentence: exceptions.length - 1, negated: prose.negated || governed || NEGATIONS.has(before) })
      before = ''
      return
    }
    const text = [...segment].slice(index === 0 ? indent + item : 0)
    const last = index === segments.length - 1
    let at = 0
    while (at < text.length) {
      const character = text[at]!
      if (isAsciiAlphanumeric(character)) {
        const [word, end] = readWord(text, at)
        if (prose.opening) {
          prose.opening = false
          prose.negated = opensProhibition(word, text, end)
        }
        if (EXCEPTIONS.has(word))
          exceptions[exceptions.length - 1] = true
        before = word
        at = end
        continue
      }
      if (!isWhitespace(character) && character !== '*' && character !== '_' && character !== '~')
        before = ''
      const end = boundary(text, at, last)
      if (end === 'sentence') {
        prose.negated = false
        prose.opening = true
        exceptions.push(false)
      }
      else if (end === 'clause') {
        // A dash ends a prohibition, as in "Never X — use `Y`". It opens none.
        prose.negated = false
      }
      else if (character === ':' && !prose.negated) {
        // A label such as "Tip:" ends, and the words after it open the sentence again.
        prose.opening = true
      }
      at++
    }
    if (last)
      lastProse = text
  })
  for (const span of spans) {
    if (!span.negated || exceptions[span.sentence])
      facts.code.push(span.text)
  }
  if (heading) {
    prose.negated = false
    prose.opening = true
  }
  else if (!table) {
    prose.leadIn = prose.negated && !exceptions.at(-1) && endsWithColon(lastProse)
  }
}

/** Whether prose ends with a colon, after closing emphasis such as `**Don't read:**`. */
function endsWithColon(text: string[]): boolean {
  let end = text.length
  while (end > 0 && (isWhitespace(text[end - 1]!) || text[end - 1] === '*' || text[end - 1] === '_'))
    end--
  return text[end - 1] === ':'
}

/** Whether ATX heading hashes open the line at `at`. */
function headingMarker(characters: string[], at: number): boolean {
  let run = 0
  while (characters[at + run] === '#')
    run++
  const after = characters[at + run]
  return run >= 1 && run <= 6 && (after === undefined || after === ' ' || after === '\t')
}

/**
 * The length of a list marker at `at`, with the whitespace and checkbox after it.
 * 0 when the line holds no list item.
 */
function listMarker(characters: string[], at: number): number {
  let end = at
  if (characters[end] === '-' || characters[end] === '*' || characters[end] === '+') {
    end++
  }
  else {
    while (end - at < 9 && isAsciiDigit(characters[end]))
      end++
    if (end === at || (characters[end] !== '.' && characters[end] !== ')'))
      return 0
    end++
  }
  if (characters[end] !== undefined && !isWhitespace(characters[end]!))
    return 0
  while (characters[end] !== undefined && isWhitespace(characters[end]!))
    end++
  const box = characters[end + 1]
  const after = characters[end + 3]
  if (characters[end] === '[' && (box === ' ' || box === 'x' || box === 'X') && characters[end + 2] === ']' && (after === undefined || isWhitespace(after)))
    end += 3
  return end - at
}

/** A word of ASCII letters, digits, and apostrophes, lowercased, from `at`. */
function readWord(text: string[], at: number): [string, number] {
  let end = at
  let word = ''
  while (end < text.length && (isAsciiAlphanumeric(text[end]!) || text[end] === '\'' || text[end] === '’')) {
    word += text[end] === '’' ? '\'' : text[end]!
    end++
  }
  return [asciiLower(word.replace(/'+$/, '')), end]
}

/** The next word after emphasis and whitespace, or '' when something else comes first. */
function nextWord(text: string[], at: number): [string, number] {
  let start = at
  while (start < text.length && (isWhitespace(text[start]!) || text[start] === '*' || text[start] === '_' || text[start] === '~'))
    start++
  return start < text.length && isAsciiAlphanumeric(text[start]!) ? readWord(text, start) : ['', start]
}

function opensProhibition(word: string, text: string[], end: number): boolean {
  const [second, afterSecond] = nextWord(text, end)
  if (PROHIBITIONS.has(word))
    return !REQUESTS.has(second)
  if (MODALS.has(word) && (second === 'not' || second === 'never'))
    return !REQUESTS.has(nextWord(text, afterSecond)[0])
  return false
}

/** Closing marks that may follow the end of a sentence, as in `**Never.**` or `(see below.)`. */
const CLOSERS = new Set(['*', '_', ')', ']', '"', '\'', '\u2019', '\u201D'])

/**
 * What the character at `at` ends, if anything.
 *
 * `;` and `|` end a sentence. `.`, `!`, and `?` end one before whitespace or the
 * end of the line, after any closing marks. A dash ends a clause.
 */
function boundary(text: string[], at: number, last: boolean): 'sentence' | 'clause' | undefined {
  const character = text[at]!
  if (character === ';' || character === '|')
    return 'sentence'
  if (character === '\u2013' || character === '\u2014')
    return 'clause'
  if (character === '-') {
    // A spaced hyphen or double hyphen is a dash: "never X - it Y".
    let end = at
    while (text[end] === '-')
      end++
    const spaced = end - at <= 2 && at > 0 && isWhitespace(text[at - 1]!) && text[end] !== undefined && isWhitespace(text[end]!)
    return spaced ? 'clause' : undefined
  }
  if (character !== '.' && character !== '!' && character !== '?')
    return undefined
  if (character === '.' && abbreviation(text, at))
    return undefined
  let next = at + 1
  while (next < text.length && CLOSERS.has(text[next]!))
    next++
  return (next === text.length ? last : isWhitespace(text[next]!)) ? 'sentence' : undefined
}

/** A period after a lone letter, as in "e.g.", abbreviates and ends no sentence. */
function abbreviation(text: string[], at: number): boolean {
  if (at < 1 || !isAsciiLetter(text[at - 1]!))
    return false
  const before = text[at - 2]
  return before === undefined || before === '.' || before === '(' || isWhitespace(before)
}

function isAsciiDigit(character: string | undefined): boolean {
  return character !== undefined && character >= '0' && character <= '9'
}

function isAsciiLetter(character: string): boolean {
  return (character >= 'a' && character <= 'z') || (character >= 'A' && character <= 'Z')
}

function isAsciiAlphanumeric(character: string): boolean {
  return isAsciiLetter(character) || isAsciiDigit(character)
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
