import { createHash } from 'node:crypto'
import { cp, lstat, mkdir, readdir, readFile, rm } from 'node:fs/promises'
import { dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const packageDir = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const sourceDir = resolve(packageDir, '../../skills')
const outputDir = resolve(packageDir, 'dist/skills')
const manifestNames = ['skilld-maintained-skills.json', 'harness-workflows.json']
const names = JSON.parse(await readFile(resolve(sourceDir, manifestNames[0]), 'utf8'))

async function inventory(root, current = root) {
  const files = []
  for (const entry of await readdir(current, { withFileTypes: true })) {
    const path = join(current, entry.name)
    const stat = await lstat(path)
    if (stat.isSymbolicLink() || (!stat.isDirectory() && !stat.isFile()))
      throw new Error(`Skill asset must be a regular file: ${relative(root, path)}`)
    if (stat.isDirectory())
      files.push(...await inventory(root, path))
    else
      files.push(relative(root, path))
  }
  return files.sort()
}

async function digest(path) {
  return createHash('sha256').update(await readFile(path)).digest('hex')
}

async function assertCopy(source, output) {
  const sourceFiles = await inventory(source)
  const outputFiles = await inventory(output)
  if (JSON.stringify(sourceFiles) !== JSON.stringify(outputFiles))
    throw new Error(`Packaged Skill file list differs: ${source}`)
  for (const path of sourceFiles) {
    if (await digest(join(source, path)) !== await digest(join(output, path)))
      throw new Error(`Packaged Skill hash differs: ${path}`)
  }
}

await rm(outputDir, { recursive: true, force: true })
await mkdir(outputDir, { recursive: true })

for (const name of names) {
  const source = resolve(sourceDir, name)
  const output = resolve(outputDir, name)
  await cp(source, output, { recursive: true })
  await assertCopy(source, output)
}

for (const name of manifestNames) {
  const source = resolve(sourceDir, name)
  const output = resolve(outputDir, name)
  await cp(source, output)
  if (await digest(source) !== await digest(output))
    throw new Error(`Packaged Skill manifest hash differs: ${name}`)
}
