import { describe, expect, it, vi } from 'vitest'
import { createAuthenticatedFetch } from '../../src/auth/client'

describe('createAuthenticatedFetch', () => {
  it('refreshes once and retries with the rotated token', async () => {
    const expired = {
      scheme: 'file' as const,
      login: 'harlan',
      accessToken: 'expired',
      refreshToken: 'refresh',
      host: 'https://skilld.dev',
    }
    const loadSession = vi.fn().mockResolvedValue(expired)
    const saveSession = vi.fn().mockResolvedValue('file')
    const unauthorized = Object.assign(new Error('unauthorized'), { statusCode: 401 })
    const fetch = vi.fn()
      .mockRejectedValueOnce(unauthorized)
      .mockResolvedValueOnce({
        login: 'harlan',
        accessToken: 'rotated',
        refreshToken: 'next-refresh',
        expiresAt: 1_800_000_000,
      })
      .mockResolvedValueOnce({ ok: true })

    const authenticatedFetch = createAuthenticatedFetch({
      baseUrl: 'https://skilld.dev/api',
      fetch,
      loadSession,
      saveSession,
    })

    await expect(authenticatedFetch('https://skilld.dev/api/me/collections')).resolves.toEqual({ ok: true })
    expect(fetch).toHaveBeenNthCalledWith(2, 'https://skilld.dev/api/cli/oauth/refresh', {
      method: 'POST',
      body: { refresh_token: 'refresh' },
    })
    expect(fetch).toHaveBeenNthCalledWith(3, 'https://skilld.dev/api/me/collections', {
      headers: { Authorization: 'Bearer rotated' },
    })
    expect(saveSession).toHaveBeenCalledOnce()
  })
})
