/**
 * RFC 7636 PKCE loopback flow.
 *
 * Binds `127.0.0.1:<port>` and `[::1]:<port>` simultaneously so the browser
 * can hit either; opens the system browser to the verification URL; serves a
 * single GET callback that captures the auth code, then exchanges it for
 * tokens against `/api/cli/oauth/token`.
 */

import type { AddressInfo } from 'node:net'
import type { TokenResponse } from './types.ts'
import { spawn } from 'node:child_process'
import { createHash, randomBytes } from 'node:crypto'
import { createServer } from 'node:http'
import { ofetch } from 'ofetch'

const SUCCESS_HTML = `<!doctype html><meta charset="utf-8"><title>skilld — signed in</title>
<body style="font-family: ui-sans-serif, system-ui; padding: 4rem; text-align: center">
<h1>Signed in to skilld</h1><p>You can close this tab and return to the CLI.</p>`

function ERROR_HTML(msg: string) {
  return `<!doctype html><meta charset="utf-8"><title>skilld — error</title>
<body style="font-family: ui-sans-serif, system-ui; padding: 4rem; text-align: center; color: #b00">
<h1>Sign-in failed</h1><p>${msg}</p>`
}

export interface PkceFlowOptions {
  registryBase: string
  cliVersion: string
  openBrowser?: (url: string) => Promise<void> | void
  timeoutMs?: number
}

const PLUS_RE = /\+/g
const SLASH_RE = /\//g
const EQ_RE = /=+$/
const TRAILING_SLASH_RE = /\/$/
const TRAILING_API_RE = /\/api$/

function base64url(buf: Buffer): string {
  return buf.toString('base64').replace(PLUS_RE, '-').replace(SLASH_RE, '_').replace(EQ_RE, '')
}

function generateVerifier(): string {
  return base64url(randomBytes(32))
}

function challengeFromVerifier(verifier: string): string {
  return base64url(createHash('sha256').update(verifier).digest())
}

export async function runPkceFlow(opts: PkceFlowOptions): Promise<TokenResponse> {
  const verifier = generateVerifier()
  const challenge = challengeFromVerifier(verifier)
  const state = base64url(randomBytes(16))

  const { port, server, gotCode } = await bindLoopback(state)
  const verificationUrl = new URL(`${opts.registryBase.replace(TRAILING_SLASH_RE, '').replace(TRAILING_API_RE, '')}/cli/authorize`)
  verificationUrl.searchParams.set('challenge', challenge)
  verificationUrl.searchParams.set('port', String(port))
  verificationUrl.searchParams.set('state', state)
  verificationUrl.searchParams.set('v', opts.cliVersion)

  await (opts.openBrowser ?? defaultOpenBrowser)(verificationUrl.toString())

  try {
    const code = await Promise.race([
      gotCode,
      new Promise<never>((_, reject) => setTimeout(() => reject(new Error('PKCE flow timed out')), opts.timeoutMs ?? 5 * 60_000)),
    ])

    return await ofetch<TokenResponse>(`${opts.registryBase}/cli/oauth/token`, {
      method: 'POST',
      body: { code, code_verifier: verifier, redirect_uri: `http://127.0.0.1:${port}/` },
    })
  }
  finally {
    server.close()
  }
}

interface LoopbackBinding {
  port: number
  server: { close: () => void }
  gotCode: Promise<string>
}

async function bindLoopback(expectedState: string): Promise<LoopbackBinding> {
  let resolveCode!: (code: string) => void
  let rejectCode!: (err: Error) => void
  const gotCode = new Promise<string>((res, rej) => {
    resolveCode = res
    rejectCode = rej
  })

  const handler = (req: import('node:http').IncomingMessage, res: import('node:http').ServerResponse): void => {
    const url = new URL(req.url ?? '/', 'http://localhost')
    const code = url.searchParams.get('code')
    const state = url.searchParams.get('state')
    if (!code || state !== expectedState) {
      res.writeHead(400, { 'content-type': 'text/html' }).end(ERROR_HTML('Missing or invalid state parameter.'))
      rejectCode(new Error('PKCE callback missing code or state mismatch'))
      return
    }
    res.writeHead(200, { 'content-type': 'text/html' }).end(SUCCESS_HTML)
    resolveCode(code)
  }

  const v4 = createServer(handler)
  const v6 = createServer(handler)

  await new Promise<void>((resolve, reject) => {
    v4.once('error', reject).listen(0, '127.0.0.1', () => resolve())
  })
  const port = (v4.address() as AddressInfo).port
  await new Promise<void>((resolve) => {
    v6.once('error', () => resolve()).listen(port, '::1', () => resolve())
  })

  const close = (): void => {
    // closeAllConnections() forces still-open keep-alive sockets shut so the
    // process can exit promptly after the browser hits the success page.
    v4.closeAllConnections()
    v6.closeAllConnections()
    v4.close()
    v6.close()
  }

  return {
    port,
    server: { close },
    gotCode,
  }
}

function defaultOpenBrowser(url: string): void {
  const cmd = process.platform === 'darwin'
    ? 'open'
    : process.platform === 'win32'
      ? 'start'
      : 'xdg-open'
  spawn(cmd, [url], { stdio: 'ignore', detached: true }).unref()
}
