//! SNMP v3 with user-based security (RFC 3414, 3826, 7860), read-only:
//! authentication HMAC-MD5-96, HMAC-SHA-96 or HMAC-SHA-256-192; privacy
//! DES-CBC or AES-128-CFB. Engine discovery, key localization, time window.

use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;

use crate::snmp::{int, parse_pdu, pdu_bytes, read_int, tlv, Oid, Reader, Value};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Auth {
    #[default]
    None,
    Md5,
    Sha,
    Sha256,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Privacy {
    #[default]
    None,
    Des,
    Aes128,
}

/// A v3 user: name, authentication and privacy.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct V3 {
    pub user: String,
    pub auth: Auth,
    pub auth_password: String,
    pub privacy: Privacy,
    pub priv_password: String,
}

impl V3 {
    pub fn check(&self) -> Result<(), String> {
        if self.user.trim().is_empty() {
            return Err("the snmp user cannot be empty".into());
        }
        if self.auth != Auth::None && self.auth_password.len() < 8 {
            return Err("the authentication password has at least 8 characters".into());
        }
        if self.privacy != Privacy::None {
            if self.auth == Auth::None {
                return Err("privacy needs authentication".into());
            }
            if self.priv_password.len() < 8 {
                return Err("the privacy password has at least 8 characters".into());
            }
        }
        Ok(())
    }
}

fn hash(a: Auth, data: &[u8]) -> Vec<u8> {
    use sha2::Digest as _;
    match a {
        Auth::Md5 => md5::Md5::digest(data).to_vec(),
        Auth::Sha => sha1::Sha1::digest(data).to_vec(),
        _ => sha2::Sha256::digest(data).to_vec(),
    }
}

/// RFC 3414 A.2: the password stretched over 1 MB, hashed (Ku).
pub fn password_key(a: Auth, password: &str) -> Vec<u8> {
    use sha2::Digest as _;
    let pw = password.as_bytes();
    let mut buf = Vec::with_capacity(1_048_576);
    let mut i = 0;
    while buf.len() < 1_048_576 {
        buf.push(pw[i % pw.len()]);
        i += 1;
    }
    match a {
        Auth::Md5 => md5::Md5::digest(&buf).to_vec(),
        Auth::Sha => sha1::Sha1::digest(&buf).to_vec(),
        _ => sha2::Sha256::digest(&buf).to_vec(),
    }
}

/// Kul = H(Ku || engineID || Ku).
pub fn localize(a: Auth, ku: &[u8], engine: &[u8]) -> Vec<u8> {
    hash(a, &[ku, engine, ku].concat())
}

/// Length of the authentication parameters.
fn mac_len(a: Auth) -> usize {
    match a {
        Auth::None => 0,
        Auth::Md5 | Auth::Sha => 12,
        Auth::Sha256 => 24,
    }
}

fn hmac(a: Auth, key: &[u8], msg: &[u8]) -> Vec<u8> {
    let out = match a {
        Auth::Md5 => {
            let mut m = Hmac::<md5::Md5>::new_from_slice(key).expect("hmac key");
            m.update(msg);
            m.finalize().into_bytes().to_vec()
        }
        Auth::Sha => {
            let mut m = Hmac::<sha1::Sha1>::new_from_slice(key).expect("hmac key");
            m.update(msg);
            m.finalize().into_bytes().to_vec()
        }
        _ => {
            let mut m = Hmac::<sha2::Sha256>::new_from_slice(key).expect("hmac key");
            m.update(msg);
            m.finalize().into_bytes().to_vec()
        }
    };
    out[..mac_len(a)].to_vec()
}

struct Engine {
    id: Vec<u8>,
    boots: u32,
    time: u32,
    at: Instant,
    auth_key: Vec<u8>,
    priv_key: Vec<u8>,
}

impl Engine {
    fn now(&self) -> u32 {
        self.time.saturating_add(self.at.elapsed().as_secs() as u32)
    }
}

pub struct V3Session {
    user: V3,
    engine: Mutex<Engine>,
    salt: AtomicU64,
}

fn err(m: &str) -> io::Error {
    io::Error::other(m.to_string())
}

/// What a USM report means.
fn report_text(oid: &Oid) -> &'static str {
    match oid.to_string().as_str() {
        "1.3.6.1.6.3.15.1.1.1.0" => "the switch refuses this security level (authentication / privacy)",
        "1.3.6.1.6.3.15.1.1.2.0" => "not in time window",
        "1.3.6.1.6.3.15.1.1.3.0" => "unknown snmp user",
        "1.3.6.1.6.3.15.1.1.4.0" => "unknown engine",
        "1.3.6.1.6.3.15.1.1.5.0" => "wrong authentication password or protocol",
        "1.3.6.1.6.3.15.1.1.6.0" => "wrong privacy password or protocol",
        _ => "the switch answered with a report",
    }
}

struct Incoming {
    flags: u8,
    engine: Vec<u8>,
    boots: u32,
    time: u32,
    auth_params: Vec<u8>,
    priv_params: Vec<u8>,
    data: Vec<u8>,
    /// Where the authentication parameters are in the message (to check them).
    auth_at: Option<usize>,
}

fn parse_message(msg: &[u8]) -> Option<Incoming> {
    let (tag, body) = Reader { b: msg, pos: 0 }.next()?;
    if tag != 0x30 {
        return None;
    }
    let outer_header = msg.len() - body.len();
    let mut r = Reader { b: body, pos: 0 };
    if read_int(r.next()?.1) != 3 {
        return None;
    }
    let (_, header) = r.next()?;
    let mut h = Reader { b: header, pos: 0 };
    h.next()?; // msgID
    h.next()?; // max size
    let flags = *h.next()?.1.first()?;
    let sec_start = r.pos;
    let (_, sec) = r.next()?;
    let sec_body_start = outer_header + sec_start + (r.pos - sec_start - sec.len());
    let (_, secseq) = Reader { b: sec, pos: 0 }.next()?;
    let secseq_start = sec.len() - secseq.len();
    let mut s = Reader { b: secseq, pos: 0 };
    let engine = s.next()?.1.to_vec();
    let boots = read_int(s.next()?.1) as u32;
    let time = read_int(s.next()?.1) as u32;
    s.next()?; // user
    let (_, auth_params) = s.next()?;
    let auth_at = (!auth_params.is_empty()).then(|| sec_body_start + secseq_start + s.pos - auth_params.len());
    let priv_params = s.next()?.1.to_vec();
    let rest = &body[r.pos..];
    Some(Incoming { flags, engine, boots, time, auth_params: auth_params.to_vec(), priv_params, data: rest.to_vec(), auth_at })
}

impl V3Session {
    /// Asks the agent for its engine id, boots and time, then localizes the keys.
    pub async fn discover(sock: &UdpSocket, agent: SocketAddr, user: &V3, timeout: Duration) -> io::Result<V3Session> {
        user.check().map_err(|e| err(&e))?;
        let id = rand::random::<i32>() & 0x7FFF_FFFF;
        let scoped = tlv(0x30, &[tlv(0x04, b""), tlv(0x04, b""), pdu_bytes(0xA0, id, &[], 0)].concat());
        let sec = tlv(0x30, &[tlv(0x04, b""), int(0), int(0), tlv(0x04, b""), tlv(0x04, b""), tlv(0x04, b"")].concat());
        let msg = tlv(0x30, &[int(3), header(id, 0x04), tlv(0x04, &sec), scoped].concat());
        let mut buf = vec![0u8; 65_535];
        for _ in 0..3 {
            sock.send_to(&msg, agent).await?;
            let deadline = tokio::time::Instant::now() + timeout;
            while let Ok(r) = tokio::time::timeout_at(deadline, sock.recv_from(&mut buf)).await {
                let (n, from) = r?;
                if from != agent {
                    continue;
                }
                let Some(m) = parse_message(&buf[..n]) else { continue };
                if m.engine.is_empty() {
                    continue;
                }
                let ku_a = if user.auth == Auth::None { Vec::new() } else { password_key(user.auth, &user.auth_password) };
                let auth_key = if user.auth == Auth::None { Vec::new() } else { localize(user.auth, &ku_a, &m.engine) };
                let priv_key = if user.privacy == Privacy::None { Vec::new() } else { localize(user.auth, &password_key(user.auth, &user.priv_password), &m.engine) };
                return Ok(V3Session {
                    user: user.clone(),
                    engine: Mutex::new(Engine { id: m.engine, boots: m.boots, time: m.time, at: Instant::now(), auth_key, priv_key }),
                    salt: AtomicU64::new(rand::random()),
                });
            }
        }
        Err(io::Error::new(io::ErrorKind::TimedOut, "no snmp v3 answer (address, port or firewall?)"))
    }

    fn flags(&self) -> u8 {
        let mut f = 0x04; // reportable
        if self.user.auth != Auth::None {
            f |= 0x01;
        }
        if self.user.privacy != Privacy::None {
            f |= 0x02;
        }
        f
    }

    /// One request message, authenticated and encrypted as the user asks.
    fn build(&self, msg_id: i32, pdu: &[u8]) -> Vec<u8> {
        let e = self.engine.lock().unwrap();
        let (boots, time) = (e.boots, e.now());
        let scoped = tlv(0x30, &[tlv(0x04, &e.id), tlv(0x04, b""), pdu.to_vec()].concat());
        let (data, priv_params) = match self.user.privacy {
            Privacy::None => (scoped, Vec::new()),
            Privacy::Des => {
                use cbc::cipher::{BlockEncryptMut, KeyIvInit};
                let salt = [boots.to_be_bytes(), (self.salt.fetch_add(1, Ordering::Relaxed) as u32).to_be_bytes()].concat();
                let iv: Vec<u8> = e.priv_key[8..16].iter().zip(&salt).map(|(a, b)| a ^ b).collect();
                let mut plain = scoped;
                while !plain.len().is_multiple_of(8) {
                    plain.push(0);
                }
                let len = plain.len();
                let c = cbc::Encryptor::<des::Des>::new_from_slices(&e.priv_key[..8], &iv).expect("des key");
                let out = c.encrypt_padded_mut::<cbc::cipher::block_padding::NoPadding>(&mut plain, len).expect("des").to_vec();
                (tlv(0x04, &out), salt)
            }
            Privacy::Aes128 => {
                use cfb_mode::cipher::{AsyncStreamCipher, KeyIvInit};
                let salt = self.salt.fetch_add(1, Ordering::Relaxed).to_be_bytes().to_vec();
                let iv = [boots.to_be_bytes().as_slice(), &time.to_be_bytes(), &salt].concat();
                let mut buf = scoped;
                cfb_mode::Encryptor::<aes::Aes128>::new_from_slices(&e.priv_key[..16], &iv).expect("aes key").encrypt(&mut buf);
                (tlv(0x04, &buf), salt)
            }
        };
        let mac = vec![0u8; mac_len(self.user.auth)];
        let sec = tlv(
            0x30,
            &[tlv(0x04, &e.id), int(i64::from(boots)), int(i64::from(time)), tlv(0x04, self.user.user.as_bytes()), tlv(0x04, &mac), tlv(0x04, &priv_params)].concat(),
        );
        let mut msg = tlv(0x30, &[int(3), header(msg_id, self.flags()), tlv(0x04, &sec), data].concat());
        if self.user.auth != Auth::None {
            let digest = hmac(self.user.auth, &e.auth_key, &msg);
            // The zeroed MAC sits right before the privacy parameters at the end of the security parameters.
            let sec_at = msg.windows(sec.len()).position(|w| w == sec.as_slice()).expect("security parameters");
            let mac_at = sec_at + sec.len() - priv_params.len() - 2 - mac.len();
            msg[mac_at..mac_at + mac.len()].copy_from_slice(&digest);
        }
        msg
    }

    /// Decrypts and checks an answer; returns its PDU tag and body.
    fn open_answer(&self, raw: &[u8], m: &Incoming) -> io::Result<(u8, Vec<u8>)> {
        let e = self.engine.lock().unwrap();
        if m.flags & 0x01 != 0 {
            if let Some(at) = m.auth_at {
                let mut copy = raw.to_vec();
                copy[at..at + m.auth_params.len()].fill(0);
                if hmac(self.user.auth, &e.auth_key, &copy) != m.auth_params {
                    return Err(err("the answer's authentication is wrong"));
                }
            }
        }
        let scoped = if m.flags & 0x02 != 0 {
            let (_, enc) = Reader { b: &m.data, pos: 0 }.next().ok_or_else(|| err("bad encrypted answer"))?;
            let mut buf = enc.to_vec();
            match self.user.privacy {
                Privacy::Des => {
                    use cbc::cipher::{BlockDecryptMut, KeyIvInit};
                    let iv: Vec<u8> = e.priv_key[8..16].iter().zip(&m.priv_params).map(|(a, b)| a ^ b).collect();
                    let d = cbc::Decryptor::<des::Des>::new_from_slices(&e.priv_key[..8], &iv).map_err(|_| err("des key"))?;
                    let len = buf.len() - buf.len() % 8;
                    d.decrypt_padded_mut::<cbc::cipher::block_padding::NoPadding>(&mut buf[..len]).map_err(|_| err("cannot decrypt the answer"))?;
                }
                Privacy::Aes128 => {
                    use cfb_mode::cipher::{AsyncStreamCipher, KeyIvInit};
                    let iv = [m.boots.to_be_bytes().as_slice(), &m.time.to_be_bytes(), &m.priv_params].concat();
                    cfb_mode::Decryptor::<aes::Aes128>::new_from_slices(&e.priv_key[..16], &iv).map_err(|_| err("aes key"))?.decrypt(&mut buf);
                }
                Privacy::None => return Err(err("the answer is encrypted")),
            }
            buf
        } else {
            m.data.clone()
        };
        let (_, sc) = Reader { b: &scoped, pos: 0 }.next().ok_or_else(|| err("bad answer (wrong privacy password?)"))?;
        let mut r = Reader { b: sc, pos: 0 };
        r.next().ok_or_else(|| err("bad answer"))?;
        r.next().ok_or_else(|| err("bad answer"))?;
        let (tag, pdu) = r.next().ok_or_else(|| err("bad answer"))?;
        Ok((tag, pdu.to_vec()))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn ask(&self, sock: &UdpSocket, agent: SocketAddr, pdu: u8, oids: &[Oid], max_rep: i64, timeout: Duration, retries: u32) -> io::Result<Vec<(Oid, Value)>> {
        let mut resynced = false;
        let mut buf = vec![0u8; 65_535];
        let mut attempt = 0;
        'send: while attempt <= retries {
            attempt += 1;
            let id = rand::random::<i32>() & 0x7FFF_FFFF;
            let msg = self.build(id, &pdu_bytes(pdu, id, oids, max_rep));
            sock.send_to(&msg, agent).await?;
            let deadline = tokio::time::Instant::now() + timeout;
            while let Ok(r) = tokio::time::timeout_at(deadline, sock.recv_from(&mut buf)).await {
                let (n, from) = r?;
                if from != agent {
                    continue;
                }
                let raw = &buf[..n];
                let Some(m) = parse_message(raw) else { continue };
                let (tag, body) = match self.open_answer(raw, &m) {
                    Ok(x) => x,
                    Err(e) if m.flags & 0x01 == 0 => {
                        // Unauthenticated reports (unknown user, wrong digest…) still say why.
                        let _ = e;
                        let plain = (Reader { b: &m.data, pos: 0 }).next().and_then(|(_, sc)| {
                            let mut r = Reader { b: sc, pos: 0 };
                            r.next()?;
                            r.next()?;
                            let (t, p) = r.next()?;
                            Some((t, p.to_vec()))
                        });
                        match plain {
                            Some(x) => x,
                            None => continue,
                        }
                    }
                    Err(e) => return Err(e),
                };
                let Some((rid, status, binds)) = parse_pdu(&body) else { continue };
                if tag == 0xA8 {
                    let what = binds.first().map(|(o, _)| report_text(o)).unwrap_or("the switch answered with a report");
                    if what == "not in time window" && !resynced {
                        resynced = true;
                        let mut e = self.engine.lock().unwrap();
                        e.boots = m.boots;
                        e.time = m.time;
                        e.at = Instant::now();
                        attempt -= 1;
                        continue 'send;
                    }
                    return Err(err(what));
                }
                if rid != id {
                    continue;
                }
                if status != 0 {
                    return Err(err(&format!("the agent answered with error {status}")));
                }
                return Ok(binds);
            }
        }
        Err(io::Error::new(io::ErrorKind::TimedOut, "no snmp v3 answer"))
    }
}

fn header(id: i32, flags: u8) -> Vec<u8> {
    tlv(0x30, &[int(i64::from(id)), int(65_507), tlv(0x04, &[flags]), int(3)].concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace().map(|b| u8::from_str_radix(b, 16).unwrap()).collect()
    }

    /// RFC 3414 A.3.1 and A.3.2: "maplesyrup", engine 00…02.
    #[test]
    fn rfc_key_vectors() {
        let engine = hex("00 00 00 00 00 00 00 00 00 00 00 02");
        let ku = password_key(Auth::Md5, "maplesyrup");
        assert_eq!(ku, hex("9f af 32 83 88 4e 92 83 4e bc 98 47 d8 ed d9 63"));
        assert_eq!(localize(Auth::Md5, &ku, &engine), hex("52 6f 5e ed 9f cc e2 6f 89 64 c2 93 07 87 d8 2b"));
        let ku = password_key(Auth::Sha, "maplesyrup");
        assert_eq!(ku, hex("9f b5 cc 03 81 49 7b 37 93 52 89 39 ff 78 8d 5d 79 14 52 11"));
        assert_eq!(localize(Auth::Sha, &ku, &engine), hex("66 95 fe bc 92 88 e3 62 82 23 5f c7 15 1f 12 84 97 b3 8f 3f"));
    }

    #[test]
    fn users_are_checked() {
        assert!(V3 { user: "strata".into(), auth: Auth::Sha, auth_password: "longenough".into(), privacy: Privacy::Aes128, priv_password: "longenough".into() }.check().is_ok());
        assert!(V3 { user: "".into(), ..V3::default() }.check().is_err());
        assert!(V3 { user: "x".into(), auth: Auth::Sha, auth_password: "short".into(), ..V3::default() }.check().is_err());
        assert!(V3 { user: "x".into(), privacy: Privacy::Des, priv_password: "longenough".into(), ..V3::default() }.check().is_err());
    }
}
