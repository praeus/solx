/**
 * FileEditor — modal for viewing/uploading/deleting a single file.
 *
 * Three modes:
 *   - Open mode (existing path): show relPath, size, byte preview (text or
 *     base64-inlined image), and Download/Delete buttons.
 *   - Create mode (no path), which itself offers two ways to produce
 *     content:
 *       - "From disk" — a file picker + relPath field + Upload button.
 *       - "New text file" — relPath field + language picker + a
 *         CodeMirror editor, for authoring a `.solx` script, note, or JSON
 *         fixture directly without leaving the browser (mirrors old
 *         sol-browser's `TextArtifactDialog`).
 *
 * Both create-mode paths end in the same `uploadFile` call and close the
 * dialog on success. Open mode is download-only.
 */

import { useEffect, useMemo, useRef, useState } from "react";
import { useDeleteFile, useFileContent, useUploadFile } from "../api/hooks";
import { CodeEditField } from "./CodeEditField";
import { LANGUAGE_OPTIONS, type LanguageId } from "./codemirror/languages";

interface Props {
  open: boolean;
  onClose: () => void;
  /** When provided, the editor shows the file. When null, this is an upload dialog. */
  relPath: string | null;
  defaultRelPath?: string;
  onUploaded?: (relPath: string) => void;
  onDeleted?: () => void;
}

export function FileEditor({ open, onClose, relPath, defaultRelPath, onUploaded, onDeleted }: Props) {
  const isOpen = relPath !== null;
  const contentQuery = useFileContent(relPath ?? undefined, open && isOpen);
  const uploadMutation = useUploadFile();
  const deleteMutation = useDeleteFile();

  const [relPathInput, setRelPathInput] = useState(defaultRelPath ?? "files/shared/");
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const [pendingFile, setPendingFile] = useState<File | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Create-mode source picker: disk upload vs. authoring text inline.
  const [createMode, setCreateMode] = useState<"upload" | "text">("upload");
  const [textContent, setTextContent] = useState("");
  const [language, setLanguage] = useState<LanguageId>("plaintext");

  // Reset when the dialog opens.
  useEffect(() => {
    if (!open) return;
    setError(null);
    setPendingFile(null);
    if (!isOpen) {
      setRelPathInput(defaultRelPath ?? "files/shared/");
      setCreateMode("upload");
      setTextContent("");
      setLanguage("plaintext");
    }
  }, [open, isOpen, defaultRelPath]);

  const preview = useMemo(() => {
    const data = contentQuery.data;
    if (!data) return null;
    const bytes = base64ToBytes(data.bytesB64);
    const text = bytesToUtf8(bytes);
    const isText = looksLikeText(bytes);
    return {
      ...data,
      isText,
      textPreview: text.length > 8000 ? `${text.slice(0, 8000)}…` : text,
    };
  }, [contentQuery.data]);

  if (!open) return null;

  const handlePickFile = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    setPendingFile(file);
    // If the user hasn't customized the path, suggest the file's name.
    if (!relPathInput || relPathInput === defaultRelPath || relPathInput.endsWith("/")) {
      setRelPathInput((current) => {
        const base = current.replace(/\/+$/, "");
        return `${base}/${file.name}`;
      });
    }
  };

  const handleUpload = async () => {
    setError(null);
    if (!pendingFile) {
      setError("Pick a file to upload.");
      return;
    }
    const target = relPathInput.trim();
    if (!target) {
      setError("relPath is required.");
      return;
    }
    const bytes = new Uint8Array(await pendingFile.arrayBuffer());
    try {
      const result = await uploadMutation.mutateAsync({ relPath: target, bytes });
      onUploaded?.(result.relPath);
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const handleCreateText = async () => {
    setError(null);
    const target = relPathInput.trim();
    if (!target) {
      setError("relPath is required.");
      return;
    }
    const bytes = new TextEncoder().encode(textContent);
    try {
      const result = await uploadMutation.mutateAsync({ relPath: target, bytes });
      onUploaded?.(result.relPath);
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const handleDownload = () => {
    if (!preview) return;
    const bytes = base64ToBytes(preview.bytesB64);
    const blob = new Blob([bytes as BlobPart], { type: "application/octet-stream" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = relPath!.split("/").pop() ?? relPath!;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  };

  const handleDelete = async () => {
    if (!relPath) return;
    if (!window.confirm(`Delete file "${relPath}"?`)) return;
    try {
      await deleteMutation.mutateAsync(relPath);
      onDeleted?.();
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <>
      <div className="dialog-backdrop" onClick={onClose} />
      <div className="dialog dialog--wide" role="dialog" aria-label={isOpen ? "File viewer" : "New file"}>
        <div className="dialog__header">
          <h2 className="dialog__title">
            {isOpen ? "File" : createMode === "text" ? "New Text File" : "Upload File"}
          </h2>
          <button className="dialog__close" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </div>

        <div className="dialog__body">
          {isOpen ? (
            <>
              <div className="dialog__field">
                <label className="dialog__label">Path</label>
                <input className="dialog__input" value={relPath ?? ""} readOnly />
                <div className="dialog__hint">
                  {preview ? `${preview.size.toLocaleString()} bytes` : "Loading…"}
                </div>
              </div>
              <div className="dialog__field">
                <label className="dialog__label">Preview</label>
                {contentQuery.isLoading ? (
                  <div className="file-preview">Loading…</div>
                ) : preview?.isText ? (
                  <textarea
                    className="dialog__textarea form-json"
                    rows={18}
                    value={preview.textPreview}
                    readOnly
                    spellCheck={false}
                  />
                ) : (
                  <div className="file-preview file-preview--binary">
                    Binary content ({(preview?.size ?? 0).toLocaleString()} bytes). Use Download to save.
                  </div>
                )}
              </div>
            </>
          ) : (
            <>
              <div style={{ display: "flex", gap: 8, marginBottom: 12 }}>
                <button
                  type="button"
                  className="dialog__btn"
                  style={
                    createMode === "upload"
                      ? { background: "var(--accent)", color: "#fff", borderColor: "var(--accent)" }
                      : undefined
                  }
                  onClick={() => setCreateMode("upload")}
                >
                  From disk
                </button>
                <button
                  type="button"
                  className="dialog__btn"
                  style={
                    createMode === "text"
                      ? { background: "var(--accent)", color: "#fff", borderColor: "var(--accent)" }
                      : undefined
                  }
                  onClick={() => setCreateMode("text")}
                >
                  New text file
                </button>
              </div>

              <div className="dialog__field">
                <label className="dialog__label">Path</label>
                <div className="dialog__input-row">
                  <input
                    className="dialog__input"
                    value={relPathInput}
                    onChange={(e) => setRelPathInput(e.target.value)}
                    placeholder={
                      createMode === "text" ? "files/scripts/my-script.solx" : "files/docs/abc/image.png"
                    }
                    spellCheck={false}
                  />
                  {createMode === "upload" && (
                    <button className="dialog__btn" onClick={() => fileInputRef.current?.click()}>
                      Browse…
                    </button>
                  )}
                </div>
              </div>

              {createMode === "upload" ? (
                <div className="dialog__field">
                  <input
                    ref={fileInputRef}
                    type="file"
                    style={{ display: "none" }}
                    onChange={handlePickFile}
                  />
                  {pendingFile && (
                    <div className="dialog__hint">
                      {pendingFile.name} — {pendingFile.size.toLocaleString()} bytes
                    </div>
                  )}
                </div>
              ) : (
                <>
                  <div className="dialog__field">
                    <label className="dialog__label">Language</label>
                    <select
                      className="form-input"
                      value={language}
                      onChange={(e) => setLanguage(e.target.value as LanguageId)}
                    >
                      {LANGUAGE_OPTIONS.map((l) => (
                        <option key={l.id} value={l.id}>
                          {l.label}
                        </option>
                      ))}
                    </select>
                  </div>
                  <div className="dialog__field">
                    <label className="dialog__label">Content</label>
                    <div className="file-editor__code">
                      <CodeEditField
                        value={textContent}
                        onChange={setTextContent}
                        language={language}
                        height="clamp(180px, 42vh, 320px)"
                        placeholder="File contents…"
                        ariaLabel="File contents"
                      />
                    </div>
                  </div>
                </>
              )}
            </>
          )}
          {error && <div className="dialog__error">{error}</div>}
        </div>

        <div className="dialog__footer">
          <button className="dialog__btn" onClick={onClose} disabled={uploadMutation.isPending}>
            Close
          </button>
          {isOpen ? (
            <>
              <button className="dialog__btn" onClick={handleDownload} disabled={!preview}>
                Download
              </button>
              <button
                className="dialog__btn dialog__btn--danger"
                onClick={() => void handleDelete()}
                disabled={deleteMutation.isPending}
              >
                {deleteMutation.isPending ? "Deleting…" : "Delete"}
              </button>
            </>
          ) : createMode === "upload" ? (
            <button
              className="dialog__btn dialog__btn--primary"
              onClick={() => void handleUpload()}
              disabled={uploadMutation.isPending || !pendingFile}
            >
              {uploadMutation.isPending ? "Uploading…" : "Upload"}
            </button>
          ) : (
            <button
              className="dialog__btn dialog__btn--primary"
              onClick={() => void handleCreateText()}
              disabled={uploadMutation.isPending || !relPathInput.trim()}
            >
              {uploadMutation.isPending ? "Creating…" : "Create"}
            </button>
          )}
        </div>
      </div>
    </>
  );
}

function base64ToBytes(b64: string): Uint8Array {
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

function bytesToUtf8(bytes: Uint8Array): string {
  try {
    return new TextDecoder("utf-8", { fatal: false }).decode(bytes);
  } catch {
    return "";
  }
}

function looksLikeText(bytes: Uint8Array): boolean {
  // NUL byte in the first 8 KiB → treat as binary.
  const cap = Math.min(bytes.length, 8192);
  for (let i = 0; i < cap; i++) {
    if (bytes[i] === 0) return false;
  }
  return true;
}
