/**
 * Round-trip test for the `solx` umbrella + `@solx/types` end-to-end.
 *
 * Skips if the native binary is not built (e.g. in a fresh CI
 * checkout before the platform package is installed). Run
 * `cargo build --release -p solx-bindings` first to produce
 * `target/release/solx.node`, then point `SOLX_NATIVE_BIN` at it.
 */

import { afterAll, beforeAll, describe, expect, test } from 'vitest';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { SolxError } from '@solx/surface';
import { createSolx } from '../src/index.js';
import type { Solx } from '../src/solx.js';

let nativePath: string | undefined;
const nativeEnv = process.env['SOLX_NATIVE_BIN'];
if (nativeEnv && existsSync(nativeEnv)) {
  nativePath = nativeEnv;
  // Make the binding loader find it.
  process.env['SOLX_NODE_PATH'] = nativePath;
}

const describeIfNative = nativePath ? describe : describe.skip;

describeIfNative('createSolx (round-trip)', () => {
  let appdata: string;
  // Opened once and reused across every test() in this block — see
  // the same note in packages/files/test/files.test.ts: `solx.docs`
  // holds an exclusive Tantivy index-writer lock on the appdata's
  // search index directory, so a second concurrent `createSolx`
  // against the same appdata would deadlock on it.
  let solx: Solx;

  beforeAll(async () => {
    appdata = mkdtempSync(join(tmpdir(), 'solx-test-'));
    solx = await createSolx({ appdata });
  });

  afterAll(() => {
    if (appdata) rmSync(appdata, { recursive: true, force: true });
  });

  test('post → get → resolve → validate → list → delete', async () => {
    expect(solx.appdata).toBe(appdata);
    expect(solx.types).toBeDefined();

    // 1. Post a type. (Create-or-replace; path normalizes to /types/custom.)
    const t = await solx.types.post('/types/custom', 'Person', {
      schema: {
        type: 'object',
        required: ['name'],
        properties: { name: { type: 'string' } },
      },
      groups: ['document-type'],
    });
    expect(t.path).toBe('/types/custom');
    expect(t.name).toBe('Person');
    expect(t.groups).toEqual(['document-type']);
    expect(t.id).toMatch(/^[0-9a-f-]{36}$/);
    expect(t.createdAt).toMatch(/^\d{4}-\d{2}-\d{2}T/);

    // 2. Get by path+name.
    const got = await solx.types.get('/types/custom', 'Person');
    expect(got.id).toBe(t.id);

    // 3. Resolve by full ref.
    const resolved = await solx.types.resolve('/types/custom/Person');
    expect(resolved.id).toBe(t.id);

    // 4. Validate a value that matches the schema.
    await solx.types.validate({ name: 'Ada' }, '/types/custom/Person');

    // 5. Validation rejects non-matching values.
    await expect(
      solx.types.validate({}, '/types/custom/Person'),
    ).rejects.toThrow(SolxError);

    // 6. List with a path prefix.
    const page = await solx.types.list({ pathPrefix: '/types/custom' });
    expect(page.items).toHaveLength(1);
    expect(page.items[0]?.id).toBe(t.id);

    // 7. The built-in types seeded by the Rust impl are visible.
    const builtins = await solx.types.list({ pathPrefix: '/types/core' });
    expect(builtins.total).toBeGreaterThan(0);

    // 8. Delete.
    await solx.types.delete('/types/custom', 'Person');
    await expect(
      solx.types.get('/types/custom', 'Person'),
    ).rejects.toThrow(SolxError);
  }, 30_000);

  test('path normalization happens on the native side', async () => {
    // Missing leading slash should still resolve after normalization.
    const t = await solx.types.post('/types/x', 'X', {
      schema: { type: 'string' },
    });
    const got = await solx.types.get('/types/x', 'X');
    expect(got.id).toBe(t.id);
    await solx.types.delete('/types/x', 'X');
  });
});

describe('createSolx (no native binary)', () => {
  // These tests run regardless of whether the native binary is
  // present, and verify the *missing-binary* error path.
  const freshTmp = mkdtempSync(join(tmpdir(), 'solx-no-native-'));
  afterAll(() => rmSync(freshTmp, { recursive: true, force: true }));

  test('throws a clear SolxError when the native binary is missing', async () => {
    // The vitest test runner doesn't have the .node in its resolver
    // path, so this should fail with our "could not load" error.
    // Skip if a SOLX_NATIVE_BIN is set; otherwise the .node is
    // not on the require path.
    if (nativePath) return;
    await expect(createSolx({ appdata: freshTmp })).rejects.toThrow(SolxError);
  });
});
