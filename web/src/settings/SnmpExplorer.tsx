import { useMemo, useState } from 'react';
import { Copy, Download, EthernetPort, FileText, ListTree, Search } from 'lucide-react';
import { t, tn } from '../i18n';
import { api, useApp } from '../store';
import { Dropdown } from '../ui/Dropdown';
import { copyText, downloadText, fail, fmtInt } from '../net/model';
import type { SnmpAgent } from '../net/types';

/** Une ligne lue (`POST /api/snmp/{id}/walk|get`). */
export interface SnmpRow {
  oid: string;
  type: string;
  text: string;
  hex?: string;
}
interface Result {
  rows: SnmpRow[];
  truncated?: boolean;
  error?: string;
  ms: number;
  /** L'OID de départ (préfixe estompé) ; absent pour une lecture de valeurs. */
  base?: string;
}

/** Les branches proposées. */
const PRESETS: { label: () => string; oid: string }[] = [
  { label: () => t('System'), oid: '1.3.6.1.2.1.1' },
  { label: () => t('Interfaces'), oid: '1.3.6.1.2.1.2.2.1' },
  { label: () => t('Port descriptions'), oid: '1.3.6.1.2.1.31.1.1.1.18' },
  { label: () => t('64-bit counters'), oid: '1.3.6.1.2.1.31.1.1.1.6' },
  { label: () => t('VLAN (Q-BRIDGE)'), oid: '1.3.6.1.2.1.17.7.1.4' },
  { label: () => t('Cisco VLAN'), oid: '1.3.6.1.4.1.9.9.46.1' },
  { label: () => t('MAC table'), oid: '1.3.6.1.2.1.17.7.1.2.2.1.2' },
  { label: () => t('ARP'), oid: '1.3.6.1.2.1.4.22.1.2' },
  { label: () => 'LLDP', oid: '1.0.8802.1.1.2.1.4.1.1' },
  { label: () => 'CDP', oid: '1.3.6.1.4.1.9.9.23.1.2.1.1' },
  { label: () => t('Entities'), oid: '1.3.6.1.2.1.47.1.1.1.1' },
  { label: () => t('Sensors'), oid: '1.3.6.1.2.1.99.1.1.1' },
  { label: () => t('CPU · memory'), oid: '1.3.6.1.2.1.25' },
  { label: () => t('Aggregates'), oid: '1.2.840.10006.300.43.1.2.1.1' },
  { label: () => 'STP', oid: '1.3.6.1.2.1.17.2' },
  { label: () => t('IGMP snooping'), oid: '1.3.6.1.2.1.17.7.1.2.3' },
  { label: () => 'PoE', oid: '1.3.6.1.2.1.105' },
];

/** Les noms MIB connus : OID → [nom, module]. Les colonnes des tables lues par Strata et les plus courantes. */
const MIB: Record<string, [string, string]> = {};
function mib(module: string, base: string, names: Record<number, string>) {
  for (const [n, name] of Object.entries(names)) MIB[`${base}.${n}`] = [name, module];
}
mib('SNMPv2-MIB', '1.3.6.1.2.1.1', { 1: 'sysDescr', 2: 'sysObjectID', 3: 'sysUpTime', 4: 'sysContact', 5: 'sysName', 6: 'sysLocation', 7: 'sysServices' });
mib('IF-MIB', '1.3.6.1.2.1.2', { 1: 'ifNumber' });
mib('IF-MIB', '1.3.6.1.2.1.2.2.1', {
  1: 'ifIndex', 2: 'ifDescr', 3: 'ifType', 4: 'ifMtu', 5: 'ifSpeed', 6: 'ifPhysAddress', 7: 'ifAdminStatus', 8: 'ifOperStatus', 9: 'ifLastChange',
  10: 'ifInOctets', 11: 'ifInUcastPkts', 13: 'ifInDiscards', 14: 'ifInErrors', 15: 'ifInUnknownProtos', 16: 'ifOutOctets', 17: 'ifOutUcastPkts', 19: 'ifOutDiscards', 20: 'ifOutErrors',
});
mib('IF-MIB', '1.3.6.1.2.1.31.1.1.1', {
  1: 'ifName', 2: 'ifInMulticastPkts', 3: 'ifInBroadcastPkts', 4: 'ifOutMulticastPkts', 5: 'ifOutBroadcastPkts', 6: 'ifHCInOctets', 7: 'ifHCInUcastPkts', 8: 'ifHCInMulticastPkts',
  9: 'ifHCInBroadcastPkts', 10: 'ifHCOutOctets', 11: 'ifHCOutUcastPkts', 12: 'ifHCOutMulticastPkts', 13: 'ifHCOutBroadcastPkts', 14: 'ifLinkUpDownTrapEnable', 15: 'ifHighSpeed',
  16: 'ifPromiscuousMode', 17: 'ifConnectorPresent', 18: 'ifAlias', 19: 'ifCounterDiscontinuityTime',
});
mib('IF-MIB', '1.3.6.1.2.1.31.1.2.1', { 3: 'ifStackStatus' });
mib('IP-MIB', '1.3.6.1.2.1.4.22.1', { 1: 'ipNetToMediaIfIndex', 2: 'ipNetToMediaPhysAddress', 3: 'ipNetToMediaNetAddress', 4: 'ipNetToMediaType' });
mib('BRIDGE-MIB', '1.3.6.1.2.1.17.1.4.1', { 1: 'dot1dBasePort', 2: 'dot1dBasePortIfIndex' });
mib('BRIDGE-MIB', '1.3.6.1.2.1.17.2', { 1: 'dot1dStpProtocolSpecification', 2: 'dot1dStpPriority', 3: 'dot1dStpTimeSinceTopologyChange', 4: 'dot1dStpTopChanges', 5: 'dot1dStpDesignatedRoot', 6: 'dot1dStpRootCost', 7: 'dot1dStpRootPort' });
mib('BRIDGE-MIB', '1.3.6.1.2.1.17.2.15.1', { 1: 'dot1dStpPort', 2: 'dot1dStpPortPriority', 3: 'dot1dStpPortState', 4: 'dot1dStpPortEnable', 5: 'dot1dStpPortPathCost', 6: 'dot1dStpPortDesignatedRoot', 8: 'dot1dStpPortDesignatedBridge', 9: 'dot1dStpPortDesignatedPort', 10: 'dot1dStpPortForwardTransitions' });
mib('BRIDGE-MIB', '1.3.6.1.2.1.17.4.3.1', { 1: 'dot1dTpFdbAddress', 2: 'dot1dTpFdbPort', 3: 'dot1dTpFdbStatus' });
mib('Q-BRIDGE-MIB', '1.3.6.1.2.1.17.7.1.2.2.1', { 2: 'dot1qTpFdbPort', 3: 'dot1qTpFdbStatus' });
mib('Q-BRIDGE-MIB', '1.3.6.1.2.1.17.7.1.2.3.1', { 1: 'dot1qTpGroupAddress', 2: 'dot1qTpGroupEgressPorts', 3: 'dot1qTpGroupLearnt' });
mib('Q-BRIDGE-MIB', '1.3.6.1.2.1.17.7.1.2.4.1', { 1: 'dot1qForwardAllPorts', 2: 'dot1qForwardAllStaticPorts' });
mib('Q-BRIDGE-MIB', '1.3.6.1.2.1.17.7.1.4.2.1', { 3: 'dot1qVlanFdbId', 4: 'dot1qVlanCurrentEgressPorts', 5: 'dot1qVlanCurrentUntaggedPorts', 6: 'dot1qVlanStatus', 7: 'dot1qVlanCreationTime' });
mib('Q-BRIDGE-MIB', '1.3.6.1.2.1.17.7.1.4.3.1', { 1: 'dot1qVlanStaticName', 2: 'dot1qVlanStaticEgressPorts', 3: 'dot1qVlanForbiddenEgressPorts', 4: 'dot1qVlanStaticUntaggedPorts', 5: 'dot1qVlanStaticRowStatus' });
mib('Q-BRIDGE-MIB', '1.3.6.1.2.1.17.7.1.4.5.1', { 1: 'dot1qPvid', 2: 'dot1qPortAcceptableFrameTypes', 3: 'dot1qPortIngressFiltering' });
mib('ENTITY-MIB', '1.3.6.1.2.1.47.1.1.1.1', {
  2: 'entPhysicalDescr', 3: 'entPhysicalVendorType', 4: 'entPhysicalContainedIn', 5: 'entPhysicalClass', 6: 'entPhysicalParentRelPos', 7: 'entPhysicalName', 8: 'entPhysicalHardwareRev',
  9: 'entPhysicalFirmwareRev', 10: 'entPhysicalSoftwareRev', 11: 'entPhysicalSerialNum', 12: 'entPhysicalMfgName', 13: 'entPhysicalModelName', 14: 'entPhysicalAlias', 15: 'entPhysicalAssetID', 16: 'entPhysicalIsFRU',
});
mib('ENTITY-MIB', '1.3.6.1.2.1.47.1.3.2.1', { 2: 'entAliasMappingIdentifier' });
mib('ENTITY-SENSOR-MIB', '1.3.6.1.2.1.99.1.1.1', { 1: 'entPhySensorType', 2: 'entPhySensorScale', 3: 'entPhySensorPrecision', 4: 'entPhySensorValue', 5: 'entPhySensorOperStatus', 6: 'entPhySensorUnitsDisplay' });
mib('ENTITY-STATE-MIB', '1.3.6.1.2.1.131.1.1.1', { 2: 'entStateAdmin', 3: 'entStateOper', 4: 'entStateUsage', 5: 'entStateAlarm' });
mib('HOST-RESOURCES-MIB', '1.3.6.1.2.1.25.3.3.1', { 1: 'hrProcessorFrwID', 2: 'hrProcessorLoad' });
mib('HOST-RESOURCES-MIB', '1.3.6.1.2.1.25.2.3.1', { 1: 'hrStorageIndex', 2: 'hrStorageType', 3: 'hrStorageDescr', 4: 'hrStorageAllocationUnits', 5: 'hrStorageSize', 6: 'hrStorageUsed' });
mib('HOST-RESOURCES-MIB', '1.3.6.1.2.1.25.2', { 2: 'hrMemorySize' });
mib('LLDP-MIB', '1.0.8802.1.1.2.1.3.7.1', { 2: 'lldpLocPortIdSubtype', 3: 'lldpLocPortId', 4: 'lldpLocPortDesc' });
mib('LLDP-MIB', '1.0.8802.1.1.2.1.4.1.1', { 4: 'lldpRemChassisIdSubtype', 5: 'lldpRemChassisId', 6: 'lldpRemPortIdSubtype', 7: 'lldpRemPortId', 8: 'lldpRemPortDesc', 9: 'lldpRemSysName', 10: 'lldpRemSysDesc', 11: 'lldpRemSysCapSupported', 12: 'lldpRemSysCapEnabled' });
mib('LLDP-MIB', '1.0.8802.1.1.2.1.4.2.1', { 3: 'lldpRemManAddrIfSubtype', 4: 'lldpRemManAddrIfId' });
mib('CISCO-CDP-MIB', '1.3.6.1.4.1.9.9.23.1.2.1.1', { 3: 'cdpCacheAddressType', 4: 'cdpCacheAddress', 5: 'cdpCacheVersion', 6: 'cdpCacheDeviceId', 7: 'cdpCacheDevicePort', 8: 'cdpCachePlatform', 9: 'cdpCacheCapabilities' });
mib('CISCO-VTP-MIB', '1.3.6.1.4.1.9.9.46.1.3.1.1', { 2: 'vtpVlanState', 3: 'vtpVlanType', 4: 'vtpVlanName' });
mib('CISCO-VTP-MIB', '1.3.6.1.4.1.9.9.46.1.6.1.1', { 4: 'vlanTrunkPortVlansEnabled', 5: 'vlanTrunkPortNativeVlan', 13: 'vlanTrunkPortDynamicState', 14: 'vlanTrunkPortDynamicStatus', 17: 'vlanTrunkPortVlansEnabled2k', 18: 'vlanTrunkPortVlansEnabled3k', 19: 'vlanTrunkPortVlansEnabled4k' });
mib('CISCO-VLAN-MEMBERSHIP-MIB', '1.3.6.1.4.1.9.9.68.1.2.2.1', { 1: 'vmVlanType', 2: 'vmVlan' });
mib('CISCO-ENTITY-SENSOR-MIB', '1.3.6.1.4.1.9.9.91.1.1.1.1', { 1: 'entSensorType', 2: 'entSensorScale', 3: 'entSensorPrecision', 4: 'entSensorValue', 5: 'entSensorStatus' });
mib('CISCO-PROCESS-MIB', '1.3.6.1.4.1.9.9.109.1.1.1.1', { 2: 'cpmCPUMonInterval', 6: 'cpmCPUTotal5secRev', 7: 'cpmCPUTotal1minRev', 8: 'cpmCPUTotal5minRev', 12: 'cpmCPUMemoryUsed', 13: 'cpmCPUMemoryFree' });
mib('IEEE8023-LAG-MIB', '1.2.840.10006.300.43.1.2.1.1', { 1: 'dot3adAggPortActorSystemPriority', 12: 'dot3adAggPortSelectedAggID', 13: 'dot3adAggPortAttachedAggID' });
mib('POWER-ETHERNET-MIB', '1.3.6.1.2.1.105.1.1.1', { 3: 'pethPsePortAdminEnable', 6: 'pethPsePortDetectionStatus', 10: 'pethPsePortPowerClassifications' });
mib('POWER-ETHERNET-MIB', '1.3.6.1.2.1.105.1.3.1.1', { 2: 'pethMainPsePower', 3: 'pethMainPseOperStatus', 4: 'pethMainPseConsumptionPower', 5: 'pethMainPseUsageThreshold' });

// Les branches proposées qui ne sont pas une colonne.
mib('SNMPv2-MIB', '1.3.6.1.2.1', { 1: 'system' });
mib('IF-MIB', '1.3.6.1.2.1.2.2', { 1: 'ifEntry' });
mib('Q-BRIDGE-MIB', '1.3.6.1.2.1.17.7.1', { 4: 'dot1qVlan' });
mib('Q-BRIDGE-MIB', '1.3.6.1.2.1.17.7.1.2', { 3: 'dot1qTpGroupTable' });
mib('CISCO-VTP-MIB', '1.3.6.1.4.1.9.9.46', { 1: 'vtpMIBObjects' });
mib('LLDP-MIB', '1.0.8802.1.1.2.1.4.1', { 1: 'lldpRemEntry' });
mib('CISCO-CDP-MIB', '1.3.6.1.4.1.9.9.23.1.2.1', { 1: 'cdpCacheEntry' });
mib('ENTITY-MIB', '1.3.6.1.2.1.47.1.1.1', { 1: 'entPhysicalEntry' });
mib('ENTITY-SENSOR-MIB', '1.3.6.1.2.1.99.1.1', { 1: 'entPhySensorEntry' });
mib('HOST-RESOURCES-MIB', '1.3.6.1.2.1', { 25: 'host' });
mib('IEEE8023-LAG-MIB', '1.2.840.10006.300.43.1.2.1', { 1: 'dot3adAggPortEntry' });
mib('BRIDGE-MIB', '1.3.6.1.2.1.17', { 2: 'dot1dStp' });
mib('POWER-ETHERNET-MIB', '1.3.6.1.2.1', { 105: 'powerEthernetMIB' });

/** « ifAlias.436207616 » : le nom connu le plus long, suivi du reste de l'OID. */
export function oidName(oid: string): { name: string; module: string } | null {
  const parts = oid.split('.');
  for (let n = parts.length; n > 1; n--) {
    const m = MIB[parts.slice(0, n).join('.')];
    if (m) return { name: [m[0], ...parts.slice(n)].join('.'), module: m[1] };
  }
  return null;
}

const TYPE_TONE: Record<string, string> = { OctetString: 'str', Integer: 'int', Gauge32: 'int', TimeTicks: 'int', Counter32: 'cnt', Counter64: 'cnt', OID: 'oid', IpAddress: 'oid' };
const NUMERIC = new Set(['Integer', 'Gauge32', 'Counter32', 'Counter64']);

/** La valeur affichée : nombres groupés, durée pour TimeTicks. */
function valueText(r: SnmpRow): string {
  if (NUMERIC.has(r.type) && /^-?\d+$/.test(r.text)) return fmtInt(Number(r.text));
  if (r.type === 'TimeTicks' && /^\d+$/.test(r.text)) {
    const secs = Math.floor(Number(r.text) / 100);
    const d = Math.floor(secs / 86400);
    const h = Math.floor((secs % 86400) / 3600);
    const m = Math.floor((secs % 3600) / 60);
    const span = d > 0 ? t('{d} d {h} h', { d, h }) : t('{h} h {m} min', { h, m });
    return `${fmtInt(Number(r.text))} · ${span}`;
  }
  return r.text;
}

const csvField = (s: string) => (/[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s);

export function SnmpExplorer({ agents }: { agents: SnmpAgent[] }) {
  const toast = useApp((s) => s.toast);
  const [agentId, setAgentId] = useState<string | null>(agents[0]?.id ?? null);
  const [oid, setOid] = useState('1.3.6.1.2.1.1');
  const [busy, setBusy] = useState<'walk' | 'get' | null>(null);
  const [res, setRes] = useState<Result | null>(null);
  const [q, setQ] = useState('');
  const agent = agents.find((a) => a.id === agentId) ?? null;
  const cleanOid = oid.trim().replace(/^\./, '');
  const validOid = /^\d+(\.\d+)+$/.test(cleanOid);

  const run = async (kind: 'walk' | 'get', base = cleanOid) => {
    if (!agent || !/^\d+(\.\d+)+$/.test(base)) return;
    setBusy(kind);
    try {
      if (kind === 'walk') {
        const r = await api<Result>('POST', `/api/snmp/${agent.id}/walk`, { oid: base });
        setRes({ ...r, base });
      } else {
        const r = await api<Result>('POST', `/api/snmp/${agent.id}/get`, { oids: [base] });
        setRes(r);
      }
    } catch (e) {
      fail(e);
    }
    setBusy(null);
  };

  const rows = useMemo(() => {
    const all = res?.rows ?? [];
    const needle = q.trim().toLowerCase();
    if (!needle) return all;
    return all.filter((r) => `${r.oid} ${oidName(r.oid)?.name ?? ''} ${r.type} ${r.text} ${r.hex ?? ''}`.toLowerCase().includes(needle));
  }, [res, q]);
  const baseName = res?.base ? oidName(res.base) : null;
  const table = () => [['oid', 'name', 'type', 'value', 'hex'], ...rows.map((r) => [r.oid, oidName(r.oid)?.name ?? '', r.type, r.text, r.hex ?? ''])];

  const copy = async () => {
    const ok = await copyText(table().map((r) => r.join('\t')).join('\n'));
    toast(ok ? t('Copied') : t('Copy failed'), ok ? 'info' : 'error');
  };
  const exportCsv = () => {
    const name = `${agent?.name ?? 'snmp'}-${res?.base ?? 'get'}.csv`.replace(/[\\/"]/g, '-');
    downloadText(name, table().map((r) => r.map(csvField).join(',')).join('\r\n'), 'text/csv;charset=utf-8');
  };

  if (agents.length === 0)
    return (
      <div className="empty big">
        <EthernetPort size={22} />
        <p>{t('No agent')}</p>
      </div>
    );
  return (
    <>
      <div className="xbar">
        <Dropdown
          value={agentId}
          onChange={setAgentId}
          options={agents.map((a) => ({ value: a.id, label: `${a.name} · ${a.ip}${a.port === 161 ? '' : `:${a.port}`} · ${a.v3 ? 'v3' : 'v2c'}`, icon: <EthernetPort size={13} /> }))}
        />
        <input
          className="oid"
          value={oid}
          spellCheck={false}
          aria-invalid={cleanOid !== '' && !validOid}
          onChange={(e) => setOid(e.target.value)}
          onKeyDown={(e) => {
            e.stopPropagation();
            if (e.key === 'Enter') void run('walk');
          }}
        />
        <button className="btn primary" disabled={!agent || !validOid || busy !== null} onClick={() => void run('walk')}>
          <ListTree size={14} className={busy === 'walk' ? 'spin' : undefined} /> {t('Read the branch')}
        </button>
        <button className="btn" disabled={!agent || !validOid || busy !== null} onClick={() => void run('get')}>
          <FileText size={14} className={busy === 'get' ? 'spin' : undefined} /> {t('Read a value')}
        </button>
      </div>
      <div className="presets">
        {PRESETS.map((p) => (
          <button
            key={p.oid}
            className={`asr-chip ${res?.base === p.oid ? 'on' : ''}`}
            title={p.oid}
            disabled={!agent || busy !== null}
            onClick={() => {
              setOid(p.oid);
              void run('walk', p.oid);
            }}
          >
            {p.label()}
          </button>
        ))}
      </div>
      {res && (
        <>
          <div className="media-toolbar">
            <label className="search" style={{ width: 260 }}>
              <Search size={14} />
              <input value={q} placeholder={t('Filter the results')} onChange={(e) => setQ(e.target.value)} onKeyDown={(e) => e.stopPropagation()} />
            </label>
            <div className="xmeta">
              <span>
                <b>{fmtInt(rows.length)}</b> {tn(rows.length, 'row', 'rows')}
              </span>
              <span>
                <b>{fmtInt(res.ms)}</b> {t('ms')}
              </span>
              {baseName && (
                <span>
                  {baseName.name} · {baseName.module}
                </span>
              )}
              {res.truncated && !res.error && <span className="warn">{t('Limit reached')}</span>}
              {res.error && (
                <span className="bad" title={res.error}>
                  {res.error === 'timed out' ? t('Stopped: timed out') : t('Stopped: {why}', { why: res.error })}
                </span>
              )}
            </div>
            <div className="spacer" />
            <button className="btn small" disabled={rows.length === 0} onClick={() => void copy()}>
              <Copy size={13} /> {t('Copy')}
            </button>
            <button className="btn small" disabled={rows.length === 0} onClick={exportCsv}>
              <Download size={13} /> CSV
            </button>
          </div>
          {res.rows.length === 0 ? (
            <div className="empty">{t('Nothing under this OID')}</div>
          ) : (
            <table className="pt">
              <colgroup>
                <col style={{ width: 270 }} />
                <col style={{ width: 210 }} />
                <col style={{ width: 120 }} />
                <col />
                <col style={{ width: '28%' }} />
              </colgroup>
              <thead>
                <tr>
                  <th>{t('OID')}</th>
                  <th>{t('Name')}</th>
                  <th>{t('Type')}</th>
                  <th>{t('Value')}</th>
                  <th>{t('Hexadecimal')}</th>
                </tr>
              </thead>
              <tbody>
                {rows.slice(0, 5000).map((r) => {
                  const base = res.base && r.oid.startsWith(`${res.base}.`) ? `${res.base}.` : '';
                  const v = valueText(r);
                  return (
                    <tr key={r.oid}>
                      <td className="ip oidc" title={r.oid}>
                        {base && <span className="base">{base}</span>}
                        {r.oid.slice(base.length)}
                      </td>
                      <td className="ip" title={oidName(r.oid)?.module}>
                        {oidName(r.oid)?.name ?? ''}
                      </td>
                      <td>
                        <span className={`ty ${TYPE_TONE[r.type] ?? ''}`}>{r.type}</span>
                      </td>
                      <td className={r.type === 'OID' || r.type === 'IpAddress' ? 'ip' : undefined} title={v}>
                        {r.type === 'OctetString' && r.text === '' ? <span className="muted2">""</span> : v}
                      </td>
                      <td className="hex" title={r.hex}>
                        {r.hex}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
        </>
      )}
    </>
  );
}
