/**
 * Re-export the public API of `@solx/surface` plus the
 * `FileStore` class and the `FilePath` builder helpers.
 * Consumers can `import { FileStore, FilePath } from 'solx/files'`.
 *
 * @packageDocumentation
 */

export { FileStore, FilePath } from './native.js';
export type { NativeFilesModule, FileStoreHandle } from './native-types.js';
export { SolxError } from '@solx/surface';
