/**
 * pi-ai auth — OAuth credentials, env API keys, login/logout flows.
 *
 * OAuth providers known to ban accounts for unauthorized usage are blocked.
 * API key access (env vars) remains supported for those providers.
 */

import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { getEnvApiKey, getProviders } from '@earendil-works/pi-ai'
import { getOAuthApiKey, getOAuthProvider, getOAuthProviders } from '@earendil-works/pi-ai/oauth'
import { join } from 'pathe'
import { CACHE_DIR, PI_AI_AUTH_PATH } from '../../core/paths.ts'

/**
 * Consumer-OAuth providers that ban accounts for unauthorized usage.
 * API-key access remains supported for these.
 */
export const BLOCKED_OAUTH_PROVIDERS: ReadonlySet<string> = new Set([
  'google-antigravity',
  'google-gemini-cli',
  'github-copilot',
  'anthropic',
  'openai-codex',
])

const PI_AGENT_AUTH_PATH = join(
  process.env.PI_CODING_AGENT_DIR || join(homedir(), '.pi', 'agent'),
  'auth.json',
)
const SKILLD_AUTH_PATH = PI_AI_AUTH_PATH

/**
 * Overrides for model-provider → OAuth-provider mapping. Most providers share
 * the same id in both systems (auto-matched); list only divergences.
 */
const OAUTH_PROVIDER_OVERRIDES: Record<string, string> = {
  google: 'google-gemini-cli',
  openai: 'openai-codex',
}

interface OAuthCredentials {
  type: 'oauth'
  refresh: string
  access: string
  expires: number
  [key: string]: unknown
}

function readAuthFile(path: string): Record<string, OAuthCredentials> {
  if (!existsSync(path))
    return {}
  try {
    return JSON.parse(readFileSync(path, 'utf-8'))
  }
  catch { return {} }
}

/** Load auth from pi coding agent first, then skilld's own. pi agent wins on conflict. */
export function loadAuth(): Record<string, OAuthCredentials> {
  const piAuth = readAuthFile(PI_AGENT_AUTH_PATH)
  const skilldAuth = readAuthFile(SKILLD_AUTH_PATH)
  return { ...skilldAuth, ...piAuth }
}

function saveAuth(auth: Record<string, OAuthCredentials>): void {
  mkdirSync(CACHE_DIR, { recursive: true, mode: 0o700 })
  writeFileSync(SKILLD_AUTH_PATH, JSON.stringify(auth, null, 2), { mode: 0o600 })
}

/** Resolve model provider id → OAuth provider id (returns null for blocked providers). */
export function resolveOAuthProviderId(modelProvider: string): string | null {
  const oauthId = OAUTH_PROVIDER_OVERRIDES[modelProvider] ?? modelProvider
  if (BLOCKED_OAUTH_PROVIDERS.has(oauthId))
    return null
  if (OAUTH_PROVIDER_OVERRIDES[modelProvider])
    return OAUTH_PROVIDER_OVERRIDES[modelProvider]!
  const oauthIds = new Set((getOAuthProviders() as Array<{ id: string }>).map(p => p.id))
  if (oauthIds.has(modelProvider))
    return modelProvider
  return null
}

/** Resolve API key for a provider — env vars first, then stored OAuth credentials. */
export async function resolveApiKey(provider: string): Promise<string | null> {
  const envKey = getEnvApiKey(provider)
  if (envKey)
    return envKey

  const oauthProviderId = resolveOAuthProviderId(provider)
  if (!oauthProviderId)
    return null

  const auth = loadAuth()
  if (!auth[oauthProviderId])
    return null

  const result = await getOAuthApiKey(oauthProviderId, auth)
  if (!result)
    return null

  // Refreshed credentials go to skilld's own file only, never leak pi-agent tokens.
  const skilldAuth = readAuthFile(SKILLD_AUTH_PATH)
  skilldAuth[oauthProviderId] = { type: 'oauth', ...result.newCredentials }
  saveAuth(skilldAuth)
  return result.apiKey
}

export interface LoginCallbacks {
  onAuth: (url: string, instructions?: string) => void
  onPrompt: (message: string, placeholder?: string) => Promise<string>
  onProgress?: (message: string) => void
}

export function getOAuthProviderList(): Array<{ id: string, name: string, loggedIn: boolean }> {
  const auth = loadAuth()
  const providers = getOAuthProviders() as Array<{ id: string, name: string }>
  return providers
    .filter(p => !BLOCKED_OAUTH_PROVIDERS.has(p.id))
    .map(p => ({ id: p.id, name: p.name ?? p.id, loggedIn: !!auth[p.id] }))
}

export async function loginOAuthProvider(providerId: string, callbacks: LoginCallbacks): Promise<boolean> {
  const provider = getOAuthProvider(providerId)
  if (!provider)
    return false

  const credentials = await provider.login({
    onAuth: (info: any) => callbacks.onAuth(info.url, info.instructions),
    onPrompt: async (prompt: any) => callbacks.onPrompt(prompt.message, prompt.placeholder),
    onProgress: (msg: string) => callbacks.onProgress?.(msg),
  })

  const auth = loadAuth()
  auth[providerId] = { type: 'oauth', ...credentials }
  saveAuth(auth)
  return true
}

export function logoutOAuthProvider(providerId: string): void {
  const auth = loadAuth()
  delete auth[providerId]
  saveAuth(auth)
}

/** Re-export for callers that want pi-ai's provider list without importing from pi-ai directly. */
export { getEnvApiKey, getProviders }
