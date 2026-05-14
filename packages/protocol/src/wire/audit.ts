/**
 * Audit endpoint wire shapes.
 *
 * `SkillLiveResponse` is the actual server return from
 * `GET /api/skill-live/{owner}/{repo}/{name}` — note it does NOT carry an
 * aggregated `status` field. CLI computes that locally from `audits[]`.
 */

import { z } from 'zod'

/**
 * Audit entry as returned by `skills.sh` and proxied unchanged through
 * skilld.dev's `/api/skill-live/{id}`. `status` is intentionally a loose
 * string on the wire — upstream may grow values we don't yet know about.
 * Consumers narrow against `AUDIT_ENTRY_STATUSES` when they care.
 */
export const AuditEntrySchema = z.object({
  provider: z.string(),
  slug: z.string(),
  status: z.string(),
  summary: z.string().optional(),
  auditedAt: z.string().optional(),
  riskLevel: z.string().optional(),
  categories: z.array(z.string()).optional(),
})

export const SkillLiveResponseSchema = z.object({
  id: z.string(),
  installs: z.number().nullable(),
  formatted: z.string().nullable(),
  audits: z.array(AuditEntrySchema),
  source: z.literal('skills.sh'),
  fetchedAt: z.string(),
})

export type AuditEntry = z.infer<typeof AuditEntrySchema>
export type SkillLiveResponse = z.infer<typeof SkillLiveResponseSchema>
