/**
 * Prefix-based input parser for `skilld add`
 *
 * All sources require an explicit prefix:
 *   npm:vue         → package skill from registry
 *   crate:serde     → Rust crate from crates.io
 *   gh:owner/repo   → git skill
 *   github:o/r      → git skill (alias)
 *   @handle          → curator's skills
 *   @handle/coll     → collection, with scoped npm fallback
 *
 * Bare names (no prefix) are deprecated but still resolve as npm: with a warning.
 */

import type { GitSkillSource } from '../sources/git-skills.ts'
import { parseGitSkillInput } from '../sources/git-skills.ts'

const STATIC_REGEX_1 = /^[\w.-]+\/[\w.-]+/
const EXPLICIT_NON_NPM_PREFIX_RE = /^(?:crate|gh|github):/
const SCOPED_NPM_PACKAGE_RE = /^@[^/\s]+\/[^@\s]+(?:@.+)?$/

export type SkillSource
  = | { type: 'npm', package: string, tag?: string }
    | { type: 'crate', package: string, version?: string }
    | { type: 'git', source: GitSkillSource, skillFilter?: string }
    | { type: 'curator', handle: string }
    | { type: 'collection-or-npm', handle: string, name: string, package: string }
    | { type: 'bare', package: string, tag?: string }

export type NpmPackageInputResult
  = | { _tag: 'Ok', packageSpecs: string[] }
    | { _tag: 'Err', input: string }

export function parseNpmPackageInputs(inputs: string[]): NpmPackageInputResult {
  const packageSpecs: string[] = []

  for (const input of inputs) {
    if (SCOPED_NPM_PACKAGE_RE.test(input)) {
      const { name, tag } = splitPackageTag(input)
      packageSpecs.push(tag ? `${name}@${tag}` : name)
      continue
    }
    const source = parseSkillInput(input)
    const isMalformedExplicitSource = source.type === 'bare' && EXPLICIT_NON_NPM_PREFIX_RE.test(input)
    if ((source.type !== 'npm' && source.type !== 'bare') || isMalformedExplicitSource || !source.package)
      return { _tag: 'Err', input }

    packageSpecs.push(source.tag ? `${source.package}@${source.tag}` : source.package)
  }

  return { _tag: 'Ok', packageSpecs }
}

/**
 * Parse a single CLI input token into a typed SkillSource.
 *
 * Does NOT emit deprecation warnings; callers handle that for `bare` type.
 */
export function parseSkillInput(input: string): SkillSource {
  const trimmed = input.trim()

  // npm: prefix → package skill
  if (trimmed.startsWith('npm:')) {
    const rest = trimmed.slice(4)
    const { name, tag } = splitPackageTag(rest)
    return { type: 'npm', package: name, tag }
  }

  // crate: prefix → Rust crate from crates.io
  if (trimmed.startsWith('crate:')) {
    const rest = trimmed.slice(6).trim()
    const atIdx = rest.indexOf('@')
    const name = (atIdx === -1 ? rest : rest.slice(0, atIdx)).toLowerCase()
    const version = atIdx === -1 ? undefined : rest.slice(atIdx + 1) || undefined
    return { type: 'crate', package: name, version }
  }

  // gh: or github: prefix → git skill
  if (trimmed.startsWith('gh:') || trimmed.startsWith('github:')) {
    const rest = trimmed.startsWith('gh:') ? trimmed.slice(3) : trimmed.slice(7)
    const gitSource = parseGitSkillInput(rest)
    if (gitSource)
      return { type: 'git', source: gitSource }
    // If gh: prefix used but can't parse as git, treat as github shorthand
    if (STATIC_REGEX_1.test(rest)) {
      const [owner, repo] = rest.split('/')
      return { type: 'git', source: { type: 'github', owner, repo } }
    }
    // Invalid gh: input, fall through to bare
    return { type: 'bare', package: rest }
  }

  // @handle (curator), @handle/collection, or legacy @scope/package
  if (trimmed.startsWith('@')) {
    const { name: packageName, tag } = splitPackageTag(trimmed)
    if (tag)
      return { type: 'bare', package: packageName, tag }
    const rest = packageName.slice(1)
    const slashIdx = rest.indexOf('/')
    if (slashIdx === -1)
      return { type: 'curator', handle: rest }
    return {
      type: 'collection-or-npm',
      handle: rest.slice(0, slashIdx),
      name: rest.slice(slashIdx + 1),
      package: packageName,
    }
  }

  // Try existing git detection (SSH, URLs, local paths, owner/repo shorthand)
  const gitSource = parseGitSkillInput(trimmed)
  if (gitSource)
    return { type: 'git', source: gitSource }

  // Bare name (deprecated) → resolves as npm
  const { name, tag } = splitPackageTag(trimmed)
  return { type: 'bare', package: name, tag }
}

/**
 * Resolve a CLI input to the bare package/skill name used for lookup in the lockfile.
 * Strips `npm:` / `gh:` prefixes. Returns null for curators, which do not address
 * a single skill name.
 */
export function resolveSkillName(input: string): string | null {
  const source = parseSkillInput(input)
  switch (source.type) {
    case 'npm':
    case 'bare':
    case 'collection-or-npm':
      return source.package
    case 'crate':
      return `crate:${source.package}`
    case 'git':
      if (source.source.type === 'github' && source.source.repo)
        return source.source.repo
      return null
    case 'curator':
      return null
    default: {
      const _exhaustive: never = source
      throw new Error(`Unhandled SkillSource type: ${JSON.stringify(_exhaustive)}`)
    }
  }
}

/**
 * Map a lockfile/identity package name to the storage-safe name used for
 * cache directories and symlinks. `crate:serde` → `@skilld-crate/serde`;
 * other names pass through unchanged.
 */
export function toStoragePackageName(identityName: string): string {
  if (identityName.startsWith('crate:'))
    return `@skilld-crate/${identityName.slice('crate:'.length)}`
  return identityName
}

/** True if `spec` targets crates.io (`crate:<name>` form). */
export function isCrateSpec(spec: string): boolean {
  return spec.startsWith('crate:')
}

/** Wrap a bare crate name as the lockfile identity name. */
export function toCrateIdentity(crateName: string): string {
  return `crate:${crateName}`
}

/**
 * Split "package@tag" into name and optional tag.
 * Handles scoped packages: "@scope/pkg@tag"
 */
function splitPackageTag(spec: string): { name: string, tag?: string } {
  // Scoped: @scope/pkg@tag → find the @ after the scope
  if (spec.startsWith('@')) {
    const slashIdx = spec.indexOf('/')
    if (slashIdx !== -1) {
      const afterSlash = spec.indexOf('@', slashIdx)
      if (afterSlash !== -1)
        return { name: spec.slice(0, afterSlash), tag: spec.slice(afterSlash + 1) || undefined }
    }
    return { name: spec }
  }
  // Unscoped: pkg@tag
  const atIdx = spec.indexOf('@')
  if (atIdx !== -1)
    return { name: spec.slice(0, atIdx), tag: spec.slice(atIdx + 1) || undefined }
  return { name: spec }
}
