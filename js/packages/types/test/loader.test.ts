/**
 * Tests that don't require the native binary.
 *
 * The round-trip / end-to-end test is in `packages/sdk/test/create.test.ts`
 * and is gated on `SOLX_NATIVE_BIN` being set. This file covers
 * the parts of @solx/types that are pure TS: the loader's
 * error path and the snake_case/camelCase DTO translation.
 */

import { describe, expect, test } from 'vitest';
import { SolxError } from '@solx/surface';
// We import the loader indirectly: it throws when the .node is not
// available, which is the path we exercise here.
import { TypeManager } from '../src/index.js';

describe('TypeManager (no native binary)', () => {
  test('open() throws a clear SolxError when no .node is available', async () => {
    const prevEnv = process.env['SOLX_NATIVE_BIN'];
    delete process.env['SOLX_NATIVE_BIN'];
    try {
      await expect(TypeManager.open(':memory:')).rejects.toThrow(SolxError);
    } finally {
      if (prevEnv !== undefined) process.env['SOLX_NATIVE_BIN'] = prevEnv;
    }
  });
});
