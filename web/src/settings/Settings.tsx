import { Cpu, RotateCcw, Server } from 'lucide-react';
import { t } from '../i18n';
import { api, useApp } from '../store';
import { PageHead } from '../pages/SidePage';
import { errorText } from '../ui/serverError';
import { fmtDuration } from '../ui/format';
import { Card, Field, SaveBar } from './ui';
import { useServerConfig } from './serverConfig';

/** Système › Serveur › Général. */
export function General() {
  const { cfg, dirty, saving, set, save, discard } = useServerConfig();
  if (!cfg) return <div className="loading">{t('Loading…')}</div>;
  const empty = !cfg.name.trim();
  return (
    <>
      <PageHead title={t('General')} />
      <Card icon={<Server size={16} />} title={t('General')}>
        <div className="set-grid">
          <Field label={t('Server name')}>
            <input value={cfg.name} aria-invalid={empty} onChange={(e) => set((c) => ({ ...c, name: e.target.value }))} />
          </Field>
        </div>
      </Card>
      <SaveBar show={dirty} saving={saving} onSave={() => void save()} onDiscard={discard} />
    </>
  );
}

/** Système › Serveur › Maintenance. */
export function Maintenance() {
  const { system } = useServerConfig();
  const toast = useApp((s) => s.toast);
  if (!system) return <div className="loading">{t('Loading…')}</div>;

  const restart = async () => {
    try {
      await api('POST', '/api/system/restart');
      toast(t('The server is restarting'));
    } catch (e) {
      toast(errorText(e), 'error', (e as Error).message);
    }
  };

  return (
    <>
      <PageHead
        title={t('Maintenance')}
        sub={t('Strata {version} · {machine} · {os}/{arch} · running for {time}', { version: system.version, machine: system.hostname, os: system.os, arch: system.arch, time: fmtDuration(system.uptimeSecs) })}
      >
        <button className="btn" onClick={() => void restart()}>
          <RotateCcw size={14} /> {t('Restart')}
        </button>
      </PageHead>
      <Card icon={<Cpu size={16} />} title={t('System')}>
        <dl className="sys-list">
          <dt>{t('Version')}</dt>
          <dd>{system.version}</dd>
          <dt>{t('Machine')}</dt>
          <dd>
            {system.hostname} · {system.os}/{system.arch}
          </dd>
          <dt>{t('Data folder')}</dt>
          <dd>
            <code>{system.dataDir}</code>
          </dd>
          <dt>{t('Listening on')}</dt>
          <dd>
            <code>{system.bind}</code>
          </dd>
          <dt>{t('CPU threads')}</dt>
          <dd>{system.cpus}</dd>
          <dt>{t('Running for')}</dt>
          <dd>{fmtDuration(system.uptimeSecs)}</dd>
        </dl>
      </Card>
    </>
  );
}
