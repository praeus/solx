/**
 * ResolvedImage — a Tiptap `Image` node whose `src` attribute holds a
 * solx-files `relPath`, not a directly-loadable URL.
 *
 * The stock `@tiptap/extension-image` renders `<img src={attrs.src}>`
 * verbatim, which worked when `src` was the old Bun backend's binary
 * `/api/files/raw?relPath=…` route — a real, token-free, cacheable URL a
 * browser could load directly. `solx-server` has no such route (files are
 * base64 JSON only) and the token can't ride along on a plain `<img src>`
 * anyway, so the persisted `src` is now the bare `relPath`, resolved to a
 * `blob:` URL at render time via `objectUrlCache` — the same mechanism
 * `useFileObjectUrl` wraps for other file previews, applied here through a
 * custom NodeView since Tiptap node attributes are static values, not
 * something a `Promise` can resolve into.
 *
 * Back-compat: documents saved before this change persisted the full
 * `/api/files/raw?relPath=…` URL as `src`. `relPathFromSrc` recovers the
 * relPath from that shape so old documents keep rendering without a data
 * migration.
 */
import Image from "@tiptap/extension-image";
import { NodeViewWrapper, ReactNodeViewRenderer } from "@tiptap/react";
import type { NodeViewProps } from "@tiptap/react";
import { useEffect, useState } from "react";
import { acquireObjectUrl, releaseObjectUrl } from "../../api/objectUrlCache";

function relPathFromSrc(src: string): string {
  if (!src.includes("relPath=")) return src;
  try {
    const url = new URL(src, "http://placeholder.invalid");
    return url.searchParams.get("relPath") ?? src;
  } catch {
    return src;
  }
}

function ResolvedImageView({ node }: NodeViewProps) {
  const relPath = relPathFromSrc(String(node.attrs.src ?? ""));
  const [url, setUrl] = useState<string | undefined>(undefined);

  useEffect(() => {
    if (!relPath) return;
    let cancelled = false;
    acquireObjectUrl(relPath).then((resolved) => {
      if (!cancelled) setUrl(resolved);
    });
    return () => {
      cancelled = true;
      releaseObjectUrl(relPath);
    };
  }, [relPath]);

  return (
    <NodeViewWrapper as="span" className="rich-text-field__image-wrapper">
      {url ? (
        <img
          src={url}
          alt={String(node.attrs.alt ?? "")}
          title={node.attrs.title ? String(node.attrs.title) : undefined}
          className="rich-text-field__image"
        />
      ) : (
        <span className="rich-text-field__image-loading" aria-label="Loading image" />
      )}
    </NodeViewWrapper>
  );
}

export const ResolvedImage = Image.extend({
  addNodeView() {
    return ReactNodeViewRenderer(ResolvedImageView);
  },
});
