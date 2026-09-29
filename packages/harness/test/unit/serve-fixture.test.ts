import { spawn } from 'node:child_process'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

const script = resolve(import.meta.dirname, '../../../../skills/generate-package-skill/scripts/serve-fixture.mjs')

// The server records its pid, so a test can prove the group stop reached a grandchild.
const server = `
import { createServer } from 'node:http'
import { writeFileSync } from 'node:fs'
writeFileSync(process.argv[3], String(process.pid))
process.on('SIGTERM', () => {})
createServer((request, response) => {
  response.setHeader('content-type', 'text/html')
  response.end('<p>' + request.url + ' ' + request.headers.accept + '</p>')
}).listen(Number(process.argv[2]), '127.0.0.1')
`

interface Run {
  code: number | null
  stdout: string
  stderr: string
}

function collect(child: ReturnType<typeof spawn>): Promise<Run> {
  let stdout = ''
  let stderr = ''
  child.stdout?.on('data', (chunk) => { stdout += chunk })
  child.stderr?.on('data', (chunk) => { stderr += chunk })
  return new Promise(resolve => child.once('exit', code => resolve({ code, stdout, stderr })))
}

function isAlive(pid: number): boolean {
  try {
    process.kill(pid, 0)
    return true
  }
  catch {
    return false
  }
}

describe('serve-fixture script', () => {
  let dir: string
  let wrapper: string
  let pidFile: string

  beforeEach(async () => {
    dir = await mkdtemp(join(tmpdir(), 'serve-fixture-'))
    pidFile = join(dir, 'server.pid')
    await writeFile(join(dir, 'server.mjs'), server)
    // Like the pnpm shim: a shell that runs the server as a child, not with exec.
    wrapper = join(dir, 'wrapper.sh')
    await writeFile(wrapper, `node "${join(dir, 'server.mjs')}" "$1" "${pidFile}"\nexit $?\n`)
  })

  afterEach(async () => {
    await rm(dir, { recursive: true, force: true })
  })

  it('fetches a path as HTML and stops the server behind a wrapper', async () => {
    const run = await collect(spawn(process.execPath, [script, '--fetch', '/hello', '--', 'sh', wrapper, '{port}']))

    expect(run.code).toBe(0)
    expect(run.stdout).toContain('<p>/hello text/html</p>')
    expect(run.stderr).toMatch(/GET \/hello 200 text\/html \d+/)
    const pid = Number(await readFile(pidFile, 'utf8'))
    expect(isAlive(pid)).toBe(false)
  })

  it('writes each body to the output directory', async () => {
    const out = join(dir, 'pages')
    const run = await collect(spawn(process.execPath, [script, '--fetch', '/', '--fetch', '/a/b', '--out', out, '--', 'sh', wrapper, '{port}']))

    expect(run.code).toBe(0)
    await expect(readFile(join(out, 'index.html'), 'utf8')).resolves.toBe('<p>/ text/html</p>')
    await expect(readFile(join(out, 'a_b.html'), 'utf8')).resolves.toBe('<p>/a/b text/html</p>')
  })

  it('reports a server that exits before it answers', async () => {
    const run = await collect(spawn(process.execPath, [script, '--fetch', '/', '--', 'node', '-e', 'process.exit(3)']))

    expect(run.code).toBe(1)
    expect(run.stderr).toContain('exited before it answered: code 3')
  })

  it('holds the server until SIGTERM, then stops the group', async () => {
    const child = spawn(process.execPath, [script, '--', 'sh', wrapper, '{port}'])
    const result = collect(child)
    await new Promise<void>((resolve) => {
      child.stderr.on('data', (chunk: Buffer) => {
        if (chunk.toString().includes('ready http://localhost:'))
          resolve()
      })
    })
    const pid = Number(await readFile(pidFile, 'utf8'))
    expect(isAlive(pid)).toBe(true)

    child.kill('SIGTERM')
    const run = await result

    expect(run.code).toBe(130)
    expect(isAlive(pid)).toBe(false)
  }, 15_000)

  it('stops the server when its parent shell dies', async () => {
    // Like a `node` shim: the shell runs the script as a child and dies on SIGTERM.
    const shell = spawn('sh', ['-c', `"${process.execPath}" "${script}" -- sh "${wrapper}" "{port}"; exit $?`], { stdio: 'ignore' })
    const exited = new Promise(resolve => shell.once('exit', resolve))
    const pid = await waitForPid()

    shell.kill('SIGTERM')
    await exited

    await expect.poll(() => isAlive(pid), { timeout: 5_000 }).toBe(false)
  }, 15_000)

  async function waitForPid(): Promise<number> {
    await expect.poll(() => readFile(pidFile, 'utf8').catch(() => ''), { timeout: 10_000 }).not.toBe('')
    return Number(await readFile(pidFile, 'utf8'))
  }
})
