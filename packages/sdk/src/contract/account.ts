import { z } from 'zod'
import { defineListResponse, defineOperation, defineRegistry, defineResponseObject, pageQueryShape } from './core'
import {
  countSchema,
  exampleSkillSummary,
  isoDateTimeSchema,
  loginSchema,
  ownerSchema,
  repositoryParams,
  repositorySchema,
  skillParams,
  skillSummaryShape,
  urlSchema,
} from './schemas'

/**
 * Loop 2: the signed-in account, its likes, its watches, its imported GitHub
 * stars, the changes its digest reports, and its skilld tokens.
 *
 * Every operation here needs a skilld.dev sign-in or a skilld token sent as a
 * Bearer credential, and no answer is cached. Account deletion is not in the
 * API: it needs a skilld.dev sign-in in the browser.
 */

const accountErrors = ['AUTH_REQUIRED'] as const
const accountInputErrors = ['INVALID_REQUEST', 'AUTH_REQUIRED'] as const

export const accountSchema = defineResponseObject({
  login: loginSchema,
  /** The GitHub profile name. */
  name: z.string().nullable(),
  avatarUrl: urlSchema.nullable(),
  /** The Author page, `/@login`. */
  pageUrl: urlSchema,
  /** The address skilld emails: the one you saved, else your GitHub address. */
  email: z.string().nullable(),
  /** True when the digest of your watched Repositories goes to `email`. */
  digest: z.boolean(),
  /** True when the weekly goes to `email`. A GitHub address alone is not consent: save an address, or turn on `digest`. */
  weekly: z.boolean(),
  /** True when anyone can read your liked Skills at `/@login/liked`. */
  likesPublic: z.boolean(),
  /** True when skilld may scan your public Repositories for Skills. */
  repositoryIndexing: z.boolean(),
  /** When your last GitHub star import finished. */
  starsImportedAt: isoDateTimeSchema.nullable(),
})

const exampleAccount = {
  login: 'harlan-zw',
  name: 'Harlan Wilton',
  avatarUrl: 'https://avatars.githubusercontent.com/u/5326365?v=4',
  pageUrl: 'https://skilld.dev/@harlan-zw',
  email: 'harlan@example.com',
  digest: true,
  weekly: true,
  likesPublic: true,
  repositoryIndexing: true,
  starsImportedAt: '2026-09-21T08:14:02.000Z',
} as const

const accountUpdateBody = z.strictObject({
  email: z.email().max(254).transform(address => address.toLowerCase()).optional(),
  digest: z.boolean().optional(),
  weekly: z.boolean().optional(),
  likesPublic: z.boolean().optional(),
  repositoryIndexing: z.boolean().optional(),
}).refine(body => Object.values(body).some(value => value !== undefined), {
  message: 'Send at least one setting to change',
})

export const repositoryScanSchema = defineResponseObject({
  /**
   * `complete`: the scan read every Repository. `partial`: it read only some,
   * so run it again later. A `github-*` outcome means GitHub refused or failed
   * the search.
   */
  outcome: z.enum(['complete', 'partial', 'github-auth-failure', 'github-rate-limited', 'github-failure']),
  /** Repositories that hold a SKILL.md. */
  repositoriesFound: countSchema,
  /** Repositories the registry indexed during this scan. */
  repositoriesIndexed: countSchema,
  repositoriesFailed: countSchema,
})

export const likedSkillSchema = defineResponseObject({
  ...skillSummaryShape,
  likedAt: isoDateTimeSchema,
})

export const watchSchema = defineResponseObject({
  owner: ownerSchema,
  repository: repositorySchema,
  /** The Repository page on skilld.dev. */
  pageUrl: urlSchema,
  /**
   * Why the watch exists. `direct`: you watched the Repository. `like`: a like
   * added it, and removing your last like in the Repository removes it.
   * `star-import`: you watched it from your imported stars. `collection`: you
   * watched a collection that names it.
   */
  reason: z.enum(['direct', 'like', 'star-import', 'collection']),
  watchedAt: isoDateTimeSchema,
})

export const starredRepositorySchema = defineResponseObject({
  owner: ownerSchema,
  repository: repositorySchema,
  /** The Repository page on skilld.dev. */
  pageUrl: urlSchema,
  starredAt: isoDateTimeSchema,
  /** True when you watch the Repository. */
  watching: z.boolean(),
  /** Skills the registry holds for the Repository. */
  skillCount: countSchema,
})

export const starsImportSchema = defineResponseObject({
  page: z.number().int().min(1).max(10),
  /** The page to import next, or `null` when the import is finished. */
  nextPage: z.number().int().min(2).max(10).nullable(),
  /** Starred Repositories imported so far. */
  imported: countSchema,
  /** Imported Repositories that hold Skills. */
  withSkills: countSchema,
  /** Set when the last page is imported. */
  importedAt: isoDateTimeSchema.nullable(),
})

const changedSkillShape = {
  ...skillSummaryShape,
  /** The latest change in the window. */
  changedAt: isoDateTimeSchema,
  /** Changes in the window. */
  changeCount: countSchema,
  /** Commit messages from the author's Repository, newest first. The author wrote them, not skilld. */
  commitMessages: z.array(z.string()),
  /** The latest commit on GitHub. */
  changeUrl: urlSchema,
}

const changesWindowShape = {
  since: isoDateTimeSchema,
  /** Send this as `since` next time to read only newer changes. */
  until: isoDateTimeSchema,
}

export const accountChangesSchema = {
  producer: z.strictObject({ ...changesWindowShape, items: z.array(z.strictObject(changedSkillShape)) }),
  client: z.looseObject({ ...changesWindowShape, items: z.array(z.looseObject(changedSkillShape)) }),
}

export const tokenSchema = defineResponseObject({
  id: z.number().int().positive(),
  label: z.string().nullable(),
  /** `oauth`: a `skilld auth login` sign-in. `pat`: a token you created. `oidc`: a short CI token from GitHub Actions. */
  kind: z.enum(['oauth', 'pat', 'oidc']),
  createdAt: isoDateTimeSchema,
  lastUsedAt: isoDateTimeSchema,
  /** When the token stops working. `null` means it has no set end. */
  expiresAt: isoDateTimeSchema.nullable(),
  /** True for the token that sent this request. */
  current: z.boolean(),
})

export const issuedTokenSchema = defineResponseObject({
  id: z.number().int().positive(),
  label: z.string(),
  expiresAt: isoDateTimeSchema.nullable(),
  /** The secret. This answer is the only place it appears. */
  token: z.string().min(1),
})

const tokenParams = z.strictObject({
  id: z.coerce.number().int().positive(),
})

const exampleWatch = {
  owner: 'vercel-labs',
  repository: 'agent-skills',
  pageUrl: 'https://skilld.dev/gh/vercel-labs/agent-skills',
  reason: 'like',
  watchedAt: '2026-09-12T10:31:44.000Z',
} as const

export const accountV1 = defineRegistry({
  namespace: 'account',
  description: 'Read and change the signed-in account, and manage the Skills of your own Repositories.',
  operations: {
    get: defineOperation({
      id: 'account.get',
      method: 'GET',
      path: '/api/v1/account',
      access: 'account',
      semantics: { kind: 'query' },
      cache: { _tag: 'private' },
      request: { params: null, query: null, body: null },
      response: { status: 200, body: accountSchema },
      errors: accountErrors,
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'Get your account',
        description: 'Your login, your Author page, the address skilld emails, and your email and privacy settings.',
        tag: 'Account',
        examples: [{ request: {}, response: exampleAccount }],
      },
    }),
    update: defineOperation({
      id: 'account.update',
      method: 'PATCH',
      path: '/api/v1/account',
      access: 'account',
      semantics: { kind: 'mutation', retry: 'never' },
      cache: { _tag: 'private' },
      request: { params: null, query: null, body: accountUpdateBody },
      response: { status: 200, body: accountSchema },
      errors: accountInputErrors,
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'Change your settings',
        description: 'Send only the settings to change. A setting you leave out keeps its value. The new address takes effect at once, without a confirmation email. To turn on `digest` or `weekly`, the account needs an address: send `email`, or keep the saved address or your GitHub address. You cannot remove an address; turn both emails off instead. The answer is the account after the change.',
        tag: 'Account',
        examples: [
          {
            request: { body: { email: 'harlan@example.com', digest: true } },
            response: exampleAccount,
          },
          {
            request: { body: { likesPublic: false } },
            response: { ...exampleAccount, likesPublic: false },
          },
        ],
      },
    }),
    scanRepositories: defineOperation({
      id: 'account.repositories.scan',
      method: 'POST',
      path: '/api/v1/account/repositories/scan',
      access: 'account',
      semantics: { kind: 'mutation', retry: 'never' },
      cache: { _tag: 'private' },
      request: { params: null, query: null, body: null },
      response: { status: 200, body: repositoryScanSchema },
      errors: ['AUTH_REQUIRED', 'FORBIDDEN'],
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'Scan your Repositories for Skills',
        description: 'Searches the public Repositories of your GitHub account and its organizations for Skills, and indexes them. Sign-in runs the same scan. If `repositoryIndexing` is off, the answer is FORBIDDEN. If skilld holds no GitHub access for the account, the answer is AUTH_REQUIRED: sign in on skilld.dev again.',
        tag: 'Account',
        examples: [{
          request: {},
          response: { outcome: 'complete', repositoriesFound: 3, repositoriesIndexed: 2, repositoriesFailed: 0 },
        }],
      },
    }),
    unpublishRepository: defineOperation({
      id: 'account.repositories.unpublish',
      method: 'DELETE',
      path: '/api/v1/account/repositories/{owner}/{repository}',
      access: 'account',
      semantics: { kind: 'mutation', retry: 'idempotent' },
      cache: { _tag: 'private' },
      request: { params: repositoryParams, query: null, body: null },
      response: { status: 204, body: null },
      errors: ['INVALID_REQUEST', 'AUTH_REQUIRED', 'FORBIDDEN'],
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'Unpublish one of your Repositories',
        description: 'Removes every Skill of the Repository from the registry. The Owner must be your own login; any other Owner is FORBIDDEN.',
        tag: 'Account',
        examples: [{ request: { params: { owner: 'harlan-zw', repository: 'skills' } }, response: null }],
      },
    }),
  },
})

export const likesV1 = defineRegistry({
  namespace: 'likes',
  description: 'Like Skills. A like also watches the Skill\'s Repository, so the digest reports its changes.',
  operations: {
    list: defineOperation({
      id: 'likes.list',
      method: 'GET',
      path: '/api/v1/account/likes',
      access: 'account',
      semantics: { kind: 'query' },
      cache: { _tag: 'private' },
      request: { params: null, query: z.object(pageQueryShape({ defaultLimit: 20, maxLimit: 50 })), body: null },
      response: { status: 200, body: defineListResponse(likedSkillSchema) },
      errors: accountInputErrors,
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'List your liked Skills',
        description: 'Your liked Skills, newest like first.',
        tag: 'Likes',
        examples: [{
          request: { query: { limit: 1 } },
          response: { items: [{ ...exampleSkillSummary, likedAt: '2026-09-12T10:31:44.000Z' }], total: 14 },
        }],
      },
    }),
    create: defineOperation({
      id: 'likes.create',
      method: 'PUT',
      path: '/api/v1/account/likes/{owner}/{repository}/{name}',
      access: 'account',
      semantics: { kind: 'mutation', retry: 'idempotent' },
      cache: { _tag: 'private' },
      request: { params: skillParams, query: null, body: null },
      response: { status: 204, body: null },
      errors: ['INVALID_REQUEST', 'AUTH_REQUIRED', 'NOT_FOUND', 'RATE_LIMITED'],
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'Like a Skill',
        description: 'Likes the Skill and watches its Repository, so the digest reports changes to the Skill. If you already watch the Repository, that watch stays as it is. An account can add 200 likes a day.',
        tag: 'Likes',
        examples: [{ request: { params: { owner: 'vercel-labs', repository: 'agent-skills', name: 'web-design-guidelines' } }, response: null }],
      },
    }),
    delete: defineOperation({
      id: 'likes.delete',
      method: 'DELETE',
      path: '/api/v1/account/likes/{owner}/{repository}/{name}',
      access: 'account',
      semantics: { kind: 'mutation', retry: 'idempotent' },
      cache: { _tag: 'private' },
      request: { params: skillParams, query: null, body: null },
      response: { status: 204, body: null },
      errors: accountInputErrors,
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'Remove a like',
        description: 'Removes your like. If it was your last like in the Repository, the watch that a like added goes too. A watch you added another way stays.',
        tag: 'Likes',
        examples: [{ request: { params: { owner: 'vercel-labs', repository: 'agent-skills', name: 'web-design-guidelines' } }, response: null }],
      },
    }),
  },
})

export const watchesV1 = defineRegistry({
  namespace: 'watches',
  description: 'Watch Repositories, so the digest reports changes to their Skills.',
  operations: {
    list: defineOperation({
      id: 'watches.list',
      method: 'GET',
      path: '/api/v1/account/watches',
      access: 'account',
      semantics: { kind: 'query' },
      cache: { _tag: 'private' },
      request: { params: null, query: z.object(pageQueryShape({ defaultLimit: 50, maxLimit: 100 })), body: null },
      response: { status: 200, body: defineListResponse(watchSchema) },
      errors: accountInputErrors,
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'List your watched Repositories',
        description: 'The Repositories your digest reports on, newest watch first, with the reason each watch exists.',
        tag: 'Watches',
        examples: [{ request: { query: { limit: 1 } }, response: { items: [exampleWatch], total: 6 } }],
      },
    }),
    create: defineOperation({
      id: 'watches.create',
      method: 'PUT',
      path: '/api/v1/account/watches/{owner}/{repository}',
      access: 'account',
      semantics: { kind: 'mutation', retry: 'idempotent' },
      cache: { _tag: 'private' },
      request: { params: repositoryParams, query: null, body: null },
      response: { status: 204, body: null },
      errors: ['INVALID_REQUEST', 'AUTH_REQUIRED', 'NOT_FOUND'],
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'Watch a Repository',
        description: 'Watches a Repository the registry holds. A watch that a like added becomes a `direct` watch, so removing the like no longer removes it.',
        tag: 'Watches',
        examples: [{ request: { params: { owner: 'vercel-labs', repository: 'agent-skills' } }, response: null }],
      },
    }),
    delete: defineOperation({
      id: 'watches.delete',
      method: 'DELETE',
      path: '/api/v1/account/watches/{owner}/{repository}',
      access: 'account',
      semantics: { kind: 'mutation', retry: 'idempotent' },
      cache: { _tag: 'private' },
      request: { params: repositoryParams, query: null, body: null },
      response: { status: 204, body: null },
      errors: accountInputErrors,
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'Stop watching a Repository',
        description: 'Removes the watch, whatever its reason. Your likes stay.',
        tag: 'Watches',
        examples: [{ request: { params: { owner: 'vercel-labs', repository: 'agent-skills' } }, response: null }],
      },
    }),
  },
})

export const starsV1 = defineRegistry({
  namespace: 'stars',
  description: 'Import your GitHub stars and find the starred Repositories that hold Skills.',
  operations: {
    list: defineOperation({
      id: 'stars.list',
      method: 'GET',
      path: '/api/v1/account/stars',
      access: 'account',
      semantics: { kind: 'query' },
      cache: { _tag: 'private' },
      request: { params: null, query: z.object(pageQueryShape({ defaultLimit: 50, maxLimit: 100 })), body: null },
      response: { status: 200, body: defineListResponse(starredRepositorySchema) },
      errors: accountInputErrors,
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'List your starred Repositories that hold Skills',
        description: 'Reads your last import, newest star first. Run `stars.import` to read GitHub again.',
        tag: 'Stars',
        examples: [{
          request: { query: { limit: 1 } },
          response: {
            items: [{
              owner: 'vercel-labs',
              repository: 'agent-skills',
              pageUrl: 'https://skilld.dev/gh/vercel-labs/agent-skills',
              starredAt: '2026-08-30T21:05:10.000Z',
              watching: false,
              skillCount: 6,
            }],
            total: 3,
          },
        }],
      },
    }),
    import: defineOperation({
      id: 'stars.import',
      method: 'POST',
      path: '/api/v1/account/stars/import',
      access: 'account',
      semantics: { kind: 'mutation', retry: 'never' },
      cache: { _tag: 'private' },
      request: {
        params: null,
        query: null,
        body: z.strictObject({ page: z.number().int().min(1).max(10).default(1) }),
      },
      response: { status: 200, body: starsImportSchema },
      errors: accountInputErrors,
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'Import your GitHub stars',
        description: 'Imports one page of 100 GitHub stars and keeps the Repositories whose name mentions a Skill. Page 1 replaces the last import. Send each `nextPage` until it is `null`; the import stops after page 10. If skilld holds no GitHub access for the account, the answer is AUTH_REQUIRED: sign in on skilld.dev again.',
        tag: 'Stars',
        examples: [
          {
            request: { body: {} },
            response: { page: 1, nextPage: 2, imported: 4, withSkills: 2, importedAt: null },
          },
          {
            request: { body: { page: 2 } },
            response: { page: 2, nextPage: null, imported: 5, withSkills: 3, importedAt: '2026-10-01T07:45:00.000Z' },
          },
        ],
      },
    }),
  },
})

export const changesV1 = defineRegistry({
  namespace: 'changes',
  description: 'Read what changed in your watched Repositories: the content of your digest.',
  operations: {
    list: defineOperation({
      id: 'changes.list',
      method: 'GET',
      path: '/api/v1/account/changes',
      access: 'account',
      semantics: { kind: 'query' },
      cache: { _tag: 'private' },
      request: {
        params: null,
        query: z.object({ since: isoDateTimeSchema.optional() }),
        body: null,
      },
      response: { status: 200, body: accountChangesSchema },
      errors: accountInputErrors,
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'List changes to your watched Skills',
        description: 'The Skills that changed in your watched Repositories between `since` and now, the same selection as your digest. A watch that a like added reports only the Skills you liked. Without `since`, the window is the last 30 days. The answer covers at most the 30 Repositories with the most changes.',
        tag: 'Changes',
        examples: [{
          request: { query: { since: '2026-09-01T00:00:00.000Z' } },
          response: {
            since: '2026-09-01T00:00:00.000Z',
            until: '2026-10-01T09:00:00.000Z',
            items: [{
              ...exampleSkillSummary,
              changedAt: '2026-09-28T14:02:11.000Z',
              changeCount: 2,
              commitMessages: ['docs: add focus ring rules', 'docs: clarify form labels'],
              changeUrl: 'https://github.com/vercel-labs/agent-skills/commit/4f1c2a9e0b7d3c5a8e6f1b2d4c7a9e0f3b5d8c1a',
            }],
          },
        }],
      },
    }),
  },
})

export const tokensV1 = defineRegistry({
  namespace: 'tokens',
  description: 'Create, list, and revoke the skilld tokens that act for your account.',
  operations: {
    list: defineOperation({
      id: 'tokens.list',
      method: 'GET',
      path: '/api/v1/account/tokens',
      access: 'account',
      semantics: { kind: 'query' },
      cache: { _tag: 'private' },
      request: { params: null, query: z.object(pageQueryShape({ defaultLimit: 50, maxLimit: 100 })), body: null },
      response: { status: 200, body: defineListResponse(tokenSchema) },
      errors: accountInputErrors,
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'List your tokens',
        description: 'Your tokens that still work, most recently used first. A revoked or expired token is not listed. The answer never holds a secret.',
        tag: 'Tokens',
        examples: [{
          request: {},
          response: {
            items: [{
              id: 412,
              label: 'CI deploy',
              kind: 'pat',
              createdAt: '2026-09-02T11:20:00.000Z',
              lastUsedAt: '2026-10-01T06:58:31.000Z',
              expiresAt: '2026-12-01T11:20:00.000Z',
              current: true,
            }],
            total: 1,
          },
        }],
      },
    }),
    create: defineOperation({
      id: 'tokens.create',
      method: 'POST',
      path: '/api/v1/account/tokens',
      access: 'account',
      semantics: { kind: 'mutation', retry: 'never' },
      cache: { _tag: 'private' },
      request: {
        params: null,
        query: null,
        body: z.strictObject({
          label: z.string().trim().min(1).max(80),
          /** Days until the token stops working. Leave it out for a token with no set end. */
          ttlDays: z.number().int().min(1).max(3650).optional(),
        }),
      },
      response: { status: 201, body: issuedTokenSchema },
      errors: [...accountInputErrors, 'FORBIDDEN'],
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'Create a token',
        description: 'Creates a skilld token that acts for your account. Send it as a Bearer credential. Store `token` now: no later answer shows it. A one-hour GitHub Actions token cannot create a token, so that request is FORBIDDEN.',
        tag: 'Tokens',
        examples: [{
          request: { body: { label: 'CI deploy', ttlDays: 90 } },
          response: {
            id: 412,
            label: 'CI deploy',
            expiresAt: '2026-12-31T09:00:00.000Z',
            token: 'eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOjEsInRpZCI6NDEyfQ.c2lnbmF0dXJl',
          },
        }],
      },
    }),
    revoke: defineOperation({
      id: 'tokens.revoke',
      method: 'DELETE',
      path: '/api/v1/account/tokens/{id}',
      access: 'account',
      semantics: { kind: 'mutation', retry: 'idempotent' },
      cache: { _tag: 'private' },
      request: { params: tokenParams, query: null, body: null },
      response: { status: 204, body: null },
      errors: ['INVALID_REQUEST', 'AUTH_REQUIRED', 'NOT_FOUND'],
      lifecycle: { introduced: '1.0.0' },
      docs: {
        summary: 'Revoke a token',
        description: 'Revokes one of your tokens. It stops working at once, even if it sent this request. A token of another account is NOT_FOUND.',
        tag: 'Tokens',
        examples: [{ request: { params: { id: 412 } }, response: null }],
      },
    }),
  },
})
