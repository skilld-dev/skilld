import type { detectBehaviors } from '../src/behaviors.ts'
import { mkdtemp, rm } from 'node:fs/promises'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { runInNewContext } from 'node:vm'
import replace from '@rollup/plugin-replace'
import { build as buildPackage } from 'obuild'
import { build as buildConsumer } from 'vite'
import { expect, it } from 'vitest'
import config from '../build.config.ts'

it('detects environment reads after a consumer production build', async () => {
  const output = await mkdtemp(join(tmpdir(), 'skilld-protocol-consumer-'))
  const require = createRequire(import.meta.url)
  const files = [{ path: 'SKILL.md', text: '`import.meta.env.TOKEN`' }]
  try {
    await buildPackage({
      ...config,
      cwd: fileURLToPath(new URL('..', import.meta.url)),
      entries: config.entries!.map(entry => ({ ...typeof entry === 'string' ? { type: 'bundle' as const, input: entry } : entry, outDir: output, dts: false, license: false })),
    })
    const result = await buildConsumer({
      configFile: false,
      logLevel: 'silent',
      // Nitro replaces this text even inside string literals.
      plugins: [replace({ 'preventAssignment': true, 'import.meta.env': 'globalThis._importMeta_.env' })],
      resolve: { alias: { zod: require.resolve('zod') } },
      build: {
        write: false,
        minify: false,
        lib: { entry: join(output, 'behaviors.mjs'), formats: ['iife'], name: 'protocol' },
      },
    })
    const bundle = (Array.isArray(result) ? result[0] : result) as { output: Array<{ type: string, code?: string }> }
    const code = bundle.output.find(chunk => chunk.type === 'chunk')!.code!
    const context = { protocol: undefined as { detectBehaviors: typeof detectBehaviors } | undefined }
    runInNewContext(code, context)
    expect(context.protocol!.detectBehaviors(files)).toEqual([{
      id: 'env',
      tier: 'show',
      label: 'Reads environment variables',
      locations: [{ path: 'SKILL.md', line: 1 }],
      total: 1,
    }])
  }
  finally {
    await rm(output, { recursive: true, force: true })
  }
}, 30_000)
