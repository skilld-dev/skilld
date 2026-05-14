/**
 * `skilld-protocol/wire` — every shape that crosses the CLI ↔ skilld.dev
 * boundary, expressed as zod schemas with matching inferred TS types.
 *
 * Schemas and types share names: `import { AuditEntry } from 'skilld-protocol/wire'`
 * resolves to the zod schema for runtime use; `import type { AuditEntry } from ...`
 * gives you the inferred TS type. Pick whichever the call site needs.
 */

export * from './wire/audit.ts'
export * from './wire/auth.ts'
export * from './wire/collections.ts'
export * from './wire/device.ts'
export * from './wire/skills.ts'
export * from './wire/telemetry.ts'
