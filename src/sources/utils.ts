/**
 * Shared utilities for doc resolution
 */

import { ofetch } from 'ofetch'
import { getGitHubToken, isKnownPrivateRepo, markRepoPrivate } from './github-common.ts'

export const SKILLD_USER_AGENT = 'skilld/1.0 (+https://github.com/harlan-zw/skilld)'

export const $fetch = ofetch.create({
  retry: 3,
  retryDelay: 1000,
  retryStatusCodes: [408, 429, 500, 502, 503, 504],
  timeout: 15_000,
  headers: { 'User-Agent': SKILLD_USER_AGENT },
})

/**
 * Create a rate-limited runner that enforces a minimum gap between task starts.
 * Queues tasks serially so consumers don't need to coordinate.
 */
export function createRateLimitedRunner(intervalMs: number): <T>(task: () => Promise<T>) => Promise<T> {
  let queue: Promise<void> = Promise.resolve()
  let lastRunAt = 0

  return async function runRateLimited<T>(task: () => Promise<T>): Promise<T> {
    const run = async (): Promise<T> => {
      const elapsed = Date.now() - lastRunAt
      const waitMs = intervalMs - elapsed
      if (waitMs > 0)
        await new Promise(resolve => setTimeout(resolve, waitMs))

      lastRunAt = Date.now()
      return task()
    }

    const request = queue.then(run, run)
    queue = request.then(() => undefined, () => undefined)
    return request
  }
}

/**
 * Fetch text content from URL
 */
export async function fetchText(url: string): Promise<string | null> {
  return $fetch(url, { responseType: 'text' }).catch(() => null)
}

const RAW_GH_RE = /raw\.githubusercontent\.com\/([^/]+)\/([^/]+)/

/** Extract owner/repo from a GitHub raw content URL */
function extractGitHubRepo(url: string): { owner: string, repo: string } | null {
  const match = url.match(RAW_GH_RE)
  return match ? { owner: match[1]!, repo: match[2]! } : null
}

/**
 * Fetch text from a GitHub raw URL with auth fallback for private repos.
 * Tries unauthenticated first (fast path), falls back to authenticated
 * request when the repo is known to be private or unauthenticated fails.
 *
 * Only sends auth tokens to raw.githubusercontent.com — returns null for
 * non-GitHub URLs that fail unauthenticated to prevent token leakage.
 */
export async function fetchGitHubRaw(url: string): Promise<string | null> {
  const gh = extractGitHubRepo(url)
  const isKnownPrivate = gh ? isKnownPrivateRepo(gh.owner, gh.repo) : false

  // Fast path: skip unauthenticated attempt for known private repos
  if (!isKnownPrivate) {
    const content = await fetchText(url)
    if (content)
      return content
  }

  // Only send auth tokens to raw.githubusercontent.com
  if (!gh)
    return null

  // Fallback: authenticated request for private repos
  const token = getGitHubToken()
  if (!token)
    return null

  const content = await $fetch(url, {
    responseType: 'text',
    headers: { Authorization: `token ${token}` },
  }).catch(() => null) as string | null
  if (content)
    markRepoPrivate(gh.owner, gh.repo)
  return content
}

/**
 * Verify URL exists and is not HTML (likely 404 page)
 */
export async function verifyUrl(url: string): Promise<boolean> {
  const res = await $fetch.raw(url, { method: 'HEAD' }).catch(() => null)
  if (!res)
    return false
  const contentType = res.headers.get('content-type') || ''
  return !contentType.includes('text/html')
}
