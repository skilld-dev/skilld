import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('node:fs', async () => {
  const actual = await vi.importActual<typeof import('node:fs')>('node:fs')
  return {
    ...actual,
    existsSync: vi.fn(),
    lstatSync: vi.fn(),
    mkdirSync: vi.fn(),
    symlinkSync: vi.fn(),
    unlinkSync: vi.fn(),
  }
})

vi.mock('../../src/cache/internal/version', () => ({
  getCacheDir: (name: string, version: string) => `/home/.skilld/references/${name}@${version}`,
}))

describe('restorePkgSymlink', () => {
  beforeEach(() => vi.resetAllMocks())
  afterEach(() => vi.restoreAllMocks())

  it('skips when skill directory does not exist', async () => {
    const fs = await import('node:fs')
    const { restorePkgSymlink } = await import('../../src/core/prepare')

    vi.mocked(fs.existsSync).mockImplementation((p) => {
      // skill dir doesn't exist — triggers early return
      if (String(p).endsWith('/vue'))
        return false
      return true
    })

    restorePkgSymlink('/project/.skills', 'vue', { version: '3.4.0' }, '/project')

    expect(fs.symlinkSync).not.toHaveBeenCalled()
  })

  it('skips when pkg symlink target is valid', async () => {
    const fs = await import('node:fs')
    const { restorePkgSymlink } = await import('../../src/core/prepare')

    vi.mocked(fs.existsSync).mockReturnValue(true)
    vi.mocked(fs.lstatSync).mockReturnValue({ isSymbolicLink: () => true } as any)

    restorePkgSymlink('/project/.skills', 'vue', { version: '3.4.0' }, '/project')

    expect(fs.symlinkSync).not.toHaveBeenCalled()
  })

  it('removes dangling symlink before re-creating', async () => {
    const fs = await import('node:fs')
    const { restorePkgSymlink } = await import('../../src/core/prepare')

    vi.mocked(fs.existsSync).mockImplementation((p) => {
      const path = String(p)
      // skill dir exists
      if (path === '/project/.skills/vue')
        return true
      // dangling symlink: existsSync returns false (follows symlink, target gone)
      if (path.endsWith('/pkg'))
        return false
      // node_modules/vue exists (freshly installed)
      if (path.includes('node_modules/vue'))
        return true
      return false
    })
    // lstatSync succeeds — the symlink itself exists on disk
    vi.mocked(fs.lstatSync).mockReturnValue({ isSymbolicLink: () => true } as any)

    restorePkgSymlink('/project/.skills', 'vue', { version: '3.4.0' }, '/project')

    // Should remove the dangling symlink first
    expect(fs.unlinkSync).toHaveBeenCalledWith(
      expect.stringContaining('pkg'),
    )
    // Then create a fresh symlink
    expect(fs.symlinkSync).toHaveBeenCalledOnce()
  })

  it('creates symlink when no prior link exists', async () => {
    const fs = await import('node:fs')
    const { restorePkgSymlink } = await import('../../src/core/prepare')

    vi.mocked(fs.existsSync).mockImplementation((p) => {
      const path = String(p)
      if (path === '/project/.skills/vue')
        return true
      if (path.includes('node_modules/vue'))
        return true
      return false
    })
    // lstatSync throws ENOENT — no file at all
    vi.mocked(fs.lstatSync).mockImplementation(() => {
      const err = new Error('ENOENT') as NodeJS.ErrnoException
      err.code = 'ENOENT'
      throw err
    })

    restorePkgSymlink('/project/.skills', 'vue', { version: '3.4.0' }, '/project')

    expect(fs.unlinkSync).not.toHaveBeenCalled()
    expect(fs.symlinkSync).toHaveBeenCalledOnce()
  })

  it('skips when real file exists at pkg path', async () => {
    const fs = await import('node:fs')
    const { restorePkgSymlink } = await import('../../src/core/prepare')

    vi.mocked(fs.existsSync).mockReturnValue(true)
    // lstatSync returns a regular file, not a symlink
    vi.mocked(fs.lstatSync).mockReturnValue({ isSymbolicLink: () => false } as any)

    restorePkgSymlink('/project/.skills', 'vue', { version: '3.4.0' }, '/project')

    expect(fs.symlinkSync).not.toHaveBeenCalled()
  })
})
