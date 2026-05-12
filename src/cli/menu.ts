import * as p from '@clack/prompts'

export class MenuCancel extends Error { override name = 'MenuCancel' }

export function guard<T>(value: T | symbol): T {
  if (p.isCancel(value))
    throw new MenuCancel()
  return value as T
}

export interface MenuOption {
  label: string
  value: string
  hint?: string
}

export async function menuLoop(opts: {
  message: string
  options: () => MenuOption[] | Promise<MenuOption[]>
  onSelect: (value: string) => Promise<boolean | void>
  initialValue?: string | (() => string | undefined)
  searchable?: boolean
}): Promise<void> {
  while (true) {
    const options = await opts.options()
    const initial = typeof opts.initialValue === 'function' ? opts.initialValue() : opts.initialValue
    const choice = opts.searchable
      ? await p.autocomplete({ message: opts.message, options, ...(initial != null ? { initialValue: initial } : {}) })
      : await p.select({ message: opts.message, options, ...(initial != null ? { initialValue: initial } : {}) })
    if (p.isCancel(choice))
      return
    try {
      if (await opts.onSelect(choice as string))
        return
    }
    catch (err) {
      if (err instanceof MenuCancel)
        continue
      throw err
    }
  }
}
