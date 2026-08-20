import { _setPreopens } from '@bytecodealliance/preview2-shim/filesystem'

async function main() {
  _setPreopens({ '/': process.cwd() })

  const componentUrl = process.env.SKILLD_WASI_COMPONENT
    ? new URL(process.env.SKILLD_WASI_COMPONENT, `file://${process.cwd()}/`)
    : new URL('./component/skilld-wasi.js', import.meta.url)

  const component = await import(componentUrl.href)
  process.exitCode = component.run()
}

main().catch((error) => {
  process.stderr.write(`SERVICE_UNAVAILABLE: ${error instanceof Error ? error.message : String(error)}\n`)
  process.exitCode = 2
})
