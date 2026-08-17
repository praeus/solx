/**
 * DocumentEditor — full editor for solx Documents.
 *
 * Architecture: fully controlled React state, no service layer.
 *
 * Top-level fields: name, path, title, summary, author, pub_date, confidence,
 * type. (KnowledgeBase was a legacy concept from sol-browser; we don't
 * carry it over — documents are scoped by their `path`.)
 *
 * Schema-driven `contents` editor introspects the selected Type's schema
 * and dispatches per-field rendering to one of:
 *
 *   - text / number / integer → simple `<textarea>`
 *   - boolean → checkbox
 *   - array / object → `<JsonEditField>` (CodeMirror 6 with JSON linter)
 *   - rich_text (matched by `x-sol-editor: "rich_text"` or `$ref` to
 *     `RichTextDoc`) → `<RichTextField>` (Tiptap)
 *   - DocRef / ActionRef (matched via `$ref`, `allOf[*].$ref`, or
 *     `anyOf[*].$ref`) → name-or-id input + datalist of entity names
 *
 * Modes: Fields ↔ JSON (whole document), plus "Show helper fields"
 * which reveals `x-sol-hidden: true` fields or {chapters, metadata}.
 *
 * CompoundDocument sections: when type is `CompoundDocument` and the
 * document is persisted, list DocRef rows as clickable buttons.
 *
 * Files: instead of legacy `artifact_names`, the editor maintains a
 * `FileRef[]` (name, relPath, contentType) attached via `<FilesField>`
 * which pulls from `useFilesList("")`. The wire shape on save is
 * `files: FileRef[]`.
 *
 * Preview: the right column renders the document via
 * `<DocumentPreview>`, which runs the type's `x-sol-template` (or
 * `x-sol-preview`) through EJS on the frontend. Falls back to a
 * built-in dump when no template is set.
 */

import { useEffect, useMemo, useState } from "react";
import { searchDocuments, type DocumentSummary } from "../api/api";
import {
  useActions,
  useCreateDocument,
  useDocuments,
  useDocumentTypes,
  useUpdateDocument,
  useType,
} from "../api/hooks";
import { DocumentPreview } from "./DocumentPreview";
import { FilesField, coerceFilesField, type FileRef } from "./FilesField";
import { ImageField } from "./ImageField";
import { JsonEditField } from "./JsonEditField";
import { RichTextField } from "./RichTextField";

const DOC_BUILTIN_TYPES = new Set([
  "Action",
  "Document",
  "Artifact",
  "Type",
  "DocRef",
  "ActionRef",
  "Model",
  "ModelRole",
  "Permission",
  "Schedule",
]);

const FALLBACK_DOC_SCHEMA = {
  type: "object",
  properties: {},
  required: [],
};

const HELPER_FIELD_NAMES = new Set(["chapters", "metadata"]);

type RefKind = "DocRef" | "ActionRef";

interface RefFieldValue {
  kind: RefKind;
  mode: "name" | "id";
  value: string;
}

interface CompoundSection {
  name: string;
  targetDocName: string;
  fieldName: string;
}

interface Props {
  open: boolean;
  onClose: () => void;
  /** When provided, the editor is in edit mode; when null, create mode. */
  existing: { path: string; name: string } | null;
  /** Optional preloaded entity (already fetched by the parent). */
  existingEntity?: DocumentSummary | null;
  /** Default path for create mode (e.g. from the current browse). */
  defaultPath?: string;
  onSaved?: (saved: { path: string; name: string }) => void;
}

export function DocumentEditor({
  open,
  onClose,
  existing,
  existingEntity,
  defaultPath,
  onSaved,
}: Props) {
  const isEdit = existing !== null;
  const typesQuery = useDocumentTypes();
  const documentsListQuery = useDocuments({ limit: 500, sort_by: "name", sort_order: "asc" });
  const actionsListQuery = useActions({ limit: 500, sort_by: "name", sort_order: "asc" });
  const createMutation = useCreateDocument();
  const updateMutation = useUpdateDocument();

  // ── Top-level fields ──
  const [path, setPath] = useState(defaultPath ?? "/");
  const [name, setName] = useState("");
  const [title, setTitle] = useState("");
  const [summary, setSummary] = useState("");
  const [author, setAuthor] = useState("");
  const [pubDate, setPubDate] = useState("");
  const [confidence, setConfidence] = useState("");
  const [typeName, setTypeName] = useState("Post");

  // ── Contents schema-driven editor ──
  const [docMode, setDocMode] = useState<"fields" | "json">("fields");
  const [showHiddenDocFields, setShowHiddenDocFields] = useState(false);
  const [docContentsText, setDocContentsText] = useState(
    JSON.stringify(existingEntity?.contents ?? {}, null, 2),
  );
  const [docFieldValues, setDocFieldValues] = useState<Record<string, string>>({});
  const [docRefValues, setDocRefValues] = useState<Record<string, RefFieldValue>>({});
  const [attachedFiles, setAttachedFiles] = useState<FileRef[]>([]);
  const [compoundSections, setCompoundSections] = useState<CompoundSection[]>([]);
  const [previewTab, setPreviewTab] = useState<"preview" | "json">("preview");

  const [error, setError] = useState<string | null>(null);

  // Look up the full TypeEntity (with its schema) for the currently selected
  // type. We first find the type's full path from the list query, then
  // fetch the detail (which carries the schema). This avoids hardcoding
  // `/builtin/types` — types live under `/packages/<name>` for installed
  // packages and `/builtin/types` for built-ins.
  const currentTypeListEntry = useMemo(
    () => typesQuery.data?.items?.find((t) => t.name === typeName) ?? null,
    [typesQuery.data, typeName],
  );
  const currentTypeQuery = useType(
    currentTypeListEntry?.path,
    typeName || undefined,
  );

  // Hydrate when dialog opens.
  useEffect(() => {
    if (!open) return;
    setError(null);
    if (existing && existingEntity) {
      setPath(existingEntity.path);
      setName(existingEntity.name);
      setTitle(String(existingEntity.title ?? ""));
      setSummary(String(existingEntity.summary ?? ""));
      setAuthor(String(existingEntity.author ?? ""));
      setPubDate(String(existingEntity.pub_date ?? ""));
      setConfidence(
        existingEntity.confidence != null ? String(existingEntity.confidence) : "",
      );
      // Backend returns `typeRef` (camelCase). Strip the leading path so
      // the type selector matches the bare name (e.g. "/types/docs/Post"
      // → "Post"). Falls back to the raw value if the slash isn't there.
      const rawTypeRef = String(
        (existingEntity as Record<string, unknown>).typeRef ??
          (existingEntity as Record<string, unknown>).type_ref ??
          "Post",
      );
      setTypeName(rawTypeRef.replace(/^.*\//, "") || "Post");

      const doc = (existingEntity.contents ?? {}) as Record<string, unknown>;
      setDocContentsText(JSON.stringify(doc, null, 2));

      const out: Record<string, string> = {};
      const refOut: Record<string, RefFieldValue> = {};
      for (const [key, value] of Object.entries(doc)) {
        if (value && typeof value === "object" && !Array.isArray(value)) {
          const obj = value as Record<string, unknown>;
          if (typeof obj.name === "string") {
            refOut[key] = { kind: "DocRef", mode: "name", value: obj.name };
            continue;
          }
          if (typeof obj.id === "string") {
            refOut[key] = { kind: "DocRef", mode: "id", value: obj.id };
            continue;
          }
        }
        const valueType = Array.isArray(value) ? "array" : typeof value;
        out[key] = valueFromUnknown(value, valueType);
      }
      setDocFieldValues(out);
      setDocRefValues(refOut);

      // The wire field is `files: FileRef[]`. Accept either that or a
      // legacy `{name}` array.
      setAttachedFiles(coerceFilesField(existingEntity.files));
    } else if (existing) {
      setPath(existing.path);
      setName(existing.name);
    } else {
      // Create mode.
      setPath(defaultPath ?? "/");
      setName("");
      setTitle("");
      setSummary("");
      setAuthor("");
      setPubDate("");
      setConfidence("");
      setTypeName("Post");
      setDocContentsText("{}");
      setDocFieldValues({});
      setDocRefValues({});
      setAttachedFiles([]);
    }
  }, [open, existing, existingEntity, defaultPath]);

  // ── Lookups for type combos / ref fields ──
  const documentNames = useMemo(
    () =>
      (documentsListQuery.data?.items ?? []).map((d) => String(d.name ?? "")).filter(Boolean),
    [documentsListQuery.data],
  );
  const actionNames = useMemo(
    () =>
      (actionsListQuery.data?.items ?? []).map((a) => String(a.name ?? "")).filter(Boolean),
    [actionsListQuery.data],
  );
  // We don't surface the full file-list here — `FilesField` has its own
  // picker driven by `useFilesList` internally.

  // ── Current type's schema ──
  const currentType = currentTypeQuery.data ?? null;
  const currentDocSchema = useMemo(
    () => currentType?.schema ?? FALLBACK_DOC_SCHEMA,
    [currentType],
  );
  const currentDocProperties = useMemo(
    () => docPropertySchema(currentDocSchema),
    [currentDocSchema],
  );

  // ── CompoundDocument sections ──
  useEffect(() => {
    if (typeName !== "CompoundDocument" || !existing?.name) {
      setCompoundSections([]);
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const data = (await searchDocuments({ type_ref: "DocRef", limit: 50 })) as {
          hits?: Array<{
            name?: string;
            target_document_name?: string;
            field_name?: string;
          }>;
        };
        if (cancelled) return;
        const sections: CompoundSection[] = (data.hits ?? [])
          .map((row) => ({
            name: String(row.name ?? ""),
            targetDocName: String(row.target_document_name ?? ""),
            fieldName: String(row.field_name ?? ""),
          }))
          .filter((s) => s.targetDocName);
        setCompoundSections(sections);
      } catch {
        if (!cancelled) setCompoundSections([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [typeName, existing?.name, existing?.path]);

  // ── Hydrate field/ref values when schema changes ──
  useEffect(() => {
    if (Object.keys(currentDocProperties).length === 0) return;
    let currentContents = {} as Record<string, unknown>;
    try {
      currentContents = JSON.parse(docContentsText) as Record<string, unknown>;
    } catch {
      currentContents = {};
    }
    const nextFieldValues: Record<string, string> = { ...docFieldValues };
    const nextRefValues: Record<string, RefFieldValue> = { ...docRefValues };
    for (const [fieldName, schema] of Object.entries(currentDocProperties)) {
      const refKind = refKindFromSchema(schema);
      if (refKind) {
        const existingRef = nextRefValues[fieldName];
        if (!existingRef) {
          const source = currentContents[fieldName] as Record<string, unknown> | undefined;
          if (source && typeof source === "object") {
            if (typeof source.name === "string") {
              nextRefValues[fieldName] = { kind: refKind, mode: "name", value: source.name };
            } else if (typeof source.id === "string") {
              nextRefValues[fieldName] = { kind: refKind, mode: "id", value: source.id };
            } else {
              nextRefValues[fieldName] = { kind: refKind, mode: "name", value: "" };
            }
          } else {
            nextRefValues[fieldName] = { kind: refKind, mode: "name", value: "" };
          }
        } else if (existingRef.kind !== refKind) {
          nextRefValues[fieldName] = { ...existingRef, kind: refKind };
        }
        continue;
      }
      if (fieldName in nextFieldValues) continue;
      const fieldType = schemaValueType(schema);
      nextFieldValues[fieldName] = valueFromUnknown(currentContents[fieldName], fieldType);
    }
    setDocFieldValues(nextFieldValues);
    setDocRefValues(nextRefValues);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentDocSchema]);

  // ── Build `contents` from field state ──
  const builtContents = useMemo<Record<string, unknown>>(() => {
    if (docMode === "json" || Object.keys(currentDocProperties).length === 0) {
      try {
        const parsed = JSON.parse(docContentsText);
        if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
          return parsed as Record<string, unknown>;
        }
      } catch {
        /* fall through */
      }
      return {};
    }
    const contents: Record<string, unknown> = {};
    for (const [fieldName, schema] of Object.entries(currentDocProperties)) {
      const refKind = refKindFromSchema(schema);
      if (refKind) {
        const refValue = docRefValues[fieldName];
        if (!refValue || !refValue.value.trim()) {
          contents[fieldName] = null;
        } else {
          contents[fieldName] = { [refValue.mode]: refValue.value.trim() };
        }
        continue;
      }
      const rawValue = docFieldValues[fieldName] ?? "";
      const raw = rawValue.trim();
      const fieldType = schemaValueType(schema);
      if (!raw) {
        if (fieldType === "boolean") contents[fieldName] = false;
        else if (fieldType === "rich_text")
          contents[fieldName] = JSON.parse(emptyRichTextValue());
        else contents[fieldName] = null;
        continue;
      }
      if (fieldType === "number" || fieldType === "integer") {
        const numericValue = Number(raw);
        contents[fieldName] = Number.isFinite(numericValue)
          ? fieldType === "integer"
            ? Math.trunc(numericValue)
            : numericValue
          : null;
      } else if (fieldType === "boolean") {
        contents[fieldName] = raw.toLowerCase() === "true";
      } else if (fieldType === "array" || fieldType === "object" || fieldType === "rich_text") {
        try {
          contents[fieldName] = JSON.parse(raw);
        } catch {
          contents[fieldName] = fieldType === "rich_text" ? JSON.parse(emptyRichTextValue()) : null;
        }
      } else {
        contents[fieldName] = raw;
      }
    }
    return contents;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    docMode,
    docContentsText,
    docFieldValues,
    docRefValues,
    currentDocProperties,
    currentDocSchema,
  ]);

  const contentsIsValidJson = useMemo(() => {
    if (docMode !== "json") return true;
    try {
      JSON.parse(docContentsText);
      return true;
    } catch {
      return false;
    }
  }, [docMode, docContentsText]);

  const canSave =
    name.trim().length > 0 &&
    path.trim().startsWith("/") &&
    typeName.trim().length > 0 &&
    contentsIsValidJson &&
    !createMutation.isPending &&
    !updateMutation.isPending;

  const handleSave = async () => {
    setError(null);
    const trimmedPath = path.trim();
    const trimmedName = name.trim();
    const trimmedType = typeName.trim() || "Post";
    if (!trimmedPath || !trimmedName) {
      setError("Name and path are required.");
      return;
    }
    if (!trimmedPath.startsWith("/")) {
      setError("Path must start with '/'.");
      return;
    }
    try {
      if (isEdit && existing) {
        const result = await updateMutation.mutateAsync({
          path: existing.path,
          name: existing.name,
          input: {
            type_name: trimmedType,
            title: title.trim() || undefined,
            summary: summary.trim() || undefined,
            author: author.trim() || undefined,
            pub_date: pubDate.trim() || undefined,
            confidence: confidence.trim() ? Number(confidence.trim()) : undefined,
            contents: builtContents,
            files: attachedFiles,
          },
        });
        onSaved?.({ path: result.path, name: result.name });
      } else {
        const result = await createMutation.mutateAsync({
          path: trimmedPath,
          name: trimmedName,
          type_name: trimmedType,
          title: title.trim() || undefined,
          summary: summary.trim() || undefined,
          author: author.trim() || undefined,
          pub_date: pubDate.trim() || undefined,
          confidence: confidence.trim() ? Number(confidence.trim()) : undefined,
          contents: builtContents,
          files: attachedFiles,
        });
        onSaved?.({ path: result.path, name: result.name });
      }
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  // Preview doc shape — used by the right-pane preview. In create mode we
  // use a synthetic DocumentSummary so the template still renders.
  const previewDocument: DocumentSummary = useMemo(() => {
    if (existingEntity) return existingEntity;
    return {
      path,
      name,
      title,
      summary,
      contents: builtContents,
      files: attachedFiles,
    } as DocumentSummary;
  }, [existingEntity, path, name, title, summary, builtContents, attachedFiles]);

  if (!open) return null;

  const isBusy = createMutation.isPending || updateMutation.isPending;
  const hasSchema = Object.keys(currentDocProperties).length > 0;
  const typeOptions = (typesQuery.data?.items ?? [])
    .map((t) => String(t.name ?? ""))
    .filter(Boolean);

  return (
    <>
      <div className="dialog-backdrop" onClick={onClose} />
      <div className="dialog dialog--xxl" role="dialog" aria-label="Document editor">
        <div className="dialog__header">
          <h2 className="dialog__title">{isEdit ? "Edit Document" : "New Document"}</h2>
          <button className="dialog__close" onClick={onClose} aria-label="Close">
            ×
          </button>
        </div>

        <div className="dialog__body">
          <div className="doc-editor-layout">
            {/* ── LEFT: form pane ── */}
            <div className="doc-editor-pane">
              <div className="form-field">
                <label className="form-label">
                  Name <span className="required-star">*</span>
                </label>
                <input
                  className="form-input"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="e.g. my-first-post"
                  spellCheck={false}
                  disabled={isEdit}
                  autoFocus
                />
              </div>

              <div className="form-field">
                <label className="form-label">
                  Path <span className="required-star">*</span>
                </label>
                <input
                  className="form-input"
                  value={path}
                  onChange={(e) => setPath(e.target.value)}
                  placeholder="/research"
                  spellCheck={false}
                  disabled={isEdit}
                />
              </div>

              <div className="form-field">
                <label className="form-label">Title</label>
                <input
                  className="form-input"
                  value={title}
                  onChange={(e) => setTitle(e.target.value)}
                  placeholder="Optional display title"
                />
              </div>

              <div className="form-field">
                <label className="form-label">Summary</label>
                <textarea
                  className="form-textarea"
                  rows={3}
                  value={summary}
                  onChange={(e) => setSummary(e.target.value)}
                  placeholder="Optional summary used by search and listing"
                />
              </div>

              <div className="form-field">
                <label className="form-label">Author</label>
                <input
                  className="form-input"
                  value={author}
                  onChange={(e) => setAuthor(e.target.value)}
                  placeholder="Optional author name"
                />
              </div>

              <div className="doc-editor-section-sep">Structure</div>

              <div className="form-field">
                <label className="form-label">
                  Type <span className="required-star">*</span>
                </label>
                <select
                  className="form-input"
                  value={typeName}
                  onChange={(e) => setTypeName(e.target.value)}
                >
                  {!typeOptions.includes("Post") && <option value="Post">Post</option>}
                  {typeOptions.map((n) => (
                    <option key={n} value={n}>
                      {n}
                    </option>
                  ))}
                </select>
                {DOC_BUILTIN_TYPES.has(typeName) && (
                  <div className="dialog__hint">
                    Type "{typeName}" is a built-in entity type and may not include a document schema.
                  </div>
                )}
              </div>

              <div className="form-field">
                <label className="form-label">Publication Date</label>
                <input
                  className="form-input"
                  value={pubDate}
                  onChange={(e) => setPubDate(e.target.value)}
                  placeholder="YYYY-MM (e.g. 2024-03)"
                  pattern="\d{4}(-\d{2})?"
                  spellCheck={false}
                />
              </div>

              <div className="form-field">
                <label className="form-label">Confidence</label>
                <input
                  className="form-input"
                  type="number"
                  min={0}
                  max={1}
                  step={0.01}
                  value={confidence}
                  onChange={(e) => setConfidence(e.target.value)}
                  placeholder="0.0 – 1.0"
                />
              </div>

              <div className="doc-editor-section-sep">Contents</div>

              {typeName === "CompoundDocument" && compoundSections.length > 0 && (
                <div className="form-field">
                  <label className="form-label">
                    Sections ({compoundSections.length})
                  </label>
                  <div className="doc-editor-compound-list">
                    {compoundSections.map((section, index) => (
                      <button
                        key={`${section.name}-${index}`}
                        type="button"
                        className="doc-editor-compound-item"
                        title={`Open ${section.targetDocName}`}
                      >
                        <span className="doc-editor-compound-item__label">
                          {section.fieldName || `Section ${index + 1}`}
                        </span>
                        <span className="doc-editor-compound-item__target">
                          {section.targetDocName}
                        </span>
                      </button>
                    ))}
                  </div>
                </div>
              )}

              <div className="doc-editor-mode-toggle">
                <button
                  type="button"
                  className={`doc-editor-toggle-btn${docMode === "fields" ? " doc-editor-toggle-btn--active" : ""}`}
                  onClick={() => setDocMode("fields")}
                >
                  Fields
                </button>
                <button
                  type="button"
                  className={`doc-editor-toggle-btn${docMode === "json" ? " doc-editor-toggle-btn--active" : ""}`}
                  onClick={() => setDocMode("json")}
                >
                  JSON
                </button>
                <label className="doc-editor-helper-toggle">
                  <input
                    type="checkbox"
                    checked={showHiddenDocFields}
                    onChange={(e) => setShowHiddenDocFields(e.target.checked)}
                  />
                  Show helper fields
                </label>
              </div>

              {docMode === "fields" ? (
                !hasSchema ? (
                  <div className="dialog__hint">
                    No schema available for type "{typeName}". Use JSON mode.
                  </div>
                ) : (
                  <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
                    {Object.entries(currentDocProperties)
                      .filter(
                        ([fieldName, schema]) =>
                          showHiddenDocFields || !isHiddenDocField(fieldName, schema),
                      )
                      .map(([fieldName, schema]) =>
                        renderDocField({
                          fieldName,
                          schema,
                          docFieldValues,
                          setDocFieldValues,
                          docRefValues,
                          setDocRefValues,
                          documentNames,
                          actionNames,
                        }),
                      )}
                  </div>
                )
              ) : (
                <JsonEditField
                  value={docContentsText}
                  onChange={setDocContentsText}
                  placeholder={`{\n  "field": "value"\n}`}
                  ariaLabel="Document contents as JSON"
                  minLines={12}
                />
              )}

              <div className="doc-editor-section-sep">Files &amp; Data</div>

              <FilesField value={attachedFiles} onChange={setAttachedFiles} />

              {error && <div className="dialog__error">{error}</div>}
            </div>

            {/* ── RIGHT: preview pane ── */}
            <div className="doc-editor-pane doc-editor-pane--preview">
              <div className="doc-editor-tabs">
                <button
                  type="button"
                  className={`doc-editor-tab${previewTab === "preview" ? " doc-editor-tab--active" : ""}`}
                  onClick={() => setPreviewTab("preview")}
                >
                  Preview
                </button>
                <button
                  type="button"
                  className={`doc-editor-tab${previewTab === "json" ? " doc-editor-tab--active" : ""}`}
                  onClick={() => setPreviewTab("json")}
                >
                  Contents JSON
                </button>
              </div>

              {previewTab === "preview" ? (
                <DocumentPreview document={previewDocument} type={currentType} />
              ) : (
                <pre
                  style={{
                    margin: 0,
                    padding: 12,
                    background: "var(--bg-sunken)",
                    borderRadius: "var(--radius)",
                    fontSize: 12,
                    fontFamily: "var(--font-mono)",
                    overflowX: "auto",
                  }}
                >
                  {JSON.stringify(builtContents, null, 2)}
                </pre>
              )}
            </div>
          </div>
        </div>

        <div className="dialog__footer">
          <button className="dialog__btn" onClick={onClose} disabled={isBusy}>
            Cancel
          </button>
          <button
            className="dialog__btn dialog__btn--primary"
            onClick={handleSave}
            disabled={!canSave}
          >
            {isBusy ? "Saving…" : isEdit ? "Save" : "Create"}
          </button>
        </div>
      </div>
    </>
  );
}

// ───────────────────────────────────────────────────────────────────────
// Schema introspection helpers
// ───────────────────────────────────────────────────────────────────────

function docPropertySchema(typeSchema: unknown): Record<string, Record<string, unknown>> {
  if (!typeSchema || typeof typeSchema !== "object") return {};
  const props = (typeSchema as Record<string, unknown>).properties;
  if (!props || typeof props !== "object") return {};
  return props as Record<string, Record<string, unknown>>;
}

function refKindFromSchema(schema: Record<string, unknown>): RefKind | null {
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

function schemaRefTarget(schema: Record<string, unknown>): string {
  const direct = typeof schema.$ref === "string" ? schema.$ref : "";
  if (direct) return direct;
  const allOf = Array.isArray(schema.allOf) ? schema.allOf : [];
  for (const item of allOf) {
    if (item && typeof item === "object" && typeof (item as Record<string, unknown>).$ref === "string") {
      return String((item as Record<string, unknown>).$ref ?? "");
    }
  }
  const anyOf = Array.isArray(schema.anyOf) ? schema.anyOf : [];
  for (const item of anyOf) {
    if (item && typeof item === "object" && typeof (item as Record<string, unknown>).$ref === "string") {
      return String((item as Record<string, unknown>).$ref ?? "");
    }
  }
  return "";
}

function isRichTextSchema(schema: Record<string, unknown>): boolean {
  return schema["x-sol-editor"] === "rich_text" || schemaRefTarget(schema).includes("RichTextDoc");
}

function isImageSchema(schema: Record<string, unknown>): boolean {
  if (schema["x-sol-editor"] === "image") return true;
  const ref = schemaRefTarget(schema);
  // Generic heuristic: any $ref whose name contains "Image" or
  // "ArtifactRef" (the legacy image-ref type from sol-browser).
  return /Image$|ArtifactRef$/.test(ref);
}

function isSchemaHidden(schema: Record<string, unknown>): boolean {
  return schema["x-sol-hidden"] === true;
}

function isHiddenDocField(fieldName: string, schema: Record<string, unknown>): boolean {
  return isSchemaHidden(schema) || HELPER_FIELD_NAMES.has(fieldName);
}

function schemaValueType(schema: Record<string, unknown>): string {
  if (isRichTextSchema(schema)) return "rich_text";
  if (isImageSchema(schema)) return "image";
  if (typeof schema.type === "string") return schema.type;
  if (schemaRefTarget(schema)) return "object";
  return "string";
}

function emptyRichTextValue(): string {
  return JSON.stringify({ type: "doc", content: [] }, null, 2);
}

function valueFromUnknown(value: unknown, asType: string): string {
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

// ───────────────────────────────────────────────────────────────────────
// Per-field renderer — dispatches on schema type.
// ───────────────────────────────────────────────────────────────────────

interface FieldRenderContext {
  fieldName: string;
  schema: Record<string, unknown>;
  docFieldValues: Record<string, string>;
  setDocFieldValues: React.Dispatch<React.SetStateAction<Record<string, string>>>;
  docRefValues: Record<string, RefFieldValue>;
  setDocRefValues: React.Dispatch<React.SetStateAction<Record<string, RefFieldValue>>>;
  documentNames: string[];
  actionNames: string[];
}

function renderDocField(ctx: FieldRenderContext) {
  const {
    fieldName,
    schema,
    docFieldValues,
    setDocFieldValues,
    docRefValues,
    setDocRefValues,
    documentNames,
    actionNames,
  } = ctx;

  const refKind = refKindFromSchema(schema);
  if (refKind) {
    const refState =
      docRefValues[fieldName] ?? { kind: refKind, mode: "name" as const, value: "" };
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
              setDocRefValues((prev) => ({
                ...prev,
                [fieldName]: { ...refState, mode: e.target.value as "name" | "id" },
              }))
            }
          >
            <option value="name">name</option>
            <option value="id">id</option>
          </select>
          <input
            className="form-input"
            value={refState.value}
            onChange={(e) =>
              setDocRefValues((prev) => ({
                ...prev,
                [fieldName]: { ...refState, value: e.target.value, kind: refKind },
              }))
            }
            list={
              refState.mode === "name"
                ? `document-editor-ref-options-${fieldName}`
                : undefined
            }
            placeholder={refState.mode === "name" ? `Lookup ${refKind} name` : "UUID"}
          />
        </div>
        <datalist id={`document-editor-ref-options-${fieldName}`}>
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

  // rich_text → Tiptap editor.
  if (fieldType === "rich_text") {
    return (
      <div key={fieldName} className="form-field">
        <label className="form-label" style={{ textTransform: "none", letterSpacing: 0 }}>
          {label} <span className="dialog__hint">rich_text</span>
        </label>
        <RichTextField
          value={docFieldValues[fieldName] ?? ""}
          onChange={(next) =>
            setDocFieldValues((prev) => ({ ...prev, [fieldName]: next }))
          }
          ariaLabel={label}
          placeholder={placeholder}
        />
      </div>
    );
  }

  // image → ImageField (relPath string with live preview + upload).
  if (fieldType === "image") {
    return (
      <ImageField
        key={fieldName}
        label={label}
        ariaLabel={label}
        value={docFieldValues[fieldName] ?? ""}
        onChange={(next) =>
          setDocFieldValues((prev) => ({ ...prev, [fieldName]: next }))
        }
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
          checked={String(docFieldValues[fieldName] ?? "false").toLowerCase() === "true"}
          onChange={(e) =>
            setDocFieldValues((prev) => ({
              ...prev,
              [fieldName]: e.target.checked ? "true" : "false",
            }))
          }
          style={{ marginRight: 8 }}
        />
        {label}
      </label>
    );
  }

  // array / object → CodeMirror JSON editor with live parse badge.
  if (fieldType === "array" || fieldType === "object") {
    return (
      <div key={fieldName} className="form-field">
        <label className="form-label" style={{ textTransform: "none", letterSpacing: 0 }}>
          {label} <span className="dialog__hint">{fieldType}</span>
        </label>
        <JsonEditField
          value={docFieldValues[fieldName] ?? ""}
          onChange={(next) =>
            setDocFieldValues((prev) => ({ ...prev, [fieldName]: next }))
          }
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

  // text / number / integer → simple input/textarea.
  const isLong = /contents|summary|text|body|description/i.test(fieldName);
  return (
    <div key={fieldName} className="form-field">
      <label className="form-label" style={{ textTransform: "none", letterSpacing: 0 }}>
        {label} <span className="dialog__hint">{fieldType}</span>
      </label>
      <textarea
        className="form-textarea"
        rows={isLong ? 4 : 2}
        value={docFieldValues[fieldName] ?? ""}
        onChange={(e) =>
          setDocFieldValues((prev) => ({ ...prev, [fieldName]: e.target.value }))
        }
        placeholder={placeholder}
        spellCheck={fieldType !== "number" && fieldType !== "integer"}
      />
    </div>
  );
}
