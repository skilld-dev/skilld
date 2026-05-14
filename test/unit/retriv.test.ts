import { describe, expect, it } from 'vitest'
import { SearchDepsUnavailableError } from '../../src/retriv/index'

describe('searchDepsUnavailableError', () => {
  it('wraps ERR_MODULE_NOT_FOUND with descriptive message', () => {
    const cause = Object.assign(new Error('Cannot find package \'sqlite-vec\''), { code: 'ERR_MODULE_NOT_FOUND' })
    const err = new SearchDepsUnavailableError(cause)
    expect(err.name).toBe('SearchDepsUnavailableError')
    expect(err.message).toContain('sqlite-vec')
    expect(err.cause).toBe(cause)
  })

  it('is instanceof Error', () => {
    const err = new SearchDepsUnavailableError(new Error('test'))
    expect(err).toBeInstanceOf(Error)
    expect(err).toBeInstanceOf(SearchDepsUnavailableError)
  })
})
