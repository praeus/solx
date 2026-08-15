/**
 * ActionList — table-style list of actions with delete + pagination.
 *
 * Mirrors DocumentList in shape (mode, total, loading, error, page, etc.).
 */

import type { ActionSummary } from "../api/api";
import { useDeleteAction } from "../api/hooks";

const PAGE_SIZE = 20;

interface Props {
  documents: ActionSummary[];
  total: number;
  loading: boolean;
  error?: Error | null;
  page: number;
  onPageChange: (page: number) => void;
  onCreateClick: () => void;
  onRetry?: () => void;
}

export function ActionList({
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
      <div className="browser-state browser-state--error">
        <p>Failed to load actions: {error.message}</p>
        {onRetry && (
          <button className="browser-state__btn" onClick={onRetry}>
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
      <div className="browser-state">
        <p>No actions found.</p>
        <button className="browser-state__btn browser-state__btn--primary" onClick={onCreateClick}>
          + Create your first action
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
        {documents.map((action, i) => (
          <ActionCard
            key={String(action.id ?? `${action.path}/${action.name}-${i}`)}
            action={action}
          />
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

function ActionCard({ action }: { action: ActionSummary }) {
  const path = String(action.path ?? "/");
  const name = String(action.name ?? "Unnamed");
  const caption = String(action.caption ?? "").trim();
  const category = String(action.category ?? "").trim();
  const actionType = String(action.action_type ?? "").trim().toLowerCase() || "wasm";
  const trusted = Boolean(action.trusted);
  const updatedAt = String(action.updated_at ?? "");
  const deleteMutation = useDeleteAction();

  const handleDelete = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (!window.confirm(`Delete action "${name}" at ${path}?`)) return;
    deleteMutation.mutate({ path, name });
  };

  return (
    <div className="doc-card" role="button" tabIndex={0}>
      <div className="doc-card__body">
        <div className="doc-card__header">
          <span className="doc-card__name">{name}</span>
          <span className="doc-card__type-badge">{actionType}</span>
          {trusted && <span className="doc-card__type-badge" data-kind="trusted">trusted</span>}
        </div>
        {caption && <div className="doc-card__title">{caption}</div>}
        <div className="doc-card__path">{path}</div>
        {category && <div className="doc-card__meta">Category: {category}</div>}
        {updatedAt && <div className="doc-card__meta">{new Date(updatedAt).toLocaleString()}</div>}
      </div>
      <button
        className="doc-card__delete"
        onClick={handleDelete}
        aria-label={`Delete ${name}`}
        title={trusted ? "Cannot delete trusted built-in" : "Delete"}
        disabled={deleteMutation.isPending || trusted}
      >
        ✕
      </button>
    </div>
  );
}
