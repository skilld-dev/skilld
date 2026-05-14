/**
 * Auth credential store. Prefers OS keychain via the optional `@napi-rs/keyring`
 * dependency; falls back to a 0600 JSON file at `~/.skilld/auth.json`.
 *
 * `SKILLD_TOKEN` env var trumps everything: hard expiry, no refresh, no marker
 * file. Use it in CI when keychain access isn't available.
 */

import type { AuthSession } from '../registry/client.ts'
import { chmodSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { dirname } from 'pathe'
import { AUTH_PATH, CACHE_DIR } from '../core/paths.ts'

const KEYRING_SERVICE = 'skilld'
const DEFAULT_HOST = 'https://skilld.dev'

export type StorageScheme = 'keychain' | 'file' | 'env'

export interface StoredTokens {
  accessToken: string
  refreshToken?: string
}

export interface AuthMarker {
  scheme: StorageScheme
  login: string
  expiresAt?: number
  host: string
  /** Present only when scheme = 'file' */
  tokens?: StoredTokens
  /** ISO timestamp of the last in-terminal digest the user saw. */
  /** Unix seconds, matches the server's `windowEnd` in the digest envelope. */
  lastDigestAt?: number
}

export interface StoredSession extends AuthSession {
  scheme: StorageScheme
}

interface KeyringEntry {
  new(service: string, account: string): {
    getPassword: () => string | null
    setPassword: (value: string) => void
    deletePassword: () => boolean
  }
}

let keyringPromise: Promise<KeyringEntry | null> | null = null

async function loadKeyring(): Promise<KeyringEntry | null> {
  if (!keyringPromise) {
    keyringPromise = (import('@napi-rs/keyring' as any) as Promise<{ Entry: KeyringEntry }>)
      .then(m => m.Entry)
      .catch(() => null)
  }
  return keyringPromise
}

function readMarker(): AuthMarker | null {
  if (!existsSync(AUTH_PATH))
    return null
  const raw = readFileSync(AUTH_PATH, 'utf8').trim()
  if (!raw)
    return null
  return JSON.parse(raw) as AuthMarker
}

function writeMarker(marker: AuthMarker): void {
  if (!existsSync(CACHE_DIR))
    mkdirSync(dirname(AUTH_PATH), { recursive: true })
  writeFileSync(AUTH_PATH, `${JSON.stringify(marker, null, 2)}\n`, { mode: 0o600 })
  chmodSync(AUTH_PATH, 0o600)
}

export async function saveSession(session: AuthSession & { tokens: StoredTokens }): Promise<StorageScheme> {
  const Keyring = await loadKeyring()
  const host = session.host ?? DEFAULT_HOST

  if (Keyring) {
    new Keyring(KEYRING_SERVICE, `access:${session.login}`).setPassword(session.tokens.accessToken)
    if (session.tokens.refreshToken)
      new Keyring(KEYRING_SERVICE, `refresh:${session.login}`).setPassword(session.tokens.refreshToken)

    writeMarker({
      scheme: 'keychain',
      login: session.login,
      expiresAt: session.expiresAt,
      host,
    })
    return 'keychain'
  }

  writeMarker({
    scheme: 'file',
    login: session.login,
    expiresAt: session.expiresAt,
    host,
    tokens: session.tokens,
  })
  return 'file'
}

export async function loadSession(): Promise<StoredSession | null> {
  const envToken = process.env.SKILLD_TOKEN
  if (envToken) {
    return {
      scheme: 'env',
      accessToken: envToken,
      login: process.env.SKILLD_LOGIN ?? 'ci',
      host: DEFAULT_HOST,
    }
  }

  const marker = readMarker()
  if (!marker)
    return null

  if (marker.scheme === 'file') {
    if (!marker.tokens?.accessToken)
      return null
    return {
      scheme: 'file',
      accessToken: marker.tokens.accessToken,
      refreshToken: marker.tokens.refreshToken,
      login: marker.login,
      expiresAt: marker.expiresAt,
      host: marker.host,
    }
  }

  const Keyring = await loadKeyring()
  if (!Keyring)
    return null

  const accessToken = new Keyring(KEYRING_SERVICE, `access:${marker.login}`).getPassword()
  if (!accessToken)
    return null
  const refreshToken = new Keyring(KEYRING_SERVICE, `refresh:${marker.login}`).getPassword() ?? undefined

  return {
    scheme: 'keychain',
    accessToken,
    refreshToken,
    login: marker.login,
    expiresAt: marker.expiresAt,
    host: marker.host,
  }
}

export async function clearSession(): Promise<void> {
  const marker = readMarker()
  if (marker) {
    const Keyring = await loadKeyring()
    if (Keyring && marker.scheme === 'keychain') {
      new Keyring(KEYRING_SERVICE, `access:${marker.login}`).deletePassword()
      new Keyring(KEYRING_SERVICE, `refresh:${marker.login}`).deletePassword()
    }
    rmSync(AUTH_PATH, { force: true })
  }
}

export function updateMarker(patch: Partial<AuthMarker>): void {
  const current = readMarker()
  if (!current)
    return
  writeMarker({ ...current, ...patch })
}

export function peekMarker(): AuthMarker | null {
  try {
    return readMarker()
  }
  catch {
    return null
  }
}
