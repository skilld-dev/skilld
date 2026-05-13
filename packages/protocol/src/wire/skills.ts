/**
 * Skill registry wire shapes.
 *
 * `SkillsResolveInput` is the body to `POST /api/skills/resolve`; the
 * response maps each requested package name to the curated repo coords plus
 * an `official` flag. `SkillDetailResponse` is the slim subset of
 * `GET /api/skills/{owner}/{repo}/{name}` that the CLI relies on — the server
 * may return additional fields (frontmatter, html, tags, etc.) which are
 * preserved as passthrough, but the schema pins the load-bearing contract.
 */

import { z } from 'zod'

export const SkillsResolveInputSchema = z.object({
  items: z.array(z.object({
    packageName: z.string().min(1),
    owner: z.string().optional(),
  })).max(200).default([]),
})

export const SkillsResolveEntrySchema = z.object({
  owner: z.string(),
  repo: z.string(),
  official: z.boolean(),
})

export const SkillsResolveResponseSchema = z.record(z.string(), SkillsResolveEntrySchema)

export const SkillDetailResponseSchema = z.object({
  owner: z.string(),
  repo: z.string(),
  name: z.string(),
  displayName: z.string(),
  installs: z.number(),
  branch: z.string().optional(),
  skillPath: z.string().nullable().optional(),
  raw: z.string().nullable().optional(),
  pushedAt: z.string().nullable().optional(),
})

export type SkillsResolveInput = z.infer<typeof SkillsResolveInputSchema>
export type SkillsResolveEntry = z.infer<typeof SkillsResolveEntrySchema>
export type SkillsResolveResponse = z.infer<typeof SkillsResolveResponseSchema>
export type SkillDetailResponse = z.infer<typeof SkillDetailResponseSchema>
