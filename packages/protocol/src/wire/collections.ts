/**
 * Collection / curator / subscription / change wire shapes.
 */

import { z } from 'zod'
import { SOURCE_KINDS } from '../constants.ts'

export const CollectionManifestItemSchema = z.object({
  kind: z.enum(['npm', 'gh', 'crate']),
  package: z.string().optional(),
  owner: z.string().optional(),
  repo: z.string().optional(),
  /**
   * For `kind: 'gh'` — the specific skill directory name within the repo.
   * Required to disambiguate when one repo hosts many skills (e.g.
   * `addyosmani/web-quality-skills` has 6 distinct skills). Optional on
   * `kind: 'npm'` and `kind: 'crate'` where `package` is the unique identifier.
   */
  name: z.string().optional(),
})

export const CollectionManifestSchema = z.object({
  name: z.string(),
  preamble: z.string().optional(),
  items: z.array(CollectionManifestItemSchema),
})

export const CollectionSummarySchema = z.object({
  slug: z.string(),
  name: z.string(),
  itemCount: z.number().int().nonnegative(),
})

export const SubscriptionSummarySchema = z.object({
  login: z.string(),
  slug: z.string(),
})

export const CuratorPayloadSchema = z.object({
  login: z.string(),
  collections: z.array(CollectionSummarySchema),
})

export const ChangeEntrySchema = z.object({
  repo: z.string(),
  skill: z.string(),
  at: z.string(),
  summary: z.string().optional(),
})

/**
 * `/api/cli/changes` envelope. CLI uses `windowEnd` as the canonical
 * `lastDigestAt` watermark when persisting; reads `entries` for rendering.
 */
export const DigestResponseSchema = z.object({
  user: z.object({ id: z.number(), login: z.string() }).optional(),
  windowStart: z.number(),
  windowEnd: z.number(),
  entries: z.array(ChangeEntrySchema),
})

export const InstallEventPayloadSchema = z.object({
  slug: z.string(),
  sourceKind: z.enum(SOURCE_KINDS),
  surface: z.string().min(1).max(32),
  agent: z.string().optional(),
})

export type CollectionManifestItem = z.infer<typeof CollectionManifestItemSchema>
export type CollectionManifest = z.infer<typeof CollectionManifestSchema>
export type CollectionSummary = z.infer<typeof CollectionSummarySchema>
export type SubscriptionSummary = z.infer<typeof SubscriptionSummarySchema>
export type CuratorPayload = z.infer<typeof CuratorPayloadSchema>
export type ChangeEntry = z.infer<typeof ChangeEntrySchema>
export type DigestResponse = z.infer<typeof DigestResponseSchema>
export type InstallEventPayload = z.infer<typeof InstallEventPayloadSchema>
