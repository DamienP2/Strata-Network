import { Component, type ReactNode } from 'react';
import { RotateCcw, TriangleAlert } from 'lucide-react';
import { t } from '../i18n';

/** A page that fails shows a message instead of emptying the whole app; the top bar stays usable. */
export class PageGuard extends Component<{ children: ReactNode; page: string }, { failed: string | null }> {
  state = { failed: null as string | null };

  static getDerivedStateFromError(e: unknown) {
    return { failed: e instanceof Error ? e.message : String(e) };
  }

  componentDidCatch(e: unknown) {
    console.error(e);
  }

  componentDidUpdate(prev: { page: string }) {
    if (prev.page !== this.props.page && this.state.failed) this.setState({ failed: null });
  }

  render() {
    if (!this.state.failed) return this.props.children;
    return (
      <div className="empty big" title={this.state.failed}>
        <TriangleAlert size={22} />
        <p>{t('This page could not be shown')}</p>
        <button className="btn" onClick={() => location.reload()}>
          <RotateCcw size={14} /> {t('Reload')}
        </button>
      </div>
    );
  }
}
