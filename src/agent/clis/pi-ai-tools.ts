/**
 * Sandboxed tool execution for the pi-ai agentic loop.
 *
 * The model can Read/Glob/Write within `.skilld/` and run a tightly
 * allowlisted set of shell commands. All file operations resolve through
 * `resolveSandboxedPath` which blocks traversal outside `skilldDir`.
 */

import type { ToolCall } from '@earendil-works/pi-ai'
import { execFileSync } from 'node:child_process'
import { existsSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { join } from 'pathe'
import { Type } from 'typebox'
import { TRAILING_SLASH_RE } from '../../core/regex.ts'
import { sanitizeMarkdown } from '../../core/sanitize.ts'

const STATIC_REGEX_1 = /^\.\/\.skilld\//
const STATIC_REGEX_2 = /^\.skilld\//
const STATIC_REGEX_3 = /^\.\//
const STATIC_REGEX_5 = /\s+/

export const TOOLS = [
  {
    name: 'Read',
    description: 'Read a file. Path is relative to the working directory (e.g. "./.skilld/docs/api.md").',
    parameters: Type.Object({ path: Type.String({ description: 'File path to read' }) }),
  },
  {
    name: 'Glob',
    description: 'List files matching a glob pattern (e.g. "./.skilld/docs/*.md"). Returns newline-separated paths.',
    parameters: Type.Object({
      pattern: Type.String({ description: 'Glob pattern' }),
      no_ignore: Type.Optional(Type.Boolean({ description: 'Include gitignored files' })),
    }),
  },
  {
    name: 'Write',
    description: 'Write content to a file.',
    parameters: Type.Object({
      path: Type.String({ description: 'File path to write' }),
      content: Type.String({ description: 'File content' }),
    }),
  },
  {
    name: 'Bash',
    description: 'Run a shell command. Use for `skilld search`, `skilld validate`, etc.',
    parameters: Type.Object({ command: Type.String({ description: 'Shell command to run' }) }),
  },
]

export const MAX_TOOL_TURNS = 30

const SAFE_COMMANDS = new Set(['skilld', 'ls', 'cat', 'find'])
const SHELL_META_RE = /[;&|`$()<>]/

/** Resolve a path safely within skilldDir, blocking traversal. */
function resolveSandboxedPath(p: string, skilldDir: string): string {
  const cleaned = String(p).replace(STATIC_REGEX_1, './').replace(STATIC_REGEX_2, './').replace(STATIC_REGEX_3, '')
  const resolved = resolve(skilldDir, cleaned)
  if (!resolved.startsWith(`${skilldDir}/`) && resolved !== skilldDir)
    throw new Error(`Path traversal blocked: ${p}`)
  return resolved
}

/** Match a file path against a glob pattern using simple segment matching (no regex from user input). */
function globMatch(filePath: string, pattern: string): boolean {
  const segments = pattern.split('**')
  if (segments.length === 1) {
    const parts = pattern.split('*')
    if (parts.length === 1)
      return filePath === pattern
    let pos = 0
    for (let i = 0; i < parts.length; i++) {
      const part = parts[i]!
      if (!part)
        continue
      const idx = filePath.indexOf(part, pos)
      if (idx === -1)
        return false
      if (i === 0 && idx !== 0)
        return false
      pos = idx + part.length
    }
    if (parts.at(-1) !== '')
      return pos === filePath.length
    return true
  }
  let remaining = filePath
  for (let i = 0; i < segments.length; i++) {
    const seg = segments[i]!
    if (!seg)
      continue
    const segParts = seg.split('*')
    let pos = 0
    let matched = false
    for (let attempt = remaining.indexOf(segParts[0]!, 0); attempt !== -1; attempt = remaining.indexOf(segParts[0]!, attempt + 1)) {
      pos = attempt
      matched = true
      for (const sp of segParts) {
        if (!sp)
          continue
        const idx = remaining.indexOf(sp, pos)
        if (idx === -1) {
          matched = false
          break
        }
        pos = idx + sp.length
      }
      if (matched)
        break
    }
    if (!matched)
      return false
    remaining = remaining.slice(pos)
  }
  return true
}

/** Execute a tool call against the .skilld/ directory. */
export function executeTool(toolCall: ToolCall, skilldDir: string): string {
  const args = toolCall.arguments as Record<string, unknown>

  switch (toolCall.name) {
    case 'Read': {
      const filePath = resolveSandboxedPath(args.path as string, skilldDir)
      if (!existsSync(filePath))
        return `Error: file not found: ${args.path}`
      return sanitizeMarkdown(readFileSync(filePath, 'utf-8'))
    }
    case 'Glob': {
      const pattern = String(args.pattern).replace(STATIC_REGEX_1, './').replace(STATIC_REGEX_2, './').replace(STATIC_REGEX_3, '')
      const results: string[] = []
      const walkDir = (dir: string, prefix: string) => {
        if (!existsSync(dir))
          return
        for (const entry of readdirSync(dir, { withFileTypes: true })) {
          const relPath = prefix ? `${prefix}/${entry.name}` : entry.name
          if (entry.isDirectory())
            walkDir(join(dir, entry.name), relPath)
          else results.push(`./.skilld/${relPath}`)
        }
      }
      const baseDir = pattern.split('*')[0]?.replace(TRAILING_SLASH_RE, '') ?? ''
      walkDir(join(skilldDir, baseDir), baseDir)
      const matched = results.filter(r => globMatch(r.replace(STATIC_REGEX_1, ''), pattern))
      return matched.length > 0 ? matched.join('\n') : `No files matching: ${args.pattern}`
    }
    case 'Write': {
      const filePath = resolveSandboxedPath(args.path as string, skilldDir)
      writeFileSync(filePath, sanitizeMarkdown(String(args.content)))
      return 'File written successfully.'
    }
    case 'Bash': {
      const cmd = String(args.command).trim()
      const parts = cmd.split(STATIC_REGEX_5)
      const bin = parts[0] ?? ''
      if (!SAFE_COMMANDS.has(bin) || SHELL_META_RE.test(cmd))
        return `Error: command not allowed. Only skilld, ls, cat, find commands are permitted.`
      try {
        return execFileSync(bin, parts.slice(1), { cwd: skilldDir, timeout: 15_000, encoding: 'utf-8', maxBuffer: 512 * 1024 }).trim()
      }
      catch (err) {
        return `Error: ${(err as Error).message}`
      }
    }
    default:
      return `Unknown tool: ${toolCall.name}`
  }
}
