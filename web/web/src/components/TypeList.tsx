/**
 * TypeList — table of Type entities with delete + pagination.
 */

import type { TypeEntity } from "../api/api";
import { useDeleteType } from "../api/hooks";

const PAGE_SIZE = 20;

interface Props {
  documents: TypeEntity[];
  total: number;
  loading: boolean;
  error?: Error | null;
  page: number;
  onPageChange: (page: number) => void;
  onCreateClick: () => void;
  onRetry?: () => void;
  /** Open the editor for an existing type. */
  onOpen?: (type: TypeEntity) => void;
}

export function TypeList({
  documents,
  total,
  loading,
  error,
  page,
  onPageChange,
  onCreateClick,
  onRetry,
  onOpen,
}: Props) {
  if (error) {
    return (
      <div className="browser-state browser-state--error">
        <p>Failed to load types: {error.message}</p>
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
        <p>No types found.</p>
        <button className="browser-state__btn browser-state__btn--primary" onClick={onCreateClick}>
          + Create your first type
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
        {documents.map((type, i) => (
          <TypeCard
            key={String(type.id ?? `${type.path}/${type.name}-${i}`)}
            type={type}
            onOpen={onOpen}
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

function TypeCard({
  type,
  onOpen,
}: {
  type: TypeEntity;
  onOpen?: (type: TypeEntity) => void;
}) {
  const path = String(type.path ?? "/");
  const name = String(type.name ?? "Unnamed");
  const description = String(type.description ?? "").trim();
  const updatedAt = String(type.updatedAt ?? "");
  const props = (type.schema as Record<string, unknown> | undefined)?.properties as
    | Record<string, unknown>
    | undefined;
  const propCount = props ? Object.keys(props).length : 0;
  const deleteMutation = useDeleteType();

  const handleDelete = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (!window.confirm(`Delete type "${name}" at ${path}?`)) return;
    deleteMutation.mutate({ path, name });
  };

  return (
    <div
      className="doc-card"
      role="button"
      tabIndex={0}
      onClick={() => onOpen?.(type)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onOpen?.(type);
        }
      }}
    >
      <div className="doc-card__body">
        <div className="doc-card__header">
          <span className="doc-card__name">{name}</span>
          <span className="doc-card__type-badge">Type</span>
        </div>
        <div className="doc-card__path">{path}</div>
        {description && <div className="doc-card__title">{description}</div>}
        {propCount > 0 && (
          <div className="doc-card__meta">{propCount} schema propert{propCount === 1 ? "y" : "ies"}</div>
        )}
        {updatedAt && <div className="doc-card__meta">{new Date(updatedAt).toLocaleString()}</div>}
      </div>
      <button
        className="doc-card__delete"
        onClick={handleDelete}
        aria-label={`Delete ${name}`}
        title="Delete"
        disabled={deleteMutation.isPending}
      >
        ✕
      </button>
    </div>
  );
}
