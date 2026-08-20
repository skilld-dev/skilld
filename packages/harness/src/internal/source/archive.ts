import type { SkillOutputPolicy, SkillRunError } from '../../types.ts'
import type { Result } from '../result.ts'
import type { PreparedFile } from './host.ts'
import { Readable } from 'node:stream'
import { createGunzip } from 'node:zlib'
import { normalizeOutputPath } from '../paths.ts'
import { err, ok } from '../result.ts'

const blockBytes = 512

export type ArchiveLayout = 'npm' | 'github'

interface PendingEntry {
  readonly kind: 'directory' | 'file' | 'metadata' | 'skip'
  readonly path?: string
  readonly metadataType?: 'long-path' | 'pax'
  readonly size: number
  remaining: number
  padding: number
  readonly chunks: Array<Buffer<ArrayBufferLike>>
}

function unavailable(message: string, cause?: unknown): Result<never, SkillRunError> {
  return err({ _tag: 'SourceUnavailable', message, attempts: [], cause })
}

function readString(buffer: Uint8Array, start: number, length: number): string {
  const bytes = buffer.subarray(start, start + length)
  const zero = bytes.indexOf(0)
  return Buffer.from(zero === -1 ? bytes : bytes.subarray(0, zero)).toString('utf8').trim()
}

function readOctal(buffer: Uint8Array, start: number, length: number): number | null {
  const value = readString(buffer, start, length).replace(/^\s+|\s+$/g, '')
  if (!/^[0-7]+$/.test(value))
    return null
  const parsed = Number.parseInt(value, 8)
  return Number.isSafeInteger(parsed) ? parsed : null
}

function hasValidChecksum(header: Uint8Array): boolean {
  const expected = readOctal(header, 148, 8)
  if (expected === null)
    return false
  let actual = 0
  for (let index = 0; index < header.byteLength; index += 1)
    actual += index >= 148 && index < 156 ? 32 : header[index]!
  return actual === expected
}

function parsePaxPath(content: Uint8Array): string | null {
  const text = Buffer.from(content).toString('utf8')
  let offset = 0
  let path: string | null = null
  while (offset < text.length) {
    const space = text.indexOf(' ', offset)
    if (space === -1)
      return null
    const length = Number.parseInt(text.slice(offset, space), 10)
    if (!Number.isSafeInteger(length) || length <= 0 || offset + length > text.length)
      return null
    const record = text.slice(space + 1, offset + length - 1)
    const equals = record.indexOf('=')
    if (equals !== -1 && record.slice(0, equals) === 'path')
      path = record.slice(equals + 1)
    offset += length
  }
  return path
}

function archivePath(value: string, layout: ArchiveLayout): string | null {
  const normalized = normalizeOutputPath(value)
  if (normalized === null)
    return null
  const segments = normalized.split('/')
  if (layout === 'npm') {
    if (segments[0] !== 'package' || segments.length < 2)
      return null
  }
  else if (segments.length < 2) {
    return null
  }
  return normalizeOutputPath(segments.slice(1).join('/'))
}

function entryContent(entry: PendingEntry): Buffer<ArrayBufferLike> {
  return entry.chunks.length === 1
    ? entry.chunks[0]!
    : Buffer.concat(entry.chunks, entry.size)
}

export async function extractArchive(
  compressed: AsyncIterable<Uint8Array>,
  policy: SkillOutputPolicy,
  layout: ArchiveLayout,
): Promise<Result<ReadonlyArray<PreparedFile>, SkillRunError>> {
  const source = Readable.from(compressed)
  const gunzip = createGunzip()
  source.on('error', error => gunzip.destroy(error))
  source.pipe(gunzip)
  const files: PreparedFile[] = []
  const seen = new Set<string>()
  const maxArchiveBytes = policy.maxSourceBytes + (policy.maxSourceFiles + 2) * blockBytes * 2
  let archiveBytes = 0
  let fileEntries = 0
  let nextPath: string | null = null
  let pending: Buffer<ArrayBufferLike> = Buffer.alloc(0)
  let entry: PendingEntry | null = null
  let totalBytes = 0
  let ended = false

  try {
    for await (const value of gunzip) {
      const chunk = Buffer.isBuffer(value) ? value : Buffer.from(value as Uint8Array)
      archiveBytes += chunk.byteLength
      if (archiveBytes > maxArchiveBytes)
        return unavailable('Source archive exceeds the uncompressed byte limit.')
      pending = pending.byteLength === 0 ? chunk : Buffer.concat([pending, chunk])

      while (pending.byteLength > 0) {
        if (ended) {
          if (pending.some(byte => byte !== 0))
            return unavailable('Source archive has data after its end marker.')
          pending = Buffer.alloc(0)
          break
        }

        if (entry !== null) {
          if (entry.remaining > 0) {
            const consumed = Math.min(entry.remaining, pending.byteLength)
            if (entry.kind === 'file' || entry.kind === 'metadata')
              entry.chunks.push(pending.subarray(0, consumed))
            pending = pending.subarray(consumed)
            entry.remaining -= consumed
            if (entry.remaining > 0)
              break
          }
          if (entry.padding > 0) {
            const consumed = Math.min(entry.padding, pending.byteLength)
            pending = pending.subarray(consumed)
            entry.padding -= consumed
            if (entry.padding > 0)
              break
          }

          if (entry.kind === 'metadata') {
            const content = entryContent(entry)
            nextPath = entry.metadataType === 'pax'
              ? parsePaxPath(content)
              : content.toString('utf8').replace(/\0.*$/s, '').trim()
            if (nextPath === null || nextPath.length === 0)
              return unavailable('Source archive has invalid path metadata.')
          }
          else if (entry.kind === 'file' && entry.path) {
            files.push({ path: entry.path, content: Uint8Array.from(entryContent(entry)) })
            totalBytes += entry.size
          }
          entry = null
          continue
        }

        if (pending.byteLength < blockBytes)
          break
        const header = pending.subarray(0, blockBytes)
        pending = pending.subarray(blockBytes)
        if (header.every(byte => byte === 0)) {
          ended = true
          continue
        }
        if (!hasValidChecksum(header))
          return unavailable('Source archive has an invalid header checksum.')

        const size = readOctal(header, 124, 12)
        if (size === null)
          return unavailable('Source archive has an invalid entry size.')
        const name = readString(header, 0, 100)
        const prefix = readString(header, 345, 155)
        const headerPath = prefix.length > 0 ? `${prefix}/${name}` : name
        const type = String.fromCharCode(header[156] ?? 0)
        const padding = (blockBytes - size % blockBytes) % blockBytes

        if (type === 'x' || type === 'L') {
          if (size > policy.maxSourceFileBytes)
            return unavailable('Source archive path metadata exceeds the file byte limit.')
          entry = {
            kind: 'metadata',
            metadataType: type === 'x' ? 'pax' : 'long-path',
            size,
            remaining: size,
            padding,
            chunks: [],
          }
          continue
        }
        if (type === '5') {
          nextPath = null
          entry = { kind: 'directory', size, remaining: size, padding, chunks: [] }
          continue
        }
        if (type !== '0' && type !== '\0')
          return unavailable('Source archive contains a linked or special entry.')

        const path = archivePath(nextPath ?? headerPath, layout)
        nextPath = null
        if (path === null)
          return unavailable('Source archive contains an invalid path.')
        if (seen.has(path))
          return unavailable('Source archive contains duplicate paths.')
        seen.add(path)
        fileEntries += 1
        if (fileEntries > policy.maxSourceFiles)
          return unavailable('Source archive contains too many files.')
        if (size <= policy.maxSourceFileBytes && totalBytes + size > policy.maxSourceBytes)
          return unavailable('Source archive exceeds the total byte limit.')

        entry = {
          kind: size > policy.maxSourceFileBytes ? 'skip' : 'file',
          path,
          size,
          remaining: size,
          padding,
          chunks: [],
        }
      }
    }
  }
  catch (cause) {
    const message = cause instanceof Error ? cause.message : ''
    return unavailable(message.startsWith('Source archive ')
      ? message
      : 'Source archive is not valid gzip data.', cause)
  }

  if (entry !== null || pending.some(byte => byte !== 0) || !ended)
    return unavailable('Source archive ended before its final entry.')
  if (files.length === 0)
    return unavailable('Source archive has no usable files.')
  files.sort((left, right) => left.path.localeCompare(right.path))
  return ok(files)
}
