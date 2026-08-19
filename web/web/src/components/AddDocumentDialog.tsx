/**
 * AddDocumentDialog — modal for creating a new document.
 *
 * Fields:
 *   - Name (required, e.g. "my-first-post")
 *   - Path (required, must start with "/", e.g. "/blog")
 *   - Title (optional)
 *   - Type (dropdown, populated from useDocumentTypes, defaults to "Post")
 *   - Summary (optional textarea)
 */

import { useEffect, useState } from "react";
import { useCreateDocument, useDocumentTypes } from "../api/hooks";

interface Props {
  open: boolean;
  onClose: () => void;
  defaultPath?: string;
  onCreated?: () => void;
}

export function AddDocumentDialog({ open, onClose, defaultPath, onCreated }: Props) {
  const [name, setName] = useState("");
  const [path, setPath] = useState(defaultPath ?? "/");
  const [title, setTitle] = useState("");
  /** Full type reference (e.g. "/types/docs/Post"), not a bare name. */
  const [typeRef, setTypeRef] = useState("");
  const [summary, setSummary] = useState("");

  const typesQuery = useDocumentTypes(500, { groups: "document-type" });
  const createMutation = useCreateDocument();

  // Sync default path when the dialog opens
  useEffect(() => {
    if (open) {
      setPath(defaultPath ?? "/");
      setName("");
      setTitle("");
      setTypeRef("");
      setSummary("");
      createMutation.reset();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  if (!open) return null;

  const handleCreate = async () => {
    const trimmedName = name.trim();
    const trimmedPath = path.trim();
    if (!trimmedName) {
      window.alert("Name is required.");
      return;
    }
    if (!trimmedPath) {
      window.alert("Path is required.");
      return;
    }
    if (!trimmedPath.startsWith("/")) {
      window.alert("Path must start with '/'.");
      return;
    }

    try {
      await createMutation.mutateAsync({
        path: trimmedPath,
        name: trimmedName,
        typeRef,
        title: title.trim() || undefined,
        summary: summary.trim() || undefined,
        contents: {},
      });
      onCreated?.();
      onClose();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      window.alert(`Failed to create document: ${message}`);
    }
  };

  const documentTypes = (typesQuery.data?.items ?? []).slice().sort((a, b) =>
    String(a.name ?? "").localeCompare(String(b.name ?? "")),
  );

  return (
    <>
      <div className="dialog-backdrop" onClick={onClose} />
      <div className="dialog" role="dialog" aria-label="Create document">
        <div className="dialog__header">
          <h2 className="dialog__title">New Document</h2>
          <button className="dialog__close" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </div>

        <div className="dialog__body">
          <label className="dialog__field">
            <span className="dialog__label">
              Name <span className="dialog__required">*</span>
            </span>
            <input
              className="dialog__input"
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="my-first-post"
              spellCheck={false}
              autoFocus
            />
          </label>

          <label className="dialog__field">
            <span className="dialog__label">
              Path <span className="dialog__required">*</span>
            </span>
            <input
              className="dialog__input"
              type="text"
              value={path}
              onChange={(e) => setPath(e.target.value)}
              placeholder="/blog"
              spellCheck={false}
            />
          </label>

          <label className="dialog__field">
            <span className="dialog__label">Title</span>
            <input
              className="dialog__input"
              type="text"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="My First Post"
            />
          </label>

          <label className="dialog__field">
            <span className="dialog__label">Type</span>
            <select
              className="dialog__input"
              value={typeRef}
              onChange={(e) => setTypeRef(e.target.value)}
            >
              <option value="">(none)</option>
              {documentTypes.map((t) => {
                const fullRef = `${t.path ?? ""}/${t.name ?? ""}`;
                return (
                  <option key={fullRef} value={fullRef}>
                    {String(t.name ?? "")}
                  </option>
                );
              })}
            </select>
          </label>

          <label className="dialog__field">
            <span className="dialog__label">Summary</span>
            <textarea
              className="dialog__textarea"
              value={summary}
              onChange={(e) => setSummary(e.target.value)}
              rows={3}
              placeholder="Optional summary used by search and listing"
            />
          </label>

          {createMutation.isError && (
            <div className="dialog__error">
              {createMutation.error instanceof Error
                ? createMutation.error.message
                : String(createMutation.error)}
            </div>
          )}
        </div>

        <div className="dialog__footer">
          <button className="dialog__btn" onClick={onClose} disabled={createMutation.isPending}>
            Cancel
          </button>
          <button
            className="dialog__btn dialog__btn--primary"
            onClick={handleCreate}
            disabled={createMutation.isPending}
          >
            {createMutation.isPending ? "Creating…" : "Create"}
          </button>
        </div>
      </div>
    </>
  );
}
