#!/usr/bin/env node
// Start a fixture server on a free port, fetch paths as HTML, then stop its whole process group.
//
// Usage: node serve-fixture.mjs [--fetch PATH]... [--out DIR] [--hold SECONDS] [--timeout SECONDS] -- COMMAND [ARG]...
//
// The script replaces `{port}` in each argument and sets PORT, NITRO_PORT, and NUXT_PORT.
// It prints `ready http://localhost:PORT` on stderr when the server answers.
// Each fetch prints `GET PATH STATUS CONTENT-TYPE BYTES` on stderr. The body goes to stdout, or to DIR when `--out` is set.
// Without `--fetch`, or with `--hold`, the server stays up until the hold time ends, the script gets SIGTERM, or its parent exits.
// The script never kills by port or by name. It stops only the process group it started. POSIX only.

import { spawn } from 'node:child_process'
import { mkdir, writeFile } from 'node:fs/promises'
import { createServer } from 'node:net'
import { join } from 'node:path'
import process from 'node:process'
import { setTimeout as delay } from 'node:timers/promises'

function usage(message) {
  process.stderr.write(`${message}\nUsage: node serve-fixture.mjs [--fetch PATH]... [--out DIR] [--hold SECONDS] [--timeout SECONDS] -- COMMAND [ARG]...\n`)
  process.exit(2)
}

function parseSeconds(flag, value) {
  const seconds = Number(value)
  if (!Number.isFinite(seconds) || seconds < 0)
    usage(`${flag} needs a number of seconds.`)
  return seconds
}

function parseArgs(argv) {
  const options = { fetch: [], out: undefined, hold: undefined, timeout: 120, command: [] }
  for (let index = 0; index < argv.length; index++) {
    const flag = argv[index]
    if (flag === '--') {
      options.command = argv.slice(index + 1)
      break
    }
    const value = argv[++index]
    if (value === undefined)
      usage(`${flag} needs a value.`)
    if (flag === '--fetch')
      options.fetch.push(value.startsWith('/') ? value : `/${value}`)
    else if (flag === '--out')
      options.out = value
    else if (flag === '--hold')
      options.hold = parseSeconds(flag, value)
    else if (flag === '--timeout')
      options.timeout = parseSeconds(flag, value)
    else
      usage(`Unknown option: ${flag}`)
  }
  if (options.command.length === 0)
    usage('Give the server command after --.')
  return options
}

function freePort() {
  return new Promise((resolve, reject) => {
    const server = createServer()
    server.once('error', reject)
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address()
      server.close(() => resolve(port))
    })
  })
}

const LEADING_SLASHES = /^\/+/
const UNSAFE_CHARACTERS = /[^\w.-]+/g

function fileName(path) {
  const name = path.replace(LEADING_SLASHES, '').replace(UNSAFE_CHARACTERS, '_')
  return `${name || 'index'}.html`
}

const options = parseArgs(process.argv.slice(2))
const port = await freePort()
const origin = `http://localhost:${port}`
const [command, ...args] = options.command.map(part => part.replaceAll('{port}', String(port)))

// `detached` makes the child a process group leader, so one signal reaches a package manager wrapper and its server.
const child = spawn(command, args, {
  detached: true,
  stdio: ['ignore', 2, 2],
  env: { ...process.env, PORT: String(port), NITRO_PORT: String(port), NUXT_PORT: String(port) },
})
let exited = false
const exit = new Promise((resolve) => {
  child.once('exit', (code, signal) => {
    exited = true
    resolve({ code, signal })
  })
  child.once('error', (error) => {
    exited = true
    process.stderr.write(`Could not start ${command}: ${error.message}\n`)
    resolve({ code: 127, signal: null })
  })
})

function signalGroup(signal) {
  try {
    process.kill(-child.pid, signal)
  }
  catch (error) {
    // ESRCH: the group has already exited, which is the goal.
    if (error.code !== 'ESRCH')
      throw error
  }
}

let stopping
function stop() {
  stopping ??= (async () => {
    if (child.pid === undefined)
      return
    signalGroup('SIGTERM')
    await Promise.race([exit, delay(5000)])
    // Kill any group member that ignored SIGTERM or outlived the leader.
    signalGroup('SIGKILL')
  })()
  return stopping
}

for (const signal of ['SIGINT', 'SIGTERM', 'SIGHUP']) {
  process.once(signal, () => {
    stop().then(() => process.exit(130))
  })
}

// A `node` shim, such as the pnpm one, can die on a signal and leave this script orphaned. Stop the server then too.
const parent = process.ppid
setInterval(() => {
  if (process.ppid !== parent)
    stop().then(() => process.exit(130))
}, 500).unref()

async function waitUntilReady(path) {
  const deadline = Date.now() + options.timeout * 1000
  while (Date.now() < deadline) {
    if (exited)
      return { _tag: 'Exited' }
    const response = await fetch(`${origin}${path}`, { headers: { accept: 'text/html' }, signal: AbortSignal.timeout(Math.max(deadline - Date.now(), 1)) })
      .catch(() => undefined) // Connection refused while the server starts. Retry until the deadline.
    if (response && ![502, 503, 504].includes(response.status)) {
      await response.body?.cancel()
      return { _tag: 'Ready' }
    }
    await response?.body?.cancel()
    await delay(500)
  }
  return { _tag: 'TimedOut' }
}

async function fetchPath(path) {
  const response = await fetch(`${origin}${path}`, { headers: { accept: 'text/html' }, signal: AbortSignal.timeout(options.timeout * 1000) })
  const body = await response.text()
  process.stderr.write(`GET ${path} ${response.status} ${response.headers.get('content-type') ?? '-'} ${Buffer.byteLength(body)}\n`)
  if (options.out) {
    await mkdir(options.out, { recursive: true })
    await writeFile(join(options.out, fileName(path)), body)
  }
  else {
    process.stdout.write(`${body}\n`)
  }
}

async function run() {
  const ready = await waitUntilReady(options.fetch[0] ?? '/')
  if (ready._tag === 'Exited') {
    const { code, signal } = await exit
    process.stderr.write(`The server exited before it answered: code ${code}, signal ${signal}.\n`)
    return 1
  }
  if (ready._tag === 'TimedOut') {
    process.stderr.write(`The server did not answer within ${options.timeout} seconds.\n`)
    return 1
  }
  process.stderr.write(`ready ${origin}\n`)
  let status = 0
  for (const path of options.fetch) {
    // A crashed route or an unwritable DIR fails one path. Report it and keep going.
    await fetchPath(path).catch((error) => {
      process.stderr.write(`GET ${path} failed: ${error.cause?.message ?? error.message}\n`)
      status = 1
    })
  }
  if (options.fetch.length === 0 || options.hold !== undefined)
    await Promise.race([exit, options.hold === undefined ? new Promise(() => {}) : delay(options.hold * 1000)])
  return status
}

// Stop the group on any failure, so an error never orphans the server.
const status = await run().catch((error) => {
  process.stderr.write(`${error.stack ?? error}\n`)
  return 1
})
await stop()
process.exit(status)
