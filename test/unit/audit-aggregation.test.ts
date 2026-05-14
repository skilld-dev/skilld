import { describe, expect, it, vi } from 'vitest'

vi.mock('ofetch', () => ({
  ofetch: vi.fn(),
}))

describe('aggregateAuditStatus', () => {
  it('returns unaudited for empty audit list', async () => {
    const { aggregateAuditStatus } = await import('../../src/registry/client')
    expect(aggregateAuditStatus([])).toBe('unaudited')
  })

  it('returns pass when all entries pass', async () => {
    const { aggregateAuditStatus } = await import('../../src/registry/client')
    expect(aggregateAuditStatus([
      { provider: 'skills.sh', slug: 'static', status: 'pass' },
      { provider: 'skills.sh', slug: 'deps', status: 'pass' },
    ])).toBe('pass')
  })

  it('returns warn when any warn but no fail', async () => {
    const { aggregateAuditStatus } = await import('../../src/registry/client')
    expect(aggregateAuditStatus([
      { provider: 'skills.sh', slug: 'static', status: 'pass' },
      { provider: 'skills.sh', slug: 'deps', status: 'warn' },
    ])).toBe('warn')
  })

  it('returns fail when any fail, regardless of warns', async () => {
    const { aggregateAuditStatus } = await import('../../src/registry/client')
    expect(aggregateAuditStatus([
      { provider: 'skills.sh', slug: 'static', status: 'fail' },
      { provider: 'skills.sh', slug: 'deps', status: 'warn' },
      { provider: 'skills.sh', slug: 'license', status: 'pass' },
    ])).toBe('fail')
  })
})

describe('createRegistryClient.audit', () => {
  it('returns unaudited on network error', async () => {
    const { ofetch } = await import('ofetch')
    vi.mocked(ofetch).mockReset().mockRejectedValueOnce(new Error('network'))
    const { createRegistryClient } = await import('../../src/registry/client')
    const client = createRegistryClient()
    const res = await client.audit({ owner: 'foo', repo: 'bar', name: 'baz' })
    expect(res).toEqual({ status: 'unaudited', audits: [] })
  })

  it('aggregates server response with audits + riskLevel + summary', async () => {
    const { ofetch } = await import('ofetch')
    vi.mocked(ofetch).mockReset().mockResolvedValueOnce({
      riskLevel: 'medium',
      summary: 'large asset tree',
      audits: [
        { provider: 'skills.sh', slug: 'static', status: 'pass' },
        { provider: 'skills.sh', slug: 'deps', status: 'warn', summary: 'wildcard import' },
      ],
    })
    const { createRegistryClient } = await import('../../src/registry/client')
    const client = createRegistryClient()
    const res = await client.audit({ owner: 'foo', repo: 'bar', name: 'baz' })
    expect(res.status).toBe('warn')
    expect(res.riskLevel).toBe('medium')
    expect(res.audits).toHaveLength(2)
  })

  it('treats missing audits array as unaudited', async () => {
    const { ofetch } = await import('ofetch')
    vi.mocked(ofetch).mockReset().mockResolvedValueOnce({})
    const { createRegistryClient } = await import('../../src/registry/client')
    const client = createRegistryClient()
    const res = await client.audit({ owner: 'foo', repo: 'bar', name: 'baz' })
    expect(res.status).toBe('unaudited')
  })
})

describe('gateInstall', () => {
  it('pass → install', async () => {
    const { gateInstall } = await import('../../src/registry/client')
    expect(gateInstall({ status: 'pass', audits: [] }, { sourceKind: 'npm' })).toBe('install')
  })

  it('warn → install', async () => {
    const { gateInstall } = await import('../../src/registry/client')
    expect(gateInstall({ status: 'warn', audits: [] }, { sourceKind: 'npm' })).toBe('install')
  })

  it('fail → skip without --allow-unsafe', async () => {
    const { gateInstall } = await import('../../src/registry/client')
    expect(gateInstall({ status: 'fail', audits: [] }, { sourceKind: 'npm' })).toBe('skip')
  })

  it('fail + allowUnsafe → install', async () => {
    const { gateInstall } = await import('../../src/registry/client')
    expect(gateInstall({ status: 'fail', audits: [] }, { sourceKind: 'npm', allowUnsafe: true })).toBe('install')
  })

  it('unaudited npm → install silently', async () => {
    const { gateInstall } = await import('../../src/registry/client')
    expect(gateInstall({ status: 'unaudited', audits: [] }, { sourceKind: 'npm' })).toBe('install')
  })

  it('unaudited gh + no --yes → prompt', async () => {
    const { gateInstall } = await import('../../src/registry/client')
    expect(gateInstall({ status: 'unaudited', audits: [] }, { sourceKind: 'gh' })).toBe('prompt')
  })

  it('unaudited gh + --yes → install', async () => {
    const { gateInstall } = await import('../../src/registry/client')
    expect(gateInstall({ status: 'unaudited', audits: [] }, { sourceKind: 'gh', yes: true })).toBe('install')
  })
})

describe('createRegistryClient.my requires session', () => {
  it('throws auth required without a session', async () => {
    const { createRegistryClient } = await import('../../src/registry/client')
    const client = createRegistryClient()
    await expect(client.my.collections()).rejects.toThrow('auth required')
    await expect(client.my.subscriptions()).rejects.toThrow('auth required')
    await expect(client.my.changes({})).rejects.toThrow('auth required')
    await expect(client.my.installs({ slug: 'x', sourceKind: 'npm', surface: 'cli:add' })).rejects.toThrow('auth required')
  })

  it('passes Bearer header when session present', async () => {
    const { ofetch } = await import('ofetch')
    vi.mocked(ofetch).mockReset().mockResolvedValueOnce([])
    const { createRegistryClient } = await import('../../src/registry/client')
    const client = createRegistryClient({ session: { accessToken: 'tok', login: 'me' } })
    await client.my.collections()
    expect(ofetch).toHaveBeenCalledWith(
      expect.stringContaining('/me/collections'),
      expect.objectContaining({ headers: expect.objectContaining({ Authorization: 'Bearer tok' }) }),
    )
  })
})
