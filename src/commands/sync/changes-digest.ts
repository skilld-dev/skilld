import { loadSession, peekMarker, updateMarker } from '../../auth/store.ts'
import { renderDigest } from '../../cli/digest-render.ts'
import { createRegistryClient } from '../../registry/client.ts'

export async function renderChangesDigest(showEmpty = false): Promise<'auth-required' | 'shown' | 'empty'> {
  const session = await loadSession()
  if (!session)
    return 'auth-required'
  const marker = peekMarker()
  const client = createRegistryClient()
  const digest = await client.my.changes({ since: marker?.lastDigestAt })
  if (digest.entries.length > 0 || showEmpty)
    renderDigest(digest.entries)
  updateMarker({ lastDigestAt: digest.windowEnd })
  return digest.entries.length > 0 ? 'shown' : 'empty'
}
