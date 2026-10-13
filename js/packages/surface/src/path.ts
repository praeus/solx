/**
 * Mirror of `solx_surface::path` (see solx-core/solx-surface/src/path.rs).
 *
 * solx uses a flat store with `(path, name)` identity, addressed as a
 * full ref of the form `"/path/to/thing"` (or just `"/"` for the root).
 * `name` is the trailing segment; `path` is everything before it.
 */

import { SolxError } from './error.js';

/** A normalized path: always starts with `/`, no trailing slash (except `/` itself). */
export type Path = string;

/** A name segment: 1+ chars, no `/`, no whitespace at edges. */
export type Name = string;

/** The full reference: `path` + `name`, e.g. `"/research/ai/note"`. */
export type FullRef = string;

const NAME_RE = /^[A-Za-z0-9._-]+$/;

export const Path = {
  /**
   * Normalize a path. Throws {@link SolxError} of kind `Invalid` if the
   * path is malformed. The canonical form is:
   *   - always begins with `/`
   *   - no trailing `/` (except for the root itself)
   *   - no empty segments
   *   - no `.` or `..` segments
   */
  normalize(path: string): Path {
    if (path === '' || path === '/') return '/';
    if (!path.startsWith('/')) {
      throw new SolxError('Invalid', `path must start with '/': ${path}`);
    }
    const segments = path.split('/').slice(1);
    if (segments.some((s) => s === '')) {
      throw new SolxError('Invalid', `path has empty segment: ${path}`);
    }
    if (segments.some((s) => s === '.' || s === '..')) {
      throw new SolxError('Invalid', `path may not contain '.' or '..': ${path}`);
    }
    return '/' + segments.join('/');
  },

  /** True if the string is a well-formed normalized path. */
  isValid(path: string): boolean {
    try {
      Path.normalize(path);
      return true;
    } catch {
      return false;
    }
  },
} as const;

export const Name = {
  /**
   * Validate a name. Throws {@link SolxError} of kind `Invalid` if the
   * name is empty, contains `/`, or contains forbidden characters.
   */
  validate(name: string): Name {
    if (name === '' || name.length === 0) {
      throw new SolxError('Invalid', 'name must not be empty');
    }
    if (name.includes('/')) {
      throw new SolxError('Invalid', `name may not contain '/': ${name}`);
    }
    if (!NAME_RE.test(name)) {
      throw new SolxError(
        'Invalid',
        `name may only contain [A-Za-z0-9._-]: ${name}`,
      );
    }
    return name;
  },

  isValid(name: string): boolean {
    try {
      Name.validate(name);
      return true;
    } catch {
      return false;
    }
  },
} as const;

export const Ref = {
  /** Combine `path` + `name` into a full ref. */
  full(path: string, name: string): FullRef {
    const p = Path.normalize(path);
    const n = Name.validate(name);
    return p === '/' ? `/${n}` : `${p}/${n}`;
  },

  /**
   * Split a full ref into `(path, name)`. Throws {@link SolxError} of
   * kind `Invalid` if the ref is malformed.
   */
  split(full: string): { path: Path; name: Name } {
    if (full === '' || full === '/') {
      throw new SolxError('Invalid', `empty full ref: '${full}'`);
    }
    if (!full.startsWith('/')) {
      throw new SolxError('Invalid', `full ref must start with '/': ${full}`);
    }
    const lastSlash = full.lastIndexOf('/');
    if (lastSlash === 0) {
      // "/name" — root path, single-segment name.
      return { path: '/', name: Name.validate(full.slice(1)) };
    }
    const path = Path.normalize(full.slice(0, lastSlash));
    const name = Name.validate(full.slice(lastSlash + 1));
    return { path, name };
  },
} as const;
