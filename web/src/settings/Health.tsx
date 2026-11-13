import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { Bug, CircleAlert, Download, HeartPulse, Pause, Play, ScrollText, Search, TriangleAlert } from 'lucide-react';
import { t, tn } from '../i18n';
import { api, useApp, type Incident } from '../store';
import { navigate } from '../router';
import { PageHead, SubTabs } from '../pages/SidePage';
import { Dropdown } from '../ui/Dropdown';
import { Empty } from '../ui/Empty';
import { fmtAgo, fmtBytes, fmtDateTime, fmtDuration, fmtTime } from '../ui/format';
import { Card } from './ui';

/** Système › Supervision › Santé : incidents et journal du serveur. */
export function Health({ sub }: { sub?: string }) {
  const health = useApp((s) => s.health);
  const loadHealth = useApp((s) => s.loadHealth);
  const tab = sub === 'log' ? 'log' : 'incidents';

  useEffect(() => {
    void loadHealth();
  }, [loadHealth]);

  // Ouvrir la page marque le plantage comme vu.
  const unseen = !!health?.unseenCrash;
  useEffect(() => {
    if (!unseen) return;
    void api('POST', '/api/system/health/seen')
      .catch(() => {})
      .then(() => loadHealth());
  }, [unseen, loadHealth]);

  return (
    <>
      <PageHead title={t('Health')} sub={health ? t('Running for {time}', { time: fmtDuration(health.uptimeSecs) }) : undefined}>
        <a className="btn" href="/api/system/health/export.zip" download>
          <Download size={14} /> {t('Export')}
        </a>
      </PageHead>
      <SubTabs
        current={tab}
        onPick={(id) => navigate(id === 'log' ? '/system/health/log' : '/system/health')}
        items={[
          { id: 'incidents', icon: <HeartPulse size={14} />, label: t('Incidents'), count: health?.incidents.length },
          { id: 'log', icon: <ScrollText size={14} />, label: t('Log') },
        ]}
      />
      {tab === 'incidents' ? <Incidents /> : <LogConsole />}
    </>
  );
}

const LEVEL_ICON = { CRASH: Bug, ERROR: CircleAlert, WARN: TriangleAlert };

function Incidents() {
  const health = useApp((s) => s.health);
  if (!health) return <div className="loading">{t('Loading…')}</div>;
  const levelText = (l: Incident['level']) => (l === 'CRASH' ? t('Crash') : l === 'ERROR' ? t('Error') : t('Warning'));
  return (
    <>
      <Card icon={<HeartPulse size={16} />} title={t('Server')}>
        <dl className="sys-list">
          <dt>{t('Version')}</dt>
          <dd>{health.version}</dd>
          <dt>{t('Started')}</dt>
          <dd>{fmtDateTime(health.startedAt)}</dd>
          <dt>{t('Crashes')}</dt>
          <dd>{health.crashes ? `${health.crashes} · ${t('last one {when}', { when: fmtAgo(health.lastCrash) })}` : '0'}</dd>
          <dt>{t('Errors')}</dt>
          <dd>{health.lastHourErrors ? `${health.errors} · ${tn(health.lastHourErrors, '{n} in the last hour', '{n} in the last hour')}` : health.errors}</dd>
          <dt>{t('Warnings')}</dt>
          <dd>{health.warnings}</dd>
          <dt>{t('Free disk space')}</dt>
          <dd>{t('{used} of {total}', { used: fmtBytes(health.disk.free), total: fmtBytes(health.disk.total) })}</dd>
        </dl>
      </Card>
      {health.incidents.length === 0 ? (
        <div className="empty big">
          <HeartPulse size={22} />
          <p>{t('No incident')}</p>
        </div>
      ) : (
        <div className="incident-list">
          {health.incidents.map((i, n) => {
            const Icon = LEVEL_ICON[i.level] ?? CircleAlert;
            return (
              <div key={n} className={`incident lvl-${i.level.toLowerCase()}`}>
                <Icon size={16} />
                <div className="incident-main">
                  <div className="incident-title">
                    <span className={`badge ${i.level === 'WARN' ? 'warn' : 'bad'}`}>{levelText(i.level)}</span>
                    <strong>{i.title}</strong>
                    {i.count > 1 && <span className="count">×{i.count}</span>}
                  </div>
                  {i.detail && <pre className="incident-detail">{i.detail}</pre>}
                </div>
                <span className="muted" title={`${fmtDateTime(i.first)} → ${fmtDateTime(i.last)}`}>
                  {fmtAgo(i.last)}
                </span>
              </div>
            );
          })}
        </div>
      )}
    </>
  );
}

interface LogEntry {
  seq: number;
  at: number;
  level: string;
  target: string;
  message: string;
  fields: string;
}
type Level = 'all' | 'INFO' | 'WARN' | 'ERROR';
const MAX = 5000;

function LogConsole() {
  const toast = useApp((s) => s.toast);
  const [entries, setEntries] = useState<LogEntry[]>([]);
  const [level, setLevel] = useState<Level>('all');
  const [q, setQ] = useState('');
  const [paused, setPaused] = useState(false);
  const last = useRef<number | null>(null);
  const box = useRef<HTMLDivElement>(null);
  const atBottom = useRef(true);

  const fetchMore = useCallback(async () => {
    const params = new URLSearchParams();
    if (last.current !== null) params.set('after', String(last.current));
    if (level !== 'all') params.set('level', level);
    params.set('limit', '1000');
    try {
      const r = await api<{ entries: LogEntry[]; last: number; file?: string }>('GET', `/api/system/logs?${params}`);
      last.current = r.last ?? last.current;
      if (r.entries?.length) setEntries((old) => [...old, ...r.entries].slice(-MAX));
    } catch (e) {
      toast((e as Error).message, 'error');
    }
  }, [level, toast]);

  // Nouveau filtre de niveau : tout relire.
  useEffect(() => {
    last.current = null;
    setEntries([]);
  }, [level]);

  useEffect(() => {
    if (paused) return;
    void fetchMore();
    const id = setInterval(() => void fetchMore(), 2000);
    return () => clearInterval(id);
  }, [paused, fetchMore]);

  const shown = useMemo(() => {
    const needle = q.trim().toLowerCase();
    if (!needle) return entries;
    return entries.filter((e) => `${e.target} ${e.message} ${e.fields}`.toLowerCase().includes(needle));
  }, [entries, q]);

  // Reste en bas si on y était.
  useLayoutEffect(() => {
    const el = box.current;
    if (el && atBottom.current) el.scrollTop = el.scrollHeight;
  }, [shown]);

  const levels: { value: Level; label: string }[] = [
    { value: 'all', label: t('All levels') },
    { value: 'INFO', label: t('Info and above') },
    { value: 'WARN', label: t('Warnings and errors') },
    { value: 'ERROR', label: t('Errors') },
  ];
  return (
    <>
      <div className="media-toolbar">
        <label className="search">
          <Search size={14} />
          <input value={q} placeholder={t('Search the log')} onChange={(e) => setQ(e.target.value)} onKeyDown={(e) => e.stopPropagation()} />
        </label>
        <div className="log-level">
          <Dropdown value={level} options={levels} onChange={setLevel} title={t('Level')} />
        </div>
        <div className="spacer" />
        <button className={`btn ${paused ? 'on' : ''}`} onClick={() => setPaused(!paused)}>
          {paused ? <Play size={14} /> : <Pause size={14} />} {paused ? t('Resume') : t('Pause')}
        </button>
        <a className="btn" href="/api/system/logs/download" download>
          <Download size={14} /> {t('Download')}
        </a>
      </div>
      <div
        className="log-box"
        ref={box}
        onScroll={(e) => {
          const el = e.currentTarget;
          atBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
        }}
      >
        {shown.length === 0 ? (
          <Empty icon={ScrollText} text={q ? t('No match') : t('Nothing in the log')} />
        ) : (
          shown.map((e) => (
            <div key={e.seq} className={`log-line lvl-${e.level.toLowerCase()}`}>
              <span className="log-at">{fmtTime(e.at)}</span>
              <span className="log-lvl">{e.level}</span>
              <span className="log-target">{e.target}</span>
              <span className="log-msg">
                {e.message}
                {e.fields && <span className="log-fields"> {e.fields}</span>}
              </span>
            </div>
          ))
        )}
      </div>
    </>
  );
}
