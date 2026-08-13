import { beforeEach, describe, expect, it, vi } from 'vitest'

const mocks = vi.hoisted(() => ({
  loadSession: vi.fn(),
  saveSession: vi.fn(),
  runDeviceFlow: vi.fn(),
  runOidcExchange: vi.fn(),
  runPkceFlow: vi.fn(),
  success: vi.fn(),
}))

vi.mock('../../src/auth/store', () => ({
  loadSession: mocks.loadSession,
  saveSession: mocks.saveSession,
}))
vi.mock('../../src/auth/device-flow', () => ({ runDeviceFlow: mocks.runDeviceFlow }))
vi.mock('../../src/auth/oidc', () => ({
  isGhaOidcAvailable: () => false,
  runOidcExchange: mocks.runOidcExchange,
}))
vi.mock('../../src/auth/pkce-flow', () => ({ runPkceFlow: mocks.runPkceFlow }))
vi.mock('@clack/prompts', () => ({
  log: { success: mocks.success },
}))

const { loginCommandDef } = await import('../../src/commands/login')

describe('login command', () => {
  beforeEach(() => vi.clearAllMocks())

  it('does not start authentication when the existing session can refresh', async () => {
    mocks.loadSession.mockResolvedValue({
      scheme: 'file',
      login: 'harlan-zw',
      accessToken: 'expired',
      refreshToken: 'refresh',
      expiresAt: 100,
      host: 'https://skilld.dev',
    })

    await loginCommandDef.run!({ args: { device: false } } as any)

    expect(mocks.runOidcExchange).not.toHaveBeenCalled()
    expect(mocks.runDeviceFlow).not.toHaveBeenCalled()
    expect(mocks.runPkceFlow).not.toHaveBeenCalled()
    expect(mocks.success).toHaveBeenCalledWith('Already logged in as @harlan-zw')
  })
})
