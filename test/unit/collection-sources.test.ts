import type { CollectionManifestItem } from 'skilld-protocol/wire'
import { describe, expect, it } from 'vitest'
import { expandPublicSources } from '../../src/commands/sync/install-many'
import { manifestToSources } from '../../src/registry/collections'

describe('manifestToSources', () => {
  it('collapses GitHub skills by repository and preserves package sources', () => {
    const items: CollectionManifestItem[] = [
      { kind: 'gh', owner: 'nuxt', repo: 'nuxt', name: 'seo' },
      { kind: 'gh', owner: 'nuxt', repo: 'nuxt', name: 'modules' },
      { kind: 'npm', package: '@nuxt/ui' },
      { kind: 'crate', package: 'serde' },
    ]

    expect(manifestToSources(items)).toEqual([
      {
        source: { type: 'git', source: { type: 'github', owner: 'nuxt', repo: 'nuxt' } },
        skillFilter: 'seo,modules',
      },
      { source: { type: 'npm', package: '@nuxt/ui' } },
      { source: { type: 'crate', package: 'serde' } },
    ])
  })

  it('expands the collection command from skilld.dev without authentication', async () => {
    const result = await expandPublicSources([
      { type: 'collection', handle: 'harlan', name: 'nuxt' },
    ], {
      fetchCollection: async () => ({
        name: 'Nuxt',
        items: [{ kind: 'gh', owner: 'nuxt', repo: 'nuxt', name: 'seo' }],
      }),
      fetchCurator: async () => null,
    }, true)

    expect(result).toEqual({
      items: [{
        type: 'git',
        source: { type: 'github', owner: 'nuxt', repo: 'nuxt' },
        skillFilter: 'seo',
      }],
      skipped: 0,
      failed: 0,
    })
  })
})
