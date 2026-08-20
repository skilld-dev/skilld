import type { SkillOutputPolicy, SkillRun, SkillRunError } from '../types.ts'
import type { Result } from './result.ts'
import { isSkillName, resolveWithin } from './paths.ts'
import { err, ok } from './result.ts'

function invalid(message: string): Result<never, SkillRunError> {
  return err({ _tag: 'InvalidInput', message })
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === 'string' && value.trim().length > 0
}

export function parseSkillRun(input: SkillRun): Result<SkillRun, SkillRunError> {
  if (!input || typeof input !== 'object' || !('_tag' in input))
    return invalid('Skill run input must be an object with a tag.')

  if (input._tag === 'PackageSkill') {
    if (!input.source || !input.destination)
      return invalid('Package Skill input is incomplete.')
    if (!isNonEmptyString(input.destination.rootDir) || !isSkillName(input.destination.name))
      return invalid('Package Skill destination is invalid.')
    if (input.source._tag === 'NpmPackage') {
      if (!isNonEmptyString(input.source.spec))
        return invalid('npm package spec is required.')
      return ok(input)
    }
    if (input.source._tag === 'LocalPackage') {
      if (!isNonEmptyString(input.source.rootDir) || !isNonEmptyString(input.source.packageDir))
        return invalid('Local package paths are required.')
      if (resolveWithin(input.source.rootDir, input.source.packageDir) === null)
        return invalid('Local package directory must stay inside its root directory.')
      return ok(input)
    }
    return invalid('Package source tag is invalid.')
  }

  if (input._tag === 'ProjectSkill') {
    if (!isNonEmptyString(input.projectDir) || !input.destination)
      return invalid('Project Skill input is incomplete.')
    if (!isNonEmptyString(input.destination.rootDir) || !isSkillName(input.destination.name))
      return invalid('Project Skill destination is invalid.')
    return ok(input)
  }

  if (input._tag === 'ReviewSkill') {
    if (!isNonEmptyString(input.skillDir))
      return invalid('Skill review directory is required.')
    return ok(input)
  }

  return invalid('Skill run tag is invalid.')
}

export function parseOutputPolicy(policy: SkillOutputPolicy): SkillOutputPolicy {
  for (const [name, value] of Object.entries(policy)) {
    if (!Number.isSafeInteger(value) || value <= 0)
      throw new TypeError(`${name} must be a positive integer.`)
  }
  return Object.freeze(policy)
}
