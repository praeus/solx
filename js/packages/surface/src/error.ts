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

const WIRE_KIND_MAP: Record<string, SolxErrorKind> = {
  not_found: 'NotFound',
  invalid: 'Invalid',
  validation: 'Validation',
  conflict: 'Conflict',
  io: 'Io',
  db: 'Db',
  exec: 'Exec',
  config: 'Config',
  other: 'Other',
};

/**
 * Turn a serialized `solx_surface::SolxError` back into a real
 * {@link SolxError}. On the wire it is adjacently tagged —
 * `{"kind":"not_found","message":"..."}`, snake_case (see
 * solx-core/solx-surface/src/error.rs) — which is how both `solx-server`
 * (as an HTTP error body) and the solx Tauri plugin (as a rejected
 * `invoke`) report it. Mirrors `solx-client::error::read_response`.
 */
export function mapWireError(body: unknown): SolxError {
  if (body && typeof body === 'object') {
    const b = body as { kind?: unknown; message?: unknown };
    if (typeof b.kind === 'string' && typeof b.message === 'string') {
      return new SolxError(WIRE_KIND_MAP[b.kind] ?? 'Other', b.message);
    }
  }
  return new SolxError(
    'Other',
    `solx host returned an unrecognized error: ${JSON.stringify(body)}`,
  );
}
