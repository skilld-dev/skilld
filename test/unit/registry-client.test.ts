import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('ofetch', () => ({
  ofetch: vi.fn(),
}))

function mockResolveAndDetail(ofetch: ReturnType<typeof vi.fn>, opts: {
  packageName: string
  owner?: string
  repo?: string
  raw?: string
  pushedAt?: string
}) {
  const owner = opts.owner ?? 'antfu'
  const repo = opts.repo ?? 'skills'
  ofetch.mockResolvedValueOnce({
    [opts.packageName]: { owner, repo, official: false },
  })
  ofetch.mockResolvedValueOnce({
    owner,
    repo,
    name: opts.packageName,
    displayName: opts.packageName,
    stars: 1,
    branch: 'main',
    skillPath: `skills/${opts.packageName}/SKILL.md`,
    raw: opts.raw ?? '# skill',
    pushedAt: opts.pushedAt ?? '2026-01-01T00:00:00Z',
  })
}

describe('registry client', () => {
  let originalUrl: string | undefined

  beforeEach(() => {
    originalUrl = process.env.SKILLD_REGISTRY_URL
    vi.resetAllMocks()
  })

  afterEach(() => {
    if (originalUrl === undefined)
      delete process.env.SKILLD_REGISTRY_URL
    else
      process.env.SKILLD_REGISTRY_URL = originalUrl
  })

  it('fetchRegistrySkill uses default base when env unset', async () => {
    delete process.env.SKILLD_REGISTRY_URL
    const { ofetch } = await import('ofetch')
    mockResolveAndDetail(vi.mocked(ofetch), { packageName: 'vue' })

    const { fetchRegistrySkill } = await import('../../src/registry/client')
    const skill = await fetchRegistrySkill('vue')

    expect(skill).not.toBeNull()
    expect(ofetch).toHaveBeenNthCalledWith(1, 'https://skilld.dev/api/skills/resolve', {
      method: 'POST',
      body: { items: [{ packageName: 'vue', owner: undefined }] },
    })
    expect(ofetch).toHaveBeenNthCalledWith(2, 'https://skilld.dev/api/skills/antfu/skills/vue')
  })

  it('fetchRegistrySkill respects SKILLD_REGISTRY_URL override', async () => {
    process.env.SKILLD_REGISTRY_URL = 'http://localhost:3000/api'
    const { ofetch } = await import('ofetch')
    mockResolveAndDetail(vi.mocked(ofetch), { packageName: 'vue' })

    const { fetchRegistrySkill } = await import('../../src/registry/client')
    await fetchRegistrySkill('vue')

    expect(ofetch).toHaveBeenNthCalledWith(1, 'http://localhost:3000/api/skills/resolve', expect.any(Object))
    expect(ofetch).toHaveBeenNthCalledWith(2, 'http://localhost:3000/api/skills/antfu/skills/vue')
  })

  it('strips trailing slash from override', async () => {
    process.env.SKILLD_REGISTRY_URL = 'http://localhost:3000/api/'
    const { ofetch } = await import('ofetch')
    mockResolveAndDetail(vi.mocked(ofetch), { packageName: 'vue' })

    const { fetchRegistrySkill } = await import('../../src/registry/client')
    await fetchRegistrySkill('vue')

    expect(ofetch).toHaveBeenNthCalledWith(1, 'http://localhost:3000/api/skills/resolve', expect.any(Object))
  })

  it('propagates resolve transport failures', async () => {
    const { ofetch } = await import('ofetch')
    vi.mocked(ofetch).mockRejectedValueOnce(new Error('network'))

    const { fetchRegistrySkill } = await import('../../src/registry/client')
    await expect(fetchRegistrySkill('nonexistent')).rejects.toThrow('network')
  })

  it('returns null when resolve has no hit', async () => {
    const { ofetch } = await import('ofetch')
    vi.mocked(ofetch).mockResolvedValueOnce({})

    const { fetchRegistrySkill } = await import('../../src/registry/client')
    expect(await fetchRegistrySkill('nonexistent')).toBeNull()
    expect(ofetch).toHaveBeenCalledTimes(1)
  })

  it('returns null when detail has no raw SKILL.md', async () => {
    const { ofetch } = await import('ofetch')
    vi.mocked(ofetch)
      .mockResolvedValueOnce({ vue: { owner: 'antfu', repo: 'skills', official: false } })
      .mockResolvedValueOnce({
        owner: 'antfu',
        repo: 'skills',
        name: 'vue',
        displayName: 'Vue',
        stars: 1,
        branch: 'main',
        skillPath: 'skills/vue/SKILL.md',
        raw: null,
        pushedAt: null,
      })

    const { fetchRegistrySkill } = await import('../../src/registry/client')
    expect(await fetchRegistrySkill('vue')).toBeNull()
  })

  it('passes scoped package names unencoded to resolve body', async () => {
    delete process.env.SKILLD_REGISTRY_URL
    const { ofetch } = await import('ofetch')
    mockResolveAndDetail(vi.mocked(ofetch), { packageName: '@nuxt/ui', owner: 'nuxt', repo: 'ui' })

    const { fetchRegistrySkill } = await import('../../src/registry/client')
    await fetchRegistrySkill('@nuxt/ui')

    expect(ofetch).toHaveBeenNthCalledWith(1, 'https://skilld.dev/api/skills/resolve', {
      method: 'POST',
      body: { items: [{ packageName: '@nuxt/ui', owner: undefined }] },
    })
    expect(ofetch).toHaveBeenNthCalledWith(2, 'https://skilld.dev/api/skills/nuxt/ui/@nuxt/ui')
  })

  it('maps detail payload into RegistrySkill', async () => {
    const { ofetch } = await import('ofetch')
    mockResolveAndDetail(vi.mocked(ofetch), {
      packageName: 'vue',
      owner: 'antfu',
      repo: 'skills',
      raw: '---\nname: vue\n---\n# vue',
      pushedAt: '2026-03-16T06:16:24Z',
    })

    const { fetchRegistrySkill } = await import('../../src/registry/client')
    const skill = await fetchRegistrySkill('vue')

    expect(skill).toMatchObject({
      name: 'vue',
      packageName: 'vue',
      owner: 'antfu',
      repo: 'antfu/skills',
      content: '---\nname: vue\n---\n# vue',
      updatedAt: '2026-03-16T06:16:24Z',
      branch: 'main',
    })
  })
})
