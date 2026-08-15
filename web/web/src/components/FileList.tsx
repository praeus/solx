/**
 * FileList — flat list of files matching a prefix filter.
 *
 * The FileStore returns flat paths, so this view is a flat table with a
 * prefix filter and a row-click → editor flow.
 */

import { useEffect, useState } from "react";
import type { UseQueryResult } from "@tanstack/react-query";
import { useDeleteFile, useFilesList, type FileListResponse } from "../api/hooks";

const PAGE_SIZE = 50;

interface Props {
  /** When provided, the editor is opened for a specific file. */
  onFileOpen: (relPath: string) => void;
  onCreateClick: () => void;
  /** Optional injected query result (lets App.tsx surface errors at the top). */
  filesQuery?: UseQueryResult<FileListResponse>;
}

export function FileList({ onFileOpen, onCreateClick, filesQuery: providedQuery }: Props) {
  const [prefix, setPrefix] = useState("");
  const [debouncedPrefix, setDebouncedPrefix] = useState("");
  const [page, setPage] = useState(0);

  // Debounce prefix changes (300ms).
  useEffect(() => {
    const t = window.setTimeout(() => setDebouncedPrefix(prefix), 300);
    return () => window.clearTimeout(t);
  }, [prefix]);

  // Reset page when prefix changes.
  useEffect(() => {
    setPage(0);
  }, [debouncedPrefix]);

  const localQuery = useFilesList(debouncedPrefix);
  const query = providedQuery ?? localQuery;
  const deleteMutation = useDeleteFile();

  // Defensive: backend may return a JSON-stringified array if its unwrap
  // regresses; normalize to a real string[].
  const rawPaths = query.data?.paths ?? [];
  const paths: string[] = Array.isArray(rawPaths)
    ? rawPaths.filter((p): p is string => typeof p === "string")
    : [];
  const total = paths.length;
  const totalPages = Math.max(1, Math.ceil(total / PAGE_SIZE));
  const pagePaths = paths.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE);
  const rangeStart = total === 0 ? 0 : page * PAGE_SIZE + 1;
  const rangeEnd = total === 0 ? 0 : Math.min((page + 1) * PAGE_SIZE, total);

  const handleDelete = (relPath: string) => {
    if (!window.confirm(`Delete file "${relPath}"?`)) return;
    deleteMutation.mutate(relPath);
  };

  return (
    <div className="doc-list-container">
      <div className="file-list-toolbar">
        <input
          className="file-list-toolbar__prefix"
          type="text"
          value={prefix}
          onChange={(e) => setPrefix(e.target.value)}
          placeholder="Prefix filter (e.g. files/actions/shared/)"
          spellCheck={false}
        />
        <button
          className="file-list-toolbar__upload"
          onClick={onCreateClick}
          title="Upload a file"
        >
          + Upload
        </button>
      </div>

      {query.isError && (
        <div className="browser-state browser-state--error">
          <p>Failed to list files: {(query.error as Error).message}</p>
          <button className="browser-state__btn" onClick={() => query.refetch()}>
            Retry
          </button>
        </div>
      )}

      {!query.isError && query.isLoading && paths.length === 0 && (
        <div className="doc-list">
          {Array.from({ length: 4 }).map((_, i) => (
            <div key={i} className="doc-list__skeleton" />
          ))}
        </div>
      )}

      {!query.isError && !query.isLoading && paths.length === 0 && (
        <div className="browser-state">
          <p>No files{prefix ? ` matching "${prefix}"` : ""}.</p>
          <button
            className="browser-state__btn browser-state__btn--primary"
            onClick={onCreateClick}
          >
            + Upload your first file
          </button>
        </div>
      )}

      {pagePaths.length > 0 && (
        <div className="file-list-table">
          {pagePaths.map((relPath) => (
            <div
              key={relPath}
              className="file-list-row"
              role="button"
              tabIndex={0}
              onClick={() => onFileOpen(relPath)}
              onKeyDown={(e) => {
                if (e.key === "Enter") onFileOpen(relPath);
              }}
            >
              <span className="file-list-row__name">{relPath}</span>
              <button
                className="file-list-row__delete"
                onClick={(e) => {
                  e.stopPropagation();
                  handleDelete(relPath);
                }}
                aria-label={`Delete ${relPath}`}
                title="Delete"
                tabIndex={-1}
                disabled={deleteMutation.isPending}
              >
                ✕
              </button>
            </div>
          ))}
        </div>
      )}

      {paths.length > 0 && (
        <div className="doc-list-pagination">
          <button
            className="doc-list-pagination__btn"
            disabled={page === 0 || query.isFetching}
            onClick={() => setPage((p) => p - 1)}
            aria-label="Previous page"
          >
            ←
          </button>
          <span className="doc-list-pagination__indicator">
            {rangeStart}–{rangeEnd} of {total}
          </span>
          <button
            className="doc-list-pagination__btn"
            disabled={page >= totalPages - 1 || query.isFetching}
            onClick={() => setPage((p) => p + 1)}
            aria-label="Next page"
          >
            →
          </button>
        </div>
      )}
    </div>
  );
}
