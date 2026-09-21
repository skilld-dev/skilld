import type { HarnessV1NetworkSandboxSession, HarnessV1PortEndpoint, HarnessV1SandboxProvider } from '@ai-sdk/harness'
import type { ChildProcessWithoutNullStreams } from 'node:child_process'
import { spawn as spawnChildProcess } from 'node:child_process'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { createServer } from 'node:net'
import { constants, tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { Readable } from 'node:stream'

/**
 * The local sandbox runs real processes on this computer.
 * It is a working directory and a port, not a security boundary.
 * Use a hosted sandbox provider when the Harness must contain what it runs.
 */
export interface CreateLocalSandboxOptions {
  /** Session root directory. The default is a new temporary directory. */
  readonly root?: string
  /** Bridge port. The default asks the operating system for a free port. */
  readonly port?: number
  /** Keep the session root after `destroy`. The default removes it. */
  readonly keepRoot?: boolean
}

/** The file and process surface the Harness hands to Skill code. */
type SandboxSession = ReturnType<HarnessV1NetworkSandboxSession['restricted']>
type SandboxProcessOptions = Parameters<SandboxSession['run']>[0]
type SandboxProcess = Awaited<ReturnType<SandboxSession['spawn']>>

const { signals } = constants

/**
 * Terminate a process and every process it started.
 * A detached child leads a process group that shares its identifier.
 */
function killProcessGroup(child: ChildProcessWithoutNullStreams): void {
  if (child.pid === undefined || child.exitCode !== null)
    return
  try {
    process.kill(-child.pid, 'SIGTERM')
  }
  catch (cause) {
    // ESRCH means the group already exited. Anything else needs the fallback.
    if (errorCode(cause) !== 'ESRCH')
      child.kill('SIGTERM')
  }
}

function errorCode(cause: unknown): string | undefined {
  return (cause as NodeJS.ErrnoException | undefined)?.code
}

async function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = createServer()
    server.once('error', reject)
    server.listen(0, '127.0.0.1', () => {
      const address = server.address()
      if (address === null || typeof address === 'string') {
        server.close(() => reject(new Error('The operating system did not report a free port.')))
        return
      }
      server.close(() => resolve(address.port))
    })
  })
}

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

interface StartedProcess {
  readonly handle: SandboxProcess
  /** Resolves with the exit code even when the caller aborted the process. */
  readonly exited: Promise<{ exitCode: number }>
}

function startProcess(
  options: SandboxProcessOptions,
  root: string,
  children: Set<ChildProcessWithoutNullStreams>,
): StartedProcess {
  // The process leads its own group so that killing it also kills what it
  // started. The OpenCode bridge starts OpenCode, which starts more processes.
  const child = spawnChildProcess('/bin/sh', ['-c', options.command], {
    cwd: options.workingDirectory ?? root,
    env: { ...process.env, ...options.env },
    detached: true,
  })
  children.add(child)

  const exited = new Promise<{ exitCode: number }>((resolve) => {
    child.once('close', (code, signal) => {
      children.delete(child)
      resolve({ exitCode: code ?? (signal === null ? 1 : 128 + (signals[signal] ?? 0)) })
    })
  })

  const kill = async (): Promise<void> => {
    killProcessGroup(child)
    await exited
  }

  const aborted = options.abortSignal === undefined
    ? undefined
    : new Promise<never>((_, reject) => {
        const onAbort = (): void => {
          killProcessGroup(child)
          reject(options.abortSignal?.reason ?? new Error('The sandbox process was aborted.'))
        }
        if (options.abortSignal?.aborted === true)
          onAbort()
        else
          options.abortSignal?.addEventListener('abort', onAbort, { once: true })
      })
  // The abort rejection is reported through `wait`. Nothing else observes it.
  aborted?.catch(() => {})

  return {
    exited,
    handle: {
      pid: child.pid,
      stdout: Readable.toWeb(child.stdout) as ReadableStream<Uint8Array>,
      stderr: Readable.toWeb(child.stderr) as ReadableStream<Uint8Array>,
      wait: () => aborted === undefined ? exited : Promise.race([exited, aborted]),
      kill,
    },
  }
}

function createSandboxSession(root: string, children: Set<ChildProcessWithoutNullStreams>): SandboxSession {
  const readBinaryFile = async ({ path }: { path: string }): Promise<Uint8Array | null> =>
    readFile(path).then(
      value => Uint8Array.from(value),
      cause => errorCode(cause) === 'ENOENT' ? null : Promise.reject(cause),
    )

  return {
    description: `Local sandbox rooted at ${root}. POSIX sh, real processes, no isolation.`,
    readBinaryFile,

    async readFile({ path }) {
      const content = await readBinaryFile({ path })
      if (content === null)
        return null
      return new ReadableStream<Uint8Array>({
        start(controller) {
          controller.enqueue(content)
          controller.close()
        },
      })
    },

    async readTextFile({ path, encoding = 'utf8', startLine, endLine }) {
      const content = await readFile(path, { encoding: encoding as BufferEncoding })
        .catch(cause => errorCode(cause) === 'ENOENT' ? null : Promise.reject(cause))
      if (content === null || (startLine === undefined && endLine === undefined))
        return content
      const lines = content.split('\n')
      return lines.slice((startLine ?? 1) - 1, endLine ?? lines.length).join('\n')
    },

    async writeFile({ path, content }) {
      await mkdir(dirname(path), { recursive: true })
      await writeFile(path, Buffer.from(await readStream(content), 'utf8'))
    },

    async writeBinaryFile({ path, content }) {
      await mkdir(dirname(path), { recursive: true })
      await writeFile(path, content)
    },

    async writeTextFile({ path, content, encoding = 'utf8' }) {
      await mkdir(dirname(path), { recursive: true })
      await writeFile(path, content, { encoding: encoding as BufferEncoding })
    },

    async spawn(options) {
      return startProcess(options, root, children).handle
    },

    async run(options) {
      // An aborted `run` reports the exit code of the killed process.
      // Only `spawn` rejects its wait, because its caller holds the handle.
      const { handle, exited } = startProcess(options, root, children)
      const [stdout, stderr, exit] = await Promise.all([
        readStream(handle.stdout),
        readStream(handle.stderr),
        exited,
      ])
      return { exitCode: exit.exitCode, stdout, stderr }
    },
  }
}

/**
 * Create a sandbox provider that runs Harness sessions on this computer.
 *
 * The session needs POSIX `sh` at `/bin/sh`, so it runs on macOS and Linux
 * and not on Windows. It exposes one port on
 * `127.0.0.1` for bridge-backed Harness adapters. It applies no isolation:
 * every process reaches the whole computer and the caller's environment.
 */
export function createLocalSandbox(options: CreateLocalSandboxOptions = {}): HarnessV1SandboxProvider {
  return {
    specificationVersion: 'harness-sandbox-v1',
    providerId: 'skilld-local',

    async createSession(): Promise<HarnessV1NetworkSandboxSession> {
      const root = options.root ?? await mkdtemp(join(tmpdir(), 'skilld-local-sandbox-'))
      await mkdir(root, { recursive: true })
      const children = new Set<ChildProcessWithoutNullStreams>()
      const session = createSandboxSession(root, children)
      let ports: ReadonlyArray<number> = [options.port ?? await freePort()]

      const stop = async (): Promise<void> => {
        for (const child of children)
          killProcessGroup(child)
        children.clear()
      }

      const endpoint = ({ port, protocol = 'http' }: { port: number, protocol?: 'http' | 'https' | 'ws' }): HarnessV1PortEndpoint =>
        ({ url: `${protocol}://127.0.0.1:${port}` })

      return {
        ...session,
        id: root,
        defaultWorkingDirectory: root,
        get ports() {
          return ports
        },
        async getPortEndpoint(request) {
          return endpoint(request)
        },
        async getPortUrl(request) {
          return endpoint(request).url
        },
        async setPorts(next) {
          ports = [...next]
        },
        stop,
        async destroy() {
          await stop()
          if (options.keepRoot !== true)
            await rm(root, { recursive: true, force: true })
        },
        restricted: () => session,
      }
    },
  }
}
