import type { ResolveAttempt, ResolvedPackage, ResolveResult } from './types.ts'
import { isLikelyCodeHostUrl, isUselessDocsUrl, normalizeRepoUrl, parseGitHubUrl } from '../core/url.ts'
import { resolveGitHubRepo } from './github.ts'
import { fetchLlmsUrl } from './llms.ts'
import { $fetch, createRateLimitedRunner } from './utils.ts'

const VALID_CRATE_NAME = /^[a-z0-9][\w-]*$/
const runCratesApiRateLimited = createRateLimitedRunner(1000)

interface CratesApiResponse {
  crate?: {
    id?: string
    name?: string
    description?: string
    homepage?: string | null
    documentation?: string | null
    repository?: string | null
    max_version?: string
    newest_version?: string
    max_stable_version?: string
    default_version?: string
    updated_at?: string
  }
  versions?: Array<{
    num?: string
    yanked?: boolean
    created_at?: string
    description?: string | null
    homepage?: string | null
    documentation?: string | null
    repository?: string | null
  }>
}

function selectCrateVersion(
  data: CratesApiResponse,
  requestedVersion?: string,
): { version: string, entry?: NonNullable<CratesApiResponse['versions']>[number] } | null {
  const versions = data.versions || []

  if (requestedVersion) {
    const exact = versions.find(v => v.num === requestedVersion && !v.yanked)
    if (exact?.num)
      return { version: exact.num, entry: exact }
  }

  const crate = data.crate
  const preferred = [
    crate?.max_stable_version,
    crate?.newest_version,
    crate?.max_version,
    crate?.default_version,
  ].find(Boolean)

  if (preferred) {
    const match = versions.find(v => v.num === preferred && !v.yanked)
    if (match?.num)
      return { version: preferred, entry: match }
    if (versions.length === 0)
      return { version: preferred }
  }

  const firstStable = versions.find(v => !v.yanked && v.num)
  if (firstStable?.num)
    return { version: firstStable.num, entry: firstStable }

  return null
}

function pickPreferredUrl(...urls: Array<string | null | undefined>): string | undefined {
  return urls.map(v => v?.trim()).find(v => !!v)
}

async function fetchCratesApi<T>(url: string): Promise<T | null> {
  return runCratesApiRateLimited(() => $fetch<T>(url).catch(() => null))
}

export async function resolveCrateDocsWithAttempts(
  crateName: string,
  options: { version?: string, onProgress?: (message: string) => void } = {},
): Promise<ResolveResult> {
  const attempts: ResolveAttempt[] = []
  const onProgress = options.onProgress
  const normalizedName = crateName.trim().toLowerCase()

  if (!normalizedName || !VALID_CRATE_NAME.test(normalizedName)) {
    attempts.push({
      source: 'crates',
      status: 'error',
      message: `Invalid crate name: ${crateName}`,
    })
    return { package: null, attempts }
  }

  onProgress?.('crates.io metadata')
  const apiUrl = `https://crates.io/api/v1/crates/${encodeURIComponent(normalizedName)}`
  const data = await fetchCratesApi<CratesApiResponse>(apiUrl)

  if (!data?.crate) {
    attempts.push({
      source: 'crates',
      url: apiUrl,
      status: 'not-found',
      message: 'Crate not found on crates.io',
    })
    return { package: null, attempts }
  }

  attempts.push({
    source: 'crates',
    url: apiUrl,
    status: 'success',
    message: `Found crate: ${data.crate.name || normalizedName}`,
  })

  const selected = selectCrateVersion(data, options.version)
  if (!selected) {
    attempts.push({
      source: 'crates',
      url: apiUrl,
      status: 'error',
      message: 'No usable crate versions found',
    })
    return { package: null, attempts }
  }

  const version = selected.version
  const versionEntry = selected.entry
  const docsRsUrl = `https://docs.rs/${encodeURIComponent(normalizedName)}/${encodeURIComponent(version)}`

  const repositoryRaw = pickPreferredUrl(versionEntry?.repository, data.crate.repository)
  const homepage = pickPreferredUrl(versionEntry?.homepage, data.crate.homepage)
  const documentation = pickPreferredUrl(versionEntry?.documentation, data.crate.documentation)
  const normalizedRepo = repositoryRaw ? normalizeRepoUrl(repositoryRaw) : undefined
  const repoUrl = normalizedRepo && isLikelyCodeHostUrl(normalizedRepo)
    ? normalizedRepo
    : isLikelyCodeHostUrl(homepage)
      ? homepage
      : undefined

  let resolved: ResolvedPackage = {
    name: normalizedName,
    version,
    releasedAt: versionEntry?.created_at || data.crate.updated_at || undefined,
    description: versionEntry?.description || data.crate.description,
    docsUrl: (() => {
      if (documentation && !isUselessDocsUrl(documentation) && !isLikelyCodeHostUrl(documentation))
        return documentation
      if (homepage && !isUselessDocsUrl(homepage) && !isLikelyCodeHostUrl(homepage))
        return homepage
      return docsRsUrl
    })(),
    repoUrl,
  }

  const gh = repoUrl ? parseGitHubUrl(repoUrl) : null
  if (gh) {
    onProgress?.('GitHub enrichment')
    const ghResolved = await resolveGitHubRepo(gh.owner, gh.repo)
    if (ghResolved) {
      attempts.push({
        source: 'github-meta',
        url: repoUrl,
        status: 'success',
        message: 'Enriched via GitHub repo metadata',
      })
      resolved = {
        ...ghResolved,
        name: normalizedName,
        version,
        releasedAt: resolved.releasedAt || ghResolved.releasedAt,
        description: resolved.description || ghResolved.description,
        docsUrl: resolved.docsUrl || ghResolved.docsUrl,
        repoUrl,
        readmeUrl: ghResolved.readmeUrl || resolved.readmeUrl,
      }
    }
    else {
      attempts.push({
        source: 'github-meta',
        url: repoUrl,
        status: 'not-found',
        message: 'GitHub enrichment failed, using crates.io metadata',
      })
    }
  }

  if (!resolved.llmsUrl && resolved.docsUrl) {
    onProgress?.('llms.txt discovery')
    resolved.llmsUrl = await fetchLlmsUrl(resolved.docsUrl).catch(() => null) ?? undefined
    if (resolved.llmsUrl) {
      attempts.push({
        source: 'llms.txt',
        url: resolved.llmsUrl,
        status: 'success',
      })
    }
  }

  return { package: resolved, attempts }
}
