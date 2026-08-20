import { execFile } from 'node:child_process'
import { readFile, unlink } from 'node:fs/promises'
import { promisify } from 'node:util'

const run = promisify(execFile)
const packageJson = JSON.parse(await readFile(new URL('../package.json', import.meta.url), 'utf8'))
const archive = `skilld-harness-${packageJson.version}.tgz`

await run('pnpm', ['pack'])
try {
  const { stdout, stderr } = await run('attw', [archive, '--profile', 'esm-only'])
  process.stdout.write(stdout)
  process.stderr.write(stderr)
}
finally {
  await unlink(archive).catch((error) => {
    if (error.code !== 'ENOENT')
      throw error
  })
}
