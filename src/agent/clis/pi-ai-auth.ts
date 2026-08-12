/**
 * pi-ai auth — OAuth credentials, env API keys, login/logout flows.
 *
 * OAuth providers known to ban accounts for unauthorized usage are blocked.
 * API key access (env vars) remains supported for those providers.
 */

import type { AuthEvent, AuthInteraction, AuthPrompt, Credential, CredentialStore, Models } from '@earendil-works/pi-ai'
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { builtinModels, builtinProviders } from '@earendil-works/pi-ai/providers/all'
import { join } from 'pathe'
import { CACHE_DIR, PI_AI_AUTH_PATH } from '../../core/paths.ts'

/**
 * Consumer-OAuth providers that ban accounts for unauthorized usage.
 * API-key access remains supported for these.
 */
export const BLOCKED_OAUTH_PROVIDERS: ReadonlySet<string> = new Set([
  'github-copilot',
  'anthropic',
  'openai-codex',
])

const PI_AGENT_AUTH_PATH = join(
  process.env.PI_CODING_AGENT_DIR || join(homedir(), '.pi', 'agent'),
  'auth.json',
)
const SKILLD_AUTH_PATH = PI_AI_AUTH_PATH

function isObjectRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object'
    && value !== null
    && !Array.isArray(value)
}

function isStringRecord(value: unknown): value is Record<string, string> {
  return isObjectRecord(value)
    && Object.values(value).every(entry => typeof entry === 'string')
}

function isCredential(value: unknown): value is Credential {
  if (!isObjectRecord(value) || !('type' in value))
    return false
  if (value.type === 'oauth') {
    return 'refresh' in value
      && typeof value.refresh === 'string'
      && 'access' in value
      && typeof value.access === 'string'
      && 'expires' in value
      && typeof value.expires === 'number'
  }
  if (value.type !== 'api_key')
    return false
  return (!('key' in value) || value.key === undefined || typeof value.key === 'string')
    && (!('env' in value) || value.env === undefined || isStringRecord(value.env))
}

function readAuthFile(path: string): Record<string, Credential> {
  if (!existsSync(path))
    return {}
  try {
    const parsed: unknown = JSON.parse(readFileSync(path, 'utf-8'))
    if (!isObjectRecord(parsed))
      return {}
    return Object.fromEntries(Object.entries(parsed).filter((entry): entry is [string, Credential] => isCredential(entry[1])))
  }
  catch {
    // A malformed optional auth file must not prevent API-key providers from loading.
    return {}
  }
}

/** Load shared pi credentials, then apply skilld's locally refreshed values. */
export function loadAuth(): Record<string, Credential> {
  const piAuth = readAuthFile(PI_AGENT_AUTH_PATH)
  const skilldAuth = readAuthFile(SKILLD_AUTH_PATH)
  return { ...piAuth, ...skilldAuth }
}

function saveAuth(auth: Record<string, Credential>): void {
  mkdirSync(CACHE_DIR, { recursive: true, mode: 0o700 })
  writeFileSync(SKILLD_AUTH_PATH, JSON.stringify(auth, null, 2), { mode: 0o600 })
}

function createCredentialStore(): CredentialStore {
  const chains = new Map<string, Promise<void>>()

  return {
    async read(providerId) {
      return loadAuth()[providerId]
    },
    async list() {
      return Object.entries(loadAuth()).map(([providerId, credential]) => ({ providerId, type: credential.type }))
    },
    async modify(providerId, update) {
      let result: Credential | undefined
      const previous = chains.get(providerId) ?? Promise.resolve()
      const operation = previous.then(async () => {
        const current = loadAuth()[providerId]
        const next = await update(current)
        result = next ?? current
        if (next) {
          const localAuth = readAuthFile(SKILLD_AUTH_PATH)
          localAuth[providerId] = next
          saveAuth(localAuth)
        }
      })
      // Keep later updates runnable; this operation still rejects to its caller below.
      chains.set(providerId, operation.then(() => undefined, () => undefined))
      await operation
      return result
    },
    async delete(providerId) {
      const localAuth = readAuthFile(SKILLD_AUTH_PATH)
      delete localAuth[providerId]
      saveAuth(localAuth)
    },
  }
}

export function createPiAiModels(): Models {
  return builtinModels({ credentials: createCredentialStore() })
}

export interface LoginCallbacks {
  onAuth: (url: string, instructions?: string) => void
  onPrompt: (message: string, placeholder?: string) => Promise<string>
  onProgress?: (message: string) => void
  onDeviceCode?: (userCode: string, verificationUri: string) => void
  onSelect?: (message: string, options: Array<{ id: string, label: string }>) => Promise<string | undefined>
}

export function getOAuthProviderList(): Array<{ id: string, name: string, loggedIn: boolean }> {
  const auth = loadAuth()
  return builtinProviders()
    .filter(provider => provider.auth.oauth && !BLOCKED_OAUTH_PROVIDERS.has(provider.id))
    .map(provider => ({
      id: provider.id,
      name: provider.auth.oauth?.name ?? provider.name,
      loggedIn: auth[provider.id]?.type === 'oauth',
    }))
}

export async function loginOAuthProvider(providerId: string, callbacks: LoginCallbacks): Promise<boolean> {
  const models = createPiAiModels()
  const provider = models.getProvider(providerId)
  if (!provider?.auth.oauth || BLOCKED_OAUTH_PROVIDERS.has(providerId))
    return false

  const notify = (event: AuthEvent): void => {
    if (event.type === 'auth_url') {
      callbacks.onAuth(event.url, event.instructions)
    }
    else if (event.type === 'device_code') {
      if (callbacks.onDeviceCode) {
        callbacks.onDeviceCode(event.userCode, event.verificationUri)
      }
      else {
        callbacks.onAuth(event.verificationUri, `Enter code ${event.userCode}`)
      }
    }
    else {
      callbacks.onProgress?.(event.message)
    }
  }
  const prompt = async (input: AuthPrompt): Promise<string> => {
    if (input.type !== 'select')
      return callbacks.onPrompt(input.message, input.placeholder)
    const options = input.options.map(option => ({ id: option.id, label: option.label }))
    const selected = await callbacks.onSelect?.(input.message, options)
    return selected ?? options[0]?.id ?? ''
  }
  const interaction: AuthInteraction = { notify, prompt }
  await models.login(providerId, 'oauth', interaction)
  return true
}

export async function logoutOAuthProvider(providerId: string): Promise<void> {
  await createPiAiModels().logout(providerId)
}
