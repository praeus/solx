/**
 * Schema introspection helpers shared by the schema-driven field editors.
 *
 * These were originally private to `DocumentEditor.tsx`; they're extracted
 * here so the new `FieldsEditorPanel` (used by `ActionRunner`) can reuse the
 * exact same dispatch rules without duplicating them. `DocumentEditor` keeps
 * its own inline copies for now — this file is the shared source of truth
 * for new consumers.
 *
 * The helpers answer three questions about a JSON-Schema node:
 *   1. What is its "kind" for rendering? (`schemaValueType`)
 *   2. Is it a reference to another entity? (`refKindFromSchema`)
 *   3. Should it be hidden by default? (`isHiddenDocField`)
 */

export type RefKind = "DocRef" | "ActionRef";

export interface RefFieldValue {
  kind: RefKind;
  mode: "name" | "id";
  value: string;
}

const HELPER_FIELD_NAMES = new Set(["chapters", "metadata"]);

/** Extract the `properties` map from a type schema (or `{}` if absent). */
export function propertySchema(
  typeSchema: unknown,
): Record<string, Record<string, unknown>> {
  if (!typeSchema || typeof typeSchema !== "object") return {};
  const props = (typeSchema as Record<string, unknown>).properties;
  if (!props || typeof props !== "object") return {};
  return props as Record<string, Record<string, unknown>>;
}

/** Resolve the first `$ref` target on a schema node (direct, allOf, anyOf). */
export function schemaRefTarget(schema: Record<string, unknown>): string {
  const direct = typeof schema.$ref === "string" ? schema.$ref : "";
  if (direct) return direct;
  const allOf = Array.isArray(schema.allOf) ? schema.allOf : [];
  for (const item of allOf) {
    if (
      item &&
      typeof item === "object" &&
      typeof (item as Record<string, unknown>).$ref === "string"
    ) {
      return String((item as Record<string, unknown>).$ref ?? "");
    }
  }
  const anyOf = Array.isArray(schema.anyOf) ? schema.anyOf : [];
  for (const item of anyOf) {
    if (
      item &&
      typeof item === "object" &&
      typeof (item as Record<string, unknown>).$ref === "string"
    ) {
      return String((item as Record<string, unknown>).$ref ?? "");
    }
  }
  return "";
}

/** Detect a DocRef/ActionRef field via `$ref`, `allOf[*].$ref`, or `anyOf[*].$ref`. */
export function refKindFromSchema(schema: Record<string, unknown>): RefKind | null {
  const direct = String(schema.$ref ?? "");
  if (direct.includes("DocRef")) return "DocRef";
  if (direct.includes("ActionRef")) return "ActionRef";
  const allOf = Array.isArray(schema.allOf) ? schema.allOf : [];
  for (const item of allOf) {
    if (item && typeof item === "object") {
      const ref = String((item as Record<string, unknown>).$ref ?? "");
      if (ref.includes("DocRef")) return "DocRef";
      if (ref.includes("ActionRef")) return "ActionRef";
    }
  }
  const anyOf = Array.isArray(schema.anyOf) ? schema.anyOf : [];
  for (const item of anyOf) {
    if (item && typeof item === "object") {
      const ref = String((item as Record<string, unknown>).$ref ?? "");
      if (ref.includes("DocRef")) return "DocRef";
      if (ref.includes("ActionRef")) return "ActionRef";
    }
  }
  return null;
}

export function isRichTextSchema(schema: Record<string, unknown>): boolean {
  return (
    schema["x-sol-editor"] === "rich_text" ||
    schemaRefTarget(schema).includes("RichTextDoc")
  );
}

export function isImageSchema(schema: Record<string, unknown>): boolean {
  if (schema["x-sol-editor"] === "image") return true;
  const ref = schemaRefTarget(schema);
  // Generic heuristic: any $ref whose name contains "Image" or
  // "ArtifactRef" (the legacy image-ref type from sol-browser).
  return /Image$|ArtifactRef$/.test(ref);
}

export function isSchemaHidden(schema: Record<string, unknown>): boolean {
  return schema["x-sol-hidden"] === true;
}

export function isHiddenDocField(
  fieldName: string,
  schema: Record<string, unknown>,
): boolean {
  return isSchemaHidden(schema) || HELPER_FIELD_NAMES.has(fieldName);
}

/**
 * The render "kind" for a schema node. One of:
 *   - "rich_text" / "image" (annotation-driven)
 *   - "string" | "number" | "integer" | "boolean" | "array" | "object"
 *   - "object" for any `$ref` target without an explicit `type`
 */
export function schemaValueType(schema: Record<string, unknown>): string {
  if (isRichTextSchema(schema)) return "rich_text";
  if (isImageSchema(schema)) return "image";
  if (typeof schema.type === "string") return schema.type;
  if (schemaRefTarget(schema)) return "object";
  return "string";
}

export function emptyRichTextValue(): string {
  return JSON.stringify({ type: "doc", content: [] }, null, 2);
}

/** Coerce an unknown value into the string form the field editors store. */
export function valueFromUnknown(value: unknown, asType: string): string {
  if (value == null) return asType === "rich_text" ? emptyRichTextValue() : "";
  if (asType === "boolean") return value ? "true" : "false";
  if (asType === "object" || asType === "array" || asType === "rich_text") {
    try {
      return JSON.stringify(value, null, 2);
    } catch {
      return asType === "rich_text" ? emptyRichTextValue() : "";
    }
  }
  return String(value);
}
