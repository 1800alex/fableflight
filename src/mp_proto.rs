//! The multiplayer protocol, shared word for word by the game (`mp.rs`) and
//! the relay (`relay/`, which includes this file): small binary UDP
//! datagrams, little-endian, each starting with `FFMP` and the version.
//!
//! - `Hello` (client → relay): "I want in, call me this". Padded to at least
//!   `HELLO_MIN` bytes, so the answer is never bigger than the question (a
//!   forged sender gets nothing amplified).
//! - `Welcome` (relay → client): your id and a cookie. Nothing else is ever
//!   sent to an address until it has sent that cookie back.
//! - `State` (client → relay, ~15 a second): where the aircraft is and how
//!   it is set up, with the cookie.
//! - `Peers` (relay → client, ~10 a second): the other players nearby.
//! - `Bye` (client → relay): leaving.
//!
//! Only std, no allocation beyond the Vec of a packet: the relay builds it
//! with nothing else.

pub const MAGIC: [u8; 4] = *b"FFMP";
pub const VERSION: u8 = 1;
pub const DEFAULT_PORT: u16 = 7480;
/// the longest name, in bytes (UTF-8, printable)
pub const NAME_MAX: usize = 24;
/// a Hello is at least this long (see the module notes)
pub const HELLO_MIN: usize = 48;
/// at most this many players in one Peers packet (it stays under ~1100 bytes)
pub const PEERS_PER_PACKET: usize = 10;

/// The aircraft by `PlayerState::aircraft` (`aircraft::Spec::id`), for the
/// relay's status page; ids past the end are shown as unknown. (The game's
/// tests check this list against its aircraft.)
pub const AIRCRAFT: &[&str] = &["Cessna 172RG", "Boeing 747-400", "F/A-18C Hornet", "F-4E Phantom II", "F-22A Raptor", "SpaceX Starship", "Concorde", "SR-71A Blackbird", "Robin DR 400/180"];
/// the planet's radius (m), as the game uses it (geo.rs)
#[allow(dead_code)] // (the relay's status page uses it)
pub const PLANET_R: f64 = 6_371_008.8;

const K_HELLO: u8 = 1;
const K_WELCOME: u8 = 2;
const K_STATE: u8 = 3;
const K_PEERS: u8 = 4;
const K_BYE: u8 = 5;

/// One player's aircraft at one instant. Positions are in the game's world
/// frame (metres from the planet's centre), the attitude a unit quaternion
/// (x, y, z, w) from the body frame to the world frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerState {
    /// counts up with each State the client sends
    pub seq: u32,
    /// which aircraft (`aircraft::Spec::id`)
    pub aircraft: u8,
    pub pos: [f64; 3],
    pub rot: [f32; 4],
    /// velocity (m/s, world frame)
    pub vel: [f32; 3],
    /// 0..255: flaps, gear, throttle, canopy
    pub flaps: u8,
    pub gear: u8,
    pub throttle: u8,
    pub canopy: u8,
    /// -127..127: elevator, aileron, rudder
    pub pitch: i8,
    pub roll: i8,
    pub yaw: i8,
    /// bits: 1 navigation lights, 2 strobes, 4 beacon, 8 landing lights,
    /// 16 on the ground, 32 crashed
    pub flags: u8,
}
#[allow(dead_code)] // (a State's size on the wire; the tests check it)
pub const STATE_LEN: usize = 4 + 1 + 24 + 16 + 12 + 4 + 3 + 1;

pub const F_NAV: u8 = 1;
pub const F_STROBE: u8 = 2;
pub const F_BEACON: u8 = 4;
pub const F_LANDING: u8 = 8;
pub const F_GROUND: u8 = 16;
pub const F_CRASHED: u8 = 32;

#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    Hello { name: String },
    Welcome { id: u32, cookie: u64 },
    State { cookie: u64, state: PlayerState },
    Peers { peers: Vec<(u32, String, PlayerState)> },
    Bye { cookie: u64 },
}

/// A name as it may be shown: printable characters only, trimmed, at most
/// `NAME_MAX` bytes (cut on a character boundary).
pub fn clean_name(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().chars().filter(|c| !c.is_control()) {
        if out.len() + c.len_utf8() > NAME_MAX {
            break;
        }
        out.push(c);
    }
    out
}

struct W(Vec<u8>);
impl W {
    fn new(kind: u8) -> Self {
        let mut v = Vec::with_capacity(128);
        v.extend_from_slice(&MAGIC);
        v.push(VERSION);
        v.push(kind);
        W(v)
    }
    fn u8(&mut self, x: u8) {
        self.0.push(x);
    }
    fn u32(&mut self, x: u32) {
        self.0.extend_from_slice(&x.to_le_bytes());
    }
    fn u64(&mut self, x: u64) {
        self.0.extend_from_slice(&x.to_le_bytes());
    }
    fn f32(&mut self, x: f32) {
        self.0.extend_from_slice(&x.to_le_bytes());
    }
    fn f64(&mut self, x: f64) {
        self.0.extend_from_slice(&x.to_le_bytes());
    }
    fn name(&mut self, s: &str) {
        let s = clean_name(s);
        self.u8(s.len() as u8);
        self.0.extend_from_slice(s.as_bytes());
    }
    fn state(&mut self, s: &PlayerState) {
        self.u32(s.seq);
        self.u8(s.aircraft);
        s.pos.iter().for_each(|&x| self.f64(x));
        s.rot.iter().for_each(|&x| self.f32(x));
        s.vel.iter().for_each(|&x| self.f32(x));
        for x in [s.flaps, s.gear, s.throttle, s.canopy] {
            self.u8(x);
        }
        for x in [s.pitch, s.roll, s.yaw] {
            self.u8(x as u8);
        }
        self.u8(s.flags);
    }
}

struct R<'a>(&'a [u8]);
impl R<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        if self.0.len() < n {
            return None;
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Some(a)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    }
    fn u64(&mut self) -> Option<u64> {
        self.take(8).map(|b| u64::from_le_bytes(b.try_into().unwrap()))
    }
    fn f32(&mut self) -> Option<f32> {
        self.take(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).filter(|x| x.is_finite())
    }
    fn f64(&mut self) -> Option<f64> {
        self.take(8).map(|b| f64::from_le_bytes(b.try_into().unwrap())).filter(|x| x.is_finite())
    }
    fn name(&mut self) -> Option<String> {
        let n = self.u8()? as usize;
        if n > NAME_MAX {
            return None;
        }
        let s = std::str::from_utf8(self.take(n)?).ok()?;
        Some(clean_name(s))
    }
    fn state(&mut self) -> Option<PlayerState> {
        let mut s = PlayerState { seq: self.u32()?, aircraft: self.u8()?, ..Default::default() };
        for x in s.pos.iter_mut() {
            *x = self.f64()?;
        }
        for x in s.rot.iter_mut() {
            *x = self.f32()?;
        }
        for x in s.vel.iter_mut() {
            *x = self.f32()?;
        }
        (s.flaps, s.gear, s.throttle, s.canopy) = (self.u8()?, self.u8()?, self.u8()?, self.u8()?);
        (s.pitch, s.roll, s.yaw) = (self.u8()? as i8, self.u8()? as i8, self.u8()? as i8);
        s.flags = self.u8()?;
        // Somewhere near the planet (within 10 000 km of its centre, past its
        // core), moving slower than 10 km/s, with a usable attitude.
        let r = (s.pos[0] * s.pos[0] + s.pos[1] * s.pos[1] + s.pos[2] * s.pos[2]).sqrt();
        let q = s.rot.iter().map(|x| x * x).sum::<f32>().sqrt();
        let v = s.vel.iter().map(|x| x * x).sum::<f32>().sqrt();
        (r > 6.0e6 && r < 1.0e7 && (q - 1.0).abs() < 0.05 && v < 10_000.0).then_some(s)
    }
}

impl Msg {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Msg::Hello { name } => {
                let mut w = W::new(K_HELLO);
                w.name(name);
                w.0.resize(w.0.len().max(HELLO_MIN), 0);
                w.0
            }
            Msg::Welcome { id, cookie } => {
                let mut w = W::new(K_WELCOME);
                w.u32(*id);
                w.u64(*cookie);
                w.0
            }
            Msg::State { cookie, state } => {
                let mut w = W::new(K_STATE);
                w.u64(*cookie);
                w.state(state);
                w.0
            }
            Msg::Peers { peers } => {
                let mut w = W::new(K_PEERS);
                let n = peers.len().min(PEERS_PER_PACKET);
                w.u8(n as u8);
                for (id, name, s) in &peers[..n] {
                    w.u32(*id);
                    w.name(name);
                    w.state(s);
                }
                w.0
            }
            Msg::Bye { cookie } => {
                let mut w = W::new(K_BYE);
                w.u64(*cookie);
                w.0
            }
        }
    }

    /// The message in a datagram; None for anything that is not a whole,
    /// sane message of this version.
    pub fn decode(b: &[u8]) -> Option<Msg> {
        let mut r = R(b);
        if r.take(4)? != MAGIC || r.u8()? != VERSION {
            return None;
        }
        let m = match r.u8()? {
            K_HELLO => {
                if b.len() < HELLO_MIN {
                    return None;
                }
                Msg::Hello { name: r.name()? }
            }
            K_WELCOME => Msg::Welcome { id: r.u32()?, cookie: r.u64()? },
            K_STATE => Msg::State { cookie: r.u64()?, state: r.state()? },
            K_PEERS => {
                let n = r.u8()? as usize;
                if n > PEERS_PER_PACKET {
                    return None;
                }
                let mut peers = Vec::with_capacity(n);
                for _ in 0..n {
                    peers.push((r.u32()?, r.name()?, r.state()?));
                }
                Msg::Peers { peers }
            }
            K_BYE => Msg::Bye { cookie: r.u64()? },
            _ => return None,
        };
        Some(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> PlayerState {
        PlayerState { seq: 7, aircraft: 8, pos: [4_000_000.0, 300_000.0, 4_950_000.0], rot: [0.0, 0.0, 0.0, 1.0], vel: [10.0, -2.0, 55.0], flaps: 128, gear: 255, throttle: 200, canopy: 0, pitch: -40, roll: 12, yaw: 0, flags: F_NAV | F_BEACON }
    }

    #[test]
    fn every_message_comes_back_the_same() {
        for m in [
            Msg::Hello { name: "F-GXYZ".into() },
            Msg::Welcome { id: 3, cookie: 0xdead_beef_1234 },
            Msg::State { cookie: 99, state: state() },
            Msg::Peers { peers: vec![(1, "Dany".into(), state()), (2, "Roland".into(), state())] },
            Msg::Bye { cookie: 5 },
        ] {
            let b = m.encode();
            assert_eq!(Msg::decode(&b), Some(m.clone()), "{m:?}");
        }
        assert_eq!(Msg::State { cookie: 1, state: state() }.encode().len(), 6 + 8 + STATE_LEN);
    }

    #[test]
    fn the_answer_to_a_hello_is_never_bigger() {
        let hello = Msg::Hello { name: String::new() }.encode();
        let welcome = Msg::Welcome { id: u32::MAX, cookie: u64::MAX }.encode();
        assert!(hello.len() >= HELLO_MIN && welcome.len() <= hello.len());
        // A short Hello is not one.
        assert_eq!(Msg::decode(&hello[..HELLO_MIN - 1]), None);
    }

    #[test]
    fn junk_is_turned_away() {
        let good = Msg::State { cookie: 1, state: state() }.encode();
        for n in 0..good.len() {
            assert_eq!(Msg::decode(&good[..n]), None, "cut at {n}");
        }
        let mut v = good.clone();
        v[4] = VERSION + 1;
        assert_eq!(Msg::decode(&v), None);
        let mut s = state();
        s.pos[0] = f64::NAN;
        assert_eq!(Msg::decode(&Msg::State { cookie: 1, state: s }.encode()), None);
        s = state();
        s.pos = [1.0, 2.0, 3.0];
        assert_eq!(Msg::decode(&Msg::State { cookie: 1, state: s }.encode()), None);
        s = state();
        s.rot = [0.0, 0.0, 0.0, 3.0];
        assert_eq!(Msg::decode(&Msg::State { cookie: 1, state: s }.encode()), None);
    }

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean_name("  Dany\n\u{7}  "), "Dany");
        assert_eq!(clean_name(&"é".repeat(40)).len(), NAME_MAX);
        let peers = (0..20).map(|i| (i, "x".repeat(NAME_MAX), state())).collect();
        let b = Msg::Peers { peers }.encode();
        assert!(b.len() < 1200, "{}", b.len());
    }
}
