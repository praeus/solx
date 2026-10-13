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
    <div className="fields-editor-table">
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

  // Wrap each field in a row with a left "label" cell and a right "control"
  // cell. We do this for every field type (including rich-text / json /
  // image) so the row layout is consistent. Each row also gets a description
  // line below the label, when one is provided in the schema.
  const renderLabel = (text: string) => (
    <label className="fields-editor-table__label" style={{ textTransform: "none", letterSpacing: 0 }}>
      {text}
    </label>
  );

  const refKind = refKindFromSchema(schema);
  if (refKind) {
    const refState =
      refValues[fieldName] ?? { kind: refKind, mode: "name" as const, value: "" };
    const lookupOptions = refKind === "DocRef" ? documentNames : actionNames;
    return (
      <div key={fieldName} className="form-field fields-editor-table__row">
        {renderLabel(`${String(schema.title ?? fieldName)} (${refKind})`)}
        <div className="fields-editor-table__control">
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
      </div>
    );
  }

  const fieldType = schemaValueType(schema);
  const label = String(schema.title ?? fieldName);
  const description = String(schema.description ?? "");
  const descriptionNode = description ? (
    <div className="fields-editor-table__description">{description}</div>
  ) : null;

  if (fieldType === "rich_text") {
    return (
      <div key={fieldName} className="form-field fields-editor-table__row">
        {renderLabel(label)}
        <div className="fields-editor-table__control">
          <RichTextField
            value={values[fieldName] ?? ""}
            onChange={setValue}
            ariaLabel={label}
            placeholder={description}
          />
          {descriptionNode}
        </div>
      </div>
    );
  }

  if (fieldType === "image") {
    return (
      <div key={fieldName} className="form-field fields-editor-table__row">
        {renderLabel(label)}
        <div className="fields-editor-table__control">
          <ImageField
            label=""
            ariaLabel={label}
            value={values[fieldName] ?? ""}
            onChange={setValue}
          />
          {descriptionNode}
        </div>
      </div>
    );
  }

  if (fieldType === "boolean") {
    return (
      <div key={fieldName} className="form-field fields-editor-table__row">
        {renderLabel(label)}
        <div className="fields-editor-table__control">
          <label
            className="fields-editor-table__inline-control"
            style={{ textTransform: "none", letterSpacing: 0, fontWeight: 500 }}
          >
            <input
              type="checkbox"
              checked={String(values[fieldName] ?? "false").toLowerCase() === "true"}
              onChange={(e) => setValue(e.target.checked ? "true" : "false")}
            />
            <span>{label}</span>
          </label>
          {descriptionNode}
        </div>
      </div>
    );
  }

  if (fieldType === "array" || fieldType === "object") {
    return (
      <div key={fieldName} className="form-field fields-editor-table__row">
        {renderLabel(label)}
        <div className="fields-editor-table__control">
          <JsonEditField
            value={values[fieldName] ?? ""}
            onChange={setValue}
            placeholder={
              description ||
              (fieldType === "array" ? "[\n  {}\n]" : `{\n  "key": "value"\n}`)
            }
            ariaLabel={label}
            minLines={4}
          />
          {descriptionNode}
        </div>
      </div>
    );
  }

  // Number/integer fields use a single-line <input type="number">; everything
  // else gets a plain <input> so short values render as a one-line box rather
  // than a 2-row textarea. Rich-text / array / object / image already
  // short-circuit above.
  const isNumber = fieldType === "number" || fieldType === "integer";
  return (
    <div key={fieldName} className="form-field fields-editor-table__row">
      {renderLabel(label)}
      <div className="fields-editor-table__control">
        <input
          className="form-input"
          type={isNumber ? "number" : "text"}
          value={values[fieldName] ?? ""}
          onChange={(e) => setValue(e.target.value)}
          placeholder={description}
          spellCheck={!isNumber}
        />
        {descriptionNode}
      </div>
    </div>
  );
}
