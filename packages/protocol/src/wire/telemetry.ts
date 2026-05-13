/**
 * Telemetry wire shape: `POST /api/events/cli` body.
 *
 * `surface` is intentionally open (`z.string().min(1).max(32)`) so the CLI
 * can introduce new surfaces (`cli:foo`) without a coordinated protocol bump.
 * The closed `TelemetrySurface` TS union in `../constants.ts` documents the
 * currently canonical list for tooling.
 */

import { z } from 'zod'
import { AUTH_FLOWS, SOURCE_KINDS, TELEMETRY_EVENTS } from '../constants.ts'

export const CliEventInputSchema = z.object({
  event: z.enum(TELEMETRY_EVENTS),
  surface: z.string().min(1).max(32),
  sourceKind: z.enum(SOURCE_KINDS).optional(),
  slug: z.string().max(256).optional(),
  cliVersion: z.string().max(32),
  agent: z.string().max(32).optional(),
  durationMs: z.number().int().nonnegative().optional(),
  userId: z.number().int().optional(),
  flow: z.enum(AUTH_FLOWS).optional(),
})

export type CliEventInput = z.infer<typeof CliEventInputSchema>
