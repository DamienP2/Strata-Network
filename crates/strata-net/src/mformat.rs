//! What an SDP media says about its format (ST 2110-20 / -22 / -30 / -31 / -40,
//! ST 2022-6, AES67, MPEG-TS) and the bitrate it implies.

use serde::{Deserialize, Serialize};

/// RTP + UDP + IPv4 headers of one packet (bytes); Ethernet is not counted.
pub const PACKET_HEADERS: f64 = 40.0;
/// ST 2110-20: headers of ~1200-byte packets, payload header included (≈ 5 %).
pub const VIDEO_OVERHEAD: f64 = 1.05;
/// ST 2110-40 without a `b=` line: a few ancillary packets per frame.
pub const ANC_DEFAULT_BPS: f64 = 200_000.0;

/// The format of one media, from its SDP lines.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Format {
    /// "ST 2110-20", "ST 2110-22", "ST 2110-30", "ST 2110-31", "ST 2110-40", "ST 2022-6", "MPEG-TS".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standard: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Frames per second (`exactframerate`, `a=framerate`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_rate: Option<f64>,
    /// As written: "50", "60000/1001".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_rate_text: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub interlaced: bool,
    /// "YCbCr-4:2:2", "RGB"…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampling: Option<String>,
    /// Bits per sample.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colorimetry: Option<String>,
    /// Transfer characteristic (SDR, PQ, HLG…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tcs: Option<String>,
    /// RTP clock rate (also the audio sample rate).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock_rate: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_rate: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channels: Option<u16>,
    /// Packet time (milliseconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ptime_ms: Option<f64>,
    /// `a=ts-refclk` as written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ts_refclk: Option<String>,
    /// The PTP grandmaster of `a=ts-refclk:ptp=…:<gm>:<domain>` ("EC:46:70:FF:FE:00:1A:2B").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ptp_gm: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ptp_domain: Option<u8>,
    /// `a=mediaclk` as written ("direct=0", "sender").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_clock: Option<String>,
    /// `b=AS` (kb/s) or `b=TIAS` (b/s), in bits per second.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bandwidth_bps: Option<f64>,
}

fn fmtp_params(v: &str) -> Vec<(String, String)> {
    // "96 sampling=YCbCr-4:2:2; width=1920; …; interlace"
    let rest = v.split_once(' ').map(|x| x.1).unwrap_or("");
    rest.split(';')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (k.trim().to_ascii_lowercase(), v.trim().to_string()),
            None => (p.to_ascii_lowercase(), String::new()),
        })
        .collect()
}

/// "60000/1001" or "50" or "29.97".
pub fn parse_rate(s: &str) -> Option<f64> {
    let r = match s.split_once('/') {
        Some((a, b)) => a.trim().parse::<f64>().ok()? / b.trim().parse::<f64>().ok().filter(|d| *d > 0.0)?,
        None => s.trim().parse().ok()?,
    };
    (r > 0.0 && r < 1000.0).then_some(r)
}

/// "IEEE1588-2008:EC-46-70-FF-FE-00-1A-2B:127" → (gm, domain).
fn ptp_ref(v: &str) -> (Option<String>, Option<u8>) {
    let Some(p) = v.trim().strip_prefix("ptp=") else { return (None, None) };
    let parts: Vec<&str> = p.split(':').collect();
    let gm = parts.get(1).map(|g| g.replace('-', ":").to_ascii_uppercase()).filter(|g| g.len() == 23 && g.split(':').all(|b| b.len() == 2 && u8::from_str_radix(b, 16).is_ok()));
    let domain = parts.get(2).and_then(|d| d.trim().parse().ok());
    (gm, domain)
}

impl Format {
    /// From the lines (`k`, `v`) of the session then of the media (media lines win),
    /// and the RTP encoding of the media.
    pub fn parse(session: &[(String, String)], media: &[(String, String)], encoding: Option<&str>) -> Format {
        let mut f = Format::default();
        for (k, v) in session.iter().chain(media.iter()) {
            match k.as_str() {
                "b" => {
                    if let Some(x) = v.strip_prefix("AS:") {
                        f.bandwidth_bps = x.trim().parse::<f64>().ok().map(|k| k * 1000.0);
                    } else if let Some(x) = v.strip_prefix("TIAS:") {
                        f.bandwidth_bps = x.trim().parse().ok();
                    }
                }
                "a" => {
                    let (name, val) = v.split_once(':').unwrap_or((v.as_str(), ""));
                    match name {
                        "rtpmap" => {
                            // "96 raw/90000", "97 L24/48000/8"
                            let enc = val.split_whitespace().nth(1).unwrap_or("");
                            let mut it = enc.split('/').skip(1);
                            f.clock_rate = it.next().and_then(|x| x.parse().ok());
                            if let Some(ch) = it.next().and_then(|x| x.parse().ok()) {
                                f.channels = Some(ch);
                            }
                        }
                        "fmtp" => {
                            for (pk, pv) in fmtp_params(val) {
                                match pk.as_str() {
                                    "width" => f.width = pv.parse().ok(),
                                    "height" => f.height = pv.parse().ok(),
                                    "exactframerate" | "framerate" => {
                                        f.frame_rate = parse_rate(&pv);
                                        f.frame_rate_text = Some(pv);
                                    }
                                    "sampling" => f.sampling = Some(pv),
                                    "depth" => f.depth = pv.trim_end_matches(|c: char| !c.is_ascii_digit()).parse().ok(),
                                    "colorimetry" => f.colorimetry = Some(pv),
                                    "tcs" => f.tcs = Some(pv),
                                    "interlace" => f.interlaced = true,
                                    _ => {}
                                }
                            }
                        }
                        "framerate" if f.frame_rate.is_none() => {
                            f.frame_rate = parse_rate(val);
                            f.frame_rate_text = Some(val.trim().to_string());
                        }
                        "ptime" => f.ptime_ms = val.trim().parse().ok().filter(|p: &f64| *p > 0.0),
                        "ts-refclk" => {
                            let (gm, d) = ptp_ref(val);
                            if gm.is_some() {
                                f.ptp_gm = gm;
                            }
                            if d.is_some() {
                                f.ptp_domain = d;
                            }
                            f.ts_refclk = Some(val.trim().to_string());
                        }
                        "mediaclk" => f.media_clock = Some(val.trim().to_string()),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        let enc = encoding.unwrap_or("");
        f.standard = match enc.to_ascii_lowercase().as_str() {
            "raw" => Some("ST 2110-20"),
            "jxsv" => Some("ST 2110-22"),
            "l16" | "l24" | "l32" => Some("ST 2110-30"),
            "am824" => Some("ST 2110-31"),
            "smpte291" => Some("ST 2110-40"),
            "smpte2022-6" => Some("ST 2022-6"),
            "mp2t" => Some("MPEG-TS"),
            _ => None,
        }
        .map(str::to_string);
        if matches!(f.standard.as_deref(), Some("ST 2110-30" | "ST 2110-31")) {
            f.audio_rate = f.clock_rate;
            if f.channels.is_none() {
                f.channels = Some(1);
            }
            if f.depth.is_none() {
                f.depth = audio_depth(enc);
            }
        }
        f
    }

    /// Bits per second the format implies, RTP / UDP / IP headers included; `None` when unknown.
    pub fn est_bps(&self) -> Option<f64> {
        let v = match self.standard.as_deref()? {
            "ST 2110-20" => {
                let samples = samples_per_pixel(self.sampling.as_deref()?)?;
                let px = f64::from(self.width?) * f64::from(self.height?);
                px * samples * f64::from(self.depth?) * self.frame_rate? * VIDEO_OVERHEAD
            }
            "ST 2110-30" | "ST 2110-31" => {
                let bytes = self.depth.map(|d| f64::from(d) / 8.0).unwrap_or(3.0);
                let rate = f64::from(self.audio_rate?);
                let ch = f64::from(self.channels.unwrap_or(1));
                let ptime = self.ptime_ms.unwrap_or(1.0) / 1000.0;
                let pps = 1.0 / ptime;
                (rate * ch * bytes + PACKET_HEADERS * pps) * 8.0
            }
            "ST 2110-40" => self.bandwidth_bps.unwrap_or(ANC_DEFAULT_BPS),
            "ST 2022-6" => {
                // 3G-SDI for 1080p above 30 frames per second (and beyond 1080 lines), else HD-SDI.
                let three_g = self.height.unwrap_or(0) > 1080 || (self.height.unwrap_or(0) >= 1080 && !self.interlaced && self.frame_rate.unwrap_or(0.0) > 30.0);
                let sdi = if three_g { 2.970e9 } else { 1.485e9 };
                // 1376-byte media payload + RTP 12 + HBRMT 8 + UDP 8 + IP 20.
                sdi * (1376.0 + 48.0) / 1376.0
            }
            _ => self.bandwidth_bps?,
        };
        (v > 0.0).then_some(v)
    }
}

/// Samples per pixel of an ST 2110-20 sampling (4:2:2 → 2).
pub fn samples_per_pixel(sampling: &str) -> Option<f64> {
    let s = sampling.to_ascii_uppercase();
    Some(if s.ends_with("4:4:4") || s == "RGB" || s == "XYZ" {
        3.0
    } else if s.ends_with("4:2:2") {
        2.0
    } else if s.ends_with("4:2:0") || s.ends_with("4:1:1") {
        1.5
    } else if s == "KEY" {
        1.0
    } else {
        return None;
    })
}

/// The audio sample size of an RTP encoding (L16 → 16 bits, L24 → 24, AM824 → 32).
pub fn audio_depth(encoding: &str) -> Option<u8> {
    match encoding.to_ascii_uppercase().as_str() {
        "L16" => Some(16),
        "L24" => Some(24),
        "L32" | "AM824" => Some(32),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<(String, String)> {
        s.lines().filter_map(|l| l.trim().split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn video(w: u32, h: u32, rate: &str, sampling: &str, depth: u8) -> Format {
        let m = lines(&format!(
            "a=rtpmap:96 raw/90000\na=fmtp:96 sampling={sampling}; width={w}; height={h}; exactframerate={rate}; depth={depth}; TCS=SDR; colorimetry=BT709; PM=2110GPM; SSN=ST2110-20:2017; TP=2110TPN\n\
             a=ts-refclk:ptp=IEEE1588-2008:EC-46-70-FF-FE-00-1A-2B:127\na=mediaclk:direct=0"
        ));
        Format::parse(&[], &m, Some("raw"))
    }

    #[test]
    fn st2110_20_rates() {
        let f = video(1920, 1080, "50", "YCbCr-4:2:2", 10);
        assert_eq!((f.width, f.height, f.depth, f.sampling.as_deref()), (Some(1920), Some(1080), Some(10), Some("YCbCr-4:2:2")));
        assert_eq!((f.ptp_gm.as_deref(), f.ptp_domain, f.media_clock.as_deref()), (Some("EC:46:70:FF:FE:00:1A:2B"), Some(127), Some("direct=0")));
        assert_eq!(f.standard.as_deref(), Some("ST 2110-20"));
        // Active video: 1920 × 1080 × 20 bits × 50 = 2.07 Gb/s, + 5 % headers.
        let b = f.est_bps().unwrap();
        assert!((b / 1e9 - 2.177).abs() < 0.01, "{b}");
        let f = video(1920, 1080, "60000/1001", "YCbCr-4:2:2", 10);
        assert_eq!(f.frame_rate_text.as_deref(), Some("60000/1001"));
        let b = f.est_bps().unwrap();
        assert!((b / 1e9 - 2.610).abs() < 0.01, "{b}");
        // 2160p50 4:2:2 10 bits ≈ 8.7 Gb/s; 720p50 4:4:4 12 bits.
        assert!((video(3840, 2160, "50", "YCbCr-4:2:2", 10).est_bps().unwrap() / 1e9 - 8.709).abs() < 0.01);
        assert!((video(1280, 720, "50", "RGB", 12).est_bps().unwrap() / 1e9 - 1.742).abs() < 0.01);
        // Interlaced: the frame rate is per frame, the height the whole frame.
        let mut f = video(1920, 1080, "25", "YCbCr-4:2:2", 10);
        f.interlaced = true;
        assert!((f.est_bps().unwrap() / 1e9 - 1.089).abs() < 0.01);
        assert_eq!(video(1920, 1080, "50", "Unknown", 10).est_bps(), None);
    }

    #[test]
    fn st2110_30_rates() {
        let m = lines("a=rtpmap:97 L24/48000/8\na=ptime:1\na=ts-refclk:ptp=IEEE1588-2008:traceable");
        let f = Format::parse(&[], &m, Some("L24"));
        assert_eq!((f.audio_rate, f.channels, f.ptime_ms, f.ptp_gm.as_deref()), (Some(48_000), Some(8), Some(1.0), None));
        // 48 000 × 8 × 3 bytes + 40 bytes × 1000 packets = 9.536 Mb/s.
        assert!((f.est_bps().unwrap() - 9_536_000.0).abs() < 1.0);
        // 2 channels at 125 µs: the headers weigh more.
        let f = Format::parse(&[], &lines("a=rtpmap:97 L24/48000/2\na=ptime:0.125"), Some("L24"));
        assert!((f.est_bps().unwrap() - (48_000.0 * 2.0 * 3.0 + 40.0 * 8000.0) * 8.0).abs() < 1.0);
        // AM824 (-31): 4 bytes a sample.
        let f = Format::parse(&[], &lines("a=rtpmap:98 AM824/48000/2"), Some("AM824"));
        assert_eq!(f.standard.as_deref(), Some("ST 2110-31"));
        assert!((f.est_bps().unwrap() - (48_000.0 * 2.0 * 4.0 + 40_000.0) * 8.0).abs() < 1.0);
    }

    #[test]
    fn other_rates() {
        let f = Format::parse(&[], &lines("a=rtpmap:100 smpte291/90000"), Some("smpte291"));
        assert_eq!(f.est_bps(), Some(ANC_DEFAULT_BPS));
        let f = Format::parse(&lines("b=AS:12000"), &lines("a=rtpmap:33 MP2T/90000"), Some("MP2T"));
        assert_eq!(f.est_bps(), Some(12_000_000.0));
        let f = Format::parse(&[], &lines("b=TIAS:150000000\na=rtpmap:96 jxsv/90000"), Some("jxsv"));
        assert_eq!((f.standard.as_deref(), f.est_bps()), (Some("ST 2110-22"), Some(150e6)));
        let f = Format::parse(&[], &lines("a=rtpmap:98 SMPTE2022-6/27000000"), Some("SMPTE2022-6"));
        assert!((f.est_bps().unwrap() / 1e9 - 1.537).abs() < 0.01);
        let f = Format::parse(&[], &lines("a=rtpmap:98 SMPTE2022-6/27000000\na=fmtp:98 width=1920; height=1080; exactframerate=50"), Some("SMPTE2022-6"));
        assert!((f.est_bps().unwrap() / 1e9 - 3.074).abs() < 0.01);
        assert_eq!(Format::parse(&[], &[], Some("MP2T")).est_bps(), None);
    }
}
