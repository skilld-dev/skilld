/**
 * Prefix-based input parser for `skilld add`
 *
 * All sources require an explicit prefix:
 *   npm:vue         → package skill from registry
 *   crate:serde     → Rust crate from crates.io
 *   gh:owner/repo   → git skill
 *   github:o/r      → git skill (alias)
 *   @handle          → curator's skills
 *   @handle/coll     → specific collection
 *
 * Bare names (no prefix) are deprecated but still resolve as npm: with a warning.
 */

import type { GitSkillSource } from '../sources/git-skills.ts'
import { parseGitSkillInput } from '../sources/git-skills.ts'

const STATIC_REGEX_1 = /^[\w.-]+\/[\w.-]+/

export type SkillSource
  = | { type: 'npm', package: string, tag?: string }
    | { type: 'crate', package: string, version?: string }
    | { type: 'git', source: GitSkillSource, skillFilter?: string }
    | { type: 'curator', handle: string }
    | { type: 'collection', handle: string, name: string }
    | { type: 'bare', package: string, tag?: string }

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

  // @handle (curator) or @scope/pkg (npm scoped package)
  if (trimmed.startsWith('@')) {
    const rest = trimmed.slice(1)
    const slashIdx = rest.indexOf('/')
    if (slashIdx === -1) {
      return { type: 'curator', handle: rest }
    }
    // @scope/pkg → treat as npm scoped package (bare, deprecated form)
    // Collections must be installed via npm:@handle/coll or a future prefix.
    const { name, tag } = splitPackageTag(trimmed)
    return { type: 'bare', package: name, tag }
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
 * Strips `npm:` / `gh:` prefixes. Returns null for curator/collection (not addressable
 * as a single skill name).
 */
export function resolveSkillName(input: string): string | null {
  const source = parseSkillInput(input)
  switch (source.type) {
    case 'npm':
    case 'bare':
      return source.package
    case 'crate':
      return `crate:${source.package}`
    case 'git':
      if (source.source.type === 'github' && source.source.repo)
        return source.source.repo
      return null
    case 'curator':
    case 'collection':
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
