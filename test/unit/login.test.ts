import type { StoredSession } from '../../src/auth/store'
import { describe, expect, it } from 'vitest'
import { resolveLoginRequirement } from '../../src/commands/login'

const session: StoredSession = {
  scheme: 'file',
  login: 'harlan-zw',
  accessToken: 'access',
  refreshToken: 'refresh',
  expiresAt: 100,
  host: 'https://skilld.dev',
}

describe('resolveLoginRequirement', () => {
  it('reuses a refreshable session after its access token expires', () => {
    expect(resolveLoginRequirement(session, 200)).toEqual({ _tag: 'Reuse', session })
  })

  it('reuses an unexpired session without a refresh token', () => {
    const current = { ...session, refreshToken: undefined, expiresAt: 300 }
    expect(resolveLoginRequirement(current, 200)).toEqual({ _tag: 'Reuse', session: current })
  })

  it('authenticates again when a nonrefreshable session expires', () => {
    const expired = { ...session, refreshToken: undefined }
    expect(resolveLoginRequirement(expired, 200)).toEqual({ _tag: 'Authenticate' })
  })
})
