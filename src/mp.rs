//! Multiplayer: the other players, through a relay (see `relay/` and the
//! protocol in `mp_proto.rs`). A background thread says Hello, then sends
//! where our aircraft is about fifteen times a second and takes in where the
//! others are. Each of them is drawn a little in the past (`DELAY`), between
//! two reports, so that they move smoothly whatever the network does; past
//! the last report they fly on along their velocity for a moment.
//!
//! Nothing is sent unless multiplayer is switched on; what is sent is the
//! aircraft's state and the name the player chose.
use crate::airmodels::{self as am, TYPES};
use crate::mp_proto::{self as proto, Msg, PlayerState};
use crate::airfield::{Light, L_GREEN, L_RED, L_STROBE, L_WHITE};
use crate::render::Inst;
use glam::{DQuat, DVec3, Vec3};
use std::collections::{HashMap, VecDeque};
use std::net::{ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// how far in the past the others are shown (s)
const DELAY: f64 = 0.15;
/// how often our state goes out
const SEND_EVERY: Duration = Duration::from_millis(66);
/// a player not heard of for this long is gone (s)
const STALE: f64 = 3.0;
/// no word from the relay for this long: say Hello again
const LOST: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Connecting,
    Connected,
    Error(String),
}

struct Peer {
    name: String,
    /// reports, oldest first: when they came (s since `epoch`), what they said
    snaps: VecDeque<(f64, PlayerState)>,
}

struct Shared {
    epoch: Instant,
    ours: Mutex<Option<PlayerState>>,
    peers: Mutex<HashMap<u32, Peer>>,
    status: Mutex<Status>,
    stop: AtomicBool,
}

/// Another player, where they are now (as best we know).
#[derive(Clone, Debug)]
pub struct Remote {
    pub id: u32,
    pub name: String,
    pub aircraft: u8,
    pub pos: DVec3,
    pub rot: DQuat,
    pub vel: DVec3,
    pub gear: f32,
    pub flags: u8,
}

pub struct Mp {
    pub server: String,
    pub name: String,
    shared: Arc<Shared>,
}

impl Mp {
    /// Connects to the relay at `server` ("host" or "host:port") as `name`.
    pub fn start(server: &str, name: &str) -> Self {
        let shared = Arc::new(Shared { epoch: Instant::now(), ours: Mutex::new(None), peers: Mutex::new(HashMap::new()), status: Mutex::new(Status::Connecting), stop: AtomicBool::new(false) });
        let (sv, nm, sh) = (server.trim().to_string(), proto::clean_name(name), shared.clone());
        std::thread::Builder::new()
            .name("multiplayer".into())
            .spawn(move || {
                if let Err(e) = run(&sv, &nm, &sh) {
                    log::warn!("multiplayer: {e}");
                    *sh.status.lock().unwrap() = Status::Error(e);
                }
            })
            .ok();
        Mp { server: server.trim().to_string(), name: proto::clean_name(name), shared }
    }

    /// What to send next: our aircraft now.
    pub fn set_state(&self, s: PlayerState) {
        *self.shared.ours.lock().unwrap() = Some(s);
    }

    pub fn status(&self) -> Status {
        self.shared.status.lock().unwrap().clone()
    }

    /// The other players, interpolated to now (minus `DELAY`).
    pub fn players(&self) -> Vec<Remote> {
        let t = self.shared.epoch.elapsed().as_secs_f64() - DELAY;
        let peers = self.shared.peers.lock().unwrap();
        let mut out: Vec<Remote> = peers.iter().filter_map(|(id, p)| at(*id, p, t)).collect();
        out.sort_by_key(|r| r.id);
        out
    }
}

impl Drop for Mp {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

fn v3(a: [f64; 3]) -> DVec3 {
    DVec3::new(a[0], a[1], a[2])
}
fn quat(a: [f32; 4]) -> DQuat {
    DQuat::from_xyzw(a[0] as f64, a[1] as f64, a[2] as f64, a[3] as f64).normalize()
}

/// A player at time `t`: between the two reports around it, or carried on
/// from the last one (for at most a second).
fn at(id: u32, p: &Peer, t: f64) -> Option<Remote> {
    let (first, last) = (p.snaps.front()?, p.snaps.back()?);
    let (pos, rot, s) = if t <= first.0 {
        (v3(first.1.pos), quat(first.1.rot), first.1)
    } else if t >= last.0 {
        let dt = (t - last.0).min(1.0);
        (v3(last.1.pos) + v3(last.1.vel.map(|x| x as f64)) * dt, quat(last.1.rot), last.1)
    } else {
        let k = p.snaps.iter().position(|(ts, _)| *ts > t).unwrap_or(p.snaps.len() - 1).max(1);
        let ((t0, a), (t1, b)) = (p.snaps[k - 1], p.snaps[k]);
        let u = ((t - t0) / (t1 - t0).max(1e-6)).clamp(0.0, 1.0);
        (v3(a.pos).lerp(v3(b.pos), u), quat(a.rot).slerp(quat(b.rot), u), if u < 0.5 { a } else { b })
    };
    Some(Remote { id, name: p.name.clone(), aircraft: s.aircraft, pos, rot, vel: v3(s.vel.map(|x| x as f64)), gear: s.gear as f32 / 255.0, flags: s.flags })
}

/// The network side, until told to stop.
fn run(server: &str, name: &str, sh: &Shared) -> Result<(), String> {
    let with_port = if server.rsplit_once(':').is_some_and(|(_, p)| p.parse::<u16>().is_ok()) { server.to_string() } else { format!("{server}:{}", proto::DEFAULT_PORT) };
    let addr = with_port.to_socket_addrs().map_err(|e| format!("{server}: {e}"))?.next().ok_or_else(|| format!("{server}: no address"))?;
    let sock = UdpSocket::bind(if addr.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" }).map_err(|e| e.to_string())?;
    sock.connect(addr).map_err(|e| e.to_string())?;
    sock.set_read_timeout(Some(Duration::from_millis(15))).map_err(|e| e.to_string())?;
    let mut cookie: Option<u64> = None;
    let (mut last_send, mut last_hello, mut last_heard) = (Instant::now() - Duration::from_secs(10), Instant::now() - Duration::from_secs(10), Instant::now());
    let mut seq = 0u32;
    let mut buf = [0u8; 2048];
    while !sh.stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        match cookie {
            None if now.duration_since(last_hello) > Duration::from_secs(1) => {
                last_hello = now;
                let _ = sock.send(&Msg::Hello { name: name.to_string() }.encode());
            }
            Some(c) if now.duration_since(last_send) >= SEND_EVERY => {
                last_send = now;
                if let Some(mut s) = *sh.ours.lock().unwrap() {
                    seq = seq.wrapping_add(1);
                    s.seq = seq;
                    let _ = sock.send(&Msg::State { cookie: c, state: s }.encode());
                }
            }
            _ => {}
        }
        if cookie.is_some() && now.duration_since(last_heard) > LOST {
            // The relay went quiet (restarted, or the network dropped): start over.
            cookie = None;
            *sh.status.lock().unwrap() = Status::Connecting;
        }
        let n = match sock.recv(&mut buf) {
            Ok(n) => n,
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => continue,
            // (An ICMP "port unreachable" from a relay that is down.)
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        match Msg::decode(&buf[..n]) {
            Some(Msg::Welcome { cookie: c, .. }) => {
                cookie = Some(c);
                last_heard = now;
            }
            Some(Msg::Peers { peers }) => {
                last_heard = now;
                *sh.status.lock().unwrap() = Status::Connected;
                let t = sh.epoch.elapsed().as_secs_f64();
                let mut map = sh.peers.lock().unwrap();
                for (id, name, s) in peers {
                    let p = map.entry(id).or_insert_with(|| Peer { name: name.clone(), snaps: VecDeque::new() });
                    p.name = name;
                    // (Reordered packets: keep the reports in the order they were sent.)
                    if p.snaps.back().is_none_or(|(_, b)| s.seq > b.seq) {
                        p.snaps.push_back((t, s));
                    }
                    while p.snaps.len() > 8 {
                        p.snaps.pop_front();
                    }
                }
                map.retain(|_, p| p.snaps.back().is_some_and(|(ts, _)| t - ts < STALE));
            }
            _ => {}
        }
    }
    if let Some(c) = cookie {
        let _ = sock.send(&Msg::Bye { cookie: c }.encode());
    }
    Ok(())
}

/// Which traffic model stands for each of our aircraft (airmodels::TYPES:
/// light single, business jet, regional, narrow-body, wide-body).
pub fn size_of(aircraft: u8) -> u8 {
    match aircraft {
        0 | 8 => 0,
        1 => 4,
        6 => 2,
        _ => 1,
    }
}

/// How far below the body frame's origin the wheels touch, for an aircraft id
/// (the traffic models stand on their wheels at their origin).
fn wheels_below(aircraft: u8) -> f64 {
    crate::aircraft::ALL.iter().find(|s| s.id == aircraft as u32).map_or(1.2, |s| -s.gear.iter().map(|l| l.at.y - l.wheel_r).fold(0.0, f64::min))
}

/// The other players as traffic models with their lights (as live.rs draws
/// the real traffic).
pub fn instances(players: &[Remote], cam: DVec3, out: &mut Vec<(u32, Inst)>, lights: &mut Vec<Light>) {
    for (i, p) in players.iter().enumerate() {
        if (p.pos - cam).length() > 40_000.0 {
            continue;
        }
        let (x, y, z) = ((p.rot * DVec3::X).as_vec3(), (p.rot * DVec3::Y).as_vec3(), (p.rot * DVec3::Z).as_vec3());
        let base = p.pos + p.rot * DVec3::new(0.0, -wheels_below(p.aircraft), 0.0);
        let o = (base - cam).as_vec3();
        let inst = |tint: [f32; 3]| Inst { m: [[x.x, y.x, z.x, o.x], [x.y, y.y, z.y, o.y], [x.z, y.z, z.z, o.z]], tint: [tint[0], tint[1], tint[2], 0.0], ext: [0.0; 4] };
        let size = size_of(p.aircraft);
        let m = am::M_BODY + 3 * size as u32;
        // Each player gets a fin colour of their own.
        out.push((m, inst(crate::traffic::BODY[0])));
        out.push((m + 1, inst(crate::traffic::FIN[p.id as usize % crate::traffic::FIN.len()])));
        if p.gear > 0.5 {
            out.push((m + 2, inst([0.5; 3])));
        }
        let ty = TYPES[size as usize];
        let at = |q: Vec3| -> [f32; 3] { (o + x * q.x + y * q.y - z * q.z).into() };
        let mut l = |q: Vec3, inten: f32, col: [u8; 3], fx: [u8; 4]| {
            if lights.len() < crate::render::MAX_DYN_LIGHTS {
                lights.push(Light { pos: at(q), inten, dir: [0.0; 3], spread: 0.0, col: [col[0], col[1], col[2], 0], fx });
            }
        };
        let steady = [0, 0, 255, 1];
        let ph = ((i * 53) % 255) as u8;
        if p.flags & proto::F_NAV != 0 {
            l(Vec3::new(-ty.tip.x, ty.tip.y, -ty.tip.z), 40.0, L_RED, steady);
            l(Vec3::new(ty.tip.x, ty.tip.y, -ty.tip.z), 40.0, L_GREEN, steady);
            l(Vec3::new(0.0, ty.tail.y * 0.35, -ty.tail.z), 25.0, L_WHITE, steady);
        }
        if p.flags & proto::F_BEACON != 0 {
            l(Vec3::new(0.0, ty.cy + ty.rf + 0.1, 0.0), 120.0, L_RED, [12, ph, 25, 1]);
        }
        if p.flags & proto::F_STROBE != 0 {
            for sx in [-1.0f32, 1.0] {
                l(Vec3::new(sx * ty.tip.x, ty.tip.y, -ty.tip.z), 1600.0, L_STROBE, [11, ph.wrapping_add(60), 7, 1]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(t: f64, x: f64, seq: u32) -> (f64, PlayerState) {
        (t, PlayerState { seq, pos: [6_372_000.0, x, 0.0], rot: [0.0, 0.0, 0.0, 1.0], vel: [0.0, 50.0, 0.0], ..Default::default() })
    }

    #[test]
    fn players_are_smoothed_between_reports_and_carried_on_after() {
        let p = Peer { name: "a".into(), snaps: VecDeque::from([snap(1.0, 0.0, 1), snap(1.1, 5.0, 2)]) };
        assert!((at(1, &p, 1.05).unwrap().pos.y - 2.5).abs() < 1e-6, "halfway");
        assert!((at(1, &p, 1.3).unwrap().pos.y - 15.0).abs() < 1e-6, "flown on at 50 m/s");
        assert!((at(1, &p, 9.0).unwrap().pos.y - 55.0).abs() < 1e-6, "for a second at most");
    }

    #[test]
    fn the_names_list_matches_the_aircraft() {
        for s in crate::aircraft::ALL.iter() {
            assert_eq!(proto::AIRCRAFT.get(s.id as usize).copied(), Some(s.name), "id {}", s.id);
        }
        assert_eq!(proto::PLANET_R, crate::geo::R);
    }

    #[test]
    fn two_players_meet_through_a_relay() {
        // A minimal relay in a thread: it answers Hello and passes each
        // State on to the other address as Peers (the real one is relay/).
        let relay = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = relay.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            let mut who: Vec<std::net::SocketAddr> = Vec::new();
            loop {
                let Ok((n, from)) = relay.recv_from(&mut buf) else { break };
                match Msg::decode(&buf[..n]) {
                    Some(Msg::Hello { .. }) => {
                        if !who.contains(&from) {
                            who.push(from);
                        }
                        let _ = relay.send_to(&Msg::Welcome { id: who.len() as u32, cookie: 7 }.encode(), from);
                    }
                    Some(Msg::State { state, .. }) => {
                        let me = who.iter().position(|a| *a == from).unwrap_or(0) as u32 + 1;
                        for a in who.iter().filter(|a| **a != from) {
                            let _ = relay.send_to(&Msg::Peers { peers: vec![(me, format!("p{me}"), state)] }.encode(), a);
                        }
                    }
                    _ => {}
                }
            }
        });
        let server = format!("127.0.0.1:{port}");
        let (a, b) = (Mp::start(&server, "A"), Mp::start(&server, "B"));
        let s = PlayerState { pos: [6_372_000.0, 0.0, 0.0], rot: [0.0, 0.0, 0.0, 1.0], aircraft: 8, ..Default::default() };
        a.set_state(s);
        b.set_state(s);
        let t0 = Instant::now();
        while (a.players().is_empty() || b.players().is_empty()) && t0.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(a.status(), Status::Connected);
        assert_eq!(b.players().len(), 1);
        assert_eq!(a.players()[0].aircraft, 8);
    }
}
