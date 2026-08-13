import { describe, expect, it } from 'vitest'
import { parseRepositoryInputs } from '../../src/commands/watch'

describe('parseRepositoryInputs', () => {
  it('deduplicates GitHub repositories', () => {
    expect(parseRepositoryInputs(['nuxt/nuxt', 'nuxt/nuxt'])).toEqual({
      _tag: 'Ok',
      repositories: [{ owner: 'nuxt', repo: 'nuxt' }],
    })
  })

  it('returns invalid inputs as an error value', () => {
    expect(parseRepositoryInputs(['https://github.com/nuxt/nuxt'])).toEqual({
      _tag: 'Err',
      invalid: ['https://github.com/nuxt/nuxt'],
    })
  })
})
