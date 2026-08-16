/**
 * FilesField — picker + uploader for files attached to a document.
 *
 * Replaces the legacy `artifact_names` chip input. Each entry in the
 * resulting array is a `FileRef`:
 *   { name: string, relPath: string, contentType?: string }
 *
 * The user can:
 *   - Pick from files already in the `solx-files` store (`useFilesList("")`)
 *   - Upload a new file from disk right here, which is then auto-attached
 *
 * The default upload target is `files/docs/<doc>/<name>` (mirrors the
 * conventional solx-files layout). The caller can override the prefix via
 * the `uploadPrefix` prop (e.g. `"files/actions/<action>"`).
 *
 * Wire format on save: passed up as `FileRef[]` in the document input.
 * On load, accepts either the wire format or a legacy `{ name: string }`
 * shape and normalizes both into `FileRef[]`.
 */

import { fileRawUrl } from "../api/api";
import { useEffect, useMemo, useRef, useState } from "react";
import { useFilesList, useUploadFile } from "../api/hooks";

export interface FileRef {
  name: string;
  relPath: string;
  contentType?: string;
}

interface Props {
  value: FileRef[];
  onChange: (next: FileRef[]) => void;
  /** Label shown above the picker. */
  label?: string;
  /** Disable all interactions (read-only display). */
  readOnly?: boolean;
  /** Default target directory when uploading from disk. */
  uploadPrefix?: string;
}

function contentTypeFromName(name: string): string | undefined {
  const lower = name.toLowerCase();
  if (lower.endsWith(".png")) return "image/png";
  if (lower.endsWith(".jpg") || lower.endsWith(".jpeg")) return "image/jpeg";
  if (lower.endsWith(".gif")) return "image/gif";
  if (lower.endsWith(".webp")) return "image/webp";
  if (lower.endsWith(".svg")) return "image/svg+xml";
  if (lower.endsWith(".pdf")) return "application/pdf";
  if (lower.endsWith(".md") || lower.endsWith(".txt")) return "text/plain";
  if (lower.endsWith(".html") || lower.endsWith(".htm")) return "text/html";
  if (lower.endsWith(".json")) return "application/json";
  if (lower.endsWith(".wasm")) return "application/wasm";
  if (lower.endsWith(".solx")) return "text/x-solx";
  return undefined;
}

function basename(relPath: string): string {
  const idx = Math.max(relPath.lastIndexOf("/"), relPath.lastIndexOf("\\"));
  return idx >= 0 ? relPath.slice(idx + 1) : relPath;
}

function isImageType(contentType: string | undefined, name: string): boolean {
  if (contentType && contentType.startsWith("image/")) return true;
  const lower = name.toLowerCase();
  return /\.(png|jpe?g|gif|webp|svg|avif|bmp|ico)$/.test(lower);
}

export function FilesField({
  value,
  onChange,
  label = "Attached files",
  readOnly,
  uploadPrefix = "files/docs/shared/",
}: Props) {
  const filesQuery = useFilesList("");
  const available = (filesQuery.data?.paths ?? []) as string[];

  const attachedRelPaths = useMemo(
    () => new Set(value.map((v) => v.relPath)),
    [value],
  );

  const handleAdd = (relPath: string) => {
    if (attachedRelPaths.has(relPath)) return;
    const name = basename(relPath);
    const next: FileRef = {
      name,
      relPath,
      contentType: contentTypeFromName(name),
    };
    onChange([...value, next]);
  };

  const handleRemove = (relPath: string) => {
    onChange(value.filter((v) => v.relPath !== relPath));
  };

  const suggestions = available.filter((p) => !attachedRelPaths.has(p));

  // ── Upload state ─────────────────────────────────────────────────────
  const uploadMutation = useUploadFile();
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const [pendingFile, setPendingFile] = useState<File | null>(null);
  const [relPathInput, setRelPathInput] = useState(uploadPrefix);
  const [uploadError, setUploadError] = useState<string | null>(null);

  // Keep the prefix in sync when the caller changes it.
  useEffect(() => {
    setRelPathInput((current) => (current.endsWith("/") ? uploadPrefix : current));
  }, [uploadPrefix]);

  const handlePickFile = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    setPendingFile(file);
    setUploadError(null);
    // If the user hasn't customized the path beyond the prefix, suggest
    // the file's name under that prefix.
    if (!relPathInput || relPathInput === uploadPrefix || relPathInput.endsWith("/")) {
      const base = (relPathInput || uploadPrefix).replace(/\/+$/, "");
      setRelPathInput(`${base}/${file.name}`);
    }
    // Reset the input so re-picking the same file re-fires onChange.
    e.target.value = "";
  };

  const handleUpload = async () => {
    setUploadError(null);
    if (!pendingFile) {
      setUploadError("Pick a file to upload.");
      return;
    }
    const target = relPathInput.trim();
    if (!target) {
      setUploadError("relPath is required.");
      return;
    }
    const bytes = new Uint8Array(await pendingFile.arrayBuffer());
    try {
      const result = await uploadMutation.mutateAsync({ relPath: target, bytes });
      const name = basename(result.relPath);
      const ref: FileRef = {
        name,
        relPath: result.relPath,
        contentType: contentTypeFromName(name) || pendingFile.type || undefined,
      };
      // Auto-attach if not already attached.
      if (!attachedRelPaths.has(result.relPath)) {
        onChange([...value, ref]);
      }
      setPendingFile(null);
      // Reset the relPath back to the prefix so the next upload defaults sensibly.
      setRelPathInput(uploadPrefix);
      if (fileInputRef.current) fileInputRef.current.value = "";
    } catch (err) {
      setUploadError(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <div className="form-field">
      <label className="form-label">{label}</label>

      {value.length === 0 ? (
        <div className="dialog__hint">No files attached.</div>
      ) : (
        <ul className="files-field__list">
          {value.map((entry) => {
            const isImage = isImageType(entry.contentType, entry.name);
            return (
              <li key={entry.relPath} className="files-field__entry">
                {isImage && (
                  <img
                    className="files-field__entry-thumb"
                    src={fileRawUrl(entry.relPath)}
                    alt={entry.name}
                    loading="lazy"
                  />
                )}
                <div className="files-field__entry-main">
                  <span className="files-field__entry-name">{entry.name}</span>
                  <span className="files-field__entry-path">{entry.relPath}</span>
                  {entry.contentType && (
                    <span className="files-field__entry-ctype">{entry.contentType}</span>
                  )}
                </div>
                {!readOnly && (
                  <button
                    type="button"
                    className="files-field__entry-remove"
                    onClick={() => handleRemove(entry.relPath)}
                    aria-label={`Detach ${entry.name}`}
                    title="Detach"
                  >
                    ×
                  </button>
                )}
              </li>
            );
          })}
        </ul>
      )}

      {!readOnly && (
        <>
          <div className="files-field__pick">
            <select
              className="form-input"
              value=""
              onChange={(e) => {
                const relPath = e.target.value;
                if (relPath) handleAdd(relPath);
                e.target.value = "";
              }}
              disabled={suggestions.length === 0}
            >
              <option value="">
                {suggestions.length === 0
                  ? "No more files available — upload one below"
                  : "Attach existing file…"}
              </option>
              {suggestions.map((relPath) => (
                <option key={relPath} value={relPath}>
                  {basename(relPath)} — {relPath}
                </option>
              ))}
            </select>
          </div>

          <div className="files-field__upload">
            <div className="files-field__upload-row">
              <input
                type="file"
                ref={fileInputRef}
                className="files-field__upload-file"
                onChange={handlePickFile}
                aria-label="Pick a file to upload"
              />
              <input
                type="text"
                className="form-input files-field__upload-path"
                value={relPathInput}
                onChange={(e) => setRelPathInput(e.target.value)}
                placeholder="files/docs/shared/example.png"
                spellCheck={false}
                aria-label="Upload destination"
              />
              <button
                type="button"
                className="files-field__upload-btn"
                onClick={handleUpload}
                disabled={!pendingFile || uploadMutation.isPending}
              >
                {uploadMutation.isPending ? "Uploading…" : "Upload"}
              </button>
            </div>
            {pendingFile && (
              <div className="files-field__upload-meta">
                Picked <code>{pendingFile.name}</code>{" "}
                ({Math.max(1, Math.round(pendingFile.size / 1024))} KB
                {pendingFile.type ? `, ${pendingFile.type}` : ""})
              </div>
            )}
            {uploadError && (
              <div className="files-field__upload-error">{uploadError}</div>
            )}
          </div>
          <div className="dialog__hint">
            Files live in the <code>files/</code> directory of the solx
            files root. Upload here or via the Files tab, then attach to
            this document.
          </div>
        </>
      )}
    </div>
  );
}

/** Normalize either a `FileRef[]` or legacy `{name}[]` shape. */
export function coerceFilesField(raw: unknown): FileRef[] {
  if (!Array.isArray(raw)) return [];
  const out: FileRef[] = [];
  for (const item of raw) {
    if (!item || typeof item !== "object") continue;
    const obj = item as Record<string, unknown>;
    const relPath = String(obj.relPath ?? obj.rel_path ?? "").trim();
    if (!relPath) continue;
    const name = String(obj.name ?? basename(relPath));
    const rawType = obj.contentType ?? obj.content_type;
    const contentType = typeof rawType === "string" && rawType.length > 0 ? rawType : undefined;
    out.push({ name, relPath, contentType });
  }
  return out;
}
