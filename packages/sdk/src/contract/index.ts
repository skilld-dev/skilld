import { accountV1, changesV1, likesV1, starsV1, tokensV1, watchesV1 } from './account'
import { collectionsV1, curatorsV1 } from './collections'
import { defineProtocol, SKILLD_V1_VERSION } from './core'
import { indexRequestsV1, ownersV1, repositoriesV1, tracksV1, trendingV1 } from './registry'
import { skillsV1 } from './skills'

export * from './account'
export * from './collections'
export * from './core'
export * from './registry'
export * from './schemas'
export * from './skills'

export const skilldV1Protocol = defineProtocol({
  version: SKILLD_V1_VERSION,
  registries: {
    skills: skillsV1,
    repositories: repositoriesV1,
    indexRequests: indexRequestsV1,
    owners: ownersV1,
    tracks: tracksV1,
    trending: trendingV1,
    curators: curatorsV1,
    collections: collectionsV1,
    account: accountV1,
    likes: likesV1,
    watches: watchesV1,
    stars: starsV1,
    changes: changesV1,
    tokens: tokensV1,
  },
})

export type SkilldV1Protocol = typeof skilldV1Protocol
