/**
 * Tiny cascade walker for content + timeline resolvers.
 *
 * Each step is `{ id, canResolve?, run }`. Steps mutate a shared `ctx`
 * accumulator in order. Unlike URL resolution there is no fatal/skip outcome —
 * steps return void; `canResolve` short-circuits cheap pre-checks.
 */

export interface StepResolver<TCtx> {
  id: string
  canResolve?: (ctx: TCtx) => boolean
  run: (ctx: TCtx) => Promise<void>
}

export function defineStep<TCtx>(step: StepResolver<TCtx>): StepResolver<TCtx> {
  return step
}

export async function walkSteps<TCtx>(steps: ReadonlyArray<StepResolver<TCtx>>, ctx: TCtx): Promise<void> {
  for (const step of steps) {
    if (step.canResolve && !step.canResolve(ctx))
      continue
    await step.run(ctx)
  }
}
