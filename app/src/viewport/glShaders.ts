// GLSL ES 3.0 ports of the WGSL passes in shaders.ts, for the WebGL2 fallback renderer.
// Uniform blocks use std140, which lays the structs out exactly like the WGSL uniform buffers, so
// both renderers fill the same Float32Arrays. Vertex shaders compute WebGPU clip space (z in
// [0, w], reversed Z) and convert it with toGl(): unchanged when EXT_clip_control sets the
// zero-to-one depth range, else z is remapped to [-w, w].

const header = (clipControl: boolean) => `#version 300 es
precision highp float;
precision highp int;
${clipControl ? "#define CLIP_ZERO_TO_ONE 1" : ""}
`;

const frameBlock = /* glsl */ `
layout(std140) uniform Frame {
  mat4 viewProj;
  vec4 eye;        // xyz camera position, w = 1 for orthographic
  vec4 back;       // unit vector target -> eye
  vec4 right;
  vec4 up;
  vec4 gridColor;  // sRGB rgb, w = line opacity
  vec4 gridParams; // x = log10 spacing (LOD), y = fade radius, zw = grid centre (x, z)
  vec4 axisX;      // sRGB
  vec4 axisZ;      // sRGB
  vec4 axisY;      // sRGB
  vec4 selection;  // linear
  vec4 sky;        // linear
  vec4 ground;     // linear
} frame;

vec3 viewVector(vec3 world) {
  if (frame.eye.w > 0.5) { return frame.back.xyz; }
  return normalize(frame.eye.xyz - world);
}

vec4 toGl(vec4 c) {
#ifdef CLIP_ZERO_TO_ONE
  return c;
#else
  return vec4(c.xy, 2.0 * c.z - c.w, c.w);
#endif
}

const vec2 QUAD[6] = vec2[6](vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0));
const vec2 UNIT_QUAD[6] = vec2[6](vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0));
`;

const shading = /* glsl */ `
vec4 shade(vec3 base, vec3 normal, vec3 world, bool selected) {
  vec3 v = viewVector(world);
  vec3 n = normal;
  float len = length(n);
  if (len < 1e-5) { n = v; } else { n = n / len; }
  if (dot(n, v) < 0.0) { n = -n; }

  vec3 key = normalize(v + frame.up.xyz * 0.9 - frame.right.xyz * 0.7);
  vec3 fill = normalize(v - frame.up.xyz * 0.2 + frame.right.xyz * 0.9);
  vec3 h = normalize(key + v);
  float diffuse = max(dot(n, key), 0.0) * 0.8 + max(dot(n, fill), 0.0) * 0.22;
  float spec = pow(max(dot(n, h), 0.0), 48.0) * 0.28;
  vec3 hemi = mix(frame.ground.rgb, frame.sky.rgb, n.y * 0.5 + 0.5);
  float rim = pow(1.0 - max(dot(n, v), 0.0), 3.0);
  vec3 color = base * (hemi * 0.22 + diffuse) + vec3(spec) + base * rim * 0.12;
  if (selected) {
    color = mix(color, frame.selection.rgb, 0.2) + frame.selection.rgb * rim * 0.9;
  }
  vec3 srgb = pow(clamp(color, vec3(0.0), vec3(1.0)), vec3(1.0 / 2.2));
  return vec4(srgb, 1.0);
}
`;

const electrodeColor = /* glsl */ `
vec3 electrodeColor(float v, float fullScale, vec3 neutral, vec3 positive, vec3 negative) {
  float amount = pow(min(abs(v) / max(fullScale, 1e-30), 1.0), 0.7);
  return mix(neutral, v >= 0.0 ? positive : negative, amount);
}
`;

export interface GlProgramSource {
  vs: string;
  fs: string;
}

export function glShaders(clipControl: boolean) {
  const h = header(clipControl);
  const common = h + frameBlock;
  const src = (vs: string, fs: string): GlProgramSource => ({ vs: common + vs, fs: common + fs });

  const mesh = src(
    /* glsl */ `
layout(std140) uniform Object { mat4 model; vec4 color; vec4 params; } object;
layout(location = 0) in vec3 position;
layout(location = 1) in vec3 normal;
out vec3 vWorld;
out vec3 vNormal;
void main() {
  vec4 world = object.model * vec4(position, 1.0);
  gl_Position = toGl(frame.viewProj * world);
  vWorld = world.xyz;
  vNormal = normal;
}`,
    /* glsl */ `
${shading}
layout(std140) uniform Object { mat4 model; vec4 color; vec4 params; } object;
in vec3 vWorld;
in vec3 vNormal;
out vec4 outColor;
void main() {
  outColor = shade(object.color.rgb, vNormal, vWorld, object.params.x > 0.5);
}`,
  );

  const grid = src(
    /* glsl */ `
out vec3 vWorld;
void main() {
  vec2 c = QUAD[gl_VertexID];
  float r = frame.gridParams.y;
  vec3 world = vec3(frame.gridParams.z + c.x * r, 0.0, frame.gridParams.w + c.y * r);
  gl_Position = toGl(frame.viewProj * vec4(world, 1.0));
  vWorld = world;
}`,
    /* glsl */ `
in vec3 vWorld;
out vec4 outColor;
float lines(vec2 coord, float spacing) {
  vec2 c = coord / spacing;
  vec2 w = max(fwidth(c), vec2(1e-6));
  vec2 g = abs(fract(c - 0.5) - 0.5) / w;
  float line = 1.0 - min(min(g.x, g.y), 1.0);
  return line * (1.0 - smoothstep(0.08, 0.25, max(w.x, w.y)));
}
float axisLine(float d) {
  float w = max(fwidth(d), 1e-6);
  return 1.0 - min(abs(d) / (w * 1.25), 1.0);
}
void main() {
  vec2 xz = vWorld.xz;
  float lod = frame.gridParams.x;
  float k = floor(lod);
  float f = lod - k;
  float l0 = lines(xz, pow(10.0, k)) * clamp((1.0 - f) * 0.5, 0.0, 1.0);
  float l1 = lines(xz, pow(10.0, k + 1.0)) * clamp((2.0 - f) * 0.5, 0.0, 1.0);
  float l2 = lines(xz, pow(10.0, k + 2.0));
  float ax = axisLine(vWorld.z);
  float az = axisLine(vWorld.x);
  vec3 color = frame.gridColor.rgb;
  float alpha = max(max(l0, l1), l2) * frame.gridColor.w;
  color = mix(color, frame.axisX.rgb, ax);
  alpha = mix(alpha, 0.9, ax);
  color = mix(color, frame.axisZ.rgb, az);
  alpha = mix(alpha, 0.9, az);
  vec3 toEye = viewVector(vWorld);
  float grazing = smoothstep(0.02, 0.3, abs(toEye.y));
  float dist = length(xz - frame.gridParams.zw);
  float radial = 1.0 - smoothstep(frame.gridParams.y * 0.3, frame.gridParams.y, dist);
  alpha = alpha * grazing * radial;
  outColor = vec4(color * alpha, alpha);
}`,
  );

  const axis = src(
    /* glsl */ `
out float vAlong;
flat out int vAxis;
void main() {
  int a = gl_VertexID / 2;
  float s = (gl_VertexID % 2) == 1 ? 1.0 : -1.0;
  vec3 dir = vec3(0.0);
  dir[a] = 1.0;
  gl_Position = toGl(frame.viewProj * vec4(dir * s * frame.gridParams.y * 0.35, 1.0));
  vAlong = s;
  vAxis = a;
}`,
    /* glsl */ `
in float vAlong;
flat in int vAxis;
out vec4 outColor;
void main() {
  vec3 color = frame.axisX.rgb;
  if (vAxis == 1) { color = frame.axisY.rgb; }
  if (vAxis == 2) { color = frame.axisZ.rgb; }
  float fade = 1.0 - smoothstep(0.3, 1.0, abs(vAlong));
  float alpha = 0.85 * fade * (vAlong > 0.0 ? 1.0 : 0.4);
  outColor = vec4(color * alpha, alpha);
}`,
  );

  const paBlock = /* glsl */ `
layout(std140) uniform Pa {
  vec4 mirror;    // xyz reflection signs, w = potential sign (anti-mirrors)
  vec4 neutral;   // linear electrode colours
  vec4 positive;
  vec4 negative;
  vec4 params;    // x = |V| at full colour, y = selected, z = cut on, w = cut side (+-1)
  vec4 cut;       // x = axis (0, 1, 2), y = plane position (mm)
  vec4 electrode; // x = highlighted electrode label (-1 = none)
  uvec4 hidden[8];
  mat4 model;
} pa;
`;

  const paSurface = src(
    /* glsl */ `
${electrodeColor}
${paBlock}
layout(location = 0) in vec3 p0;
layout(location = 1) in vec3 p1;
layout(location = 2) in vec3 p2;
layout(location = 3) in vec3 p3;
layout(location = 4) in float potential;
layout(location = 5) in float label;
out vec3 vWorld;
out vec3 vNormal;
flat out vec3 vColor;
flat out float vSelected;
out vec3 vLocal;
const int ORDER[6] = int[6](0, 1, 2, 0, 2, 3);
bool isHidden(uint l) {
  uint word = l / 32u;
  if (word >= 32u) { return false; }
  return ((pa.hidden[word / 4u][word % 4u] >> (l % 32u)) & 1u) == 1u;
}
void main() {
  int k = ORDER[gl_VertexID];
  vec3 corner = k == 0 ? p0 : k == 1 ? p1 : k == 2 ? p2 : p3;
  vec3 s = pa.mirror.xyz;
  vec3 local = corner * s;
  vec3 world = (pa.model * vec4(local, 1.0)).xyz;
  gl_Position = toGl(frame.viewProj * vec4(world, 1.0));
  vWorld = world;
  vLocal = local;
  vNormal = (pa.model * vec4(cross(p2 - p0, p3 - p1) * s, 0.0)).xyz;
  vColor = electrodeColor(potential * pa.mirror.w, pa.params.x, pa.neutral.rgb, pa.positive.rgb, pa.negative.rgb);
  bool highlighted = pa.electrode.x >= 0.0 && abs(label - pa.electrode.x) < 0.5;
  vSelected = (pa.params.y > 0.5 || highlighted) ? 1.0 : 0.0;
  if (isHidden(uint(label + 0.5))) {
    gl_Position = vec4(2.0, 2.0, 2.0, 1.0);
  }
}`,
    /* glsl */ `
${shading}
${paBlock}
in vec3 vWorld;
in vec3 vNormal;
flat in vec3 vColor;
flat in float vSelected;
in vec3 vLocal;
out vec4 outColor;
void main() {
  if (pa.params.z > 0.5 && (vLocal[int(pa.cut.x)] - pa.cut.y) * pa.params.w > 0.0) {
    discard;
  }
  outColor = shade(vColor, vNormal, vWorld, vSelected > 0.5);
}`,
  );

  const sliceBlock = /* glsl */ `
layout(std140) uniform Slice {
  vec4 origin;
  vec4 uSpan;
  vec4 vSpan;
  vec4 grid;
  vec4 mirror;
  vec4 range;
  vec4 params;
  vec4 neutral;
  vec4 positive;
  vec4 negative;
  vec4 lineDark;
  vec4 lineLight;
  vec4 stops[5];
} slice;
`;

  const sliceProgram = src(
    /* glsl */ `
${sliceBlock}
out vec2 vUv;
out vec2 vLocal;
void main() {
  vec2 c = UNIT_QUAD[gl_VertexID];
  vec3 world = slice.origin.xyz + slice.uSpan.xyz * c.x + slice.vSpan.xyz * c.y;
  gl_Position = toGl(frame.viewProj * vec4(world, 1.0));
  vUv = vec2(slice.uSpan.w, slice.vSpan.w) + c * vec2(length(slice.uSpan.xyz), length(slice.vSpan.xyz)) / max(slice.params.y, 1e-12);
  vLocal = c;
}`,
    /* glsl */ `
${electrodeColor}
${sliceBlock}
uniform highp sampler2D data;
in vec2 vUv;
in vec2 vLocal;
out vec4 outColor;
vec2 texel(int i, int j) {
  ivec2 size = ivec2(slice.grid.zw);
  return texelFetch(data, clamp(ivec2(i, j), ivec2(0), size - ivec2(1)), 0).rg;
}
vec3 colormap(float t) {
  float x = clamp(t, 0.0, 1.0) * 4.0;
  int k = min(int(x), 3);
  return mix(slice.stops[k].rgb, slice.stops[k + 1].rgb, x - float(k));
}
void main() {
  bool flipU = slice.mirror.x > 0.5 && vUv.x < 0.0;
  bool flipV = slice.mirror.y > 0.5 && vUv.y < 0.0;
  vec2 uv = vec2(flipU ? -vUv.x : vUv.x, flipV ? -vUv.y : vUv.y);
  float sgn = ((flipU && slice.mirror.z > 0.5) ? -1.0 : 1.0) * ((flipV && slice.mirror.w > 0.5) ? -1.0 : 1.0);

  vec2 g = clamp(uv / slice.grid.xy, vec2(0.0), slice.grid.zw - vec2(1.0));
  ivec2 i0 = ivec2(floor(g));
  vec2 f = g - floor(g);
  float v00 = texel(i0.x, i0.y).r;
  float v10 = texel(i0.x + 1, i0.y).r;
  float v01 = texel(i0.x, i0.y + 1).r;
  float v11 = texel(i0.x + 1, i0.y + 1).r;
  float value = mix(mix(v00, v10, f.x), mix(v01, v11, f.x), f.y) * sgn;
  vec2 near = texel(int(floor(g.x + 0.5)), int(floor(g.y + 0.5)));

  float stepV = slice.range.z;
  float level = value / max(stepV, 1e-30);
  float w = max(fwidth(level), 1e-6);
  float dist = abs(fract(level + 0.5) - 0.5) / w;
  float lineOn = (stepV > 0.0 ? 1.0 : 0.0) * smoothstep(1e-5, 1e-4, w);
  float line = (1.0 - smoothstep(0.6, 1.4, dist)) * (1.0 - smoothstep(0.2, 0.35, w)) * lineOn;

  vec2 lw = max(fwidth(vLocal), vec2(1e-6));
  float edgePx = min(min(vLocal.x, 1.0 - vLocal.x) / lw.x, min(vLocal.y, 1.0 - vLocal.y) / lw.y);
  float edge = 1.0 - smoothstep(0.5, 1.5, edgePx);

  float t = (value - slice.range.x) / max(slice.range.y - slice.range.x, 1e-30);
  vec3 color = colormap(t);
  float lum = dot(color, vec3(0.2126, 0.7152, 0.0722));
  color = mix(color, lum > 0.2 ? slice.lineDark.rgb : slice.lineLight.rgb, line * 0.7);
  vec3 electrode = electrodeColor(near.r * sgn, slice.range.w, slice.neutral.rgb, slice.positive.rgb, slice.negative.rgb);
  color = near.g > 0.5 ? electrode : color;
  color = near.g > 1.5 ? mix(electrode, frame.selection.rgb, 0.6) : color;
  vec3 edgeColor = slice.params.x > 0.5 ? frame.selection.rgb : slice.lineDark.rgb;
  color = mix(color, edgeColor, edge * (slice.params.x > 0.5 ? 1.0 : 0.5));
  vec3 srgb = pow(clamp(color, vec3(0.0), vec3(1.0)), vec3(1.0 / 2.2));
  outColor = vec4(srgb, 1.0);
}`,
  );

  const nudge = /* glsl */ `
vec4 nudge(vec4 c) {
  vec4 o = c;
  if (frame.eye.w > 0.5) { o.z = c.z + 2e-4 * c.w; } else { o.z = c.z * 1.002; }
  return o;
}
`;

  const bench = src(
    /* glsl */ `
${nudge}
layout(location = 0) in vec3 p;
void main() {
  gl_Position = toGl(nudge(frame.viewProj * vec4(p, 1.0)));
}`,
    /* glsl */ `
layout(std140) uniform Bench { vec4 color; } bench;
out vec4 outColor;
void main() {
  outColor = vec4(bench.color.rgb * bench.color.a, bench.color.a);
}`,
  );

  const trajBlock = /* glsl */ `
layout(std140) uniform Traj {
  vec4 line;
  vec4 hit;
  vec4 splat;
  vec4 params;    // xy = target size (px), z = splat radius (px), w = outline darkness
  vec4 sel;
  vec4 selParams; // x = highlight line width (px), y = end ring radius (px)
} traj;
`;

  const line = src(
    /* glsl */ `
${nudge}
layout(location = 0) in vec3 p;
void main() {
  gl_Position = toGl(nudge(frame.viewProj * vec4(p, 1.0)));
}`,
    /* glsl */ `
${trajBlock}
out vec4 outColor;
void main() {
  outColor = vec4(traj.line.rgb * traj.line.a, traj.line.a);
}`,
  );

  const splatVs = (radius: string) => /* glsl */ `
${nudge}
${trajBlock}
layout(location = 0) in vec4 end;
out vec2 vUv;
flat out float vHit;
void main() {
  vec2 c = QUAD[gl_VertexID];
  vec4 clip = nudge(frame.viewProj * vec4(end.xyz, 1.0));
  clip.x += c.x * ${radius} * 2.0 / traj.params.x * clip.w;
  clip.y += c.y * ${radius} * 2.0 / traj.params.y * clip.w;
  gl_Position = toGl(clip);
  vUv = c;
  vHit = end.w;
}`;

  const splat = src(
    splatVs("traj.params.z"),
    /* glsl */ `
${trajBlock}
in vec2 vUv;
flat in float vHit;
out vec4 outColor;
void main() {
  float r = length(vUv);
  float w = max(fwidth(r), 1e-4);
  float alpha = 1.0 - smoothstep(1.0 - w, 1.0, r);
  if (alpha <= 0.0) { discard; }
  vec4 fill = vHit > 0.5 ? traj.hit : traj.splat;
  float rim = smoothstep(0.62 - w, 0.62, r);
  vec3 color = mix(fill.rgb, fill.rgb * traj.params.w, rim);
  float a = alpha * fill.a;
  outColor = vec4(color * a, a);
}`,
  );

  const thick = src(
    /* glsl */ `
${nudge}
${trajBlock}
layout(location = 0) in vec3 a;
layout(location = 1) in vec3 b;
out float vSide;
void main() {
  vec4 ca = nudge(frame.viewProj * vec4(a, 1.0));
  vec4 cb = nudge(frame.viewProj * vec4(b, 1.0));
  if (ca.w <= 0.0 || cb.w <= 0.0) {
    gl_Position = vec4(2.0, 2.0, 2.0, 1.0);
    vSide = 0.0;
    return;
  }
  vec2 size = traj.params.xy;
  vec2 sa = ca.xy / ca.w * size;
  vec2 sb = cb.xy / cb.w * size;
  vec2 d = sb - sa;
  if (length(d) < 1e-4) { d = vec2(1.0, 0.0); }
  vec2 n = normalize(vec2(-d.y, d.x));
  vec2 c = UNIT_QUAD[gl_VertexID] * vec2(1.0, 2.0) - vec2(0.0, 1.0);
  vec4 p = c.x > 0.5 ? cb : ca;
  vec2 off = n * c.y * traj.selParams.x / size;
  gl_Position = toGl(vec4(p.xy + off * p.w, p.z, p.w));
  vSide = c.y;
}`,
    /* glsl */ `
${trajBlock}
in float vSide;
out vec4 outColor;
void main() {
  float w = max(fwidth(vSide), 1e-4);
  float a = (1.0 - smoothstep(1.0 - w, 1.0, abs(vSide))) * traj.sel.a;
  outColor = vec4(traj.sel.rgb * a, a);
}`,
  );

  const ring = src(
    splatVs("traj.selParams.y"),
    /* glsl */ `
${trajBlock}
in vec2 vUv;
flat in float vHit;
out vec4 outColor;
void main() {
  float r = length(vUv);
  float w = max(fwidth(r), 1e-4);
  float ringMask = smoothstep(0.62 - w, 0.62, r) * (1.0 - smoothstep(1.0 - w, 1.0, r));
  if (ringMask <= 0.0) { discard; }
  float a = ringMask * traj.sel.a;
  outColor = vec4(traj.sel.rgb * a, a);
}`,
  );

  return { mesh, grid, axis, paSurface, slice: sliceProgram, bench, line, splat, thick, ring };
}

export type GlShaderSet = ReturnType<typeof glShaders>;
