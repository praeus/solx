/**
 * DocumentEditorSkeleton — placeholder shown while the lazy-loaded
 * `DocumentEditor` chunk is being fetched. Sized to match the real
 * editor's `dialog--xxl` (1080px wide, two panes) so the modal
 * opens at the right size without layout shift.
 *
 * Lives in the main bundle because it's rendered before the lazy
 * chunk arrives — it must be available on first paint.
 */

export function DocumentEditorSkeleton() {
  return (
    <div
      className="dialog dialog--xxl dialog--backdrop dialog-skeleton"
      role="dialog"
      aria-modal="true"
      aria-label="Loading document editor"
    >
      <div className="dialog__header dialog-skeleton__header">
        <div className="dialog-skeleton__title" />
        <div className="dialog-skeleton__close" aria-hidden />
      </div>
      <div className="dialog__body dialog-skeleton__body">
        <div className="dialog-skeleton__pane dialog-skeleton__pane--left">
          <div className="dialog-skeleton__row" />
          <div className="dialog-skeleton__row dialog-skeleton__row--short" />
          <div className="dialog-skeleton__row" />
          <div className="dialog-skeleton__row dialog-skeleton__row--short" />
          <div className="dialog-skeleton__row" />
        </div>
        <div className="dialog-skeleton__pane dialog-skeleton__pane--right" />
      </div>
    </div>
  );
}
