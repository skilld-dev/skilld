import { readFile, writeFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

/**
 * Vendors the skilld.dev public API contract for the Rust contract test.
 *
 * Give a path to copy a local `openapi.v1.json`, such as the one
 * `packages/sdk` in the skilld.dev repository generates. Without a path, the
 * script downloads `/api/v1/openapi.json` from `SKILLD_API_URL`, or from
 * https://skilld.dev when that variable is unset.
 *
 *   node scripts/sync-api-spec.mjs ../skilld.dev/packages/sdk/generated/openapi.v1.json
 *   node scripts/sync-api-spec.mjs
 */
export const TARGET = fileURLToPath(new URL('../packages/protocol/openapi/skilld-api-v1.json', import.meta.url))

function compareStrings(left, right) {
  return left < right ? -1 : left > right ? 1 : 0
}

function orderedJson(value) {
  if (Array.isArray(value))
    return value.map(orderedJson)
  if (value === null || typeof value !== 'object')
    return value
  return Object.fromEntries(
    Object.entries(value)
      .sort(([left], [right]) => compareStrings(left, right))
      .map(([key, item]) => [key, orderedJson(item)]),
  )
}

/**
 * Parse one OpenAPI document and serialize it the way skilld.dev commits it:
 * sorted keys, two spaces, one trailing newline. The file then changes only
 * when the contract does.
 */
export function serializeApiSpec(text) {
  const document = JSON.parse(text)
  if (typeof document.openapi !== 'string' || !document.openapi.startsWith('3.1.'))
    throw new Error('The API contract must be an OpenAPI 3.1 document.')
  if (document.paths === null || typeof document.paths !== 'object' || Object.keys(document.paths).length === 0)
    throw new Error('The API contract has no paths.')
  return `${JSON.stringify(orderedJson(document), null, 2)}\n`
}

async function readSource(source) {
  if (source)
    return readFile(source, 'utf8')
  const origin = process.env.SKILLD_API_URL || 'https://skilld.dev'
  const url = new URL('/api/v1/openapi.json', origin)
  const response = await fetch(url, { headers: { accept: 'application/json' } })
  if (!response.ok)
    throw new Error(`${url} answered HTTP ${response.status}.`)
  return response.text()
}

async function main(args) {
  const text = await readSource(args[0])
  await writeFile(TARGET, serializeApiSpec(text), 'utf8')
  process.stdout.write(`Wrote ${TARGET}.\nRun cargo test -p skilld-core --test api_contract to check the Rust types.\n`)
}

if (resolve(process.argv[1] ?? '') === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).catch((error) => {
    process.stderr.write(`${error.message}\n`)
    process.exitCode = 1
  })
}
