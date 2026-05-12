import { describe, expect, it, vi } from 'vitest'
import { isSafeUrl } from '../../src/core/url'
import { extractSections, normalizeLlmsLinks, parseMarkdownLinks } from '../../src/sources/llms'

vi.mock('../../src/sources/utils', () => ({
  fetchText: vi.fn(),
  verifyUrl: vi.fn(),
}))

describe('sources/llms', () => {
  describe('parseMarkdownLinks', () => {
    it('extracts markdown links to .md files', () => {
      const content = `
# Docs
- [Getting Started](/docs/getting-started.md)
- [API Reference](/docs/api.md)
- [Some HTML](/docs/page.html)
      `
      const links = parseMarkdownLinks(content)
      expect(links).toEqual([
        { title: 'Getting Started', url: '/docs/getting-started.md' },
        { title: 'API Reference', url: '/docs/api.md' },
      ])
    })

    it('deduplicates links', () => {
      const content = `
[Same](/docs/a.md)
[Same Again](/docs/a.md)
[Different](/docs/b.md)
      `
      const links = parseMarkdownLinks(content)
      expect(links).toHaveLength(2)
    })

    it('handles absolute URLs', () => {
      const content = '[Ext](https://example.com/docs/guide.md)'
      const links = parseMarkdownLinks(content)
      expect(links[0]?.url).toBe('https://example.com/docs/guide.md')
    })
  })

  describe('normalizeLlmsLinks', () => {
    it('converts absolute paths to relative', () => {
      const content = 'See [Guide](/getting-started.md) and [API](/api.md)'
      expect(normalizeLlmsLinks(content)).toBe(
        'See [Guide](./docs/getting-started.md) and [API](./docs/api.md)',
      )
    })

    it('preserves non-absolute paths', () => {
      const content = 'See [Guide](./getting-started.md)'
      expect(normalizeLlmsLinks(content)).toBe('See [Guide](./getting-started.md)')
    })
  })

  describe('isSafeUrl', () => {
    it('allows public HTTPS URLs', () => {
      expect(isSafeUrl('https://vuejs.org/docs/guide.md')).toBe(true)
      expect(isSafeUrl('https://example.com/api.md')).toBe(true)
    })

    it('blocks HTTP URLs', () => {
      expect(isSafeUrl('http://example.com/docs.md')).toBe(false)
    })

    it('blocks localhost and loopback', () => {
      expect(isSafeUrl('https://localhost/secret')).toBe(false)
      expect(isSafeUrl('https://127.0.0.1/secret')).toBe(false)
      expect(isSafeUrl('https://[::1]/secret')).toBe(false)
    })

    it('blocks 0.0.0.0', () => {
      expect(isSafeUrl('https://0.0.0.0/secret')).toBe(false)
    })

    it('blocks full 127.0.0.0/8 loopback range', () => {
      expect(isSafeUrl('https://127.0.0.1/secret')).toBe(false)
      expect(isSafeUrl('https://127.0.1.1/secret')).toBe(false)
      expect(isSafeUrl('https://127.255.255.255/secret')).toBe(false)
    })

    it('blocks 169.254.0.0/16 link-local range', () => {
      expect(isSafeUrl('https://169.254.169.254/latest/meta-data/')).toBe(false)
      expect(isSafeUrl('https://169.254.1.1/internal')).toBe(false)
    })

    it('blocks RFC 1918 private IPs', () => {
      expect(isSafeUrl('https://10.0.0.1/internal')).toBe(false)
      expect(isSafeUrl('https://172.16.0.1/internal')).toBe(false)
      expect(isSafeUrl('https://172.31.255.1/internal')).toBe(false)
      expect(isSafeUrl('https://192.168.1.1/internal')).toBe(false)
    })

    it('blocks IPv6 unique-local (fc00::/7)', () => {
      expect(isSafeUrl('https://[fc00::1]/internal')).toBe(false)
      expect(isSafeUrl('https://[fd12:3456::1]/internal')).toBe(false)
    })

    it('blocks IPv6 link-local (fe80::/10)', () => {
      expect(isSafeUrl('https://[fe80::1]/internal')).toBe(false)
      expect(isSafeUrl('https://[feb0::1]/internal')).toBe(false)
    })

    it('blocks IPv4-mapped IPv6 addresses', () => {
      expect(isSafeUrl('https://[::ffff:127.0.0.1]/secret')).toBe(false)
      expect(isSafeUrl('https://[::ffff:10.0.0.1]/internal')).toBe(false)
    })

    it('rejects invalid URLs', () => {
      expect(isSafeUrl('not-a-url')).toBe(false)
    })
  })

  describe('downloadLlmsDocs', () => {
    it('reports progress only after each fetch completes', async () => {
      const { fetchText } = await import('../../src/sources/utils')
      const { downloadLlmsDocs } = await import('../../src/sources/llms')

      let resolveA!: (v: string) => void
      let resolveB!: (v: string) => void
      let resolveC!: (v: string) => void
      const pA = new Promise<string>((r) => {
        resolveA = r
      })
      const pB = new Promise<string>((r) => {
        resolveB = r
      })
      const pC = new Promise<string>((r) => {
        resolveC = r
      })

      vi.mocked(fetchText)
        .mockReturnValueOnce(pA)
        .mockReturnValueOnce(pB)
        .mockReturnValueOnce(pC)

      const progressCalls: Array<{ index: number, total: number }> = []

      const run = downloadLlmsDocs(
        {
          raw: '',
          links: [
            { title: 'A', url: '/a.md' },
            { title: 'B', url: '/b.md' },
            { title: 'C', url: '/c.md' },
          ],
        },
        'https://example.com',
        (_url, index, total) => {
          progressCalls.push({ index, total })
        },
      )

      // No progress before any fetch resolves
      await Promise.resolve()
      expect(progressCalls).toHaveLength(0)

      resolveA('a'.repeat(200))
      await Promise.resolve()
      expect(progressCalls.at(-1)!.index).toBe(1)

      resolveB('b'.repeat(200))
      resolveC('c'.repeat(200))
      await run

      expect(progressCalls).toHaveLength(3)
      for (const call of progressCalls) {
        expect(call.total).toBe(3)
      }
      expect(progressCalls.at(-1)!.index).toBe(3)
    })
  })

  describe('extractSections', () => {
    it('extracts sections matching patterns', () => {
      // Format: parts split by \n---\n, each part has url: line followed by content
      const content = [
        'url: /docs/intro.md',
        'Intro content here',
        '',
        '---',
        'url: /api/config.md',
        'Config content',
        '',
        '---',
        'url: /docs/guide.md',
        'Guide content',
      ].join('\n')
      const result = extractSections(content, ['/docs/'])
      expect(result).toContain('Intro content')
      expect(result).toContain('Guide content')
      expect(result).not.toContain('Config content')
    })

    it('returns null when no matches', () => {
      const content = '---\nurl: /api/thing.md\n---\nContent'
      expect(extractSections(content, ['/docs/'])).toBeNull()
    })
  })
})
