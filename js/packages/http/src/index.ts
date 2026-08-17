/**
 * @solx/http — pure-`fetch` implementation of the `@solx/surface` manager
 * interfaces, talking directly to `solx-server` over HTTP. Zero runtime
 * dependencies beyond `@solx/surface`, no native code — works unmodified in
 * a browser or in Node.
 *
 * This is the browser-capable counterpart to the Neon-backed managers'
 * `.connect(serverUrl, token)` mode (`@solx/types`, `@solx/files`, etc.),
 * which requires a native `.node` binary and therefore only runs under
 * Node. The Neon embed path (local *or* remote via `solx/sdk`'s
 * `createSolx`) is unaffected by this package and remains the right choice
 * for any Node-side tool that wants solx-core in-process.
 *
 * `config`/`scripts` have no member here — neither has an HTTP surface on
 * `solx-server` (config is local-only by design; scripts has no Rust trait
 * at all). See `solx-js/packages/sdk/src/create.ts`.
 *
 * @packageDocumentation
 */
import { HttpActionManager } from './actions.js';
import { HttpDocManager } from './docs.js';
import { HttpFileStore } from './files.js';
import { HttpTypeManager } from './types.js';

export { HttpActionManager } from './actions.js';
export { HttpDocManager } from './docs.js';
export { HttpFileStore } from './files.js';
export { HttpTypeManager } from './types.js';

export interface HttpSolx {
  types: HttpTypeManager;
  files: HttpFileStore;
  docs: HttpDocManager;
  actions: HttpActionManager;
}

/**
 * Connect to a `solx-server` over HTTP. `baseUrl` should not have a
 * trailing slash (e.g. `"http://127.0.0.1:8766"`), matching
 * `solx-client`'s `RemoteXManager::new` convention.
 */
export function connectHttp(baseUrl: string, token: string): HttpSolx {
  return {
    types: new HttpTypeManager(baseUrl, token),
    files: new HttpFileStore(baseUrl, token),
    docs: new HttpDocManager(baseUrl, token),
    actions: new HttpActionManager(baseUrl, token),
  };
}
