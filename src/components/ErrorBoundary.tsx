import { Component, type ErrorInfo, type ReactNode } from "react";

/** Last line of defence for a render error: without a boundary React
 *  unmounts the whole tree and the window goes blank. Shows what failed
 *  and offers a reload; running file operations live in the backend and
 *  are not affected. */
export class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error(error, info.componentStack);
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div className="overlay">
        <div className="dialog">
          <h2>Something went wrong</h2>
          <div className="body">
            <p>{String(error.message || error)}</p>
          </div>
          <div className="buttons">
            <button className="btn primary" onClick={() => window.location.reload()}>
              Reload
            </button>
          </div>
        </div>
      </div>
    );
  }
}
