import { Cpu } from 'lucide-react';
import { t } from '../i18n';
import { useApp } from '../store';
import { PageHead } from '../pages/SidePage';
import { fmtBytes, fmtPct, tone } from '../ui/format';
import { Card } from './ui';

function Bar({ pct }: { pct: number }) {
  return (
    <span className={`perf-bar ${tone(pct)}`}>
      <span className="perf-bar-track">
        <span className="perf-bar-fill" style={{ transform: `translateX(${Math.min(100, Math.max(0, pct)) - 100}%)` }} />
      </span>
    </span>
  );
}

/** Système › Supervision › Performances (données de la pastille, relues toutes les 3 s). */
export function Performance() {
  const m = useApp((s) => s.metrics?.machine);
  const proc = useApp((s) => s.metrics?.process);
  if (!m) return <div className="loading">{t('Loading…')}</div>;
  const mem = m.memTotal ? (m.memUsed / m.memTotal) * 100 : 0;
  return (
    <>
      <PageHead title={t('Performance')} sub={m.hostname} />
      <Card icon={<Cpu size={16} />} title={t('Machine')}>
        <div className="perf-rows">
          <span className="perf-name">
            <span className={`perf-dot ${tone(m.cpu)}`} />
            {t('CPU')}
          </span>
          <Bar pct={m.cpu} />
          <span className="perf-val">{fmtPct(m.cpu)}</span>
          <span className="perf-name">
            <span className={`perf-dot ${tone(mem)}`} />
            {t('Memory')}
          </span>
          <Bar pct={mem} />
          <span className="perf-val">{t('{used} of {total}', { used: fmtBytes(m.memUsed), total: fmtBytes(m.memTotal) })}</span>
          {proc && (
            <>
              <span className="perf-name">
                <span className={`perf-dot ${tone(proc.cpu)}`} />
                {t('Strata server')}
              </span>
              <Bar pct={proc.cpu} />
              <span className="perf-val">
                {fmtPct(proc.cpu)} · {fmtBytes(proc.mem)}
              </span>
            </>
          )}
        </div>
        <dl className="sys-list">
          <dt>{t('Processor')}</dt>
          <dd>{m.cpuName || '—'}</dd>
          <dt>{t('Cores')}</dt>
          <dd>{m.cores}</dd>
        </dl>
      </Card>
    </>
  );
}
