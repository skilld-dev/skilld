import { z } from 'zod'
import { defineListResponse, defineOperation, defineRegistry, defineResponseObject, pageQueryShape } from './core'
import {
  countSchema,
  exampleSkillSummary,
  isoDateTimeSchema,
  ownerSchema,
  repositoryParams,
  repositorySchema,
  skillSummarySchema,
  skillSummaryShape,
  urlSchema,
} from './schemas'

/*
 * A nested object keeps the same split as the top level: the server rejects a
 * field the contract does not name, and the SDK keeps one it does not know.
 * So every schema below that nests an object builds its producer and client
 * from the nested producer and client.
 */

// ---------------------------------------------------------------------------
// repositories
// ---------------------------------------------------------------------------

const repositoryProfileFields = {
  owner: ownerSchema,
  repository: repositorySchema,
  /** The Repository description on GitHub. */
  description: z.string().nullable(),
  stars: countSchema,
  /** The Repository's last push, as the registry last read it. */
  pushedAt: isoDateTimeSchema.nullable(),
  repositoryUrl: urlSchema,
  pageUrl: urlSchema,
  /** Installs every Skill in the Repository. */
  installCommand: z.string(),
}

export const repositoryProfileSchema = {
  producer: z.strictObject({ ...repositoryProfileFields, skills: z.array(skillSummarySchema.producer) }),
  client: z.looseObject({ ...repositoryProfileFields, skills: z.array(skillSummarySchema.client) }),
}

export const repositoriesV1 = defineRegistry({
  namespace: 'repositories',
  description: 'Read one Repository and every Skill the registry holds from it.',
  operations: {
    get: defineOperation({
      id: 'repositories.get',
      method: 'GET',
      path: '/api/v1/repositories/{owner}/{repository}',
      access: 'public',
      semantics: { kind: 'query' },
      cache: { _tag: 'public', maxAgeSeconds: 60, staleWhileRevalidateSeconds: 300 },
      request: { params: repositoryParams, query: null, body: null },
      response: { status: 200, body: repositoryProfileSchema },
      errors: ['INVALID_REQUEST', 'NOT_FOUND'],
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'Get a Repository',
        description: 'One Repository and every Skill the registry holds from it, most recently changed first. `installCommand` installs all of them. If the registry holds no Skill from the Repository, the answer is NOT_FOUND: send an index request to add it.',
        tag: 'Repositories',
        examples: [{
          request: { params: { owner: 'vercel-labs', repository: 'agent-skills' } },
          response: {
            owner: 'vercel-labs',
            repository: 'agent-skills',
            description: 'Skills for AI coding agents.',
            stars: 18_204,
            pushedAt: '2026-09-30T09:12:44.000Z',
            repositoryUrl: 'https://github.com/vercel-labs/agent-skills',
            pageUrl: 'https://skilld.dev/gh/vercel-labs/agent-skills',
            installCommand: 'npx skilld add vercel-labs/agent-skills',
            skills: [exampleSkillSummary],
          },
        }],
      },
    }),
  },
})

// ---------------------------------------------------------------------------
// indexRequests
// ---------------------------------------------------------------------------

/**
 * `owner/repository`, or a URL. The server reads a URL the way the skilld.dev
 * search box does: a `tree` or `blob` link, a `.git` suffix, or a trailing
 * slash still names the Repository, and a host other than github.com is
 * INVALID_REQUEST. No `i` flag, because JSON Schema patterns cannot carry one.
 */
const REPOSITORY_REFERENCE = /^(?:[\w.-]+\/[\w.-]+|https?:\/\/\S+)$/

const indexProgressSchema = z.discriminatedUnion('stage', [
  /** The request waits for a worker. */
  z.strictObject({ stage: z.literal('queued') }),
  /** A worker reads the Repository tree. */
  z.strictObject({ stage: z.literal('checking') }),
  /** A worker indexes the Skills it found, `indexed` of `total` so far. */
  z.strictObject({ stage: z.literal('indexing'), indexed: countSchema, total: countSchema }),
])
const indexProgressClientSchema = z.discriminatedUnion('stage', [
  z.looseObject({ stage: z.literal('queued') }),
  z.looseObject({ stage: z.literal('checking') }),
  z.looseObject({ stage: z.literal('indexing'), indexed: countSchema, total: countSchema }),
])

const indexTargetFields = {
  owner: ownerSchema,
  repository: repositorySchema,
}

const indexQueuedFields = {
  status: z.literal('queued'),
  /** Poll `indexRequests.get` with this until the status changes. */
  id: z.uuid(),
  ...indexTargetFields,
}
const indexIndexedFields = {
  status: z.literal('indexed'),
  ...indexTargetFields,
}
const indexFailedFields = {
  status: z.literal('failed'),
  ...indexTargetFields,
  /** Why the registry could not index the Repository, in words a person can act on. */
  reason: z.string(),
}

const indexQueued = {
  producer: z.strictObject({ ...indexQueuedFields, progress: indexProgressSchema }),
  client: z.looseObject({ ...indexQueuedFields, progress: indexProgressClientSchema }),
}
const indexIndexed = {
  producer: z.strictObject({ ...indexIndexedFields, skills: z.array(skillSummarySchema.producer) }),
  client: z.looseObject({ ...indexIndexedFields, skills: z.array(skillSummarySchema.client) }),
}
const indexFailed = {
  producer: z.strictObject(indexFailedFields),
  client: z.looseObject(indexFailedFields),
}

export const indexRequestCreatedSchema = {
  producer: z.discriminatedUnion('status', [indexIndexed.producer, indexQueued.producer]),
  client: z.discriminatedUnion('status', [indexIndexed.client, indexQueued.client]),
}

export const indexRequestSchema = {
  producer: z.discriminatedUnion('status', [indexQueued.producer, indexIndexed.producer, indexFailed.producer]),
  client: z.discriminatedUnion('status', [indexQueued.client, indexIndexed.client, indexFailed.client]),
}

const exampleIndexRequestId = '0f8b5c1e-3d4a-4f6b-9a2c-7e1d5b8c9a04'

export const indexRequestsV1 = defineRegistry({
  namespace: 'indexRequests',
  description: 'Ask the registry to index a GitHub Repository, then poll until its Skills are in.',
  operations: {
    create: defineOperation({
      id: 'index_requests.create',
      method: 'POST',
      path: '/api/v1/index-requests',
      access: 'public',
      semantics: { kind: 'mutation', retry: 'never' },
      cache: { _tag: 'private' },
      request: {
        params: null,
        query: null,
        body: z.strictObject({
          repository: z.string().trim().min(3).max(2048).regex(REPOSITORY_REFERENCE, 'must be owner/repository or a github.com URL'),
        }),
      },
      response: { status: 201, body: indexRequestCreatedSchema },
      errors: ['INVALID_REQUEST'],
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'Index a Repository',
        description: 'If the registry already holds Skills from the Repository, the status is `indexed` and the answer lists them. If not, the status is `queued`: poll `indexRequests.get` with the `id`. A second request for a Repository that is already queued answers the same `id`.',
        tag: 'Index requests',
        examples: [
          {
            request: { body: { repository: 'https://github.com/vercel-labs/agent-skills' } },
            response: {
              status: 'queued',
              id: exampleIndexRequestId,
              owner: 'vercel-labs',
              repository: 'agent-skills',
              progress: { stage: 'queued' },
            },
          },
          {
            request: { body: { repository: 'vercel-labs/agent-skills' } },
            response: {
              status: 'indexed',
              owner: 'vercel-labs',
              repository: 'agent-skills',
              skills: [exampleSkillSummary],
            },
          },
        ],
      },
    }),
    get: defineOperation({
      id: 'index_requests.get',
      method: 'GET',
      path: '/api/v1/index-requests/{id}',
      access: 'public',
      semantics: { kind: 'query' },
      // A poll target. Its state moves every second, so no cache may hold it.
      cache: { _tag: 'private' },
      request: { params: z.strictObject({ id: z.uuid() }), query: null, body: null },
      response: { status: 200, body: indexRequestSchema },
      errors: ['INVALID_REQUEST', 'NOT_FOUND'],
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'Get an index request',
        description: 'The state of one index request. Poll while the status is `queued`. `indexed` lists the Skills the registry found. `failed` gives the reason.',
        tag: 'Index requests',
        examples: [
          {
            request: { params: { id: exampleIndexRequestId } },
            response: {
              status: 'queued',
              id: exampleIndexRequestId,
              owner: 'vercel-labs',
              repository: 'agent-skills',
              progress: { stage: 'indexing', indexed: 3, total: 6 },
            },
          },
          {
            request: { params: { id: exampleIndexRequestId } },
            response: {
              status: 'failed',
              owner: 'vercel-labs',
              repository: 'agent-skills',
              reason: 'No supported SKILL.md files were found.',
            },
          },
        ],
      },
    }),
  },
})

// ---------------------------------------------------------------------------
// owners
// ---------------------------------------------------------------------------

const ownerRepositorySchema = defineResponseObject({
  repository: repositorySchema,
  description: z.string().nullable(),
  stars: countSchema,
  /** The Skills the registry holds from this Repository. */
  skillCount: countSchema,
  pageUrl: urlSchema,
})

const ownerFields = {
  login: ownerSchema,
  /** The GitHub profile name, when the registry has read one. */
  name: z.string().nullable(),
  avatarUrl: urlSchema,
  kind: z.enum(['user', 'organization']),
  pageUrl: urlSchema,
}

export const ownerProfileSchema = {
  producer: z.strictObject({ ...ownerFields, repositories: z.array(ownerRepositorySchema.producer) }),
  client: z.looseObject({ ...ownerFields, repositories: z.array(ownerRepositorySchema.client) }),
}

export const ownersV1 = defineRegistry({
  namespace: 'owners',
  description: 'Read one GitHub Owner and the Repositories it publishes Skills from.',
  operations: {
    get: defineOperation({
      id: 'owners.get',
      method: 'GET',
      path: '/api/v1/owners/{owner}',
      access: 'public',
      semantics: { kind: 'query' },
      cache: { _tag: 'public', maxAgeSeconds: 60, staleWhileRevalidateSeconds: 300 },
      request: { params: z.strictObject({ owner: ownerSchema }), query: null, body: null },
      response: { status: 200, body: ownerProfileSchema },
      errors: ['INVALID_REQUEST', 'NOT_FOUND'],
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'Get an Owner',
        description: 'One GitHub organization or user, and each Repository it publishes Skills from, the Repository with the most Skills first. If the registry holds no Skill from the Owner, the answer is NOT_FOUND.',
        tag: 'Owners',
        examples: [{
          request: { params: { owner: 'vercel-labs' } },
          response: {
            login: 'vercel-labs',
            name: 'Vercel Labs',
            avatarUrl: 'https://github.com/vercel-labs.png',
            kind: 'organization',
            pageUrl: 'https://skilld.dev/gh/vercel-labs',
            repositories: [{
              repository: 'agent-skills',
              description: 'Skills for AI coding agents.',
              stars: 18_204,
              skillCount: 6,
              pageUrl: 'https://skilld.dev/gh/vercel-labs/agent-skills',
            }],
          },
        }],
      },
    }),
  },
})

// ---------------------------------------------------------------------------
// tracks
// ---------------------------------------------------------------------------

export const trackSlugSchema = z.string().min(1).max(64).regex(/^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/, 'must be lowercase letters, digits, and hyphens')

const trackFields = {
  slug: trackSlugSchema,
  label: z.string(),
  /** The second-person line a person wrote for the track. */
  line: z.string(),
  pageUrl: urlSchema,
}

export const trackSummarySchema = defineResponseObject({
  ...trackFields,
  skillCount: countSchema,
})

export const trackDetailSchema = {
  producer: z.strictObject({ ...trackFields, items: z.array(skillSummarySchema.producer), total: countSchema }),
  client: z.looseObject({ ...trackFields, items: z.array(skillSummarySchema.client), total: countSchema }),
}

const exampleTrack = {
  slug: 'design',
  label: 'Design and interface work',
  line: 'You care how the interface looks, moves, and reads.',
  pageUrl: 'https://skilld.dev/skills/design',
} as const

export const tracksV1 = defineRegistry({
  namespace: 'tracks',
  description: 'Tracks: Skills for one kind of work. A person writes each label and line and pins the Skills that lead it.',
  operations: {
    list: defineOperation({
      id: 'tracks.list',
      method: 'GET',
      path: '/api/v1/tracks',
      access: 'public',
      semantics: { kind: 'query' },
      cache: { _tag: 'public', maxAgeSeconds: 300, staleWhileRevalidateSeconds: 3600 },
      request: { params: null, query: null, body: null },
      response: { status: 200, body: defineListResponse(trackSummarySchema) },
      errors: [],
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'List tracks',
        description: 'Every track that holds at least one Skill, the track with the most Skills first. The list is short, so it has no pages.',
        tag: 'Tracks',
        examples: [{
          request: {},
          response: { items: [{ ...exampleTrack, skillCount: 41 }], total: 14 },
        }],
      },
    }),
    get: defineOperation({
      id: 'tracks.get',
      method: 'GET',
      path: '/api/v1/tracks/{slug}',
      access: 'public',
      semantics: { kind: 'query' },
      cache: { _tag: 'public', maxAgeSeconds: 300, staleWhileRevalidateSeconds: 3600 },
      request: {
        params: z.strictObject({ slug: trackSlugSchema }),
        query: z.object(pageQueryShape({ defaultLimit: 20, maxLimit: 100 })),
        body: null,
      },
      response: { status: 200, body: trackDetailSchema },
      errors: ['INVALID_REQUEST', 'NOT_FOUND'],
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'Get a track',
        description: 'One track and its Skills, in the order its page shows them: the pinned Skills first, then the rest by stars. `total` counts every Skill in the track.',
        tag: 'Tracks',
        examples: [{
          request: { params: { slug: 'design' }, query: { limit: 1 } },
          response: { ...exampleTrack, items: [exampleSkillSummary], total: 41 },
        }],
      },
    }),
  },
})

// ---------------------------------------------------------------------------
// trending
// ---------------------------------------------------------------------------

const trendingPostSchema = defineResponseObject({
  url: urlSchema,
  platform: z.enum(['x', 'bsky']),
  authorHandle: z.string(),
  /** The display name the network sent with the post, if any. */
  authorName: z.string().nullable(),
  text: z.string(),
  postedAt: isoDateTimeSchema,
})

const socialFields = {
  /** Separate accounts that posted about the Skill in the window. */
  authorCount: countSchema,
  mentionCount: countSchema,
}
const starSurgeFields = {
  /** Stars the Repository gained on the surge day. */
  starGain: countSchema,
  /** UTC midnight of the surge day. */
  surgedOn: isoDateTimeSchema,
}

/**
 * Why a Skill is on the board. ADR-0004 makes each row state its own reason,
 * and a row a post put there ships that post.
 *
 * - `social`: devs posted about the Skill on X or Bluesky. `post` is one of those posts.
 * - `star-surge`: the Repository gained stars fast, and it holds only this Skill.
 * - `social-and-star-surge`: both of the above.
 * - `star-count`: the socials were quiet, so the board filled up with a well-starred Skill. Nobody posted about it.
 */
export const trendingSignalSchema = {
  producer: z.discriminatedUnion('kind', [
    z.strictObject({ kind: z.literal('social'), ...socialFields, post: trendingPostSchema.producer }),
    z.strictObject({ kind: z.literal('star-surge'), ...starSurgeFields }),
    z.strictObject({ kind: z.literal('social-and-star-surge'), ...socialFields, post: trendingPostSchema.producer, ...starSurgeFields }),
    z.strictObject({ kind: z.literal('star-count') }),
  ]),
  client: z.discriminatedUnion('kind', [
    z.looseObject({ kind: z.literal('social'), ...socialFields, post: trendingPostSchema.client }),
    z.looseObject({ kind: z.literal('star-surge'), ...starSurgeFields }),
    z.looseObject({ kind: z.literal('social-and-star-surge'), ...socialFields, post: trendingPostSchema.client, ...starSurgeFields }),
    z.looseObject({ kind: z.literal('star-count') }),
  ]),
}

export const trendingSkillSchema = {
  producer: z.strictObject({ ...skillSummaryShape, signal: trendingSignalSchema.producer }),
  client: z.looseObject({ ...skillSummaryShape, signal: trendingSignalSchema.client }),
}

/** The board's two windows, in the hours the ranking reads. */
export const TRENDING_WINDOW_HOURS = { week: 24 * 7, month: 24 * 30 } as const

export const trendingV1 = defineRegistry({
  namespace: 'trending',
  description: 'Skills devs talk about on X and Bluesky, each with the reason it is on the board (ADR-0004). Never ranked by installs or likes.',
  operations: {
    list: defineOperation({
      id: 'trending.list',
      method: 'GET',
      path: '/api/v1/trending',
      access: 'public',
      semantics: { kind: 'query' },
      cache: { _tag: 'public', maxAgeSeconds: 300, staleWhileRevalidateSeconds: 600 },
      request: {
        params: null,
        query: z.object({
          window: z.enum(['week', 'month']).default('week'),
          limit: z.coerce.number().int().min(1).max(30).default(30),
        }),
        body: null,
      },
      response: { status: 200, body: defineListResponse(trendingSkillSchema) },
      errors: ['INVALID_REQUEST'],
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'List trending Skills',
        description: 'The trending board at skilld.dev/skills/trending, in rank order. Rows with a `social` or `star-surge` reason always rank above `star-count` rows. `total` counts the whole board, at most 30 rows.',
        tag: 'Trending',
        examples: [{
          request: { query: { window: 'week', limit: 1 } },
          response: {
            items: [{
              ...exampleSkillSummary,
              signal: {
                kind: 'social',
                authorCount: 3,
                mentionCount: 4,
                post: {
                  url: 'https://x.com/ada_ships/status/1972000000000000000',
                  platform: 'x',
                  authorHandle: 'ada_ships',
                  authorName: 'Ada',
                  text: 'web-design-guidelines catches the UI mistakes I used to catch in review.',
                  postedAt: '2026-09-29T16:40:00.000Z',
                },
              },
            }],
            total: 30,
          },
        }],
      },
    }),
  },
})
