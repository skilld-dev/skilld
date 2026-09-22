import { lstat, mkdtemp, readFile, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import { createLocalSandbox } from '../../src/sandbox-local.ts'

async function readStream(stream: ReadableStream<Uint8Array>): Promise<string> {
  const reader = stream.getReader()
  const chunks: Uint8Array[] = []
  while (true) {
    const result = await reader.read()
    if (result.done)
      break
    chunks.push(result.value)
  }
  return Buffer.concat(chunks).toString('utf8')
}

function isRunning(pid: number): boolean {
  try {
    process.kill(pid, 0)
    return true
  }
  catch (cause) {
    return (cause as NodeJS.ErrnoException).code !== 'ESRCH'
  }
}

async function readPid(path: string): Promise<number> {
  let pid = Number.NaN
  await waitUntil(() => Number.isInteger(pid), 2000, async () => {
    pid = Number(await readFile(path, 'utf8').then(value => value.trim(), () => ''))
  })
  if (!Number.isInteger(pid))
    throw new Error('The backgrounded process never reported its identifier.')
  return pid
}

async function waitUntil(condition: () => boolean, timeoutMs = 2000, probe?: () => Promise<void>): Promise<void> {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    await probe?.()
    if (condition())
      return
    await new Promise(resolve => setTimeout(resolve, 20))
  }
}

describe('local sandbox commands', () => {
  it('runs a command and reports its output', async () => {
    const session = await createLocalSandbox().createSession()
    try {
      const result = await session.run({ command: 'echo hello' })
      expect(result).toEqual({ exitCode: 0, stdout: 'hello\n', stderr: '' })
    }
    finally {
      await session.destroy()
    }
  })

  it('reports a failing command without throwing', async () => {
    const session = await createLocalSandbox().createSession()
    try {
      const result = await session.run({ command: 'echo nope >&2; exit 3' })
      expect(result.exitCode).toBe(3)
      expect(result.stderr).toBe('nope\n')
    }
    finally {
      await session.destroy()
    }
  })

  it('runs a command in the requested directory with the requested environment', async () => {
    const root = await mkdtemp(join(tmpdir(), 'skilld-sandbox-test-'))
    const session = await createLocalSandbox({ root }).createSession()
    try {
      const result = await session.run({ command: 'printf "%s %s" "$PWD" "$SKILLD_TEST"', workingDirectory: root, env: { SKILLD_TEST: 'value' } })
      expect(result.stdout).toBe(`${root} value`)
    }
    finally {
      await session.destroy()
    }
  })

  it('defaults the working directory to the session root', async () => {
    const root = await mkdtemp(join(tmpdir(), 'skilld-sandbox-test-'))
    const session = await createLocalSandbox({ root }).createSession()
    try {
      const result = await session.run({ command: 'pwd' })
      expect(result.stdout.trim()).toBe(root)
    }
    finally {
      await session.destroy()
    }
  })
})

describe('local sandbox processes', () => {
  it('streams the output of a spawned process and waits for its exit code', async () => {
    const session = await createLocalSandbox().createSession()
    try {
      const child = await session.spawn({ command: 'echo one; echo two >&2; exit 2' })
      const [stdout, stderr, exit] = await Promise.all([
        readStream(child.stdout),
        readStream(child.stderr),
        child.wait(),
      ])
      expect(stdout).toBe('one\n')
      expect(stderr).toBe('two\n')
      expect(exit.exitCode).toBe(2)
    }
    finally {
      await session.destroy()
    }
  })

  it('rejects the wait of an aborted process', async () => {
    const session = await createLocalSandbox().createSession()
    const controller = new AbortController()
    try {
      const child = await session.spawn({ command: 'sleep 30', abortSignal: controller.signal })
      controller.abort(new Error('stop now'))
      await expect(child.wait()).rejects.toThrow('stop now')
    }
    finally {
      await session.destroy()
    }
  })

  it('kills a spawned process on request', async () => {
    const session = await createLocalSandbox().createSession()
    try {
      const child = await session.spawn({ command: 'sleep 30' })
      await child.kill()
      await expect(child.wait()).resolves.toEqual({ exitCode: 143 })
    }
    finally {
      await session.destroy()
    }
  })
})

describe('local sandbox files', () => {
  it('writes and reads a text file', async () => {
    const root = await mkdtemp(join(tmpdir(), 'skilld-sandbox-test-'))
    const session = await createLocalSandbox({ root }).createSession()
    try {
      await session.writeTextFile({ path: join(root, 'nested/note.txt'), content: 'one\ntwo\nthree\n' })
      await expect(session.readTextFile({ path: join(root, 'nested/note.txt') })).resolves.toBe('one\ntwo\nthree\n')
    }
    finally {
      await session.destroy()
    }
  })

  it('reads an inclusive line range', async () => {
    const root = await mkdtemp(join(tmpdir(), 'skilld-sandbox-test-'))
    await writeFile(join(root, 'note.txt'), 'one\ntwo\nthree\nfour\n')
    const session = await createLocalSandbox({ root }).createSession()
    try {
      await expect(session.readTextFile({ path: join(root, 'note.txt'), startLine: 2, endLine: 3 })).resolves.toBe('two\nthree')
    }
    finally {
      await session.destroy()
    }
  })

  it('reports a missing file as null', async () => {
    const root = await mkdtemp(join(tmpdir(), 'skilld-sandbox-test-'))
    const session = await createLocalSandbox({ root }).createSession()
    try {
      await expect(session.readTextFile({ path: join(root, 'missing.txt') })).resolves.toBeNull()
      await expect(session.readBinaryFile({ path: join(root, 'missing.txt') })).resolves.toBeNull()
      await expect(session.readFile({ path: join(root, 'missing.txt') })).resolves.toBeNull()
    }
    finally {
      await session.destroy()
    }
  })

  it('writes and reads bytes', async () => {
    const root = await mkdtemp(join(tmpdir(), 'skilld-sandbox-test-'))
    const session = await createLocalSandbox({ root }).createSession()
    try {
      await session.writeBinaryFile({ path: join(root, 'bytes.bin'), content: Uint8Array.from([1, 2, 3]) })
      await expect(session.readBinaryFile({ path: join(root, 'bytes.bin') })).resolves.toEqual(Uint8Array.from([1, 2, 3]))
    }
    finally {
      await session.destroy()
    }
  })
})

describe('local sandbox ports and lifecycle', () => {
  it('exposes the requested port on the loopback address', async () => {
    const session = await createLocalSandbox({ port: 4321 }).createSession()
    try {
      expect(session.ports).toEqual([4321])
      await expect(session.getPortEndpoint({ port: 4321, protocol: 'ws' })).resolves.toEqual({ url: 'ws://127.0.0.1:4321' })
    }
    finally {
      await session.destroy()
    }
  })

  it('exposes a free port when the caller names none', async () => {
    const session = await createLocalSandbox().createSession()
    try {
      expect(session.ports).toHaveLength(1)
      expect(session.ports[0]).toBeGreaterThan(0)
    }
    finally {
      await session.destroy()
    }
  })

  it('replaces the exposed ports', async () => {
    const session = await createLocalSandbox({ port: 4321 }).createSession()
    try {
      await session.setPorts?.([5555])
      expect(session.ports).toEqual([5555])
    }
    finally {
      await session.destroy()
    }
  })

  it('removes the session root on destroy', async () => {
    const root = await mkdtemp(join(tmpdir(), 'skilld-sandbox-test-'))
    const session = await createLocalSandbox({ root }).createSession()
    await session.writeTextFile({ path: join(root, 'note.txt'), content: 'gone' })
    await session.destroy()
    await expect(lstat(root)).rejects.toThrow()
  })

  it('keeps the session root when the caller asks', async () => {
    const root = await mkdtemp(join(tmpdir(), 'skilld-sandbox-test-'))
    const session = await createLocalSandbox({ root, keepRoot: true }).createSession()
    await session.destroy()
    await expect(lstat(root).then(entry => entry.isDirectory())).resolves.toBe(true)
  })

  it('stops a backgrounded process after its parent shell exits', async () => {
    const root = await mkdtemp(join(tmpdir(), 'skilld-sandbox-test-'))
    const session = await createLocalSandbox({ root }).createSession()
    // The shell exits immediately, so the session only holds the group.
    await session.spawn({ command: 'sleep 30 & echo $! > pid' })
    const pid = await readPid(join(root, 'pid'))
    expect(isRunning(pid)).toBe(true)
    await session.destroy()
    await waitUntil(() => !isRunning(pid))
    expect(isRunning(pid)).toBe(false)
  })

  it('stops a running process when the session is destroyed', async () => {
    const session = await createLocalSandbox().createSession()
    const child = await session.spawn({ command: 'sleep 30' })
    await session.destroy()
    await expect(child.wait()).resolves.toEqual({ exitCode: 143 })
  })
})
