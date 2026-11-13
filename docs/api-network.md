# Strata Network — network API (contract between server and web app)

Conventions of `docs/design.md` §2.4 apply: everything under `/api/...`, JSON in camelCase, errors are short
lowercase English plain text, `GET` needs a session, writes need `admin` or `operator` (`can_edit`) unless said
otherwise, 204 for writes without a body, 202 for work started in the background. Every change is announced to the
other clients with `{"type":"changed","what":"<list>"}` on the WebSocket (`what` given below) and written to the
history (§ History). Identifiers are 12 hex characters (`uuid simple()[..12]`) except VLANs (their number) and
addresses (their IP).

## Data

```ts
type Kind = 'computer' | 'server' | 'laptop' | 'phone' | 'printer' | 'camera' | 'monitor' | 'audio' | 'video'
          | 'switch' | 'router' | 'firewall' | 'wifi' | 'storage' | 'iot' | 'other';

interface Network {            // a range to scan
  id: string; name: string;
  range: string;               // "10.10.0.0/24", "10.10.0.1-10.10.0.80" (≤ 65 536 addresses)
  gateway?: string;            // IP
  vlan?: number;               // VLAN id
  ports?: string;              // custom TCP ports "22, 80, 8000-8010"; absent = default list
  scheduleMins: number;        // 0 = no planned scan; else every n minutes (15, 60, 360, 1440)
  lastScan?: { at: number; secs: number; up: number; stopped: boolean };   // at: unix ms
}

interface Port { port: number; service?: string; web?: { scheme: 'http' | 'https'; status: number; title?: string; server?: string }; banner?: string }

interface Address {            // only known addresses are stored; the client fills the rest of the range as free
  ip: string;
  label: string;               // '' = none
  kind?: Kind;
  notes: string;
  reserved: boolean;           // set by hand; a labelled never-seen address counts as reserved too
  watched: boolean;            // monitoring (ping every minute)
  up: boolean;                 // answered at the last scan / check
  via?: 'icmp' | 'tcp' | 'arp' | 'snmp';
  rttMs?: number;
  mac?: string; vendor?: string;
  name?: string; nameSource?: 'dns' | 'mdns' | 'netbios';
  ports: Port[];
  firstSeen?: number; lastSeen?: number;   // unix ms
  isNew: boolean;              // first seen by the last scan, until someone opens it (PATCH {isNew:false})
  parent?: string;             // topology: IP of the device it hangs from (set by hand)
  switch?: { id: string; port: number };   // where it is plugged (set by hand, or from a switch MAC table)
}
// state shown: up → 'used'; !up && lastSeen → 'offline'; reserved || label → 'reserved'; else 'free'.

interface Vlan { id: number /* 1..4094 */; name: string; color: string /* #rrggbb */; subnet?: string /* a.b.c.d/p */; gateway?: string; notes: string }

interface SwitchPort {
  n: number;                   // 1-based position on the faceplate (copper first, then SFP)
  name: string;                // interface name, e.g. "Gi1/0/12" (generated from the switch naming, or from SNMP)
  label: string;
  mode: 'access' | 'trunk' | 'off';
  vlan?: number;               // access VLAN, or native VLAN of a trunk
  tagged: number[];            // trunk VLANs
  host?: string;               // IP of the machine plugged in
  peer?: { id: string; port: number };   // another switch plugged in (uplink/downlink)
  learnt: string[];            // MACs seen on the port (SNMP, read-only)
  lldp?: { name: string; port: string }; // neighbour announced by LLDP (SNMP, read-only)
}

interface Switch {
  id: string; name: string; model: string; ip?: string;
  group: string;               // free grouping shown as a section of the left list ('' = none)
  copper: number; sfp: number; // faceplate: copper ports in pairs (odd on top), then SFP ports
  naming: string;              // "Gi1/0/{n}"; SFP: sfpNaming "Te1/1/{n}" ({n} restarts at 1)
  sfpNaming: string;
  snmp?: string;               // SnmpAgent id to read names, MAC table and LLDP from
  ports: SwitchPort[];         // always copper + sfp entries
}

interface SnmpAgent {          // read-only, SNMP v2c
  id: string; name: string; ip: string; port: number /* 161 */; community: string /* sent masked "••••••••" */;
  readArp: boolean;            // routers: find machines that answer nothing else
  readMulticast: boolean;      // routers: multicast routing table (streams, sources, rates)
  readSwitch: boolean;         // switches: interface names, MAC table, LLDP
  lastPoll?: { at: number; ok: boolean; message: string };
}

interface Stream {             // multicast; key = group:port:source
  group: string; port: number /* 0 = unknown (router table) */; source?: string;
  kind: 'video' | 'audio' | 'data' | 'mpegTs' | 'rtp' | 'ptp' | 'sap' | 'other';
  encoding?: string;           // from SDP: raw (ST 2110-20), jxsv (-22), L24/L16 (-30/AES67), smpte291 (-40), MP2T
  name?: string;               // from SAP/SDP
  label: string;               // set by hand (stored)
  rtpType?: number; ptpDomain?: number; ptpGrandmaster?: string;
  bps: number; pps: number; packets: number; history: number[];   // bps, oldest first, ≤ 60
  firstSeen: number; lastSeen: number;
  announced: boolean; routed: boolean;
}

interface Service { kind: string /* NDI, Dante, AES67 / RAVENNA, NMOS, Web… */; name: string; host: string; ip?: string; port: number; txt: string[] }

interface Alert {
  id: string; at: number;
  kind: 'newHost' | 'hostGone' | 'ipConflict' | 'vlanMismatch' | 'watchDown' | 'watchUp';
  ip: string; networkId?: string;
  detail: string;              // facts only, English, e.g. "00:11:32:AB:CD:EF and 3C:52:82:5A:00:E1"
  seen: boolean;
}

interface HistoryEntry { at: number; user: string /* display name, or "scheduler" */; action: string; target: string; detail: string }
```

## Routes

| Route | Body → answer | Notes |
| --- | --- | --- |
| `GET /api/networks` | → `Network[]` | `changed: networks` |
| `POST /api/networks` | `Network` without id → `Network` | 400 on a bad range |
| `PUT /api/networks/{id}` | `Network` → `Network` | |
| `DELETE /api/networks/{id}` | → 204 | its addresses stay (they belong to no network then) |
| `GET /api/networks/{id}/addresses` | → `Address[]` | known addresses inside the range; `changed: addresses` |
| `POST /api/networks/{id}/scan` | → 202 | 409 "a scan is already running" |
| `DELETE /api/networks/{id}/scan` | → 204 | stops it |
| `GET /api/networks/{id}/next-free` | → `{ip: string \| null}` | first address neither used, offline nor reserved, gateway excluded |
| `GET /api/networks/{id}/export.csv` | → `text/csv` | `ip,state,label,kind,name,mac,vendor,ports,notes,reserved,lastSeen` |
| `POST /api/networks/{id}/import` | CSV text (`ip,label,kind,notes,reserved` columns, header line, any order) → `{imported: n}` | merges by IP |
| `PATCH /api/addresses/{ip}` | any of `label, kind, notes, reserved, watched, parent, switch, isNew` (`null` clears) → `Address` | |
| `POST /api/addresses/{ip}/scan` | → `Address` | scans one address now |
| `GET /api/vlans` · `POST` · `PUT /{id}` · `DELETE /{id}` | `Vlan` | 409 "this vlan exists"; `changed: vlans` |
| `GET /api/switches` · `POST` · `PUT /{id}` · `DELETE /{id}` | `Switch` | PUT keeps ports by `n` when the faceplate size changes; `changed: switches` |
| `PATCH /api/switches/{id}/ports` | `{ports: number[], set: Partial<SwitchPort>}` → `Switch` | several ports at once |
| `GET /api/snmp` · `POST` · `PUT /{id}` · `DELETE /{id}` | `SnmpAgent` | admin only for writes; `changed: snmp` |
| `POST /api/snmp/{id}/poll` | → `SnmpAgent` | reads now (also every 5 min) |
| `POST /api/snmp/test` | `{ip, port, community}` → `{name, description}` | 502 + message on failure |
| `GET /api/multicast` | → `{iface: string, ifaces: {name, ip}[], streams: Stream[], checkMaxMbps: number}` | `changed: multicast` (sent at most every 2 s) |
| `PUT /api/multicast/settings` | `{iface, checkMaxMbps}` → 204 | admin |
| `POST /api/multicast/check` | `{group, port}` → `{received, cutShort, bps, pps}` | one at a time (409) |
| `PATCH /api/multicast/label` | `{group, port, source?, label}` → 204 | |
| `GET /api/services` | → `{at: number, services: Service[]}` | mDNS; `changed: services` |
| `POST /api/services/browse` | → 202 | one question per service type, answers for 3 s |
| `GET /api/alerts` | → `Alert[]` (newest first, ≤ 500) | `changed: alerts` |
| `POST /api/alerts/seen` | `{ids: string[] \| null}` (null = all) → 204 | |
| `GET /api/history?limit=` | → `HistoryEntry[]` (newest first) | admin |

WebSocket, besides `changed`:
- `{"type":"scan","networkId","running":bool,"done","total","up"}` while a scan runs (at most 4 per second) and once at the end.
- `{"type":"address","networkId","address":Address}` each time a scan examines a machine.
- The `hello` message carries `scans: {networkId: {done,total,up}}` for the scans running.

## Behaviour

- **Scan** (`strata_net::scan`): every address of the range by ICMP + TCP + ARP; a network with a `readArp` SNMP agent
  whose ARP table has the address counts it `up` with `via: 'snmp'`. Addresses not found become `up: false` (offline
  if seen before). Alerts: `newHost` (never seen before; not raised by the first scan of a network), `hostGone` (was up at the previous scan), `ipConflict`
  (the scan sees a MAC different from the one known and the old one is still in an ARP table, or two ARP tables
  disagree), `vlanMismatch` (an address plugged on a switch port whose access VLAN has a subnet that does not
  contain the address).
- **Planned scans**: `scheduleMins` > 0 runs the scan every n minutes (no two scans of the same network at once).
- **Monitoring**: every minute, each `watched` address is pinged (ICMP, else TCP on its known open ports); `watchDown`
  after 2 failures in a row, `watchUp` when it answers again; `up`/`rttMs` updated.
- **Multicast**: passive only (SAP/SDP, PTP, routers' multicast tables by SNMP). `check` joins one group at most
  2 s and stops above `checkMaxMbps` (default 100).
- **History**: every write route above adds an entry (`action`: "created network", "scanned", "changed port", …).
- Storage: `<data>/network/{networks,addresses,vlans,switches,snmp,multicast,alerts,history}.json`, atomic writes;
  history kept 1 year, alerts 500 newest.
