import type { IndexConfig, Document as RetrivDocument } from './types.ts'
import type { WorkerMessage, WorkerResponse } from './worker.ts'
import { existsSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { Worker } from 'node:worker_threads'
import { dirname, join } from 'pathe'
import { SearchDepsUnavailableError } from './index.ts'

function reconstructError(message: string, name?: string): Error {
  if (name === 'SearchDepsUnavailableError')
    return new SearchDepsUnavailableError(undefined, message)
  return new Error(message)
}

interface PendingTask {
  id: number
  resolve: () => void
  reject: (err: Error) => void
  onProgress?: IndexConfig['onProgress']
}

let worker: Worker | null = null
let taskId = 0
const pending = new Map<number, PendingTask>()
const queue: Array<() => void> = []
let running = false

function resolveWorkerPath(): { path: string, execArgv?: string[] } {
  const dir = dirname(fileURLToPath(import.meta.url))

  // Bundled: dist/retriv/worker.mjs (resolve from package root, not chunk dir)
  for (const candidate of [join(dir, 'worker.mjs'), join(dir, '..', 'retriv', 'worker.mjs')]) {
    if (existsSync(candidate))
      return { path: candidate }
  }

  // Dev stub: src/retriv/pool.ts → src/retriv/worker.ts
  return { path: join(dir, 'worker.ts'), execArgv: ['--experimental-strip-types'] }
}

function ensureWorker(): Worker {
  if (worker)
    return worker

  const config = resolveWorkerPath()
  const w = new Worker(config.path, {
    execArgv: config.execArgv,
  })

  w.on('message', (msg: WorkerResponse) => {
    const task = pending.get(msg.id)
    if (!task)
      return

    if (msg.type === 'progress') {
      task.onProgress?.({ phase: msg.phase as any, current: msg.current, total: msg.total })
    }
    else if (msg.type === 'done') {
      pending.delete(msg.id)
      task.resolve()
    }
    else if (msg.type === 'error') {
      pending.delete(msg.id)
      task.reject(reconstructError(msg.message, msg.name))
    }
  })

  w.on('error', (err: Error) => {
    for (const task of pending.values())
      task.reject(err)
    pending.clear()
    worker = null
  })

  w.on('exit', (code) => {
    if (pending.size > 0) {
      const err = new Error(`Worker exited (code ${code}) with ${pending.size} pending tasks`)
      for (const task of pending.values())
        task.reject(err)
      pending.clear()
    }
    worker = null
  })

  worker = w
  return w
}

function drainQueue() {
  if (running || queue.length === 0)
    return
  const next = queue.shift()!
  next()
}

export async function createIndexInWorker(
  documents: RetrivDocument[],
  config: IndexConfig & { removeIds?: string[] },
): Promise<void> {
  return new Promise<void>((resolve, reject) => {
    const run = () => {
      running = true
      const id = ++taskId

      let w: Worker
      try {
        w = ensureWorker()
      }
      catch (err) {
        running = false
        drainQueue()
        reject(err instanceof Error ? err : new Error(String(err)))
        return
      }

      pending.set(id, {
        id,
        resolve: () => {
          running = false
          drainQueue()
          resolve()
        },
        reject: (err) => {
          running = false
          drainQueue()
          reject(err)
        },
        onProgress: config.onProgress,
      })

      const msg: WorkerMessage = {
        type: 'index',
        id,
        documents,
        dbPath: config.dbPath,
        removeIds: config.removeIds,
      }

      w.postMessage(msg)
    }

    if (running) {
      queue.push(run)
    }
    else {
      run()
    }
  })
}

export async function shutdownWorker(): Promise<void> {
  if (!worker)
    return

  const w = worker
  worker = null

  return new Promise<void>((resolve) => {
    const timeout = setTimeout(() => {
      w.terminate().then(() => resolve(), () => resolve())
    }, 5000)

    w.once('exit', () => {
      clearTimeout(timeout)
      resolve()
    })

    w.postMessage({ type: 'shutdown' } satisfies WorkerMessage)
  })
}
