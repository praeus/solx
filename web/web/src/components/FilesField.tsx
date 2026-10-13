/**
 * FilesField — picker + uploader for files attached to a document or action.
 *
 * Replaces the legacy `artifact_names` chip input. Each entry in the
 * resulting array is a `FileRef`:
 *   { name: string, relPath: string, contentType?: string }
 *
 * The user can:
 *   - Attach a file already in the `solx-files` store, picked from a tree
 *     dialog (`AttachExistingDialog`, driven by `useFilesList("")`)
 *   - Upload a new file from disk — via the "Upload new" button, the OS
 *     file picker, or a drag-and-drop onto the field. All three routes open
 *     `UploadNameDialog` to confirm/edit the destination path before the
 *     bytes are actually sent.
 *
 * The default upload target is `files/docs/<doc>/<name>` (mirrors the
 * conventional solx-files layout). The caller can override the prefix via
 * the `uploadPrefix` prop (e.g. `"files/actions/<action>"`).
 *
 * Wire format on save: passed up as `FileRef[]` in the document input.
 * On load, accepts either the wire format or a legacy `{ name: string }`
 * shape and normalizes both into `FileRef[]`.
 */

import { useMemo, useRef, useState } from "react";
import { useFileObjectUrl, useFilesList, useUploadFile } from "../api/hooks";

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
  if (lower.endsWith(".js") || lower.endsWith(".mjs")) return "text/javascript";
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

/** One attached-file row: thumbnail (if image) + name-as-hyperlink + path. */
function FileEntryRow({
  entry,
  onRemove,
  readOnly,
}: {
  entry: FileRef;
  onRemove: () => void;
  readOnly?: boolean;
}) {
  const url = useFileObjectUrl(entry.relPath);
  const isImage = isImageType(entry.contentType, entry.name);
  return (
    <li className="files-field__entry">
      {isImage &&
        (url ? (
          <img className="files-field__entry-thumb" src={url} alt={entry.name} loading="lazy" />
        ) : (
          <span className="files-field__entry-thumb files-field__entry-thumb--loading" />
        ))}
      <div className="files-field__entry-main">
        {url ? (
          <a
            className="files-field__entry-name files-field__entry-link"
            href={url}
            target="_blank"
            rel="noreferrer"
            title={`Open ${entry.relPath}`}
          >
            {entry.name}
          </a>
        ) : (
          <span className="files-field__entry-name">{entry.name}</span>
        )}
        <span className="files-field__entry-path">{entry.relPath}</span>
        {entry.contentType && (
          <span className="files-field__entry-ctype">{entry.contentType}</span>
        )}
      </div>
      {!readOnly && (
        <button
          type="button"
          className="files-field__entry-remove"
          onClick={onRemove}
          aria-label={`Detach ${entry.name}`}
          title="Detach"
        >
          ×
        </button>
      )}
    </li>
  );
}

/** ── Tree data for the "Attach existing" dialog ─────────────────────── */

interface TreeFolder {
  name: string;
  path: string;
  folders: Map<string, TreeFolder>;
  files: string[];
}

function buildTree(paths: string[]): TreeFolder {
  const root: TreeFolder = { name: "", path: "", folders: new Map(), files: [] };
  for (const p of paths) {
    const parts = p.split("/").filter(Boolean);
    if (parts.length === 0) continue;
    let node = root;
    for (let i = 0; i < parts.length - 1; i++) {
      const part = parts[i];
      const path = node.path ? `${node.path}/${part}` : part;
      let child = node.folders.get(part);
      if (!child) {
        child = { name: part, path, folders: new Map(), files: [] };
        node.folders.set(part, child);
      }
      node = child;
    }
    node.files.push(p);
  }
  return root;
}

function FileTreeFolder({
  folder,
  depth,
  expanded,
  forceExpand,
  onToggle,
  onSelect,
}: {
  folder: TreeFolder;
  depth: number;
  expanded: Set<string>;
  forceExpand: boolean;
  onToggle: (path: string) => void;
  onSelect: (relPath: string) => void;
}) {
  const sortedFolders = Array.from(folder.folders.values()).sort((a, b) =>
    a.name.localeCompare(b.name),
  );
  const sortedFiles = [...folder.files].sort((a, b) =>
    basename(a).localeCompare(basename(b)),
  );
  return (
    <>
      {sortedFolders.map((child) => {
        const isOpen = forceExpand || expanded.has(child.path);
        return (
          <div key={child.path} className="file-tree__node">
            <button
              type="button"
              className="file-tree__row file-tree__row--folder"
              style={{ paddingLeft: 8 + depth * 16 }}
              onClick={() => onToggle(child.path)}
            >
              <span className="file-tree__caret">{isOpen ? "▾" : "▸"}</span>
              <span className="file-tree__icon">📁</span>
              <span className="file-tree__label">{child.name}</span>
            </button>
            {isOpen && (
              <FileTreeFolder
                folder={child}
                depth={depth + 1}
                expanded={expanded}
                forceExpand={forceExpand}
                onToggle={onToggle}
                onSelect={onSelect}
              />
            )}
          </div>
        );
      })}
      {sortedFiles.map((relPath) => (
        <button
          key={relPath}
          type="button"
          className="file-tree__row file-tree__row--file"
          style={{ paddingLeft: 8 + depth * 16 + 20 }}
          onClick={() => onSelect(relPath)}
          title={relPath}
        >
          <span className="file-tree__icon">📄</span>
          <span className="file-tree__label">{basename(relPath)}</span>
        </button>
      ))}
    </>
  );
}

function AttachExistingDialog({
  paths,
  onClose,
  onSelect,
}: {
  paths: string[];
  onClose: () => void;
  onSelect: (relPath: string) => void;
}) {
  const [filter, setFilter] = useState("");
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  const filtered = useMemo(() => {
    const f = filter.trim().toLowerCase();
    if (!f) return paths;
    return paths.filter((p) => p.toLowerCase().includes(f));
  }, [paths, filter]);

  const tree = useMemo(() => buildTree(filtered), [filtered]);
  const forceExpand = filter.trim().length > 0;

  const toggle = (path: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  return (
    <>
      <div className="dialog-backdrop" onClick={onClose} />
      <div className="dialog dialog--wide" role="dialog" aria-label="Attach existing file">
        <div className="dialog__header">
          <h2 className="dialog__title">Attach existing file</h2>
          <button className="dialog__close" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </div>
        <div className="dialog__body">
          <input
            className="form-input"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder="Filter by path…"
            spellCheck={false}
            autoFocus
          />
          <div className="file-tree">
            {filtered.length === 0 ? (
              <div className="dialog__hint">No matching files.</div>
            ) : (
              <FileTreeFolder
                folder={tree}
                depth={0}
                expanded={expanded}
                forceExpand={forceExpand}
                onToggle={toggle}
                onSelect={onSelect}
              />
            )}
          </div>
        </div>
        <div className="dialog__footer">
          <button className="dialog__btn" onClick={onClose}>
            Cancel
          </button>
        </div>
      </div>
    </>
  );
}

function UploadNameDialog({
  fileName,
  relPath,
  onRelPathChange,
  onCancel,
  onUpload,
  uploading,
  error,
}: {
  fileName: string;
  relPath: string;
  onRelPathChange: (v: string) => void;
  onCancel: () => void;
  onUpload: () => void;
  uploading: boolean;
  error: string | null;
}) {
  return (
    <>
      <div className="dialog-backdrop" onClick={uploading ? undefined : onCancel} />
      <div className="dialog" role="dialog" aria-label="Upload file">
        <div className="dialog__header">
          <h2 className="dialog__title">Upload file</h2>
          <button className="dialog__close" onClick={onCancel} aria-label="Close" disabled={uploading}>
            ✕
          </button>
        </div>
        <div className="dialog__body">
          <div className="dialog__hint">
            Uploading <code>{fileName}</code>
          </div>
          <div className="form-field">
            <label className="form-label">Destination path</label>
            <input
              className="form-input"
              value={relPath}
              onChange={(e) => onRelPathChange(e.target.value)}
              spellCheck={false}
              autoFocus
              onKeyDown={(e) => {
                if (e.key === "Enter" && !uploading) onUpload();
              }}
            />
          </div>
          {error && <div className="dialog__error">{error}</div>}
        </div>
        <div className="dialog__footer">
          <button className="dialog__btn" onClick={onCancel} disabled={uploading}>
            Cancel
          </button>
          <button
            className="dialog__btn dialog__btn--primary"
            onClick={onUpload}
            disabled={uploading}
          >
            {uploading ? "Uploading…" : "Upload"}
          </button>
        </div>
      </div>
    </>
  );
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

  // ── Attach existing dialog ───────────────────────────────────────────
  const [pickerOpen, setPickerOpen] = useState(false);

  // ── Upload flow: pick (button / OS picker / drag-drop) → name → upload ─
  const uploadMutation = useUploadFile();
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const [uploadDialog, setUploadDialog] = useState<{ file: File; relPath: string } | null>(null);
  const [uploadError, setUploadError] = useState<string | null>(null);
  const [isDragOver, setIsDragOver] = useState(false);

  const startUploadFlow = (file: File) => {
    const base = uploadPrefix.replace(/\/+$/, "");
    setUploadDialog({ file, relPath: `${base}/${file.name}` });
    setUploadError(null);
  };

  const handlePickFile = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (file) startUploadFlow(file);
    e.target.value = "";
  };

  const handleDrop = (e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    setIsDragOver(false);
    if (readOnly) return;
    const file = e.dataTransfer.files?.[0];
    if (file) startUploadFlow(file);
  };

  const handleConfirmUpload = async () => {
    if (!uploadDialog) return;
    const target = uploadDialog.relPath.trim();
    if (!target) {
      setUploadError("Destination path is required.");
      return;
    }
    setUploadError(null);
    const bytes = new Uint8Array(await uploadDialog.file.arrayBuffer());
    try {
      const result = await uploadMutation.mutateAsync({ relPath: target, bytes });
      const name = basename(result.relPath);
      const ref: FileRef = {
        name,
        relPath: result.relPath,
        contentType: contentTypeFromName(name) || uploadDialog.file.type || undefined,
      };
      if (!attachedRelPaths.has(result.relPath)) {
        onChange([...value, ref]);
      }
      setUploadDialog(null);
    } catch (err) {
      setUploadError(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <div className="form-field">
      <label className="form-label">{label}</label>

      <div
        className={`files-field__dropzone${isDragOver ? " is-dragover" : ""}`}
        onDragOver={(e) => {
          if (readOnly) return;
          e.preventDefault();
          setIsDragOver(true);
        }}
        onDragLeave={() => setIsDragOver(false)}
        onDrop={handleDrop}
      >
        {value.length === 0 ? (
          <div className="files-field__empty">
            <div className="files-field__empty-title">No files attached</div>
            <div className="files-field__empty-hint">
              Drag &amp; drop a file here, or use the buttons below
            </div>
          </div>
        ) : (
          <ul className="files-field__list">
            {value.map((entry) => (
              <FileEntryRow
                key={entry.relPath}
                entry={entry}
                onRemove={() => handleRemove(entry.relPath)}
                readOnly={readOnly}
              />
            ))}
          </ul>
        )}
      </div>

      {!readOnly && (
        <>
          <div className="files-field__actions">
            <button
              type="button"
              className="dialog__btn"
              onClick={() => setPickerOpen(true)}
              disabled={suggestions.length === 0}
              title={suggestions.length === 0 ? "No other files available to attach" : undefined}
            >
              Attach existing
            </button>
            <button
              type="button"
              className="dialog__btn"
              onClick={() => fileInputRef.current?.click()}
            >
              Upload new
            </button>
            <input
              type="file"
              ref={fileInputRef}
              className="files-field__hidden-input"
              onChange={handlePickFile}
              aria-label="Upload a file"
            />
          </div>
          <div className="dialog__hint">
            Files live in the <code>files/</code> directory of the solx files root.
          </div>
        </>
      )}

      {pickerOpen && (
        <AttachExistingDialog
          paths={suggestions}
          onClose={() => setPickerOpen(false)}
          onSelect={(relPath) => {
            handleAdd(relPath);
            setPickerOpen(false);
          }}
        />
      )}

      {uploadDialog && (
        <UploadNameDialog
          fileName={uploadDialog.file.name}
          relPath={uploadDialog.relPath}
          onRelPathChange={(v) => setUploadDialog((d) => (d ? { ...d, relPath: v } : d))}
          onCancel={() => {
            setUploadDialog(null);
            setUploadError(null);
          }}
          onUpload={() => void handleConfirmUpload()}
          uploading={uploadMutation.isPending}
          error={uploadError}
        />
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
