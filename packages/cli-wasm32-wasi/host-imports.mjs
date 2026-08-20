import { spawnSync } from 'node:child_process'

const credentialProof = new Map()

const ok = value => ({ tag: 'ok', val: value })
const err = code => ({ tag: 'err', val: code })

export const credentials = {
  get(service, account) {
    if (globalThis.process.env.SKILLD_WASI_MEMORY_CREDENTIAL !== '1')
      return err('unsupported-host')

    return ok(credentialProof.get(`${service}\0${account}`) ?? 'proof-token')
  },

  set(service, account, secret) {
    if (globalThis.process.env.SKILLD_WASI_MEMORY_CREDENTIAL !== '1')
      return err('unsupported-host')

    credentialProof.set(`${service}\0${account}`, secret)
    return ok(undefined)
  },

  delete(service, account) {
    if (globalThis.process.env.SKILLD_WASI_MEMORY_CREDENTIAL !== '1')
      return err('unsupported-host')

    return ok(credentialProof.delete(`${service}\0${account}`))
  },
}

export const process = {
  runGit(command) {
    if (globalThis.process.env.SKILLD_WASI_ENABLE_GIT !== '1')
      return err('unsupported-host')

    if (!Array.isArray(command.args) || !command.args.every(arg => typeof arg === 'string'))
      return err('invalid')

    const result = spawnSync('git', command.args, {
      cwd: command.cwd ?? undefined,
      input: command.stdin ? Buffer.from(command.stdin) : undefined,
      maxBuffer: Number(command.maxOutputBytes),
      shell: false,
      timeout: Number(command.timeoutMs),
    })
    if (result.error?.code === 'ENOENT')
      return err('not-found')
    if (result.error?.code === 'ETIMEDOUT')
      return err('timed-out')
    if (result.error)
      return err('io')

    return ok({
      exitCode: result.status ?? 1,
      stdout: new Uint8Array(result.stdout),
      stderr: new Uint8Array(result.stderr),
    })
  },

  openUrl() {
    return err('unsupported-host')
  },
}
