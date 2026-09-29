import type { HarnessV1Skill } from '@ai-sdk/harness'
import { readdir, readFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseDocument } from 'yaml'
import { isSkillName } from './internal/paths.ts'

const skillRoots = [
  resolve(dirname(fileURLToPath(import.meta.url)), '../skills'),
  resolve(dirname(fileURLToPath(import.meta.url)), 'skills'),
  resolve(dirname(fileURLToPath(import.meta.url)), '../../../skills'),
] as const

function parseManifest(source: string): ReadonlyArray<string> {
  const value = JSON.parse(source) as unknown
  if (!Array.isArray(value) || value.some(name => typeof name !== 'string' || !isSkillName(name)))
    throw new Error('skilld-maintained Skill manifest is invalid.')
  if (new Set(value).size !== value.length)
    throw new Error('skilld-maintained Skill manifest contains duplicate names.')
  return Object.freeze([...value])
}

/** Reads the first root that has `path`, and returns that root with the file. */
async function locateFile(path: string, roots: ReadonlyArray<string>): Promise<{ root: string, content: string }> {
  for (const root of roots) {
    const content = await readFile(resolve(root, path), 'utf8').catch((error) => {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT')
        return null
      throw error
    })
    if (content !== null)
      return { root, content }
  }
  throw new Error(`skilld-maintained Skill file is missing: ${path}`)
}

/** Lists every regular file under `dir`, nested folders included, as sorted POSIX paths. */
async function listFiles(dir: string, prefix = ''): Promise<ReadonlyArray<string>> {
  const entries = await readdir(resolve(dir, prefix), { withFileTypes: true })
  const nested = await Promise.all(entries.map(async (entry) => {
    const path = prefix ? `${prefix}/${entry.name}` : entry.name
    if (entry.isDirectory())
      return listFiles(dir, path)
    return entry.isFile() ? [path] : []
  }))
  return nested.flat().sort()
}

function splitSkill(source: string): { name: string, description: string, content: string } {
  const match = source.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n([\s\S]*)$/)
  if (!match)
    throw new Error('skilld-maintained Skill frontmatter is invalid.')

  const document = parseDocument(match[1]!, { uniqueKeys: true })
  if (document.errors.length > 0)
    throw new Error('skilld-maintained Skill frontmatter is invalid.')
  const frontmatter = document.toJS() as unknown
  if (!frontmatter || typeof frontmatter !== 'object')
    throw new Error('skilld-maintained Skill frontmatter is invalid.')

  const values = frontmatter as Record<string, unknown>
  if (typeof values.name !== 'string' || typeof values.description !== 'string')
    throw new Error('skilld-maintained Skill frontmatter is incomplete.')

  return { name: values.name, description: values.description, content: match[2]! }
}

export async function harnessSkillNames(roots: ReadonlyArray<string> = skillRoots): Promise<ReadonlyArray<string>> {
  const { content } = await locateFile('harness-skills.json', roots)
  return parseManifest(content)
}

export async function skilldMaintainedSkillNames(roots: ReadonlyArray<string> = skillRoots): Promise<ReadonlyArray<string>> {
  const { content } = await locateFile('skilld-maintained-skills.json', roots)
  return parseManifest(content)
}

/**
 * Loads one skilld-maintained Skill. A Harness Skill also carries every supporting file
 * in its folder, nested `scripts/` and `references/` folders included, so the Agent can read them.
 * `roots` lists the folders to search, first match wins.
 */
export async function loadSkilldMaintainedSkill(name: string, roots: ReadonlyArray<string> = skillRoots): Promise<HarnessV1Skill> {
  const names = await skilldMaintainedSkillNames(roots)
  if (!names.includes(name))
    throw new Error(`Unknown skilld-maintained Skill: ${name}`)

  const { root, content: source } = await locateFile(`${name}/SKILL.md`, roots)
  const skill = splitSkill(source)
  if (skill.name !== name)
    throw new Error(`skilld-maintained Skill name does not match its directory: ${name}`)

  const harnessSkills = await harnessSkillNames(roots)
  if (!harnessSkills.includes(name))
    return skill

  const skillDir = resolve(root, name)
  const paths = (await listFiles(skillDir)).filter(path => path !== 'SKILL.md')
  if (!paths.includes('assets/harness-request.md'))
    throw new Error(`skilld-maintained Skill file is missing: ${name}/assets/harness-request.md`)
  const files = await Promise.all(paths.map(async path => ({
    path,
    content: await readFile(resolve(skillDir, path), 'utf8'),
  })))
  return { ...skill, files }
}

export const DEFAULT_OUTPUT_POLICY = Object.freeze({
  maxSourceFiles: 2_000,
  maxSourceFileBytes: 512 * 1024,
  maxSourceBytes: 50 * 1024 * 1024,
  maxOutputFiles: 64,
  maxOutputFileBytes: 512 * 1024,
  maxOutputBytes: 4 * 1024 * 1024,
})
