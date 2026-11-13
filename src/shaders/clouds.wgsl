// Volumetric cloud layer: raymarched at half resolution with a fresh jitter every
// frame, accumulated over frames by reprojection, then upsampled onto the scene.
@group(1) @binding(0) var t_depth: texture_depth_2d;

const SIGMA = 0.05;   // extinction per unit density, 1/m
const MS_GAIN = 2.6;  // energy the octave sum misses in thick cloud

fn hg(c: f32, g: f32) -> f32 { return (1.0 - g * g) / (4.0 * PI * pow(1.0 + g * g - 2.0 * g * c, 1.5)); }
// Strong forward lobe (silver linings), weaker back lobe; k < 1 flattens it for
// light that has already scattered a few times.
fn cloud_phase(c: f32, k: f32) -> f32 { return mix(hg(c, 0.82 * k), hg(c, -0.25 * k), 0.28); }

// dist.x: distance (km) to the clouds this texel sees; dist.y: distance (km) to the
// scene surface the march stopped at, for the depth-aware passes below.
struct CloudOut { @location(0) col: vec4f, @location(1) dist: vec2f };

const SKY_KM = 10000.0;  // scene distance stored for sky texels
// How well two scene distances agree, 1 = same surface.
fn depth_match(a: f32, b: f32) -> f32 {
  let rel = abs(a - b) / max(max(a, b), 1e-6);
  return exp(-rel * rel * 200.0);
}

@fragment fn fs(in: FsIn) -> CloudOut {
  var o = CloudOut(vec4f(0.0, 0.0, 0.0, 1.0), vec2f(0.0));
  let ray = view_ray(in.uv);
  let rd = normalize(ray);
  // Each texel covers 2x2 scene pixels. Alternate texels march to the nearest and the
  // farthest of them (reverse Z: larger = nearer), so at a silhouette both surfaces
  // are represented in every 2x2 texel neighbourhood and the upsample can pick the
  // matching one instead of smearing cloud over the plane or clear sky around it.
  let hp = vec2i(in.pos.xy);
  let q = hp * 2;
  let z4 = vec4f(textureLoad(t_depth, q, 0), textureLoad(t_depth, q + vec2i(1, 0), 0),
                 textureLoad(t_depth, q + vec2i(0, 1), 0), textureLoad(t_depth, q + vec2i(1, 1), 0));
  var z = min(min(z4.x, z4.y), min(z4.z, z4.w));
  if (((hp.x + hp.y) & 1) == 0) { z = max(max(z4.x, z4.y), max(z4.z, z4.w)); }
  var tscene = 1e9;
  if (z > 0.0) { tscene = length(ray / z); }
  o.dist.y = min(tscene * 0.001, SKY_KM);
  let rg = G.planet.w;
  let oc = -G.planet.xyz;
  let base = G.cloud.y;
  let alt = G.up.w;
  let sun = G.sun_dir.xyz;
  let cs = dot(rd, sun);
  let jit = fract(ign(in.pos.xy) + G.time.w * 0.618034);

  var T = 1.0;
  var L = vec3f(0.0);
  var tw = 0.0;
  var ws = 0.0;

  // Sky light for the ambient term: zenith and horizon from the sky table, plus
  // sunlight bounced off the ground below.
  let mu0 = dot(G.up.xyz, sun);
  let sky_up = sky_lut(G.up.xyz);
  let sky_side = 0.5 * (sky_lut(normalize(G.east.xyz + G.up.xyz * 0.15)) + sky_lut(normalize(-G.east.xyz + G.up.xyz * 0.15)));
  // Inside a cloud field much of the "sky" is other clouds: pull the ambient towards grey.
  let sky_raw = sky_up * 0.55 + sky_side * 0.45;
  let sky_amb = mix(sky_raw, vec3f(luma(sky_raw)), 0.45);
  var gnd_amb = vec3f(0.11, 0.11, 0.1) * sun_trans(0.0, mu0) * max(mu0, 0.0) * G.sun_dir.w / PI * mix(1.0, 0.45, G.cloud.x);
  // At night the cloud bases catch the city's glow from below.
  gnd_amb += SKYGLOW * 0.0045 * G.misc.y * (1.0 - smoothstep(-0.2, 0.05, mu0));

  // Only rays that pass near a storm cell march up to the storm ceiling and look the
  // storms up; the rest keep to the ordinary layer.
  let smask = storm_mask(oc, rd, min(tscene, 200000.0));
  let topa = select(base + G.cloud.z, G.cloud.w, smask != 0u);
  let outer = ray_sphere(oc, rd, rg + topa);
  let inner = ray_sphere(oc, rd, rg + base);
  var ta = 0.0;
  var tb = outer.y;
  var march = outer.y > 0.0;
  if (alt < base) {
    if (z == 0.0 && ray_sphere(oc, rd, rg).x > 0.0) { march = false; }
    ta = inner.y;
  } else {
    if (alt > topa) { ta = outer.x; }
    if (inner.x > 0.0) { tb = inner.x; }
  }
  tb = min(min(tb, tscene), ta + 160000.0);
  if (tb <= ta || ta < 0.0) { march = false; }

  if (march) {
    // Steps grow with distance (a pixel covers more ground further out), and
    // stretch while the ray is in clear air.
    var t = ta;
    var clear = 0.0;
    for (var i = 0; i < 200; i++) {
      let dt = clamp(t * 0.016, 40.0, 2500.0) * (1.0 + clear);
      let ts = t + dt * jit;
      t += dt;
      if (ts >= tb) { break; }
      let p = oc + rd * ts;
      let r = length(p);
      let h = r - rg;
      let wx = cloud_weather_m(p, smask);
      // The close-up erosion octave only pays off within a few km.
      let fine = 1.0 - smoothstep(1500.0, 4500.0, ts);
      let d = cloud_shape_f(p, h, wx, true, fine);
      // Stride faster through the empty storm-only air above the ordinary layer.
      if (d <= 0.002) { clear = min(clear + 0.2, select(1.0, 3.0, h > base + G.cloud.z && wx.cb <= 0.0)); continue; }
      clear = 0.0;
      let upn = p / r;
      let ctop = column_top(wx);
      let hn = clamp((h - base) / ctop, 0.0, 1.0);
      // Optical depth towards the sun; the column's weather is reused along the way.
      // Tall columns take one more, long step (out to ~3 km) so towers shade their neighbours.
      var od = 0.0;
      var ls = 36.0;
      var lt = 0.0;
      let nl = select(6, 7, ctop > 2500.0);
      for (var j = 0; j < nl; j++) {
        let lp = p + sun * (lt + ls * (0.5 + 0.4 * (jit - 0.5)));
        od += cloud_shape(lp, length(lp) - rg, wx, j < 2) * ls;
        lt += ls;
        ls *= 1.85;
      }
      od *= SIGMA;
      // Cloud overhead blocks the sky light: a probe straight up, diffusion falloff.
      let pu = p + upn * 450.0;
      let od_up = cloud_shape(pu, h + 450.0, wx, false) * 900.0 * SIGMA;
      let occ = mix(1.0 / (1.0 + 0.11 * od_up), 1.0, 0.15);
      // Multiple scattering as a sum of ever dimmer, softer, more isotropic octaves.
      var ms = 0.0;
      var a = 1.0;
      var b = 1.0;
      var c = 1.0;
      for (var k = 0; k < 4; k++) {
        ms += a * exp(-od * b) * cloud_phase(cs, c);
        a *= 0.55;
        b *= 0.38;
        c *= 0.5;
      }
      // Deep inside, light arrives by diffusion, which falls off like 1/depth rather
      // than exponentially (two-stream approximation, g ~ 0.85).
      ms += 0.55 / (4.0 * PI) / (1.0 + 0.11 * od);
      // In-scatter probability: wispy edges and the undersides of clouds collect
      // less multiply-scattered light, which gives the dark creases between puffs.
      let dprob = 0.08 + pow(d, remap(clamp(hn, 0.3, 0.85), 0.3, 0.85, 0.5, 2.0));
      let vprob = pow(clamp(remap(hn, 0.05, 0.2, 0.25, 1.0), 0.25, 1.0), 0.8);
      let powder = mix(1.0, clamp(dprob * vprob * 1.25, 0.0, 1.0), 0.55 * (1.0 - 0.6 * max(cs, 0.0)));
      let mu = dot(upn, sun);
      let sunc = sun_trans(h, mu) * G.sun_dir.w;
      let amb = (sky_amb * mix(0.35, 1.05, hn) * occ + gnd_amb * mix(1.0, 0.25, hn)) * mix(0.5, 1.0, powder);
      let moonc = moon_light(upn, h);
      let S = ((sunc + moonc) * ms * MS_GAIN * powder + amb) * d * SIGMA;
      let e = d * SIGMA;
      let st = exp(-e * dt);
      L += T * S * (1.0 - st) / e;
      let wgt = T * (1.0 - st);
      tw += ts * wgt;
      ws += wgt;
      T *= st;
      if (T < 0.004) { T = 0.0; break; }
    }
    // Distant clouds sink into the haze.
    if (ws > 0.0) {
      let tc = tw / ws;
      o.dist.x = tc * 0.001;
      let top = ray_sphere(oc, rd, rg + ATM_H);
      let sc = atm_march(oc, rd, max(top.x, 0.0), tc, 6, 0.5);
      L = L * sc.T + sc.L * (1.0 - T);
    }
  }

  // Cirrus sheet, behind the cumulus layer when seen from below.
  if (T > 0.01) {
    let ci = ray_sphere(oc, rd, rg + CIRRUS_H);
    var tci = -1.0;
    if (alt < CIRRUS_H) { tci = ci.y; } else if (ci.x > 0.0 && ray_sphere(oc, rd, rg).x < 0.0) { tci = ci.x; }
    if (tci > 0.0 && tci < tscene && tci < 400000.0) {
      let p = oc + rd * tci;
      let r = length(p);
      let upn = p / r;
      let slant = 1.0 / max(abs(dot(rd, upn)), 0.08);
      let tau = cirrus(p) * slant * 0.35;
      if (tau > 0.001) {
        let mu = dot(upn, sun);
        let sunc = sun_trans(CIRRUS_H, mu) * G.sun_dir.w;
        let ph = mix(hg(cs, 0.75), hg(cs, -0.1), 0.35) + 0.04;
        var Lc = ((sunc + moon_light(upn, CIRRUS_H)) * ph + sky_up * 0.5 + gnd_amb * 0.3) * (1.0 - exp(-tau)) * 0.95;
        let ac = 1.0 - exp(-tau);
        let top = ray_sphere(oc, rd, rg + ATM_H);
        let sc = atm_march(oc, rd, max(top.x, 0.0), tci, 4, 0.5);
        Lc = Lc * sc.T + sc.L * ac;
        L += T * Lc;
        if (ws <= 0.0) { o.dist.x = tci * 0.001; }
        T *= 1.0 - ac;
      }
    }
  }
  o.col = vec4f(L, T);
  return o;
}

// Temporal accumulation: blend this frame's noisy march into the reprojected
// history, clamped to the current neighbourhood so moving clouds do not smear.
@group(1) @binding(2) var t_cur: texture_2d<f32>;
@group(1) @binding(3) var t_cdist: texture_2d<f32>;
@group(1) @binding(4) var t_hist: texture_2d<f32>;
@fragment fn fs_temporal(in: FsIn) -> @location(0) vec4f {
  let dims = vec2i(textureDimensions(t_cur));
  let px = vec2i(in.pos.xy);
  let cur = textureLoad(t_cur, px, 0);
  let cd = textureLoad(t_cdist, px, 0).rg;
  // Neighbourhood statistics over texels on the same surface only: across a
  // silhouette the clear texels in front would otherwise widen the clamp and let
  // the history bleed cloud onto the plane (and clear sky into the cloud).
  var m1 = vec4f(0.0);
  var m2 = vec4f(0.0);
  var n = 0.0;
  var mn = cur;
  var mx = cur;
  for (var y = -1; y <= 1; y++) {
    for (var x = -1; x <= 1; x++) {
      let c = clamp(px + vec2i(x, y), vec2i(0), dims - 1);
      if (depth_match(textureLoad(t_cdist, c, 0).g, cd.y) < 0.5) { continue; }
      let s = textureLoad(t_cur, c, 0);
      m1 += s;
      m2 += s * s;
      n += 1.0;
      mn = min(mn, s);
      mx = max(mx, s);
    }
  }
  let mean = m1 / n;
  let sd = sqrt(max(m2 / n - mean * mean, vec4f(0.0)));
  let lo = max(mn, mean - sd * 1.6);
  let hi = min(mx, mean + sd * 1.6);
  if (G.cam_delta.w < 0.5) { return cur; }
  // Reproject through the distance of the cloud this pixel sees.
  var dist = cd.x * 1000.0;
  if (dist <= 0.0) { dist = 60000.0; }
  let P = normalize(view_ray(in.uv)) * dist;
  let pc = G.prev_vp * vec4f(P + G.cam_delta.xyz, 1.0);
  if (pc.w <= 0.0) { return cur; }
  let puv = pc.xy / pc.w * vec2f(0.5, -0.5) + 0.5;
  if (any(puv < vec2f(0.0)) || any(puv > vec2f(1.0))) { return cur; }
  let hist = clamp(textureSampleLevel(t_hist, s_clamp, puv, 0.0), lo, hi);
  return mix(hist, cur, 0.1);
}

// Upsample onto the scene: out = cloud + scene * transmittance (done by the blend state).
// Joint bilateral: the four nearest texels are weighted by bilinear position and by
// how well the scene distance they marched to matches the wanted one, falling back to
// the best-matching texel when none agrees.
@group(1) @binding(1) var t_cloud: texture_2d<f32>;
@group(1) @binding(5) var t_color: texture_2d<f32>;
fn cloud_at(pos: vec2f, d: f32) -> vec4f {
  let dims = vec2i(textureDimensions(t_cloud));
  let f = pos * 0.5 - 0.5;
  let b = vec2i(floor(f));
  let fr = f - floor(f);
  var acc = vec4f(0.0);
  var wsum = 0.0;
  var best = vec4f(0.0, 0.0, 0.0, 1.0);
  var bestw = -1.0;
  for (var j = 0; j < 2; j++) {
    for (var i = 0; i < 2; i++) {
      let c = clamp(b + vec2i(i, j), vec2i(0), dims - 1);
      let s = textureLoad(t_cloud, c, 0);
      let m = depth_match(textureLoad(t_cdist, c, 0).g, d);
      let bw = select(1.0 - fr.x, fr.x, i == 1) * select(1.0 - fr.y, fr.y, j == 1);
      if (m > bestw) { bestw = m; best = s; }
      acc += s * (bw * m);
      wsum += bw * m;
    }
  }
  return (acc + best * 0.01) / (wsum + 0.01);
}
@fragment fn fs_apply(in: FsIn) -> @location(0) vec4f {
  let px = vec2i(in.pos.xy);
  let z = textureLoad(t_depth, px, 0);
  // The composite filled the part of an anti-aliased edge that geometry does not
  // cover (1 - alpha) with sky; that part needs the clouds in front of the sky,
  // the covered part those in front of the geometry.
  let cov = textureLoad(t_color, px, 0).a;
  if (z == 0.0 || cov < 0.004) { return cloud_at(in.pos.xy, SKY_KM); }
  let ray = view_ray(in.uv);
  let cg = cloud_at(in.pos.xy, min(length(ray / z) * 0.001, SKY_KM));
  if (cov > 0.996) { return cg; }
  let cs = cloud_at(in.pos.xy, SKY_KM);
  // Same sky (and jitter) the composite blended in. With dst = cov*G + (1-cov)*S,
  // dst*cg.a + L = cov*(G*cg.a + cg.rgb) + (1-cov)*(S*cs.a + cs.rgb).
  let S = clamp(sky_color(normalize(ray), ign(in.pos.xy)), vec3f(0.0), vec3f(3000.0));
  return vec4f(cov * cg.rgb + (1.0 - cov) * (cs.rgb + S * (cs.a - cg.a)), cg.a);
}
