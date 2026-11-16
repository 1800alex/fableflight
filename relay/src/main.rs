//! FableFlight's multiplayer relay. Players send it where their aircraft is
//! about fifteen times a second; it sends each of them the players near them
//! ten times a second. That is all: no physics, no accounts, nothing kept
//! on disk. It sees positions and the names the players chose.
//!
//! UDP, see `src/mp_proto.rs` (shared with the game). Settings, from the
//! environment:
//! - `FFMP_PORT` (7480): the UDP port;
//! - `FFMP_MAX_PLAYERS` (64);
//! - `FFMP_RANGE_KM` (150): how far apart players still see each other;
//! - `FFMP_HTTP_PORT` (7481, 0 for none): a small status page over HTTP
//!   (`/`, and `/status.json` that it reads every two seconds): how many
//!   are flying, and each one's name, aircraft, altitude and speed. Meant to
//!   sit behind a reverse proxy (Caddy, nginx) for HTTPS.
//!
//! - `FFMP_NAME`: this relay's name, as the other relays and the status page
//!   show it (say its host name);
//! - `FFMP_PEERS` (none): the other relays of the network, `host[:port]`
//!   separated by commas, and `FFMP_PEER_SECRET`, the network's shared
//!   secret (16 characters at least, the same on every relay). See below.
//!
//! Federation: relays that list each other pass their own players to each
//! other ten times a second, so a player on one sees the players near them on
//! all. A relay only ever passes on its own players (no loops); every
//! datagram between relays is signed with the shared secret (SipHash), dated
//! and numbered, and comes from a listed relay's address, or it is dropped
//! (no forged, replayed or stray players); a relay never answers one.
//!
//! Safe to leave open on the internet: nothing is sent to an address until
//! it has echoed a cookie only that address could have received (so a forged
//! sender cannot make it send anything anywhere, and the one answer to a
//! Hello is smaller than the Hello); packets are size- and value-checked;
//! each address is held to a packet rate; idle players are forgotten.

#[path = "../../src/mp_proto.rs"]
#[allow(dead_code)]
mod proto;

use proto::{Msg, PlayerState};
use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, Hasher};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Player {
    id: u32,
    name: String,
    state: Option<PlayerState>,
    last: Instant,
    /// packet budget (token bucket): what is left, refilled at RATE a second
    tokens: f64,
}

struct Config {
    max_players: usize,
    range_m: f64,
    /// this relay's name, and the network's secret (None: no federation)
    name: String,
    secret: Option<String>,
}

/// Another relay of the network.
struct PeerRelay {
    /// as configured ("host:port"), and what it resolves to
    host: String,
    addrs: Vec<SocketAddr>,
    /// what it calls itself, its id this run, the last datagram number, when last heard
    name: String,
    relay: u32,
    last_seq: u32,
    heard: Option<Instant>,
}

/// A player on another relay, as last reported.
struct Remote {
    name: String,
    state: PlayerState,
    last: Instant,
}

/// how long a player from another relay stays without news
const REMOTE_IDLE: Duration = Duration::from_secs(3);
/// how far a relay's clock may be from ours (s)
const CLOCK_SLACK: u64 = 30;
/// local player ids stay below this; players from the n-th relay are n << 24 | theirs
const LOCAL_IDS: u32 = 1 << 24;

/// Packets a second an address may send (it sends ~15), and the burst.
const RATE: f64 = 40.0;
const BURST: f64 = 80.0;
/// forgotten after this long without a word
const IDLE: Duration = Duration::from_secs(10);
/// names of players who said Hello but have not sent a State yet
const PENDING_MAX: usize = 256;
/// most players listed to one player, nearest first
const SHOWN_MAX: usize = 30;

struct Relay {
    cfg: Config,
    /// keys the cookies: a cookie is the address hashed with them, so the
    /// relay needs no memory of who said Hello
    secret: RandomState,
    players: HashMap<SocketAddr, Player>,
    pending: HashMap<SocketAddr, (String, Instant)>,
    next_id: u32,
    /// federation: the other relays, the players they report (by relay index
    /// and their id), our id for this run and the number of our next datagram
    peers: Vec<PeerRelay>,
    remote: HashMap<(usize, u32), Remote>,
    me: u32,
    fed_seq: u32,
}

impl Relay {
    fn new(cfg: Config) -> Self {
        let secret = RandomState::new();
        let me = { let mut h = secret.build_hasher(); std::process::id().hash(&mut h); Instant::now().hash(&mut h); h.finish() as u32 };
        Relay { cfg, secret, players: HashMap::new(), pending: HashMap::new(), next_id: 1, peers: Vec::new(), remote: HashMap::new(), me, fed_seq: 0 }
    }

    /// Looks the other relays' names up again (their addresses may change).
    fn resolve_peers(&mut self) {
        for p in &mut self.peers {
            match p.host.to_socket_addrs() {
                Ok(a) => p.addrs = a.filter(|a| a.is_ipv4()).collect(),
                Err(e) => eprintln!("peer {}: {e}", p.host),
            }
        }
    }

    /// A datagram from another relay: its players, if it is one of ours, the
    /// signature holds, and it is new.
    #[allow(clippy::too_many_arguments)]
    fn federate(&mut self, addr: SocketAddr, data: &[u8], relay: u32, name: String, ts: u64, seq: u32, players: Vec<(u32, String, PlayerState)>, tag: u64, now: Instant, unix: u64) {
        let Some(secret) = &self.cfg.secret else { return };
        // (Our own datagrams, come back somehow: not other players.)
        if relay == self.me {
            return;
        }
        // (The relay at that very address; else at that IP, should a NAT have changed the port.)
        let Some(i) = self.peers.iter().position(|p| p.addrs.contains(&addr)).or_else(|| self.peers.iter().position(|p| p.addrs.iter().any(|a| a.ip() == addr.ip()))) else { return };
        if data.len() < 8 || proto::fed_tag(secret, &data[..data.len() - 8]) != tag || ts.abs_diff(unix) > CLOCK_SLACK {
            return;
        }
        let p = &mut self.peers[i];
        // (The same run of that relay: only newer datagrams. A restarted one has a new id.)
        if p.relay == relay && seq <= p.last_seq && p.last_seq - seq < u32::MAX / 2 {
            return;
        }
        if p.heard.is_none() || p.relay != relay {
            println!("~ relay {} ({}) joined", name, p.host);
        }
        (p.relay, p.last_seq, p.name, p.heard) = (relay, seq, name, Some(now));
        for (id, name, state) in players {
            self.remote.insert((i, id), Remote { name, state, last: now });
        }
    }

    /// Our own players for every other relay, signed, ten to a datagram (an
    /// empty one when nobody flies here, to say we are up).
    fn fed_packets(&mut self, unix: u64) -> Vec<(SocketAddr, Vec<u8>)> {
        let Some(secret) = self.cfg.secret.clone() else { return Vec::new() };
        if self.peers.is_empty() {
            return Vec::new();
        }
        let mine: Vec<(u32, String, PlayerState)> = self.players.values().filter_map(|p| Some((p.id, p.name.clone(), p.state?))).collect();
        let chunks: Vec<&[(u32, String, PlayerState)]> = if mine.is_empty() { vec![&[]] } else { mine.chunks(proto::PEERS_PER_PACKET).collect() };
        let mut out = Vec::new();
        for c in chunks {
            self.fed_seq = self.fed_seq.wrapping_add(1);
            let mut d = Msg::Fed { relay: self.me, relay_name: self.cfg.name.clone(), ts: unix, seq: self.fed_seq, players: c.to_vec(), tag: 0 }.encode();
            let n = d.len() - 8;
            let tag = proto::fed_tag(&secret, &d[..n]);
            d[n..].copy_from_slice(&tag.to_le_bytes());
            for p in &self.peers {
                if let Some(a) = p.addrs.first() {
                    out.push((*a, d.clone()));
                }
            }
        }
        out
    }

    fn cookie(&self, addr: &SocketAddr) -> u64 {
        let mut h = self.secret.build_hasher();
        addr.hash(&mut h);
        h.finish() | 1
    }

    /// One datagram from `addr`; what to send back to it, if anything.
    fn receive(&mut self, addr: SocketAddr, data: &[u8], now: Instant, unix: u64) -> Option<Vec<u8>> {
        if let Some(p) = self.players.get_mut(&addr) {
            let dt = now.saturating_duration_since(p.last).as_secs_f64();
            p.tokens = (p.tokens + dt * RATE).min(BURST);
            if p.tokens < 1.0 {
                return None;
            }
            p.tokens -= 1.0;
        }
        match Msg::decode(data)? {
            Msg::Hello { name } => {
                // Remember the name for a moment (bounded: under a flood, the oldest go).
                if self.pending.len() >= PENDING_MAX {
                    if let Some(old) = self.pending.iter().min_by_key(|(_, v)| v.1).map(|(k, _)| *k) {
                        self.pending.remove(&old);
                    }
                }
                self.pending.insert(addr, (name, now));
                let id = self.players.get(&addr).map_or(0, |p| p.id);
                Some(Msg::Welcome { id, cookie: self.cookie(&addr) }.encode())
            }
            Msg::State { cookie, state } => {
                if cookie != self.cookie(&addr) {
                    return None;
                }
                if !self.players.contains_key(&addr) {
                    if self.players.len() >= self.cfg.max_players {
                        return None;
                    }
                    let name = self.pending.remove(&addr).map(|p| p.0).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("pilot {}", self.next_id));
                    let id = self.next_id;
                    self.next_id = (self.next_id + 1) % LOCAL_IDS;
                    self.next_id = self.next_id.max(1);
                    println!("+ {name} (#{id}), {} flying", self.players.len() + 1);
                    self.players.insert(addr, Player { id, name, state: None, last: now, tokens: BURST });
                }
                let p = self.players.get_mut(&addr).unwrap();
                // (A late, reordered packet does not take the player back in time.)
                if p.state.is_none_or(|s| state.seq > s.seq || s.seq.wrapping_sub(state.seq) > u32::MAX / 2) {
                    p.state = Some(state);
                }
                p.last = now;
                None
            }
            Msg::Bye { cookie } => {
                if cookie == self.cookie(&addr) {
                    if let Some(p) = self.players.remove(&addr) {
                        println!("- {} (#{}), {} flying", p.name, p.id, self.players.len());
                    }
                }
                None
            }
            Msg::Fed { relay, relay_name, ts, seq, players, tag } => {
                self.federate(addr, data, relay, relay_name, ts, seq, players, tag, now, unix);
                None
            }
            Msg::Welcome { .. } | Msg::Peers { .. } => None,
        }
    }

    /// The Peers packets for every player: the others within range, nearest first.
    fn broadcast(&self) -> Vec<(SocketAddr, Vec<u8>)> {
        let mut out = Vec::new();
        for (addr, me) in &self.players {
            let Some(mine) = me.state else { continue };
            let dist = |s: &PlayerState| ((s.pos[0] - mine.pos[0]).powi(2) + (s.pos[1] - mine.pos[1]).powi(2) + (s.pos[2] - mine.pos[2]).powi(2)).sqrt();
            let mut near: Vec<(f64, u32, &str, PlayerState)> = self
                .players
                .iter()
                .filter(|(a, _)| *a != addr)
                .filter_map(|(_, p)| {
                    let s = p.state?;
                    let d = dist(&s);
                    (d < self.cfg.range_m).then_some((d, p.id, p.name.as_str(), s))
                })
                .chain(self.remote.iter().filter_map(|((i, id), r)| {
                    let d = dist(&r.state);
                    (d < self.cfg.range_m).then_some((d, ((*i as u32 + 1) << 24) | (id & (LOCAL_IDS - 1)), r.name.as_str(), r.state))
                }))
                .collect();
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            near.truncate(SHOWN_MAX);
            // (Nobody near: an empty list still tells the client the relay is there.)
            if near.is_empty() {
                out.push((*addr, Msg::Peers { peers: Vec::new() }.encode()));
            }
            for chunk in near.chunks(proto::PEERS_PER_PACKET) {
                let peers = chunk.iter().map(|(_, id, n, s)| (*id, n.to_string(), *s)).collect();
                out.push((*addr, Msg::Peers { peers }.encode()));
            }
        }
        out
    }

    /// What the status page shows: the players, as JSON.
    fn status_json(&self, started: Instant, packets: u64) -> String {
        let mut list: Vec<(&str, &str, PlayerState)> = self.players.values().filter_map(|p| Some((self.cfg.name.as_str(), p.name.as_str(), p.state?))).collect();
        list.extend(self.remote.iter().map(|((i, _), r)| (self.peers[*i].name.as_str(), r.name.as_str(), r.state)));
        list.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
        let here = self.players.values().filter(|p| p.state.is_some()).count();
        let players: Vec<String> = list
            .iter()
            .map(|(relay, name, s)| {
                let r = (s.pos[0] * s.pos[0] + s.pos[1] * s.pos[1] + s.pos[2] * s.pos[2]).sqrt();
                let v = s.vel.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
                let ac = proto::AIRCRAFT.get(s.aircraft as usize).copied().unwrap_or("?");
                format!(
                    "{{\"relay\":{},\"name\":{},\"aircraft\":{},\"alt_ft\":{:.0},\"speed_kt\":{:.0},\"on_ground\":{}}}",
                    json_str(relay),
                    json_str(name),
                    json_str(ac),
                    (r - proto::PLANET_R) * 3.28084,
                    v * 1.94384,
                    s.flags & proto::F_GROUND != 0
                )
            })
            .collect();
        // The network: this relay first, then the others, each up or not and
        // how many fly on it (a relay passes on all its players, near or not).
        let mut network = vec![format!("{{\"name\":{},\"up\":true,\"self\":true,\"flying\":{}}}", json_str(&self.cfg.name), here)];
        network.extend(self.peers.iter().enumerate().map(|(i, p)| {
            let up = p.heard.is_some_and(|t| t.elapsed() < Duration::from_secs(5));
            let name = if p.name.is_empty() { p.host.as_str() } else { p.name.as_str() };
            let n = self.remote.keys().filter(|(j, _)| *j == i).count();
            format!("{{\"name\":{},\"up\":{},\"self\":false,\"flying\":{}}}", json_str(name), up, if up { n } else { 0 })
        }));
        format!(
            "{{\"version\":{},\"relay\":{},\"uptime_s\":{},\"max_players\":{},\"range_km\":{:.0},\"packets\":{},\"flying\":{},\"here\":{},\"network\":[{}],\"players\":[{}]}}",
            json_str(env!("CARGO_PKG_VERSION")),
            json_str(&self.cfg.name),
            started.elapsed().as_secs(),
            self.cfg.max_players,
            self.cfg.range_m / 1000.0,
            packets,
            list.len(),
            here,
            network.join(","),
            players.join(",")
        )
    }

    /// Forgets the players gone quiet, and old Hellos.
    fn expire(&mut self, now: Instant) {
        let gone: Vec<SocketAddr> = self.players.iter().filter(|(_, p)| now.saturating_duration_since(p.last) > IDLE).map(|(a, _)| *a).collect();
        for a in gone {
            if let Some(p) = self.players.remove(&a) {
                println!("- {} (#{}) timed out, {} flying", p.name, p.id, self.players.len());
            }
        }
        self.pending.retain(|_, (_, t)| now.saturating_duration_since(*t) < IDLE);
        self.remote.retain(|_, r| now.saturating_duration_since(r.last) < REMOTE_IDLE);
    }
}

/// A JSON string literal (quotes, backslashes and control characters escaped).
fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// The status page: plain HTML that asks for /status.json every two seconds
/// and fills the table in with text (never as markup).
const PAGE: &str = include_str!("status.html");

/// The HTTP side: GET / and GET /status.json, nothing else. Each connection
/// gets two seconds and 2 KB of request.
fn serve_http(listener: TcpListener, status: Arc<Mutex<String>>) {
    for conn in listener.incoming() {
        let Ok(mut c) = conn else { continue };
        let status = status.clone();
        std::thread::spawn(move || {
            let _ = answer(&mut c, &status);
        });
    }
}
fn answer(c: &mut TcpStream, status: &Mutex<String>) -> std::io::Result<()> {
    c.set_read_timeout(Some(Duration::from_secs(2)))?;
    c.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut buf = [0u8; 2048];
    let n = c.read(&mut buf)?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let path = req.lines().next().and_then(|l| l.strip_prefix("GET ")).and_then(|l| l.split_whitespace().next()).unwrap_or("");
    let (code, kind, body) = match path.split('?').next().unwrap_or("") {
        "/" | "/index.html" => ("200 OK", "text/html; charset=utf-8", PAGE.to_string()),
        "/status.json" => ("200 OK", "application/json", status.lock().map(|s| s.clone()).unwrap_or_default()),
        _ => ("404 Not Found", "text/plain; charset=utf-8", "not found\n".to_string()),
    };
    let head = format!("HTTP/1.1 {code}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'self'; style-src 'unsafe-inline'; script-src 'unsafe-inline'\r\nConnection: close\r\n\r\n", body.len());
    c.write_all(head.as_bytes())?;
    c.write_all(body.as_bytes())
}

fn env<T: std::str::FromStr>(k: &str, d: T) -> T {
    std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn main() -> std::io::Result<()> {
    let port: u16 = env("FFMP_PORT", proto::DEFAULT_PORT);
    let name = proto::clean_name(&env("FFMP_NAME", "relay".to_string()));
    let peers: Vec<String> = env("FFMP_PEERS", String::new()).split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).map(|s| if s.rsplit_once(':').is_some_and(|(_, p)| p.parse::<u16>().is_ok()) { s.to_string() } else { format!("{s}:{}", proto::DEFAULT_PORT) }).collect();
    let mut secret = std::env::var("FFMP_PEER_SECRET").ok().filter(|s| !s.is_empty());
    if !peers.is_empty() && secret.as_ref().is_none_or(|s| s.len() < 16) {
        eprintln!("FFMP_PEERS needs FFMP_PEER_SECRET (16 characters at least, the same on every relay): no federation");
        secret = None;
    }
    let cfg = Config { max_players: env("FFMP_MAX_PLAYERS", 64usize).clamp(1, 10_000), range_m: env("FFMP_RANGE_KM", 150.0f64).clamp(1.0, 20_000.0) * 1000.0, name, secret };
    let sock = UdpSocket::bind(("0.0.0.0", port))?;
    sock.set_read_timeout(Some(Duration::from_millis(20)))?;
    println!("fableflight-relay {} on udp/{port}: up to {} players, {} km", env!("CARGO_PKG_VERSION"), cfg.max_players, cfg.range_m / 1000.0);
    let federated = cfg.secret.is_some();
    let mut relay = Relay::new(cfg);
    if federated {
        relay.peers = peers.iter().map(|h| PeerRelay { host: h.clone(), addrs: Vec::new(), name: String::new(), relay: 0, last_seq: 0, heard: None }).collect();
        relay.resolve_peers();
        println!("federated with {} relay(s): {}", relay.peers.len(), peers.join(", "));
    }
    let started = Instant::now();
    let mut packets: u64 = 0;
    let status = Arc::new(Mutex::new(relay.status_json(started, 0)));
    let http_port: u16 = env("FFMP_HTTP_PORT", 7481);
    if http_port != 0 {
        let listener = TcpListener::bind(("0.0.0.0", http_port))?;
        println!("status page on http/{http_port}");
        let st = status.clone();
        std::thread::spawn(move || serve_http(listener, st));
    }
    let mut buf = [0u8; 2048];
    let (mut last_send, mut last_expire, mut last_resolve) = (Instant::now(), Instant::now(), Instant::now());
    loop {
        match sock.recv_from(&mut buf) {
            Ok((n, addr)) => {
                packets += 1;
                if let Some(reply) = relay.receive(addr, &buf[..n], Instant::now(), unix_now()) {
                    let _ = sock.send_to(&reply, addr);
                }
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => eprintln!("recv: {e}"),
        }
        let now = Instant::now();
        if now.duration_since(last_send) >= Duration::from_millis(100) {
            last_send = now;
            for (addr, p) in relay.broadcast() {
                let _ = sock.send_to(&p, addr);
            }
            for (addr, p) in relay.fed_packets(unix_now()) {
                let _ = sock.send_to(&p, addr);
            }
        }
        if now.duration_since(last_expire) >= Duration::from_secs(1) {
            last_expire = now;
            relay.expire(now);
            if federated && now.duration_since(last_resolve) >= Duration::from_secs(60) {
                last_resolve = now;
                relay.resolve_peers();
            }
            if let Ok(mut s) = status.lock() {
                *s = relay.status_json(started, packets);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(seq: u32, x: f64) -> PlayerState {
        PlayerState { seq, aircraft: 0, pos: [6_371_000.0 + 1000.0, x, 0.0], rot: [0.0, 0.0, 0.0, 1.0], ..Default::default() }
    }
    fn relay() -> Relay {
        Relay::new(Config { max_players: 3, range_m: 150_000.0, name: "here".into(), secret: None })
    }
    const SECRET: &str = "a secret for the tests";
    const T: u64 = 1_800_000_000;
    /// Relay `name` at 10.0.0.<ip>, federated with the relays at the other ips.
    fn fed_relay(name: &str, peers: &[u8]) -> Relay {
        let mut r = Relay::new(Config { max_players: 10, range_m: 150_000.0, name: name.into(), secret: Some(SECRET.into()) });
        r.peers = peers.iter().map(|ip| PeerRelay { host: format!("10.0.0.{ip}:7480"), addrs: vec![SocketAddr::from(([10, 0, 0, *ip], 7480))], name: String::new(), relay: 0, last_seq: 0, heard: None }).collect();
        r
    }
    fn addr(n: u16) -> SocketAddr {
        SocketAddr::from(([10, 0, 0, 1], 40_000 + n))
    }
    fn join(r: &mut Relay, a: SocketAddr, name: &str, x: f64, now: Instant) {
        let w = r.receive(a, &Msg::Hello { name: name.into() }.encode(), now, T).expect("welcome");
        let Some(Msg::Welcome { cookie, .. }) = Msg::decode(&w) else { panic!() };
        r.receive(a, &Msg::State { cookie, state: st(1, x) }.encode(), now, T);
    }
    fn peers_of(out: &[(SocketAddr, Vec<u8>)], a: SocketAddr) -> Vec<String> {
        out.iter().filter(|(x, _)| *x == a).flat_map(|(_, b)| match Msg::decode(b) { Some(Msg::Peers { peers }) => peers.into_iter().map(|p| p.1).collect(), _ => vec![] }).collect()
    }

    #[test]
    fn players_see_each_other_and_not_themselves() {
        let (mut r, now) = (relay(), Instant::now());
        join(&mut r, addr(1), "Dany", 0.0, now);
        join(&mut r, addr(2), "Roland", 5_000.0, now);
        let out = r.broadcast();
        assert_eq!(peers_of(&out, addr(1)), vec!["Roland"]);
        assert_eq!(peers_of(&out, addr(2)), vec!["Dany"]);
    }

    #[test]
    fn nothing_for_an_address_without_its_cookie() {
        let (mut r, now) = (relay(), Instant::now());
        // A State with a guessed cookie is ignored: no player, nothing sent.
        assert!(r.receive(addr(1), &Msg::State { cookie: 42, state: st(1, 0.0) }.encode(), now, T).is_none());
        assert!(r.players.is_empty() && r.broadcast().is_empty());
        // Someone else's cookie does not work from another address.
        let w = r.receive(addr(2), &Msg::Hello { name: "x".into() }.encode(), now, T).unwrap();
        let Some(Msg::Welcome { cookie, .. }) = Msg::decode(&w) else { panic!() };
        r.receive(addr(3), &Msg::State { cookie, state: st(1, 0.0) }.encode(), now, T);
        assert!(r.players.is_empty());
        // Junk gets no answer at all.
        assert!(r.receive(addr(4), b"GET / HTTP/1.1", now, T).is_none());
    }

    #[test]
    fn far_players_full_relays_and_quiet_players() {
        let (mut r, now) = (relay(), Instant::now());
        join(&mut r, addr(1), "a", 0.0, now);
        join(&mut r, addr(2), "b", 400_000.0, now);
        assert!(peers_of(&r.broadcast(), addr(1)).is_empty(), "too far to see");
        join(&mut r, addr(3), "c", 1000.0, now);
        join(&mut r, addr(4), "d", 2000.0, now);
        assert_eq!(r.players.len(), 3, "full");
        r.expire(now + Duration::from_secs(11));
        assert!(r.players.is_empty(), "quiet players are forgotten");
    }

    #[test]
    fn the_status_is_json_and_names_cannot_break_it() {
        let (mut r, now) = (relay(), Instant::now());
        join(&mut r, addr(1), "Dany \"<b>\\", 0.0, now);
        let j = r.status_json(now, 12);
        assert!(j.contains("\"flying\":1") && j.contains("\"aircraft\":\"Cessna 172RG\"") && j.contains("\"relay\":\"here\""), "{j}");
        assert!(j.contains(r#""name":"Dany \"<b>\\""#), "{j}");
        assert_eq!(json_str("a\nb"), "\"a\\u000ab\"");
    }

    #[test]
    fn federated_relays_see_each_others_players() {
        let now = Instant::now();
        // Relay A at 10.0.0.1 and B at 10.0.0.2, each with a player 5 km apart.
        let (mut a, mut b) = (fed_relay("A", &[2]), fed_relay("B", &[1]));
        join(&mut a, addr(1), "Dany", 0.0, now);
        join(&mut b, addr(2), "Roland", 5_000.0, now);
        let from = |ip: u8| SocketAddr::from(([10, 0, 0, ip], 7480));
        for (_, d) in a.fed_packets(T) {
            assert!(b.receive(from(1), &d, now, T).is_none(), "a relay never answers a relay");
        }
        for (_, d) in b.fed_packets(T) {
            b.receive(from(1), &d, now, T); // (its own datagrams coming back: ignored)
            a.receive(from(2), &d, now, T);
        }
        assert_eq!(peers_of(&b.broadcast(), addr(2)), vec!["Dany"]);
        assert!(peers_of(&a.broadcast(), addr(1)).contains(&"Roland".to_string()));
        // B's status counts A's player, and names A.
        let j = b.status_json(now, 0);
        assert!(j.contains("\"flying\":2") && j.contains("\"here\":1") && j.contains("\"relay\":\"A\""), "{j}");
        assert!(j.contains(r#"{"name":"B","up":true,"self":true,"flying":1}"#) && j.contains(r#"{"name":"A","up":true,"self":false,"flying":1}"#), "{j}");
        // A relay passes on only its own players: B sends nothing of Dany's.
        assert!(b.fed_packets(T).iter().all(|(_, d)| !String::from_utf8_lossy(d).contains("Dany")));
        // Quiet relays' players go.
        b.expire(now + Duration::from_secs(4));
        assert!(b.remote.is_empty());
    }

    #[test]
    fn forged_replayed_or_stray_relay_datagrams_are_dropped() {
        let now = Instant::now();
        let mut a = fed_relay("A", &[2]);
        join(&mut a, addr(1), "Dany", 0.0, now);
        let d = a.fed_packets(T).remove(0).1;
        let from = |ip: u8| SocketAddr::from(([10, 0, 0, ip], 7480));
        // From an address that is not one of B's relays.
        let mut b = fed_relay("B", &[1]);
        b.receive(from(9), &d, now, T);
        assert!(b.remote.is_empty(), "stray");
        // Signed with another secret.
        let mut wrong = fed_relay("B", &[1]);
        wrong.cfg.secret = Some("another secret, same length".into());
        wrong.receive(from(1), &d, now, T);
        assert!(wrong.remote.is_empty(), "forged");
        // One byte changed.
        let mut bad = d.clone();
        bad[30] ^= 1;
        b.receive(from(1), &bad, now, T);
        assert!(b.remote.is_empty(), "tampered");
        // Too old.
        b.receive(from(1), &d, now, T + 120);
        assert!(b.remote.is_empty(), "stale");
        // Good once; the same again is a replay.
        b.receive(from(1), &d, now, T);
        assert_eq!(b.remote.len(), 1);
        b.remote.clear();
        b.receive(from(1), &d, now, T);
        assert!(b.remote.is_empty(), "replayed");
    }

    #[test]
    fn a_flood_is_cut_to_the_rate() {
        let (mut r, now) = (relay(), Instant::now());
        join(&mut r, addr(1), "a", 0.0, now);
        let cookie = r.cookie(&addr(1));
        let mut taken = 0;
        for i in 0..1000 {
            let before = r.players[&addr(1)].state.unwrap().seq;
            r.receive(addr(1), &Msg::State { cookie, state: st(2 + i, 0.0) }.encode(), now, T);
            taken += (r.players[&addr(1)].state.unwrap().seq != before) as u32;
        }
        assert!(taken <= BURST as u32 + 1, "{taken}");
    }
}
