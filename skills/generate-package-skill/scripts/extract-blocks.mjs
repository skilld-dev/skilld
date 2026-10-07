#!/usr/bin/env node
// Write each fenced code block of a Skill file to its own directory, so every example runs exactly as written.
//
// Usage: node extract-blocks.mjs FILE OUT_DIR [--replace FROM=TO]...
//
// Block N goes to OUT_DIR/N/. A first line that names a path, such as `// server/api/search.ts`,
// `# scripts/check.sh`, or `<!-- app/pages/index.vue -->`, sets the file name. Otherwise the file is `block.EXT`.
// `--replace` swaps a placeholder in every block, such as `https://example.com=http://localhost:3000`.
// OUT_DIR/blocks.json lists each block with its number, start line, language, and file.
// The script refuses a path that leaves OUT_DIR. It never runs a block.

import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, isAbsolute, join, normalize, sep } from 'node:path'
import process from 'node:process'

function usage(message) {
  process.stderr.write(`${message}\nUsage: node extract-blocks.mjs FILE OUT_DIR [--replace FROM=TO]...\n`)
  process.exit(2)
}

const EXTENSIONS = { javascript: 'js', typescript: 'ts', shell: 'sh', bash: 'sh', zsh: 'sh', console: 'sh', yml: 'yaml', jsonc: 'json', markdown: 'md' }
// A path comment: `// a/b.ts`, `# a/b.sh`, `-- a.sql`, `/* a.css */`, or `<!-- a.vue -->`. The path needs an extension.
const PATH_COMMENT = /^\s*(?:\/\/|#|--|\/\*|<!--)\s*([\w@.\-/[\]]+\.\w+)\s*(?:(?:\*\/|-->)\s*)?$/
const FENCE_OPEN = /^ {0,3}(`{3,}|~{3,})\s*([\w+-]*)/

function parseArgs(argv) {
  const [file, out, ...rest] = argv
  if (!file || !out)
    usage('Give the Skill file and an output directory.')
  const replacements = []
  for (let index = 0; index < rest.length; index++) {
    if (rest[index] !== '--replace' || rest[index + 1] === undefined)
      usage(`Unknown option: ${rest[index]}`)
    const pair = rest[++index]
    const split = pair.indexOf('=')
    if (split <= 0)
      usage('--replace needs FROM=TO.')
    replacements.push([pair.slice(0, split), pair.slice(split + 1)])
  }
  return { file, out, replacements }
}

/** Fenced blocks with the line their content starts on. Nested shorter fences stay inside the block. */
function fencedBlocks(markdown) {
  const lines = markdown.split('\n')
  const blocks = []
  for (let index = 0; index < lines.length; index++) {
    const open = FENCE_OPEN.exec(lines[index])
    if (!open)
      continue
    const [, fence, lang] = open
    const start = index + 1
    let end = start
    while (end < lines.length && !new RegExp(`^ {0,3}${fence[0]}{${fence.length},}\\s*$`).test(lines[end]))
      end++
    blocks.push({ line: start + 1, lang: lang.toLowerCase(), body: lines.slice(start, end).join('\n') })
    index = end
  }
  return blocks
}

function fileFor(block) {
  const named = PATH_COMMENT.exec(block.body.split('\n')[0] ?? '')?.[1]
  if (named) {
    const path = normalize(named)
    if (isAbsolute(path) || path === '..' || path.startsWith(`..${sep}`))
      return { _tag: 'Escapes', path: named }
    return { _tag: 'Named', path }
  }
  return { _tag: 'Default', path: `block.${EXTENSIONS[block.lang] ?? (block.lang || 'txt')}` }
}

const { file, out, replacements } = parseArgs(process.argv.slice(2))
const markdown = await readFile(file, 'utf8').catch(error => usage(`Cannot read ${file}: ${error.message}`))
const manifest = []
let status = 0
for (const [index, block] of fencedBlocks(markdown).entries()) {
  const number = index + 1
  const target = fileFor(block)
  if (target._tag === 'Escapes') {
    process.stderr.write(`block ${number} line ${block.line}: path ${target.path} leaves the output directory; skipped\n`)
    status = 1
    continue
  }
  const content = replacements.reduce((text, [from, to]) => text.replaceAll(from, to), block.body)
  const destination = join(out, String(number), target.path)
  await mkdir(dirname(destination), { recursive: true })
  await writeFile(destination, content.endsWith('\n') ? content : `${content}\n`)
  manifest.push({ block: number, line: block.line, lang: block.lang, file: join(String(number), target.path) })
  process.stderr.write(`block ${number} line ${block.line} ${block.lang || '-'} -> ${join(String(number), target.path)}\n`)
}
await mkdir(out, { recursive: true })
await writeFile(join(out, 'blocks.json'), `${JSON.stringify(manifest, null, 2)}\n`)
process.exit(status)
