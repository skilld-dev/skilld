import type { StepResolver } from '../../src/sources/resolvers/cascade'
import { describe, expect, it, vi } from 'vitest'
import { defineStep, walkSteps } from '../../src/sources/resolvers/cascade'

interface TestCtx {
  log: string[]
}

describe('cascade walker', () => {
  it('runs steps in declared order, mutating shared ctx', async () => {
    const a = defineStep<TestCtx>({ id: 'a', run: async (c) => { c.log.push('a') } })
    const b = defineStep<TestCtx>({ id: 'b', run: async (c) => { c.log.push('b') } })
    const c = defineStep<TestCtx>({ id: 'c', run: async (c) => { c.log.push('c') } })
    const ctx: TestCtx = { log: [] }
    await walkSteps([a, b, c], ctx)
    expect(ctx.log).toEqual(['a', 'b', 'c'])
  })

  it('skips a step when canResolve returns false (run is not invoked)', async () => {
    const run = vi.fn()
    const a = defineStep<TestCtx>({ id: 'a', run: async (c) => { c.log.push('a') } })
    const b = defineStep<TestCtx>({ id: 'b', canResolve: () => false, run })
    const c = defineStep<TestCtx>({ id: 'c', run: async (c) => { c.log.push('c') } })
    const ctx: TestCtx = { log: [] }
    await walkSteps([a, b, c], ctx)
    expect(ctx.log).toEqual(['a', 'c'])
    expect(run).not.toHaveBeenCalled()
  })

  it('canResolve sees mutations from earlier steps', async () => {
    const set = defineStep<TestCtx>({ id: 'set', run: async (c) => { c.log.push('set') } })
    const gated = defineStep<TestCtx>({
      id: 'gated',
      canResolve: c => c.log.length === 0,
      run: async (c) => { c.log.push('gated') },
    })
    const ctx: TestCtx = { log: [] }
    await walkSteps([set, gated] as StepResolver<TestCtx>[], ctx)
    expect(ctx.log).toEqual(['set'])
  })

  it('awaits async steps sequentially', async () => {
    const order: string[] = []
    const slow = defineStep<TestCtx>({
      id: 'slow',
      run: async () => {
        await new Promise(r => setTimeout(r, 10))
        order.push('slow')
      },
    })
    const fast = defineStep<TestCtx>({ id: 'fast', run: async () => { order.push('fast') } })
    await walkSteps([slow, fast], { log: [] })
    expect(order).toEqual(['slow', 'fast'])
  })
})
