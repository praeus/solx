import { describe, expect, test } from 'vitest';
import { JsonValue } from '../src/index.js';

describe('JsonValue predicates', () => {
  test('is accepts every JSON shape', () => {
    expect(JsonValue.is(null)).toBe(true);
    expect(JsonValue.is(true)).toBe(true);
    expect(JsonValue.is(42)).toBe(true);
    expect(JsonValue.is('hi')).toBe(true);
    expect(JsonValue.is([1, 2, 3])).toBe(true);
    expect(JsonValue.is({ a: 1 })).toBe(true);
  });

  test('isObject rejects arrays and null', () => {
    expect(JsonValue.isObject({ a: 1 })).toBe(true);
    expect(JsonValue.isObject(null)).toBe(false);
    expect(JsonValue.isObject([])).toBe(false);
  });

  test('type guards narrow', () => {
    const v: unknown = { a: 1, b: 'x' };
    if (JsonValue.isObject(v)) {
      // After narrowing, v is JsonObject; index access returns JsonValue | undefined
      // because of noUncheckedIndexedAccess in tsconfig.base.json.
      const a = v['a'];
      expect(a).toBe(1);
    }
  });
});
