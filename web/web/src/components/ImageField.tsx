/**
 * ImageField — schema-driven editor for an image reference.
 *
 * Replaces the legacy `ArtifactRef`-style single-name inputs. A field
 * that resolves to `ImageField` is a string relPath under the
 * `solx-files` store (e.g. `files/docs/shared/icon.png`); the value
 * round-trips as that string.
 *
 * The component shows:
 *   - A live preview of the image (via `/api/files/raw?relPath=…`).
 *   - A relPath text input.
 *   - An upload-from-disk control (writes to the solx-files store and
 *     updates the relPath on success).
 *
 * Wire format: a single `string` (the relPath). On the legacy
 * `{ name: string }` shape, the field coerces to `name`.
 *
 * Detection (set up in DocumentEditor.renderDocField):
 *   - `x-sol-editor: "image"` on the schema
 *   - `$ref` containing `Image` (e.g. `#/definitions/Image`)
 *   - `x-sol-content-type` of `image/…`
 */

import { useEffect, useMemo, useRef, useState } from "react";
import { fileRawUrl } from "../api/api";
import { useUploadFile } from "../api/hooks";

interface Props {
  value: string;
  onChange: (nextRelPath: string) => void;
  label?: string;
  readOnly?: boolean;
  /** Default destination when uploading from disk. */
  uploadPrefix?: string;
  /** aria-label for screen readers on the path input. */
  ariaLabel?: string;
}

function basename(relPath: string): string {
  if (!relPath) return "";
  const idx = Math.max(relPath.lastIndexOf("/"), relPath.lastIndexOf("\\"));
  return idx >= 0 ? relPath.slice(idx + 1) : relPath;
}

/**
 * Normalize the incoming value into a relPath string. Accepts either:
 *   - a plain string (the new wire format)
 *   - a legacy `{ name: string }` shape, where `name` is treated as a
 *     file basename — we fall back to `files/docs/shared/<name>` if no
 *     relPath is present.
 */
function coerceValue(raw: unknown): string {
  if (typeof raw === "string") return raw;
  if (raw && typeof raw === "object") {
    const obj = raw as Record<string, unknown>;
    const relPath = typeof obj.relPath === "string" ? obj.relPath : "";
    if (relPath) return relPath;
    const name = typeof obj.name === "string" ? obj.name : "";
    if (name) return `files/docs/shared/${name}`;
  }
  return "";
}

export function ImageField({
  value,
  onChange,
  label = "Image",
  readOnly,
  uploadPrefix = "files/docs/shared/",
  ariaLabel,
}: Props) {
  const initial = useMemo(() => coerceValue(value), [value]);
  const [relPath, setRelPath] = useState(initial);
  const [error, setError] = useState<string | null>(null);

  // Keep the local input synced if the upstream value changes (e.g. on
  // hydration when the document is first opened).
  useEffect(() => {
    setRelPath(initial);
  }, [initial]);

  // Push upstream on every change so saves always reflect the latest text.
  useEffect(() => {
    if (relPath !== initial) onChange(relPath);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [relPath]);

  // ── Upload-from-disk ─────────────────────────────────────────────────
  const upload = useUploadFile();
  const inputRef = useRef<HTMLInputElement | null>(null);

  const handlePick = async (e: React.ChangeEvent<HTMLInputElement>) => {
    setError(null);
    const file = e.target.files?.[0];
    if (!file) return;
    if (!file.type.startsWith("image/")) {
      setError(`Not an image: ${file.type || "unknown"}`);
      e.target.value = "";
      return;
    }
    const target = `${uploadPrefix.replace(/\/+$/, "")}/${file.name}`;
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      const result = await upload.mutateAsync({ relPath: target, bytes });
      setRelPath(result.relPath);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
    e.target.value = "";
  };

  return (
    <div className="form-field image-field">
      <label className="form-label" style={{ textTransform: "none", letterSpacing: 0 }}>
        {label}
      </label>
      <div className="image-field__row">
        <div className="image-field__preview">
          {relPath ? (
            <img
              src={fileRawUrl(relPath)}
              alt={basename(relPath) || "image"}
              loading="lazy"
              onError={() =>
                setError(
                  `Could not load "${relPath}". Make sure the file exists in the files root.`,
                )
              }
            />
          ) : (
            <div className="image-field__preview-empty">No image</div>
          )}
        </div>
        <div className="image-field__controls">
          <input
            type="text"
            className="form-input"
            value={relPath}
            onChange={(e) => setRelPath(e.target.value)}
            placeholder="files/docs/shared/example.png"
            spellCheck={false}
            readOnly={readOnly}
            aria-label={ariaLabel ?? label}
          />
          {!readOnly && (
            <div className="image-field__buttons">
              <input
                ref={inputRef}
                type="file"
                accept="image/*"
                onChange={handlePick}
                className="image-field__file"
                aria-hidden
              />
              <button
                type="button"
                className="image-field__upload-btn"
                onClick={() => inputRef.current?.click()}
                disabled={upload.isPending}
              >
                {upload.isPending ? "Uploading…" : "Upload image"}
              </button>
              {relPath && (
                <button
                  type="button"
                  className="image-field__clear-btn"
                  onClick={() => setRelPath("")}
                  title="Clear"
                >
                  ×
                </button>
              )}
            </div>
          )}
        </div>
      </div>
      {error && <div className="image-field__error">{error}</div>}
    </div>
  );
}
