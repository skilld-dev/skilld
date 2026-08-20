import { isAbsolute, posix, relative, resolve, sep } from 'node:path'

export function isSkillName(value: string): boolean {
  return value.length >= 1
    && value.length <= 64
    && /^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(value)
}

export function resolveWithin(root: string, candidate: string): string | null {
  const absoluteRoot = resolve(root)
  const absoluteCandidate = resolve(root, candidate)
  const fromRoot = relative(absoluteRoot, absoluteCandidate)
  if (fromRoot === '' || (!fromRoot.startsWith(`..${sep}`) && fromRoot !== '..' && !isAbsolute(fromRoot)))
    return absoluteCandidate
  return null
}

export function normalizeOutputPath(value: string): string | null {
  if (value.length === 0 || value.includes('\\') || value.includes('\0') || posix.isAbsolute(value))
    return null
  const normalized = posix.normalize(value)
  if (normalized === '.' || normalized === '..' || normalized.startsWith('../') || normalized !== value)
    return null
  return normalized
}
