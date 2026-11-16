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
  watched: boolean;            // alert when its link goes down (online switch, rule linkDown); default false
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
  rules: AlertRules;           // alerts raised while online (defaults below); changed by its own route only
}

interface Rule { on: boolean; value: number }   // value unused by linkDown, drift, neighbours
interface AlertRules {         // defaults
  linkDown: Rule;              // on            watched port's link down (admin up, oper down)
  flap: Rule;                  // on, 3         more than `value` link drops in 10 min (1..1000)
  errors: Rule;                // on, 100       more than `value` in+out errors per minute (≥ 1)
  discards: Rule;              // on, 10        more than `value` in+out discarded packets a second, averaged over 1 min (0.1.., one decimal)
  opticRx: Rule;               // on, -10       optic Rx power below `value` dBm (-40..10); the optic's own thresholds are not used
  saturation: Rule;            // on, 90        max(in, out) above `value` % of the link speed for 1 min (1..100)
  health: Rule;                // on, 60        power supply or fan not ok, or a temperature above `value` °C (20..150)
  drift: Rule;                 // on            a new switch / inventory difference (those there when the VLAN setup is first read are not new)
  neighbours: Rule;            // off           a CDP / LLDP neighbour appeared or disappeared (compared with the previous read)
  poe: Rule;                   // on, 85        PoE power used above `value` % of the budget (PoE switches only)
  ptpMute: Rule;               // on, 60        a device behind a PTP master port stopped sending Delay_Req: the port's received
                               //               PTP counter has not moved for more than `value` s (15..3600) while the link is up,
                               //               after it moved at least once (switches with CISCO-PTP-MIB only)
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
  inBps: number; outBps: number;           // from the counter deltas between two reads (a counter that went back is skipped; see Rates)
  inErrors: number; outErrors: number;     // since the switch started
  inDiscards: number; outDiscards: number; // ifIn/OutDiscards since the switch started (Counter32 as read)
  inDiscardsPps: number; outDiscardsPps: number;   // discarded packets a second, from the same two reads (to 0.01)
  alias: string;                           // port description in the switch
  inHistory: number[]; outHistory: number[];   // rates, oldest first, ≤ 60
}

interface InfoPort { n?: number /* faceplate port, when the interface is one */; port: string /* interface name */ }

interface SwitchInfo {         // an online switch's slow reads (strata_net::switchinfo), every 60 s, in memory only
  at: number;                  // last successful read (unix ms); 0 before the first one
  error?: string;              // why the last read failed entirely (the last values stay)
  errors: string[];            // the parts the switch did not answer ("optics: timed out", "poe: skipped"…)
  neighbours: (InfoPort & { name: string; remotePort: string; ip?: string; platform: string; chassisId: string; protocol: 'lldp' | 'cdp' })[];
  optics: (InfoPort & { kind: string; vendor: string; part: string; serial: string; rxDbm?: number; txDbm?: number /* weakest lane */;
            rxLanesDbm: number[]; tempC?: number; biasMa?: number; voltage?: number; rxLowWarnDbm?: number; rxLowAlarmDbm?: number })[];
  mcast: { vlan: number; group?: string /* read, else the one stream seen (multicast page) whose 01:00:5E MAC matches */;
           mac: string; name?: string /* stream label, else its SAP name */; ports: InfoPort[] }[];   // IGMP snooping
  routerPorts: { vlan: number; ports: InfoPort[] }[];   // multicast router ports
  health: { cpuPct?: number; cpu5minPct?: number; memUsedBytes?: number; memTotalBytes?: number;
            temps: { name: string; c: number }[];
            fans: { name: string; ok: boolean; detail: string }[];
            psus: { name: string; ok: boolean; detail: string; watts?: number }[];
            cpu: [number, number][] };   // [unix ms, CPU %] at each read, the last 24 h
            // detail: the switch's words ("up", "off (no input power)", "disabled"…)
  lags: { n: number; port: string; agg: string /* "port-channel1" */ }[];   // faceplate members of an aggregate
  stp: { n: number; state: 'disabled' | 'blocking' | 'listening' | 'learning' | 'forwarding' | 'broken' }[];   // members take the aggregate's state
  poe?: { budgetW?: number; usedW?: number;
          ports: (InfoPort & { adminOn: boolean; on: boolean; status: 'disabled' | 'searching' | 'delivering' | 'fault' | 'test' | 'otherFault' | 'unknown';
                               class?: number; watts?: number })[] };   // absent: no PoE; port "g/p" when not matched to an interface
  links: { n: number; switchId: string; port: number }[];   // neighbours that are another inventory switch, not linked yet (see link-neighbours)
  ptp?: PtpSwitch;             // the switch's PTP clock (CISCO-PTP-MIB, read every 15 s); absent: not read or no PTP
}

type PtpPortState = 'initializing' | 'faulty' | 'disabled' | 'listening' | 'preMaster' | 'master' | 'passive' | 'uncalibrated' | 'slave' | 'unknown';
interface PtpSwitch {          // the main clock: the one with a slave port, else the one with the most ports
  at: number;                  // last PTP read (unix ms)
  domain: number;
  clockType: 'ordinary' | 'boundary' | 'transparent' | 'boundaryNode' | 'other';
  identity?: string;           // "70:69:5A:FF:FE:EE:6F:79" (default DS)
  twoStep?: boolean;
  stepsRemoved?: number;       // current DS
  offsetNs?: number;           // offset from the master (the correction), ns
  meanPathDelayNs?: number;
  parent?: string; parentPort?: number;   // the master port it follows (parent DS)
  gm?: { identity: string; name?: string; ip?: string /* inventory address whose MAC the identity holds (EUI-64) */;
         priority1?: number; priority2?: number; clockClass?: number; clockAccuracy?: number; accuracyNs?: number /* none beyond the IEEE table */ };
  servo?: 'freerun' | 'holdover' | 'acquiring' | 'frequencyLocked' | 'phaseAligned';
  slave?: InfoPort;
  corrections: [number, number][];   // [unix ms, offsetNs] at each read, oldest first, ≤ 60
  ports: PtpPort[];            // every clock's ports: slave first, then masters, others, disabled
}
interface PtpPort extends InfoPort {   // InfoPort: from the MIB's ifIndex, else the interface named like the PTP port
  number: number;              // PTP port number
  domain: number;
  state: PtpPortState;
  logAnnounce?: number; logSync?: number; logDelayReq?: number;   // log2 s (port DS; logMinDelayReqInterval)
  announceTimeout?: number; delayMech?: 'e2e' | 'p2p' | 'disabled';
  rxPs?: number; txPs?: number;   // PTP packets a second, every kind, between the last two reads (absent: first read, no
                                  // counter, or a counter that went back). A master port receives the Delay_Req of the device behind it.
  activity?: 'active' | 'none' | 'mute';   // master ports, from the second read: received packets (active), never since
                                           // watched (none), stopped after some (mute)
  muteSince?: number;          // mute: the read that last saw the counter move (unix ms)
  device?: string;             // what is plugged: "<switch> · <port>" (inventory peer, or an LLDP name that is an inventory
                               // switch), the inventory host's name (else IP; else an NMOS node of that IP), the LLDP name, the
                               // inventory address (else NMOS node) of one of ≤ 3 learnt MACs, the port label
  deviceIp?: string;
  peerSwitch?: string;         // the port leads to this inventory switch

interface PortHistory {        // GET …/ports/{n}/history
  from: number; to: number;    // the range (unix ms)
  stepSecs: number;            // 1h: the read interval; 24h: 300; 7d: 1800
  points: { at: number; in: number; out: number; inPeak?: number; outPeak?: number;     // b/s; 24h / 7d: bucket average + peak
            inDiscards: number; outDiscards: number; inDiscardsPeak?: number; outDiscardsPeak?: number }[];   // packets/s, same
  in: { avg: number; peak: number }; out: { avg: number; peak: number };
  inDiscards: { avg: number; peak: number }; outDiscards: { avg: number; peak: number };   // packets/s
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
  format?: Format;             // from the SDP
  estBps?: number;             // what the SDP implies, headers included (see Bitrate)
  lastAnnounced?: number;      // last SAP announcement (unix ms)
  sapIntervalMs?: number;      // time between the last two announcements
  dup?: { group: string; port: number; source?: string };   // the other leg named by the SDP's a=group:DUP
  flooded: number;             // last time its packets reached the interface without a join (unix ms; 0 = never)
  floodVlan?: number;          // 802.1Q tag of those frames
}

interface Format {             // every field optional
  standard?: 'ST 2110-20' | 'ST 2110-22' | 'ST 2110-30' | 'ST 2110-31' | 'ST 2110-40' | 'ST 2022-6' | 'MPEG-TS';
  width?: number; height?: number; frameRate?: number; frameRateText?: string /* "60000/1001" */; interlaced?: true;
  sampling?: string /* "YCbCr-4:2:2" */; depth?: number /* bits a sample, audio too */; colorimetry?: string; tcs?: string;
  clockRate?: number; audioRate?: number; channels?: number; ptimeMs?: number;
  tsRefclk?: string /* a=ts-refclk as written */; ptpGm?: string /* "EC:46:70:FF:FE:00:1A:2B" */; ptpDomain?: number;
  mediaClock?: string /* a=mediaclk: "direct=0" */; bandwidthBps?: number /* b=AS ×1000 or b=TIAS */;
}

// ── Multicast flows (GET /api/multicast/flows) ──
interface Attach { switchId: string; switchName: string; n?: number; port: string /* interface name */;
                   edge: boolean /* not a switch link, an aggregate member or a trunk */;
                   via: 'inventory' | 'macTable' | 'lldp' /* lldp: a switch's LLDP / CDP neighbour, or the NMOS node's attached_network_device */ }
interface Receiver {           // a port of an IGMP snooping entry (or a multicast router port of its VLAN)
  switchId: string; switchName: string; n?: number; port: string; vlan: number;
  label?: string;              // the port's inventory label
  host?: string; hostIp?: string;   // the machine on the port (inventory name, else IP; else the LLDP neighbour); never on a switch link
  router: boolean;             // multicast router port
  peer?: string;               // the port (or aggregate) leads to this other switch (name)
  agg?: string;                // aggregate of the faceplate port ("port-channel1")
}
interface PathNode {           // one switch of the path, parents before children
  switchId: string; switchName: string;
  parent?: string;             // the switch before (none: the emitter's switch)
  egress?: string;             // the parent's port(s) towards it: aggregate name, else "Eth1/3 · Eth1/4"
  ingress?: string;            // its port(s) towards the parent; on the emitter's switch: the emitter's port
  igmpPorts: number; receivers: number;   // snooping ports of the group on it, and the edge ones among them
}
interface Flow {
  key: string;                 // "group:port:source" ("group:0:" for a group known from snooping only)
  group: string; port: number; source?: string;
  kind: Stream['kind']; encoding?: string;
  name?: string;               // stream label, else group label, else SAP name
  label: string;               // stream label ('' = none)
  groupLabel: string;          // group label (PUT /labels/{group}; '' = none)
  sapName?: string;
  announced: boolean; routed: boolean;
  flooded: boolean;            // a flood sample saw it in the last 11 min
  floodVlan?: number;
  snoopOnly: boolean;          // only in switches' snooping tables (no packet, no announcement)
  guessed: boolean;            // snoopOnly and the switch gave only the MAC: one of its 32 groups, guessed (see below)
  ambiguous: boolean;          // its snooping MAC matched several known groups of the same VLAN (all got the receivers)
  live: boolean;               // see Liveness
  state: 'ok' | 'warn' | 'unk' | 'off';   // off: not live; warn: conflict, pair missing or flooded; unk: not announced
  issues: ('unannounced' | 'noReceivers' | 'pairMissing' | 'conflict' | 'flooded' | 'gone')[];
  bps: number; pps: number;    // measured: router table, flood sample or check (passive)
  estBps?: number;             // from the SDP
  history: number[]; firstSeen: number; lastSeen: number /* last packet or announcement */;
  lastAnnounced?: number; sapIntervalMs?: number;
  format?: Format;
  vlan?: number;               // flood tag, else the VLAN (subnet) or network VLAN holding the source, else the plan range's VLAN
  emitter?: { ip: string; name?: string; mac?: string; at?: Attach };
  sources?: string[];          // conflict: every source of the group
  conflict: boolean;
  receivers: Receiver[];       // over every online switch whose slow read is done
  receiverCount: number;       // receivers that are neither router ports nor switch links
  path: PathNode[];            // from the emitter's switch to every switch that has the group in its snooping table
  unreachable: string[];       // those switches no chain of `peer` links reaches
  pair?: { key: string; group: string; port: number; source?: string; name?: string; at?: Attach };   // ST 2022-7 other leg
  pairState?: 'ok' | 'missing';  // missing: a leg is not live, or a Red/Blue-named stream has no other leg
  pairBy?: 'sdp' | 'name';
  leg?: 'a' | 'b';             // a: Red / Rouge / A / 1 / primary; b: Blue / Bleu / B / 2 / secondary
  origins: ('nmos' | 'sap' | 'igmp')[];   // in that order: an NMOS sender's SDP, a SAP announcement, switches' snooping / routers' tables
  nmos?: { senderId: string; sender: string /* label */; nodeId?: string; node?: string /* label */;
           inventory: boolean /* the node's addresses or MACs are in the inventory */; active?: boolean /* subscription.active */ };
  idle?: true;                 // an NMOS sender turned off (active false) and nothing else says it is sent: not live, no 'gone'
  subscribers?: { id: string; label: string; node?: string /* inventory name, else node label */; at?: Attach;
                  igmp: boolean /* a snooping receiver is on the same port: counted once */ }[];   // NMOS receivers set to its sender
}
interface PtpClock {           // a clock that sends Announce (224.0.1.129:320)
  ip: string; domain: number;
  clock: string;               // the sender's clock identity
  port: number;                // the sender's PTP port number
  gm: string;                  // the grandmaster it announces
  priority1: number; clockClass: number; clockAccuracy: number; accuracyNs?: number; variance: number; priority2: number;
  stepsRemoved: number; timeSource: 'atomic' | 'gps' | 'radio' | 'ptp' | 'ntp' | 'handSet' | 'other' | 'internalOscillator' | 'unknown';
  utcOffset: number; firstSeen: number; lastSeen: number; count: number; rate: number /* announces a second */;
  logAnnounce?: number;        // its Announce interval (log2 s, header logMessageInterval)
  logSync?: number;            // of its Sync / Follow_Up (absent until heard)
  logDelayReq?: number;        // the minimum Delay_Req interval of its Delay_Resp (absent until heard)
  name?: string;               // inventory name of its IP
  role: 'gm' | 'backup' | 'boundary';   // gm: best of its domain (BMCA) among those heard in the last 10 s
  live: boolean;               // heard in the last 10 s
  wrongDomain: boolean;        // not the main domain (that of the best clock of all)
  at?: Attach;
}
interface PtpDevice {          // a device that sends Delay_Req (224.0.1.129:319)
  clock: string; ip: string; domain: number; firstSeen: number; lastSeen: number; count: number;
  name?: string; mac?: string /* inventory, ARP, else from an EUI-64 clock identity */; live: boolean /* 30 s */;
  wrongDomain: boolean; at?: Attach;
}
interface PtpPlace { switchId: string; switchName: string; n?: number; port: string }
interface PtpGrandmaster {     // the grandmaster: the dataset the best Announce carries (`via: 'ptp'`), else a switch's
                               // parent dataset (`via: 'snmp'`); never the sender's address
  identity: string; name?: string; ip?: string;   // inventory address whose MAC the identity holds (EUI-64)
  domain: number; priority1?: number; priority2?: number; clockClass?: number; clockAccuracy?: number; accuracyNs?: number;
  timeSource?: string; live: boolean; via: 'ptp' | 'snmp';
  at?: PtpPlace;               // the slave port of the switch that follows it directly, else where its MAC is learnt
  portState?: PtpPortState;    // that slave port's state (SNMP)
  path: PtpPlace[];            // from the switch of the talker up to it: each switch's slave port
}
interface PtpTalker {          // the sender of the best Announce: what this machine hears
  clock: string; port: number; ip: string; name?: string; domain: number; stepsRemoved: number;
  role: 'gm' | 'boundary';     // gm: it is the grandmaster (its identity, 0 steps)
  live: boolean; logAnnounce?: number; logSync?: number; logDelayReq?: number;
  at?: PtpPlace;               // snmp: the switch whose clock identity it is, port of the Announce's port number;
                               // ip: the switch with its address, port where this machine is plugged; mac: a boundary sender
                               // not read, the switch where this machine's MAC is learnt
  via?: 'snmp' | 'ip' | 'mac';
  state?: PtpPortState;
}
interface PtpTreeNode {        // grandmaster, switches (parent: the slave port's inventory peer, else the switch whose identity
                               // its parent DS names, else the grandmaster; loops cut), devices on master ports
  id: string;                  // "gm", "s:<switch id>", "d:<switch id>:<ptp port>", "talker"
  kind: 'gm' | 'switch' | 'device' | 'clock';   // clock: the sender when no switch is read
  name: string; parent?: string;
  link: 'ok' | 'idle' | 'bad' | 'unknown';   // idle: no Delay_Req; bad: silent or another domain; unknown: guessed
  port?: string;               // switch: its slave port; device: the master port
  offsetNs?: number; activity?: 'active' | 'none' | 'mute'; clockType?: string; clockClass?: number; timeSource?: string; domain?: number;
}
interface GmChange { at: number; domain: number; from?: string; to?: string; fromIp?: string; toIp?: string }   // identities; absent = none
interface McastRange { id: string; cidr: string /* "239.10.1.0/24", /16 to /32, inside 224.0.0.0/4 */; usage: string; vlan?: number }
interface Reservation { group: string; name: string }
interface GroupUse { group: string; state: 'used' | 'reserved' | 'conflict'; name?: string }
interface McastRules {         // all true by default
  streamGone: boolean; noReceivers: boolean; conflict: boolean; flooding: boolean;
  pairMissing: boolean; gmChange: boolean; ptpDomain: boolean;
}
interface Flows {
  at: number;
  flows: Flow[];               // by group, port, source
  summary: { flows: number; live: number; byKind: Record<string, number>; estBps: number /* live flows */; bps: number;
             receivers: number /* distinct edge ports */; pairs: number; pairsIncomplete: number; unannounced: number;
             noReceivers: number; conflicts: number /* groups */; flooded: number; switchesOnline: number; switchesRead: number };
  switches: { id: string; name: string; ip?: string; online: boolean; read: boolean; at: number /* last slow read */ }[];
  links: { a: string; b: string; aPort: string; bPort: string }[];   // the peer links between them (names as in egress)
  plan: {
    ranges: (McastRange & { first: string; last: string; size: number; used: number /* incl. conflicts */; reserved: number;
              conflicts: number; free: number; nextFree?: string /* lowest free, x.x.x.0 and .255 skipped */;
              groups: GroupUse[] /* the groups not free, in order */ })[];
    outside: { used: number; reserved: number; conflicts: number; groups: GroupUse[] };   // « hors plan »
    reservations: Reservation[];
  };
  ptp: { gm?: PtpClock /* the best clock heard: the clock that talks to this machine, not always the grandmaster */;
         domain?: number /* main domain */; clocks: PtpClock[] /* BMCA order */; devices: PtpDevice[];
         changes: GmChange[] /* newest first, ≤ 50 */;
         grandmaster?: PtpGrandmaster; talker?: PtpTalker;
         switchSync: { switchId: string; switchName: string; clockType: string; slave?: InfoPort; offsetNs?: number;
                       meanPathDelayNs?: number; stepsRemoved?: number; history: number[] /* offsets, oldest first */ }[];
         ports: (PtpPort & { switchId: string; switchName: string; strata: boolean /* this machine's port */ })[];
                               // master ports of the switches read that face a device (not a switch link)
         tree: PtpTreeNode[] };
  flood: { iface: string; enabled: boolean /* floodWatch */; available: boolean; error?: string; at: number /* last sample */;
           groups: string[] /* flooded then */; vlan?: number };
  rules: McastRules;
  check: { enabled: boolean /* checkEnabled */; maxMbps: number };
}

interface Service { kind: string /* NDI, Dante, AES67 / RAVENNA, NMOS, NMOS Query, Web… */; name: string; host: string; ip?: string; port: number; txt: string[] }

interface Alert {
  id: string; at: number;
  kind: 'newHost' | 'hostGone' | 'ipConflict' | 'vlanMismatch' | 'watchDown' | 'watchUp'
      // switch alerts (online switches, rules of the switch):
      | 'portDown' | 'portFlap' | 'portErrors' | 'portDiscards' | 'portSaturated' | 'configDrift' | 'opticLow'
      | 'psuFault' | 'fanFault' | 'tempHigh' | 'poeBudget' | 'ptpMute' | 'neighbourNew' | 'neighbourGone'
      // multicast alerts (McastRules):
      | 'streamGone' | 'noReceivers' | 'groupConflict' | 'flooding' | 'pairMissing' | 'ptpGmChange' | 'ptpDomain';
  ip: string; networkId?: string;   // switch alerts: the switch's IP, else its agent's IP, else 0.0.0.0
  detail: string;              // facts only, English, e.g. "00:11:32:AB:CD:EF and 3C:52:82:5A:00:E1"
  seen: boolean;
  switchId?: string;           // switch alerts: the switch,
  port?: number;               // the faceplate port (none for power supply, fan, temperature, PoE budget),
  value?: number; limit?: number;   // and the measure with the rule's limit
}
// Switch alert details: portDown: the port label; configDrift: "<switch fact> / <inventory fact>";
// opticLow: the interface name when not a faceplate port; psuFault / fanFault: "<name> · <switch's state>";
// tempHigh: the sensor name; poeBudget: "<used> / <budget> W"; ptpMute: the device's name (limit: the rule's seconds);
// neighbourNew / Gone: "<remote name> <remote port>"
// (prefixed by "<interface> · " when not a faceplate port). Others: empty.
// Multicast alert details (ip: the source, else the group; flooding: the interface's address; PTP: the device):
// streamGone / noReceivers: "<group>:<port> <name>"; groupConflict: "<group> from <ip> and <ip>";
// flooding: "<iface> · VLAN <n> · <count> groups: <g>, <g>, <g>, <g> +<more>" ("no VLAN" when unknown);
// pairMissing: "<group>:<port> <name> · <other group>:<port> missing" or "… · other leg missing";
// ptpGmChange: "domain <d>: <identity> (<ip>) → <identity> (<ip>)" ("none" when there is none);
// ptpDomain: "<name or ip> domain <d>, grandmaster domain <main>".

interface HistoryEntry { at: number; user: string /* display name, or "scheduler" */; action: string; target: string; detail: string }

interface NotificationSettings {   // admin only; secrets sent masked "••••••••" and kept when sent back masked
  email: { enabled: boolean; host: string; port: number /* 587; 0 = 25 / 587 / 465 by security */;
           security: 'none' | 'starttls' | 'tls'; user: string /* '' = no authentication */;
           password: string /* masked */; from: string /* "Strata <strata@example.com>" */; to: string[] };
  webhooks: { id: string /* '' or absent when new */; name: string; kind: 'teams' | 'slack' | 'generic';
              url: string /* masked except scheme and host: "https://x.webhook.office.com/••••••••" */; enabled: boolean }[];
  filter: { kinds: 'all' | AlertKind[]; groupSecs: number /* 0..3600, default 60 */ };
  appUrl: string;              // the web app's address as the recipients reach it ('' = no link in the messages)
}
// Messages (French, server-written): subject "[<server name>] <kind label> · <place>" (one alert) or
// "[<server name>] <n> alertes"; one line per alert "16/11/2026 14:03:12 — Lien coupé — SW-CORE-1 · port 12 (Gi1/0/12) — <detail>"
// (time in the server's time zone; place: switch and port, else the IP and the address's label or name), at most 50 lines
// then "… et n autres", then "Ouvrir Strata Network : <appUrl>/alerts".
// teams: {"text": <lines joined by blank lines>}; slack: {"text": <lines>};
// generic: {"alerts": [{kind, at, switchId, switchName, port, ip, detail, value, limit}]} (absent fields null).
// Test message: same shapes; generic sends {"test": true, "text", "alerts": []}.

interface NmosSettings {       // admin only
  enabled: boolean;
  url: string;                 // the Query API as typed: "http://10.1.1.20:8010" or "…/x-nmos/query/v1.3" (trailing / removed)
  version: 'v1.2' | 'v1.3';    // read: <url up to /x-nmos/query>/x-nmos/query/<version>
  refresh: 'live' | '60s';     // live: polled every 5 s (no Query API subscription: nothing is created on the registry)
}
interface NmosStatus {
  enabled: boolean; url: string /* the Query API read, '' when off */; version: string; refresh: 'live' | '60s';
  registry?: string;           // its mDNS instance name (same address and port), else its host
  backup?: string;             // another Query API found by mDNS
  at: number;                  // last good read (0: none); its data stay in use while the registry does not answer
  tried: number; error?: string;   // last try and its error ("nmos: …", see the routes)
  nodes: number; devices: number; senders: number; video: number /* flow format video, else SDP */;
  receivers: number; subscribed: number /* subscription.sender_id set, active not false */;
  unknown: number;             // nodes (in the last read) none of whose addresses or MACs the inventory knows
  withoutSdp: number;          // senders whose SDP could not be read: not in the flows
}
interface NmosNode {           // GET /api/nmos/nodes, present ones first, by label
  id: string; label: string; hostname?: string;
  maker?: string;              // "Cobalt · BIDI4": asset tags (BCP-002-02) manufacturer and product, of the node else of its devices
  ip?: string;                 // its API endpoint (else href) address, else a media address
  ips: string[];               // API and media addresses (sources of its senders' SDPs)
  name?: string;               // its name in the inventory
  senders: number; receivers: number;
  at: Attach[];                // the switch ports it is on, most certain first
  inventory: boolean;
  seen: number; present: boolean;   // last read it was in; nodes that left stay 10 min with present false
}
interface NmosFound {          // a Query API found by mDNS (_nmos-query._tcp)
  name: string; host: string; ip?: string; port: number;
  url: string;                 // "<api_proto>://<ip>:<port>/x-nmos/query/<v1.3, else v1.2 when api_ver only has it>"
  priority?: number /* TXT pri */; proto: string /* TXT api_proto */; versions: string[] /* TXT api_ver */;
}

interface SnmpRow {            // raw SNMP explorer
  oid: string;                 // dotted
  type: 'Integer' | 'OctetString' | 'Null' | 'OID' | 'IpAddress' | 'Counter32' | 'Gauge32' | 'TimeTicks' | 'Counter64'
      | 'NoSuchObject' | 'NoSuchInstance' | 'EndOfMibView' | 'Unknown';   // Opaque is read as OctetString
  text: string;                // OctetString: the text when printable UTF-8 (trailing NULs dropped), else a MAC
                               // "00:11:32:AB:CD:EF" for 6 bytes, else the hex; OID / IpAddress dotted; numbers in
                               // decimal (TimeTicks as raw hundredths); '' for Null / NoSuch… / EndOfMibView
  hex?: string;                // OctetString only, always: "43 69 73 63 6F"
}
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
| `PATCH /api/switches/{id}/ports` | `{ports: number[], set: Partial<SwitchPort>}` → `Switch` | several ports at once (`label, mode, vlan, tagged, host, peer, watched`) |
| `PATCH /api/switches/{id}/online` | `{online?, onlineSecs?}` → 204 | 400 "the switch has no snmp agent", 400 "the read interval is 5, 10, 30 or 60 seconds"; `changed: switches` |
| `GET /api/switches/{id}/live` | → `Live` | 404 "the switch is not online" |
| `POST /api/switches/{id}/take` | `{ports: number[] \| null}` (null = every port with a diff) → `Switch` | copies the switch's setup into the inventory; 404 "the switch is not online", 409 "the vlan setup of the switch is not known yet"; `changed: switches` |
| `GET /api/switches/{id}/info` | → `SwitchInfo` | 404 "the switch is not online"; ports mapped to the faceplate as for `Live` |
| `GET /api/switches/{id}/ports/{n}/history?range=1h\|24h\|7d` | → `PortHistory` | default 1h; 400 "the range is 1h, 24h or 7d", 404 "no such switch" / "no such port"; also for a switch offline now |
| `PUT /api/switches/{id}/rules` | `AlertRules` → `Switch` | limits clamped; `changed: switches`; history "changed alert rules" |
| `POST /api/switches/{id}/link-neighbours` | → `{linked: number}` | records every `SwitchInfo.links` entry as `peer` on both ports (their `host` cleared); 404 "the switch is not online"; `changed: switches`; history "linked switches" |
| `GET /api/snmp` · `POST` · `PUT /{id}` · `DELETE /{id}` | `SnmpAgent` | admin only for writes; `changed: snmp` |
| `POST /api/snmp/{id}/poll` | → `SnmpAgent` | reads now (also every 5 min) |
| `POST /api/snmp/test` | `{ip, port, community, v3?, id?}` → `{name, description}` | 502 + message on failure; masked secrets are taken from agent `id` |
| `GET /api/multicast` | → `{iface: string, ifaces: {name, ip}[], streams: Stream[], checkMaxMbps: number}` | `changed: multicast` (sent at most every 2 s) |
| `PUT /api/multicast/settings` | `{iface, checkMaxMbps}` → 204 | admin |
| `POST /api/multicast/check` | `{group, port}` → `{received, cutShort, bps, pps}` | one at a time (409 "a check is already running"); refused before any join: 403 "the stream check is turned off" (`checkEnabled` false, the default), 404 "no such stream" (not seen or announced), 409 "the stream is heavier than the limit" (its `estBps` or its measured rate above `checkMaxMbps`), 409 "the rate of this video stream is not known" (video, -20/-22/2022-6, without an estimate) |
| `PATCH /api/multicast/label` | `{group, port, source?, label}` → 204 | |
| `GET /api/multicast/flows` | → `Flows` | `changed: multicast`; computed on each call |
| `PUT /api/multicast/labels/{group}` | `{label}` → 204 | names a group (unannounced ones); '' removes it; 400 "not a multicast group"; history "named group" |
| `GET /api/multicast/ranges` · `POST` · `PUT /{id}` · `DELETE /{id}` | `McastRange` | 400 "a range is inside 224.0.0.0/4", "a range holds at most 65536 groups (/16)", "a vlan is a number from 1 to 4094"; 409 "this range overlaps another one"; 404 "no such range"; history "created / changed / deleted multicast range" |
| `GET /api/multicast/reservations` · `POST` | `Reservation` | 409 "this group is already reserved"; history "reserved group" |
| `PUT /api/multicast/reservations/{group}` · `DELETE` | `{name}` → `Reservation` · 204 | 404 "no such reservation"; history "changed reservation" / "released group" |
| `GET /api/multicast/rules` · `PUT` | `McastRules` | history "changed multicast alert rules" |
| `GET /api/multicast/sdp/{group}/{port}` | → `application/sdp` text | the last SAP announcement, else the SDP of the NMOS sender of that group and port; 404 "no such sdp" |
| `PUT /api/multicast/options` | `{checkEnabled?, floodWatch?}` → 204 | admin; stored in multicast.json (`checkEnabled` default false, `floodWatch` default true); history "changed multicast settings" |
| `GET /api/services` | → `{at: number, services: Service[]}` | mDNS; `changed: services` |
| `GET /api/nmos` | → `NmosSettings` | admin; `changed: nmos` |
| `PUT /api/nmos` | `NmosSettings` → `NmosSettings` | admin; reads at once; 400 "the nmos version is v1.2 or v1.3", "the nmos refresh is live or 60s", "the registry address cannot be empty" (enabled), "the registry address is written http://host:port"; history "changed nmos settings" (detail "on · <url> · v1.3 · live") |
| `POST /api/nmos/test` | `{url?, version?}` (default: the saved ones) → `{url, nodes, devices, senders, receivers, ms}` | admin; one read of nodes, devices, senders, receivers (GET, paged); 400 as `PUT`; 502 + "nmos: timed out" / "nmos: unknown host" / "nmos: tls failed" / "nmos: cannot reach the registry" / "nmos: the registry answered <status>" / "nmos: not a query api" (not a JSON array) / "nmos: the answer is too big" / "nmos: bad url" |
| `GET /api/nmos/discovered` | → `{at, registries: NmosFound[]}` | admin; preferred first (`pri`, then name); asks the network again (mDNS, 3 s) when the last answers are older than 60 s |
| `GET /api/nmos/status` | → `NmosStatus` | `changed: nmos` (after each read) |
| `GET /api/nmos/nodes` | → `NmosNode[]` | `changed: nmos`; `[]` when off or never read |
| `POST /api/services/browse` | → 202 | one question per service type, answers for 3 s |
| `GET /api/alerts` | → `Alert[]` (newest first, ≤ 500) | `changed: alerts` |
| `POST /api/alerts/seen` | `{ids: string[] \| null}` (null = all) → 204 | |
| `GET /api/history?limit=` | → `HistoryEntry[]` (newest first) | admin |
| `GET /api/notifications` | → `NotificationSettings` | admin; `changed: notifications` |
| `PUT /api/notifications` | `NotificationSettings` → `NotificationSettings` | admin; 400 "the smtp server cannot be empty", "the email has no recipient", "the sender address cannot be empty", "not an email address: <a>", "the webhook url starts with https://", "two webhooks have the same id", "the app address starts with https://"; history "changed notifications" |
| `POST /api/notifications/test` | `{channel: 'email' \| <webhook id>}` → `{sent: true, ms}` | admin; sends now (enabled or not) with the saved settings; 400 "the email is not set up", 404 "no such webhook", 502 + "smtp: authentication failed" / "smtp: the server refused the message (<code>)" / "smtp: the server answered <code>" / "smtp: tls failed" / "smtp: the server does not offer starttls" / "smtp: timed out" / "smtp: cannot connect to the server" / "webhook: the server answered <status>" / "webhook: timed out" / "webhook: unknown host" / "webhook: tls failed" / "webhook: cannot reach the server"; history "tested notification" (detail "sent" or the error) |
| `POST /api/snmp/{id}/walk` | `{oid, max?}` → `{rows: SnmpRow[], truncated: boolean, error?: string, ms: number}` | admin; GETBULK steps under `oid` (`max` default 2000, at most 10 000); nothing under it → GET of `oid` itself; `truncated` when more rows were there or the walk stopped (`error`: "timed out" after 20 s, or the SNMP error after the first rows); 400 "the oid is written with numbers and dots (1.3.6.1.2.1.1)", 404 "no such snmp agent", 502 + SNMP message when nothing could be read |
| `POST /api/snmp/{id}/get` | `{oids: string[]}` → `{rows: SnmpRow[], ms}` | admin; 1..64 OIDs (400 "no oid to read", "at most 64 oids at once"); a missing one comes back NoSuchObject / NoSuchInstance; 502 on failure |

WebSocket, besides `changed`:
- `{"type":"scan","networkId","running":bool,"done","total","up"}` while a scan runs (at most 4 per second) and once at the end.
- `{"type":"address","networkId","address":Address}` each time a scan examines a machine.
- `{"type":"switchLive","id","live":Live}` after each live read of an online switch, and when its inventory changes.
- `{"type":"switchInfo","id","info":SwitchInfo}` after each slow read of an online switch, and after `link-neighbours`.
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
  2 s and stops above `checkMaxMbps` (default 100); it is off unless `checkEnabled`, and refused before joining when
  the stream is not known, its SDP estimate or measured rate is above the limit, or it is video with no estimate.
  Groups joined: the SAP groups (239.255.255.255, 224.2.127.254) and PTP (224.0.1.129) all the time, a media group
  only by a check. Nothing is ever sent: no IGMP query, no PTP, no SAP.
- **Multicast flows** (`net/mflows.rs`, `strata_net::{mcast, mformat, ptp, flood}`):
  - *Receivers*: the IGMP snooping entries (`SwitchInfo.mcast`) and multicast router ports of every online switch
    already read. An entry that gives only the MAC (Q-BRIDGE) goes to the known groups (streams, named groups,
    reservations) with that MAC, those whose VLAN is the entry's first (32 groups share a MAC); several left: all get
    it, `ambiguous`. None known: the group of a plan range holding one of its 32 groups (the range of the entry's
    VLAN first), else 239.<mac[3]>.<mac[4]>.<mac[5]>, `guessed`. MACs of control groups (224.0.0.x, PTP, SAP, SSDP)
    are left out, as are those groups as flows.
  - *Emitter port*: the source's address `switch` when set, else its MAC (inventory, else a router's ARP table) in the
    switches' MAC tables (SNMP poller, `SwitchPort.learnt`): the port with no `peer`, not an aggregate member, not a
    trunk, with the fewest MACs wins.
  - *Path*: breadth first over `SwitchPort.peer` (either side recorded) from the emitter's switch to every switch
    holding the group; the union of the shortest chains; parallel links named by their aggregate (slow read) when all
    are members of one.
  - *Liveness*: an announcement within max(30 s, 3 × the announcement interval), or packets of a check within 60 s,
    or a flood sample or a router's table within 11 min. A group only in snooping tables is live while it is there.
  - *Conflict*: two live sources for one group, unless both sources are in two different known VLANs (the two legs
    of a pair sent on one group in two networks).
  - *ST 2022-7*: the medias of one SDP's `a=group:DUP` are paired; else two streams whose names differ only by a leg
    word (Rouge/Red ↔ Bleu/Blue anywhere in the name; A ↔ B, 1 ↔ 2, primary ↔ secondary as last word; case and
    separators ignored), same kind, encoding, size, frame rate and channels, and a different group, VLAN or source;
    a unique candidate (else the one with the same port). `missing`: one leg not live, or a Red/Blue name with no
    other leg.
  - *Bitrate* (`estBps`): ST 2110-20 width × height × samples a pixel (4:4:4/RGB 3, 4:2:2 2, 4:2:0 1.5) × depth ×
    frame rate (exactframerate; interlaced: frames) × 1.05 (RTP/UDP/IP headers of ~1200-byte packets) — 1080p50 4:2:2
    10 bits ≈ 2.18 Gb/s, 1080p59.94 ≈ 2.61 Gb/s (active video, not the SDI raster); -30/-31: rate × channels × sample
    bytes (L16 2, L24 3, AM824 4) + 40 bytes a packet (1 / ptime, default 1 ms); -40: b=AS else 200 kb/s; 2022-6:
    1.485 Gb/s, 2.970 above 1080i / 1080p30, × (1376 + 48) / 1376; others (MPEG-TS, -22): b=AS / b=TIAS.
  - *Flooding* (`floodWatch`, on by default): every 5 min (first 10 s after start or after being turned on) the
    interface is sampled for 1 s with a packet socket, a BPF filter keeping 164 bytes of IPv4 UDP multicast frames,
    and all-multicast reception on the interface (local to the machine; Linux, needs root or CAP_NET_RAW; receive
    only, nothing sent, no group joined, no IGMP): UDP to a multicast group that
    looks like media (RTP v2 or MPEG-TS) for a group not joined on the machine (`/proc/net/igmp`, 224.0.0.x left out),
    frames sent by the machine ignored, a sample during a check discarded. Those streams get `flooded` and the
    sample's rate (`flooded` for 11 min). Unavailable: `flood.available` false with the error.
  - *PTP*: Announce (BMCA order: priority1, clockClass, clockAccuracy, variance, priority2, grandmaster identity,
    stepsRemoved, sender) and Delay_Req / Pdelay_Req on the groups already listened to; clocks and devices are
    forgotten after an hour. The grandmaster of a domain changing (or none heard for 10 s) is recorded in `ptp.changes`
    (multicast.json, 50 kept; the first one heard is not a change).
  - *Alerts* (every 5 s, rules of `McastRules`): `streamGone` when an announced live stream stops being live (not
    when a new interface restarts the engine); `noReceivers` when a live stream's edge receivers all left while the
    switches they were on are still read; `groupConflict`; `flooding` (per interface and VLAN); `pairMissing` (a live
    leg whose pair is missing); `ptpGmChange`; `ptpDomain` (a live clock or device not in the main domain). Conditions
    must hold 15 s, then are raised once until they clear; transitions (`streamGone`, `ptpGmChange`) at once.
- **NMOS** (`net/nmos.rs`, `net/mflows/registry.rs`): read only, HTTP GET and nothing else — no registration, no
  IS-05 connection, no Query API subscription (WebSocket), never a write to the registry or a node.
  - *Reads*: while `enabled`, every 5 s (`live`) or 60 s, and at once after `PUT /api/nmos`: `nodes`, `devices`,
    `sources`, `flows`, `senders`, `receivers` of the Query API, each from `?paging.since=0:0&paging.limit=500`
    following `Link: rel="next"` (else `X-Paging-Until`) until a page is empty, shorter than `X-Paging-Limit`, or
    brings no new id (200 pages at most); a registry that answers 400 to the paging parameters is asked without them
    (everything at once). 3 s to connect, 10 s a request, no proxy, 64 MB at most. Items without `id` are skipped;
    every field is optional (a missing or mistyped one is ignored). One resource that fails fails the read: the
    last good read stays in use (`error` set) until the next good one.
  - *SDP*: each sender's `manifest_href` (http/https only, GET, 256 kB) is read when the sender is new or its
    `version` or `manifest_href` changed, else kept; a failed read is tried again after 60 s; 8 at once at most.
    Parsed as the SAP SDPs (`strata_net::mcast::parse_sdp`): multicast medias, format, `estBps`, `a=group:DUP`.
  - *Flows*: each multicast media of a sender's SDP is merged into the flow of the same group and port (same source
    first, else one without source; a different source makes another flow, then a conflict), else makes a new flow
    (origin `nmos`, `announced`: the registry announces it). Name: the hand label, else the group label, else the
    sender's label (the SAP name stays in `sapName`); format, encoding, kind (flow format), `estBps` from the SDP when
    not known. An SDP with two medias (ST 2022-7) gives two flows paired by `a=group:DUP` (`pairBy: 'sdp'`).
  - *Liveness*: an NMOS flow is live when its sender is in the registry and not turned off (`subscription.active`
    not false; absent before v1.2), or when SAP / packets / tables say so. A sender in the registry but turned off
    with nothing else is `idle`: not live (`state: 'off'`), no `gone` issue, and no `streamGone` alert when it goes
    idle — only a sender that leaves the registry (or a SAP stream that stops) is gone. While the registry does not
    answer, the last read is kept (no alert). Turning NMOS off or changing its address starts the alert watcher
    again (as a new multicast interface does).
  - *Receivers*: an NMOS receiver whose `subscription.sender_id` is the flow's sender (and `active` not false) is a
    subscriber; when a snooping receiver is on one of its node's ports, they are one receiver (`igmp`, named after
    the node when the port has no inventory host); `receiverCount` adds the subscribers not in the snooping tables;
    the path goes to their switches too. The `noReceivers` alert still reads the snooping tables only.
  - *Nodes* (where they are plugged, in this order): the inventory switch port of one of their addresses (API
    endpoints and `href`, and the sources of their senders' SDPs); a switch's LLDP / CDP neighbour (edge ports only)
    whose chassis ID is one of the node's interface MACs (`chassis_id` / `port_id`), whose address is one of the
    node's, or whose name is the node's host name or label (host part, case ignored); the v1.3
    `attached_network_device` (its chassis ID as another inventory switch's neighbour table names that switch, its
    `port_id` the same interface as a port, `Ethernet1/12` = `Eth1/12`); the switches' MAC tables (the node's MACs,
    and the MACs of its addresses from the inventory or the routers' ARP tables). `inventory`: one of its addresses
    is an inventory address, or one of its MACs an inventory address's MAC. An emitter without inventory name takes
    its node's name, without port its node's port for that address (or its only port).
  - *mDNS*: `_nmos-query._tcp` (kind "NMOS Query") with the other service types (`POST /api/services/browse`, every
    10 min); `_nmos-register._tcp` services are not offered (only Query APIs are read).
- **Online switches** (read-only SNMP v2c): a task per online switch reads the interfaces' state and 64-bit counters
  every `onlineSecs` (also at server start for the switches saved online), the identity (system group, ENTITY-MIB
  chassis) and the VLAN setup (Q-BRIDGE-MIB) every 5 min. Interfaces are matched to the faceplate ports by name
  (`GigabitEthernet1/0/12` = `Gi1/0/12`). The switch's setup of a port: off when administratively down, trunk when
  it carries tagged VLANs (native = the PVID when it leaves untagged), else access on the PVID; it differs from the
  inventory when the mode, the access VLAN or the tagged VLANs differ (a trunk's native VLAN only when the
  inventory sets one; VLAN 1, where switches leave unconfigured ports, matches an inventory port without VLAN). `take` copies mode, VLAN and tagged VLANs (a disabled port keeps its VLANs; VLAN 1 becomes no VLAN unless
  the inventory has a VLAN 1), and the port description as label when the inventory has none. Removing the switch's agent puts it offline.
- **Slow reads of an online switch** (`strata_net::switchinfo::all`, read-only GET/GETNEXT/GETBULK, same SNMP session
  kind as the live reads): when it goes online, then every 60 s. Interfaces are mapped to the faceplate through the
  live reads (ifIndex → port), the other interfaces keep their name (`port-channel1`).
- **PTP of an online switch** (`strata_net::switchinfo::ptp` + `strata_net::ptpmib`, read-only walks of CISCO-PTP-MIB
  ciscoPtpMIBClockInfo `1.3.6.1.4.1.9.9.760.1.2`): with the first slow read, then every 15 s while the switch has a
  PTP clock (else again with each slow read). Tables (entry `.1`, index domain.clockType.instance[.portNumber]):
  3 default DS (4 twoStep, 5 identity, 6/7 priorities, 8 slaveOnly, 9 class, 10 accuracy), 9 port running (5 name,
  6 state, 8 ifIndex, 13/14 packets received/sent, Counter64), 1 current DS (4 stepsRemoved, 5 offsetFromMaster,
  6 meanPathDelay), 2 parent DS (4 parent port identity, 8 GM identity, 9/10 GM priorities, 11 GM class, 12 GM
  accuracy), 4 running (4 servo state), 5 time properties (11 time source), 8 port DS (5 name, 6 port identity,
  7 logAnnounceInterval, 8 announceReceiptTimeout, 9 logSyncInterval, 10 logMinDelayReqInterval, 12 delay
  mechanism); 7 port table only for names when 8 and 9 give none. Tables 3 and 9 answering nothing: no PTP (the
  switch tab is hidden, the multicast view keeps to what is heard). Agents that move columns: a port's name is taken
  from any column of interface-like texts, its state from an integer column holding only 1..9, its counters from the
  Counter columns in order (received, sent). Time intervals (ClockTimeInterval, an OCTET STRING): text in ns ("−12",
  "6900 ns", "6.9 us"), 8 bytes of IEEE 1588 scaledNanoseconds (ns × 2^16), or an integer in ns. Identities: 8 bytes
  (port identity 8 + 2) or hex text. Ports map to the faceplate by the ifIndex column, else by name. Between two
  reads, the received counter of a master port gives the Delay_Req of its device (`activity`, `rxPs`); the
  offsets of the main clock are kept (60) for the correction curve. Rule `ptpMute` checked after each PTP read.
- **Multicast PTP view**: the best Announce heard (BMCA) is the *talker*; its grandmaster identity and dataset are the
  grandmaster's (a boundary clock relays them: its address is not the grandmaster's). The talker is a switch when
  a read switch has its clock identity (its port: the PTP port of the Announce's port number), or when its address is
  an inventory switch's; a boundary sender not read is taken as the switch where this machine is plugged. Without
  Announce, the switches' parent DS give the grandmaster (`via: 'snmp'`). The grandmaster is named by the inventory
  address of the MAC its EUI-64 identity holds. Rows (`ports`): the master ports of the read switches that face a
  device; the talker's port is this machine's.
- **Neighbour switches**: a CDP / LLDP neighbour on a faceplate port is another inventory switch when its name equals
  that switch's name or its name in the switch (sysName, when it is online), host part only and case ignored, or its
  IP is the switch's IP or its agent's IP; its port is the one whose name is the same interface (`Ethernet1/1` =
  `Eth1/1`). Proposed in `links` unless the two ports are already linked to each other, or the other end is linked
  to a third port.
- **Rates** (`strata_net::snmp::if_stats`, every live read): ifName, ifAlias, ifAdminStatus, ifOperStatus,
  ifHighSpeed, ifHCIn/OutOctets (Counter64), ifIn/OutErrors, ifIn/OutDiscards (Counter32). ifHCInOctets is walked with
  sysUpTime.0 as a GETBULK non-repeater (`1.3.6.1.2.1.1.3` asked by GETNEXT in every request of that walk), so each
  interface's octets come with the switch's uptime of the same answer; the other columns follow in the next requests
  of the read, at a near constant offset. An agent that refuses the non-repeater is walked without it.
  The interval of a rate is the sysUpTime delta (wrapping at 2^32 cs) when both reads have it and it is within half
  the interval (at least 5 s) of the server's clock; else the server's clock, taken right after the counters (a
  switch that restarted, no sysUpTime). Octet counters that went back skip the sample; a Counter32 (discards) that
  went from the upper half of 2^32 to the lower half wrapped, else it was cleared and gives 0.
  *Cached counters* (NX-OS refreshes its interface counters every few seconds): when both octet counters of a port
  that is up did not move while it had traffic (or before its first rate), the read is stale: nothing is recorded,
  the rates stay, and the next read that moved gives the rate over the whole time. At most 2 reads are skipped in a
  row (a rate covers at most 3 intervals), then a stop is taken as real. Once stale reads came twice among the last 24
  moves of a port's counters while it carries ≥ 10 kb/s, each move is taken as one refresh of the switch and the
  interval is the refresh period (the average time between those moves), not the time between the two reads that
  saw it, which jitters by up to one read interval as the reads drift across the refreshes. The first rate of a port is computed from
  the counters' first move, not from the first read (which can catch them anywhere in their refresh period).
- **Rate history**: each live rate and discard rate is kept per faceplate port: 1 hour as read, 24 hours in 5-minute
  buckets, 7 days in 30-minute buckets (sum, count and peak); written every 5 minutes to
  `<data>/network/rates/<switchId>.json` (read back when first needed, removed with the switch). Fine entries are
  `[ms, in, out, inDiscards, outDiscards]`, buckets `[start, n, sumIn, sumOut, peakIn, peakOut, sumInDisc,
  sumOutDisc, peakInDisc, peakOutDisc]`; files saved before the discards (3 and 6 numbers) read back with none.
- **Switch alerts**: the rules are checked at each live read (link down, flapping, errors, discards, saturation, differences)
  and each slow read (optics, power supplies, fans, temperatures, PoE budget, neighbours). A condition raises its
  alert once, then not again until it has cleared (in memory: a restart may raise it again, but never while the same
  alert waits unseen); the drops of a flapping port are not reported one by one. A part of the slow read that failed
  keeps its conditions as they were. `portDiscards`: the in + out discard rate of the reads, weighted by time, over the
  last minute (once a minute is covered), above the limit (`value` = that average in packets/s); like `portErrors`
  it stays raised while the port still drops, under the limit. `portSaturated` compares the busier direction,
  max(in, out), with the link speed (full duplex), never their sum.
- **Detection** (`POST /api/switches/detect`): reads the system group and ENTITY chassis (name, description, model,
  serial, firmware, location) and the interface table (ifName or ifDescr, ifType, ifHighSpeed). Physical ports are the
  interfaces of type 6, 62, 69 or 117 (or, when the type is not given, named like Ethernet / Eth / Gi / Te / Twe / Fo /
  Hu / xe- / ge- / et- / ether / sfp), minus Vlan, Po / port-channel, Lo / loopback, mgmt, Null, Tunnel, nve, sup-eth;
  sorted naturally. When the ENTITY model or sysDescr names a built-in model (whole word, longest match wins) its groups
  are proposed (`matched` = its id); else consecutive ports with the same naming pattern (prefix + trailing number) and
  speed class (≥ 40 G qsfp, ≥ 10 G sfp, else rj45; a port without speed follows the one before) make a group.
- **History**: every write route above adds an entry (`action`: "created network", "scanned", "changed port", "switch online", "switch offline",
  "changed read interval", "took config from switch", "saved switch model", "deleted switch model",
  "changed alert rules", "linked switches", "changed notifications", "tested notification", "changed nmos settings", …).
- **Notifications**: every alert raised (`Net::alert`, `Net::switch_alert`) is queued without waiting (1000 at most,
  more are dropped and logged). Alerts of the kinds of `filter` are grouped: the first opens a window of `groupSecs`,
  every alert raised before its end goes in the same message, one message per enabled channel (e-mail, each enabled
  webhook). SMTP with lettre (rustls, the system's certificates), 15 s; webhooks POST JSON, 10 s; a failed send is
  logged, not retried. Messages go to the mail server and the webhook URLs only.
- **SNMP explorer**: read-only (GET and GETBULK, never SET), the agent's v2c or v3 session.
- Storage (`multicast.json` also keeps `groupLabels`, `ranges`, `reservations`, `rules`, `ptpChanges`, `checkEnabled`, `floodWatch`): `<data>/network/{networks,addresses,vlans,switches,snmp,multicast,alerts,history,templates}.json`
  (`templates.json`: the user switch models), `<data>/network/notifications.json` (`NotificationSettings`, unmasked; 0600 like `snmp.json`), `<data>/network/nmos.json` (`NmosSettings`) and `<data>/network/rates/<switchId>.json` (rate history), atomic writes;
  history kept 1 year, alerts 500 newest.
