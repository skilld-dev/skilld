import { z } from 'zod'
import { defineResponseObject } from './core'

/**
 * Field schemas shared by every registry. One field means one thing across
 * the whole contract, so a client reads `owner` or `pageUrl` the same way on
 * every operation.
 *
 * Conventions every operation follows:
 * - Field names are camelCase. Times are ISO 8601 strings. A known-absent value is `null`, never a missing key.
 * - A Skill always carries its provenance: `owner`, `repository`, and `sourceUrl`, the SKILL.md in the author's Repository (VISION principle 1).
 * - Machine-generated text is named for what it is: `generatedSummary`, never `summary`.
 * - Install counts never appear. Stars and likes are the only counts (VISION anti-scope 4).
 */

/** A GitHub login or organization name. */
export const ownerSchema = z.string().min(1).max(39).regex(/^[A-Z0-9](?:[A-Z0-9-]*[A-Z0-9])?$/i, 'must be a GitHub login')
export const loginSchema = ownerSchema
/** A GitHub Repository name. */
export const repositorySchema = z.string().min(1).max(100).regex(/^[\w.-]+$/, 'must be a GitHub Repository name')
/** A Skill directory name as the registry stores it. */
export const skillNameSchema = z.string().min(1).max(100).regex(/^[\w.-]+$/, 'must be a Skill name')
/** A collection slug, as it appears in `/@login/slug`. */
export const collectionSlugSchema = z.string().min(1).max(64).regex(/^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/, 'must be lowercase letters, digits, and hyphens')
export const isoDateTimeSchema = z.iso.datetime({ offset: true })
export const urlSchema = z.url()
export const countSchema = z.number().int().nonnegative()

export const skillParams = z.strictObject({
  owner: ownerSchema,
  repository: repositorySchema,
  name: skillNameSchema,
})

export const repositoryParams = z.strictObject({
  owner: ownerSchema,
  repository: repositorySchema,
})

/** The fields every Skill card carries, on every list. */
export const skillSummaryShape = {
  owner: ownerSchema,
  repository: repositorySchema,
  name: skillNameSchema,
  displayName: z.string(),
  description: z.string().nullable(),
  stars: countSchema,
  likes: countSchema,
  /** When the SKILL.md last changed upstream. */
  updatedAt: isoDateTimeSchema.nullable(),
  pageUrl: urlSchema,
  /** The SKILL.md in the author's Repository. `null` only while the first sync is pending. */
  sourceUrl: urlSchema.nullable(),
  runCommand: z.string(),
  installCommand: z.string(),
}

export const skillSummarySchema = defineResponseObject(skillSummaryShape)

export const exampleSkillSummary = {
  owner: 'vercel-labs',
  repository: 'agent-skills',
  name: 'web-design-guidelines',
  displayName: 'Web Design Guidelines',
  description: 'Review UI code for compliance with web interface guidelines.',
  stars: 18_204,
  likes: 41,
  updatedAt: '2026-09-28T14:02:11.000Z',
  pageUrl: 'https://skilld.dev/gh/vercel-labs/agent-skills/web-design-guidelines',
  sourceUrl: 'https://github.com/vercel-labs/agent-skills/blob/main/skills/web-design-guidelines/SKILL.md',
  runCommand: 'npx skilld run vercel-labs/agent-skills/web-design-guidelines',
  installCommand: 'npx skilld install vercel-labs/agent-skills/web-design-guidelines',
} as const
