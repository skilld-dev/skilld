/**
 * Registry client for skilld.dev
 *
 * Talks to the public skilld.dev JSON API: resolves an npm package name to a
 * curated skill's owner/repo, then fetches the detail payload which includes
 * the raw SKILL.md. For local development, set SKILLD_REGISTRY_URL (e.g.
 * http://localhost:3000/api) to point at a running Nuxt dev server.
 *
 * Returns null when a skill is not curated or has no SKILL.md. Transport and
 * contract failures propagate to the command boundary.
 */

import type { AuditStatus } from 'skilld-protocol/constants'
import type {
  AuditEntry,
  ChangeEntry,
  CollectionManifest,
  CollectionManifestItem,
  CollectionSummary,
  CuratorPayload,
  DigestResponse,
  SkillDetailResponse,
  SkillsResolveResponse,
} from 'skilld-protocol/wire'
import type { AuthedFetcher } from '../auth/client.ts'
import { ofetch } from 'ofetch'
import {
  AuditEntrySchema,
  CollectionManifestSchema,
  CollectionSummarySchema,
  CuratorPayloadSchema,
  DigestResponseSchema,
  SkillDetailResponseSchema,
  SkillsResolveResponseSchema,
} from 'skilld-protocol/wire'
import { withAuth } from '../auth/client.ts'
import { TRAILING_SLASH_RE } from '../core/regex.ts'

export type { AuditEntry, AuditStatus, ChangeEntry, CollectionManifest, CollectionManifestItem, CollectionSummary, CuratorPayload, DigestResponse, SkillDetailResponse, SkillsResolveResponse }

const DEFAULT_REGISTRY_URL = 'https://skilld.dev/api'

export function getRegistryBase(): string {
  return (process.env.SKILLD_REGISTRY_URL || DEFAULT_REGISTRY_URL).replace(TRAILING_SLASH_RE, '')
}

export interface RegistrySkill {
  name: string
  packageName: string
  content: string
  owner: string
  repo: string
  displayName?: string
  official?: boolean
  branch?: string
  skillPath?: string
  updatedAt?: string
}

export interface FetchRegistrySkillOptions {
  owner?: string
}

/**
 * CLI-internal computed shape. Built locally after fetching the wire
 * `SkillLiveResponse`; `status` is computed by `aggregateAuditStatus`.
 */
export interface AuditResult {
  status: AuditStatus
  riskLevel?: 'low' | 'medium' | 'high'
  summary?: string
  audits: AuditEntry[]
}

export interface RepositoryRef {
  owner: string
  repo: string
}

export interface RegistryClient {
  resolveSkill: (packageName: string, opts?: FetchRegistrySkillOptions) => Promise<RegistrySkill | null>
  fetchSkillDetail: (owner: string, repo: string, name: string) => Promise<SkillDetailResponse | null>
  audit: (params: { owner: string, repo: string, name: string }) => Promise<AuditResult>
  fetchCollection: (login: string, slug: string) => Promise<CollectionManifest | null>
  fetchCurator: (login: string) => Promise<CuratorPayload | null>
  my: {
    collections: () => Promise<CollectionSummary[]>
    changes: (params: { since?: number }) => Promise<DigestResponse>
    watch: (repos: RepositoryRef[]) => Promise<{ inserted: number }>
    unwatch: (repo: RepositoryRef) => Promise<void>
  }
}

export type GateDecision = 'install' | 'skip' | 'prompt'

export interface GateOptions {
  allowUnsafe?: boolean
  yes?: boolean
  /** Source kind drives unaudited behaviour: gh → prompt, npm/crate → silent install */
  sourceKind: 'npm' | 'gh' | 'crate' | 'collection'
}

/**
 * Pure gating rule from an audit result. Caller is responsible for the prompt
 * itself when the decision is `'prompt'`.
 */
export function gateInstall(result: AuditResult, opts: GateOptions): GateDecision {
  switch (result.status) {
    case 'pass':
      return 'install'
    case 'warn':
      return 'install'
    case 'fail':
      return opts.allowUnsafe ? 'install' : 'skip'
    case 'unaudited':
      if (opts.sourceKind !== 'gh')
        return 'install'
      return opts.yes ? 'install' : 'prompt'
  }
}

export function aggregateAuditStatus(audits: AuditEntry[]): AuditStatus {
  if (audits.length === 0)
    return 'unaudited'
  if (audits.some(a => a.status === 'fail'))
    return 'fail'
  if (audits.some(a => a.status === 'warn'))
    return 'warn'
  return 'pass'
}

export interface RegistryClientOptions {
  baseUrl?: string
  publicFetch?: AuthedFetcher
  authenticatedFetch?: AuthedFetcher
}

function errorStatus(error: unknown): number | null {
  if (typeof error !== 'object' || error === null)
    return null
  if ('statusCode' in error && typeof error.statusCode === 'number')
    return error.statusCode
  if ('status' in error && typeof error.status === 'number')
    return error.status
  return null
}

async function optional<T>(request: Promise<T>): Promise<T | null> {
  return request.catch((error) => {
    if (errorStatus(error) === 404)
      return null
    throw error
  })
}

function parseAuditResponse(value: unknown): AuditResult {
  if (typeof value !== 'object' || value === null)
    throw new TypeError('Invalid audit response')
  const record = value as Record<string, unknown>
  const audits = AuditEntrySchema.array().parse(record.audits ?? [])
  const riskLevel = record.riskLevel === 'low' || record.riskLevel === 'medium' || record.riskLevel === 'high'
    ? record.riskLevel
    : undefined
  const summary = typeof record.summary === 'string' ? record.summary : undefined
  return { status: aggregateAuditStatus(audits), riskLevel, summary, audits }
}

function parseWatchResponse(value: unknown): { inserted: number } {
  if (typeof value !== 'object' || value === null || !('inserted' in value) || typeof value.inserted !== 'number')
    throw new TypeError('Invalid watch response')
  return { inserted: value.inserted }
}

export function createRegistryClient(opts: RegistryClientOptions = {}): RegistryClient {
  const base = (opts.baseUrl ?? getRegistryBase()).replace(TRAILING_SLASH_RE, '')
  const publicFetch = opts.publicFetch ?? ofetch
  const authenticatedFetch = opts.authenticatedFetch ?? withAuth(base)

  return {
    async resolveSkill(packageName, fetchOpts = {}) {
      const resolved = SkillsResolveResponseSchema.parse(await publicFetch<unknown>(`${base}/skills/resolve`, {
        method: 'POST',
        body: { items: [{ packageName, owner: fetchOpts.owner }] },
      }))

      const hit = resolved[packageName]
      if (!hit)
        return null

      const rawDetail = await optional(publicFetch<unknown>(`${base}/skills/${hit.owner}/${hit.repo}/${packageName}`))
      const detail = rawDetail === null ? null : SkillDetailResponseSchema.parse(rawDetail)
      if (!detail?.raw)
        return null

      return {
        name: detail.name,
        packageName,
        content: detail.raw,
        owner: detail.owner,
        repo: `${detail.owner}/${detail.repo}`,
        displayName: detail.displayName,
        official: hit.official,
        branch: detail.branch,
        skillPath: detail.skillPath ?? undefined,
        updatedAt: detail.pushedAt ?? undefined,
      }
    },

    async fetchSkillDetail(owner, repo, name) {
      const response = await optional(publicFetch<unknown>(`${base}/skills/${owner}/${repo}/${name}`))
      return response === null ? null : SkillDetailResponseSchema.parse(response)
    },

    async audit({ owner, repo, name }) {
      return parseAuditResponse(await publicFetch<unknown>(`${base}/skill-live/${owner}/${repo}/${name}`))
    },

    async fetchCollection(login, slug) {
      const response = await optional(publicFetch<unknown>(`${base}/collections/by-author/${login}/${slug}/manifest`))
      return response === null ? null : CollectionManifestSchema.parse(response)
    },

    async fetchCurator(login) {
      const response = await optional(publicFetch<unknown>(`${base}/curators/${login}`))
      return response === null ? null : CuratorPayloadSchema.parse(response)
    },

    my: {
      async collections() {
        return CollectionSummarySchema.array().parse(await authenticatedFetch<unknown>(`${base}/cli/collections`))
      },
      async changes({ since }) {
        const qs = since != null ? `?since=${since}` : ''
        return DigestResponseSchema.parse(await authenticatedFetch<unknown>(`${base}/cli/changes${qs}`))
      },
      async watch(repos) {
        const response = await authenticatedFetch<unknown>(`${base}/me/subscriptions`, {
          method: 'POST',
          body: { source: 'cli', repos },
        })
        return parseWatchResponse(response)
      },
      async unwatch(repo) {
        await authenticatedFetch<void>(`${base}/me/subscriptions/${repo.owner}/${repo.repo}`, { method: 'DELETE' })
      },
    },
  }
}

/**
 * Fetch a curated package skill from the registry.
 * Returns null if no curated skill exists or the SKILL.md cannot be loaded.
 */
export async function fetchRegistrySkill(
  packageName: string,
  opts: FetchRegistrySkillOptions = {},
): Promise<RegistrySkill | null> {
  return createRegistryClient().resolveSkill(packageName, opts)
}
