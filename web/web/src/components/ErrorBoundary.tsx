/**
 * ErrorBoundary — catches any uncaught rendering error in its children
 * and displays a fallback UI with a "Try again" button.
 *
 * Without an error boundary, a thrown error during render unmounts the
 * entire React tree (which is what causes the "blank white screen" you
 * see when a list component receives an unexpected shape, e.g. when the
 * backend returns a JSON-stringified array and `.map` doesn't exist).
 */

import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  children: ReactNode;
  /** Optional label shown above the error message (e.g. "File list"). */
  label?: string;
}

interface State {
  error: Error | null;
}

export class ErrorBoundary extends Component<Props, State> {
  override state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  override componentDidCatch(error: Error, info: ErrorInfo): void {
    // eslint-disable-next-line no-console
    console.error("[ErrorBoundary]", this.props.label ?? "component", error, info);
  }

  handleReset = () => {
    this.setState({ error: null });
  };

  override render() {
    const { error } = this.state;
    if (!error) return this.props.children;

    return (
      <div className="error-boundary" role="alert">
        <h3 className="error-boundary__title">
          {this.props.label ? `${this.props.label} crashed` : "Something went wrong"}
        </h3>
        <p className="error-boundary__message">{error.message}</p>
        {error.stack && (
          <pre className="error-boundary__stack">{error.stack.split("\n").slice(0, 6).join("\n")}</pre>
        )}
        <button className="error-boundary__btn" onClick={this.handleReset}>
          Try again
        </button>
      </div>
    );
  }
}