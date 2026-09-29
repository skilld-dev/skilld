#!/usr/bin/env node
// Start a fixture server on a free port, fetch paths, then stop its whole process group.
//
// Usage: node serve-fixture.mjs [--fetch PATH]... [--fetch-raw PATH]... [--out DIR] [--hold SECONDS] [--timeout SECONDS] -- COMMAND [ARG]...
//
// The script replaces `{port}` in each argument and sets PORT, NITRO_PORT, and NUXT_PORT.
// It prints `ready http://localhost:PORT` on stderr when the server answers.
// `--fetch` asks for HTML. `--fetch-raw` asks for any type, such as an image, and needs `--out`.
// Each fetch prints `GET PATH STATUS CONTENT-TYPE BYTES` on stderr. The body goes to stdout, or to DIR when `--out` is set.
// With `--out`, DIR/responses.json lists each path with its file, status, content type, and byte count.
// Without `--fetch`, or with `--hold`, the server stays up until the hold time ends, the script gets SIGTERM, or its parent exits.
// The script never kills by port or by name. It stops only the process group it started. POSIX only.

import { spawn } from 'node:child_process'
import { mkdir, writeFile } from 'node:fs/promises'
import { createServer } from 'node:net'
import { join } from 'node:path'
import process from 'node:process'
import { setTimeout as delay } from 'node:timers/promises'

function usage(message) {
  process.stderr.write(`${message}\nUsage: node serve-fixture.mjs [--fetch PATH]... [--fetch-raw PATH]... [--out DIR] [--hold SECONDS] [--timeout SECONDS] -- COMMAND [ARG]...\n`)
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
    if (flag === '--fetch' || flag === '--fetch-raw')
      options.fetch.push({ path: value.startsWith('/') ? value : `/${value}`, raw: flag === '--fetch-raw' })
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
  if (options.fetch.some(request => request.raw) && options.out === undefined)
    usage('--fetch-raw needs --out, because a binary body cannot go to stdout.')
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

const NOT_EXTENSION = /[^a-z0-9]+/g

function extension(contentType) {
  // `image/svg+xml; charset=utf-8` becomes `svg`. An unknown type becomes `bin`.
  const subtype = contentType?.split(';')[0].split('/')[1]?.split('+')[0].toLowerCase().replace(NOT_EXTENSION, '')
  return subtype || 'bin'
}

// encodeURIComponent maps each path to one name, so `/a/b` and `/a_b` never share a file.
// It always escapes `#`, so the `#N` suffix for a repeated name never matches a path.
const usedNames = new Set()
function fileName(path, contentType) {
  const base = `${encodeURIComponent(path)}.${extension(contentType)}`
  let name = base
  for (let copy = 2; usedNames.has(name); copy++)
    name = `${base.slice(0, base.lastIndexOf('.'))}#${copy}.${extension(contentType)}`
  usedNames.add(name)
  return name
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

function kill() {
  if (child.pid !== undefined)
    signalGroup('SIGKILL')
}

let stopping
function stop() {
  stopping ??= (async () => {
    if (child.pid === undefined)
      return
    signalGroup('SIGTERM')
    await Promise.race([exit, delay(5000)])
    // Kill any group member that ignored SIGTERM or outlived the leader.
    kill()
  })()
  return stopping
}

// Keep the handlers after the first signal. Otherwise a second signal ends this script before the final kill.
let signalled = false
for (const signal of ['SIGINT', 'SIGTERM', 'SIGHUP']) {
  process.on(signal, () => {
    if (signalled) {
      kill()
      process.exit(130)
    }
    signalled = true
    stop().then(() => process.exit(130))
  })
}

// A `node` shim, such as the pnpm one, can die on a signal and leave this script orphaned. Stop the server then too.
const parent = process.ppid
setInterval(() => {
  if (process.ppid !== parent)
    stop().then(() => process.exit(130))
}, 500).unref()

async function waitUntilReady({ path, raw }) {
  const deadline = Date.now() + options.timeout * 1000
  while (Date.now() < deadline) {
    if (exited)
      return { _tag: 'Exited' }
    const response = await fetch(`${origin}${path}`, { headers: { accept: raw ? '*/*' : 'text/html' }, signal: AbortSignal.timeout(Math.max(deadline - Date.now(), 1)) })
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

const responses = []

async function fetchPath({ path, raw }) {
  const response = await fetch(`${origin}${path}`, { headers: { accept: raw ? '*/*' : 'text/html' }, signal: AbortSignal.timeout(options.timeout * 1000) })
  const body = Buffer.from(await response.arrayBuffer())
  const contentType = response.headers.get('content-type')
  process.stderr.write(`GET ${path} ${response.status} ${contentType ?? '-'} ${body.byteLength}\n`)
  if (options.out) {
    const file = fileName(path, contentType)
    await mkdir(options.out, { recursive: true })
    await writeFile(join(options.out, file), body)
    responses.push({ path, file, status: response.status, contentType, bytes: body.byteLength })
    await writeFile(join(options.out, 'responses.json'), `${JSON.stringify(responses, null, 2)}\n`)
  }
  else {
    process.stdout.write(`${body}\n`)
  }
}

async function run() {
  const ready = await waitUntilReady(options.fetch[0] ?? { path: '/', raw: false })
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
  for (const request of options.fetch) {
    // A crashed route or an unwritable DIR fails one path. Report it and keep going.
    await fetchPath(request).catch((error) => {
      process.stderr.write(`GET ${request.path} failed: ${error.cause?.message ?? error.message}\n`)
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
