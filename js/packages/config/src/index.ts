/**
 * Re-export the public API of `@solx/surface` plus the
 * `ConfigService` class. Consumers can
 * `import { ConfigService } from 'solx/config'`.
 *
 * @packageDocumentation
 */

export { ConfigService } from './native.js';
export type { NativeConfigModule, ConfigServiceHandle } from './native-types.js';
export type { SolxConfig, InstalledPackage } from '@solx/surface';
export { SolxError } from '@solx/surface';