import * as p from '@clack/prompts'
import { parseTree } from 'jsonc-parser'
import { join } from 'pathe'
import { editJsonProperty, patchPackageJson, readPackageJsonSafe } from '../core/package-json.ts'
import { isInteractive } from './env.ts'

export function hasPrepareHook(cwd: string = process.cwd()): boolean {
  const pkg = readPackageJsonSafe(join(cwd, 'package.json'))
  if (!pkg)
    return true
  const existing = (pkg.parsed.scripts as Record<string, unknown> | undefined)?.prepare
  return typeof existing === 'string' && existing.includes('skilld')
}

export async function suggestPrepareHook(cwd: string = process.cwd()): Promise<boolean> {
  const pkgJsonPath = join(cwd, 'package.json')
  const pkg = readPackageJsonSafe(pkgJsonPath)
  if (!pkg)
    return false

  const rawExisting = (pkg.parsed.scripts as Record<string, unknown> | undefined)?.prepare
  const existing: string | undefined = typeof rawExisting === 'string' ? rawExisting : undefined

  if (existing?.includes('skilld'))
    return true

  const prepareCmd = buildPrepareScript(existing, cwd)

  if (!isInteractive()) {
    p.log.info(
      `\x1B[90mAdd to package.json scripts:\n`
      + `  \x1B[36m"prepare": "${prepareCmd}"\x1B[0m\n`
      + `  \x1B[90mRestores references and shipped skills on install.\x1B[0m`,
    )
    return false
  }

  const confirmed = await p.confirm({
    message: `Add \x1B[36m"prepare": "${prepareCmd}"\x1B[0m to package.json?`,
    initialValue: true,
  })
  if (p.isCancel(confirmed) || !confirmed)
    return false

  patchPackageJson(pkgJsonPath, (content) => {
    const tree = parseTree(content)
    const hasScripts = tree?.children?.some(c =>
      c.type === 'property' && c.children?.[0]?.value === 'scripts',
    )

    let patched = content
    if (!hasScripts)
      patched = editJsonProperty(patched, ['scripts'], {})

    return editJsonProperty(patched, ['scripts', 'prepare'], prepareCmd)
  })
  p.log.success('Added \x1B[36mskilld prepare\x1B[0m to package.json')
  return true
}

export function buildPrepareScript(existing: string | undefined, cwd: string = process.cwd()): string {
  const bin = isNpxExecution() && !isSkilldDep(cwd) ? 'npx skilld' : 'skilld'
  const cmd = `${bin} prepare || true`
  if (!existing || !existing.trim())
    return cmd

  const trimmed = existing.trim()
  const cleaned = trimmed.replace(/[&|;]+\s*$/, '').trim()
  if (!cleaned)
    return cmd

  return `${cleaned} && (${cmd})`
}

function isNpxExecution(): boolean {
  if (process.env.npm_command === 'exec')
    return true
  const execPath = process.env._ || ''
  return /npx|\.store|dlx/.test(execPath)
}

function isSkilldDep(cwd: string): boolean {
  const pkg = readPackageJsonSafe(join(cwd, 'package.json'))
  if (!pkg)
    return false
  const deps = pkg.parsed as Record<string, any>
  return !!(deps.dependencies?.skilld || deps.devDependencies?.skilld)
}
