import { z } from 'zod'
import { defineListResponse, defineOperation, defineRegistry, defineResponseObject, pageQueryShape } from './core'
import {
  countSchema,
  exampleSkillSummary,
  isoDateTimeSchema,
  ownerSchema,
  skillParams,
  skillSummarySchema,
  skillSummaryShape,
  urlSchema,
} from './schemas'

/**
 * A Skill name as the Agent Skills specification allows it. Search answers
 * only names that pass, because the skilld CLI rejects any other.
 */
export const specSkillNameSchema = z.string()
  .max(64)
  .regex(/^[a-z0-9](?:[a-z0-9]|-(?!-)){0,62}[a-z0-9]$|^[a-z0-9]$/)

export const isSpecSkillName = (value: string): boolean => specSkillNameSchema.safeParse(value).success

const searchSource = z.strictObject({
  provider: z.literal('github'),
  owner: z.string().min(1).max(39),
  repository: z.string().min(1).max(100),
  selector: z.strictObject({
    type: z.literal('named-skill'),
    name: specSkillNameSchema,
  }),
})

/**
 * Frozen. skilld 3.2.0 parses this answer with `deny_unknown_fields`, so a new
 * field here fails every released `skilld search`. Add new fields to
 * `skills.get` instead.
 */
const searchItemShape = {
  name: specSkillNameSchema,
  description: z.string().max(500).nullable(),
  source: searchSource,
  stargazerCount: countSchema,
}

export const skillSearchResponse = {
  producer: z.strictObject({
    items: z.array(z.strictObject(searchItemShape)).max(50),
    total: countSchema,
  }),
  client: z.looseObject({
    items: z.array(z.looseObject(searchItemShape)).max(50),
    total: countSchema,
  }),
}

export const skillDetailSchema = defineResponseObject({
  ...skillSummaryShape,
  /** The GitHub profile name of the Owner, when the registry has synced it. */
  authorName: z.string().nullable(),
  license: z.string().nullable(),
  repositoryUrl: urlSchema,
  /** The SKILL.md path inside the Repository. */
  skillPath: z.string().nullable(),
  /** The commit the registry last read. */
  sourceCommit: z.string().nullable(),
  /** True when the SKILL.md is gone upstream. The registry keeps the last copy it read. */
  sourceGone: z.boolean(),
  /** The Repository's last push. `updatedAt` is the SKILL.md's own last change. */
  pushedAt: isoDateTimeSchema.nullable(),
  tags: z.array(z.string()),
  /** The `allowed-tools` the SKILL.md frontmatter asks for. */
  allowedTools: z.array(z.string()),
  /** Files beside the SKILL.md. `run --file` reads one. */
  files: z.array(z.strictObject({ path: z.string(), size: countSchema })),
  /** Machine-generated from the SKILL.md. Never written by the author. */
  generatedSummary: z.string().nullable(),
  /** The SKILL.md text the registry last read, frontmatter included. */
  markdown: z.string().nullable(),
})

/**
 * The skilld.dev/skills listing. Stars order it by default. Likes order it
 * only when the caller asks (ADR-0003), and install counts never do.
 */
export const skillBrowseQuery = z.object({
  q: z.string().trim().min(1).max(200).optional(),
  owner: ownerSchema.optional(),
  tag: z.string().trim().toLowerCase().min(1).max(64).regex(/^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/, 'must be a tag slug').optional(),
  sort: z.enum(['stars', 'likes', 'updated']).default('stars'),
  ...pageQueryShape({ defaultLimit: 20, maxLimit: 100 }),
})

export const skillsV1 = defineRegistry({
  namespace: 'skills',
  description: 'Search the registry and read one Skill with its provenance.',
  operations: {
    search: defineOperation({
      id: 'skills.search',
      method: 'GET',
      path: '/api/v1/skills',
      access: 'public',
      semantics: { kind: 'query' },
      cache: { _tag: 'public', maxAgeSeconds: 60, staleWhileRevalidateSeconds: 300 },
      request: {
        params: null,
        query: z.strictObject({
          q: z.string().trim().min(1).max(200),
          limit: z.coerce.number().int().min(1).max(50).default(20),
        }),
        body: null,
      },
      response: { status: 200, body: skillSearchResponse },
      errors: ['INVALID_REQUEST'],
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'Search Skills',
        description: 'Semantic search over admitted Skills. The answer shape is frozen for skilld 3.2.0, so use `skills.get` for anything beyond the source and the star count.',
        tag: 'Skills',
        examples: [{
          request: { query: { q: 'tailwind', limit: 1 } },
          response: {
            items: [{
              name: 'tailwind-v4',
              description: 'Tailwind CSS v4 patterns and migration notes.',
              source: { provider: 'github', owner: 'nuxt', repository: 'ui', selector: { type: 'named-skill', name: 'tailwind-v4' } },
              stargazerCount: 5_421,
            }],
            total: 12,
          },
        }],
      },
    }),
    get: defineOperation({
      id: 'skills.get',
      method: 'GET',
      path: '/api/v1/skills/{owner}/{repository}/{name}',
      access: 'public',
      semantics: { kind: 'query' },
      cache: { _tag: 'public', maxAgeSeconds: 60, staleWhileRevalidateSeconds: 300 },
      request: { params: skillParams, query: null, body: null },
      response: { status: 200, body: skillDetailSchema },
      errors: ['INVALID_REQUEST', 'NOT_FOUND'],
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'Get a Skill',
        description: 'One Skill with its provenance: the Owner, the exact SKILL.md, the commit the registry read, and the run and install commands.',
        tag: 'Skills',
        examples: [{
          request: { params: { owner: 'vercel-labs', repository: 'agent-skills', name: 'web-design-guidelines' } },
          response: {
            ...exampleSkillSummary,
            authorName: 'Vercel Labs',
            license: 'MIT',
            repositoryUrl: 'https://github.com/vercel-labs/agent-skills',
            skillPath: 'skills/web-design-guidelines/SKILL.md',
            sourceCommit: '4f1c2a9e0b7d3c5a8e6f1b2d4c7a9e0f3b5d8c1a',
            sourceGone: false,
            pushedAt: '2026-09-30T09:12:44.000Z',
            tags: ['design', 'accessibility'],
            allowedTools: [],
            files: [{ path: 'references/checklist.md', size: 4_210 }],
            generatedSummary: 'Checks interface code against a published list of web design rules.',
            markdown: '---\nname: web-design-guidelines\ndescription: Review UI code for compliance with web interface guidelines.\n---\n\n# Web Design Guidelines\n',
          },
        }],
      },
    }),
    browse: defineOperation({
      id: 'skills.browse',
      method: 'GET',
      path: '/api/v1/browse',
      access: 'public',
      semantics: { kind: 'query' },
      cache: { _tag: 'public', maxAgeSeconds: 60, staleWhileRevalidateSeconds: 300 },
      request: { params: null, query: skillBrowseQuery, body: null },
      response: { status: 200, body: defineListResponse(skillSummarySchema) },
      errors: ['INVALID_REQUEST'],
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'Browse Skills',
        description: 'The listing behind skilld.dev/skills, filtered by `owner` and `tag`. `sort` orders it by stars, likes, or the last SKILL.md change. With `q`, Skills rank by relevance to the query and `sort` does not apply. Use `tracks.get` for the Skills of one track.',
        tag: 'Skills',
        examples: [{
          request: { query: { owner: 'vercel-labs', sort: 'stars', limit: 1 } },
          response: { items: [exampleSkillSummary], total: 6 },
        }],
      },
    }),
  },
})
