import { spawnSync } from 'node:child_process'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

const script = resolve(import.meta.dirname, '../../../../skills/generate-package-skill/scripts/extract-blocks.mjs')

const skill = [
  '# Example',
  '',
  '```ts',
  '// server/api/search.ts',
  'export default defineEventHandler(() => fetch(\'https://example.com/api\'))',
  '```',
  '',
  '````md',
  '```ts',
  'nested',
  '```',
  '````',
  '',
  '```vue',
  '<!-- app/pages/index.vue -->',
  '<template><p>Hi</p></template>',
  '```',
  '',
  '```sh',
  'pnpm add retriv',
  '```',
  '',
  '```ts',
  '// ../outside.ts',
  'export {}',
  '```',
  '',
].join('\n')

describe('extract-blocks script', () => {
  let dir: string

  beforeEach(async () => {
    dir = await mkdtemp(join(tmpdir(), 'extract-blocks-'))
    await writeFile(join(dir, 'SKILL.md'), skill)
  })

  afterEach(async () => {
    await rm(dir, { recursive: true, force: true })
  })

  it('writes each block to its own directory under the path its first line names', async () => {
    const out = join(dir, 'blocks')
    const run = spawnSync(process.execPath, [script, join(dir, 'SKILL.md'), out, '--replace', 'https://example.com=http://localhost:4000'], { encoding: 'utf8' })

    expect(await readFile(join(out, '1/server/api/search.ts'), 'utf8')).toBe('// server/api/search.ts\nexport default defineEventHandler(() => fetch(\'http://localhost:4000/api\'))\n')
    expect(await readFile(join(out, '2/block.md'), 'utf8')).toBe('```ts\nnested\n```\n')
    expect(await readFile(join(out, '3/app/pages/index.vue'), 'utf8')).toContain('<template>')
    expect(await readFile(join(out, '4/block.sh'), 'utf8')).toBe('pnpm add retriv\n')
    const manifest = JSON.parse(await readFile(join(out, 'blocks.json'), 'utf8'))
    expect(manifest.map((entry: { block: number, line: number, file: string }) => [entry.block, entry.line, entry.file])).toEqual([
      [1, 4, '1/server/api/search.ts'],
      [2, 9, '2/block.md'],
      [3, 15, '3/app/pages/index.vue'],
      [4, 20, '4/block.sh'],
    ])
    expect(run.status).toBe(1)
    expect(run.stderr).toContain('block 5 line 24: path ../outside.ts leaves the output directory; skipped')
  })

  it('refuses a replacement without a target', () => {
    const run = spawnSync(process.execPath, [script, join(dir, 'SKILL.md'), join(dir, 'out'), '--replace', 'https://example.com'], { encoding: 'utf8' })

    expect(run.status).toBe(2)
    expect(run.stderr).toContain('--replace needs FROM=TO.')
  })
})
