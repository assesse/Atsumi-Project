import { Component, Suspense, lazy, useState, type ComponentType, type ReactNode } from "react";

class ViewBoundary extends Component<{ children: ReactNode }, { failed: boolean; attempt: number }> {
  state = { failed: false, attempt: 0 };
  static getDerivedStateFromError() { return { failed: true }; }
  render() {
    if (this.state.failed) return <div className="platform-load-error" role="alert"><p>화면을 열지 못했습니다.</p><button type="button" onClick={() => this.setState({ failed: false, attempt: this.state.attempt + 1 })}>다시 시도</button></div>;
    return <div key={this.state.attempt} className="platform-content-boundary">{this.props.children}</div>;
  }
}

type ViewProps<Props extends object> = { load: () => Promise<{ default: ComponentType<Props> }>; viewProps: Props };

function LazyView<Props extends object>({ load, viewProps }: ViewProps<Props>) {
  // React.lazy caches rejected imports. Remounting after a retry must create a
  // fresh lazy object, not just reset the boundary around the rejected object.
  const [View] = useState(() => lazy(load));
  return <Suspense fallback={<div className="platform-loading" role="status"><span className="platform-spinner" aria-hidden="true" />화면 준비 중</div>}><View {...viewProps} /></Suspense>;
}

export function ReloadableView<Props extends object>(props: ViewProps<Props>) {
  return <ViewBoundary><LazyView {...props} /></ViewBoundary>;
}
