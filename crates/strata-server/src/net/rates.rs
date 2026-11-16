//! Bandwidth history of the online switches' ports: 1 hour at the live read
//! interval, 24 hours in 5-minute buckets, 7 days in 30-minute buckets
//! (average and peak). Kept in memory and written to `<data>/network/rates/`
//! every 5 minutes, so that a restart keeps it.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::Net;
use crate::paths::{now_ms, read_json};

const HOUR: i64 = 3_600_000;
const MID_STEP: i64 = 300_000;
const MID_SPAN: i64 = 24 * HOUR;
const COARSE_STEP: i64 = 1_800_000;
const COARSE_SPAN: i64 = 7 * 24 * HOUR;

/// One bucket: start (unix ms), samples, sums and peaks of the in / out rates (b/s),
/// then sums and peaks of the in / out discards (packets/s).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "BucketRepr")]
pub struct Bucket(pub i64, pub u32, pub f64, pub f64, pub u64, pub u64, pub f64, pub f64, pub f64, pub f64);

/// A bucket as saved: with discards, or as saved before them.
#[derive(Deserialize)]
#[serde(untagged)]
enum BucketRepr {
    Now(i64, u32, f64, f64, u64, u64, f64, f64, f64, f64),
    Before(i64, u32, f64, f64, u64, u64),
}

impl From<BucketRepr> for Bucket {
    fn from(r: BucketRepr) -> Self {
        match r {
            BucketRepr::Now(a, n, si, so, pi, po, sdi, sdo, pdi, pdo) => Bucket(a, n, si, so, pi, po, sdi, sdo, pdi, pdo),
            BucketRepr::Before(a, n, si, so, pi, po) => Bucket(a, n, si, so, pi, po, 0.0, 0.0, 0.0, 0.0),
        }
    }
}

impl Bucket {
    fn avg(&self) -> (u64, u64) {
        let n = f64::from(self.1.max(1));
        ((self.2 / n).round() as u64, (self.3 / n).round() as u64)
    }

    fn avg_discards(&self) -> (f64, f64) {
        let n = f64::from(self.1.max(1));
        (self.6 / n, self.7 / n)
    }
}

/// One rate as read: unix ms, in / out b/s, in / out discards (packets/s).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "FineRepr")]
pub struct Fine(pub i64, pub u64, pub u64, pub f64, pub f64);

#[derive(Deserialize)]
#[serde(untagged)]
enum FineRepr {
    Now(i64, u64, u64, f64, f64),
    Before(i64, u64, u64),
}

impl From<FineRepr> for Fine {
    fn from(r: FineRepr) -> Self {
        match r {
            FineRepr::Now(a, i, o, di, d_o) => Fine(a, i, o, di, d_o),
            FineRepr::Before(a, i, o) => Fine(a, i, o, 0.0, 0.0),
        }
    }
}

/// A new reading of a port: rates (b/s) and discards (packets/s).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sample {
    pub n: u16,
    pub in_bps: u64,
    pub out_bps: u64,
    pub in_discards: f64,
    pub out_discards: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PortRates {
    /// The last hour, as read.
    #[serde(default)]
    pub fine: VecDeque<Fine>,
    #[serde(default)]
    pub mid: VecDeque<Bucket>,
    #[serde(default)]
    pub coarse: VecDeque<Bucket>,
}

fn into_bucket(list: &mut VecDeque<Bucket>, step: i64, span: i64, at: i64, s: &Sample) {
    let start = at - at.rem_euclid(step);
    let (i, o) = (s.in_bps, s.out_bps);
    match list.back_mut() {
        Some(b) if b.0 == start => {
            b.1 += 1;
            b.2 += i as f64;
            b.3 += o as f64;
            b.4 = b.4.max(i);
            b.5 = b.5.max(o);
            b.6 += s.in_discards;
            b.7 += s.out_discards;
            b.8 = b.8.max(s.in_discards);
            b.9 = b.9.max(s.out_discards);
        }
        _ => list.push_back(Bucket(start, 1, i as f64, o as f64, i, o, s.in_discards, s.out_discards, s.in_discards, s.out_discards)),
    }
    while list.front().is_some_and(|b| b.0 < at - span - step) {
        list.pop_front();
    }
}

impl PortRates {
    pub fn add(&mut self, at: i64, s: &Sample) {
        // A clock that went back: start the fine list again rather than mix the order.
        if self.fine.back().is_some_and(|x| x.0 > at) {
            self.fine.clear();
        }
        self.fine.push_back(Fine(at, s.in_bps, s.out_bps, s.in_discards, s.out_discards));
        while self.fine.front().is_some_and(|x| x.0 < at - HOUR) {
            self.fine.pop_front();
        }
        into_bucket(&mut self.mid, MID_STEP, MID_SPAN, at, s);
        into_bucket(&mut self.coarse, COARSE_STEP, COARSE_SPAN, at, s);
    }
}

/// The history of a switch: by faceplate port.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SwitchRates {
    #[serde(default)]
    pub ports: BTreeMap<u16, PortRates>,
    #[serde(skip)]
    pub dirty: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Range {
    Hour,
    Day,
    Week,
}

impl Range {
    pub fn parse(s: &str) -> Option<Range> {
        match s {
            "1h" => Some(Range::Hour),
            "24h" => Some(Range::Day),
            "7d" => Some(Range::Week),
            _ => None,
        }
    }

    pub fn span(self) -> i64 {
        match self {
            Range::Hour => HOUR,
            Range::Day => MID_SPAN,
            Range::Week => COARSE_SPAN,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Point {
    pub at: i64,
    #[serde(rename = "in")]
    pub inn: u64,
    pub out: u64,
    /// Peaks of the bucket (24 h and 7 d).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_peak: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out_peak: Option<u64>,
    /// Discarded packets a second (bucket average for 24 h and 7 d).
    pub in_discards: f64,
    pub out_discards: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_discards_peak: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out_discards_peak: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub avg: u64,
    pub peak: u64,
}

/// Average and peak of discards (packets/s).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PpsSummary {
    pub avg: f64,
    pub peak: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    /// Start and end of the range (unix ms).
    pub from: i64,
    pub to: i64,
    /// Seconds between two points (the live interval for 1 h).
    pub step_secs: u32,
    pub points: Vec<Point>,
    #[serde(rename = "in")]
    pub inn: Summary,
    pub out: Summary,
    pub in_discards: PpsSummary,
    pub out_discards: PpsSummary,
}

/// Packets a second, to the hundredth.
fn pps(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn peak(v: impl Iterator<Item = f64>) -> f64 {
    pps(v.fold(0.0, f64::max))
}

/// The points of a port over a range ending at `now`.
pub fn history(p: Option<&PortRates>, range: Range, now: i64, live_secs: u32) -> History {
    let from = now - range.span();
    let mut h = History { from, to: now, ..History::default() };
    let Some(p) = p else {
        h.step_secs = match range {
            Range::Hour => live_secs,
            Range::Day => (MID_STEP / 1000) as u32,
            Range::Week => (COARSE_STEP / 1000) as u32,
        };
        return h;
    };
    match range {
        Range::Hour => {
            h.step_secs = live_secs;
            let pts: Vec<_> = p.fine.iter().filter(|x| x.0 >= from).collect();
            for x in &pts {
                h.points.push(Point { at: x.0, inn: x.1, out: x.2, in_discards: pps(x.3), out_discards: pps(x.4), ..Point::default() });
            }
            let n = pts.len().max(1) as f64;
            h.inn = Summary { avg: (pts.iter().map(|x| x.1 as f64).sum::<f64>() / n).round() as u64, peak: pts.iter().map(|x| x.1).max().unwrap_or(0) };
            h.out = Summary { avg: (pts.iter().map(|x| x.2 as f64).sum::<f64>() / n).round() as u64, peak: pts.iter().map(|x| x.2).max().unwrap_or(0) };
            h.in_discards = PpsSummary { avg: pps(pts.iter().map(|x| x.3).sum::<f64>() / n), peak: peak(pts.iter().map(|x| x.3)) };
            h.out_discards = PpsSummary { avg: pps(pts.iter().map(|x| x.4).sum::<f64>() / n), peak: peak(pts.iter().map(|x| x.4)) };
        }
        Range::Day | Range::Week => {
            let (list, step) = if range == Range::Day { (&p.mid, MID_STEP) } else { (&p.coarse, COARSE_STEP) };
            h.step_secs = (step / 1000) as u32;
            let bs: Vec<&Bucket> = list.iter().filter(|b| b.0 + step > from).collect();
            for b in &bs {
                let (i, o) = b.avg();
                let (di, d_o) = b.avg_discards();
                h.points.push(Point {
                    at: b.0,
                    inn: i,
                    out: o,
                    in_peak: Some(b.4),
                    out_peak: Some(b.5),
                    in_discards: pps(di),
                    out_discards: pps(d_o),
                    in_discards_peak: Some(pps(b.8)),
                    out_discards_peak: Some(pps(b.9)),
                });
            }
            let n = f64::from(bs.iter().map(|b| b.1).sum::<u32>().max(1));
            h.inn = Summary { avg: (bs.iter().map(|b| b.2).sum::<f64>() / n).round() as u64, peak: bs.iter().map(|b| b.4).max().unwrap_or(0) };
            h.out = Summary { avg: (bs.iter().map(|b| b.3).sum::<f64>() / n).round() as u64, peak: bs.iter().map(|b| b.5).max().unwrap_or(0) };
            h.in_discards = PpsSummary { avg: pps(bs.iter().map(|b| b.6).sum::<f64>() / n), peak: peak(bs.iter().map(|b| b.8)) };
            h.out_discards = PpsSummary { avg: pps(bs.iter().map(|b| b.7).sum::<f64>() / n), peak: peak(bs.iter().map(|b| b.9)) };
        }
    }
    h
}

impl Net {
    fn rates_dir(&self) -> PathBuf {
        self.dir.join("rates")
    }

    fn rates_file(&self, id: &str) -> PathBuf {
        self.rates_dir().join(format!("{id}.json"))
    }

    /// Runs `f` on the history of a switch, read from its file the first time.
    pub fn with_rates<T>(&self, id: &str, f: impl FnOnce(&mut SwitchRates) -> T) -> T {
        let mut map = self.rates.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !map.contains_key(id) {
            let loaded = match read_json::<SwitchRates>(&self.rates_file(id)) {
                Ok(v) => v.unwrap_or_default(),
                Err(e) => {
                    tracing::warn!("cannot read the rates of switch {id}: {e}");
                    SwitchRates::default()
                }
            };
            map.insert(id.to_string(), loaded);
        }
        f(map.get_mut(id).expect("just inserted"))
    }

    /// New rates of an online switch's ports.
    pub fn record_rates(&self, id: &str, samples: &[Sample]) {
        if samples.is_empty() {
            return;
        }
        let at = now_ms();
        self.with_rates(id, |r| {
            for s in samples {
                r.ports.entry(s.n).or_default().add(at, s);
            }
            r.dirty = true;
        });
    }

    /// Forgets the history of a deleted switch.
    pub fn forget_rates(&self, id: &str) {
        self.rates.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(id);
        let _ = std::fs::remove_file(self.rates_file(id));
    }

    /// Writes the histories changed since the last time.
    pub fn save_rates(&self) {
        let mut map = self.rates.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for (id, r) in map.iter_mut().filter(|(_, r)| r.dirty) {
            r.dirty = false;
            match serde_json::to_vec(&*r) {
                Ok(b) => {
                    let _ = self.writer.send((self.rates_file(id), b));
                }
                Err(e) => tracing::error!("cannot serialize the rates of switch {id}: {e}"),
            }
        }
    }
}

/// Writes the histories every 5 minutes.
pub fn spawn_saver(net: Arc<Net>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(300));
        tick.tick().await;
        loop {
            tick.tick().await;
            net.save_rates();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(i: u64, o: u64) -> Sample {
        Sample { n: 1, in_bps: i, out_bps: o, ..Sample::default() }
    }

    #[test]
    fn buckets_and_ranges() {
        let mut p = PortRates::default();
        let t0 = 1_700_000_000_000 - 1_700_000_000_000 % COARSE_STEP;
        // Every 10 s for 2 hours: 1 Mb/s in, a 9 Mb/s peak at 30 min; out twice the in.
        for k in 0..720 {
            let at = t0 + k * 10_000;
            let i = if k == 180 { 9_000_000 } else { 1_000_000 };
            p.add(at, &sample(i, 2 * i));
        }
        let now = t0 + 719 * 10_000;
        // The fine list keeps one hour.
        assert_eq!(p.fine.len(), 361);
        assert_eq!(p.mid.len(), 24);
        assert_eq!(p.coarse.len(), 4);
        let h = history(Some(&p), Range::Hour, now, 10);
        assert_eq!((h.step_secs, h.points.len(), h.inn.peak, h.inn.avg), (10, 361, 1_000_000, 1_000_000));
        let d = history(Some(&p), Range::Day, now, 10);
        assert_eq!((d.step_secs, d.points.len(), d.inn.peak, d.out.peak), (300, 24, 9_000_000, 18_000_000));
        // Average weighted by samples: (719 × 1 + 9) / 720 Mb/s.
        assert_eq!(d.inn.avg, ((719.0 * 1e6 + 9e6) / 720.0_f64).round() as u64);
        assert_eq!(d.points[6].in_peak, Some(9_000_000));
        let w = history(Some(&p), Range::Week, now, 10);
        assert_eq!((w.step_secs, w.points.len()), (1800, 4));
        // Old buckets go.
        p.add(now + 8 * 24 * HOUR, &sample(0, 0));
        assert_eq!((p.fine.len(), p.mid.len(), p.coarse.len()), (1, 1, 1));
        assert!(history(None, Range::Day, now, 10).points.is_empty());
        assert_eq!(Range::parse("7d"), Some(Range::Week));
        assert_eq!(Range::parse("2h"), None);
    }

    #[test]
    fn saved_and_read_back() {
        let mut r = SwitchRates::default();
        r.ports.entry(3).or_default().add(1_000_000, &Sample { n: 3, in_bps: 5, out_bps: 6, in_discards: 0.5, out_discards: 12.25 });
        r.dirty = true;
        let back: SwitchRates = serde_json::from_slice(&serde_json::to_vec(&r).unwrap()).unwrap();
        assert_eq!(back.ports, r.ports);
        assert!(!back.dirty);
        // A history saved before the discards reads back with none.
        let old: SwitchRates = serde_json::from_str(r#"{"ports":{"3":{"fine":[[1000000,5,6]],"mid":[[900000,1,5.0,6.0,5,6]],"coarse":[[0,1,5.0,6.0,5,6]]}}}"#).unwrap();
        let p = &old.ports[&3];
        assert_eq!(p.fine[0], Fine(1_000_000, 5, 6, 0.0, 0.0));
        assert_eq!(p.mid[0], Bucket(900_000, 1, 5.0, 6.0, 5, 6, 0.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn discards_in_buckets() {
        let mut p = PortRates::default();
        let t0 = 1_700_000_000_000 - 1_700_000_000_000 % COARSE_STEP;
        // Every 10 s for 5 min: no discards, but 30 packets/s out for one reading.
        for k in 0..30 {
            let out = if k == 7 { 30.0 } else { 0.0 };
            p.add(t0 + k * 10_000, &Sample { n: 1, in_bps: 1_000, out_bps: 2_000, in_discards: 0.0, out_discards: out });
        }
        let now = t0 + 29 * 10_000;
        let h = history(Some(&p), Range::Hour, now, 10);
        assert_eq!((h.points[7].out_discards, h.out_discards.peak, h.out_discards.avg, h.in_discards.peak), (30.0, 30.0, 1.0, 0.0));
        let d = history(Some(&p), Range::Day, now, 10);
        assert_eq!(d.points.len(), 1);
        assert_eq!((d.points[0].out_discards, d.points[0].out_discards_peak), (1.0, Some(30.0)));
        let json = serde_json::to_value(&d).unwrap();
        assert_eq!(json["points"][0]["outDiscardsPeak"], 30.0);
        assert_eq!(json["outDiscards"]["avg"], 1.0);
    }
}
