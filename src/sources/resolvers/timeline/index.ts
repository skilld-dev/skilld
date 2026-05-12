/**
 * Default timeline cascade. Order is independent (each step has its own
 * existsSync guard) but kept stable for telemetry consistency:
 *   1. issues  2. discussions  3. releases
 */

import type { StepResolver } from '../cascade.ts'
import type { TimelineCtx } from './types.ts'
import { discussionsStep } from './discussions.ts'
import { issuesStep } from './issues.ts'
import { releasesStep } from './releases.ts'

export type { TimelineCtx } from './types.ts'

export const defaultTimelineSteps: ReadonlyArray<StepResolver<TimelineCtx>> = [
  issuesStep,
  discussionsStep,
  releasesStep,
]
