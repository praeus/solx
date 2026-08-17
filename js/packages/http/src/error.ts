/**
 * Turn a `solx-server` HTTP error body back into a real {@link SolxError}.
 *
 * `solx-surface::error::SolxError` is adjacently tagged on the wire —
 * `{"kind":"not_found","message":"..."}`, snake_case (see
 * solx-core/solx-surface/src/error.rs) — which is a different convention
 * from the Neon binding's `"SolxError::NotFound: ..."` string prefix that
 * `@solx/surface`'s `SolxError.fromMessage` parses. This module is the HTTP
 * analog of that reconstruction, mirroring `solx-client::error::read_response`
 * (solx-core/solx-client/src/error.rs).
 */
import { SolxError, type SolxErrorKind } from '@solx/surface';

const KIND_MAP: Record<string, SolxErrorKind> = {
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

export function mapWireError(body: unknown): SolxError {
  if (body && typeof body === 'object') {
    const b = body as { kind?: unknown; message?: unknown };
    if (typeof b.kind === 'string' && typeof b.message === 'string') {
      return new SolxError(KIND_MAP[b.kind] ?? 'Other', b.message);
    }
  }
  return new SolxError(
    'Other',
    `solx-server returned an unrecognized error body: ${JSON.stringify(body)}`,
  );
}
