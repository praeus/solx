/**
 * Schema → default-value generator.
 *
 * Produces a `JsonValue` default for a JSON-Schema node so the ActionRunner's
 * JSON params tab can pre-fill a sensible starting value from the action's
 * parameter type. Rules, in priority order:
 *
 *   1. `default` — if the schema declares one, use it verbatim.
 *   2. `enum` — first entry.
 *   3. `const` — the constant.
 *   4. `$ref` — recurse into the referenced schema (resolved against the
 *      type's `$defs`/`definitions`), falling back to `{}` if unresolvable.
 *   5. `type` — `object` → recurse over `properties`; `array` → `[]` (or a
 *      single defaulted item when `items` is an object schema); `boolean` →
 *      `false`; `number`/`integer` → `0`; `string` → `""`; `null` → `null`.
 *   6. `anyOf`/`oneOf` — first branch.
 *   7. Fallback — `null`.
 */

import type { JsonValue } from "@solx/surface";

type SchemaNode = Record<string, unknown>;

/** Resolve a `$ref` (e.g. `#/$defs/Foo`) against a schema's definition map. */
function resolveRef(
  ref: string,
  root: SchemaNode,
): SchemaNode | null {
  if (!ref.startsWith("#/")) return null;
  const parts = ref.slice(2).split("/").map((p) => p.replace(/~1/g, "/").replace(/~0/g, "~"));
  let node: unknown = root;
  for (const part of parts) {
    if (node && typeof node === "object") {
      node = (node as Record<string, unknown>)[part];
    } else {
      return null;
    }
  }
  return node && typeof node === "object" ? (node as SchemaNode) : null;
}

function firstBranch(schema: SchemaNode): SchemaNode | null {
  for (const key of ["anyOf", "oneOf"]) {
    const arr = schema[key];
    if (Array.isArray(arr) && arr.length > 0 && arr[0] && typeof arr[0] === "object") {
      return arr[0] as SchemaNode;
    }
  }
  return null;
}

export function defaultValueFromSchema(
  schema: unknown,
  root?: SchemaNode,
): JsonValue {
  if (!schema || typeof schema !== "object") return null;
  const node = schema as SchemaNode;
  const effectiveRoot = root ?? node;

  if ("default" in node) return node.default as JsonValue;
  if (Array.isArray(node.enum) && node.enum.length > 0) return node.enum[0] as JsonValue;
  if ("const" in node) return node.const as JsonValue;

  const ref = typeof node.$ref === "string" ? node.$ref : "";
  if (ref) {
    const resolved = resolveRef(ref, effectiveRoot);
    if (resolved) return defaultValueFromSchema(resolved, effectiveRoot);
    return {};
  }

  const branch = firstBranch(node);
  if (branch) return defaultValueFromSchema(branch, effectiveRoot);

  const type = typeof node.type === "string" ? node.type : "";
  switch (type) {
    case "object": {
      const props = node.properties;
      if (props && typeof props === "object") {
        const out: Record<string, JsonValue> = {};
        for (const [key, sub] of Object.entries(props as Record<string, unknown>)) {
          out[key] = defaultValueFromSchema(sub, effectiveRoot);
        }
        return out;
      }
      return {};
    }
    case "array": {
      const items = node.items;
      if (items && typeof items === "object" && !Array.isArray(items)) {
        return [defaultValueFromSchema(items, effectiveRoot)];
      }
      return [];
    }
    case "boolean":
      return false;
    case "number":
    case "integer":
      return 0;
    case "string":
      return "";
    case "null":
      return null;
    default:
      return null;
  }
}
