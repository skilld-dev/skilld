import { stripVTControlCharacters } from 'node:util'
import { describe, expect, it } from 'vitest'
import { formatDigestLines } from '../../src/cli/digest-render'

describe('formatDigestLines', () => {
  it('links each changed skill to its exact skill page', () => {
    const lines = formatDigestLines([{
      repo: 'nuxt/nuxt',
      skill: 'seo',
      at: '2026-08-12T00:00:00.000Z',
      summary: 'Improve canonical URL guidance',
    }], Date.parse('2026-08-13T00:00:00.000Z')).map(stripVTControlCharacters)

    expect(lines).toContain('    https://skilld.dev/gh/nuxt/nuxt/seo')
    expect(lines.join('\n')).not.toContain('/me/activity')
  })
})
