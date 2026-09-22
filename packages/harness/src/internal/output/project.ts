const searchCommands = ['rg ', 'grep ', 'git grep ']

/** Inline code spans, which is where a Skill writes its file pointers. */
const inlineCode = /`([^`\n]+)`/g

/** Fenced blocks, which hold commands rather than pointers. */
const fencedBlock = /```[^\n]*\n[\s\S]*?```/g

/**
 * A Skill may report a file the project lacks, such as a missing entry point.
 * A line that denies a path states a fact about the project, not a pointer.
 */
const denial = /\b(no|not|none|never|absent|missing|lacks|without|instead|rather|empty|undeclared|unavailable|nothing)\b|\bdoes ?n[o']t\b/i

function fencedBlocks(markdown: string): ReadonlyArray<string> {
  return markdown.match(fencedBlock) ?? []
}

/** Prose lines, with fenced commands removed so a command never reads as a pointer. */
function proseLines(markdown: string): ReadonlyArray<string> {
  return markdown.replace(fencedBlock, '').split('\n')
}

/**
 * A token counts as a file pointer only when it cannot be anything else.
 * A false pointer fails a correct Skill, so every rule here excludes rather than includes.
 */
function pathCandidate(token: string): string | null {
  if (token.length === 0 || /\s/.test(token))
    return null
  // A flag, a variable, a comment, or a placeholder is never a pointer.
  if (/^[-$#<{[]/.test(token))
    return null
  // A URL, a package specifier, or a shell construct is never a pointer.
  if (token.includes('://') || /[()|;&"'@:]/.test(token))
    return null
  const trimmed = token.replace(/^\.\//, '').replace(/\/+$/, '')
  if (trimmed.length === 0 || trimmed.startsWith('..'))
    return null
  // A bare word such as `rg` or `pnpm` carries no path shape.
  if (!trimmed.includes('/') && !/\.[a-z0-9]+$/i.test(trimmed))
    return null
  return trimmed
}

/** The literal prefix of a glob, which is the part a real path must start with. */
function literalPrefix(candidate: string): string {
  const wildcard = candidate.search(/[*?]/)
  if (wildcard === -1)
    return candidate
  const cut = candidate.slice(0, wildcard).replace(/[^/]*$/, '')
  return cut.replace(/\/+$/, '')
}

function resolves(candidate: string, known: ReadonlySet<string>): boolean {
  if (known.has(candidate))
    return true
  const prefix = literalPrefix(candidate)
  if (prefix.length === 0)
    return true
  // A directory pointer resolves when the project holds a file under it.
  for (const path of known) {
    if (path === prefix || path.startsWith(`${prefix}/`))
      return true
  }
  return false
}

export interface ProjectSkillInput {
  /** The generated SKILL.md text. */
  readonly markdown: string
  /** Project-relative paths the Harness prepared for the Agent. */
  readonly projectPaths: ReadonlyArray<string>
  /** Output-relative paths the Skill writes beside SKILL.md. */
  readonly outputPaths: ReadonlyArray<string>
}

/**
 * Check that a project Skill points at files the project contains and gives a
 * search the Agent can repeat. The Agent cannot verify its own pointers.
 *
 * The Harness hides generated directories from the Agent, so a pointer at one is
 * invention like any other. A line that states a path is missing passes, because
 * reporting an absent file is a fact rather than a pointer.
 */
export function checkProjectSkill(input: ProjectSkillInput): ReadonlyArray<string> {
  const issues: string[] = []
  const known = new Set([...input.projectPaths, ...input.outputPaths])
  const unknown = new Set<string>()

  for (const line of proseLines(input.markdown)) {
    if (denial.test(line))
      continue
    for (const match of line.matchAll(inlineCode)) {
      const candidate = pathCandidate(match[1]!.trim())
      if (candidate !== null && !resolves(candidate, known))
        unknown.add(candidate)
    }
  }

  for (const path of [...unknown].sort())
    issues.push(`SKILL.md points at a path the project does not contain: ${path}.`)

  const inline = [...input.markdown.matchAll(inlineCode)].map(match => match[1]!)
  const commands = [...fencedBlocks(input.markdown), ...inline].join('\n')
  if (!searchCommands.some(command => commands.includes(command)))
    issues.push('SKILL.md must give the Agent at least one search command it can repeat.')

  return issues
}
