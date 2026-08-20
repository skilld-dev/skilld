import type {
  HarnessV1,
  HarnessV1NetworkSandboxSession,
  HarnessV1PromptTurnOptions,
  HarnessV1Skill,
  HarnessV1StartOptions,
  HarnessV1StreamPart,
} from '@ai-sdk/harness'
import type { Experimental_SandboxSession } from '@ai-sdk/provider-utils'
import { execFile } from 'node:child_process'
import { mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { promisify } from 'node:util'

const execFileAsync = promisify(execFile)

export interface FakePromptContext {
  readonly options: HarnessV1PromptTurnOptions
  readonly sandbox: Experimental_SandboxSession
  readonly workDir: string
}

export interface FakeHarnessCapture {
  readonly starts: HarnessV1StartOptions[]
  readonly prompts: HarnessV1PromptTurnOptions[]
  destroyed: number
}

export interface FakeHarnessOptions {
  readonly onPrompt: (context: FakePromptContext) => Promise<void>
  readonly failStart?: Error
  readonly failPrompt?: Error
}

const usage = {
  inputTokens: { total: 0, noCache: 0, cacheRead: 0, cacheWrite: 0 },
  outputTokens: { total: 0, text: 0, reasoning: 0 },
}

function resumeLifecycle() {
  return {
    type: 'resume-session' as const,
    harnessId: 'fake',
    specificationVersion: 'harness-v1' as const,
    data: {},
  }
}

function continueLifecycle() {
  return {
    type: 'continue-turn' as const,
    harnessId: 'fake',
    specificationVersion: 'harness-v1' as const,
    data: {},
  }
}

export function createFakeHarness(settings: FakeHarnessOptions): {
  harness: HarnessV1
  capture: FakeHarnessCapture
} {
  const capture: FakeHarnessCapture = { starts: [], prompts: [], destroyed: 0 }
  const harness: HarnessV1 = {
    specificationVersion: 'harness-v1',
    harnessId: 'fake',
    builtinTools: {},
    async doStart(options) {
      capture.starts.push(options)
      if (settings.failStart)
        throw settings.failStart
      return {
        sessionId: options.sessionId,
        isResume: false,
        async doPromptTurn(promptOptions) {
          capture.prompts.push(promptOptions)
          if (settings.failPrompt)
            throw settings.failPrompt
          await settings.onPrompt({
            options: promptOptions,
            sandbox: options.sandboxSession.restricted(),
            workDir: options.sessionWorkDir,
          })
          promptOptions.emit({ type: 'stream-start' })
          promptOptions.emit({ type: 'finish', finishReason: { unified: 'stop' }, totalUsage: usage } as HarnessV1StreamPart)
          return {
            submitToolResult: async () => {},
            done: Promise.resolve(),
          }
        },
        async doContinueTurn() {
          throw new Error('Fake Harness does not continue turns.')
        },
        async doCompact() {},
        async doSuspendTurn() {
          return continueLifecycle()
        },
        async doDetach() {
          return resumeLifecycle()
        },
        async doStop() {
          return resumeLifecycle()
        },
        async doDestroy() {
          capture.destroyed += 1
        },
      }
    },
  }
  return { harness, capture }
}

async function createSandboxSession(): Promise<HarnessV1NetworkSandboxSession> {
  const root = await mkdtemp(join(tmpdir(), 'skilld-harness-sandbox-'))

  const readBinaryFile = async ({ path }: { path: string }) =>
    readFile(path).then(value => Uint8Array.from(value), error => (error as NodeJS.ErrnoException).code === 'ENOENT' ? null : Promise.reject(error))

  const restricted: Experimental_SandboxSession = {
    description: 'Local fake sandbox.',
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
    readBinaryFile,
    async readTextFile({ path, encoding = 'utf8' }) {
      return readFile(path, { encoding: encoding as BufferEncoding }).catch(error => (error as NodeJS.ErrnoException).code === 'ENOENT' ? null : Promise.reject(error))
    },
    async writeFile({ path, content }) {
      await mkdir(dirname(path), { recursive: true })
      const reader = content.getReader()
      const chunks: Uint8Array[] = []
      while (true) {
        const result = await reader.read()
        if (result.done)
          break
        chunks.push(result.value)
      }
      await writeFile(path, Buffer.concat(chunks))
    },
    async writeBinaryFile({ path, content }) {
      await mkdir(dirname(path), { recursive: true })
      await writeFile(path, content)
    },
    async writeTextFile({ path, content, encoding = 'utf8' }) {
      await mkdir(dirname(path), { recursive: true })
      await writeFile(path, content, { encoding: encoding as BufferEncoding })
    },
    async run({ command, workingDirectory, env, abortSignal }) {
      const result = await execFileAsync('/bin/sh', ['-c', command], {
        cwd: workingDirectory ?? root,
        env: { ...process.env, ...env },
        signal: abortSignal,
        maxBuffer: 16 * 1024 * 1024,
        encoding: 'buffer',
      }).then(
        value => ({ exitCode: 0, stdout: value.stdout.toString('utf8'), stderr: value.stderr.toString('utf8') }),
        error => ({
          exitCode: typeof (error as { code?: unknown }).code === 'number' ? (error as { code: number }).code : 1,
          stdout: Buffer.from((error as { stdout?: Uint8Array }).stdout ?? []).toString('utf8'),
          stderr: Buffer.from((error as { stderr?: Uint8Array }).stderr ?? []).toString('utf8'),
        }),
      )
      return result
    },
    async spawn() {
      throw new Error('Fake sandbox does not spawn processes.')
    },
  }

  return {
    ...restricted,
    id: root,
    defaultWorkingDirectory: root,
    ports: [],
    async getPortEndpoint() {
      throw new Error('Fake sandbox does not expose ports.')
    },
    async getPortUrl() {
      throw new Error('Fake sandbox does not expose ports.')
    },
    async stop() {},
    async destroy() {},
    restricted: () => restricted,
  }
}

export function createFakeSandboxProvider() {
  return {
    specificationVersion: 'harness-sandbox-v1' as const,
    providerId: 'fake',
    createSession: createSandboxSession,
  }
}

export const skillSource = (name: string, body = '# Instructions\n\nUse the package API.\n'): string => `---\nname: ${name}\ndescription: Use ${name} when working with its public API.\n---\n\n${body}`

export function skillFromStart(skills: ReadonlyArray<HarnessV1Skill> | undefined, name: string): HarnessV1Skill | undefined {
  return skills?.find(skill => skill.name === name)
}
