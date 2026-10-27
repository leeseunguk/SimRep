// The fine operator is -W L with eliminated electrode columns. Coarse
// operators are P^T A P; rows have a local 27-point stencil.
struct Params {
    dims: vec4<u32>, // xyz, fine flag
    coarse: vec4<u32>,
    offsets: vec4<u32>, // axis data offsets and mask offset
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> matrix: array<f32>;
@group(0) @binding(2) var<storage, read> x: array<f32>;
@group(0) @binding(3) var<storage, read> rhs: array<f32>;
@group(0) @binding(4) var<storage, read_write> out: array<f32>;
@group(0) @binding(5) var<storage, read> auxiliary: array<f32>;

fn linear(g: vec3<u32>, d: vec3<u32>) -> u32 {
    return g.x + d.x * (g.y + d.y * g.z);
}
fn coord(i: u32, d: vec3<u32>) -> vec3<u32> {
    return vec3(i % d.x, i / d.x % d.y, i / (d.x * d.y));
}
fn invocation(g: vec3<u32>, groups: vec3<u32>) -> u32 {
    return g.x + groups.x * 128u * g.y;
}
fn count(d: vec3<u32>) -> u32 { return d.x * d.y * d.z; }
fn is_free(i: u32) -> bool {
    if p.dims.w == 1u { return matrix[p.offsets.w + i] <= 0.0; }
    return matrix[13u * count(p.dims.xyz) + i] > 0.0;
}
fn weight(c: vec3<u32>) -> f32 {
    var w = 1.0;
    for(var a = 0u; a < 3u; a++) { w *= matrix[p.offsets[a] + 3u * c[a] + 2u]; }
    return w;
}
fn diagonal(c: vec3<u32>) -> f32 {
    let i=linear(c,p.dims.xyz);
    let tag=matrix[p.offsets.w+i];
    if tag<0.0 {
        return weight(c)*matrix[p.offsets.w+count(p.dims.xyz)+8u*(u32(-tag)-1u)+6u];
    }
    var d = 0.0;
    for(var a = 0u; a < 3u; a++) {
        let k = p.offsets[a] + 3u * c[a];
        d += matrix[k] + matrix[k + 1u];
    }
    return weight(c) * d;
}
fn neighbor(c: vec3<u32>, s: u32) -> vec3<i32> {
    return vec3<i32>(c) + vec3<i32>(i32(s % 3u)-1, i32(s/3u % 3u)-1, i32(s/9u)-1);
}
fn inside(c: vec3<i32>, d: vec3<u32>) -> bool {
    return all(c >= vec3(0)) && all(c < vec3<i32>(d));
}
fn coefficient(i: u32, c: vec3<u32>, s: u32) -> f32 {
    if p.dims.w == 0u { return matrix[s * count(p.dims.xyz) + i]; }
    if s == 13u { return diagonal(c); }
    var axis = 3u;
    var plus = 0u;
    switch s {
        case 12u: { axis=0u; }
        case 14u: { axis=0u; plus=1u; }
        case 10u: { axis=1u; }
        case 16u: { axis=1u; plus=1u; }
        case 4u: { axis=2u; }
        case 22u: { axis=2u; plus=1u; }
        default: { return 0.0; }
    }
    let nc = neighbor(c,s);
    if !inside(nc,p.dims.xyz) { return 0.0; }
    let j = linear(vec3<u32>(nc),p.dims.xyz);
    if !is_free(j) { return 0.0; }
    let tag=matrix[p.offsets.w+i];
    if tag<0.0 {
        return -weight(c)*matrix[p.offsets.w+count(p.dims.xyz)+8u*(u32(-tag)-1u)+2u*axis+plus];
    }
    return -weight(c) * matrix[p.offsets[axis] + 3u*c[axis] + plus];
}
// Endpoints are retained, including a final interval of length one.
fn selected(c: u32, n: u32) -> u32 { return min(2u*c,n-1u); }
fn lower(i: u32, n: u32, nc: u32) -> u32 {
    if n == 1u { return 0u; }
    return min(i/2u,nc-2u);
}
fn fraction(i: u32, n: u32, lo: u32) -> f32 {
    if n == 1u { return 0.0; }
    return f32(i-selected(lo,n))/f32(selected(lo+1u,n)-selected(lo,n));
}
fn transfer_weight(fc: vec3<u32>, cc: vec3<u32>) -> f32 {
    var w = 1.0;
    for(var a=0u;a<3u;a++) {
        let lo = lower(fc[a],p.dims[a],p.coarse[a]);
        let t = fraction(fc[a],p.dims[a],lo);
        if cc[a] == lo { w *= 1.0-t; }
        else if cc[a] == lo+1u { w *= t; }
        else { return 0.0; }
    }
    return w;
}
fn support_lo(c: vec3<u32>) -> vec3<u32> {
    var lo = vec3(0u);
    for(var a=0u;a<3u;a++) {
        if c[a]>0u { lo[a] = selected(c[a]-1u,p.dims[a])+1u; }
    }
    return lo;
}
fn support_hi(c: vec3<u32>) -> vec3<u32> {
    var hi = p.dims.xyz-vec3(1u);
    for(var a=0u;a<3u;a++) {
        if c[a]+1u<p.coarse[a] { hi[a] = selected(c[a]+1u,p.dims[a])-1u; }
    }
    return hi;
}

@compute @workgroup_size(128)
fn galerkin(@builtin(global_invocation_id) g: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i=invocation(g,groups);
    if i>=count(p.coarse.xyz) { return; }
    let c=coord(i,p.coarse.xyz);
    let lo=support_lo(c);
    let hi=support_hi(c);
    ASSEMBLY_INIT
    for(var z=lo.z;z<=hi.z;z++) {
        for(var y=lo.y;y<=hi.y;y++) {
            for(var xx=lo.x;xx<=hi.x;xx++) {
                let fc=vec3(xx,y,z);
                let j=linear(fc,p.dims.xyz);
                if !is_free(j) { continue; }
                let left=transfer_weight(fc,c);
                for(var s=0u;s<27u;s++) {
                    let a=coefficient(j,fc,s);
                    if a==0.0 { continue; }
                    let kc=vec3<u32>(neighbor(fc,s));
                    // Static vector components avoid FXC forcing the enclosing
                    // variable-length support loops to unroll.
                    let base=vec3(
                        lower(kc.x,p.dims.x,p.coarse.x),
                        lower(kc.y,p.dims.y,p.coarse.y),
                        lower(kc.z,p.dims.z,p.coarse.z));
                    let t=vec3(
                        fraction(kc.x,p.dims.x,base.x),
                        fraction(kc.y,p.dims.y,base.y),
                        fraction(kc.z,p.dims.z,base.z));
                    for(var corner=0u;corner<8u;corner++) {
                        let delta=vec3(corner&1u,(corner>>1u)&1u,(corner>>2u)&1u);
                        let weights=select(vec3(1.0)-t,t,delta==vec3(1u));
                        let right=weights.x*weights.y*weights.z;
                        if right==0.0 { continue; }
                        let node=base+delta;
                        let offset=vec3<i32>(node)-vec3<i32>(c)+vec3(1);
                        let slot=u32(offset.x+3*offset.y+9*offset.z);
                        ASSEMBLY_ADD
                    }
                }
            }
        }
    }
    // Stencil-major layout coalesces each coefficient's accesses across lanes.
    ASSEMBLY_STORE
}

fn product(i:u32,c:vec3<u32>) -> vec2<f32> {
    if p.dims.w==1u {
        var sum=diagonal(c)*x[i];
        let strides=vec3(1u,p.dims.x,p.dims.x*p.dims.y);
        let w=weight(c);
        let tag=matrix[p.offsets.w+i];
        if tag<0.0 {
            let base=p.offsets.w+count(p.dims.xyz)+8u*(u32(-tag)-1u);
            for(var a=0u;a<3u;a++) {
                if c[a]>0u { sum-=w*matrix[base+2u*a]*x[i-strides[a]]; }
                if c[a]+1u<p.dims[a] { sum-=w*matrix[base+2u*a+1u]*x[i+strides[a]]; }
            }
            return vec2(sum,diagonal(c)*2.0);
        }
        for(var a=0u;a<3u;a++) {
            let k=p.offsets[a]+3u*c[a];
            if c[a]>0u { sum -= w*matrix[k]*x[i-strides[a]]; }
            if c[a]+1u<p.dims[a] { sum -= w*matrix[k+1u]*x[i+strides[a]]; }
        }
        return vec2(sum,diagonal(c)*2.0);
    }
    var ax=0.0;
    var bound=0.0;
    for(var s=0u;s<27u;s++) {
        let a=matrix[s*count(p.dims.xyz)+i];
        if a!=0.0 {
            let j=linear(vec3<u32>(neighbor(c,s)),p.dims.xyz);
            ax += a*x[j];
            bound += abs(a);
        }
    }
    return vec2(ax,bound);
}

// Physical (unweighted) correction residual; only a scalar leaves the GPU.
var<workgroup> norm_block: array<f32,128>;
fn store_norm(i: u32, lane: u32, value: f32) {
    norm_block[lane]=value;
    workgroupBarrier();
    for(var stride=64u;stride>0u;stride/=2u) {
        if lane<stride { norm_block[lane]=max(norm_block[lane],norm_block[lane+stride]); }
        workgroupBarrier();
    }
    if lane==0u && i/128u<arrayLength(&out) { out[i/128u]=norm_block[0]; }
}
@compute @workgroup_size(128)
fn correction_norm(@builtin(global_invocation_id) g: vec3<u32>,
                   @builtin(local_invocation_index) lane: u32,
                   @builtin(num_workgroups) groups: vec3<u32>) {
    let i=invocation(g,groups);
    var r=0.0;
    if i<count(p.dims.xyz) && is_free(i) {
        let c=coord(i,p.dims.xyz);
        r=abs(rhs[i]-product(i,c).x)/weight(c);
        // A NaN must not be hidden by max's NaN behavior.
        if !(r<=bitcast<f32>(0x7f7fffffu)) { r=bitcast<f32>(0x7f800000u); }
    }
    store_norm(i,lane,r);
}
@compute @workgroup_size(128)
fn reduce_norm(@builtin(global_invocation_id) g: vec3<u32>,
               @builtin(local_invocation_index) lane: u32,
               @builtin(num_workgroups) groups: vec3<u32>) {
    let i=invocation(g,groups);
    var r=0.0;
    if i<arrayLength(&x) { r=x[i]; }
    store_norm(i,lane,r);
}
@compute @workgroup_size(128)
fn jacobi_sweep(@builtin(global_invocation_id) g:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    let i=invocation(g,groups);
    if i>=count(p.dims.xyz) { return; }
    if !is_free(i) { out[i]=0.0; return; }
    let ax=product(i,coord(i,p.dims.xyz));
    out[i]=x[i]+(4.0/3.0)*(rhs[i]-ax.x)/ax.y;
}
@compute @workgroup_size(128)
fn residual(@builtin(global_invocation_id) g:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    let i=invocation(g,groups);
    if i>=count(p.dims.xyz) { return; }
    if !is_free(i) { out[i]=0.0; return; }
    out[i]=rhs[i]-product(i,coord(i,p.dims.xyz)).x;
}
@compute @workgroup_size(128)
fn restrict_rhs(@builtin(global_invocation_id) g:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    let i=invocation(g,groups);
    if i>=count(p.coarse.xyz) { return; }
    let c=coord(i,p.coarse.xyz);
    let lo=support_lo(c);
    let hi=support_hi(c);
    var sum=0.0;
    for(var z=lo.z;z<=hi.z;z++) {
        for(var y=lo.y;y<=hi.y;y++) {
            for(var xx=lo.x;xx<=hi.x;xx++) {
                let fc=vec3(xx,y,z);
                sum+=transfer_weight(fc,c)*x[linear(fc,p.dims.xyz)];
            }
        }
    }
    out[i]=sum;
}
@compute @workgroup_size(128)
fn prolong(@builtin(global_invocation_id) g:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    let i=invocation(g,groups);
    if i>=count(p.dims.xyz) { return; }
    if !is_free(i) { out[i]=0.0; return; }
    let c=coord(i,p.dims.xyz);
    var base:vec3<u32>;
    var t:vec3<f32>;
    for(var a=0u;a<3u;a++) {
        base[a]=lower(c[a],p.dims[a],p.coarse[a]);
        t[a]=fraction(c[a],p.dims[a],base[a]);
    }
    var value=x[i];
    for(var corner=0u;corner<8u;corner++) {
        let delta=vec3(corner&1u,(corner>>1u)&1u,(corner>>2u)&1u);
        let w=select(vec3(1.0)-t,t,delta==vec3(1u));
        if w.x*w.y*w.z!=0.0 { value+=w.x*w.y*w.z*auxiliary[linear(base+delta,p.coarse.xyz)]; }
    }
    out[i]=value;
}
