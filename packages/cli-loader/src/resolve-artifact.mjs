const nativePackages = new Map([
  ['darwin:arm64', '@skilld/cli-darwin-arm64'],
  ['darwin:x64', '@skilld/cli-darwin-x64'],
  ['linux:arm64:gnu', '@skilld/cli-linux-arm64-gnu'],
  ['linux:arm64:musl', '@skilld/cli-linux-arm64-musl'],
  ['linux:x64:gnu', '@skilld/cli-linux-x64-gnu'],
  ['linux:x64:musl', '@skilld/cli-linux-x64-musl'],
  ['win32:arm64', '@skilld/cli-win32-arm64-msvc'],
  ['win32:x64', '@skilld/cli-win32-x64-msvc'],
])

export function detectLibc(report = process.report) {
  const header = report?.getReport?.().header
  return header?.glibcVersionRuntime ? 'gnu' : 'musl'
}

export function nativePackage(runtime) {
  const key = runtime.platform === 'linux'
    ? `${runtime.platform}:${runtime.arch}:${runtime.libc}`
    : `${runtime.platform}:${runtime.arch}`
  return nativePackages.get(key)
}

export function selectArtifact(runtime, resolvePackage) {
  const packageName = nativePackage(runtime)
  if (packageName) {
    const executable = resolvePackage(packageName, runtime.platform === 'win32' ? 'bin/skilld.exe' : 'bin/skilld')
    if (executable)
      return { _tag: 'Native', executable }
  }

  return {
    _tag: 'Unavailable',
    message: 'UNSUPPORTED_HOST: no skilld CLI artifact is installed',
  }
}
