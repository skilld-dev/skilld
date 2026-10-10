import { expect, it, vi } from 'vitest'
import { waitForNpmPackages } from '../../scripts/release/npm-availability'

const packages = [{ name: 'skilld', version: '3.6.8', tag: 'latest' }, { name: 'skilld-native', version: '3.6.8', tag: 'latest' }]

it.each(['version', 'tag'])('waits for the last package %s after npm accepts the uploads', async (pending) => {
  let now = 0
  let ready = false
  const sleep = vi.fn(async (ms: number) => {
    now += ms
    ready = true
  })
  const fetcher = vi.fn(async (input: string | URL | Request) => {
    const url = String(input)
    if (!ready && url.includes('skilld-native') && pending === 'version')
      return new Response(null, { status: 404 })
    return Response.json({ name: url.includes('skilld-native') ? 'skilld-native' : 'skilld', version: !ready && url.includes('skilld-native/latest') ? '3.6.7' : '3.6.8' })
  })
  await waitForNpmPackages({ packages, fetch: fetcher, now: () => now, sleep, timeoutMs: 100, intervalMs: 10 })
  expect(sleep).toHaveBeenCalledOnce()
  expect(fetcher.mock.calls.some(([url]) => String(url).endsWith('skilld-native/latest'))).toBe(true)
})

it('fails with the pending package when npm does not make it available before the deadline', async () => {
  let now = 0
  await expect(waitForNpmPackages({
    packages,
    fetch: async () => new Response(null, { status: 404 }),
    now: () => now,
    sleep: async (ms) => {
      now += ms
    },
    timeoutMs: 20,
    intervalMs: 10,
  })).rejects.toThrow(/skilld@3.6.8/)
  expect(now).toBe(20)
})

it('retries transient registry failures without hiding the reason', async () => {
  let now = 0
  const progress = vi.fn()
  const fetcher = vi.fn(async () => now === 0 ? new Response(null, { status: 503 }) : Response.json({ name: 'skilld', version: '3.6.8' }))
  await waitForNpmPackages({
    packages: [packages[0]!],
    fetch: fetcher,
    now: () => now,
    sleep: async (ms) => {
      now += ms
    },
    onProgress: progress,
    timeoutMs: 20,
    intervalMs: 10,
  })
  expect(progress).toHaveBeenCalledWith(expect.stringContaining('HTTP 503'))
})

it('rejects metadata for a different package instead of reporting readiness', async () => {
  await expect(waitForNpmPackages({ packages, fetch: async () => Response.json({ name: 'other', version: '3.6.8' }) })).rejects.toThrow(/different package/)
})

it('rejects availability that arrives after the deadline', async () => {
  let now = 0
  await expect(waitForNpmPackages({
    packages: [packages[0]!],
    now: () => now,
    timeoutMs: 20,
    fetch: async () => {
      now = 21
      return Response.json({ name: 'skilld', version: '3.6.8' })
    },
  })).rejects.toThrow(/timed out/)
})
