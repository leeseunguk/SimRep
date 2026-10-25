// WGSL for the viewport. Frame uniforms are shared by every pipeline (group 0).
const frameStruct = /* wgsl */ `
struct Frame {
  viewProj: mat4x4f,
  eye: vec4f,        // xyz camera position, w = 1 for orthographic
  back: vec4f,       // unit vector target -> eye
  right: vec4f,
  up: vec4f,
  gridColor: vec4f,  // sRGB rgb, w = line opacity
  gridParams: vec4f, // x = log10 spacing (LOD), y = fade radius, zw = grid centre (x, z)
  axisX: vec4f,      // sRGB
  axisZ: vec4f,      // sRGB
  axisY: vec4f,      // sRGB
  selection: vec4f,  // linear
  sky: vec4f,        // linear
  ground: vec4f,     // linear
};
@group(0) @binding(0) var<uniform> frame: Frame;

fn viewVector(world: vec3f) -> vec3f {
  if (frame.eye.w > 0.5) { return frame.back.xyz; }
  return normalize(frame.eye.xyz - world);
}
`;

export const FRAME_FLOATS = 64;

// Studio lighting tied to the camera (key from upper left, soft fill from the right), two-sided
// because STL winding and voxel faces are not consistently oriented. base is linear rgb.
const shading = /* wgsl */ `
fn shade(base: vec3f, normal: vec3f, world: vec3f, selected: bool) -> vec4f {
  let v = viewVector(world);
  var n = normal;
  let len = length(n);
  if (len < 1e-5) { n = v; } else { n = n / len; }
  if (dot(n, v) < 0.0) { n = -n; }

  let key = normalize(v + frame.up.xyz * 0.9 - frame.right.xyz * 0.7);
  let fill = normalize(v - frame.up.xyz * 0.2 + frame.right.xyz * 0.9);
  let h = normalize(key + v);
  let diffuse = max(dot(n, key), 0.0) * 0.8 + max(dot(n, fill), 0.0) * 0.22;
  let spec = pow(max(dot(n, h), 0.0), 48.0) * 0.28;
  let hemi = mix(frame.ground.rgb, frame.sky.rgb, n.y * 0.5 + 0.5);
  let rim = pow(1.0 - max(dot(n, v), 0.0), 3.0);
  var color = base * (hemi * 0.22 + diffuse) + vec3f(spec) + base * rim * 0.12;
  if (selected) {
    color = mix(color, frame.selection.rgb, 0.2) + frame.selection.rgb * rim * 0.9;
  }
  let srgb = pow(clamp(color, vec3f(0.0), vec3f(1.0)), vec3f(1.0 / 2.2));
  return vec4f(srgb, 1.0);
}
`;

export const meshShader = /* wgsl */ `
${frameStruct}
${shading}

struct Object {
  model: mat4x4f,
  color: vec4f,   // linear rgb
  params: vec4f,  // x = selected
};
@group(1) @binding(0) var<uniform> object: Object;

struct VsOut {
  @builtin(position) clip: vec4f,
  @location(0) world: vec3f,
  @location(1) normal: vec3f,
};

@vertex
fn vs(@location(0) position: vec3f, @location(1) normal: vec3f) -> VsOut {
  var out: VsOut;
  let world = object.model * vec4f(position, 1.0);
  out.clip = frame.viewProj * world;
  out.world = world.xyz;
  out.normal = normal; // models only carry uniform scale, so directions are unchanged
  return out;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4f {
  return shade(object.color.rgb, in.normal, in.world, object.params.x > 0.5);
}
`;

export const gridShader = /* wgsl */ `
${frameStruct}

struct GridOut {
  @builtin(position) clip: vec4f,
  @location(0) world: vec3f,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> GridOut {
  var corners = array<vec2f, 6>(
    vec2f(-1.0, -1.0), vec2f(1.0, -1.0), vec2f(1.0, 1.0),
    vec2f(-1.0, -1.0), vec2f(1.0, 1.0), vec2f(-1.0, 1.0),
  );
  let c = corners[i];
  let r = frame.gridParams.y;
  let world = vec3f(frame.gridParams.z + c.x * r, 0.0, frame.gridParams.w + c.y * r);
  var out: GridOut;
  out.clip = frame.viewProj * vec4f(world, 1.0);
  out.world = world;
  return out;
}

fn lines(coord: vec2f, spacing: f32) -> f32 {
  let c = coord / spacing;
  let w = max(fwidth(c), vec2f(1e-6));
  let g = abs(fract(c - 0.5) - 0.5) / w;
  let line = 1.0 - min(min(g.x, g.y), 1.0);
  // Fade a level out before its cells get denser than a few pixels (no moire).
  return line * (1.0 - smoothstep(0.08, 0.25, max(w.x, w.y)));
}

fn axisLine(d: f32) -> f32 {
  let w = max(fwidth(d), 1e-6);
  return 1.0 - min(abs(d) / (w * 1.25), 1.0);
}

@fragment
fn fs(in: GridOut) -> @location(0) vec4f {
  let xz = in.world.xz;
  let lod = frame.gridParams.x;
  let k = floor(lod);
  let f = lod - k;
  // Three decades of lines; weights blend continuously as the camera zooms.
  let l0 = lines(xz, pow(10.0, k)) * clamp((1.0 - f) * 0.5, 0.0, 1.0);
  let l1 = lines(xz, pow(10.0, k + 1.0)) * clamp((2.0 - f) * 0.5, 0.0, 1.0);
  let l2 = lines(xz, pow(10.0, k + 2.0));
  let ax = axisLine(in.world.z); // X axis lies on z = 0
  let az = axisLine(in.world.x); // Z axis lies on x = 0

  var color = frame.gridColor.rgb;
  var alpha = max(max(l0, l1), l2) * frame.gridColor.w;
  color = mix(color, frame.axisX.rgb, ax);
  alpha = mix(alpha, 0.9, ax);
  color = mix(color, frame.axisZ.rgb, az);
  alpha = mix(alpha, 0.9, az);

  let toEye = viewVector(in.world);
  let grazing = smoothstep(0.02, 0.3, abs(toEye.y));
  let dist = length(xz - frame.gridParams.zw);
  let radial = 1.0 - smoothstep(frame.gridParams.y * 0.3, frame.gridParams.y, dist);
  alpha = alpha * grazing * radial;
  return vec4f(color * alpha, alpha);
}
`;

export const axisShader = /* wgsl */ `
${frameStruct}

struct AxisOut {
  @builtin(position) clip: vec4f,
  @location(0) along: f32,
  @location(1) @interpolate(flat) axis: u32,
};

// X, Y, Z axis lines through the origin (2 vertices each). The grid also draws X/Z on its plane,
// but that fades at grazing angles, so the lines keep the axes visible in front/side views.
@vertex
fn vs(@builtin(vertex_index) i: u32) -> AxisOut {
  let axis = i / 2u;
  let s = select(-1.0, 1.0, (i % 2u) == 1u);
  var dir = vec3f(0.0);
  dir[axis] = 1.0;
  var out: AxisOut;
  out.clip = frame.viewProj * vec4f(dir * s * frame.gridParams.y * 0.35, 1.0);
  out.along = s;
  out.axis = axis;
  return out;
}

@fragment
fn fs(in: AxisOut) -> @location(0) vec4f {
  var color = frame.axisX.rgb;
  if (in.axis == 1u) { color = frame.axisY.rgb; }
  if (in.axis == 2u) { color = frame.axisZ.rgb; }
  let fade = 1.0 - smoothstep(0.3, 1.0, abs(in.along));
  let alpha = 0.85 * fade * select(0.4, 1.0, in.along > 0.0);
  return vec4f(color * alpha, alpha);
}
`;

// Electrode colour for a potential: neutral metal tinted towards the pole colour (linear rgb).
const electrodeColor = /* wgsl */ `
fn electrodeColor(v: f32, fullScale: f32, neutral: vec3f, positive: vec3f, negative: vec3f) -> vec3f {
  let amount = pow(min(abs(v) / max(fullScale, 1e-30), 1.0), 0.7);
  return mix(neutral, select(negative, positive, v >= 0.0), amount);
}
`;

/** 7 vec4f + 8 vec4u (hidden-electrode bits), 4-byte words. */
export const PA_UNIFORM_FLOATS = 76;
/** Offset of the model matrix (array mm to workbench mm, see scene/placement.ts). */
export const PA_MODEL_OFFSET = 60;
/** Offset of the hidden-electrode bit words; labels 0..HIDDEN_LABELS-1 can be hidden. */
export const PA_HIDDEN_OFFSET = 28;
export const HIDDEN_LABELS = 1024;

/**
 * Potential-array electrodes: one instanced quad per surface face (4 corners + potential + label).
 * One electrode (label) can be highlighted and any can be hidden (bit set per label).
 * Mirrored copies reuse the same buffer with a reflection; faces on the camera side of the
 * section plane are cut away so the section stays visible.
 */
export const paSurfaceShader = /* wgsl */ `
${frameStruct}
${shading}
${electrodeColor}

struct Pa {
  mirror: vec4f,    // xyz reflection signs, w = potential sign (anti-mirrors)
  neutral: vec4f,   // linear electrode colours
  positive: vec4f,
  negative: vec4f,
  params: vec4f,    // x = |V| at full colour, y = selected, z = cut on, w = cut side (+-1)
  cut: vec4f,       // x = axis (0, 1, 2), y = plane position (mm)
  electrode: vec4f, // x = highlighted electrode label (-1 = none)
  hidden: array<vec4u, 8>, // bit per electrode label
  model: mat4x4f,   // array mm -> workbench mm (placement: rotation, uniform scale, translation)
};
@group(1) @binding(0) var<uniform> pa: Pa;

struct VsOut {
  @builtin(position) clip: vec4f,
  @location(0) world: vec3f,
  @location(1) normal: vec3f,
  @location(2) @interpolate(flat) color: vec3f,
  @location(3) @interpolate(flat) selected: f32,
  @location(4) local: vec3f,
};

fn isHidden(label: u32) -> bool {
  let word = label / 32u;
  if (word >= 32u) { return false; }
  return ((pa.hidden[word / 4u][word % 4u] >> (label % 32u)) & 1u) == 1u;
}

@vertex
fn vs(
  @builtin(vertex_index) i: u32,
  @location(0) p0: vec3f,
  @location(1) p1: vec3f,
  @location(2) p2: vec3f,
  @location(3) p3: vec3f,
  @location(4) potential: f32,
  @location(5) label: f32,
) -> VsOut {
  var corners = array<vec3f, 4>(p0, p1, p2, p3);
  var order = array<u32, 6>(0u, 1u, 2u, 0u, 2u, 3u);
  let s = pa.mirror.xyz;
  let local = corners[order[i]] * s;
  let world = (pa.model * vec4f(local, 1.0)).xyz;
  var out: VsOut;
  out.clip = frame.viewProj * vec4f(world, 1.0);
  out.world = world;
  out.local = local;
  out.normal = (pa.model * vec4f(cross(p2 - p0, p3 - p1) * s, 0.0)).xyz;
  out.color = electrodeColor(potential * pa.mirror.w, pa.params.x, pa.neutral.rgb, pa.positive.rgb, pa.negative.rgb);
  let highlighted = pa.electrode.x >= 0.0 && abs(label - pa.electrode.x) < 0.5;
  out.selected = select(0.0, 1.0, pa.params.y > 0.5 || highlighted);
  if (isHidden(u32(label + 0.5))) {
    out.clip = vec4f(2.0, 2.0, 2.0, 1.0); // every corner outside the clip volume: nothing drawn
  }
  return out;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4f {
  if (pa.params.z > 0.5 && (in.local[u32(pa.cut.x)] - pa.cut.y) * pa.params.w > 0.0) {
    discard;
  }
  return shade(in.color, in.normal, in.world, in.selected > 0.5);
}
`;

export const SLICE_UNIFORM_FLOATS = 68;

/**
 * Potential section: a quad over the displayed array extent. The fragment maps its in-plane
 * position (mm) back into the stored quadrant (mirrors, anti-mirror sign), interpolates the
 * rg32float slice (r = V, g = electrode) bilinearly, colours it with the field colour map and
 * draws anti-aliased equipotential lines. Must match sampleSlice() in scene/pa.ts.
 */
export const sliceShader = /* wgsl */ `
${frameStruct}
${electrodeColor}

struct Slice {
  origin: vec4f,     // world position of (uMin, vMin)
  uSpan: vec4f,      // world vector across u; w = uMin (mm)
  vSpan: vec4f,      // world vector across v; w = vMin (mm)
  grid: vec4f,       // xy = mm per texel along u, v; zw = texture size
  mirror: vec4f,     // xy = mirrored at negative u, v; zw = anti-mirrored (sign flips)
  range: vec4f,      // x = V at colour-map start, y = V at end, z = line step (0 = off), w = |V| at full electrode colour
  params: vec4f,     // x = selected, y = placement scale (world length per array mm)
  neutral: vec4f,    // linear electrode colours
  positive: vec4f,
  negative: vec4f,
  lineDark: vec4f,   // linear equipotential line colours (on light / dark map colours)
  lineLight: vec4f,
  stops: array<vec4f, 5>, // linear colour map
};
@group(1) @binding(0) var<uniform> slice: Slice;
@group(1) @binding(1) var data: texture_2d<f32>;

struct VsOut {
  @builtin(position) clip: vec4f,
  @location(0) uv: vec2f,     // in-plane position, mm
  @location(1) local: vec2f,  // 0..1 across the quad
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
  var corners = array<vec2f, 6>(
    vec2f(0.0, 0.0), vec2f(1.0, 0.0), vec2f(1.0, 1.0),
    vec2f(0.0, 0.0), vec2f(1.0, 1.0), vec2f(0.0, 1.0),
  );
  let c = corners[i];
  let world = slice.origin.xyz + slice.uSpan.xyz * c.x + slice.vSpan.xyz * c.y;
  var out: VsOut;
  out.clip = frame.viewProj * vec4f(world, 1.0);
  out.uv = vec2f(slice.uSpan.w, slice.vSpan.w) + c * vec2f(length(slice.uSpan.xyz), length(slice.vSpan.xyz)) / max(slice.params.y, 1e-12);
  out.local = c;
  return out;
}

fn texel(i: i32, j: i32) -> vec2f {
  let size = vec2i(slice.grid.zw);
  return textureLoad(data, clamp(vec2i(i, j), vec2i(0), size - vec2i(1)), 0).rg;
}

fn colormap(t: f32) -> vec3f {
  let x = clamp(t, 0.0, 1.0) * 4.0;
  let k = min(u32(x), 3u);
  return mix(slice.stops[k].rgb, slice.stops[k + 1u].rgb, x - f32(k));
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4f {
  let flipU = slice.mirror.x > 0.5 && in.uv.x < 0.0;
  let flipV = slice.mirror.y > 0.5 && in.uv.y < 0.0;
  let uv = vec2f(select(in.uv.x, -in.uv.x, flipU), select(in.uv.y, -in.uv.y, flipV));
  let sign = select(1.0, -1.0, flipU && slice.mirror.z > 0.5) * select(1.0, -1.0, flipV && slice.mirror.w > 0.5);

  let g = clamp(uv / slice.grid.xy, vec2f(0.0), slice.grid.zw - vec2f(1.0));
  let i0 = vec2i(floor(g));
  let f = g - floor(g);
  let v00 = texel(i0.x, i0.y).r;
  let v10 = texel(i0.x + 1, i0.y).r;
  let v01 = texel(i0.x, i0.y + 1).r;
  let v11 = texel(i0.x + 1, i0.y + 1).r;
  let value = mix(mix(v00, v10, f.x), mix(v01, v11, f.x), f.y) * sign;
  let near = texel(i32(round(g.x)), i32(round(g.y)));

  // Equipotential lines: distance to the nearest multiple of the step, in screen pixels.
  let step = slice.range.z;
  let level = value / max(step, 1e-30);
  let w = max(fwidth(level), 1e-6);
  let dist = abs(fract(level + 0.5) - 0.5) / w;
  // No lines on a flat field (e.g. the unsolved space of a .pa#): it would sit on one level everywhere.
  let lineOn = select(0.0, 1.0, step > 0.0) * smoothstep(1e-5, 1e-4, w);
  // Fade lines out where they would be closer than ~3 px (steep gradients) instead of aliasing.
  let line = (1.0 - smoothstep(0.6, 1.4, dist)) * (1.0 - smoothstep(0.2, 0.35, w)) * lineOn;

  // Outline of the section quad, in pixels.
  let lw = max(fwidth(in.local), vec2f(1e-6));
  let edgePx = min(min(in.local.x, 1.0 - in.local.x) / lw.x, min(in.local.y, 1.0 - in.local.y) / lw.y);
  let edge = 1.0 - smoothstep(0.5, 1.5, edgePx);

  let t = (value - slice.range.x) / max(slice.range.y - slice.range.x, 1e-30);
  var color = colormap(t);
  let lum = dot(color, vec3f(0.2126, 0.7152, 0.0722));
  color = mix(color, select(slice.lineLight.rgb, slice.lineDark.rgb, lum > 0.2), line * 0.7);
  let electrode = electrodeColor(near.r * sign, slice.range.w, slice.neutral.rgb, slice.positive.rgb, slice.negative.rgb);
  color = select(color, electrode, near.g > 0.5);
  // Selected electrode (g = 2): tinted with the selection colour.
  color = select(color, mix(electrode, frame.selection.rgb, 0.6), near.g > 1.5);
  let edgeColor = select(slice.lineDark.rgb, frame.selection.rgb, slice.params.x > 0.5);
  color = mix(color, edgeColor, edge * select(0.5, 1.0, slice.params.x > 0.5));

  let srgb = pow(clamp(color, vec3f(0.0), vec3f(1.0)), vec3f(1.0 / 2.2));
  return vec4f(srgb, 1.0);
}
`;

export const BENCH_UNIFORM_FLOATS = 4;

/** Workbench boxes: line list in workbench mm, one sRGB + alpha colour, nudged like trajectories. */
export const benchShader = /* wgsl */ `
${frameStruct}

struct Bench {
  color: vec4f, // sRGB + alpha
};
@group(1) @binding(0) var<uniform> bench: Bench;

@vertex
fn vs(@location(0) p: vec3f) -> @builtin(position) vec4f {
  let c = frame.viewProj * vec4f(p, 1.0);
  var out = c;
  if (frame.eye.w > 0.5) { out.z = c.z + 2e-4 * c.w; } else { out.z = c.z * 1.002; }
  return out;
}

@fragment
fn fs() -> @location(0) vec4f {
  return vec4f(bench.color.rgb * bench.color.a, bench.color.a);
}
`;

export const TRAJECTORY_UNIFORM_FLOATS = 24;

/**
 * Ion trajectories (line strips, one per ion, split by primitive restart) and splat markers
 * (screen-sized discs at each ion's end point). Colours are sRGB with alpha, blended
 * premultiplied. Trajectories often lie in the section plane (z = 0 for 2D arrays), so depth is
 * nudged towards the camera to keep them in front of the coplanar section.
 */
export const trajectoryShader = /* wgsl */ `
${frameStruct}

struct Traj {
  line: vec4f,     // sRGB + alpha
  hit: vec4f,      // splat on an electrode
  splat: vec4f,    // other splats
  params: vec4f,   // xy = target size (px), z = splat radius (px), w = outline darkness
  sel: vec4f,      // selected ion highlight colour
  selParams: vec4f, // x = highlight line width (px), y = end ring radius (px)
};
@group(1) @binding(0) var<uniform> traj: Traj;

fn nudge(c: vec4f) -> vec4f {
  var out = c;
  if (frame.eye.w > 0.5) { out.z = c.z + 2e-4 * c.w; } else { out.z = c.z * 1.002; }
  return out;
}

@vertex
fn vsLine(@location(0) p: vec3f) -> @builtin(position) vec4f {
  return nudge(frame.viewProj * vec4f(p, 1.0));
}

@fragment
fn fsLine() -> @location(0) vec4f {
  return vec4f(traj.line.rgb * traj.line.a, traj.line.a);
}

struct SplatOut {
  @builtin(position) clip: vec4f,
  @location(0) uv: vec2f,
  @location(1) @interpolate(flat) hit: f32,
};

@vertex
fn vsSplat(@builtin(vertex_index) i: u32, @location(0) end: vec4f) -> SplatOut {
  var corners = array<vec2f, 6>(
    vec2f(-1.0, -1.0), vec2f(1.0, -1.0), vec2f(1.0, 1.0),
    vec2f(-1.0, -1.0), vec2f(1.0, 1.0), vec2f(-1.0, 1.0),
  );
  let c = corners[i];
  var clip = nudge(frame.viewProj * vec4f(end.xyz, 1.0));
  clip.x += c.x * traj.params.z * 2.0 / traj.params.x * clip.w;
  clip.y += c.y * traj.params.z * 2.0 / traj.params.y * clip.w;
  var out: SplatOut;
  out.clip = clip;
  out.uv = c;
  out.hit = end.w;
  return out;
}

@fragment
fn fsSplat(in: SplatOut) -> @location(0) vec4f {
  let r = length(in.uv);
  let w = max(fwidth(r), 1e-4);
  let alpha = 1.0 - smoothstep(1.0 - w, 1.0, r);
  if (alpha <= 0.0) { discard; }
  let fill = select(traj.splat, traj.hit, in.hit > 0.5);
  // Thin dark rim so markers read on both light and bright colour-map areas.
  let rim = smoothstep(0.62 - w, 0.62, r);
  let color = mix(fill.rgb, fill.rgb * traj.params.w, rim);
  let a = alpha * fill.a;
  return vec4f(color * a, a);
}

// Selected ion: its trajectory as a screen-space ribbon (one instance per segment) and a ring at
// its end point, drawn over everything.
struct ThickOut {
  @builtin(position) clip: vec4f,
  @location(0) side: f32,
};

@vertex
fn vsThick(@builtin(vertex_index) i: u32, @location(0) a: vec3f, @location(1) b: vec3f) -> ThickOut {
  var out: ThickOut;
  let ca = nudge(frame.viewProj * vec4f(a, 1.0));
  let cb = nudge(frame.viewProj * vec4f(b, 1.0));
  if (ca.w <= 0.0 || cb.w <= 0.0) {
    out.clip = vec4f(2.0, 2.0, 2.0, 1.0);
    out.side = 0.0;
    return out;
  }
  let size = traj.params.xy;
  let sa = ca.xy / ca.w * size;
  let sb = cb.xy / cb.w * size;
  var d = sb - sa;
  if (length(d) < 1e-4) { d = vec2f(1.0, 0.0); }
  let n = normalize(vec2f(-d.y, d.x));
  var corners = array<vec2f, 6>(
    vec2f(0.0, -1.0), vec2f(1.0, -1.0), vec2f(1.0, 1.0),
    vec2f(0.0, -1.0), vec2f(1.0, 1.0), vec2f(0.0, 1.0),
  );
  let c = corners[i];
  var p = select(ca, cb, c.x > 0.5);
  // Half the width on each side, px -> clip space (2 / size per px).
  let off = n * c.y * traj.selParams.x / size;
  p = vec4f(p.xy + off * p.w, p.z, p.w);
  out.clip = p;
  out.side = c.y;
  return out;
}

@fragment
fn fsThick(in: ThickOut) -> @location(0) vec4f {
  let w = max(fwidth(in.side), 1e-4);
  let a = (1.0 - smoothstep(1.0 - w, 1.0, abs(in.side))) * traj.sel.a;
  return vec4f(traj.sel.rgb * a, a);
}

@vertex
fn vsRing(@builtin(vertex_index) i: u32, @location(0) end: vec4f) -> SplatOut {
  var corners = array<vec2f, 6>(
    vec2f(-1.0, -1.0), vec2f(1.0, -1.0), vec2f(1.0, 1.0),
    vec2f(-1.0, -1.0), vec2f(1.0, 1.0), vec2f(-1.0, 1.0),
  );
  let c = corners[i];
  var clip = nudge(frame.viewProj * vec4f(end.xyz, 1.0));
  clip.x += c.x * traj.selParams.y * 2.0 / traj.params.x * clip.w;
  clip.y += c.y * traj.selParams.y * 2.0 / traj.params.y * clip.w;
  var out: SplatOut;
  out.clip = clip;
  out.uv = c;
  out.hit = 0.0;
  return out;
}

@fragment
fn fsRing(in: SplatOut) -> @location(0) vec4f {
  let r = length(in.uv);
  let w = max(fwidth(r), 1e-4);
  let ring = smoothstep(0.62 - w, 0.62, r) * (1.0 - smoothstep(1.0 - w, 1.0, r));
  if (ring <= 0.0) { discard; }
  let a = ring * traj.sel.a;
  return vec4f(traj.sel.rgb * a, a);
}
`;
