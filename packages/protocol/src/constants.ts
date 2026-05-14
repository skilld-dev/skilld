/**
 * Canonical readonly tuples for closed enums and their inferred TS unions.
 *
 * The wire schemas import these tuples for `z.enum(...)`. The TS unions are
 * exported for tooling (autocomplete, exhaustive switches). Telemetry surface
 * is intentionally open on the wire (`z.string().min(1).max(32)` in
 * `wire/telemetry.ts`) so the CLI can ship new surfaces without a coordinated
 * protocol bump; the closed tuple here is the *currently canonical* list.
 */

export const TELEMETRY_EVENTS = [
  'install',
  'install-failed',
  'update',
  'audit-warn',
  'audit-fail',
  'audit-blocked',
  'auth-flow',
  'pull-checklist',
] as const

export const TELEMETRY_SURFACES = [
  'cli:add',
  'cli:pull',
  'cli:prepare',
  'cli:update',
  'cli:wizard',
  'cli:auth',
] as const

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

export const AUTH_FLOWS = [
  'pkce',
  'device',
  'oidc',
] as const

export type TelemetryEvent = typeof TELEMETRY_EVENTS[number]
export type TelemetrySurface = typeof TELEMETRY_SURFACES[number]
export type SourceKind = typeof SOURCE_KINDS[number]
export type AuditStatus = typeof AUDIT_STATUSES[number]
export type AuditEntryStatus = typeof AUDIT_ENTRY_STATUSES[number]
export type AuthFlow = typeof AUTH_FLOWS[number]
