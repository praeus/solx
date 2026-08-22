/**
 * FieldsEditorPanel — reusable schema-driven fields editor.
 *
 * Renders the `properties` of a JSON-Schema (a Type's `schema`) as a form,
 * dispatching each field to the appropriate control based on its schema
 * "kind" (see `schemaFields.ts`):
 *
 *   - text / number / integer → `<textarea>`
 *   - boolean → checkbox
 *   - array / object → `<JsonEditField>` (CodeMirror 6 with JSON linter)
 *   - rich_text → `<RichTextField>` (Tiptap)
 *   - image → `<ImageField>`
 *   - DocRef / ActionRef → name-or-id input + datalist of entity names
 *
 * This is the same dispatch logic `DocumentEditor` uses inline; it's
 * extracted here so `ActionRunner` can reuse it for the params "Fields" tab.
 * `DocumentEditor` is intentionally left as-is for now.
 *
 * State model mirrors `DocumentEditor`: scalar/JSON fields live in a
 * `Record<string, string>` (stringified), while ref fields live in a
 * separate `Record<string, RefFieldValue>`.
 */

import type { ReactNode } from "react";
import { ImageField } from "./ImageField";
import { JsonEditField } from "./JsonEditField";
import { RichTextField } from "./RichTextField";
import {
  isHiddenDocField,
  propertySchema,
  refKindFromSchema,
  schemaValueType,
  type RefFieldValue,
} from "./schemaFields";

interface Props {
  /** The Type's schema (or any JSON-Schema object with `properties`). */
  schema: unknown;
  /** Stringified values for scalar/JSON fields, keyed by field name. */
  values: Record<string, string>;
  /** Ref-field values, keyed by field name. */
  refValues: Record<string, RefFieldValue>;
  onValuesChange: (next: Record<string, string>) => void;
  onRefValuesChange: (next: Record<string, RefFieldValue>) => void;
  /** Entity names for DocRef/ActionRef datalists. */
  documentNames: string[];
  actionNames: string[];
  /** Reveal `x-sol-hidden` / helper fields. */
  showHidden?: boolean;
  /** Optional empty-state message when the schema has no properties. */
  emptyMessage?: string;
}

export function FieldsEditorPanel({
  schema,
  values,
  refValues,
  onValuesChange,
  onRefValuesChange,
  documentNames,
  actionNames,
  showHidden = false,
  emptyMessage = "No fields for this schema.",
}: Props) {
  const properties = propertySchema(schema);
  const entries = Object.entries(properties);

  if (entries.length === 0) {
    return <div className="dialog__hint">{emptyMessage}</div>;
  }

  const visible = entries.filter(
    ([fieldName, fieldSchema]) => showHidden || !isHiddenDocField(fieldName, fieldSchema),
  );

  if (visible.length === 0) {
    return <div className="dialog__hint">{emptyMessage}</div>;
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
      {visible.map(([fieldName, fieldSchema]) =>
        renderField({
          fieldName,
          schema: fieldSchema,
          values,
          refValues,
          onValuesChange,
          onRefValuesChange,
          documentNames,
          actionNames,
        }),
      )}
    </div>
  );
}

// ───────────────────────────────────────────────────────────────────────
// Per-field renderer — dispatches on schema type.
// ───────────────────────────────────────────────────────────────────────

interface FieldContext {
  fieldName: string;
  schema: Record<string, unknown>;
  values: Record<string, string>;
  refValues: Record<string, RefFieldValue>;
  onValuesChange: (next: Record<string, string>) => void;
  onRefValuesChange: (next: Record<string, RefFieldValue>) => void;
  documentNames: string[];
  actionNames: string[];
}

function renderField(ctx: FieldContext): ReactNode {
  const {
    fieldName,
    schema,
    values,
    refValues,
    onValuesChange,
    onRefValuesChange,
    documentNames,
    actionNames,
  } = ctx;

  const setValue = (next: string) =>
    onValuesChange({ ...values, [fieldName]: next });

  const refKind = refKindFromSchema(schema);
  if (refKind) {
    const refState =
      refValues[fieldName] ?? { kind: refKind, mode: "name" as const, value: "" };
    const lookupOptions = refKind === "DocRef" ? documentNames : actionNames;
    return (
      <div key={fieldName} className="form-field">
        <label className="form-label" style={{ textTransform: "none", letterSpacing: 0 }}>
          {String(schema.title ?? fieldName)} ({refKind})
        </label>
        <div className="doc-editor-ref-field">
          <select
            className="form-input doc-editor-ref-field__mode"
            value={refState.mode}
            onChange={(e) =>
              onRefValuesChange({
                ...refValues,
                [fieldName]: { ...refState, mode: e.target.value as "name" | "id" },
              })
            }
          >
            <option value="name">name</option>
            <option value="id">id</option>
          </select>
          <input
            className="form-input"
            value={refState.value}
            onChange={(e) =>
              onRefValuesChange({
                ...refValues,
                [fieldName]: { ...refState, value: e.target.value, kind: refKind },
              })
            }
            list={refState.mode === "name" ? `fields-editor-ref-options-${fieldName}` : undefined}
            placeholder={refState.mode === "name" ? `Lookup ${refKind} name` : "UUID"}
          />
        </div>
        <datalist id={`fields-editor-ref-options-${fieldName}`}>
          {lookupOptions.map((opt) => (
            <option key={opt} value={opt} />
          ))}
        </datalist>
      </div>
    );
  }

  const fieldType = schemaValueType(schema);
  const label = String(schema.title ?? fieldName);
  const placeholder = String(schema.description ?? "");

  if (fieldType === "rich_text") {
    return (
      <div key={fieldName} className="form-field">
        <label className="form-label" style={{ textTransform: "none", letterSpacing: 0 }}>
          {label}
        </label>
        <RichTextField
          value={values[fieldName] ?? ""}
          onChange={setValue}
          ariaLabel={label}
          placeholder={placeholder}
        />
      </div>
    );
  }

  if (fieldType === "image") {
    return (
      <ImageField
        key={fieldName}
        label={label}
        ariaLabel={label}
        value={values[fieldName] ?? ""}
        onChange={setValue}
      />
    );
  }

  if (fieldType === "boolean") {
    return (
      <label
        key={fieldName}
        className="form-label"
        style={{ textTransform: "none", letterSpacing: 0, fontWeight: 500 }}
      >
        <input
          type="checkbox"
          checked={String(values[fieldName] ?? "false").toLowerCase() === "true"}
          onChange={(e) => setValue(e.target.checked ? "true" : "false")}
          style={{ marginRight: 8 }}
        />
        {label}
      </label>
    );
  }

  if (fieldType === "array" || fieldType === "object") {
    return (
      <div key={fieldName} className="form-field">
        <label className="form-label" style={{ textTransform: "none", letterSpacing: 0 }}>
          {label}
        </label>
        <JsonEditField
          value={values[fieldName] ?? ""}
          onChange={setValue}
          placeholder={
            placeholder ||
            (fieldType === "array" ? "[\n  {}\n]" : "{\n  \"key\": \"value\"\n}")
          }
          ariaLabel={label}
          minLines={4}
        />
      </div>
    );
  }

  const isLong = /contents|summary|text|body|description/i.test(fieldName);
  return (
    <div key={fieldName} className="form-field">
      <label className="form-label" style={{ textTransform: "none", letterSpacing: 0 }}>
        {label}
      </label>
      <textarea
        className="form-textarea"
        rows={isLong ? 4 : 2}
        value={values[fieldName] ?? ""}
        onChange={(e) => setValue(e.target.value)}
        placeholder={placeholder}
        spellCheck={fieldType !== "number" && fieldType !== "integer"}
      />
    </div>
  );
}
