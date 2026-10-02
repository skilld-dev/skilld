import { z } from 'zod'
import { defineListResponse, defineOperation, defineRegistry, defineResponseObject, pageQueryShape } from './core'
import {
  collectionSlugSchema,
  countSchema,
  exampleSkillSummary,
  loginSchema,
  ownerSchema,
  repositorySchema,
  skillNameSchema,
  skillSummaryShape,
  urlSchema,
} from './schemas'

/*
 * A nested object keeps the same split as the top level: the server rejects a
 * field the contract does not name, and the SDK keeps one it does not know.
 */

/**
 * Slugs that already name a static route under `/@login/`. A collection with
 * one of them would be created and then never reached, because the static
 * route wins.
 */
export const RESERVED_COLLECTION_SLUGS: ReadonlySet<string> = new Set(['liked'])

const curatorParams = z.strictObject({ login: loginSchema })

const collectionParams = z.strictObject({ login: loginSchema, slug: collectionSlugSchema })

const collectionSkillParams = z.strictObject({
  login: loginSchema,
  slug: collectionSlugSchema,
  owner: ownerSchema,
  repository: repositorySchema,
  name: skillNameSchema,
})

/** Why the curator picked the Skill, in their own words. */
const reasonSchema = z.string().trim().min(1).max(2000)

const curatorIdentityFields = {
  login: loginSchema,
  /** The GitHub profile name, when the account has one. */
  name: z.string().nullable(),
  avatarUrl: urlSchema.nullable(),
}

const collectionFields = {
  slug: collectionSlugSchema,
  title: z.string(),
  /** The curator's introduction to the collection. */
  description: z.string().nullable(),
  pageUrl: urlSchema,
  /** Installs every Skill the collection names. */
  installCommand: z.string(),
}

const curatorIdentitySchema = defineResponseObject(curatorIdentityFields)

/** One Skill in a collection, with the curator's reason for it. */
export const collectionSkillSchema = defineResponseObject({
  ...skillSummaryShape,
  reason: z.string().nullable(),
})

const collectionSkillPage = defineListResponse(collectionSkillSchema)

export const collectionDetailSchema = {
  producer: z.strictObject({ ...collectionFields, curator: curatorIdentitySchema.producer, skills: collectionSkillPage.producer }),
  client: z.looseObject({ ...collectionFields, curator: curatorIdentitySchema.client, skills: collectionSkillPage.client }),
}

const collectionSummarySchema = defineResponseObject({
  ...collectionFields,
  /** Every Skill the collection names, also a Skill the registry cannot show now. */
  skillCount: countSchema,
})

const curatorFields = {
  ...curatorIdentityFields,
  pageUrl: urlSchema,
}

export const curatorListItemSchema = defineResponseObject({
  ...curatorFields,
  collectionCount: countSchema,
})

const curatorDetailFields = {
  ...curatorFields,
  /** Installs every Skill that the curator's collections name. */
  installCommand: z.string(),
}

export const curatorDetailSchema = {
  producer: z.strictObject({ ...curatorDetailFields, collections: z.array(collectionSummarySchema.producer) }),
  client: z.looseObject({ ...curatorDetailFields, collections: z.array(collectionSummarySchema.client) }),
}

const collectionSkillEntry = z.strictObject({
  owner: ownerSchema,
  repository: repositorySchema,
  name: skillNameSchema,
  reason: reasonSchema.nullable().optional(),
})

export const createCollectionBody = z.strictObject({
  slug: collectionSlugSchema.refine(slug => !RESERVED_COLLECTION_SLUGS.has(slug), 'is reserved'),
  title: z.string().trim().min(1).max(120),
  description: z.string().trim().min(1).max(4000).nullable().optional(),
  skills: z.array(collectionSkillEntry)
    .max(100)
    .refine(
      skills => new Set(skills.map(skill => `${skill.owner}/${skill.repository}/${skill.name}`)).size === skills.length,
      'must name each Skill once',
    )
    .default([]),
})

const exampleCurator = {
  login: 'harlan-zw',
  name: 'Harlan Wilton',
  avatarUrl: 'https://avatars.githubusercontent.com/u/5326365?v=4',
} as const

const exampleCollectionFields = {
  slug: 'design-engineering-essentials',
  title: 'Design Engineering Essentials',
  description: 'The Skills I give an agent before it touches interface code.',
  pageUrl: 'https://skilld.dev/@harlan-zw/design-engineering-essentials',
  installCommand: 'npx skilld add @harlan-zw/design-engineering-essentials',
} as const

const exampleCollectionSkill = {
  ...exampleSkillSummary,
  reason: 'Checks interface code against a published list of rules before review.',
} as const

const exampleCollection = {
  ...exampleCollectionFields,
  curator: exampleCurator,
  skills: { items: [exampleCollectionSkill], total: 1 },
} as const

export const curatorsV1 = defineRegistry({
  namespace: 'curators',
  description: 'Read the curators who assemble collections on skilld.dev, their collections, and the Skills they like.',
  operations: {
    list: defineOperation({
      id: 'curators.list',
      method: 'GET',
      path: '/api/v1/curators',
      access: 'public',
      semantics: { kind: 'query' },
      cache: { _tag: 'public', maxAgeSeconds: 300, staleWhileRevalidateSeconds: 3600 },
      request: {
        params: null,
        query: z.object(pageQueryShape({ defaultLimit: 30, maxLimit: 60 })),
        body: null,
      },
      response: { status: 200, body: defineListResponse(curatorListItemSchema) },
      errors: ['INVALID_REQUEST'],
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'List curators',
        description: 'The skilld.dev/community directory. It holds the first 60 curators: curators with a featured collection come first, then the rest by GitHub stars. A curator who publishes Skills but no collection yet has a `collectionCount` of 0.',
        tag: 'Curators',
        examples: [{
          request: { query: { limit: 1 } },
          response: {
            items: [{ ...exampleCurator, pageUrl: 'https://skilld.dev/@harlan-zw', collectionCount: 2 }],
            total: 41,
          },
        }],
      },
    }),
    get: defineOperation({
      id: 'curators.get',
      method: 'GET',
      path: '/api/v1/curators/{login}',
      access: 'public',
      semantics: { kind: 'query' },
      cache: { _tag: 'public', maxAgeSeconds: 60, staleWhileRevalidateSeconds: 300 },
      request: { params: curatorParams, query: null, body: null },
      response: { status: 200, body: curatorDetailSchema },
      errors: ['INVALID_REQUEST', 'NOT_FOUND'],
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'Get a curator',
        description: 'One curator and their collections, newest first. `installCommand` installs every Skill that the collections name. If no skilld.dev account has the login, the answer is NOT_FOUND.',
        tag: 'Curators',
        examples: [{
          request: { params: { login: 'harlan-zw' } },
          response: {
            ...exampleCurator,
            pageUrl: 'https://skilld.dev/@harlan-zw',
            installCommand: 'npx skilld add @harlan-zw',
            collections: [{ ...exampleCollectionFields, skillCount: 8 }],
          },
        }],
      },
    }),
    likes: defineOperation({
      id: 'curators.likes',
      method: 'GET',
      path: '/api/v1/curators/{login}/likes',
      access: 'public',
      semantics: { kind: 'query' },
      cache: { _tag: 'public', maxAgeSeconds: 60, staleWhileRevalidateSeconds: 300 },
      request: {
        params: curatorParams,
        query: z.object(pageQueryShape({ defaultLimit: 50, maxLimit: 100 })),
        body: null,
      },
      response: { status: 200, body: defineListResponse(defineResponseObject(skillSummaryShape)) },
      errors: ['INVALID_REQUEST', 'NOT_FOUND'],
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'List the Skills a curator likes',
        description: 'The 200 Skills the curator liked most recently, newest like first: the list at skilld.dev/@login/liked. A curator can keep the list private. If the list is private, or no account has the login, the answer is NOT_FOUND. This operation never shows a private list, not even to its owner. Use `likes.list` for your own likes.',
        tag: 'Curators',
        examples: [{
          request: { params: { login: 'harlan-zw' }, query: { limit: 1 } },
          response: { items: [exampleSkillSummary], total: 12 },
        }],
      },
    }),
  },
})

export const collectionsV1 = defineRegistry({
  namespace: 'collections',
  description: 'Read a collection, build your own, and watch one so the digest reports changes to its Skills.',
  operations: {
    get: defineOperation({
      id: 'collections.get',
      method: 'GET',
      path: '/api/v1/collections/{login}/{slug}',
      access: 'public',
      semantics: { kind: 'query' },
      cache: { _tag: 'public', maxAgeSeconds: 60, staleWhileRevalidateSeconds: 300 },
      request: {
        params: collectionParams,
        query: z.object(pageQueryShape({ defaultLimit: 100, maxLimit: 100 })),
        body: null,
      },
      response: { status: 200, body: collectionDetailSchema },
      errors: ['INVALID_REQUEST', 'NOT_FOUND'],
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'Get a collection',
        description: 'One collection at skilld.dev/@login/slug, with its Skills in the curator\'s order and the reason for each. A collection can name a whole Repository: that entry shows the Repository\'s most recently changed Skill. A Skill the registry cannot show now, for example because its Repository is broken, is left out of `skills`.',
        tag: 'Collections',
        examples: [{
          request: { params: { login: 'harlan-zw', slug: 'design-engineering-essentials' } },
          response: exampleCollection,
        }],
      },
    }),
    create: defineOperation({
      id: 'collections.create',
      method: 'POST',
      path: '/api/v1/collections',
      access: 'account',
      semantics: { kind: 'mutation', retry: 'never' },
      cache: { _tag: 'private' },
      request: { params: null, query: null, body: createCollectionBody },
      response: { status: 201, body: collectionDetailSchema },
      errors: ['INVALID_REQUEST', 'AUTH_REQUIRED', 'CONFLICT'],
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'Create a collection',
        description: 'Creates a collection at skilld.dev/@your-login/slug, with up to 100 Skills in the order you send them. Every Skill must be in the registry, or the answer is INVALID_REQUEST. If you already have a collection with the slug, the answer is CONFLICT.',
        tag: 'Collections',
        examples: [{
          request: {
            body: {
              slug: 'design-engineering-essentials',
              title: 'Design Engineering Essentials',
              description: 'The Skills I give an agent before it touches interface code.',
              skills: [{
                owner: 'vercel-labs',
                repository: 'agent-skills',
                name: 'web-design-guidelines',
                reason: 'Checks interface code against a published list of rules before review.',
              }],
            },
          },
          response: exampleCollection,
        }],
      },
    }),
    addSkill: defineOperation({
      id: 'collections.skills.add',
      method: 'PUT',
      path: '/api/v1/collections/{login}/{slug}/skills/{owner}/{repository}/{name}',
      access: 'account',
      semantics: { kind: 'mutation', retry: 'idempotent' },
      cache: { _tag: 'private' },
      request: {
        params: collectionSkillParams,
        query: null,
        body: z.strictObject({ reason: reasonSchema.nullable().optional() }).default({}),
      },
      response: { status: 200, body: collectionSkillSchema },
      errors: ['INVALID_REQUEST', 'AUTH_REQUIRED', 'FORBIDDEN', 'NOT_FOUND'],
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'Add a Skill to a collection',
        description: 'Adds the Skill to the end of one of your collections, and answers the collection entry. If the collection already has the Skill, its place stays the same. Send `reason` to set the reason, or `null` to clear it. If you leave out `reason`, the stored reason stays. Only the curator can change a collection: for another login the answer is FORBIDDEN.',
        tag: 'Collections',
        examples: [{
          request: {
            params: { login: 'harlan-zw', slug: 'design-engineering-essentials', owner: 'vercel-labs', repository: 'agent-skills', name: 'web-design-guidelines' },
            body: { reason: 'Checks interface code against a published list of rules before review.' },
          },
          response: exampleCollectionSkill,
        }],
      },
    }),
    removeSkill: defineOperation({
      id: 'collections.skills.remove',
      method: 'DELETE',
      path: '/api/v1/collections/{login}/{slug}/skills/{owner}/{repository}/{name}',
      access: 'account',
      semantics: { kind: 'mutation', retry: 'idempotent' },
      cache: { _tag: 'private' },
      request: { params: collectionSkillParams, query: null, body: null },
      response: { status: 204, body: null },
      errors: ['INVALID_REQUEST', 'AUTH_REQUIRED', 'FORBIDDEN', 'NOT_FOUND'],
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'Remove a Skill from a collection',
        description: 'Removes the Skill from one of your collections. If the Skill shows for a whole-Repository entry, that entry goes. If the collection does not have the Skill, nothing changes. Only the curator can change a collection: for another login the answer is FORBIDDEN.',
        tag: 'Collections',
        examples: [{
          request: { params: { login: 'harlan-zw', slug: 'design-engineering-essentials', owner: 'vercel-labs', repository: 'agent-skills', name: 'web-design-guidelines' } },
          response: null,
        }],
      },
    }),
    watch: defineOperation({
      id: 'collections.watch',
      method: 'PUT',
      path: '/api/v1/collections/{login}/{slug}/watch',
      access: 'account',
      semantics: { kind: 'mutation', retry: 'idempotent' },
      cache: { _tag: 'private' },
      request: { params: collectionParams, query: null, body: null },
      response: {
        status: 200,
        body: defineResponseObject({
          /** The Repositories of the collection, all of which you now watch. */
          watched: countSchema,
        }),
      },
      errors: ['INVALID_REQUEST', 'AUTH_REQUIRED', 'NOT_FOUND'],
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'Watch a collection',
        description: 'Watches every Repository the collection names, so your digest reports their changes. A Repository you already watch stays watched. A Repository added to the collection later is not watched until you send this again.',
        tag: 'Collections',
        examples: [{
          request: { params: { login: 'harlan-zw', slug: 'design-engineering-essentials' } },
          response: { watched: 8 },
        }],
      },
    }),
  },
})
