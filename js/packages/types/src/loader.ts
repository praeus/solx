/**
 * Loader for the native `solx.node` binary.
 *
 * The binary is shipped as a separate per-platform npm package
 * (`@solx/<triple>`, see platform-packages/). Each platform
 * package puts `solx.node` at its package root and the umbrella
 * `solx` package declares them as `optionalDependencies`. So
 * `require('@solx/<triple>')` resolves the right binary on the
 * current host.
 *
 * Resolution order at runtime:
 *
 *   1. `SOLX_NATIVE_BIN` env var (absolute path to a `.node` file)
 *      — useful for local dev with `cargo build` output.
 *   2. `require('@solx/<detected-triple>')` — auto-detect the
 *      current napi triple and load the matching platform package.
 *   3. `require('@solx/<fallback-triple>')` — try a few common
 *      fallbacks (e.g. win32-x64-gnu) if the exact triple isn't
 *      available.
 *   4. Fail with a clear "install a solx platform package" error.
 */

import { existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import { SolxError } from '@solx/surface';
import type { NativeTypesModule } from './native-types.js';

// `createRequire` lets us use `require()` from ESM modules.
// Without this, `require` is undefined in our `.js` output
// (because the package is `type: "module"`).
const require = createRequire(import.meta.url);

let cached: NativeTypesModule | null = null;

const TRIPLE_BY_PLATFORM: Record<string, string> = {
  'darwin-arm64': 'darwin-arm64',
  'darwin-x64': 'darwin-x64',
  'linux-x64-glibc': 'linux-x64-gnu',
  'linux-x64-musl': 'linux-x64-musl',
  'linux-arm64-glibc': 'linux-arm64-gnu',
  'linux-arm64-musl': 'linux-arm64-musl',
  'win32-x64': 'win32-x64-msvc',
  'win32-arm64': 'win32-arm64-msvc',
};

/**
 * Resolve the current platform to a napi triple.
 *
 * This intentionally diverges from the exact napi-rs naming
 * (`linux-x64-gnu` instead of `linux-x64-glibc`) because that's
 * what we publish under.
 */
function currentTriple(): string {
  const key = `${process.platform}-${process.arch}${isMusl() ? '-musl' : '-glibc'}`;
  return TRIPLE_BY_PLATFORM[key] ?? `${process.platform}-${process.arch}`;
}

function isMusl(): boolean {
  // No reliable way to detect musl from JS without a probe; the
  // most common case is glibc. Users on Alpine can set
  // `SOLX_TRIPLE=linux-x64-musl` to override.
  return process.env['SOLX_TRIPLE']?.endsWith('musl') ?? false;
}

function loadFromEnv(): NativeTypesModule | null {
  const env = process.env['SOLX_NATIVE_BIN'];
  if (!env) return null;
  try {
    // Normalize the path to whatever the host OS understands:
    //   - Git-Bash-style `/d/...` → `D:\...` on Windows
    //   - Forward slashes → backslashes on Windows
    //   - POSIX paths left as-is on Linux/macOS
    // We do this with `path.resolve` + `existsSync` rather than
    // passing the raw string to `require`, because Node's
    const resolved = toOsPath(env);
    if (!existsSync(resolved)) {
      // Surface this so the final error message makes sense.
      throw new Error(`SOLX_NATIVE_BIN=${env} (resolved ${resolved}) does not exist`);
    }
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    return require(resolved) as NativeTypesModule;
  } catch (e) {
    // Re-throw with the original env path so the top-level
    // error message can include it.
    throw new SolxError(
      'Other',
      `SOLX_NATIVE_BIN=${env} failed: ${(e as Error).message}`,
    );
  }
}

/**
 * Normalize any Git-Bash-style (`/d/...`) or POSIX path into
 * the OS-native form that Node's `require` accepts. On Windows
 * this returns a `D:\...` (or `\\?\D:\...`) path; on POSIX it
 * returns the path as-is.
 */
function toOsPath(p: string): string {
  // Git-Bash MSYS path: /d/foo -> D:\foo
  const m = p.match(/^\/([a-zA-Z])\/(.*)$/);
  if (m && process.platform === 'win32') {
    return `${(m[1] ?? 'c').toUpperCase()}:\\${(m[2] ?? '').replace(/\//g, '\\')}`;
  }
  // Otherwise, let Node resolve it.
  return p;
}

function loadFromTriple(triple: string): NativeTypesModule | null {
  // The platform package is `@solx/<triple>`.
  const pkg = `@solx/${triple}`;
  try {
    return require(pkg) as NativeTypesModule;
  } catch {
    return null;
  }
}

export function loadNative(): NativeTypesModule {
  if (cached) return cached;
  const explicit = process.env['SOLX_NATIVE_BIN'];
  if (explicit) {
    // If SOLX_NATIVE_BIN is set, that's the only source we try.
    // A clear error here is more useful than a silent fallback
    // to the per-platform package (which may not be installed).
    try {
      cached = loadFromEnv();
    } catch (e) {
      throw e instanceof SolxError ? e : new SolxError('Other', String(e));
    }
    if (cached) return cached;
    throw new SolxError(
      'Other',
      `SOLX_NATIVE_BIN=${explicit} could not be loaded. ` +
        `Check the path and that the .node binary is built.`,
    );
  }
  const triple = process.env['SOLX_TRIPLE'] ?? currentTriple();
  const mod = loadFromTriple(triple) ?? loadFromTriple(`${process.platform}-${process.arch}`);
  if (mod) {
    cached = mod;
    return cached;
  }
  throw new SolxError(
    'Other',
    `could not load native solx bindings (tried @solx/${triple} and ` +
      `${process.platform}-${process.arch}). ` +
      `Install a solx platform package or set SOLX_NATIVE_BIN to a built solx.node.`,
  );
}
