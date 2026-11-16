//! Real weather: the current conditions where the player is (or where they
//! are about to start), turned into the weather sliders. The nearest METAR
//! (aviationweather.gov) within 60 km gives the wind, gusts, visibility, the
//! cloud layers, thunderstorms and rain; away from any airport, Open-Meteo's model
//! stands in. A background thread looks again every ten minutes or after a
//! move of 30 km, and the sky eases into each new report.

use crate::geo;
use crate::sources::Sources;
use crate::weather::Weather;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How often to look again (s), and how far the player may move first (km).
const REFRESH: f64 = 600.0;
const MOVE_KM: f64 = 30.0;
/// The nearest METAR station is used up to this far away (km).
const STATION_KM: f64 = 60.0;

/// The weather sliders a report comes to.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub clouds: f32,
    /// None: the report says nothing about it (keep what there is)
    pub base_m: Option<f32>,
    pub depth_m: f32,
    pub alto: f32,
    pub storms: f32,
    /// rain falling at the station, 0..1 (the Rain slider: it brings the
    /// nimbostratus rain deck); 0 = none
    pub rain: f32,
    pub haze: f32,
    /// m/s, and where from (None: variable)
    pub wind: f64,
    pub wind_from: Option<f64>,
    pub gusts: f64,
    /// what it came from, for the weather panel: "EBBR" or "Open-Meteo",
    /// and the report itself (the raw METAR)
    pub station: String,
    pub raw: String,
}

#[derive(Default)]
struct Shared {
    want: Option<(f64, f64)>,
    got: Option<(Instant, Target)>,
    error: String,
}

pub struct LiveWx {
    shared: Arc<Mutex<Shared>>,
    thread: bool,
    last_got: Option<Instant>,
    target: Option<Target>,
    /// ease in (false right after a new place is picked: jump straight there)
    settled: bool,
    /// the Rain slider (`rain_set`) is ours: handed back when live weather goes off
    owns_rain: bool,
}

impl LiveWx {
    pub fn new() -> Self {
        Self { shared: Arc::default(), thread: false, last_got: None, target: None, settled: false, owns_rain: false }
    }

    pub fn error(&self) -> String {
        self.shared.lock().error.clone()
    }

    /// Each frame: where the weather is wanted (degrees), and the time since
    /// the last frame (s; 0 jumps straight to the report, as on the globe).
    pub fn step(&mut self, src: &Arc<Sources>, wx: &mut Weather, at: Option<(f64, f64)>, dt: f64) {
        if !wx.live {
            self.settled = false;
            if self.owns_rain {
                self.owns_rain = false;
                wx.rain_set = None;
            }
            return;
        }
        let got = {
            let mut s = self.shared.lock();
            s.want = at;
            s.got.as_ref().filter(|(t, _)| Some(*t) != self.last_got).cloned()
        };
        if !self.thread && at.is_some() {
            self.thread = true;
            spawn(src.clone(), self.shared.clone());
        }
        if let Some((t, target)) = got {
            self.last_got = Some(t);
            wx.live_station = target.station.clone();
            wx.live_raw = target.raw.clone();
            self.target = Some(target);
        }
        let Some(t) = &self.target else {
            let e = self.error();
            if !e.is_empty() {
                wx.live_raw = format!("({e})");
            }
            return;
        };
        // About half way in 10 s; at once on the globe or the first time.
        let k = if dt <= 0.0 || !self.settled { 1.0 } else { 1.0 - (-dt / 14.0).exp() };
        self.settled = true;
        let mix = |a: f32, b: f32| a + (b - a) * k as f32;
        wx.clouds = mix(wx.clouds, t.clouds);
        if let Some(b) = t.base_m {
            wx.cloud_base = mix(wx.cloud_base, b);
        }
        wx.cloud_depth = mix(wx.cloud_depth, t.depth_m);
        wx.alto = mix(wx.alto, t.alto);
        wx.storms = mix(wx.storms, t.storms);
        wx.haze = mix(wx.haze, t.haze);
        // The rain deck eases in and out by itself (weather.rs `step_deck`).
        wx.rain_set = (t.rain > 0.0).then_some(t.rain);
        self.owns_rain = true;
        wx.wind += (t.wind - wx.wind) * k;
        wx.gusts += (t.gusts - wx.gusts) * k;
        if let Some(from) = t.wind_from {
            let d = (from - wx.wind_from + 540.0).rem_euclid(360.0) - 180.0;
            wx.wind_from = (wx.wind_from + d * k).rem_euclid(360.0);
        }
    }
}

fn spawn(src: Arc<Sources>, shared: Arc<Mutex<Shared>>) {
    std::thread::spawn(move || {
        let mut last: Option<(Instant, (f64, f64))> = None;
        loop {
            let want = shared.lock().want;
            if let Some(p) = want {
                let due = match last {
                    None => true,
                    Some((t, q)) => t.elapsed().as_secs_f64() > REFRESH || dist_km(p, q) > MOVE_KM,
                };
                if due {
                    let res = metar(&src, p).or_else(|e| {
                        log::info!("live weather: {e}; using Open-Meteo");
                        open_meteo(&src, p)
                    });
                    let mut s = shared.lock();
                    match res {
                        Ok(t) => {
                            s.got = Some((Instant::now(), t));
                            s.error.clear();
                            last = Some((Instant::now(), p));
                        }
                        Err(e) => {
                            if e != "offline" {
                                log::warn!("live weather: {e}");
                            }
                            s.error = e;
                            // try again in a minute, not every second
                            last = Some((Instant::now() - Duration::from_secs_f64(REFRESH - 60.0), p));
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

fn dist_km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let p = |q: (f64, f64)| geo::lla_to_pos(q.0.to_radians(), q.1.to_radians(), 0.0);
    (p(a) - p(b)).length() / 1000.0
}

fn get_json(src: &Sources, url: &str) -> Result<serde_json::Value, String> {
    let d = src.fetch.get(url).map_err(|e| e.to_string())?;
    serde_json::from_slice(&d).map_err(|e| e.to_string())
}

/// The nearest METAR station's latest report.
fn metar(src: &Sources, (lat, lon): (f64, f64)) -> Result<Target, String> {
    let dlat = STATION_KM / 111.0;
    let dlon = dlat / lat.to_radians().cos().max(0.2);
    let url = format!("https://aviationweather.gov/api/data/metar?bbox={:.3},{:.3},{:.3},{:.3}&format=json", lat - dlat, lon - dlon, lat + dlat, lon + dlon);
    let j = get_json(src, &url)?;
    let near = j
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| Some((dist_km((lat, lon), (m["lat"].as_f64()?, m["lon"].as_f64()?)), m)))
        .filter(|(d, _)| *d < STATION_KM)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, m)| m)
        .ok_or_else(|| "no METAR station near".to_string())?;
    from_metar(near).ok_or_else(|| "unreadable METAR".to_string())
}

/// A METAR (aviationweather.gov's JSON) as the weather sliders.
fn from_metar(m: &serde_json::Value) -> Option<Target> {
    let raw = m["rawOb"].as_str().unwrap_or("").to_string();
    let station = m["icaoId"].as_str()?.to_string();
    let elev = m["elev"].as_f64().unwrap_or(0.0);
    let kt = |v: &serde_json::Value| v.as_f64();
    let wind = kt(&m["wspd"]).unwrap_or(0.0) / 1.94384;
    let gust = kt(&m["wgst"]).map(|g| g / 1.94384);
    let wind_from = m["wdir"].as_f64();
    // Visibility in statute miles: 6, "6+", "1/2", "1 1/2".
    let vis_km = match &m["visib"] {
        serde_json::Value::Number(n) => n.as_f64(),
        // ("6+": at least six miles, as clear as the sky model goes)
        serde_json::Value::String(s) if s.trim().ends_with('+') => None,
        serde_json::Value::String(s) => sm(s),
        _ => None,
    }
    .map(|v| v * 1.609);
    let tokens: Vec<&str> = raw.split_whitespace().collect();
    let wx = m["wxString"].as_str().unwrap_or("");
    let cavok = tokens.contains(&"CAVOK");
    // The cloud layers: (cover 0..1, base in m above sea level).
    let mut layers: Vec<(f32, f32)> = m["clouds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let cover = match c["cover"].as_str()? {
                "FEW" => 0.2,
                "SCT" => 0.45,
                "BKN" => 0.75,
                "OVC" | "OVX" | "VV" => 1.0,
                _ => return None,
            };
            Some((cover, (c["base"].as_f64()? / 3.28084 + elev) as f32))
        })
        .collect();
    layers.sort_by(|a, b| a.1.total_cmp(&b.1));
    let cb = tokens.iter().any(|t| t.ends_with("CB"));
    let tcu = tokens.iter().any(|t| t.ends_with("TCU"));
    let p = precip(wx);
    let storms: f32 = if p.thunder { 0.75 } else if cb { 0.45 } else if p.showers > 0.0 || tcu { 0.2 } else { 0.0 };
    let storms = storms.max(p.showers);
    let (clouds, base_m, alto) = match layers.first() {
        None => (0.03, None, 0.0),
        Some(&(_, base)) => {
            let low = layers.iter().filter(|l| l.1 < base + 1000.0).map(|l| l.0).fold(0.0f32, f32::max);
            let high = layers.iter().filter(|l| l.1 >= base + 1000.0).map(|l| l.0).fold(0.0f32, f32::max);
            (low, Some(base.clamp(300.0, 4000.0)), high)
        }
    };
    // Showers fall from cumulus: enough of it for the storm cells to grow.
    let clouds = if p.showers > 0.0 { clouds.max(0.45) } else { clouds };
    let foggy = ["FG", "BR", "HZ", "FU", "DU", "SA"].iter().any(|w| wx.contains(w));
    Some(Target {
        clouds,
        base_m,
        depth_m: depth(clouds, storms),
        alto,
        storms,
        rain: p.rain,
        haze: haze(if cavok { None } else { vis_km }, foggy),
        wind,
        wind_from,
        gusts: gust.map(|g| ((g - wind) / 8.0).clamp(0.0, 1.0)).unwrap_or(0.1),
        station,
        raw,
    })
}

/// What falls, from a METAR's present weather ("-SHRA BR", "+TSRA", "DZ").
#[derive(Debug, PartialEq)]
struct Precip {
    /// the rain deck (the Rain slider), 0..1
    rain: f32,
    /// showers: the storm-cell slider they want (convective cloud), 0 = none
    showers: f32,
    thunder: bool,
}

/// Rain deck amount for each intensity (light, moderate, heavy): drizzle
/// lighter still. Showers and thunderstorms rain mostly from their cells, so
/// they bring a thinner deck.
const RAIN: [f32; 3] = [0.3, 0.55, 0.85];
const DRIZZLE: [f32; 3] = [0.12, 0.18, 0.25];
const CONVECTIVE_DECK: f32 = 0.5;

fn precip(wx: &str) -> Precip {
    let mut p = Precip { rain: 0.0, showers: 0.0, thunder: false };
    for tok in wx.split_whitespace() {
        let (level, rest) = match tok.as_bytes().first() {
            Some(b'-') => (0, &tok[1..]),
            Some(b'+') => (2, &tok[1..]),
            _ => (1, tok),
        };
        // In the vicinity (VC) or recent (RE): not falling here.
        let near = !(rest.starts_with("VC") || rest.starts_with("RE"));
        let codes: Vec<&str> = (0..rest.len() / 2).map(|i| &rest[2 * i..2 * i + 2]).collect();
        let has = |c: &str| codes.contains(&c);
        let ts = has("TS");
        let sh = has("SH");
        p.thunder |= ts;
        let mut r = 0.0f32;
        if has("RA") {
            r = RAIN[level];
        } else if has("DZ") {
            r = DRIZZLE[level];
        }
        if sh && !ts {
            // stronger showers, taller and more cells
            p.showers = p.showers.max(if near { [0.25, 0.35, 0.5][level] } else { 0.2 });
        }
        if near && r > 0.0 {
            p.rain = p.rain.max(if ts || sh { r * CONVECTIVE_DECK } else { r });
        }
    }
    p
}

/// "6+", "1/2", "1 1/2" statute miles.
fn sm(s: &str) -> Option<f64> {
    let s = s.trim().trim_end_matches('+');
    s.split_whitespace().map(|p| match p.split_once('/') {
        Some((a, b)) => Some(a.parse::<f64>().ok()? / b.parse::<f64>().ok()?),
        None => p.parse().ok(),
    }).sum()
}

/// Cloud layer thickness: towering with storms, a deck when overcast.
fn depth(clouds: f32, storms: f32) -> f32 {
    if storms > 0.3 { 6000.0 } else if clouds >= 0.75 { 2500.0 } else { 1800.0 }
}

/// The haze slider for a visibility (km). The sky model's clear air already
/// sees ~180 km; it cannot show less than ~11 km, so anything under ten
/// (mist, fog) takes the thickest haze.
fn haze(vis_km: Option<f64>, foggy: bool) -> f32 {
    match vis_km {
        None => 1.0,
        Some(v) if v >= 9.5 && !foggy => 1.0 + (2.5 * (1.0 - (v - 9.5) / 15.0)).clamp(0.0, 2.5) as f32,
        Some(v) => {
            let h = (3.912 / (v.max(0.1) * 1000.0) - 13.558e-6) / 8.396e-6;
            h.clamp(6.0, 40.0) as f32
        }
    }
}

/// Open-Meteo's model where there is no METAR near.
fn open_meteo(src: &Sources, (lat, lon): (f64, f64)) -> Result<Target, String> {
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat:.3}&longitude={lon:.3}&current=temperature_2m,dew_point_2m,cloud_cover_low,cloud_cover_mid,wind_speed_10m,wind_direction_10m,wind_gusts_10m,visibility,weather_code,rain,showers&wind_speed_unit=ms"
    );
    let j = get_json(src, &url)?;
    from_open_meteo(&j).ok_or_else(|| "unreadable Open-Meteo answer".to_string())
}

fn from_open_meteo(j: &serde_json::Value) -> Option<Target> {
    let c = &j["current"];
    let f = |k: &str| c[k].as_f64();
    let elev = j["elevation"].as_f64().unwrap_or(0.0);
    let code = f("weather_code").unwrap_or(0.0) as u32;
    let low = (f("cloud_cover_low").unwrap_or(0.0) / 100.0) as f32;
    let mid = (f("cloud_cover_mid").unwrap_or(0.0) / 100.0) as f32;
    // Cumulus base from the spread: about 125 m per degree.
    let base = match (f("temperature_2m"), f("dew_point_2m")) {
        (Some(t), Some(d)) => Some(((t - d).max(0.0) * 125.0 + elev).clamp(300.0, 4000.0) as f32),
        _ => None,
    };
    let storms = match code {
        95..=99 => 0.75,
        80 => 0.25,
        81 => 0.35,
        82 => 0.5,
        _ => 0.0,
    };
    // WMO weather codes: drizzle 51-57, rain 61-67, showers 80-82, storms 95-99.
    let by_code = match code {
        51 | 56 => DRIZZLE[0],
        53 => DRIZZLE[1],
        55 | 57 => DRIZZLE[2],
        61 | 66 => RAIN[0],
        63 => RAIN[1],
        65 | 67 => RAIN[2],
        80 | 95 => RAIN[0] * CONVECTIVE_DECK,
        81 | 96 => RAIN[1] * CONVECTIVE_DECK,
        82 | 99 => RAIN[2] * CONVECTIVE_DECK,
        _ => 0.0,
    };
    // The amount (mm/h) where the code says nothing falls: light under 2.5,
    // heavy from 7.6.
    let mm = f("rain").unwrap_or(0.0) + f("showers").unwrap_or(0.0);
    let by_amount = if mm < 0.1 { 0.0 } else if mm < 2.5 { RAIN[0] } else if mm < 7.6 { RAIN[1] } else { RAIN[2] };
    let rain = if by_code > 0.0 || (50..=99).contains(&code) { by_code } else { by_amount };
    let clouds = if low > 0.05 { low } else { mid * 0.6 }.max(0.03);
    let clouds = if (80..=82).contains(&code) { clouds.max(0.45) } else { clouds };
    let wind = f("wind_speed_10m")?;
    Some(Target {
        clouds,
        base_m: base,
        depth_m: depth(clouds, storms),
        alto: mid,
        storms,
        rain,
        haze: haze(f("visibility").map(|v| v / 1000.0), matches!(code, 45 | 48)),
        wind,
        wind_from: f("wind_direction_10m"),
        gusts: f("wind_gusts_10m").map(|g| ((g - wind) / 8.0).clamp(0.0, 1.0)).unwrap_or(0.1),
        station: "Open-Meteo".into(),
        raw: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metar() {
        let m = serde_json::json!({"icaoId":"EGLL","rawOb":"METAR EGLL 151720Z 24015G28KT 9999 -TSRA FEW012 BKN025CB OVC080 12/10 Q1002","elev":25,"wdir":240,"wspd":15,"wgst":28,"visib":"6+","wxString":"-TSRA","clouds":[{"cover":"FEW","base":1200},{"cover":"BKN","base":2500},{"cover":"OVC","base":8000}]});
        let t = from_metar(&m).unwrap();
        assert_eq!(t.station, "EGLL");
        assert_eq!(t.wind_from, Some(240.0));
        assert!((t.wind - 15.0 / 1.94384).abs() < 1e-6);
        assert!(t.gusts > 0.8);
        assert_eq!(t.storms, 0.75);
        assert_eq!(t.rain, RAIN[0] * CONVECTIVE_DECK);
        // FEW at 1200 ft and BKN at 2500 ft are one layer; OVC at 8000 ft is mid-level cloud.
        assert_eq!(t.clouds, 0.75);
        assert_eq!(t.alto, 1.0);
        assert!((t.base_m.unwrap() - (1200.0 / 3.28084 + 25.0) as f32).abs() < 0.5);
        assert!(t.haze < 2.0);

        let cavok = serde_json::json!({"icaoId":"EBBR","rawOb":"METAR EBBR 151720Z 19010KT CAVOK 10/09 Q0999","elev":56,"wdir":190,"wspd":10,"visib":"6+","clouds":[]});
        let t = from_metar(&cavok).unwrap();
        assert_eq!((t.clouds, t.base_m, t.storms, t.haze, t.rain), (0.03, None, 0.0, 1.0, 0.0));

        let fog = serde_json::json!({"icaoId":"LFPG","rawOb":"METAR LFPG 150600Z VRB02KT 0300 FG VV001 05/05 Q1020","wdir":"VRB","wspd":2,"visib":0.25,"wxString":"FG","clouds":[{"cover":"OVX","base":100}]});
        let t = from_metar(&fog).unwrap();
        assert_eq!(t.wind_from, None);
        assert_eq!(t.haze, 40.0);
        assert_eq!(t.base_m, Some(300.0));
        assert_eq!(t.rain, 0.0);

        let rain = serde_json::json!({"icaoId":"KOMA","rawOb":"METAR KOMA 151852Z 18012KT 3SM +RA BR OVC008 08/07 A2990","elev":299,"wdir":180,"wspd":12,"visib":3,"wxString":"+RA BR","clouds":[{"cover":"OVC","base":800}]});
        let t = from_metar(&rain).unwrap();
        assert_eq!((t.rain, t.storms), (RAIN[2], 0.0));

        let showers = serde_json::json!({"icaoId":"KLNK","rawOb":"METAR KLNK 151854Z 27010KT 10SM -SHRA SCT040 BKN060 15/09 A2995","elev":362,"wdir":270,"wspd":10,"visib":"10+","wxString":"-SHRA","clouds":[{"cover":"SCT","base":4000},{"cover":"BKN","base":6000}]});
        let t = from_metar(&showers).unwrap();
        assert_eq!((t.rain, t.storms), (RAIN[0] * CONVECTIVE_DECK, 0.25));
        assert!(t.clouds >= 0.45);
    }

    #[test]
    fn present_weather() {
        let p = |s| precip(s);
        assert_eq!(p("").rain, 0.0);
        assert_eq!(p("BR HZ").rain, 0.0);
        assert_eq!(p("-RA").rain, RAIN[0]);
        assert_eq!(p("RA").rain, RAIN[1]);
        assert_eq!(p("+RA").rain, RAIN[2]);
        assert_eq!(p("-DZ").rain, DRIZZLE[0]);
        assert_eq!(p("DZ BR").rain, DRIZZLE[1]);
        assert_eq!(p("-RADZ").rain, RAIN[0]);
        assert_eq!(p("-DZ RA").rain, RAIN[1]);
        assert!(p("+DZ").rain < p("-RA").rain);
        assert_eq!(p("-RASN").rain, RAIN[0]);
        assert_eq!(p("SN").rain, 0.0);
        let sh = p("+SHRA");
        assert_eq!((sh.rain, sh.showers, sh.thunder), (RAIN[2] * CONVECTIVE_DECK, 0.5, false));
        let ts = p("TSRA");
        assert_eq!((ts.rain, ts.showers, ts.thunder), (RAIN[1] * CONVECTIVE_DECK, 0.0, true));
        assert!(p("TS").thunder && p("TS").rain == 0.0);
        let vc = p("VCSH");
        assert_eq!((vc.rain, vc.showers), (0.0, 0.2));
        assert_eq!(p("RERA").rain, 0.0);
        assert_eq!(p("FZRA").rain, RAIN[1]);
    }

    #[test]
    fn visibility_text() {
        assert_eq!(sm("6+"), Some(6.0));
        assert_eq!(sm("1/2"), Some(0.5));
        assert_eq!(sm("1 1/2"), Some(1.5));
    }

    #[test]
    fn open_meteo_model() {
        let j = serde_json::json!({"elevation":37.0,"current":{"temperature_2m":10.0,"dew_point_2m":8.0,"cloud_cover_low":100,"cloud_cover_mid":40,"wind_speed_10m":6.6,"wind_direction_10m":203,"wind_gusts_10m":13.1,"visibility":19420.0,"weather_code":53}});
        let t = from_open_meteo(&j).unwrap();
        assert_eq!((t.clouds, t.alto, t.storms, t.rain), (1.0, 0.4, 0.0, DRIZZLE[1]));
        assert_eq!(t.base_m, Some(300.0));
        assert_eq!(t.wind_from, Some(203.0));
        assert_eq!(t.station, "Open-Meteo");

        let om = |code: u32, rain: f64, showers: f64| {
            let j = serde_json::json!({"elevation":0.0,"current":{"cloud_cover_low":30,"cloud_cover_mid":0,"wind_speed_10m":3.0,"weather_code":code,"rain":rain,"showers":showers}});
            from_open_meteo(&j).unwrap()
        };
        assert_eq!(om(0, 0.0, 0.0).rain, 0.0);
        assert_eq!(om(3, 0.0, 0.0).rain, 0.0);
        assert_eq!(om(61, 0.5, 0.0).rain, RAIN[0]);
        assert_eq!(om(65, 9.0, 0.0).rain, RAIN[2]);
        // a cloudy code with rain falling anyway: the amount decides
        assert_eq!(om(3, 4.0, 0.0).rain, RAIN[1]);
        let t = om(81, 0.0, 3.0);
        assert_eq!((t.rain, t.storms), (RAIN[1] * CONVECTIVE_DECK, 0.35));
        assert!(t.clouds >= 0.45);
        assert_eq!(om(95, 0.0, 5.0).storms, 0.75);
        // fog (45) with a trace of rain in the model: no deck
        assert_eq!(om(45, 0.05, 0.0).rain, 0.0);
    }
}
