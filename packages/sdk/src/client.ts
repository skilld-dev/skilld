import type {
  OperationInput,
  OperationOutput,
  SkilldV1ErrorCode,
  SkilldV1OperationDefinition,
  SkilldV1Problem,
  SkilldV1ProtocolLike,
} from './contract/core'
import type { SkilldV1Protocol } from './contract/index'
import {
  buildOperationPath,
  listOperations,
  problemSchema,
  SKILLD_V1_ERROR_STATUS,
  SKILLD_V1_IMPLICIT_ERRORS,
  SKILLD_V1_ORIGIN,
  SKILLD_V1_RESPONSE_HEADERS,
  SKILLD_V1_RETRYABLE_ERRORS,
} from './contract/core'
import { skilldV1Protocol } from './contract/index'

export type Result<TValue, TError>
  = | { _tag: 'Ok', value: TValue, requestId?: string }
    | { _tag: 'Err', error: TError }

/** The input failed the operation's schema, so nothing was sent. */
export interface RequestFailure {
  _tag: 'RequestFailure'
  operationId: string
  location: 'input' | 'params' | 'query' | 'body'
  issues: readonly unknown[]
}

/** skilld.dev answered with a problem the contract declares. */
export interface ApiFailure {
  _tag: 'ApiFailure'
  operationId: string
  code: SkilldV1ErrorCode
  status: number
  title: string
  detail?: string
  requestId?: string
  retryable: boolean
  problem: SkilldV1Problem
}

/** skilld.dev answered with something the contract does not allow. Report it with the request ID. */
export interface ContractFailure {
  _tag: 'ContractFailure'
  operationId: string
  status: number
  message: string
  issues: readonly unknown[]
  requestId?: string
}

/** The request never got an answer. */
export interface TransportFailure {
  _tag: 'TransportFailure'
  operationId: string
  reason: 'aborted' | 'credential' | 'network'
  message: string
  retryable: boolean
  cause?: unknown
}

export type SkilldFailure = RequestFailure | ApiFailure | ContractFailure | TransportFailure

export type FetchImplementation = (input: string, init: RequestInit) => Promise<Response>
export type TokenResolver = string | (() => string | undefined | Promise<string | undefined>)

export interface RetryOptions {
  /** Attempts per call, the first one included. Default 3. */
  maxAttempts?: number
  baseDelayMs?: number
  maxDelayMs?: number
  sleep?: (milliseconds: number, signal?: AbortSignal) => Promise<void>
}

export interface CreateSkilldClientOptions {
  /** Default `https://skilld.dev`. */
  baseUrl?: string
  /**
   * A skilld token: `skilld auth login` stores one, and skilld.dev/me/cli-tokens/new
   * creates one. Only account operations need it.
   */
  token?: TokenResolver
  fetch?: FetchImplementation
  headers?: HeadersInit
  /** Send the skilld.dev sign-in cookie. For a page served from skilld.dev only. */
  credentials?: RequestCredentials
  retry?: RetryOptions
}

export interface CallOptions {
  signal?: AbortSignal
}

/** Calls whose input has no required location take the input as optional. */
type EmptyInput = Record<never, never>

export type OperationCall<TOperation extends SkilldV1OperationDefinition>
  = EmptyInput extends OperationInput<TOperation>
    ? (input?: OperationInput<TOperation>, options?: CallOptions) => Promise<Result<OperationOutput<TOperation>, SkilldFailure>>
    : (input: OperationInput<TOperation>, options?: CallOptions) => Promise<Result<OperationOutput<TOperation>, SkilldFailure>>

export type DomainClient<TOperations extends Readonly<Record<string, SkilldV1OperationDefinition>>> = {
  [TKey in keyof TOperations]: OperationCall<TOperations[TKey]>
}

export type ProtocolClient<TProtocol extends SkilldV1ProtocolLike> = {
  [TName in keyof TProtocol['registries']]: DomainClient<TProtocol['registries'][TName]['operations']>
} & {
  execute: <const TOperation extends SkilldV1OperationDefinition>(
    operation: TOperation,
    input?: OperationInput<TOperation>,
    options?: CallOptions,
  ) => Promise<Result<OperationOutput<TOperation>, SkilldFailure>>
}

export type SkilldClient = ProtocolClient<SkilldV1Protocol>

interface ResolvedRetry {
  maxAttempts: number
  baseDelayMs: number
  maxDelayMs: number
  sleep: (milliseconds: number, signal?: AbortSignal) => Promise<void>
}

function err<TError>(error: TError): { _tag: 'Err', error: TError } {
  return { _tag: 'Err', error }
}

function defaultSleep(milliseconds: number, signal?: AbortSignal): Promise<void> {
  if (signal?.aborted)
    return Promise.reject(signal.reason)
  return new Promise((resolve, reject) => {
    const handle = setTimeout(() => {
      signal?.removeEventListener('abort', onAbort)
      resolve()
    }, milliseconds)
    function onAbort(): void {
      clearTimeout(handle)
      reject(signal?.reason)
    }
    signal?.addEventListener('abort', onAbort, { once: true })
  })
}

function resolveRetry(options: RetryOptions | undefined): ResolvedRetry {
  const maxAttempts = options?.maxAttempts ?? 3
  const baseDelayMs = options?.baseDelayMs ?? 200
  const maxDelayMs = options?.maxDelayMs ?? 5_000
  if (!Number.isInteger(maxAttempts) || maxAttempts < 1 || maxAttempts > 10)
    throw new TypeError('retry.maxAttempts must be an integer from 1 to 10.')
  if (baseDelayMs < 0 || maxDelayMs < baseDelayMs)
    throw new TypeError('retry delays must be nonnegative, and maxDelayMs must be at least baseDelayMs.')
  return { maxAttempts, baseDelayMs, maxDelayMs, sleep: options?.sleep ?? defaultSleep }
}

function retryDelay(attempt: number, retryAfter: string | null, retry: ResolvedRetry): number {
  const exponential = Math.min(retry.maxDelayMs, retry.baseDelayMs * 2 ** (attempt - 1))
  const seconds = retryAfter === null ? Number.NaN : Number(retryAfter)
  const requested = Number.isFinite(seconds) && seconds >= 0 ? Math.ceil(seconds * 1000) : 0
  return Math.min(Math.max(exponential, requested), 60_000)
}

function appendQuery(path: string, query: Record<string, unknown> | undefined): string {
  if (!query)
    return path
  const search = new URLSearchParams()
  for (const key of Object.keys(query).sort()) {
    const value = query[key]
    if (value === undefined || value === null)
      continue
    for (const item of Array.isArray(value) ? value : [value])
      search.append(key, String(item))
  }
  const serialized = search.toString()
  return serialized ? `${path}?${serialized}` : path
}

type Prepared = { _tag: 'Ok', path: string, body?: string } | { _tag: 'Err', error: RequestFailure }

function prepare(operation: SkilldV1OperationDefinition, input: unknown): Prepared {
  const source = (input ?? {}) as Record<string, unknown>
  if (typeof source !== 'object' || Array.isArray(source))
    return err({ _tag: 'RequestFailure', operationId: operation.id, location: 'input', issues: ['input must be an object'] })
  const extra = Object.keys(source).filter(key => key !== 'params' && key !== 'query' && key !== 'body')
  if (extra.length > 0)
    return err({ _tag: 'RequestFailure', operationId: operation.id, location: 'input', issues: [`unknown input keys: ${extra.join(', ')}`] })
  const parsed: Record<string, unknown> = {}
  for (const location of ['params', 'query', 'body'] as const) {
    const schema = operation.request[location]
    const supplied = source[location] !== undefined
    if (!schema) {
      if (supplied)
        return err({ _tag: 'RequestFailure', operationId: operation.id, location, issues: [`${operation.id} takes no ${location}`] })
      continue
    }
    const result = schema.safeParse(supplied ? source[location] : location === 'body' ? undefined : {})
    if (!result.success)
      return err({ _tag: 'RequestFailure', operationId: operation.id, location, issues: result.error.issues })
    parsed[location] = result.data
  }
  return {
    _tag: 'Ok',
    path: appendQuery(buildOperationPath(operation, parsed.params), parsed.query as Record<string, unknown> | undefined),
    body: parsed.body === undefined ? undefined : JSON.stringify(parsed.body),
  }
}

async function readJson(response: Response): Promise<{ _tag: 'Ok', value: unknown } | { _tag: 'Err', cause: unknown }> {
  const text = await response.text()
  if (!text)
    return { _tag: 'Ok', value: null }
  return Promise.resolve()
    .then(() => ({ _tag: 'Ok' as const, value: JSON.parse(text) as unknown }))
    .catch((cause: unknown) => ({ _tag: 'Err' as const, cause }))
}

async function resolveToken(token: TokenResolver | undefined): Promise<string | undefined> {
  const value = typeof token === 'function' ? await token() : token
  return value?.trim() || undefined
}

/**
 * A typed client for the skilld API. Every call answers a `Result`: check
 * `_tag` before you read `value`. Nothing throws for an expected failure.
 *
 * ```ts
 * const skilld = createSkilldClient({ token: process.env.SKILLD_TOKEN })
 * const found = await skilld.skills.search({ query: { q: 'tailwind' } })
 * if (found._tag === 'Err')
 *   throw new Error(found.error._tag)
 * ```
 */
export function createSkilldClient(options: CreateSkilldClientOptions = {}): SkilldClient {
  return createProtocolClient(skilldV1Protocol, options)
}

export function createProtocolClient<const TProtocol extends SkilldV1ProtocolLike>(
  protocol: TProtocol,
  options: CreateSkilldClientOptions = {},
): ProtocolClient<TProtocol> {
  const retry = resolveRetry(options.retry)
  const baseUrl = (options.baseUrl ?? SKILLD_V1_ORIGIN).replace(/\/+$/, '')
  const fetchImplementation = options.fetch ?? globalThis.fetch?.bind(globalThis)
  if (typeof fetchImplementation !== 'function')
    throw new TypeError('createSkilldClient needs a fetch implementation in this runtime.')
  const registered = new Map(listOperations(protocol).map(({ operation }) => [operation.id, operation]))

  async function execute<const TOperation extends SkilldV1OperationDefinition>(
    operation: TOperation,
    input?: OperationInput<TOperation>,
    callOptions: CallOptions = {},
  ): Promise<Result<OperationOutput<TOperation>, SkilldFailure>> {
    const known = registered.get(operation.id)
    if (!known || known.method !== operation.method || known.path !== operation.path)
      return err({ _tag: 'RequestFailure', operationId: operation.id, location: 'input', issues: [`${operation.id} is not part of this client's protocol`] })
    const prepared = prepare(operation, input)
    if (prepared._tag === 'Err')
      return prepared
    const canRetry = operation.semantics.kind === 'query' || operation.semantics.retry === 'idempotent'
    const declared = new Set<SkilldV1ErrorCode>([...operation.errors, ...SKILLD_V1_IMPLICIT_ERRORS])

    for (let attempt = 1; ; attempt++) {
      const headers = new Headers(options.headers)
      headers.set('accept', 'application/json')
      if (prepared.body !== undefined)
        headers.set('content-type', 'application/json')
      const token = await Promise.resolve(options.token)
        .then(resolveToken)
        .then(value => ({ _tag: 'Ok' as const, value }), (cause: unknown) => ({ _tag: 'Err' as const, cause }))
      if (token._tag === 'Err')
        return err({ _tag: 'TransportFailure', operationId: operation.id, reason: 'credential', message: 'Could not resolve the skilld token.', retryable: false, cause: token.cause })
      if (token.value)
        headers.set('authorization', `Bearer ${token.value}`)

      const sent = await fetchImplementation(`${baseUrl}${prepared.path}`, {
        method: operation.method,
        headers,
        body: prepared.body,
        signal: callOptions.signal,
        credentials: options.credentials,
      }).then(
        response => ({ _tag: 'Ok' as const, response }),
        (cause: unknown) => ({ _tag: 'Err' as const, cause }),
      )
      if (sent._tag === 'Err') {
        const aborted = callOptions.signal?.aborted === true
        if (!aborted && canRetry && attempt < retry.maxAttempts) {
          const waited = await retry.sleep(retryDelay(attempt, null, retry), callOptions.signal).then(() => true, () => false)
          if (waited)
            continue
        }
        return err({
          _tag: 'TransportFailure',
          operationId: operation.id,
          reason: aborted ? 'aborted' : 'network',
          message: `${operation.id}: ${aborted ? 'request aborted' : 'network request failed'}`,
          retryable: !aborted && canRetry,
          cause: sent.cause,
        })
      }

      const response = sent.response
      const requestId = response.headers.get(SKILLD_V1_RESPONSE_HEADERS.requestId) ?? undefined
      const contractFailure = (message: string, issues: readonly unknown[] = []) =>
        err<ContractFailure>({ _tag: 'ContractFailure', operationId: operation.id, status: response.status, message, issues, requestId })

      if (response.status === operation.response.status) {
        const body = operation.response.body
        if (body === null) {
          await response.body?.cancel()
          return { _tag: 'Ok', value: null as OperationOutput<TOperation>, requestId }
        }
        const payload = await readJson(response)
        if (payload._tag === 'Err')
          return contractFailure(`${operation.id}: the answer was not JSON`, [payload.cause])
        const parsed = body.client.safeParse(payload.value)
        return parsed.success
          ? { _tag: 'Ok', value: parsed.data as OperationOutput<TOperation>, requestId }
          : contractFailure(`${operation.id}: the answer broke its schema`, parsed.error.issues)
      }
      if (response.ok)
        return contractFailure(`${operation.id}: undeclared success status ${response.status}`)

      const payload = await readJson(response)
      const problem = payload._tag === 'Ok' ? problemSchema.client.safeParse(payload.value) : null
      if (!problem?.success)
        return contractFailure(`${operation.id}: status ${response.status} without a problem body`)
      const { code } = problem.data
      if (!declared.has(code) || SKILLD_V1_ERROR_STATUS[code] !== response.status)
        return contractFailure(`${operation.id}: undeclared problem ${code} with status ${response.status}`)
      const failure: ApiFailure = {
        _tag: 'ApiFailure',
        operationId: operation.id,
        code,
        status: response.status,
        title: problem.data.title,
        detail: problem.data.detail,
        requestId,
        retryable: canRetry && SKILLD_V1_RETRYABLE_ERRORS.has(code),
        problem: problem.data,
      }
      if (failure.retryable && attempt < retry.maxAttempts) {
        const waited = await retry.sleep(retryDelay(attempt, response.headers.get(SKILLD_V1_RESPONSE_HEADERS.retryAfter), retry), callOptions.signal)
          .then(() => true, () => false)
        if (waited)
          continue
      }
      return err(failure)
    }
  }

  const client: Record<string, unknown> = { execute }
  for (const [name, registry] of Object.entries(protocol.registries)) {
    client[name] = Object.fromEntries(Object.entries(registry.operations).map(([key, operation]) => [
      key,
      (input?: unknown, callOptions?: CallOptions) => execute(operation, input as never, callOptions),
    ]))
  }
  return client as ProtocolClient<TProtocol>
}
