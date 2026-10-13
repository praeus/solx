/**
 * Re-export @solx/types under the `solx/types` subpath so users
 * can `import { TypeManager } from 'solx/types'` for
 * tree-shaking.
 */
export { TypeManager } from '@solx/types';
export type { NativeTypesModule, TypeManagerHandle } from '@solx/types';
