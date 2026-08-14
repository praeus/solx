/**
 * DocumentList — scrollable list of document cards with pagination.
 *
 * States handled:
 *   - loading       → skeleton cards
 *   - empty         → friendly empty state with create CTA
 *   - error         → error banner + retry
 *   - loaded        → cards + pagination
 *
 * Each card shows: name, title, type badge, summary excerpt, updated date,
 * and a delete button with confirmation.
 */

import type { DocumentSummary } from "../api/api";
import { useDeleteDocument } from "../api/hooks";

const PAGE_SIZE = 20;

interface Props {
  mode: "browse" | "search";
  documents: DocumentSummary[];
  total: number;
  loading: boolean;
  error?: Error | null;
  page: number;
  onPageChange: (page: number) => void;
  onCreateClick: () => void;
  onRetry?: () => void;
}

export function DocumentList({
  mode,
  documents,
  total,
  loading,
  error,
  page,
  onPageChange,
  onCreateClick,
  onRetry,
}: Props) {
  if (error) {
    return (
      <div className="doc-list-state doc-list-state--error">
        <p>Failed to load documents: {error.message}</p>
        {onRetry && (
          <button className="doc-list-state__btn" onClick={onRetry}>
            Retry
          </button>
        )}
      </div>
    );
  }

  if (loading && documents.length === 0) {
    return (
      <div className="doc-list">
        {Array.from({ length: 4 }).map((_, i) => (
          <div key={i} className="doc-list__skeleton" />
        ))}
      </div>
    );
  }

  if (documents.length === 0) {
    return (
      <div className="doc-list-state">
        <p>
          {mode === "search"
            ? "No documents match your search."
            : "No documents found."}
        </p>
        <button className="doc-list-state__btn doc-list-state__btn--primary" onClick={onCreateClick}>
          + Create your first document
        </button>
      </div>
    );
  }

  const totalPages = Math.max(1, Math.ceil(total / PAGE_SIZE));
  const rangeStart = total === 0 ? 0 : page * PAGE_SIZE + 1;
  const rangeEnd = total === 0 ? 0 : Math.min((page + 1) * PAGE_SIZE, total);

  return (
    <div className="doc-list-container">
      <div className="doc-list">
        {documents.map((doc, i) => (
          <DocumentCard key={String(doc.id ?? `${doc.path}/${doc.name}-${i}`)} document={doc} />
        ))}
      </div>

      <div className="doc-list-pagination">
        <button
          className="doc-list-pagination__btn"
          disabled={page === 0 || loading}
          onClick={() => onPageChange(page - 1)}
          aria-label="Previous page"
        >
          ←
        </button>
        <span className="doc-list-pagination__indicator">
          {rangeStart}–{rangeEnd} of {total}
        </span>
        <button
          className="doc-list-pagination__btn"
          disabled={page >= totalPages - 1 || loading}
          onClick={() => onPageChange(page + 1)}
          aria-label="Next page"
        >
          →
        </button>
      </div>
    </div>
  );
}

interface CardProps {
  document: DocumentSummary;
}

function DocumentCard({ document: doc }: CardProps) {
  const path = String(doc.path ?? "/");
  const name = String(doc.name ?? "Unnamed");
  const title = (doc.title ?? "").toString().trim();
  const summary = (doc.summary ?? "").toString().trim();
  const typeName = String(doc.type_name ?? doc.type_ref ?? "").replace(/^.*\//, "");
  const updatedAt = String(doc.updated_at ?? "");
  const deleteMutation = useDeleteDocument();

  const handleDelete = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (!window.confirm(`Delete "${name}" at ${path}?`)) return;
    deleteMutation.mutate({ path, name });
  };

  return (
    <div
      className="doc-card"
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          // Future: open detail view
        }
      }}
    >
      <div className="doc-card__body">
        <div className="doc-card__header">
          <span className="doc-card__name">{name}</span>
          {typeName && <span className="doc-card__type-badge">{typeName}</span>}
        </div>
        {title && <div className="doc-card__title">{title}</div>}
        <div className="doc-card__path">{path}</div>
        {summary && <div className="doc-card__summary">{truncate(summary, 200)}</div>}
        {updatedAt && <div className="doc-card__meta">{formatDate(updatedAt)}</div>}
      </div>
      <button
        className="doc-card__delete"
        onClick={handleDelete}
        aria-label={`Delete ${name}`}
        title="Delete"
      >
        ✕
      </button>
    </div>
  );
}

function truncate(s: string, max: number): string {
  if (s.length <= max) return s;
  return s.slice(0, max - 1).trimEnd() + "…";
}

function formatDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleString();
}
