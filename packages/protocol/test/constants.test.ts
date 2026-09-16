import { describe, expect, it } from 'vitest'
import {
  AUDIT_ENTRY_STATUSES,
  AUDIT_STATUSES,
  SOURCE_KINDS,
} from '../src/constants.ts'

describe('constants are canonical tuples', () => {
  it('audit statuses split CLI-level from entry-level', () => {
    expect(AUDIT_STATUSES).toContain('unaudited')
    expect(AUDIT_ENTRY_STATUSES).not.toContain('unaudited')
  })

  it('source kinds cover npm, gh, crate, collection, curator', () => {
    expect(new Set(SOURCE_KINDS)).toEqual(new Set(['npm', 'gh', 'crate', 'collection', 'curator']))
  })
})
