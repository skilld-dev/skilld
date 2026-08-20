import type { SkillReview, SkillReviewFinding, SkillRunError } from '../../types.ts'
import type { Result } from '../result.ts'
import type { CollectedFile } from './collect.ts'
import { parseDocument } from 'yaml'
import { isSkillName, normalizeOutputPath } from '../paths.ts'
import { err, ok } from '../result.ts'

const allowedFrontmatter = new Set([
  'name',
  'description',
  'license',
  'compatibility',
  'metadata',
  'allowed-tools',
])

const invalid = (issues: ReadonlyArray<string>): Result<never, SkillRunError> =>
  err({ _tag: 'InvalidSkill', message: 'Skill output failed deterministic checks.', issues })

function decodeText(content: Uint8Array): string | null {
  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(content)
  }
  catch {
    return null
  }
}

const isStringMap = (value: unknown): boolean =>
  value !== null
  && typeof value === 'object'
  && !Array.isArray(value)
  && Object.values(value).every(item => typeof item === 'string')

export const validateGeneratedSkill = (
  name: string,
  files: ReadonlyArray<CollectedFile>,
): Result<void, SkillRunError> => {
  const issues: string[] = []
  const skillFiles = files.filter(file => file.path === 'SKILL.md')
  if (skillFiles.length !== 1)
    return invalid(['Output must contain exactly one SKILL.md file.'])

  const source = decodeText(skillFiles[0]!.content)
  if (source === null)
    return invalid(['SKILL.md must contain valid UTF-8 text.'])
  const match = source.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n([\s\S]*)$/)
  if (!match)
    return invalid(['SKILL.md must start with YAML frontmatter.'])
  if (match[2]?.trim().length === 0)
    issues.push('SKILL.md instructions must not be empty.')

  const document = parseDocument(match[1]!, { uniqueKeys: true })
  for (const error of document.errors)
    issues.push(`Frontmatter: ${error.message}`)
  for (const warning of document.warnings)
    issues.push(`Frontmatter: ${warning.message}`)

  const value = document.errors.length === 0 ? document.toJS() as unknown : null
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    issues.push('Frontmatter must be a mapping.')
    return invalid(issues)
  }

  const frontmatter = value as Record<string, unknown>
  for (const key of Object.keys(frontmatter)) {
    if (!allowedFrontmatter.has(key))
      issues.push(`Frontmatter field is not supported: ${key}`)
  }
  if (frontmatter.name !== name)
    issues.push('Frontmatter name must match the Skill directory name.')
  if (typeof frontmatter.name !== 'string' || !isSkillName(frontmatter.name))
    issues.push('Frontmatter name is invalid.')
  if (typeof frontmatter.description !== 'string' || frontmatter.description.trim().length === 0 || frontmatter.description.length > 1024)
    issues.push('Frontmatter description must contain 1 to 1024 characters.')
  if (frontmatter.license !== undefined && typeof frontmatter.license !== 'string')
    issues.push('Frontmatter license must be a string.')
  if (frontmatter.compatibility !== undefined && (typeof frontmatter.compatibility !== 'string' || frontmatter.compatibility.length > 500))
    issues.push('Frontmatter compatibility must be a string of at most 500 characters.')
  if (frontmatter.metadata !== undefined && !isStringMap(frontmatter.metadata))
    issues.push('Frontmatter metadata must map strings to strings.')
  if (frontmatter['allowed-tools'] !== undefined && typeof frontmatter['allowed-tools'] !== 'string')
    issues.push('Frontmatter allowed-tools must be a string.')

  return issues.length === 0 ? ok(undefined) : invalid(issues)
}

const findingLevels = new Set(['error', 'warning', 'note'])

export const validateSkillReview = (
  files: ReadonlyArray<CollectedFile>,
): Result<SkillReview, SkillRunError> => {
  if (files.length !== 1 || files[0]?.path !== 'review.json')
    return invalid(['Review output must contain only review.json.'])

  let value: unknown
  try {
    const source = decodeText(files[0].content)
    if (source === null)
      return invalid(['review.json must contain valid UTF-8 text.'])
    value = JSON.parse(source)
  }
  catch {
    return invalid(['review.json must contain valid JSON.'])
  }
  if (!value || typeof value !== 'object' || Array.isArray(value))
    return invalid(['review.json must contain an object.'])

  const review = value as Record<string, unknown>
  if (Object.keys(review).some(key => key !== 'summary' && key !== 'findings'))
    return invalid(['review.json contains unsupported fields.'])
  if (typeof review.summary !== 'string' || review.summary.trim().length === 0)
    return invalid(['Review summary is required.'])
  if (!Array.isArray(review.findings))
    return invalid(['Review findings must be an array.'])

  const findings: SkillReviewFinding[] = []
  for (const item of review.findings) {
    if (!item || typeof item !== 'object' || Array.isArray(item))
      return invalid(['Each review finding must be an object.'])
    const finding = item as Record<string, unknown>
    if (Object.keys(finding).some(key => !['level', 'path', 'message', 'fix'].includes(key)))
      return invalid(['A review finding contains unsupported fields.'])
    if (typeof finding.level !== 'string' || !findingLevels.has(finding.level))
      return invalid(['A review finding level is invalid.'])
    if (typeof finding.path !== 'string' || normalizeOutputPath(finding.path) === null)
      return invalid(['A review finding path is invalid.'])
    if (typeof finding.message !== 'string' || finding.message.trim().length === 0 || typeof finding.fix !== 'string' || finding.fix.trim().length === 0)
      return invalid(['A review finding is incomplete.'])
    findings.push(finding as unknown as SkillReviewFinding)
  }

  return ok({ _tag: 'SkillReview', summary: review.summary, findings })
}
