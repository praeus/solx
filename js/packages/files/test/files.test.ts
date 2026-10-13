/**
 * Round-trip test for the `solx.files` end-to-end.
 *
 * Skips if the native binary is not built (e.g. in a fresh CI
 * checkout before the platform package is installed). Run
 * `cargo build --release -p solx-bindings` first, set
 * `SOLX_NATIVE_BIN` to the resulting `solx.node`, then run
 * `bun run test`.
 */

import { afterAll, beforeAll, describe, expect, test } from 'vitest';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { SolxError } from '@solx/surface';
import { FilePath } from '@solx/files';
// `createSolx` lives in the umbrella `solx` package (sibling
// at `packages/sdk`). We import it via a relative path so the
// test works inside the workspace without a published package
// name. The same call site works in the published version
// via `import { createSolx } from 'solx'`.
import { createSolx } from '../../sdk/src/index.js';
import type { Solx } from '../../sdk/src/solx.js';

let nativePath: string | undefined;
const nativeEnv = process.env['SOLX_NATIVE_BIN'];
if (nativeEnv && existsSync(nativeEnv)) {
  nativePath = nativeEnv;
  process.env['SOLX_NODE_PATH'] = nativePath;
}

const describeIfNative = nativePath ? describe : describe.skip;

describeIfNative('solx.files (round-trip)', () => {
  let appdata: string;
  // `createSolx` is opened once and reused across every test() in
  // this block, not per-test: `solx.docs` holds a live Tantivy
  // `IndexWriter` that exclusively locks the search index directory,
  // so a second `createSolx({ appdata })` against the same appdata
  // while the first is still alive (GC timing for the first is not
  // deterministic) would deadlock on that lock file. This mirrors
  // real usage — a process opens `solx` once and keeps using it.
  let solx: Solx;

  beforeAll(async () => {
    appdata = mkdtempSync(join(tmpdir(), 'solx-files-test-'));
    solx = await createSolx({ appdata });
  });

  afterAll(() => {
    if (appdata) rmSync(appdata, { recursive: true, force: true });
  });

  test('put → get → list → delete', async () => {
    expect(solx.files.root()).toBe(join(appdata, 'files').replace(/\\/g, '/'));

    // 1. Encode some bytes (UTF-8 text so we can verify content).
    const payload = new TextEncoder().encode(
      'Hello, solx-files! 🚀\n' + 'x'.repeat(1024),
    );

    // 2. put returns the normalized rel path.
    const putPath = FilePath.docFilePath('doc-abc-123', 'hello.txt');
    const returned = await solx.files.put(putPath, payload);
    expect(returned).toBe(putPath);

    // 3. get returns identical bytes.
    const got = await solx.files.get(putPath);
    expect(got).toEqual(payload);
    // Decoding is symmetric
    expect(new TextDecoder().decode(got)).toContain('Hello, solx-files!');

    // 4. list finds the new file.
    const listed = await solx.files.list('files/docs/doc-abc-123');
    expect(listed).toContain(putPath);

    // 5. Path traversal is rejected (SolxError('Invalid')).
    await expect(
      solx.files.put('../escape.txt', new Uint8Array([1, 2, 3])),
    ).rejects.toThrow(SolxError);

    // 6. get on a missing file throws SolxError('NotFound').
    await expect(solx.files.get('files/docs/missing/x.txt')).rejects.toThrow(
      SolxError,
    );

    // 7. delete is idempotent.
    await solx.files.delete(putPath);
    await expect(solx.files.get(putPath)).rejects.toThrow(SolxError);
    await expect(solx.files.delete(putPath)).resolves.toBeUndefined();
  }, 30_000);

  test('binary round-trip (zero bytes, then 64KB)', async () => {
    // Empty file.
    const emptyPath = FilePath.sharedDocFilePath('empty.bin');
    const norm = await solx.files.put(emptyPath, new Uint8Array(0));
    expect(norm).toBe(emptyPath);
    const empty = await solx.files.get(emptyPath);
    expect(empty.byteLength).toBe(0);

    // 64 KB of pseudo-random bytes.
    const big = new Uint8Array(64 * 1024);
    for (let i = 0; i < big.length; i++) big[i] = (i * 31 + 7) & 0xff;
    const bigPath = FilePath.docFilePath('big-doc', 'big.bin');
    await solx.files.put(bigPath, big);
    const bigBack = await solx.files.get(bigPath);
    expect(bigBack.byteLength).toBe(big.length);
    expect(bigBack).toEqual(big);
  }, 30_000);
});

describe('solx.files (no native binary)', () => {
  const freshTmp = mkdtempSync(join(tmpdir(), 'solx-no-native-files-'));
  afterAll(() => rmSync(freshTmp, { recursive: true, force: true }));

  test('throws a clear SolxError when the native binary is missing', async () => {
    if (nativePath) return; // skip when the binary IS available
    await expect(createSolx({ appdata: freshTmp })).rejects.toThrow(SolxError);
  });
});
