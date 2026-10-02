import type { ZodTypeAny } from 'zod'
import type { SkilldV1ErrorCode, SkilldV1OperationDefinition, SkilldV1ProtocolLike } from './core'
import { z } from 'zod'
import {
  listOperations,
  problemSchema,
  SKILLD_V1_ERROR_STATUS,
  SKILLD_V1_IMPLICIT_ERRORS,
  SKILLD_V1_ORIGIN,
  SKILLD_V1_RESPONSE_HEADERS,
} from './core'

export type ContractDocument = Record<string, unknown>

function compareStrings(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0
}

function orderedJson(value: unknown): unknown {
  if (Array.isArray(value))
    return value.map(orderedJson)
  if (value === null || typeof value !== 'object')
    return value
  return Object.fromEntries(
    Object.entries(value as Record<string, unknown>)
      .sort(([left], [right]) => compareStrings(left, right))
      .map(([key, item]) => [key, orderedJson(item)]),
  )
}

/** Sorted keys and one trailing newline, so the committed file only changes when the contract does. */
export function serializeContractDocument(document: ContractDocument): string {
  return `${JSON.stringify(orderedJson(document), null, 2)}\n`
}

function jsonSchema(schema: ZodTypeAny, io: 'input' | 'output'): ContractDocument {
  // `io: 'input'` documents what a caller sends: a field with a default stays
  // optional, and a coerced query number documents its wire form.
  const { $schema: _schema, ...body } = z.toJSONSchema(schema, { io, unrepresentable: 'any' }) as ContractDocument
  return body
}

function rewriteDefinitionRefs(value: unknown, references: Readonly<Record<string, string>>): unknown {
  if (Array.isArray(value))
    return value.map(item => rewriteDefinitionRefs(item, references))
  if (value === null || typeof value !== 'object')
    return value
  return Object.fromEntries(Object.entries(value as Record<string, unknown>).map(([key, item]) => {
    if (key === '$ref' && typeof item === 'string' && item.startsWith('#/$defs/')) {
      const reference = references[item.slice('#/$defs/'.length)]
      if (!reference)
        throw new TypeError(`JSON Schema references unknown definition ${item}`)
      return [key, reference]
    }
    return [key, rewriteDefinitionRefs(item, references)]
  }))
}

function registerComponent(
  schema: ZodTypeAny,
  name: string,
  components: Record<string, ContractDocument>,
  io: 'input' | 'output',
): ContractDocument {
  if (name in components)
    throw new TypeError(`Duplicate component schema ${name}`)
  const { $defs, ...body } = jsonSchema(schema, io)
  if ($defs) {
    const definitions = $defs as Record<string, ContractDocument>
    const references = Object.fromEntries(
      Object.keys(definitions).map(definition => [definition, `#/components/schemas/${name}.${definition}`]),
    )
    for (const [definition, value] of Object.entries(definitions))
      components[`${name}.${definition}`] = rewriteDefinitionRefs(value, references) as ContractDocument
    components[name] = rewriteDefinitionRefs(body, references) as ContractDocument
  }
  else {
    components[name] = body
  }
  return { $ref: `#/components/schemas/${name}` }
}

function parameters(
  operation: SkilldV1OperationDefinition,
  location: 'path' | 'query',
): ContractDocument[] {
  const schema = location === 'path' ? operation.request.params : operation.request.query
  if (schema === null)
    return []
  const document = jsonSchema(schema, 'input')
  const properties = (document.properties ?? {}) as Record<string, ContractDocument>
  const required = new Set(Array.isArray(document.required) ? document.required as string[] : [])
  const example = operation.docs.examples[0]?.request[location === 'path' ? 'params' : 'query'] as Record<string, unknown> | undefined
  return Object.entries(properties).map(([name, property]) => ({
    in: location,
    name,
    required: location === 'path' || required.has(name),
    schema: property,
    ...(example && Object.hasOwn(example, name) ? { example: example[name] } : {}),
  }))
}

function responseHeaders(operation: SkilldV1OperationDefinition): Record<string, ContractDocument> {
  return {
    [SKILLD_V1_RESPONSE_HEADERS.requestId]: { description: 'Quote this when you report a problem.', schema: { type: 'string' } },
    [SKILLD_V1_RESPONSE_HEADERS.operation]: { schema: { type: 'string', const: operation.id } },
    ...(operation.lifecycle.deprecated
      ? { [SKILLD_V1_RESPONSE_HEADERS.deprecation]: { description: 'RFC 9745 deprecation date.', schema: { type: 'string' } } }
      : {}),
  }
}

function successResponse(operation: SkilldV1OperationDefinition, components: Record<string, ContractDocument>): ContractDocument {
  const { status, body } = operation.response
  const cacheControl = operation.cache._tag === 'public'
    ? `public, max-age=${operation.cache.maxAgeSeconds}, stale-while-revalidate=${operation.cache.staleWhileRevalidateSeconds}`
    : 'private, no-store'
  const headers = {
    ...responseHeaders(operation),
    'Cache-Control': { schema: { type: 'string', const: cacheControl } },
  }
  if (body === null)
    return { [status]: { description: 'Done. No content.', headers } }
  return {
    [status]: {
      description: operation.docs.summary,
      headers,
      content: {
        'application/json': {
          schema: registerComponent(body.producer, `${operation.id}.response`, components, 'output'),
          example: operation.docs.examples[0]!.response,
        },
      },
    },
  }
}

function errorResponses(operation: SkilldV1OperationDefinition): ContractDocument {
  const byStatus = new Map<number, SkilldV1ErrorCode[]>()
  for (const code of [...operation.errors, ...SKILLD_V1_IMPLICIT_ERRORS]) {
    const status = SKILLD_V1_ERROR_STATUS[code]
    byStatus.set(status, [...(byStatus.get(status) ?? []), code])
  }
  return Object.fromEntries([...byStatus.entries()].map(([status, codes]) => [String(status), {
    'description': `Problem codes: ${codes.join(', ')}.`,
    'x-skilld-problem-codes': codes,
    'headers': responseHeaders(operation),
    'content': { 'application/problem+json': { schema: { $ref: '#/components/schemas/Problem' } } },
  }]))
}

function requestBody(operation: SkilldV1OperationDefinition, components: Record<string, ContractDocument>): ContractDocument | undefined {
  if (operation.request.body === null)
    return undefined
  const example = operation.docs.examples.find(item => Object.hasOwn(item.request, 'body'))?.request.body
  return {
    required: true,
    content: {
      'application/json': {
        schema: registerComponent(operation.request.body, `${operation.id}.request`, components, 'input'),
        ...(example === undefined ? {} : { example }),
      },
    },
  }
}

function operationDocument(operation: SkilldV1OperationDefinition, components: Record<string, ContractDocument>): ContractDocument {
  const allParameters = [...parameters(operation, 'path'), ...parameters(operation, 'query')]
  const body = requestBody(operation, components)
  return {
    'operationId': operation.id,
    'summary': operation.docs.summary,
    'description': operation.docs.description,
    'tags': [operation.docs.tag],
    'security': operation.access === 'account' ? [{ bearerAuth: [] }, { sessionAuth: [] }] : [],
    ...(operation.lifecycle.deprecated ? { deprecated: true } : {}),
    ...(allParameters.length === 0 ? {} : { parameters: allParameters }),
    ...(body ? { requestBody: body } : {}),
    'responses': { ...successResponse(operation, components), ...errorResponses(operation) },
    'x-skilld-lifecycle': operation.lifecycle,
    'x-skilld-semantics': operation.semantics,
  }
}

/**
 * The OpenAPI 3.1 document for every operation in the protocol. The generator
 * script writes it to `generated/openapi.v1.json`, and the site serves that
 * file at `/api/v1/openapi.json`.
 */
export function buildOpenApiDocument(protocol: SkilldV1ProtocolLike): ContractDocument {
  const components: Record<string, ContractDocument> = {
    Problem: jsonSchema(problemSchema.producer, 'output'),
  }
  const paths: Record<string, ContractDocument> = {}
  const tags = new Map<string, string>()
  for (const { registry, operation } of listOperations(protocol)) {
    const path = paths[operation.path] ?? {}
    path[operation.method.toLowerCase()] = operationDocument(operation, components)
    paths[operation.path] = path
    if (!tags.has(operation.docs.tag))
      tags.set(operation.docs.tag, registry.description)
  }
  return orderedJson({
    openapi: '3.1.0',
    jsonSchemaDialect: 'https://json-schema.org/draft/2020-12/schema',
    info: {
      title: 'skilld API',
      version: protocol.version,
      description: 'Read the skilld registry, and manage your account: likes, watches, collections, and the digest. Every answer is plain JSON. Every failure is application/problem+json.',
      license: { name: 'MIT', identifier: 'MIT' },
    },
    servers: [{ url: SKILLD_V1_ORIGIN }],
    tags: [...tags.entries()].map(([name, description]) => ({ name, description })),
    paths,
    components: {
      securitySchemes: {
        bearerAuth: {
          type: 'http',
          scheme: 'bearer',
          description: 'A skilld token. `skilld auth login` stores one, and skilld.dev/me/cli-tokens/new creates one.',
        },
        sessionAuth: { type: 'apiKey', in: 'cookie', name: 'nuxt-session' },
      },
      schemas: components,
    },
  }) as ContractDocument
}
