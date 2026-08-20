import type { HarnessV1Skill } from '@ai-sdk/harness'
import { readFile } from 'node:fs/promises'
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

async function locateFile(path: string): Promise<string> {
  for (const root of skillRoots) {
    const value = await readFile(resolve(root, path), 'utf8').catch((error) => {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT')
        return null
      throw error
    })
    if (value !== null)
      return value
  }
  throw new Error(`skilld-maintained Skill file is missing: ${path}`)
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

export async function harnessSkillNames(): Promise<ReadonlyArray<string>> {
  const source = await locateFile('harness-skills.json')
  return parseManifest(source)
}

export async function skilldMaintainedSkillNames(): Promise<ReadonlyArray<string>> {
  const source = await locateFile('skilld-maintained-skills.json')
  return parseManifest(source)
}

export async function loadSkilldMaintainedSkill(name: string): Promise<HarnessV1Skill> {
  const names = await skilldMaintainedSkillNames()
  if (!names.includes(name))
    throw new Error(`Unknown skilld-maintained Skill: ${name}`)

  const source = await locateFile(`${name}/SKILL.md`)
  const skill = splitSkill(source)
  if (skill.name !== name)
    throw new Error(`skilld-maintained Skill name does not match its directory: ${name}`)

  const harnessSkills = await harnessSkillNames()
  if (!harnessSkills.includes(name))
    return skill

  const request = await locateFile(`${name}/assets/harness-request.md`)
  return {
    ...skill,
    files: [{ path: 'assets/harness-request.md', content: request }],
  }
}

export const DEFAULT_OUTPUT_POLICY = Object.freeze({
  maxSourceFiles: 2_000,
  maxSourceFileBytes: 512 * 1024,
  maxSourceBytes: 50 * 1024 * 1024,
  maxOutputFiles: 64,
  maxOutputFileBytes: 512 * 1024,
  maxOutputBytes: 4 * 1024 * 1024,
})
