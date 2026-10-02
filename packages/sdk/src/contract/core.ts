import type { ZodRawShape, ZodTypeAny } from 'zod'
import { z } from 'zod'

/**
 * The skilld public API contract, version 1.
 *
 * One descriptor per operation drives four things: the OpenAPI document, the
 * server's request and response checks, the `skilld-sdk` client, and the
 * route parity test. Every descriptor checks itself when it is defined, so a
 * contract that cannot hold never reaches a route or a client.
 *
 * The wire format is the one `/api/v1` already shipped to the skilld CLI:
 * bare JSON bodies, and `application/problem+json` errors. ADR-0006 records
 * why there is no `{ data, meta }` envelope.
 */
export const SKILLD_V1_VERSION = '1' as const
export const SKILLD_V1_PATH_PREFIX = '/api/v1' as const
export const SKILLD_V1_ORIGIN = 'https://skilld.dev' as const
export const SKILLD_V1_RESPONSE_HEADERS = {
  requestId: 'X-Request-Id',
  operation: 'Skilld-Operation',
  retryAfter: 'Retry-After',
  deprecation: 'Deprecation',
} as const

export const SKILLD_V1_METHODS = ['DELETE', 'GET', 'PATCH', 'POST', 'PUT'] as const
export const SKILLD_V1_ERROR_CODES = [
  'INVALID_REQUEST',
  'AUTH_REQUIRED',
  'FORBIDDEN',
  'NOT_FOUND',
  'CONFLICT',
  'RATE_LIMITED',
  'INTERNAL_ERROR',
  'SERVICE_UNAVAILABLE',
] as const

export const SKILLD_V1_ERROR_STATUS = {
  INVALID_REQUEST: 400,
  AUTH_REQUIRED: 401,
  FORBIDDEN: 403,
  NOT_FOUND: 404,
  CONFLICT: 409,
  RATE_LIMITED: 429,
  INTERNAL_ERROR: 500,
  SERVICE_UNAVAILABLE: 503,
} as const satisfies Record<typeof SKILLD_V1_ERROR_CODES[number], number>

export const SKILLD_V1_ERROR_TITLES = {
  INVALID_REQUEST: 'Invalid request',
  AUTH_REQUIRED: 'Authentication required',
  FORBIDDEN: 'Forbidden',
  NOT_FOUND: 'Not found',
  CONFLICT: 'Conflict',
  RATE_LIMITED: 'Request rate limited',
  INTERNAL_ERROR: 'Internal error',
  SERVICE_UNAVAILABLE: 'Service unavailable',
} as const satisfies Record<typeof SKILLD_V1_ERROR_CODES[number], string>

/** Every operation can answer these, so no descriptor lists them. */
export const SKILLD_V1_IMPLICIT_ERRORS = ['INTERNAL_ERROR', 'SERVICE_UNAVAILABLE'] as const

/** A retry can succeed for these codes, and for no others. */
export const SKILLD_V1_RETRYABLE_ERRORS: ReadonlySet<SkilldV1ErrorCode> = new Set(['RATE_LIMITED', 'SERVICE_UNAVAILABLE'])

export type SkilldV1Method = typeof SKILLD_V1_METHODS[number]
export type SkilldV1ErrorCode = typeof SKILLD_V1_ERROR_CODES[number]

export interface CompatibleSchema<
  TProducer extends ZodTypeAny = ZodTypeAny,
  TClient extends ZodTypeAny = ZodTypeAny,
> {
  /** The server parses its answer with this. It rejects any field the contract does not name. */
  producer: TProducer
  /** The SDK parses answers with this. It keeps unknown fields, so a newer server never breaks an older client. */
  client: TClient
}

export interface SkilldV1RequestSchemas {
  params: ZodTypeAny | null
  query: ZodTypeAny | null
  body: ZodTypeAny | null
}

/**
 * `public` operations need no credential and never vary by caller, so a shared
 * cache may hold them. `account` operations need a skilld token or a skilld.dev
 * sign-in, and are never cached.
 */
export type SkilldV1Access = 'public' | 'account'

export type SkilldV1Semantics
  = | { kind: 'query' }
    | { kind: 'mutation', retry: 'idempotent' | 'never' }

export type SkilldV1CachePolicy
  = | { _tag: 'public', maxAgeSeconds: number, staleWhileRevalidateSeconds: number }
    | { _tag: 'private' }

export interface SkilldV1OperationExample {
  request: {
    params?: unknown
    query?: unknown
    body?: unknown
  }
  /** `null` for an operation that answers 204 No Content. */
  response: unknown
}

export type SemverString = `${number}.${number}.${number}`

export interface SkilldV1OperationDefinition {
  /** Stable dotted identifier, `resource.verb` or `resource.sub.verb`. Never renamed. */
  id: string
  method: SkilldV1Method
  path: `/api/v1/${string}`
  access: SkilldV1Access
  semantics: SkilldV1Semantics
  cache: SkilldV1CachePolicy
  request: SkilldV1RequestSchemas
  response: {
    status: 200 | 201 | 204
    body: CompatibleSchema | null
  }
  /** Expected failures beyond {@link SKILLD_V1_IMPLICIT_ERRORS}. */
  errors: readonly SkilldV1ErrorCode[]
  lifecycle: {
    introduced: SemverString
    deprecated?: { version: SemverString, at: string, replacement?: string }
  }
  docs: {
    summary: string
    description: string
    tag: string
    examples: readonly SkilldV1OperationExample[]
  }
}

export interface SkilldV1Registry<
  TOperations extends Readonly<Record<string, SkilldV1OperationDefinition>> = Readonly<Record<string, SkilldV1OperationDefinition>>,
> {
  /** The SDK namespace: `client.<namespace>.<operation key>()`. */
  namespace: string
  /** OpenAPI tag description. */
  description: string
  operations: TOperations
}

export interface SkilldV1ProtocolLike {
  version: typeof SKILLD_V1_VERSION
  registries: Readonly<Record<string, SkilldV1Registry>>
}

function pathParameterNames(path: string): string[] {
  return Array.from(path.matchAll(/\{([^{}]+)\}/g), match => match[1]!)
}

const SEGMENT = String.raw`(?:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?|\{[a-z][A-Za-z0-9]*\})`
const PATH_TEMPLATE = new RegExp(String.raw`^/api/v1(?:/${SEGMENT})+$`)

function objectSchemaKeys(schema: ZodTypeAny): string[] | null {
  return schema instanceof z.ZodObject ? Object.keys(schema.shape) : null
}

function assertRequestLocations(operation: SkilldV1OperationDefinition): void {
  for (const location of ['params', 'query', 'body'] as const) {
    if (!(location in operation.request) || operation.request[location] === undefined)
      throw new TypeError(`${operation.id}: request.${location} must be a schema or null`)
  }
  for (const location of ['params', 'query'] as const) {
    const schema = operation.request[location]
    if (schema !== null && objectSchemaKeys(schema) === null)
      throw new TypeError(`${operation.id}: request.${location} must be an object schema or null`)
  }
  for (const key of operation.request.query ? objectSchemaKeys(operation.request.query) ?? [] : []) {
    if (!/^[a-z][A-Za-z0-9]*$/.test(key))
      throw new TypeError(`${operation.id}: query key ${key} must be camelCase`)
  }
  if (operation.method === 'GET' && operation.request.body !== null)
    throw new TypeError(`${operation.id}: a GET operation cannot take a body`)
}

function assertPathContract(operation: SkilldV1OperationDefinition): void {
  if (!PATH_TEMPLATE.test(operation.path))
    throw new TypeError(`${operation.id}: path must be a safe literal /api/v1 template`)
  const names = pathParameterNames(operation.path)
  if (new Set(names).size !== names.length)
    throw new TypeError(`${operation.id}: path parameters must be unique`)
  if (names.length === 0) {
    if (operation.request.params !== null)
      throw new TypeError(`${operation.id}: params must be null when the path has no parameters`)
    return
  }
  const keys = operation.request.params ? objectSchemaKeys(operation.request.params) : null
  if (!keys)
    throw new TypeError(`${operation.id}: path parameters require an object params schema`)
  if (names.join('\0') !== keys.join('\0'))
    throw new TypeError(`${operation.id}: params schema keys must match the path parameter order`)
}

function assertSemantics(operation: SkilldV1OperationDefinition): void {
  const { method, semantics, access, cache } = operation
  if (semantics.kind === 'query' && method !== 'GET')
    throw new TypeError(`${operation.id}: a query must be a GET`)
  if (semantics.kind === 'mutation' && method === 'GET')
    throw new TypeError(`${operation.id}: a mutation cannot be a GET`)
  // PUT and DELETE name the end state, so sending one twice changes nothing.
  // POST and PATCH may not, so the client never repeats them on its own.
  if (semantics.kind === 'mutation' && semantics.retry === 'idempotent' && method !== 'PUT' && method !== 'DELETE')
    throw new TypeError(`${operation.id}: only PUT and DELETE mutations may retry`)
  if (cache._tag === 'public') {
    if (semantics.kind !== 'query' || access !== 'public')
      throw new TypeError(`${operation.id}: only public queries may use a shared cache`)
    if (!Number.isSafeInteger(cache.maxAgeSeconds) || cache.maxAgeSeconds < 1)
      throw new TypeError(`${operation.id}: cache max age must be a positive number of seconds`)
    if (!Number.isSafeInteger(cache.staleWhileRevalidateSeconds) || cache.staleWhileRevalidateSeconds < 0)
      throw new TypeError(`${operation.id}: stale-while-revalidate must be a nonnegative number of seconds`)
  }
}

function assertResponse(operation: SkilldV1OperationDefinition): void {
  const { status, body } = operation.response
  if ((status === 204) !== (body === null))
    throw new TypeError(`${operation.id}: a 204 response has no body, and every other response has one`)
  if (status === 201 && operation.method !== 'POST')
    throw new TypeError(`${operation.id}: only POST creates with 201`)
}

function assertErrors(operation: SkilldV1OperationDefinition): void {
  const errors = new Set(operation.errors)
  if (errors.size !== operation.errors.length)
    throw new TypeError(`${operation.id}: errors must be unique`)
  for (const code of SKILLD_V1_IMPLICIT_ERRORS) {
    if (errors.has(code))
      throw new TypeError(`${operation.id}: ${code} is implicit, do not list it`)
  }
  const takesInput = operation.request.params || operation.request.query || operation.request.body
  if (takesInput && !errors.has('INVALID_REQUEST'))
    throw new TypeError(`${operation.id}: an operation that takes input must declare INVALID_REQUEST`)
  if (operation.access === 'account' && !errors.has('AUTH_REQUIRED'))
    throw new TypeError(`${operation.id}: an account operation must declare AUTH_REQUIRED`)
  if (operation.access === 'public' && errors.has('AUTH_REQUIRED'))
    throw new TypeError(`${operation.id}: a public operation cannot answer AUTH_REQUIRED`)
}

function assertExampleLocation(
  operation: SkilldV1OperationDefinition,
  example: SkilldV1OperationExample,
  location: keyof SkilldV1RequestSchemas,
): void {
  const schema = operation.request[location]
  const supplied = Object.hasOwn(example.request, location)
  if (schema === null) {
    if (supplied)
      throw new TypeError(`${operation.id}: example supplies request.${location} without a schema`)
    return
  }
  const value = supplied ? example.request[location] : location === 'body' ? undefined : {}
  const parsed = schema.safeParse(value)
  if (!parsed.success)
    throw new TypeError(`${operation.id}: example request.${location} does not satisfy its schema: ${parsed.error.message}`)
}

function assertExamples(operation: SkilldV1OperationDefinition): void {
  if (operation.docs.examples.length === 0)
    throw new TypeError(`${operation.id}: at least one example is required`)
  for (const example of operation.docs.examples) {
    for (const location of ['params', 'query', 'body'] as const)
      assertExampleLocation(operation, example, location)
    const body = operation.response.body
    if (body === null) {
      if (example.response !== null)
        throw new TypeError(`${operation.id}: a 204 example response must be null`)
      continue
    }
    const parsed = body.producer.safeParse(example.response)
    if (!parsed.success)
      throw new TypeError(`${operation.id}: example response does not satisfy the producer schema: ${parsed.error.message}`)
  }
}

function assertOperation(operation: SkilldV1OperationDefinition): void {
  if (!/^[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)+$/.test(operation.id))
    throw new TypeError(`${operation.id}: operation ID must be a dotted lowercase identifier`)
  assertRequestLocations(operation)
  assertPathContract(operation)
  assertSemantics(operation)
  assertResponse(operation)
  assertErrors(operation)
  if (!operation.docs.summary || !operation.docs.description || !operation.docs.tag)
    throw new TypeError(`${operation.id}: summary, description, and tag are required`)
  if (operation.lifecycle.deprecated && Number.isNaN(Date.parse(operation.lifecycle.deprecated.at)))
    throw new TypeError(`${operation.id}: deprecation must declare a date`)
  assertExamples(operation)
}

export function defineOperation<const TOperation extends SkilldV1OperationDefinition>(operation: TOperation): TOperation {
  assertOperation(operation)
  return operation
}

export function defineRegistry<
  const TOperations extends Readonly<Record<string, SkilldV1OperationDefinition>>,
>(registry: SkilldV1Registry<TOperations>): SkilldV1Registry<TOperations> {
  if (!/^[a-z][A-Za-z0-9]*$/.test(registry.namespace))
    throw new TypeError(`${registry.namespace}: registry namespace must be camelCase`)
  for (const key of Object.keys(registry.operations)) {
    if (!/^[a-z][A-Za-z0-9]*$/.test(key))
      throw new TypeError(`${registry.namespace}.${key}: operation keys must be camelCase`)
  }
  return registry
}

export function defineProtocol<
  const TRegistries extends Readonly<Record<string, SkilldV1Registry>>,
>(protocol: { version: typeof SKILLD_V1_VERSION, registries: TRegistries }): { version: typeof SKILLD_V1_VERSION, registries: TRegistries } {
  const ids = new Set<string>()
  const routes = new Map<string, string>()
  for (const [name, registry] of Object.entries(protocol.registries)) {
    if (name !== registry.namespace)
      throw new TypeError(`${name}: the registry key must equal its namespace ${registry.namespace}`)
    for (const operation of Object.values(registry.operations)) {
      if (ids.has(operation.id))
        throw new TypeError(`Duplicate operation ID ${operation.id}`)
      ids.add(operation.id)
      // `/skills/{owner}` and `/skills/{name}` would collide at runtime, so
      // compare templates with every parameter name erased.
      const route = `${operation.method} ${operation.path.replace(/\{[^{}]+\}/g, '{}')}`
      const existing = routes.get(route)
      if (existing)
        throw new TypeError(`${operation.id} and ${existing} share the route ${route}`)
      routes.set(route, operation.id)
    }
  }
  return protocol
}

export function listOperations(protocol: SkilldV1ProtocolLike): Array<{ registry: SkilldV1Registry, key: string, operation: SkilldV1OperationDefinition }> {
  return Object.values(protocol.registries).flatMap(registry =>
    Object.entries(registry.operations).map(([key, operation]) => ({ registry, key, operation })),
  )
}

/** A response object with a strict server shape and a lenient client shape. */
export function defineResponseObject<const TShape extends ZodRawShape>(
  shape: TShape,
): CompatibleSchema<z.ZodObject<TShape, z.core.$strict>, z.ZodObject<TShape, z.core.$loose>> {
  return {
    producer: z.strictObject(shape),
    client: z.looseObject(shape),
  }
}

/**
 * One list response. Every list answers `{ items, total }`, the shape the
 * shipped `skills.search` answer already has. `total` counts the whole result,
 * not this page, so a caller pages with `offset` until it has `total` items.
 */
export function defineListResponse<const TItem extends CompatibleSchema>(item: TItem) {
  return {
    producer: z.strictObject({ items: z.array(item.producer), total: z.number().int().nonnegative() }),
    client: z.looseObject({ items: z.array(item.client), total: z.number().int().nonnegative() }),
  }
}

/** `limit` and `offset` for a list query. A query string carries text, so both coerce. */
export function pageQueryShape(options: { defaultLimit: number, maxLimit: number }) {
  return {
    limit: z.coerce.number().int().min(1).max(options.maxLimit).default(options.defaultLimit),
    offset: z.coerce.number().int().min(0).max(10_000).default(0),
  }
}

export function buildOperationPath(operation: SkilldV1OperationDefinition, params?: unknown): string {
  const names = pathParameterNames(operation.path)
  if (names.length === 0)
    return operation.path
  if (operation.request.params === null)
    throw new TypeError(`${operation.id}: operation has an invalid path contract`)
  const parsed = operation.request.params.parse(params) as Record<string, unknown>
  return operation.path.replace(/\{([^{}]+)\}/g, (_match, name: string) => {
    const value = parsed[name]
    if (typeof value !== 'string' && typeof value !== 'number')
      throw new TypeError(`${operation.id}: path parameter ${name} must be a string or number`)
    const serialized = String(value)
    if (!serialized || serialized === '.' || serialized === '..')
      throw new TypeError(`${operation.id}: path parameter ${name} cannot be empty or a dot segment`)
    return encodeURIComponent(serialized)
  })
}

/**
 * RFC 9457 problem details. The skilld CLI parses exactly these six fields
 * and rejects any other, so the producer is strict and stays this shape.
 */
export const problemSchema = defineResponseObject({
  type: z.string().min(1),
  title: z.string().min(1),
  status: z.number().int().min(400).max(599),
  detail: z.string().max(1000).optional(),
  instance: z.string().optional(),
  code: z.enum(SKILLD_V1_ERROR_CODES),
})

export type SkilldV1Problem = z.output<typeof problemSchema.producer>

export function problemType(code: SkilldV1ErrorCode): string {
  return `${SKILLD_V1_ORIGIN}/problems/${code.toLowerCase().replaceAll('_', '-')}`
}

export type OperationInput<TOperation extends SkilldV1OperationDefinition>
  = (TOperation['request']['params'] extends ZodTypeAny ? { params: z.input<TOperation['request']['params']> } : { params?: never })
    & (TOperation['request']['query'] extends ZodTypeAny ? { query?: z.input<TOperation['request']['query']> } : { query?: never })
    & (TOperation['request']['body'] extends ZodTypeAny ? { body: z.input<TOperation['request']['body']> } : { body?: never })

/** What the server hands a route handler: every location already parsed. */
export interface OperationParsedInput<TOperation extends SkilldV1OperationDefinition> {
  params: TOperation['request']['params'] extends ZodTypeAny ? z.output<TOperation['request']['params']> : undefined
  query: TOperation['request']['query'] extends ZodTypeAny ? z.output<TOperation['request']['query']> : undefined
  body: TOperation['request']['body'] extends ZodTypeAny ? z.output<TOperation['request']['body']> : undefined
}

export type OperationOutput<TOperation extends SkilldV1OperationDefinition>
  = TOperation['response']['body'] extends CompatibleSchema<ZodTypeAny, infer TClient> ? z.output<TClient> : null

/** What a route handler returns: the producer's input, before the server parses it. */
export type OperationResult<TOperation extends SkilldV1OperationDefinition>
  = TOperation['response']['body'] extends CompatibleSchema<infer TProducer, ZodTypeAny> ? z.input<TProducer> : null
