/**
 * Shared defaults and factory for agent target definitions.
 * All targets share identical skillFilename, nameMatchesDir, namePattern,
 * and common frontmatter fields from the agentskills.io spec.
 */

import type { AgentTarget, FrontmatterField } from './types.ts'

/** Common frontmatter fields from agentskills.io spec */
export const SPEC_FRONTMATTER: Record<string, FrontmatterField> = {
  'name': { name: 'name', required: true, description: 'Skill identifier. Must match parent directory name.', constraints: '1-64 chars, lowercase alphanumeric + hyphens' },
  'description': { name: 'description', required: true, description: 'What the skill does and when to use it.', constraints: '1-1024 chars' },
  'license': { name: 'license', required: false, description: 'License reference' },
  'compatibility': { name: 'compatibility', required: false, description: 'Environment requirements', constraints: 'max 500 chars' },
  'metadata': { name: 'metadata', required: false, description: 'Arbitrary key-value pairs' },
  'allowed-tools': { name: 'allowed-tools', required: false, description: 'Space-delimited pre-approved tools (experimental)' },
}

/** Shared defaults for all agent targets */
const BASE_DEFAULTS = {
  skillFilename: 'SKILL.md' as const,
  nameMatchesDir: true,
  namePattern: '^[a-z0-9]+(-[a-z0-9]+)*$',
  additionalSkillsDirs: [] as string[],
  extensions: [] as string[],
  notes: [] as string[],
} satisfies Partial<AgentTarget>

type DefaultedFields = 'skillFilename' | 'nameMatchesDir' | 'namePattern' | 'additionalSkillsDirs' | 'extensions' | 'notes'

/**
 * Define an agent target with shared defaults applied.
 *
 * `globalSkillsDir` and `additionalSkillsDirs` may be provided as thunks
 * so env vars and `homedir()` are evaluated lazily (at access time, not
 * module load). Exposed as plain `string` / `string[]` via getters.
 */
export function defineTarget(
  target:
    & Omit<AgentTarget, DefaultedFields | 'globalSkillsDir'>
    & Partial<Pick<AgentTarget, Exclude<DefaultedFields, 'additionalSkillsDirs'>>>
    & {
      globalSkillsDir: () => string
      additionalSkillsDirs?: string[] | (() => string[])
    },
): AgentTarget {
  const { globalSkillsDir, additionalSkillsDirs, ...rest } = target
  const additional = additionalSkillsDirs ?? BASE_DEFAULTS.additionalSkillsDirs
  return {
    ...BASE_DEFAULTS,
    ...rest,
    get globalSkillsDir() {
      return globalSkillsDir()
    },
    get additionalSkillsDirs() {
      return typeof additional === 'function' ? additional() : additional
    },
  }
}
