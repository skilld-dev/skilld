import type { OperationInput } from '../src/contract/core'
import { describe, expect, it, vi } from 'vitest'
import { z } from 'zod'
import { createProtocolClient, createSkilldClient } from '../src/client'
import { defineOperation, defineProtocol, defineRegistry, defineResponseObject } from '../src/contract/core'
import { listOperations, skilldV1Protocol } from '../src/contract/index'

const thing = defineResponseObject({ id: z.string() })
const protocol = defineProtocol({
  version: '1',
  registries: {
    things: defineRegistry({
      namespace: 'things',
      description: 'Things.',
      operations: {
        get: defineOperation({
          id: 'things.get',
          method: 'GET',
          path: '/api/v1/things/{id}',
          access: 'public',
          semantics: { kind: 'query' },
          cache: { _tag: 'public', maxAgeSeconds: 60, staleWhileRevalidateSeconds: 300 },
          request: { params: z.strictObject({ id: z.string().min(2) }), query: z.object({ limit: z.coerce.number().int().max(5).optional() }), body: null },
          response: { status: 200, body: thing },
          errors: ['INVALID_REQUEST', 'NOT_FOUND'],
          lifecycle: { introduced: '1.0.0' },
          docs: { summary: 'Get', description: 'Reads one.', tag: 'Things', examples: [{ request: { params: { id: 'ab' } }, response: { id: 'ab' } }] },
        }),
        create: defineOperation({
          id: 'things.create',
          method: 'POST',
          path: '/api/v1/things',
          access: 'account',
          semantics: { kind: 'mutation', retry: 'never' },
          cache: { _tag: 'private' },
          request: { params: null, query: null, body: z.strictObject({ id: z.string() }) },
          response: { status: 201, body: thing },
          errors: ['INVALID_REQUEST', 'AUTH_REQUIRED', 'CONFLICT'],
          lifecycle: { introduced: '1.0.0' },
          docs: { summary: 'Create', description: 'Makes one.', tag: 'Things', examples: [{ request: { body: { id: 'ab' } }, response: { id: 'ab' } }] },
        }),
        remove: defineOperation({
          id: 'things.remove',
          method: 'DELETE',
          path: '/api/v1/things/{id}',
          access: 'account',
          semantics: { kind: 'mutation', retry: 'idempotent' },
          cache: { _tag: 'private' },
          request: { params: z.strictObject({ id: z.string() }), query: null, body: null },
          response: { status: 204, body: null },
          errors: ['INVALID_REQUEST', 'AUTH_REQUIRED'],
          lifecycle: { introduced: '1.0.0' },
          docs: { summary: 'Remove', description: 'Removes one.', tag: 'Things', examples: [{ request: { params: { id: 'ab' } }, response: null }] },
        }),
      },
    }),
  },
})

function json(body: unknown, status = 200, headers: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json', ...headers } })
}

function problem(code: string, status: number, headers: Record<string, string> = {}): Response {
  return json({ type: `https://skilld.dev/problems/${code.toLowerCase()}`, title: 'T', status, code, instance: '/x' }, status, { 'content-type': 'application/problem+json', ...headers })
}

const noWait = { sleep: async () => {} }

describe('createProtocolClient', () => {
  it('sends the token and the query, and returns the parsed answer', async () => {
    const fetch = vi.fn(async () => json({ id: 'ab', added: true }, 200, { 'x-request-id': 'req_1' }))
    const client = createProtocolClient(protocol, { baseUrl: 'https://example.test/', token: 'sk_1', fetch })
    const result = await client.things.get({ params: { id: 'ab' }, query: { limit: 2 } })
    expect(result).toEqual({ _tag: 'Ok', value: { id: 'ab', added: true }, requestId: 'req_1' })
    const [url, init] = fetch.mock.calls[0] as unknown as [string, RequestInit]
    expect(url).toBe('https://example.test/api/v1/things/ab?limit=2')
    expect(new Headers(init.headers).get('authorization')).toBe('Bearer sk_1')
  })

  it('refuses input the schema rejects without sending anything', async () => {
    const fetch = vi.fn()
    const client = createProtocolClient(protocol, { fetch })
    const result = await client.things.get({ params: { id: 'a' } })
    expect(result).toMatchObject({ _tag: 'Err', error: { _tag: 'RequestFailure', location: 'params' } })
    expect(fetch).not.toHaveBeenCalled()
  })

  it('returns a declared problem as an ApiFailure', async () => {
    const client = createProtocolClient(protocol, { fetch: async () => problem('CONFLICT', 409) })
    const result = await client.things.create({ body: { id: 'ab' } })
    expect(result).toMatchObject({ _tag: 'Err', error: { _tag: 'ApiFailure', code: 'CONFLICT', status: 409, retryable: false } })
  })

  it('treats an undeclared problem code as a contract failure', async () => {
    const client = createProtocolClient(protocol, { fetch: async () => problem('FORBIDDEN', 403) })
    const result = await client.things.get({ params: { id: 'ab' } })
    expect(result).toMatchObject({ _tag: 'Err', error: { _tag: 'ContractFailure', status: 403 } })
  })

  it('retries a query after a 503 and honours Retry-After', async () => {
    const sleep = vi.fn(async () => {})
    const fetch = vi.fn()
      .mockResolvedValueOnce(problem('SERVICE_UNAVAILABLE', 503, { 'retry-after': '2' }))
      .mockResolvedValueOnce(json({ id: 'ab' }))
    const client = createProtocolClient(protocol, { fetch, retry: { sleep } })
    const result = await client.things.get({ params: { id: 'ab' } })
    expect(result._tag).toBe('Ok')
    expect(fetch).toHaveBeenCalledTimes(2)
    expect(sleep).toHaveBeenCalledWith(2000, undefined)
  })

  it('never repeats a POST', async () => {
    const fetch = vi.fn(async () => problem('SERVICE_UNAVAILABLE', 503))
    const client = createProtocolClient(protocol, { fetch, retry: noWait })
    const result = await client.things.create({ body: { id: 'ab' } })
    expect(fetch).toHaveBeenCalledTimes(1)
    expect(result).toMatchObject({ _tag: 'Err', error: { _tag: 'ApiFailure', retryable: false } })
  })

  it('retries an idempotent DELETE after a network failure', async () => {
    const fetch = vi.fn()
      .mockRejectedValueOnce(new TypeError('fetch failed'))
      .mockResolvedValueOnce(new Response(null, { status: 204 }))
    const client = createProtocolClient(protocol, { fetch, retry: noWait })
    expect(await client.things.remove({ params: { id: 'ab' } })).toEqual({ _tag: 'Ok', value: null, requestId: undefined })
  })

  it('reports an answer that breaks the schema with the request ID', async () => {
    const client = createProtocolClient(protocol, { fetch: async () => json({ id: 7 }, 200, { 'x-request-id': 'req_9' }) })
    const result = await client.things.get({ params: { id: 'ab' } })
    expect(result).toMatchObject({ _tag: 'Err', error: { _tag: 'ContractFailure', requestId: 'req_9' } })
  })
})

describe('createSkilldClient', () => {
  it.each(listOperations(skilldV1Protocol).map(({ operation }) => [operation.id, operation] as const))(
    'returns a typed rate limit for %s without retrying when disabled',
    async (_id, operation) => {
      const fetch = vi.fn(async () => problem('RATE_LIMITED', 429, { 'x-request-id': 'req_rate_limit' }))
      const skilld = createSkilldClient({ fetch, retry: { maxAttempts: 1 } })
      // listOperations erases each operation's input type. Its examples were parsed during definition.
      const input = operation.docs.examples[0]!.request as OperationInput<typeof operation>
      const result = await skilld.execute(operation, input)
      expect(result).toMatchObject({
        _tag: 'Err',
        error: { _tag: 'ApiFailure', code: 'RATE_LIMITED', status: 429, requestId: 'req_rate_limit' },
      })
      expect(fetch).toHaveBeenCalledOnce()
    },
  )

  it('honours Retry-After when public search is rate limited', async () => {
    const sleep = vi.fn(async () => {})
    const fetch = vi.fn()
      .mockResolvedValueOnce(problem('RATE_LIMITED', 429, { 'retry-after': '2' }))
      .mockResolvedValueOnce(json({ items: [], total: 0 }))
    const skilld = createSkilldClient({ fetch, retry: { sleep } })
    expect(await skilld.skills.search({ query: { q: 'tailwind' } })).toMatchObject({
      _tag: 'Ok', value: { items: [], total: 0 },
    })
    expect(fetch).toHaveBeenCalledTimes(2)
    expect(sleep).toHaveBeenCalledWith(2000, undefined)
  })

  it('reads skilld.dev by default and exposes every registry', async () => {
    const fetch = vi.fn(async (_url: string, _init: RequestInit) => json({ items: [], total: 0 }))
    const skilld = createSkilldClient({ fetch })
    const result = await skilld.skills.search({ query: { q: 'tailwind' } })
    expect(result).toMatchObject({ _tag: 'Ok', value: { items: [], total: 0 } })
    expect(fetch.mock.calls[0]![0]).toBe('https://skilld.dev/api/v1/skills?limit=20&q=tailwind')
  })
})
