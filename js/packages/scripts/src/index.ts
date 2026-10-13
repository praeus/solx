/**
 * Re-export the public API of `@solx/surface` plus the
 * `ScriptRunner` class. Consumers can
 * `import { ScriptRunner } from 'solx/scripts'`.
 *
 * @packageDocumentation
 */

export { ScriptRunner } from './native.js';
export type { NativeScriptsModule } from './native-types.js';
export type { JsonValue, SolxErrorKind } from '@solx/surface';
export { SolxError } from '@solx/surface';
