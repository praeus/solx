/**
 * Mirror of `solx_surface::SolxError` (see solx-core/solx-surface/src/error.rs).
 *
 * The Rust side stringifies the discriminant as the error message prefix:
 *
 *   "SolxError::NotFound: type not found: /x/y"
 *
 * The Neon layer throws this as a plain `Error`; the TS side parses the
 * prefix and reconstructs a real {@link SolxError} via {@link fromMessage}.
 */
export type SolxErrorKind =
  | 'NotFound'
  | 'Invalid'
  | 'Validation'
  | 'Conflict'
  | 'Io'
  | 'Db'
  | 'Exec'
  | 'Config'
  | 'Other';

export const SOLX_ERROR_KINDS: readonly SolxErrorKind[] = [
  'NotFound',
  'Invalid',
  'Validation',
  'Conflict',
  'Io',
  'Db',
  'Exec',
  'Config',
  'Other',
] as const;

/** Canonical prefix used by the Rust binding layer. */
export const SOLX_ERROR_PREFIX = 'SolxError::';

export class SolxError extends Error {
  readonly kind: SolxErrorKind;
  readonly detail: string;

  constructor(kind: SolxErrorKind, detail: string) {
    super(`${kind}: ${detail}`);
    this.name = 'SolxError';
    this.kind = kind;
    this.detail = detail;
    // Maintain prototype chain when targeting ES5 / older runtimes.
    Object.setPrototypeOf(this, SolxError.prototype);
  }

  /** True if `e` is a {@link SolxError} (instance or duck-typed). */
  static is(e: unknown): e is SolxError {
    if (e instanceof SolxError) return true;
    if (e === null || typeof e !== 'object') return false;
    const obj = e as { name?: unknown; kind?: unknown; message?: unknown };
    return (
      obj.name === 'SolxError' &&
      typeof obj.kind === 'string' &&
      typeof obj.message === 'string'
    );
  }

  /**
   * Reconstruct a {@link SolxError} from a `JsError` thrown by the Neon
   * layer. Accepts either a raw string or an `Error` whose message
   * starts with {@link SOLX_ERROR_PREFIX}.
   *
   * If the input does not look like a Solx error, it is wrapped in
   * `SolxError('Other', String(e))`.
   */
  static fromMessage(e: unknown): SolxError {
    const raw = e instanceof Error ? e.message : typeof e === 'string' ? e : String(e);
    if (raw.startsWith(SOLX_ERROR_PREFIX)) {
      const rest = raw.slice(SOLX_ERROR_PREFIX.length);
      const colon = rest.indexOf(':');
      if (colon > 0) {
        const kind = rest.slice(0, colon) as SolxErrorKind;
        const detail = rest.slice(colon + 1).trim();
        if ((SOLX_ERROR_KINDS as readonly string[]).includes(kind)) {
          return new SolxError(kind, detail);
        }
      }
    }
    return new SolxError('Other', raw);
  }
}
