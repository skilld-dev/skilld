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
  if (request.url === '/crash')
    return request.socket.destroy()
  if (request.url === '/logo.png') {
    response.statusCode = 201
    response.setHeader('content-type', 'image/png')
    return response.end(Buffer.from([0x89, 0x50, 0x4E, 0x47, 0x00, 0xFF, 0xFE]))
  }
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
  child.stdout?.on('data', (chunk) => {
    stdout += chunk
  })
  child.stderr?.on('data', (chunk) => {
    stderr += chunk
  })
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
    // A failed test can leave the server running. Stop it so it never outlives the suite.
    const pid = Number(await readFile(pidFile, 'utf8').catch(() => '0'))
    if (pid > 0 && isAlive(pid))
      process.kill(pid, 'SIGKILL')
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

  it('writes each body to its own file and lists them in responses.json', async () => {
    const out = join(dir, 'pages')
    const run = await collect(spawn(process.execPath, [script, '--fetch', '/', '--fetch', '/a/b', '--fetch', '/a_b', '--out', out, '--', 'sh', wrapper, '{port}']))

    expect(run.code).toBe(0)
    const responses = JSON.parse(await readFile(join(out, 'responses.json'), 'utf8')) as Array<{ path: string, file: string }>
    expect(responses.map(response => response.path)).toEqual(['/', '/a/b', '/a_b'])
    expect(new Set(responses.map(response => response.file)).size).toBe(3)
    const bodies = await Promise.all(responses.map(response => readFile(join(out, response.file), 'utf8')))
    expect(bodies).toEqual(['<p>/ text/html</p>', '<p>/a/b text/html</p>', '<p>/a_b text/html</p>'])
  })

  it('saves a raw response body byte for byte with its status and content type', async () => {
    const out = join(dir, 'raw')
    const run = await collect(spawn(process.execPath, [script, '--fetch-raw', '/logo.png', '--out', out, '--', 'sh', wrapper, '{port}']))

    expect(run.code).toBe(0)
    const [response] = JSON.parse(await readFile(join(out, 'responses.json'), 'utf8')) as Array<{ file: string }>
    expect(response).toMatchObject({ path: '/logo.png', status: 201, contentType: 'image/png', bytes: 7 })
    expect([...await readFile(join(out, response!.file))]).toEqual([0x89, 0x50, 0x4E, 0x47, 0x00, 0xFF, 0xFE])
  })

  it('refuses a raw fetch without an output directory', async () => {
    const run = await collect(spawn(process.execPath, [script, '--fetch-raw', '/logo.png', '--', 'sh', wrapper, '{port}']))

    expect(run.code).toBe(2)
    expect(run.stderr).toContain('--fetch-raw needs --out')
  })

  it('reports a failed fetch and still stops the server', async () => {
    const run = await collect(spawn(process.execPath, [script, '--fetch', '/', '--fetch', '/crash', '--fetch', '/after', '--', 'sh', wrapper, '{port}']))

    expect(run.code).toBe(1)
    expect(run.stderr).toMatch(/GET \/crash failed: /)
    expect(run.stdout).toContain('<p>/after text/html</p>')
    const pid = Number(await readFile(pidFile, 'utf8'))
    expect(isAlive(pid)).toBe(false)
  }, 15_000)

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

  it('kills the group at once on a second SIGTERM', async () => {
    // The real node binary leads the group, so no shim exits early on the first SIGTERM.
    const child = spawn(process.execPath, [script, '--', process.execPath, join(dir, 'server.mjs'), '{port}', pidFile])
    const result = collect(child)
    await new Promise<void>((resolve) => {
      child.stderr.on('data', (chunk: Buffer) => {
        if (chunk.toString().includes('ready http://localhost:'))
          resolve()
      })
    })
    const pid = Number(await readFile(pidFile, 'utf8'))

    // The server ignores SIGTERM, so the first stop waits. The second signal must not skip the kill.
    child.kill('SIGTERM')
    await new Promise(resolve => setTimeout(resolve, 200))
    child.kill('SIGTERM')
    const started = Date.now()
    const run = await result

    expect(run.code).toBe(130)
    expect(Date.now() - started).toBeLessThan(3_000)
    await expect.poll(() => isAlive(pid), { timeout: 2_000 }).toBe(false)
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
