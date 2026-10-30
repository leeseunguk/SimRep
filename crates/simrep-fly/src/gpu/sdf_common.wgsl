// Shared sampled-SDF segment intersections. Each piece limits the segment's
// cell-length metric to 4; see sdf_hit.rs for the 64 geometric-cut bound.
// Polynomial roots are streamed one cell piece at a time, not stored globally.
struct sdf_Volume {
    dims: vec4<u32>,
    info: vec4<u32>, // number, sample offset, sdf_cylindrical | mirrors << 1, unused
    origin: vec4<f32>,
    spacing: vec4<f32>,
    parameters: vec4<f32>, // Lipschitz, sdf_entry tolerance, unused, unused
}
struct sdf_Poly { c: array<f32, 5>, degree: u32 }
struct sdf_Roots { value: array<f32, 16>, count: u32 }
var<private> sdf_cuts: array<f32, 64>;
var<private> sdf_cut_count: u32;
var<private> sdf_overflow: bool;

fn sdf_polynomial() -> sdf_Poly { var poly_value: sdf_Poly; return poly_value; }
fn sdf_affine(a: f32, b: f32) -> sdf_Poly {
    var poly_value = sdf_polynomial(); poly_value.c[0] = a; poly_value.c[1] = b; poly_value.degree = 1u; return poly_value;
}
fn sdf_scale(poly_value: sdf_Poly, s: f32) -> sdf_Poly {
    var r = poly_value;
    for (var i = 0u; i <= poly_value.degree; i++) { r.c[i] = poly_value.c[i] * s; }
    return r;
}
fn sdf_add(a: sdf_Poly, b: sdf_Poly) -> sdf_Poly {
    var r = sdf_polynomial(); r.degree = max(a.degree, b.degree);
    for (var i = 0u; i <= a.degree; i++) { r.c[i] += a.c[i]; }
    for (var i = 0u; i <= b.degree; i++) { r.c[i] += b.c[i]; }
    return r;
}
fn sdf_multiply(a: sdf_Poly, b: sdf_Poly) -> sdf_Poly {
    var r = sdf_polynomial(); r.degree = a.degree + b.degree;
    for (var i = 0u; i <= a.degree; i++) {
        for (var j = 0u; j <= b.degree; j++) { r.c[i+j] += a.c[i] * b.c[j]; }
    }
    return r;
}
fn sdf_evaluate(poly_value: sdf_Poly, t: f32) -> f32 {
    var result = 0.0;
    for (var i = i32(poly_value.degree); i >= 0; i--) { result = result * t + poly_value.c[u32(i)]; }
    return result;
}
fn sdf_insert_root(r: ptr<function, sdf_Roots>, t: f32) {
    if !(t >= 0.0 && t <= 1.0) { return; }
    var pos = 0u;
    while pos < (*r).count && (*r).value[pos] < t { pos++; }
    if pos < (*r).count && (*r).value[pos] == t { return; }
    for (var j = (*r).count; j > pos; j--) { (*r).value[j] = (*r).value[j-1u]; }
    (*r).value[pos] = t; (*r).count++;
}
// Match the CPU recursion's sorted/distinct return contract before building
// monotone intervals. Both loops are bounded by the 16-slot root capacity.
fn sdf_sorted_roots(input: sdf_Roots) -> sdf_Roots {
    var sorted = input;
    for (var i = 1u; i < sorted.count; i++) {
        let value = sorted.value[i];
        var j = i;
        for (var shift = 0u; shift < 16u; shift++) {
            if j == 0u || sorted.value[j-1u] <= value { break; }
            sorted.value[j] = sorted.value[j-1u]; j--;
        }
        sorted.value[j] = value;
    }
    var result: sdf_Roots;
    for (var i = 0u; i < sorted.count; i++) {
        if result.count == 0u || sorted.value[i] != result.value[result.count-1u] {
            result.value[result.count] = sorted.value[i]; result.count++;
        }
    }
    return result;
}
// Union of roots of poly_value and all its derivatives. Degree <= 4 gives at most
// 4+3+2+1 distinct interior roots and two endpoints: 16 slots suffice.
// Ascending derivative degree replaces CPU recursion. Sorting/deduplicating
// critical points removes duplicate intervals without removing any probes.
fn sdf_poly_roots(input: sdf_Poly) -> sdf_Roots {
    var poly_value = input;
    while poly_value.degree > 0u && poly_value.c[poly_value.degree] == 0.0 { poly_value.degree--; }
    var chain: array<sdf_Poly, 5>;
    let degree = poly_value.degree;
    while poly_value.degree > 0u {
        chain[poly_value.degree] = poly_value;
        var d = sdf_polynomial(); d.degree = poly_value.degree - 1u;
        for (var i = 1u; i <= poly_value.degree; i++) { d.c[i-1u] = f32(i) * poly_value.c[i]; }
        poly_value = d;
    }
    var roots: sdf_Roots;
    for (var n = 1u; n <= degree; n++) {
        let critical = sdf_sorted_roots(roots);
        roots.count = 0u;
        for (var j = 0u; j < critical.count; j++) { sdf_insert_root(&roots, critical.value[j]); }
        for (var j = 0u; j <= critical.count; j++) {
            var a = 0.0; var b = 1.0;
            if j > 0u { a = critical.value[j-1u]; }
            if j < critical.count { b = critical.value[j]; }
            let fa = sdf_evaluate(chain[n], a); let fb = sdf_evaluate(chain[n], b);
            if fa == 0.0 { sdf_insert_root(&roots, a); }
            if fb == 0.0 { sdf_insert_root(&roots, b); }
            if fa * fb >= 0.0 { continue; }
            let up = fa < 0.0;
            for (var k = 0u; k < 80u; k++) {
                let m = 0.5 * (a+b);
                if m == a || m == b { break; }
                if (sdf_evaluate(chain[n], m) < 0.0) == up { a = m; } else { b = m; }
            }
            sdf_insert_root(&roots, 0.5 * (a+b));
        }
    }
    return roots;
}
fn sdf_cylindrical(v: sdf_Volume) -> bool { return (v.info.z & 1u) != 0u; }
fn sdf_mirrored(v: sdf_Volume, a: u32) -> bool { return (v.info.z & (2u << a)) != 0u; }
fn sdf_sample_value(v: sdf_Volume, index: vec3<u32>) -> f32 {
    return sdf_samples[v.info.y + (index.z * v.dims.y + index.y) * v.dims.x + index.x];
}
fn sdf_distance(v: sdf_Volume, input: vec3<f32>) -> f32 {
    var point = input;
    if sdf_cylindrical(v) { point.y = sqrt(point.y*point.y + point.z*point.z); point.z = 0.0; }
    var boundary = -3.402823e38;
    var low: vec3<u32>; var high: vec3<u32>; var f: vec3<f32>;
    for (var a = 0u; a < 3u; a++) {
        if sdf_mirrored(v,a) { point[a] = v.origin[a] + abs(point[a]-v.origin[a]); }
        if v.dims[a] > 1u {
            var bound_low = v.origin[a]-point[a];
            if sdf_mirrored(v,a) || (sdf_cylindrical(v) && a == 1u) { bound_low = -3.402823e38; }
            boundary = max(boundary, max(bound_low, point[a]-v.origin[a]-f32(v.dims[a]-1u)*v.spacing[a]));
        }
        let x = clamp((point[a]-v.origin[a])/v.spacing[a], 0.0, f32(v.dims[a]-1u));
        low[a] = u32(floor(x)); high[a] = min(low[a]+1u,v.dims[a]-1u); f[a] = x-f32(low[a]);
    }
    var result = 0.0;
    // A one-node axis has f=0: its high corners have exactly zero weight.
    // Keep the remaining corner order and arithmetic, but avoid those loads.
    let corners = min(v.dims.xyz, vec3<u32>(2u));
    for (var z = 0u; z < corners.z; z++) {
        for (var y = 0u; y < corners.y; y++) {
            for (var x = 0u; x < corners.x; x++) {
                let corner = vec3<u32>(x,y,z); var pos: vec3<u32>; var weight = 1.0;
                for (var a = 0u; a < 3u; a++) {
                    pos[a] = select(high[a], low[a], corner[a] == 0u);
                    weight *= select(f[a],1.0-f[a],corner[a] == 0u);
                }
                result += weight * sdf_sample_value(v,pos);
            }
        }
    }
    return max(result,boundary);
}
fn sdf_push_cut(t: f32) {
    if !(t > 0.0 && t < 1.0) { return; }
    var pos = 0u;
    while pos < sdf_cut_count && sdf_cuts[pos] < t { pos++; }
    if pos < sdf_cut_count && sdf_cuts[pos] == t { return; }
    if sdf_cut_count == 64u { sdf_overflow = true; return; }
    for (var j = sdf_cut_count; j > pos; j--) { sdf_cuts[j] = sdf_cuts[j-1u]; }
    sdf_cuts[pos] = t; sdf_cut_count++;
}
fn sdf_circle(q: sdf_Poly, r: f32) {
    if r < 0.0 { return; }
    var poly_value = q; poly_value.c[0] -= r*r;
    let roots = sdf_poly_roots(poly_value);
    for (var i = 0u; i < roots.count; i++) { sdf_push_cut(roots.value[i]); }
}
fn sdf_radius(q: sdf_Poly, t: f32) -> f32 { return sqrt(max(q.c[0]+t*(q.c[1]+t*q.c[2]),0.0)); }
fn sdf_geometric_cuts(v: sdf_Volume, start: vec3<f32>, delta: vec3<f32>, q: sdf_Poly) {
    sdf_cut_count = 2u; sdf_cuts[0] = 0.0; sdf_cuts[1] = 1.0;
    for (var a = 0u; a < 3u; a++) {
        if (sdf_cylindrical(v) && a > 0u) || v.dims[a] < 2u || delta[a] == 0.0 { continue; }
        if sdf_mirrored(v,a) { sdf_push_cut((v.origin[a]-start[a])/delta[a]); }
        let c0 = start[a]-v.origin[a]; let c1 = c0+delta[a];
        for (var side = 0u; side < 2u; side++) {
            if side == 1u && !sdf_mirrored(v,a) { continue; }
            let sign = select(1.0,-1.0,side == 1u);
            let low = min(sign*c0,sign*c1)/v.spacing[a];
            let high = max(sign*c0,sign*c1)/v.spacing[a];
            let k0 = i32(max(ceil(low),0.0)); let k1 = i32(min(floor(high),f32(v.dims[a]-1u)));
            for (var k = k0; k <= k1; k++) { sdf_push_cut((sign*f32(k)*v.spacing[a]-c0)/delta[a]); }
        }
    }
    if sdf_cylindrical(v) && v.dims.y > 1u {
        var rmin = min(sdf_radius(q,0.0),sdf_radius(q,1.0)); let rmax = max(sdf_radius(q,0.0),sdf_radius(q,1.0));
        if q.c[2] > 0.0 { rmin = min(rmin,sdf_radius(q,clamp(-q.c[1]/(2.0*q.c[2]),0.0,1.0))); }
        if sdf_mirrored(v,1u) { sdf_circle(q,v.origin.y); }
        for (var side = 0u; side < 2u; side++) {
            if side == 1u && !sdf_mirrored(v,1u) { continue; }
            let sign = select(1.0,-1.0,side == 1u);
            let a = (rmin-v.origin.y)*sign; let b = (rmax-v.origin.y)*sign;
            let k0 = i32(max(ceil(min(a,b)/v.spacing.y),0.0));
            let k1 = i32(min(floor(max(a,b)/v.spacing.y),f32(v.dims.y-1u)));
            for (var k = k0; k <= k1; k++) { sdf_circle(q,v.origin.y+sign*f32(k)*v.spacing.y); }
        }
    }
}
// degree=5 marks a piece outside the sampled box.
fn sdf_piece(v: sdf_Volume, start: vec3<f32>, delta: vec3<f32>, q: sdf_Poly, t0: f32, t1: f32) -> sdf_Poly {
    let tm = 0.5*(t0+t1); let dt = t1-t0;
    var idx: vec3<u32>; var lin: array<vec2<f32>,3>;
    var invalid = sdf_polynomial(); invalid.degree = 5u;
    for (var a = 0u; a < 3u; a++) {
        if v.dims[a] < 2u || (sdf_cylindrical(v) && a > 0u) { continue; }
        let cm = start[a]+tm*delta[a];
        let sign = select(1.0,-1.0,sdf_mirrored(v,a) && cm < v.origin[a]);
        let xm = (v.origin[a]+sign*(cm-v.origin[a])-v.origin[a])/v.spacing[a];
        if xm < 0.0 || xm > f32(v.dims[a]-1u) { return invalid; }
        idx[a] = min(u32(floor(xm)),v.dims[a]-2u);
        let c0 = start[a]+t0*delta[a];
        lin[a] = vec2<f32>((v.origin[a]+sign*(c0-v.origin[a])-v.origin[a])/v.spacing[a]-f32(idx[a]), sign*delta[a]*dt/v.spacing[a]);
    }
    if !sdf_cylindrical(v) {
        var total = sdf_polynomial(); total.degree = 3u;
        for (var k = 0u; k < 2u; k++) {
            for (var j = 0u; j < 2u; j++) {
                for (var i = 0u; i < 2u; i++) {
                    let corner = vec3<u32>(i,j,k); var pos = idx; var poly_value = sdf_polynomial(); poly_value.c[0] = 1.0;
                    var skip = false;
                    for (var a = 0u; a < 3u; a++) {
                        if v.dims[a] < 2u { if corner[a] == 1u { skip = true; } continue; }
                        var w = sdf_affine(lin[a].x,lin[a].y);
                        if corner[a] == 0u { w = sdf_affine(1.0-lin[a].x,-lin[a].y); }
                        poly_value = sdf_multiply(poly_value,w); pos[a] += corner[a];
                    }
                    if skip { continue; }
                    let c = sdf_sample_value(v,pos);
                    for (var n = 0u; n <= poly_value.degree; n++) { total.c[n] += c*poly_value.c[n]; }
                }
            }
        }
        return total;
    }
    if v.dims.y < 2u { return invalid; }
    let rm = sdf_radius(q,tm);
    let sigma = select(1.0,-1.0,sdf_mirrored(v,1u) && rm < v.origin.y);
    let kappa = select(0.0,2.0*v.origin.y,sigma < 0.0);
    let vm = (sigma*rm+kappa-v.origin.y)/v.spacing.y;
    if vm < 0.0 || vm > f32(v.dims.y-1u) { return invalid; }
    let j0 = min(u32(floor(vm)),v.dims.y-2u); let r0 = v.origin.y+f32(j0)*v.spacing.y;
    let i0 = idx.x; let i1 = i0+select(0u,1u,v.dims.x > 1u);
    let c00 = sdf_sample_value(v,vec3<u32>(i0,j0,0u)); let c10 = sdf_sample_value(v,vec3<u32>(i1,j0,0u));
    let c01 = sdf_sample_value(v,vec3<u32>(i0,j0+1u,0u)); let c11 = sdf_sample_value(v,vec3<u32>(i1,j0+1u,0u));
    let u = sdf_affine(lin[0].x,lin[0].y); let one_minus_u = sdf_affine(1.0-lin[0].x,-lin[0].y);
    let a0 = sdf_add(sdf_scale(one_minus_u,c00),sdf_scale(u,c10));
    let b0 = sdf_add(sdf_scale(one_minus_u,c01-c00),sdf_scale(u,c11-c10));
    let a = sdf_add(a0,sdf_scale(b0,(kappa-r0)/v.spacing.y)); let b = sdf_scale(b0,sigma/v.spacing.y);
    var qs = sdf_polynomial(); qs.degree = 2u;
    qs.c[0] = q.c[0]+t0*(q.c[1]+t0*q.c[2]);
    qs.c[1] = (q.c[1]+2.0*t0*q.c[2])*dt; qs.c[2] = q.c[2]*dt*dt;
    return sdf_add(sdf_multiply(qs,sdf_multiply(b,b)),sdf_scale(sdf_multiply(a,a),-1.0));
}
var<private> sdf_leaving: bool;
fn sdf_entry(v: sdf_Volume, start: vec3<f32>, delta: vec3<f32>, length: f32, launch: bool) -> f32 {
    let d = sdf_distance(v,start);
    sdf_leaving = launch && d <= 0.0;
    if !launch && d <= 0.0 { return 0.0; }
    if length == 0.0 { return 2.0; }
    if d/max(v.parameters.x,1.0) > length && sdf_distance(v,start+delta) > 0.0 { return 2.0; }
    var q = sdf_polynomial(); q.degree = 2u;
    q.c[0] = start.y*start.y+start.z*start.z;
    q.c[1] = 2.0*(start.y*delta.y+start.z*delta.z); q.c[2] = delta.y*delta.y+delta.z*delta.z;
    sdf_geometric_cuts(v,start,delta,q);
    var previous = 0.0;
    for (var c = 1u; c < sdf_cut_count; c++) {
        let t0 = sdf_cuts[c-1u]; let t1 = sdf_cuts[c];
        let poly = sdf_piece(v,start,delta,q,t0,t1);
        var roots: sdf_Roots;
        if poly.degree <= 4u { roots = sdf_poly_roots(poly); }
        // Convert local polynomial roots to segment t, dropping endpoints.
        var local: sdf_Roots;
        for (var r = 0u; r < roots.count; r++) {
            let t = t0+roots.value[r]*(t1-t0);
            if t > t0 && t < t1 { sdf_insert_root(&local,t); }
        }
        sdf_insert_root(&local,t1);
        var t = t0;
        for (var r = 0u; r < local.count; r++) {
            let tb = local.value[r];
            for (var k = 0u; k < 2u; k++) {
                let probe = select(0.5*(t+tb),tb,k == 1u);
                let inside = sdf_distance(v,start+probe*delta) <= 0.0;
                if sdf_leaving {
                    if !inside { sdf_leaving = false; previous = probe; }
                    continue;
                }
                if !inside { previous = probe; continue; }
                var lo = previous; var hi = probe;
                while (hi-lo)*length > v.parameters.y*1e-3 {
                    let m = 0.5*(lo+hi);
                    if m == lo || m == hi { break; }
                    if sdf_distance(v,start+m*delta) <= 0.0 { hi = m; } else { lo = m; }
                }
                return hi;
            }
            t = tb;
        }
    }
    return 2.0;
}

// Shared split/reduce rule. Target metric 2 leaves headroom below the proven
// four-cell bound on the 64 geometric cuts, including scalar rounding.
fn sdf_first_hit(start: vec3<f32>, delta: vec3<f32>, launch: bool) -> vec3<f32> {
    var linear = vec3<f32>(0.0); var radial = 0.0;
    for (var i=0u; i<arrayLength(&sdf_headers); i++) {
        let v=sdf_headers[i];
        for (var a=0u; a<3u; a++) {
            if v.dims[a]>1u && (!sdf_cylindrical(v) || a==0u) { linear[a]=max(linear[a],1.0/v.spacing[a]); }
        }
        if sdf_cylindrical(v) && v.dims.y>1u { radial=max(radial,1.0/v.spacing.y); }
    }
    let metric=dot(abs(delta),linear)+sqrt(delta.y*delta.y+delta.z*delta.z)*radial;
    let pieces=max(1u,u32(ceil(metric/2.0)));
    var best=2.0; var number=0u;
    sdf_overflow=false;
    for (var i=0u; i<arrayLength(&sdf_headers); i++) {
        let v=sdf_headers[i];
        var leaving=launch && sdf_distance(v,start)<=0.0;
        for (var j=0u; j<pieces; j++) {
            let t0=f32(j)/f32(pieces); let t1=f32(j+1u)/f32(pieces);
            if t0>=best { break; }
            let a=start+t0*delta; let b=start+t1*delta; let d=b-a;
            if dot(abs(d),linear)+sqrt(d.y*d.y+d.z*d.z)*radial>4.0 { return vec3<f32>(0.0,0.0,2.0); }
            let length=sqrt((d.x*d.x+d.y*d.y)+d.z*d.z);
            let hit=sdf_entry(v,a,d,length,leaving);
            leaving=sdf_leaving;
            if hit<=1.0 {
                let t=t0+(t1-t0)*hit;
                if t<best { best=t; number=v.info.x; }
                break;
            }
        }
    }
    if sdf_overflow { return vec3<f32>(0.0,0.0,2.0); }
    return vec3<f32>(best,f32(number),select(0.0,1.0,best<=1.0));
}
fn sdf_membership(point: vec3<f32>) -> u32 {
    for (var i=0u; i<arrayLength(&sdf_headers); i++) {
        if sdf_distance(sdf_headers[i],point)<=0.0 { return sdf_headers[i].info.x; }
    }
    return 0u;
}
