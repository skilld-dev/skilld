export type Result<T, E>
  = | { readonly _tag: 'Ok', readonly value: T }
    | { readonly _tag: 'Err', readonly error: E }

export const ok = <T>(value: T): Result<T, never> => ({ _tag: 'Ok', value })
export const err = <E>(error: E): Result<never, E> => ({ _tag: 'Err', error })
