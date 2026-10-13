/**
 * Turns a `solx-server` HTTP error body back into a real `SolxError`. The
 * mapping itself lives in `@solx/surface` (`mapWireError`), shared with
 * `@solx/tauri`, whose IPC errors carry the same tagged shape.
 */
export { mapWireError } from '@solx/surface';
