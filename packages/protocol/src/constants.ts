/**
 * Canonical readonly tuples for closed enums and their inferred TS unions.
 *
 * The wire schemas import these tuples for `z.enum(...)`. The TS unions are
 * exported for tooling (autocomplete, exhaustive switches).
 */

export const SOURCE_KINDS = [
  'npm',
  'gh',
  'crate',
  'collection',
  'curator',
] as const

export const AUDIT_STATUSES = [
  'pass',
  'warn',
  'fail',
  'unaudited',
] as const

export const AUDIT_ENTRY_STATUSES = [
  'pass',
  'warn',
  'fail',
] as const

export type SourceKind = typeof SOURCE_KINDS[number]
export type AuditStatus = typeof AUDIT_STATUSES[number]
export type AuditEntryStatus = typeof AUDIT_ENTRY_STATUSES[number]
