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
  isNew: boolean;              // first seen by the last scan (not the first scan of the network); cleared by the next scan or when opened
  parent?: string;             // topology: IP of the device it hangs from (set by hand)
  switch?: { id: string; port: number };   // where it is plugged (set by hand, or from a switch MAC table)
}
// state shown: up → 'used'; !up && lastSeen → 'offline'; reserved || label → 'reserved'; else 'free'.

interface Vlan { id: number /* 1..4094 */; name: string; color: string /* #rrggbb */; subnet?: string /* a.b.c.d/p */; gateway?: string; notes: string }

interface SwitchPort {
  n: number;                   // 1-based position on the faceplate (1..total, in group order)
  name: string;                // interface name, e.g. "Gi1/0/12" (generated from the naming of its group)
  label: string;
  mode: 'access' | 'trunk' | 'off';
  vlan?: number;               // access VLAN, or native VLAN of a trunk
  tagged: number[];            // trunk VLANs
  host?: string;               // IP of the machine plugged in
  peer?: { id: string; port: number };   // another switch plugged in (uplink/downlink)
  learnt: string[];            // MACs seen on the port (SNMP, read-only)
  lldp?: { name: string; port: string }; // neighbour announced by LLDP (SNMP, read-only)
}

interface PortGroup {          // ports of one kind side by side on the faceplate
  kind: 'rj45' | 'sfp' | 'qsfp';
  count: number;               // 1..256
  naming: string;              // "Gi1/0/{n}": {n} = start + index (a naming without {n} gets the number at the end)
  start?: number;              // first number; absent = previous group's last number + 1 when the previous
                               // group has the same naming, else 1 (Eth1/{n} 48 + 12 → Eth1/1…Eth1/60;
                               // Gi1/0/{n} 48 then Te1/1/{n} 4 → Te1/1/1…Te1/1/4)
}

interface Switch {
  id: string; name: string; model: string; ip?: string;
  group: string;               // free grouping shown as a section of the left list ('' = none)
  groups: PortGroup[];         // the faceplate, in order; 1 to 256 ports in all
  snmp?: string;               // SnmpAgent id to read names, MAC table and LLDP from
  ports: SwitchPort[];         // always one entry per faceplate port
  online: boolean;             // read live through `snmp` (default false)
  onlineSecs: 5 | 10 | 30 | 60;   // seconds between two live reads (default 10)
}

// Switches saved before port groups ({copper, sfp, naming, sfpNaming}) are migrated when loaded to
// [{rj45, copper, naming}, {sfp, sfp, sfpNaming}] (empty groups skipped) and saved back; ports keep their n.
// POST / PUT still accept that old shape.

interface SwitchModel {        // a faceplate to start from
  id: string;                  // built-in: a fixed slug ("cisco-n9k-c93240yc-fx2"); user models: 12 hex
  brand: string;               // '' for most user models
  model: string;               // "Nexus C93240YC-FX2"
  groups: PortGroup[];
}

interface Detected {           // what POST /api/switches/detect read
  system: { name: string; description: string; model: string; serial: string; firmware: string; location: string };
  interfaces: number;          // physical Ethernet ports found
  names: string[];             // their names, in natural order
  suggestion: { name: string; model: string; groups: PortGroup[]; matched: string | null };   // matched: built-in model id
}

interface Live {               // an online switch, in memory only (not saved)
  at: number;                  // last successful read (unix ms); 0 before the first one
  error?: string;              // why the last read failed (the last values stay)
  system?: { name: string; description: string; uptimeSecs: number; contact: string; location: string;
             model: string; serial: string; firmware: string };   // read when going online, then every 5 min
  ports: LivePort[];           // faceplate ports matched to an interface by name
  vlans?: { n: number; pvid?: number; untagged: number[]; tagged: number[] }[];  // read every 5 min; absent until read
  diffs: { n: number; field: 'mode' | 'vlan' | 'tagged' | 'off'; switch: string; inventory: string }[];
                               // switch / inventory: short facts ("access VLAN 10", "trunk 30, 40", "disabled",
                               // "VLAN 20", "no VLAN", "native VLAN 1"); only when `vlans` is known
}

interface LivePort {
  n: number; ifIndex: number;
  up: boolean; adminUp: boolean; speedMbps: number;
  inBps: number; outBps: number;           // from the counter deltas between two reads (a counter that went back is skipped)
  inErrors: number; outErrors: number;     // since the switch started
  alias: string;                           // port description in the switch
  inHistory: number[]; outHistory: number[];   // rates, oldest first, ≤ 60
}

interface SnmpAgent {          // read-only, SNMP v2c or v3
  id: string; name: string; ip: string; port: number /* 161 */; community: string /* sent masked "••••••••" */;
  v3?: { user: string; auth: 'none' | 'md5' | 'sha' | 'sha256'; authPassword: string; privacy: 'none' | 'des' | 'aes128'; privPassword: string } | null;   // SNMP v3 instead of the community; passwords sent masked
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
| `GET /api/switches` · `POST` · `PUT /{id}` · `DELETE /{id}` | `Switch` | PUT keeps ports by `n` when the faceplate size changes; 400 "a switch has at least one port", 400 "a switch has at most 256 ports"; `changed: switches` |
| `POST /api/switches/detect` | `{agentId?, ip?, port?, community?, v3?}` → `Detected` | reads the switch (SNMP, read-only); with `agentId` alone the agent is used as it is, with `ip` masked secrets are taken from `agentId` (as `/api/snmp/test`); 502 + SNMP message on failure |
| `GET /api/switch-models` | → `{builtin: SwitchModel[], mine: SwitchModel[]}` | built-in catalog compiled into the server + user models; `changed: switchModels` |
| `POST /api/switch-models` | `{brand, model, groups}` → `SwitchModel` | 400 "the name cannot be empty" (model), same port limits as a switch |
| `DELETE /api/switch-models/{id}` | → 204 | user models only; 404 "no such switch model" |
| `PATCH /api/switches/{id}/ports` | `{ports: number[], set: Partial<SwitchPort>}` → `Switch` | several ports at once |
| `PATCH /api/switches/{id}/online` | `{online?, onlineSecs?}` → 204 | 400 "the switch has no snmp agent", 400 "the read interval is 5, 10, 30 or 60 seconds"; `changed: switches` |
| `GET /api/switches/{id}/live` | → `Live` | 404 "the switch is not online" |
| `POST /api/switches/{id}/take` | `{ports: number[] \| null}` (null = every port with a diff) → `Switch` | copies the switch's setup into the inventory; 404 "the switch is not online", 409 "the vlan setup of the switch is not known yet"; `changed: switches` |
| `GET /api/snmp` · `POST` · `PUT /{id}` · `DELETE /{id}` | `SnmpAgent` | admin only for writes; `changed: snmp` |
| `POST /api/snmp/{id}/poll` | → `SnmpAgent` | reads now (also every 5 min) |
| `POST /api/snmp/test` | `{ip, port, community, v3?, id?}` → `{name, description}` | 502 + message on failure; masked secrets are taken from agent `id` |
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
- `{"type":"switchLive","id","live":Live}` after each live read of an online switch, and when its inventory changes.
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
- **Online switches** (read-only SNMP v2c): a task per online switch reads the interfaces' state and 64-bit counters
  every `onlineSecs` (also at server start for the switches saved online), the identity (system group, ENTITY-MIB
  chassis) and the VLAN setup (Q-BRIDGE-MIB) every 5 min. Interfaces are matched to the faceplate ports by name
  (`GigabitEthernet1/0/12` = `Gi1/0/12`). The switch's setup of a port: off when administratively down, trunk when
  it carries tagged VLANs (native = the PVID when it leaves untagged), else access on the PVID; it differs from the
  inventory when the mode, the access VLAN or the tagged VLANs differ (a trunk's native VLAN only when the
  inventory sets one; VLAN 1, where switches leave unconfigured ports, matches an inventory port without VLAN). `take` copies mode, VLAN and tagged VLANs (a disabled port keeps its VLANs; VLAN 1 becomes no VLAN unless
  the inventory has a VLAN 1), and the port description as label when the inventory has none. Removing the switch's agent puts it offline.
- **Detection** (`POST /api/switches/detect`): reads the system group and ENTITY chassis (name, description, model,
  serial, firmware, location) and the interface table (ifName or ifDescr, ifType, ifHighSpeed). Physical ports are the
  interfaces of type 6, 62, 69 or 117 (or, when the type is not given, named like Ethernet / Eth / Gi / Te / Twe / Fo /
  Hu / xe- / ge- / et- / ether / sfp), minus Vlan, Po / port-channel, Lo / loopback, mgmt, Null, Tunnel, nve, sup-eth;
  sorted naturally. When the ENTITY model or sysDescr names a built-in model (whole word, longest match wins) its groups
  are proposed (`matched` = its id); else consecutive ports with the same naming pattern (prefix + trailing number) and
  speed class (≥ 40 G qsfp, ≥ 10 G sfp, else rj45; a port without speed follows the one before) make a group.
- **History**: every write route above adds an entry (`action`: "created network", "scanned", "changed port", "switch online", "switch offline",
  "changed read interval", "took config from switch", "saved switch model", "deleted switch model", …).
- Storage: `<data>/network/{networks,addresses,vlans,switches,snmp,multicast,alerts,history,templates}.json`
  (`templates.json`: the user switch models), atomic writes;
  history kept 1 year, alerts 500 newest.
