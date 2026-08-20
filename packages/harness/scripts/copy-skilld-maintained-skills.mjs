import { cp, mkdir, readFile, rm } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const packageDir = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const sourceDir = resolve(packageDir, '../../skills')
const outputDir = resolve(packageDir, 'dist/skills')
const names = JSON.parse(await readFile(resolve(sourceDir, 'harness-workflows.json'), 'utf8'))

await rm(outputDir, { recursive: true, force: true })
await mkdir(outputDir, { recursive: true })

for (const name of names)
  await cp(resolve(sourceDir, name), resolve(outputDir, name), { recursive: true })

await cp(resolve(sourceDir, 'harness-workflows.json'), resolve(outputDir, 'harness-workflows.json'))
