// Standalone sampled-SDF segment intersections. Host limits each segment's
// cell-length metric to 4; see sdf_hit.rs for the 64 geometric-cut bound.
// Polynomial roots are streamed one cell piece at a time, not stored globally.
struct Volume {
    dims: vec4<u32>,
    info: vec4<u32>, // number, sample offset, cylindrical | mirrors << 1, unused
    origin: vec4<f32>,
    spacing: vec4<f32>,
    parameters: vec4<f32>, // Lipschitz, entry tolerance, unused, unused
}
struct Segment { start: vec4<f32>, end: vec4<f32> }
struct Hit { value: vec4<f32> } // t, number, found (2 means cut overflow), unused
@group(0) @binding(0) var<storage, read> volumes: array<Volume>;
@group(0) @binding(1) var<storage, read> samples: array<f32>;
@group(0) @binding(2) var<storage, read> segments: array<Segment>;
@group(0) @binding(3) var<storage, read_write> hits: array<Hit>;
struct Poly { c: array<f32, 5>, degree: u32 }
struct Roots { value: array<f32, 16>, count: u32 }
var<private> cuts: array<f32, 64>;
var<private> cut_count: u32;
var<private> overflow: bool;

fn polynomial() -> Poly { var p: Poly; return p; }
fn affine(a: f32, b: f32) -> Poly {
    var p = polynomial(); p.c[0] = a; p.c[1] = b; p.degree = 1u; return p;
}
fn scale(p: Poly, s: f32) -> Poly {
    var r = p;
    for (var i = 0u; i <= p.degree; i++) { r.c[i] = p.c[i] * s; }
    return r;
}
fn add(a: Poly, b: Poly) -> Poly {
    var r = polynomial(); r.degree = max(a.degree, b.degree);
    for (var i = 0u; i <= a.degree; i++) { r.c[i] += a.c[i]; }
    for (var i = 0u; i <= b.degree; i++) { r.c[i] += b.c[i]; }
    return r;
}
fn multiply(a: Poly, b: Poly) -> Poly {
    var r = polynomial(); r.degree = a.degree + b.degree;
    for (var i = 0u; i <= a.degree; i++) {
        for (var j = 0u; j <= b.degree; j++) { r.c[i+j] += a.c[i] * b.c[j]; }
    }
    return r;
}
fn evaluate(p: Poly, t: f32) -> f32 {
    var result = 0.0;
    for (var i = i32(p.degree); i >= 0; i--) { result = result * t + p.c[u32(i)]; }
    return result;
}
fn insert_root(r: ptr<function, Roots>, t: f32) {
    if !(t >= 0.0 && t <= 1.0) { return; }
    var pos = 0u;
    while pos < (*r).count && (*r).value[pos] < t { pos++; }
    if pos < (*r).count && (*r).value[pos] == t { return; }
    for (var j = (*r).count; j > pos; j--) { (*r).value[j] = (*r).value[j-1u]; }
    (*r).value[pos] = t; (*r).count++;
}
// Union of roots of p and all its derivatives. Degree <= 4 gives at most
// 4+3+2+1 distinct interior roots and two endpoints: 16 slots suffice.
// Ascending derivative degree replaces CPU recursion. Sorting/deduplicating
// critical points removes duplicate intervals without removing any probes.
fn poly_roots(input: Poly) -> Roots {
    var p = input;
    while p.degree > 0u && p.c[p.degree] == 0.0 { p.degree--; }
    var chain: array<Poly, 5>;
    let degree = p.degree;
    while p.degree > 0u {
        chain[p.degree] = p;
        var d = polynomial(); d.degree = p.degree - 1u;
        for (var i = 1u; i <= p.degree; i++) { d.c[i-1u] = f32(i) * p.c[i]; }
        p = d;
    }
    var roots: Roots;
    for (var n = 1u; n <= degree; n++) {
        let critical = roots;
        roots.count = 0u;
        for (var j = 0u; j < critical.count; j++) { insert_root(&roots, critical.value[j]); }
        for (var j = 0u; j <= critical.count; j++) {
            var a = 0.0; var b = 1.0;
            if j > 0u { a = critical.value[j-1u]; }
            if j < critical.count { b = critical.value[j]; }
            let fa = evaluate(chain[n], a); let fb = evaluate(chain[n], b);
            if fa == 0.0 { insert_root(&roots, a); }
            if fb == 0.0 { insert_root(&roots, b); }
            if fa * fb >= 0.0 { continue; }
            let up = fa < 0.0;
            for (var k = 0u; k < 80u; k++) {
                let m = 0.5 * (a+b);
                if m == a || m == b { break; }
                if (evaluate(chain[n], m) < 0.0) == up { a = m; } else { b = m; }
            }
            insert_root(&roots, 0.5 * (a+b));
        }
    }
    return roots;
}
fn cylindrical(v: Volume) -> bool { return (v.info.z & 1u) != 0u; }
fn mirrored(v: Volume, a: u32) -> bool { return (v.info.z & (2u << a)) != 0u; }
fn sample_value(v: Volume, index: vec3<u32>) -> f32 {
    return samples[v.info.y + (index.z * v.dims.y + index.y) * v.dims.x + index.x];
}
fn distance(v: Volume, input: vec3<f32>) -> f32 {
    var point = input;
    if cylindrical(v) { point.y = sqrt(point.y*point.y + point.z*point.z); point.z = 0.0; }
    var boundary = -3.402823e38;
    var low: vec3<u32>; var high: vec3<u32>; var f: vec3<f32>;
    for (var a = 0u; a < 3u; a++) {
        if mirrored(v,a) { point[a] = v.origin[a] + abs(point[a]-v.origin[a]); }
        if v.dims[a] > 1u {
            var bound_low = v.origin[a]-point[a];
            if mirrored(v,a) || (cylindrical(v) && a == 1u) { bound_low = -3.402823e38; }
            boundary = max(boundary, max(bound_low, point[a]-v.origin[a]-f32(v.dims[a]-1u)*v.spacing[a]));
        }
        let x = clamp((point[a]-v.origin[a])/v.spacing[a], 0.0, f32(v.dims[a]-1u));
        low[a] = u32(floor(x)); high[a] = min(low[a]+1u, v.dims[a]-1u); f[a] = x-f32(low[a]);
    }
    var result = 0.0;
    for (var z = 0u; z < 2u; z++) {
        for (var y = 0u; y < 2u; y++) {
            for (var x = 0u; x < 2u; x++) {
                let corner = vec3<u32>(x,y,z); var pos: vec3<u32>; var weight = 1.0;
                for (var a = 0u; a < 3u; a++) {
                    pos[a] = select(high[a], low[a], corner[a] == 0u);
                    weight *= select(f[a], 1.0-f[a], corner[a] == 0u);
                }
                result += weight * sample_value(v,pos);
            }
        }
    }
    return max(result,boundary);
}
fn push_cut(t: f32) {
    if !(t > 0.0 && t < 1.0) { return; }
    var pos = 0u;
    while pos < cut_count && cuts[pos] < t { pos++; }
    if pos < cut_count && cuts[pos] == t { return; }
    if cut_count == 64u { overflow = true; return; }
    for (var j = cut_count; j > pos; j--) { cuts[j] = cuts[j-1u]; }
    cuts[pos] = t; cut_count++;
}
fn circle(q: Poly, r: f32) {
    if r < 0.0 { return; }
    var p = q; p.c[0] -= r*r;
    let roots = poly_roots(p);
    for (var i = 0u; i < roots.count; i++) { push_cut(roots.value[i]); }
}
fn radius(q: Poly, t: f32) -> f32 { return sqrt(max(q.c[0]+t*(q.c[1]+t*q.c[2]),0.0)); }
fn geometric_cuts(v: Volume, start: vec3<f32>, delta: vec3<f32>, q: Poly) {
    cut_count = 2u; cuts[0] = 0.0; cuts[1] = 1.0;
    for (var a = 0u; a < 3u; a++) {
        if (cylindrical(v) && a > 0u) || v.dims[a] < 2u || delta[a] == 0.0 { continue; }
        if mirrored(v,a) { push_cut((v.origin[a]-start[a])/delta[a]); }
        let c0 = start[a]-v.origin[a]; let c1 = c0+delta[a];
        for (var side = 0u; side < 2u; side++) {
            if side == 1u && !mirrored(v,a) { continue; }
            let sign = select(1.0,-1.0,side == 1u);
            let low = min(sign*c0,sign*c1)/v.spacing[a];
            let high = max(sign*c0,sign*c1)/v.spacing[a];
            let k0 = i32(max(ceil(low),0.0)); let k1 = i32(min(floor(high),f32(v.dims[a]-1u)));
            for (var k = k0; k <= k1; k++) { push_cut((sign*f32(k)*v.spacing[a]-c0)/delta[a]); }
        }
    }
    if cylindrical(v) && v.dims.y > 1u {
        var rmin = min(radius(q,0.0),radius(q,1.0)); let rmax = max(radius(q,0.0),radius(q,1.0));
        if q.c[2] > 0.0 { rmin = min(rmin,radius(q,clamp(-q.c[1]/(2.0*q.c[2]),0.0,1.0))); }
        if mirrored(v,1u) { circle(q,v.origin.y); }
        for (var side = 0u; side < 2u; side++) {
            if side == 1u && !mirrored(v,1u) { continue; }
            let sign = select(1.0,-1.0,side == 1u);
            let a = (rmin-v.origin.y)*sign; let b = (rmax-v.origin.y)*sign;
            let k0 = i32(max(ceil(min(a,b)/v.spacing.y),0.0));
            let k1 = i32(min(floor(max(a,b)/v.spacing.y),f32(v.dims.y-1u)));
            for (var k = k0; k <= k1; k++) { circle(q,v.origin.y+sign*f32(k)*v.spacing.y); }
        }
    }
}
// degree=5 marks a piece outside the sampled box.
fn piece(v: Volume, start: vec3<f32>, delta: vec3<f32>, q: Poly, t0: f32, t1: f32) -> Poly {
    let tm = 0.5*(t0+t1); let dt = t1-t0;
    var idx: vec3<u32>; var lin: array<vec2<f32>,3>;
    var invalid = polynomial(); invalid.degree = 5u;
    for (var a = 0u; a < 3u; a++) {
        if v.dims[a] < 2u || (cylindrical(v) && a > 0u) { continue; }
        let cm = start[a]+tm*delta[a];
        let sign = select(1.0,-1.0,mirrored(v,a) && cm < v.origin[a]);
        let xm = (v.origin[a]+sign*(cm-v.origin[a])-v.origin[a])/v.spacing[a];
        if xm < 0.0 || xm > f32(v.dims[a]-1u) { return invalid; }
        idx[a] = min(u32(floor(xm)),v.dims[a]-2u);
        let c0 = start[a]+t0*delta[a];
        lin[a] = vec2<f32>((v.origin[a]+sign*(c0-v.origin[a])-v.origin[a])/v.spacing[a]-f32(idx[a]), sign*delta[a]*dt/v.spacing[a]);
    }
    if !cylindrical(v) {
        var total = polynomial(); total.degree = 3u;
        for (var k = 0u; k < 2u; k++) {
            for (var j = 0u; j < 2u; j++) {
                for (var i = 0u; i < 2u; i++) {
                    let corner = vec3<u32>(i,j,k); var pos = idx; var p = polynomial(); p.c[0] = 1.0;
                    var skip = false;
                    for (var a = 0u; a < 3u; a++) {
                        if v.dims[a] < 2u { if corner[a] == 1u { skip = true; } continue; }
                        var w = affine(lin[a].x,lin[a].y);
                        if corner[a] == 0u { w = affine(1.0-lin[a].x,-lin[a].y); }
                        p = multiply(p,w); pos[a] += corner[a];
                    }
                    if skip { continue; }
                    let c = sample_value(v,pos);
                    for (var n = 0u; n <= p.degree; n++) { total.c[n] += c*p.c[n]; }
                }
            }
        }
        return total;
    }
    if v.dims.y < 2u { return invalid; }
    let rm = radius(q,tm);
    let sigma = select(1.0,-1.0,mirrored(v,1u) && rm < v.origin.y);
    let kappa = select(0.0,2.0*v.origin.y,sigma < 0.0);
    let vm = (sigma*rm+kappa-v.origin.y)/v.spacing.y;
    if vm < 0.0 || vm > f32(v.dims.y-1u) { return invalid; }
    let j0 = min(u32(floor(vm)),v.dims.y-2u); let r0 = v.origin.y+f32(j0)*v.spacing.y;
    let i0 = idx.x; let i1 = i0+select(0u,1u,v.dims.x > 1u);
    let c00 = sample_value(v,vec3<u32>(i0,j0,0u)); let c10 = sample_value(v,vec3<u32>(i1,j0,0u));
    let c01 = sample_value(v,vec3<u32>(i0,j0+1u,0u)); let c11 = sample_value(v,vec3<u32>(i1,j0+1u,0u));
    let u = affine(lin[0].x,lin[0].y); let one_minus_u = affine(1.0-lin[0].x,-lin[0].y);
    let a0 = add(scale(one_minus_u,c00),scale(u,c10));
    let b0 = add(scale(one_minus_u,c01-c00),scale(u,c11-c10));
    let a = add(a0,scale(b0,(kappa-r0)/v.spacing.y)); let b = scale(b0,sigma/v.spacing.y);
    var qs = polynomial(); qs.degree = 2u;
    qs.c[0] = q.c[0]+t0*(q.c[1]+t0*q.c[2]);
    qs.c[1] = (q.c[1]+2.0*t0*q.c[2])*dt; qs.c[2] = q.c[2]*dt*dt;
    return add(multiply(qs,multiply(b,b)),scale(multiply(a,a),-1.0));
}
fn entry(v: Volume, start: vec3<f32>, delta: vec3<f32>, length: f32) -> f32 {
    let d = distance(v,start);
    if d <= 0.0 { return 0.0; }
    if length == 0.0 { return 2.0; }
    if d/max(v.parameters.x,1.0) > length && distance(v,start+delta) > 0.0 { return 2.0; }
    var q = polynomial(); q.degree = 2u;
    q.c[0] = start.y*start.y+start.z*start.z;
    q.c[1] = 2.0*(start.y*delta.y+start.z*delta.z); q.c[2] = delta.y*delta.y+delta.z*delta.z;
    geometric_cuts(v,start,delta,q);
    var previous = 0.0;
    for (var c = 1u; c < cut_count; c++) {
        let t0 = cuts[c-1u]; let t1 = cuts[c];
        let poly = piece(v,start,delta,q,t0,t1);
        var roots: Roots;
        if poly.degree <= 4u { roots = poly_roots(poly); }
        // Convert local polynomial roots to segment t, dropping endpoints.
        var local: Roots;
        for (var r = 0u; r < roots.count; r++) {
            let t = t0+roots.value[r]*(t1-t0);
            if t > t0 && t < t1 { insert_root(&local,t); }
        }
        insert_root(&local,t1);
        var t = t0;
        for (var r = 0u; r < local.count; r++) {
            let tb = local.value[r];
            for (var k = 0u; k < 2u; k++) {
                let probe = select(0.5*(t+tb),tb,k == 1u);
                if distance(v,start+probe*delta) > 0.0 { previous = probe; continue; }
                var lo = previous; var hi = probe;
                while (hi-lo)*length > v.parameters.y*1e-3 {
                    let m = 0.5*(lo+hi);
                    if m == lo || m == hi { break; }
                    if distance(v,start+m*delta) <= 0.0 { hi = m; } else { lo = m; }
                }
                return hi;
            }
            t = tb;
        }
    }
    return 2.0;
}
@compute @workgroup_size(64)
fn first_hits(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= arrayLength(&segments) { return; }
    let start = segments[i].start.xyz; let delta = segments[i].end.xyz-start;
    let length = sqrt((delta.x*delta.x+delta.y*delta.y)+delta.z*delta.z);
    var best = 2.0; var number = 0u;
    overflow = false;
    for (var v = 0u; v < arrayLength(&volumes); v++) {
        let t = entry(volumes[v],start,delta,length);
        if t < best { best = t; number = volumes[v].info.x; }
    }
    hits[i].value = vec4<f32>(best,f32(number),select(0.0,1.0,best <= 1.0),0.0);
    if overflow { hits[i].value.z = 2.0; }
}
