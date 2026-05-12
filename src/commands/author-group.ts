import { defineCommand } from 'citty'

export const authorGroupDef = defineCommand({
  meta: { name: 'author', description: 'Create, generate, and publish skills' },
  subCommands: {
    package: () => import('./author.ts').then(m => m.authorCommandDef),
    publish: () => import('./upload.ts').then(m => m.uploadCommandDef),
    eject: () => import('./sync/eject.ts').then(m => m.ejectCommandDef),
    validate: () => import('./validate.ts').then(m => m.validateCommandDef),
    assemble: () => import('./assemble.ts').then(m => m.assembleCommandDef),
  },
})
