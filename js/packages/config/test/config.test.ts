/**
 * Round-trip test for the `solx.config` end-to-end.
 *
 * Covers:
 *   - `ConfigService.openIn` resolves the appdata / configPath
 *   - `set` / `get` round-trip a JSON value
 *   - `patch` shallow-merges an object
 *   - `mutate` (high-level) does the same thing more cleanly
 *   - `snapshot` returns the typed `SolxConfig`
 *   - `rawSnapshot` preserves unknown keys the typed snapshot drops
 *   - `listPackages` / `registerPackage` / `unregisterPackage`
 *   - derived paths (`typesDbPath`, `filesDir`, `logsDir`, …)
 *   - `createSolx` wires the config service into the umbrella
 *     and uses `config.typesDbPath()` / `config.filesDir()` to
 *     locate the other stores
 *
 * Skips if the native binary is not built. Run
 * `cargo build --release -p solx-bindings` first, set
 * `SOLX_NATIVE_BIN` to the resulting `solx.node`, then run
 * `bun run test`.
 */

import { afterAll, beforeAll, describe, expect, test } from 'vitest';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { ConfigService } from '@solx/config';
import { createSolx } from '../../sdk/src/index.js';
import type { Solx } from '../../sdk/src/solx.js';

let nativePath: string | undefined;
const nativeEnv = process.env['SOLX_NATIVE_BIN'];
if (nativeEnv && existsSync(nativeEnv)) {
  nativePath = nativeEnv;
  process.env['SOLX_NODE_PATH'] = nativePath;
}

const describeIfNative = nativePath ? describe : describe.skip;

describeIfNative('solx.config (round-trip)', () => {
  let appdata: string;

  beforeAll(() => {
    appdata = mkdtempSync(join(tmpdir(), 'solx-config-test-'));
  });

  afterAll(() => {
    if (appdata) rmSync(appdata, { recursive: true, force: true });
  });

  test('openIn resolves appdata + configPath', () => {
    const cfg = ConfigService.openIn(appdata);
    expect(cfg.appdata).toBe(appdata.replace(/\\/g, '/'));
    expect(cfg.configPath).toBe(
      join(appdata, 'solx-config.json').replace(/\\/g, '/'),
    );
  });

  test('set → get round-trips a JSON value', () => {
    const cfg = ConfigService.openIn(appdata);
    cfg.set('greeting', 'hello');
    cfg.set('count', 42);
    cfg.set('nested', { foo: 'bar', arr: [1, 2, 3] });

    expect(cfg.get('greeting')).toBe('hello');
    expect(cfg.get('count')).toBe(42);
    expect(cfg.get('nested')).toEqual({ foo: 'bar', arr: [1, 2, 3] });
  });

  test('get returns undefined for an unknown key', () => {
    const cfg = ConfigService.openIn(appdata);
    expect(cfg.get('does-not-exist')).toBeUndefined();
  });

  test('patch shallow-merges an object', () => {
    const cfg = ConfigService.openIn(appdata);
    cfg.set('existing', { a: 1, b: 2 });
    cfg.patch({ existing: { b: 99, c: 3 }, newKey: 'hi' });
    // patch REPLACES the top-level key (shallow merge):
    expect(cfg.get('existing')).toEqual({ b: 99, c: 3 });
    expect(cfg.get('newKey')).toBe('hi');
  });

  test('derived paths are under appdata and forward-slashed', () => {
    const cfg = ConfigService.openIn(appdata);
    expect(cfg.dbDir()).toBe(join(appdata, 'db').replace(/\\/g, '/'));
    expect(cfg.docsDbPath()).toBe(
      join(appdata, 'db', 'solx-docs.db').replace(/\\/g, '/'),
    );
    expect(cfg.actionsDbPath()).toBe(
      join(appdata, 'db', 'solx-actions.db').replace(/\\/g, '/'),
    );
    expect(cfg.typesDbPath()).toBe(
      join(appdata, 'db', 'solx-types.db').replace(/\\/g, '/'),
    );
    expect(cfg.filesDir()).toBe(join(appdata, 'files').replace(/\\/g, '/'));
    expect(cfg.logsDir()).toBe(join(appdata, 'logs').replace(/\\/g, '/'));
  });

  test('snapshot returns the typed SolxConfig', () => {
    const cfg = ConfigService.openIn(appdata);
    const snap = cfg.snapshot() as Record<string, unknown>;
    // `installedPackages` may be absent (default-constructed
    // config) or an empty array — both are valid.
    if ('installedPackages' in snap) {
      expect(Array.isArray(snap['installedPackages'])).toBe(true);
    }
  });

  test('rawSnapshot preserves unknown keys', () => {
    const cfg = ConfigService.openIn(appdata);
    cfg.set('futureFeature', { magic: true });
    const raw = cfg.rawSnapshot() as Record<string, unknown>;
    expect(raw['futureFeature']).toEqual({ magic: true });
  });

  test('registerPackage → listPackages → unregisterPackage', () => {
    const cfg = ConfigService.openIn(appdata);
    cfg.registerPackage({
      name: 'pkg-a',
      version: '1.0.0',
      path: '/some/where',
      installedAt: '2025-01-01T00:00:00Z',
    });
    cfg.registerPackage({
      name: 'pkg-b',
      version: '2.3.4',
      path: '/elsewhere',
      installedAt: '2025-02-02T00:00:00Z',
    });

    const pkgs = cfg.listPackages() as Array<{ name: string; version: string }>;
    const names = pkgs.map((p) => p.name).sort();
    expect(names).toEqual(['pkg-a', 'pkg-b']);
    expect(
      pkgs.find((p) => p.name === 'pkg-b')?.version,
    ).toBe('2.3.4');

    cfg.unregisterPackage('pkg-a');
    const remaining = cfg
      .listPackages()
      .map((p) => (p as { name: string }).name);
    expect(remaining).toEqual(['pkg-b']);

    // idempotent
    cfg.unregisterPackage('pkg-a');
    expect(
      cfg.listPackages().map((p) => (p as { name: string }).name),
    ).toEqual(['pkg-b']);
  });
});

describeIfNative('solx umbrella (createSolx wires config)', () => {
  let appdata: string;
  // Opened once and reused across every test() in this block —
  // `solx.docs` holds an exclusive Tantivy index-writer lock on the
  // appdata's search index directory, so a second concurrent
  // `createSolx` against the same appdata would deadlock on it (see
  // the same note in packages/files/test/files.test.ts). The
  // "persists across reopens" test below reopens a standalone
  // `ConfigService` instead of a whole second `createSolx` — config
  // has no such exclusive-lock constraint (its writes serialize via
  // a short-held OS advisory lock, not a long-held index writer), so
  // that's still a faithful test of on-disk persistence.
  let solx: Solx;

  beforeAll(async () => {
    appdata = mkdtempSync(join(tmpdir(), 'solx-config-umbrella-test-'));
    solx = await createSolx({ appdata });
  });

  afterAll(() => {
    if (appdata) rmSync(appdata, { recursive: true, force: true });
  });

  test('createSolx returns a real ConfigService', () => {
    expect(solx.config).toBeDefined();
    expect(solx.config.appdata).toBe(appdata.replace(/\\/g, '/'));
    // The config service's derived paths should match what
    // the other stores were opened against.
    expect(solx.types).toBeDefined();
    expect(solx.files).toBeDefined();
    expect(solx.files.root()).toBe(solx.config.filesDir());
  });

  test('set via the umbrella persists across reopens', () => {
    solx.config.set('umbrellaKey', { hello: 'world' });
    expect(solx.config.get('umbrellaKey')).toEqual({ hello: 'world' });

    // Reopen a fresh ConfigService from disk — should see the same value.
    const reopened = ConfigService.openIn(appdata);
    expect(reopened.get('umbrellaKey')).toEqual({ hello: 'world' });
  });

  test('mutate (high-level) checks out → edits → commits', () => {
    solx.config.set('counter', 1);
    solx.config.mutate((draft) => {
      draft['counter'] = ((draft['counter'] as number) ?? 0) + 1;
      draft['added'] = true;
    });
    expect(solx.config.get('counter')).toBe(2);
    expect(solx.config.get('added')).toBe(true);
  });
});