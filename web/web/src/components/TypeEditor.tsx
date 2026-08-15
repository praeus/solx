/**
 * TypeEditor — modal editor for solx Types.
 *
 * Fields: name (required, immutable after creation), description, groups (chips),
 * JSON Schema (Monaco editor).
 */

import { useEffect, useState } from "react";
import type { TypeEntity } from "../api/api";
import { useCreateType } from "../api/hooks";
import { ChipTagInput } from "./ChipTagInput";
import { JsonSchemaEditor } from "./JsonSchemaEditor";

interface Props {
  open: boolean;
  onClose: () => void;
  /** When provided, the editor is in edit mode; when null, create mode. */
  existing: { path: string; name: string } | null;
  /** Preloaded entity to hydrate edit-mode fields. */
  existingEntity?: TypeEntity | null;
  defaultPath?: string;
  onSaved?: (saved: { path: string; name: string }) => void;
}

export function TypeEditor({ open, onClose, existing, existingEntity, defaultPath, onSaved }: Props) {
  const isEdit = existing !== null;
  const createMutation = useCreateType();

  const [path, setPath] = useState(defaultPath ?? "/");
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [groups, setGroups] = useState<string[]>([]);
  const [schemaText, setSchemaText] = useState("");

  const [error, setError] = useState<string | null>(null);

  // Hydrate when dialog opens.
  useEffect(() => {
    if (!open) return;
    setError(null);
    if (existing && existingEntity) {
      setPath(existingEntity.path);
      setName(existingEntity.name);
      setDescription(String(existingEntity.description ?? ""));
      setGroups(Array.isArray(existingEntity.groups) ? existingEntity.groups.map(String) : []);
      const schema = existingEntity.schema ?? {};
      setSchemaText(Object.keys(schema).length > 0 ? JSON.stringify(schema, null, 2) : "");
    } else if (existing) {
      setPath(existing.path);
      setName(existing.name);
    } else {
      setPath(defaultPath ?? "/");
      setName("");
      setDescription("");
      setGroups([]);
      setSchemaText("");
    }
  }, [open, existing, existingEntity, defaultPath]);

  if (!open) return null;

  const handleSave = async () => {
    setError(null);
    const trimmedPath = path.trim();
    const trimmedName = name.trim();
    if (!trimmedPath) {
      setError("Path is required.");
      return;
    }
    if (!trimmedName) {
      setError("Name is required.");
      return;
    }
    if (!trimmedPath.startsWith("/")) {
      setError("Path must start with '/'.");
      return;
    }

    let schema: Record<string, unknown> | undefined;
    if (schemaText.trim()) {
      try {
        schema = JSON.parse(schemaText);
      } catch (e) {
        setError(`Schema JSON is invalid: ${(e as Error).message}`);
        return;
      }
    }

    try {
      const result = await createMutation.mutateAsync({
        path: trimmedPath,
        name: trimmedName,
        description: description.trim() || undefined,
        schema,
        groups: groups.length ? groups : undefined,
      });
      onSaved?.({ path: result.path, name: result.name });
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <>
      <div className="dialog-backdrop" onClick={onClose} />
      <div className="dialog dialog--wide" role="dialog" aria-label="Type editor">
        <div className="dialog__header">
          <h2 className="dialog__title">{isEdit ? "Edit Type" : "New Type"}</h2>
          <button className="dialog__close" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </div>

        <div className="dialog__body">
          <div className="form-field">
            <label className="form-label">
              Name <span className="required-star">*</span>
            </label>
            <input
              className="form-input"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="MyType"
              spellCheck={false}
              readOnly={isEdit}
              autoFocus={!isEdit}
            />
            {isEdit && (
              <div style={{ fontSize: 11, color: "var(--text-muted)", marginTop: 4 }}>
                Type names are immutable after creation.
              </div>
            )}
          </div>

          <div className="form-field">
            <label className="form-label">
              Path <span className="required-star">*</span>
            </label>
            <input
              className="form-input"
              value={path}
              onChange={(e) => setPath(e.target.value)}
              placeholder="/types/custom"
              spellCheck={false}
              readOnly={isEdit}
            />
          </div>

          <div className="form-field">
            <label className="form-label">Description</label>
            <textarea
              className="form-textarea"
              rows={2}
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="Free-form description"
            />
          </div>

          <div className="form-field">
            <label className="form-label">Groups</label>
            <div style={{ fontSize: 11, color: "var(--text-muted)", marginBottom: 4 }}>
              Tags used for facet grouping
            </div>
            <ChipTagInput
              chips={groups}
              onChange={setGroups}
              placeholder="core, custom, experimental…"
            />
          </div>

          <div className="form-field">
            <label className="form-label">JSON Schema</label>
            <div style={{ fontSize: 11, color: "var(--text-muted)", marginBottom: 4 }}>
              Draft-07 JSON Schema. Validated on save.
            </div>
            <JsonSchemaEditor
              value={schemaText}
              onChange={setSchemaText}
              height={320}
              placeholder='{"type": "object", "properties": { ... }}'
            />
          </div>

          {error && <div className="dialog__error">{error}</div>}
        </div>

        <div className="dialog__footer">
          <button className="dialog__btn" onClick={onClose} disabled={createMutation.isPending}>
            Cancel
          </button>
          <button
            className="dialog__btn dialog__btn--primary"
            onClick={() => void handleSave()}
            disabled={createMutation.isPending}
          >
            {createMutation.isPending ? "Saving…" : "Save"}
          </button>
        </div>
      </div>
    </>
  );
}
