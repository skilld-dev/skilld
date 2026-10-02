import { readFile } from 'node:fs/promises'
import { describe, expect, it } from 'vitest'
import { z } from 'zod'
import { defineOperation, defineProtocol, defineRegistry, defineResponseObject, listOperations } from '../src/contract/core'
import { skilldV1Protocol } from '../src/contract/index'
import { buildOpenApiDocument, serializeContractDocument } from '../src/contract/openapi'

const base = {
  method: 'GET',
  path: '/api/v1/things/{id}',
  access: 'public',
  semantics: { kind: 'query' },
  cache: { _tag: 'public', maxAgeSeconds: 60, staleWhileRevalidateSeconds: 300 },
  request: { params: z.strictObject({ id: z.string() }), query: null, body: null },
  response: { status: 200, body: defineResponseObject({ id: z.string() }) },
  errors: ['INVALID_REQUEST', 'NOT_FOUND'],
  lifecycle: { introduced: '1.0.0' },
  docs: { summary: 'Get', description: 'Reads one.', tag: 'Things', examples: [{ request: { params: { id: 'a' } }, response: { id: 'a' } }] },
} as const

describe('the committed OpenAPI document', () => {
  it('matches the contract, so the served file never drifts', async () => {
    const committed = await readFile(new URL('../generated/openapi.v1.json', import.meta.url), 'utf8')
    expect(serializeContractDocument(buildOpenApiDocument(skilldV1Protocol))).toBe(committed)
  })

  it('describes every operation once, under its own path and method', () => {
    const document = buildOpenApiDocument(skilldV1Protocol) as { paths: Record<string, Record<string, { operationId: string }>> }
    const described = Object.entries(document.paths)
      .flatMap(([path, methods]) => Object.entries(methods).map(([method, operation]) => `${method.toUpperCase()} ${path} ${operation.operationId}`))
      .sort()
    const declared = listOperations(skilldV1Protocol)
      .map(({ operation }) => `${operation.method} ${operation.path} ${operation.id}`)
      .sort()
    expect(described).toEqual(declared)
  })
})

describe('response schemas', () => {
  const pair = defineResponseObject({ id: z.string() })

  it('lets the server emit only named fields', () => {
    expect(pair.producer.safeParse({ id: 'a', extra: 1 }).success).toBe(false)
  })

  it('lets an older client read a newer answer', () => {
    expect(pair.client.parse({ id: 'a', extra: 1 })).toEqual({ id: 'a', extra: 1 })
  })
})

describe('defineOperation', () => {
  it('accepts a coherent descriptor', () => {
    expect(defineOperation({ id: 'things.get', ...base }).id).toBe('things.get')
  })

  it('rejects an example the response schema refuses', () => {
    expect(() => defineOperation({
      id: 'things.get',
      ...base,
      docs: { ...base.docs, examples: [{ request: { params: { id: 'a' } }, response: { id: 1 } }] },
    })).toThrow(/example response/)
  })

  it('rejects params that do not match the path', () => {
    expect(() => defineOperation({
      id: 'things.get',
      ...base,
      request: { ...base.request, params: z.strictObject({ slug: z.string() }) },
    })).toThrow(/path parameter order/)
  })

  it('rejects a shared cache on an account operation', () => {
    expect(() => defineOperation({ id: 'things.get', ...base, access: 'account', errors: ['INVALID_REQUEST', 'AUTH_REQUIRED'] })).toThrow(/shared cache/)
  })

  it('rejects an account operation that cannot answer AUTH_REQUIRED', () => {
    expect(() => defineOperation({ id: 'things.get', ...base, access: 'account', cache: { _tag: 'private' } })).toThrow(/AUTH_REQUIRED/)
  })

  it('rejects a retrying POST', () => {
    expect(() => defineOperation({
      id: 'things.create',
      ...base,
      method: 'POST',
      path: '/api/v1/things',
      semantics: { kind: 'mutation', retry: 'idempotent' },
      cache: { _tag: 'private' },
      request: { params: null, query: null, body: null },
      errors: ['NOT_FOUND'],
      docs: { ...base.docs, examples: [{ request: {}, response: { id: 'a' } }] },
    })).toThrow(/only PUT and DELETE/)
  })
})

describe('defineProtocol', () => {
  it('rejects two operations that share a route under different parameter names', () => {
    const first = defineOperation({ id: 'things.get', ...base })
    const second = defineOperation({
      id: 'things.other',
      ...base,
      path: '/api/v1/things/{slug}',
      request: { ...base.request, params: z.strictObject({ slug: z.string() }) },
      docs: { ...base.docs, examples: [{ request: { params: { slug: 'a' } }, response: { id: 'a' } }] },
    })
    expect(() => defineProtocol({
      version: '1',
      registries: { things: defineRegistry({ namespace: 'things', description: 'Things.', operations: { get: first, other: second } }) },
    })).toThrow(/share the route/)
  })
})
