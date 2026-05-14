import { describe, expect, it } from 'vitest'
import {
  AUDIT_ENTRY_STATUSES,
  AUDIT_STATUSES,
  AUTH_FLOWS,
  SOURCE_KINDS,
  TELEMETRY_EVENTS,
  TELEMETRY_SURFACES,
} from '../src/constants.ts'

describe('constants are canonical tuples', () => {
  it('telemetry events include the v2 events', () => {
    expect(TELEMETRY_EVENTS).toEqual([
      'install',
      'install-failed',
      'update',
      'audit-warn',
      'audit-fail',
      'audit-blocked',
      'auth-flow',
      'pull-checklist',
    ])
  })

  it('audit statuses split CLI-level from entry-level', () => {
    expect(AUDIT_STATUSES).toContain('unaudited')
    expect(AUDIT_ENTRY_STATUSES).not.toContain('unaudited')
  })

  it('telemetry surfaces are kebab-prefixed', () => {
    for (const s of TELEMETRY_SURFACES)
      expect(s.startsWith('cli:')).toBe(true)
  })

  it('source kinds cover npm, gh, crate, collection, curator', () => {
    expect(new Set(SOURCE_KINDS)).toEqual(new Set(['npm', 'gh', 'crate', 'collection', 'curator']))
  })

  it('auth flows match the three v2 paths', () => {
    expect(AUTH_FLOWS).toEqual(['pkce', 'device', 'oidc'])
  })
})
