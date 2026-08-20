import type { Hash } from 'node:crypto'
import type { SkillOutputPolicy, SkillRunError, SourceAttempt } from '../../types.ts'
import type { Result } from '../result.ts'
import type { PreparedFile, PreparedSource } from './host.ts'
import { createHash } from 'node:crypto'
import { posix } from 'node:path'
import { err, ok } from '../result.ts'
import { extractArchive } from './archive.ts'

interface NpmDist {
  readonly tarball: string
  readonly integrity?: string
  readonly shasum?: string
}

interface NpmRepository {
  readonly type?: string
  readonly url?: string
}

interface NpmVersion {
  readonly version: string
  readonly dist: NpmDist
  readonly gitHead?: string
  readonly repository?: NpmRepository | string
}

interface IntegrityVerifier {
  readonly update: (content: Uint8Array) => void
  readonly valid: () => boolean
}

const packageSpecPattern = /^(@[^/@]+\/[^/@]+|[^/@]+)(?:@(\S+))?$/
const exactVersionPattern = /^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)(?:-[\da-z-]+(?:\.[\da-z-]+)*)?(?:\+[\da-z-]+(?:\.[\da-z-]+)*)?$/i
const gitCommitPattern = /^[0-9a-f]{40}$/i
const npmRequestTimeoutMs = 30_000
const npmRedirectLimit = 3
const packumentByteLimit = 10 * 1024 * 1024

export type FetchClient = (input: string | URL | Request, init?: RequestInit) => Promise<Response>

function unavailable(message: string, attempts: ReadonlyArray<SourceAttempt>, cause?: unknown): Result<never, SkillRunError> {
  return err({ _tag: 'SourceUnavailable', message, attempts, cause })
}

function parsePackageSpec(spec: string): { name: string, version: string } | null {
  const match = spec.match(packageSpecPattern)
  return match?.[1] ? { name: match[1], version: match[2] ?? 'latest' } : null
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function parseRepository(value: unknown): NpmRepository | string | undefined {
  if (typeof value === 'string')
    return value
  if (!isRecord(value))
    return undefined
  return {
    type: typeof value.type === 'string' ? value.type : undefined,
    url: typeof value.url === 'string' ? value.url : undefined,
  }
}

function parseVersion(packument: unknown, requested: string): { resolvedVersion: string, version: NpmVersion, repository?: NpmRepository | string } | null {
  if (!isRecord(packument) || !isRecord(packument['dist-tags']) || !isRecord(packument.versions))
    return null
  const tagged = packument['dist-tags'][requested]
  const resolvedVersion = typeof tagged === 'string' ? tagged : requested
  const value = packument.versions[resolvedVersion]
  if (!isRecord(value) || typeof value.version !== 'string' || !isRecord(value.dist) || typeof value.dist.tarball !== 'string')
    return null
  return {
    resolvedVersion,
    repository: parseRepository(packument.repository),
    version: {
      version: value.version,
      gitHead: typeof value.gitHead === 'string' ? value.gitHead : undefined,
      repository: parseRepository(value.repository),
      dist: {
        tarball: value.dist.tarball,
        integrity: typeof value.dist.integrity === 'string' ? value.dist.integrity : undefined,
        shasum: typeof value.dist.shasum === 'string' ? value.dist.shasum : undefined,
      },
    },
  }
}

async function cancelBody(response: Response): Promise<void> {
  if (!response.body)
    return
  // A failed cleanup cannot replace the source failure.
  await response.body.cancel().catch(() => undefined)
}

function declaredBytes(response: Response): number | null {
  const value = response.headers.get('content-length')
  if (value === null || !/^\d+$/.test(value))
    return null
  const bytes = Number(value)
  return Number.isSafeInteger(bytes) ? bytes : null
}

async function* responseChunks(
  response: Response,
  maxBytes: number,
  limitMessage: string,
  onChunk: (content: Uint8Array) => void = () => {},
): AsyncGenerator<Uint8Array> {
  const declared = declaredBytes(response)
  if (declared !== null && declared > maxBytes) {
    await cancelBody(response)
    throw new Error(limitMessage)
  }
  if (!response.body)
    return

  const reader = response.body.getReader()
  let complete = false
  let totalBytes = 0
  try {
    while (true) {
      const result = await reader.read()
      if (result.done) {
        complete = true
        return
      }
      totalBytes += result.value.byteLength
      if (totalBytes > maxBytes)
        throw new Error(limitMessage)
      onChunk(result.value)
      yield result.value
    }
  }
  finally {
    if (!complete) {
      // A failed cleanup cannot replace the source failure.
      await reader.cancel().catch(() => undefined)
    }
    reader.releaseLock()
  }
}

async function readBoundedResponse(response: Response, maxBytes: number): Promise<Result<Uint8Array, SkillRunError>> {
  const chunks: Uint8Array[] = []
  let totalBytes = 0
  try {
    for await (const chunk of responseChunks(response, maxBytes, 'npm response exceeds the byte limit.')) {
      chunks.push(chunk)
      totalBytes += chunk.byteLength
    }
  }
  catch (cause) {
    return unavailable(cause instanceof Error ? cause.message : 'npm response cannot be read.', [], cause)
  }
  return ok(Uint8Array.from(Buffer.concat(chunks, totalBytes)))
}

async function fetchWithRedirects(fetchClient: FetchClient, url: string, init: RequestInit = {}, signal?: AbortSignal): Promise<Result<Response, SkillRunError>> {
  const timeout = AbortSignal.timeout(npmRequestTimeoutMs)
  const requestSignal = signal ? AbortSignal.any([signal, timeout]) : timeout
  let current = url
  for (let redirects = 0; redirects <= npmRedirectLimit; redirects += 1) {
    const response = await fetchClient(current, { ...init, redirect: 'manual', signal: requestSignal }).catch(error => error as Error)
    if (response instanceof Error)
      return unavailable(timeout.aborted ? 'npm request timed out.' : 'npm request failed.', [], response)
    if (![301, 302, 303, 307, 308].includes(response.status))
      return ok(response)
    if (redirects === npmRedirectLimit) {
      await cancelBody(response)
      return unavailable('npm request exceeded the redirect limit.', [])
    }
    const location = response.headers.get('location')
    if (!location) {
      await cancelBody(response)
      return unavailable('npm redirect is missing its location.', [])
    }
    const redirected = await Promise.resolve().then(() => new URL(location, current)).catch(() => null)
    if (redirected === null) {
      await cancelBody(response)
      return unavailable('npm redirect location is invalid.', [])
    }
    if (redirected.protocol !== 'https:') {
      await cancelBody(response)
      return unavailable('npm redirect URL must use HTTPS.', [])
    }
    await cancelBody(response)
    current = redirected.href
  }
  return unavailable('npm request exceeded the redirect limit.', [])
}

function createIntegrityVerifier(dist: NpmDist): IntegrityVerifier | null {
  const values = dist.integrity?.split(/\s+/).filter(Boolean) ?? []
  const expected = values.flatMap((value) => {
    const match = value.match(/^(sha(?:1|256|384|512))-([A-Za-z0-9+/=]+)$/)
    return match?.[1] && match[2] ? [{ algorithm: match[1], digest: match[2] }] : []
  })
  if (values.length === 0 && dist.shasum)
    expected.push({ algorithm: 'sha1', digest: Buffer.from(dist.shasum, 'hex').toString('base64') })
  if (expected.length === 0)
    return null

  const hashes = new Map<string, Hash>()
  for (const value of expected)
    hashes.set(value.algorithm, createHash(value.algorithm))
  return {
    update(content) {
      for (const hash of hashes.values())
        hash.update(content)
    },
    valid() {
      const actual = new Map([...hashes].map(([algorithm, hash]) => [algorithm, hash.digest('base64')]))
      return expected.some(value => actual.get(value.algorithm) === value.digest)
    },
  }
}

function repositoryUrl(repository: NpmRepository | string | undefined): string | null {
  if (typeof repository === 'string')
    return repository
  return typeof repository?.url === 'string' ? repository.url : null
}

function githubArchiveUrl(repository: NpmRepository | string | undefined, gitHead: string | undefined): string | null {
  if (!gitHead || !gitCommitPattern.test(gitHead))
    return null
  const source = repositoryUrl(repository)
  if (source === null)
    return null
  const match = source.match(/^(?:(?:git\+)?https?:\/\/(?:git@)?github\.com\/|git:\/\/github\.com\/|git@github\.com:)([\w.-]+)\/([\w.-]+?)(?:\.git)?\/?$/)
  if (!match?.[1] || !match[2])
    return null
  return `https://codeload.github.com/${encodeURIComponent(match[1])}/${encodeURIComponent(match[2])}/tar.gz/${gitHead}`
}

function skipped(source: string, reason: string): SourceAttempt {
  return { source, status: 'skipped', reason }
}

function used(source: string): SourceAttempt {
  return { source, status: 'used' }
}

function withAttempts(error: SkillRunError, attempts: ReadonlyArray<SourceAttempt>): Result<never, SkillRunError> {
  return error._tag === 'SourceUnavailable'
    ? unavailable(error.message, attempts, error.cause)
    : err(error)
}

function remainingPolicy(policy: SkillOutputPolicy, files: ReadonlyArray<PreparedFile>): SkillOutputPolicy | null {
  const usedBytes = files.reduce((total, file) => total + file.content.byteLength, 0)
  const maxSourceFiles = policy.maxSourceFiles - files.length
  const maxSourceBytes = policy.maxSourceBytes - usedBytes
  if (maxSourceFiles <= 0 || maxSourceBytes <= 0)
    return null
  return { ...policy, maxSourceFiles, maxSourceBytes }
}

async function extractResponse(
  response: Response,
  policy: SkillOutputPolicy,
  layout: 'github' | 'npm',
  verifier?: IntegrityVerifier,
): Promise<Result<ReadonlyArray<PreparedFile>, SkillRunError>> {
  return extractArchive(
    responseChunks(
      response,
      policy.maxSourceBytes,
      'Source archive exceeds the compressed byte limit.',
      content => verifier?.update(content),
    ),
    policy,
    layout,
  )
}

export async function prepareNpmPackage(spec: string, policy: SkillOutputPolicy, fetchClient: FetchClient, signal?: AbortSignal): Promise<Result<PreparedSource, SkillRunError>> {
  const parsed = parsePackageSpec(spec)
  if (parsed === null)
    return unavailable('npm package spec is invalid.', [])

  const attempts: SourceAttempt[] = []
  const registryUrl = `https://registry.npmjs.org/${encodeURIComponent(parsed.name)}`
  const registryResponse = await fetchWithRedirects(fetchClient, registryUrl, {
    headers: { accept: 'application/vnd.npm.install-v1+json' },
  }, signal)
  if (registryResponse._tag === 'Err')
    return withAttempts(registryResponse.error, [skipped(registryUrl, registryResponse.error.message)])
  const response = registryResponse.value
  if (!response.ok) {
    await cancelBody(response)
    const message = `npm registry returned HTTP ${response.status}.`
    return unavailable(message, [skipped(registryUrl, message)])
  }

  const packumentBytes = await readBoundedResponse(response, packumentByteLimit)
  if (packumentBytes._tag === 'Err')
    return withAttempts(packumentBytes.error, [skipped(registryUrl, packumentBytes.error.message)])

  let packument: unknown
  try {
    packument = JSON.parse(Buffer.from(packumentBytes.value).toString('utf8')) as unknown
  }
  catch (cause) {
    const message = 'npm registry returned invalid package data.'
    return unavailable(message, [skipped(registryUrl, message)], cause)
  }

  const resolution = parseVersion(packument, parsed.version)
  if (resolution === null || !exactVersionPattern.test(resolution.resolvedVersion) || resolution.version.version !== resolution.resolvedVersion) {
    const message = 'npm package did not resolve to an exact version.'
    return unavailable(message, [skipped(registryUrl, message)])
  }
  const { repository, resolvedVersion, version } = resolution
  if (!version.dist.tarball.startsWith('https://')) {
    const message = 'npm package archive URL must use HTTPS.'
    return unavailable(message, [skipped(registryUrl, message)])
  }
  attempts.push(used(registryUrl))

  const fetchedArchive = await fetchWithRedirects(fetchClient, version.dist.tarball, {}, signal)
  if (fetchedArchive._tag === 'Err')
    return withAttempts(fetchedArchive.error, [...attempts, skipped(version.dist.tarball, fetchedArchive.error.message)])
  const archiveResponse = fetchedArchive.value
  if (!archiveResponse.ok) {
    await cancelBody(archiveResponse)
    const message = `npm package archive returned HTTP ${archiveResponse.status}.`
    return unavailable(message, [...attempts, skipped(version.dist.tarball, message)])
  }

  const verifier = createIntegrityVerifier(version.dist)
  if (verifier === null) {
    await cancelBody(archiveResponse)
    const message = 'npm package archive has no supported integrity value.'
    return unavailable(message, [...attempts, skipped(version.dist.tarball, message)])
  }
  const extracted = await extractResponse(archiveResponse, policy, 'npm', verifier)
  if (extracted._tag === 'Err')
    return withAttempts(extracted.error, [...attempts, skipped(version.dist.tarball, extracted.error.message)])
  if (!verifier.valid()) {
    const message = 'npm package archive integrity check failed.'
    return unavailable(message, [...attempts, skipped(version.dist.tarball, message)])
  }
  attempts.push(used(version.dist.tarball))
  let files = [...extracted.value]

  const repositoryArchive = githubArchiveUrl(version.repository ?? repository, version.gitHead)
  const available = remainingPolicy(policy, files)
  if (repositoryArchive !== null && available === null) {
    attempts.push(skipped(repositoryArchive, 'Prepared package files reached the source limit.'))
  }
  else if (repositoryArchive !== null && available !== null) {
    const fetchedRepository = await fetchWithRedirects(fetchClient, repositoryArchive, {}, signal)
    if (fetchedRepository._tag === 'Err') {
      attempts.push(skipped(repositoryArchive, fetchedRepository.error.message))
    }
    else if (!fetchedRepository.value.ok) {
      const message = `GitHub source archive returned HTTP ${fetchedRepository.value.status}.`
      await cancelBody(fetchedRepository.value)
      attempts.push(skipped(repositoryArchive, message))
    }
    else {
      const repositoryFiles = await extractResponse(fetchedRepository.value, available, 'github')
      if (repositoryFiles._tag === 'Err') {
        attempts.push(skipped(repositoryArchive, repositoryFiles.error.message))
      }
      else {
        files = [
          ...files,
          ...repositoryFiles.value.map(file => ({ ...file, path: posix.join('repository', file.path) })),
        ]
        attempts.push(used(repositoryArchive))
      }
    }
  }

  files.sort((left, right) => left.path.localeCompare(right.path))
  return ok({
    files,
    attempts,
    npmResolution: { package: parsed.name, version: resolvedVersion },
  })
}
