import { chmodSync, existsSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('node:fs', async () => {
  const actual = await vi.importActual<typeof import('node:fs')>('node:fs')
  return {
    ...actual,
    existsSync: vi.fn(),
    readFileSync: vi.fn(),
    writeFileSync: vi.fn(),
    chmodSync: vi.fn(),
    rmSync: vi.fn(),
    mkdirSync: vi.fn(),
  }
})

interface FakePasswordEntry {
  service: string
  account: string
}

const passwords = new Map<string, string>()

function fakeKey(svc: string, acct: string): string {
  return `${svc}:${acct}`
}

class FakeEntry {
  constructor(public service: string, public account: string) {}
  getPassword(): string | null {
    return passwords.get(fakeKey(this.service, this.account)) ?? null
  }

  setPassword(value: string): void {
    passwords.set(fakeKey(this.service, this.account), value)
  }

  deletePassword(): boolean {
    return passwords.delete(fakeKey(this.service, this.account))
  }
}

void FakeEntry as unknown as { new(...args: any[]): FakePasswordEntry }

describe('auth store', () => {
  beforeEach(() => {
    vi.resetModules()
    vi.resetAllMocks()
    passwords.clear()
    delete process.env.SKILLD_TOKEN
    delete process.env.SKILLD_LOGIN
  })

  it('sKILLD_TOKEN env var trumps the marker', async () => {
    process.env.SKILLD_TOKEN = 'ci-token'
    process.env.SKILLD_LOGIN = 'ci-user'
    const { loadSession } = await import('../../src/auth/store')
    const session = await loadSession()
    expect(session).toMatchObject({ scheme: 'env', accessToken: 'ci-token', login: 'ci-user' })
    expect(existsSync).not.toHaveBeenCalled()
  })

  it('returns null when no marker exists and no env var', async () => {
    vi.mocked(existsSync).mockReturnValue(false)
    const { loadSession } = await import('../../src/auth/store')
    expect(await loadSession()).toBeNull()
  })

  it('falls back to file scheme when keychain is missing', async () => {
    vi.doMock('@napi-rs/keyring', () => {
      throw new Error('not installed')
    })
    vi.mocked(existsSync).mockReturnValue(true)
    vi.mocked(readFileSync).mockImplementation(() => '')

    const { saveSession, loadSession } = await import('../../src/auth/store')
    let written = ''
    vi.mocked(writeFileSync).mockImplementation((_p, data) => {
      written = String(data)
    })

    const scheme = await saveSession({
      login: 'harlan',
      accessToken: 'a',
      tokens: { accessToken: 'a', refreshToken: 'r' },
      expiresAt: 123,
    })
    expect(scheme).toBe('file')
    expect(written).toContain('"scheme": "file"')
    expect(written).toContain('"refreshToken": "r"')

    vi.mocked(readFileSync).mockReturnValue(written)
    const loaded = await loadSession()
    expect(loaded).toMatchObject({ scheme: 'file', accessToken: 'a', refreshToken: 'r', login: 'harlan' })
  })

  it('stores tokens in keychain when available, marker omits tokens', async () => {
    vi.doMock('@napi-rs/keyring', () => ({ Entry: FakeEntry }))
    vi.mocked(existsSync).mockReturnValue(true)
    let written = ''
    vi.mocked(writeFileSync).mockImplementation((_p, data) => {
      written = String(data)
    })

    const { saveSession, loadSession } = await import('../../src/auth/store')
    const scheme = await saveSession({
      login: 'harlan',
      accessToken: 'a',
      tokens: { accessToken: 'a', refreshToken: 'r' },
      expiresAt: 123,
    })
    expect(scheme).toBe('keychain')
    expect(written).toContain('"scheme": "keychain"')
    expect(written).not.toContain('"tokens"')
    expect(chmodSync).toHaveBeenCalled()

    vi.mocked(readFileSync).mockReturnValue(written)
    const loaded = await loadSession()
    expect(loaded).toMatchObject({ scheme: 'keychain', accessToken: 'a', refreshToken: 'r', login: 'harlan' })
  })

  it('throws on a corrupt marker file', async () => {
    vi.mocked(existsSync).mockReturnValue(true)
    vi.mocked(readFileSync).mockReturnValue('not-json{')
    const { loadSession } = await import('../../src/auth/store')
    await expect(loadSession()).rejects.toThrow()
  })

  it('clearSession removes the marker and keychain entries', async () => {
    vi.doMock('@napi-rs/keyring', () => ({ Entry: FakeEntry }))
    passwords.set('skilld:access:harlan', 'a')
    passwords.set('skilld:refresh:harlan', 'r')

    vi.mocked(existsSync).mockReturnValue(true)
    vi.mocked(readFileSync).mockReturnValue(JSON.stringify({
      scheme: 'keychain',
      login: 'harlan',
      host: 'https://skilld.dev',
    }))

    const { clearSession } = await import('../../src/auth/store')
    await clearSession()

    expect(passwords.has('skilld:access:harlan')).toBe(false)
    expect(passwords.has('skilld:refresh:harlan')).toBe(false)
    expect(rmSync).toHaveBeenCalled()
  })
})
