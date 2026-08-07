/**
 * A typed wrapper around `serde_json::Value`. We do not type-erase into
 * `any`; callers can narrow with helpers (see {@link isObject}, etc.).
 *
 * Matches the serde_json model: `null`, `boolean`, `number`, `string`,
 * array, or object — with `object` being a `Record<string, JsonValue>`.
 */
export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | { [key: string]: JsonValue };

export interface JsonObject {
  [key: string]: JsonValue;
}

export const JsonValue = {
  is(v: unknown): v is JsonValue {
    return (
      v === null ||
      typeof v === 'boolean' ||
      typeof v === 'number' ||
      typeof v === 'string' ||
      Array.isArray(v) ||
      (typeof v === 'object' && v !== null)
    );
  },
  isObject(v: unknown): v is JsonObject {
    return (
      v !== null && typeof v === 'object' && !Array.isArray(v)
    );
  },
  isArray(v: unknown): v is JsonValue[] {
    return Array.isArray(v);
  },
  isString(v: unknown): v is string {
    return typeof v === 'string';
  },
  isNumber(v: unknown): v is number {
    return typeof v === 'number';
  },
  isBoolean(v: unknown): v is boolean {
    return typeof v === 'boolean';
  },
  isNull(v: unknown): v is null {
    return v === null;
  },
} as const;
