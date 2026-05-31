/**
 * Pipeline stage versions — bump the relevant one when its logic changes so
 * cached artifacts and persisted guides become "stale" and only the affected
 * stage re-runs (on only the affected guides). This is what keeps a tweak from
 * forcing a blanket regen:
 *   - BUCKET_RULES_VERSION  → bump on bucketing-rule changes (buckets.ts).
 *   - RUNBOOK_PROMPT_VERSION → bump on synthesis prompt changes (prompt.ts).
 * Per-version sections, counts, windowing, and all UI are deterministic from
 * stored buckets and need NO version bump (zero regen).
 */

/** Bumped 1→2: `[**BREAKING**]` marker detection (jest/babel monorepo changelogs). */
export const BUCKET_RULES_VERSION = 2

/** Synthesis prompt revision. */
export const RUNBOOK_PROMPT_VERSION = 1

/** Composite stamp persisted on each guide; mismatch ⇒ regenerate that guide. */
export const PIPELINE_VERSION = `b${BUCKET_RULES_VERSION}.p${RUNBOOK_PROMPT_VERSION}`
