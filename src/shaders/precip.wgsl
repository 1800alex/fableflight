// Weather close up, drawn over the lit scene and the clouds (behind the glass):
// rain falling around the camera, as streaks blurred along their motion
// relative to the camera, and lightning channels (storm.rs). Both are thin
// lines drawn as screen-space quads a few pixels wide, tested against the
// scene's depth and dimmed by the clouds and rain shafts in front of them.
// They write how much they cover into alpha, which taa.wgsl reads.
@group(1) @binding(0) var t_depth: texture_depth_2d;
@group(1) @binding(1) var t_cloud: texture_2d<f32>;
@group(1) @binding(3) var t_cdist: texture_2d<f32>;

// Exposure time the rain streaks are blurred over, s (about the eye's 1/80 s).
const SHUTTER = 0.009;
// Drops are spread over three nested boxes around the camera (render.rs
// RAIN_DROPS instances; the box sizes must divide storm.rs WRAP).
const RAIN_N0 = 4096u;
const RAIN_N1 = 10240u;

fn pcg(v: u32) -> u32 {
  let s = v * 747796405u + 2891336453u;
  let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
  return (w >> 22u) ^ w;
}
fn u01(h: u32) -> f32 { return f32(h & 0xffffffu) / 16777216.0; }

// How much of what lies d metres along this pixel's ray still shows through the
// clouds and rain shafts drawn over the scene: the cloud pass keeps their
// transmittance and (weighted) distance at half resolution.
fn cloud_cover(px: vec2f, d: f32) -> f32 {
  let dims = vec2i(textureDimensions(t_cloud)) - 1;
  let c = clamp(vec2i(px * 0.5), vec2i(0), dims);
  let T = textureLoad(t_cloud, c, 0).a;
  let dc = textureLoad(t_cdist, c, 0).r * 1000.0;
  if (dc <= 0.0) { return 1.0; }
  return mix(T, 1.0, smoothstep(-0.12, 0.12, (dc - d) / max(d, 1.0)));
}

// A line from a to b (camera-relative) as a quad around it on screen: half
// width hw pixels, also reaching hw past each end. Corner vi of 6. Returns the
// clip position, and in q: pixels across, pixels along (from a), length in pixels.
struct Line { clip: vec4f, q: vec3f, ok: bool };
fn line_quad(a_in: vec3f, b_in: vec3f, hw: f32, vi: u32) -> Line {
  var o: Line;
  o.ok = false;
  var a = a_in;
  var b = b_in;
  // Cut off at a plane just in front of the camera.
  let wn = G.misc.x * 2.0;
  var ca = G.view_proj * vec4f(a, 1.0);
  var cb = G.view_proj * vec4f(b, 1.0);
  if (ca.w < wn && cb.w < wn) { return o; }
  if (ca.w < wn) { a = mix(a, b, (wn - ca.w) / (cb.w - ca.w)); ca = G.view_proj * vec4f(a, 1.0); }
  if (cb.w < wn) { b = mix(b, a, (wn - cb.w) / (ca.w - cb.w)); cb = G.view_proj * vec4f(b, 1.0); }
  let half = G.screen.xy * 0.5;
  let sa = ca.xy / ca.w * half;
  let sb = cb.xy / cb.w * half;
  let dv = sb - sa;
  let len = length(dv);
  var dir = vec2f(0.0, 1.0);
  if (len > 1e-3) { dir = dv / len; }
  let perp = vec2f(-dir.y, dir.x);
  let corners = array<vec2f, 6>(vec2f(-1.0, 0.0), vec2f(1.0, 0.0), vec2f(1.0, 1.0), vec2f(-1.0, 0.0), vec2f(1.0, 1.0), vec2f(-1.0, 1.0));
  let c = corners[vi];
  var clip = mix(ca, cb, c.y);
  let off = perp * (c.x * hw) + dir * ((c.y * 2.0 - 1.0) * hw);
  clip = vec4f(clip.xy + off / half * clip.w, clip.zw);
  o.clip = clip;
  o.q = vec3f(c.x * hw, c.y * len + (c.y * 2.0 - 1.0) * hw, len);
  o.ok = true;
  return o;
}

// ---------------------------------------------------------------- rain
struct RainOut {
  @builtin(position) clip: vec4f,
  // pixels across, along, the drop's distance (m)
  @location(0) q: vec3f,
  // half the drop's width (px), streak length (px), fade
  @location(1) @interpolate(flat) p: vec3f,
  @location(2) @interpolate(flat) col: vec4f,
};

@vertex fn vs_rain(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> RainOut {
  var o: RainOut;
  o.clip = vec4f(2.0, 2.0, 2.0, 1.0);
  let inten = G.rain[0].w;
  let h0 = pcg(ii * 2654435761u + 7u);
  // The heavier the rain, the more drops (and the bigger).
  if (u01(h0) > inten) { return o; }
  var B = 50.0;
  if (ii < RAIN_N0) { B = 8.0; } else if (ii < RAIN_N1) { B = 20.0; }
  let h1 = pcg(h0);
  let h2 = pcg(h1);
  let h3 = pcg(h2);
  let h4 = pcg(h3);
  let l = (fract(vec3f(u01(h1), u01(h2), u01(h3)) + G.rain[0].xyz / B) - 0.5) * B;
  let P = G.east.xyz * l.x + G.north.xyz * l.y + G.up.xyz * l.z;
  let dist = length(P);
  let near = G.rain[1].w;
  // Each box fills a sphere (its corners would show as a cube of rain).
  let fade = smoothstep(near, near * 1.6 + 1.0, dist) * (1.0 - smoothstep(0.3 * B, 0.5 * B, dist)) * (0.35 + 0.65 * u01(pcg(h4)));
  if (fade <= 0.0) { return o; }
  let D = mix(0.0007, 0.004, u01(h4) * u01(h4)) * (0.6 + 0.6 * inten);
  // Blurred along its motion relative to the camera over the exposure: at
  // flying speeds the drops rush at the camera as long streaks.
  let v = G.rain[1].xyz;
  let ls = length(v) * SHUTTER;
  let a = P - v * (SHUTTER * 0.5);
  let b = P + v * (SHUTTER * 0.5);
  let core = D / (dist * G.misc.w);
  let wq = max(core, 1.0);
  let ln = line_quad(a, b, wq * 0.5 + 1.0, vi);
  if (!ln.ok) { return o; }
  // Light: a drop shows a little fish-eye view of everything around it, mostly
  // the sky light (dim under a storm, bright in a lightning flash) and a glint
  // of the sun when looking towards it.
  let lt = light_at_nv(P);
  let vd = P / max(dist, 1e-3);
  let glint = pow(max(dot(vd, G.sun_dir.xyz), 0.0), 6.0);
  let L = lt.amb * 3.5 + lt.sun * (0.012 + 0.08 * glint);
  // The drop covers only so much of its streak and of a pixel's width; a
  // little extra keeps fast streaks from fading out altogether.
  let alpha = clamp(4.5 * (core / wq) * 0.02 / (0.5 * ls + 0.02), 0.0, 0.4) * fade;
  o.clip = ln.clip;
  o.q = vec3f(ln.q.xy, dist);
  o.p = vec3f(wq * 0.5, ln.q.z, 0.0);
  o.col = vec4f(L, alpha);
  return o;
}

@fragment fn fs_rain(in: RainOut) -> @location(0) vec4f {
  let across = clamp(in.p.x + 0.5 - abs(in.q.x), 0.0, 1.0);
  // Soft ends.
  let e = max(in.p.x * 2.0, 1.0);
  let along = clamp(min(in.q.y, in.p.y - in.q.y) / e + 0.5, 0.0, 1.0);
  let a = in.col.a * across * along * cloud_cover(in.clip.xy, in.q.z);
  return vec4f(in.col.rgb * a, a);
}

// ----------------------------------------------------------- lightning
// How bright a channel is on screen at brightness 1, relative to the exposure:
// far brighter than anything else, day or night (the bloom spreads its glow).
const BOLT_K = 8.0;

struct BoltOut {
  @builtin(position) clip: vec4f,
  // pixels across, along, the channel's distance (m)
  @location(0) q: vec3f,
  // half the core's width (px), half the quad's width (px), brightness
  @location(1) @interpolate(flat) p: vec3f,
};

@vertex fn vs_bolt(@builtin(vertex_index) vi: u32, @location(0) a: vec4f, @location(1) b: vec4f) -> BoltOut {
  var o: BoltOut;
  o.clip = vec4f(2.0, 2.0, 2.0, 1.0);
  let mid = 0.5 * (a.xyz + b.xyz);
  let dist = length(mid);
  // The core: its width in metres, but never thinner than a pixel and a half.
  let core = max(a.w / (dist * G.misc.w), 2.2) * 0.5;
  // A soft halo around it, wider the brighter.
  let hw = core + 3.0 + 10.0 * clamp(b.w, 0.0, 1.0);
  let ln = line_quad(a.xyz, b.xyz, hw, vi);
  if (!ln.ok) { return o; }
  o.clip = ln.clip;
  o.q = vec3f(ln.q.xy, dist);
  o.p = vec3f(core, hw, b.w);
  return o;
}

@fragment fn fs_bolt(in: BoltOut) -> @location(0) vec4f {
  let x = abs(in.q.x);
  let core = clamp(in.p.x + 0.6 - x, 0.0, 1.0);
  let halo = exp(-x / 2.0) * 0.1 + exp(-x / 7.0) * 0.02;
  // The haze between, and any cloud or rain in front.
  let ext = (RAYLEIGH.g + (MIE_S + MIE_A) * G.weather.w) * 0.8;
  let seen = exp(-in.q.z * ext) * cloud_cover(in.clip.xy, in.q.z);
  let b = in.p.z * seen;
  let col = (vec3f(0.92, 0.94, 1.0) * core + vec3f(0.62, 0.66, 1.0) * halo) * b * BOLT_K / max(G.time.z, 0.05);
  return vec4f(col, clamp(core * min(b * 30.0, 1.0), 0.0, 1.0));
}
