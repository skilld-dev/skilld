/**
 * Registry client for skilld.dev
 *
 * Talks to the public skilld.dev JSON API: resolves an npm package name to a
 * curated skill's owner/repo, then fetches the detail payload which includes
 * the raw SKILL.md. For local development, set SKILLD_REGISTRY_URL (e.g.
 * http://localhost:3000/api) to point at a running Nuxt dev server.
 *
 * Returns null when a skill isn't curated, the API is unreachable, or the
 * skill has no resolvable SKILL.md, so callers fall through to the
 * doc-generation pipeline.
 */

import type { AuditStatus } from 'skilld-protocol/constants'
import type {
  AuditEntry,
  AuthSession,
  ChangeEntry,
  CollectionManifest,
  CollectionManifestItem,
  CollectionSummary,
  CuratorPayload,
  DigestResponse,
  InstallEventPayload,
  SkillDetailResponse,
  SkillsResolveEntry,
  SkillsResolveResponse,
  SubscriptionSummary,
} from 'skilld-protocol/wire'
import { ofetch } from 'ofetch'
import { TRAILING_SLASH_RE } from '../core/regex.ts'

export type { AuditEntry, AuditStatus, AuthSession, ChangeEntry, CollectionManifest, CollectionManifestItem, CollectionSummary, CuratorPayload, DigestResponse, InstallEventPayload, SkillDetailResponse, SkillsResolveResponse, SubscriptionSummary }

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
  installs?: number
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

interface AuditApiResponse {
  riskLevel?: 'low' | 'medium' | 'high'
  summary?: string
  audits?: AuditEntry[]
}

export interface RegistryClient {
  resolveSkill: (packageName: string, opts?: FetchRegistrySkillOptions) => Promise<RegistrySkill | null>
  fetchSkillDetail: (owner: string, repo: string, name: string) => Promise<SkillDetailResponse | null>
  audit: (params: { owner: string, repo: string, name: string }) => Promise<AuditResult>
  fetchCollection: (login: string, slug: string) => Promise<CollectionManifest | null>
  fetchCurator: (login: string) => Promise<CuratorPayload | null>
  my: {
    collections: () => Promise<CollectionSummary[]>
    subscriptions: () => Promise<SubscriptionSummary[]>
    changes: (params: { since?: number }) => Promise<DigestResponse | null>
    installs: (payload: InstallEventPayload) => Promise<void>
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

export function createRegistryClient(opts: { session?: AuthSession, baseUrl?: string } = {}): RegistryClient {
  const base = (opts.baseUrl ?? getRegistryBase()).replace(TRAILING_SLASH_RE, '')
  const headers = opts.session ? { Authorization: `Bearer ${opts.session.accessToken}` } : undefined

  const fetcher = <T>(url: string, init?: Parameters<typeof ofetch<T>>[1]): Promise<T> => {
    if (!headers && !init)
      return ofetch<T>(url)
    if (!headers)
      return ofetch<T>(url, init)
    return ofetch<T>(url, { ...init, headers: { ...headers, ...(init?.headers as any) } })
  }

  const requireSession = (): void => {
    if (!opts.session)
      throw new Error('auth required')
  }

  return {
    async resolveSkill(packageName, fetchOpts = {}) {
      const resolved = await fetcher<Record<string, SkillsResolveEntry>>(`${base}/skills/resolve`, {
        method: 'POST',
        body: { items: [{ packageName, owner: fetchOpts.owner }] },
      }).catch(() => null)

      const hit = resolved?.[packageName]
      if (!hit)
        return null

      const detail = await fetcher<SkillDetailResponse>(`${base}/skills/${hit.owner}/${hit.repo}/${packageName}`).catch(() => null)
      if (!detail?.raw)
        return null

      return {
        name: detail.name,
        packageName,
        content: detail.raw,
        owner: detail.owner,
        repo: `${detail.owner}/${detail.repo}`,
        displayName: detail.displayName,
        installs: detail.installs,
        official: hit.official,
        branch: detail.branch,
        skillPath: detail.skillPath ?? undefined,
        updatedAt: detail.pushedAt ?? undefined,
      }
    },

    async fetchSkillDetail(owner, repo, name) {
      return fetcher<SkillDetailResponse>(`${base}/skills/${owner}/${repo}/${name}`).catch(() => null)
    },

    async audit({ owner, repo, name }) {
      const res = await fetcher<AuditApiResponse>(`${base}/skill-live/${owner}/${repo}/${name}`).catch(() => null)
      if (!res)
        return { status: 'unaudited', audits: [] }
      const audits = res.audits ?? []
      return {
        status: aggregateAuditStatus(audits),
        riskLevel: res.riskLevel,
        summary: res.summary,
        audits,
      }
    },

    async fetchCollection(login, slug) {
      return fetcher<CollectionManifest>(`${base}/collections/by-author/${login}/${slug}/manifest`).catch(() => null)
    },

    async fetchCurator(login) {
      return fetcher<CuratorPayload>(`${base}/curators/${login}`).catch(() => null)
    },

    my: {
      async collections() {
        requireSession()
        return fetcher<CollectionSummary[]>(`${base}/me/collections`).catch(() => [])
      },
      async subscriptions() {
        requireSession()
        return fetcher<SubscriptionSummary[]>(`${base}/me/subscriptions`).catch(() => [])
      },
      async changes({ since }) {
        requireSession()
        const qs = since != null ? `?since=${since}` : ''
        return fetcher<DigestResponse>(`${base}/cli/changes${qs}`).catch(() => null)
      },
      async installs(payload) {
        requireSession()
        await fetcher<void>(`${base}/me/installs`, { method: 'POST', body: payload }).catch(() => {})
      },
    },
  }
}

/**
 * Fetch a curated package skill from the registry.
 * Returns null if no curated skill exists, the SKILL.md can't be loaded, or the API is unreachable.
 *
 * Thin wrapper over `createRegistryClient().resolveSkill` for back-compat.
 */
export async function fetchRegistrySkill(
  packageName: string,
  opts: FetchRegistrySkillOptions = {},
): Promise<RegistrySkill | null> {
  return createRegistryClient().resolveSkill(packageName, opts)
}
