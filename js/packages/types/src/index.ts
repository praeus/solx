/**
 * Re-export the public API of `@solx/surface` plus the
 * `TypeManager` class. Consumers can
 * `import { TypeManager, TypeEntity } from 'solx/types'`.
 *
 * The shared native-binary `loadNative` helper is also
 * re-exported so the other `@solx/*` packages (files, docs,
 * actions, ...) can reuse the same `solx.node` resolution
 * logic without duplicating it.
 *
 * @packageDocumentation
 */

export { TypeManager } from './native.js';
export { loadNative } from './loader.js';
export type { NativeTypesModule, TypeManagerHandle } from './native-types.js';
export type {
  TypeEntity,
  TypeInput,
  ListOptions,
  Page,
  JsonValue,
  JsonObject,
  SolxErrorKind,
} from '@solx/surface';
export { SolxError } from '@solx/surface';
