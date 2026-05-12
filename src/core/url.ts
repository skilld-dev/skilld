/**
 * Pure URL and package-spec parsers (no fetching, no I/O).
 *
 * Moved from `src/sources/utils.ts` to keep the sources barrel focused on
 * fetching primitives and resolvers.
 */

/**
 * Parse owner/repo from GitHub URL
 */
export function parseGitHubUrl(url: string): { owner: string, repo: string } | null {
  const match = url.match(/github\.com\/([^/]+)\/([^/]+?)(?:\.git)?(?:[/#]|$)/)
  if (!match)
    return null
  return { owner: match[1]!, repo: match[2]! }
}

/** Parse owner/repo slug from GitHub URL */
export function parseGitHubRepoSlug(url: string | undefined): string | undefined {
  if (!url)
    return undefined
  const parsed = parseGitHubUrl(url)
  return parsed ? `${parsed.owner}/${parsed.repo}` : undefined
}

/**
 * Parse package spec with optional dist-tag or version: "vue@beta" → { name: "vue", tag: "beta" }
 * Handles scoped packages: "@vue/reactivity@beta" → { name: "@vue/reactivity", tag: "beta" }
 */
export function parsePackageSpec(spec: string): { name: string, tag?: string } {
  // Scoped: @scope/pkg@tag — find the second @
  if (spec.startsWith('@')) {
    const slashIdx = spec.indexOf('/')
    if (slashIdx !== -1) {
      const atIdx = spec.indexOf('@', slashIdx + 1)
      if (atIdx !== -1)
        return { name: spec.slice(0, atIdx), tag: spec.slice(atIdx + 1) }
    }
    return { name: spec }
  }
  // Unscoped: pkg@tag
  const atIdx = spec.indexOf('@')
  if (atIdx !== -1)
    return { name: spec.slice(0, atIdx), tag: spec.slice(atIdx + 1) }
  return { name: spec }
}

/**
 * Normalize git repo URL to https
 */
export function normalizeRepoUrl(url: string): string {
  return url
    .replace(/^git\+/, '')
    .replace(/#.*$/, '')
    .replace(/\.git$/, '')
    .replace(/^git:\/\//, 'https://')
    .replace(/^ssh:\/\/git@github\.com/, 'https://github.com')
    // SSH format: git@github.com:owner/repo
    .replace(/^git@github\.com:/, 'https://github.com/')
}

/**
 * Extract branch hint from URL fragment (e.g. "git+https://...#main" → "main")
 */
export function extractBranchHint(url: string): string | undefined {
  const hash = url.indexOf('#')
  if (hash === -1)
    return undefined
  const fragment = url.slice(hash + 1)
  // Ignore non-branch fragments like "readme"
  if (!fragment || fragment === 'readme')
    return undefined
  return fragment
}

/**
 * Check if URL is a GitHub repo URL (not a docs site)
 */
export function isGitHubRepoUrl(url: string): boolean {
  try {
    const parsed = new URL(url)
    return parsed.hostname === 'github.com' || parsed.hostname === 'www.github.com'
  }
  catch {
    return false
  }
}

/** Check if URL points to a code hosting provider (GitHub/GitLab) rather than a docs site */
export function isLikelyCodeHostUrl(url: string | undefined): boolean {
  if (!url)
    return false
  try {
    const parsed = new URL(url)
    return ['github.com', 'www.github.com', 'gitlab.com', 'www.gitlab.com'].includes(parsed.hostname)
  }
  catch {
    return false
  }
}

/**
 * Check if URL points to a social media or package registry site (not real docs)
 */
const USELESS_HOSTS = new Set([
  'twitter.com',
  'x.com',
  'facebook.com',
  'linkedin.com',
  'youtube.com',
  'instagram.com',
  'npmjs.com',
  'www.npmjs.com',
  'yarnpkg.com',
])

export function isUselessDocsUrl(url: string): boolean {
  try {
    const { hostname } = new URL(url)
    return USELESS_HOSTS.has(hostname)
  }
  catch { return false }
}

/** Reject non-https URLs and private/link-local IPs */
export function isSafeUrl(url: string): boolean {
  try {
    const parsed = new URL(url)
    if (parsed.protocol !== 'https:')
      return false
    const host = parsed.hostname
    // Reject private/link-local/loopback
    if (host === 'localhost' || host === '0.0.0.0' || host === '[::1]')
      return false
    if (/^(?:127\.|10\.|172\.(?:1[6-9]|2\d|3[01])\.|192\.168\.|169\.254\.)/.test(host))
      return false
    // IPv6 private/link-local — hostname keeps brackets in Node.js
    if (/^\[(?:f[cd]|fe[89ab]|::ffff:)/i.test(host))
      return false
    return true
  }
  catch { return false }
}
