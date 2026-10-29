// Shared functions and Volume are supplied by sdf_common.wgsl.
struct Segment { start: vec4<f32>, end: vec4<f32> }
struct Hit { value: vec4<f32> }
@group(0) @binding(0) var<storage, read> sdf_headers: array<sdf_Volume>;
@group(0) @binding(1) var<storage, read> sdf_samples: array<f32>;
@group(0) @binding(2) var<storage, read> segments: array<Segment>;
@group(0) @binding(3) var<storage, read_write> hits: array<Hit>;
@compute @workgroup_size(64)
fn first_hits(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= arrayLength(&segments) { return; }
    let start = segments[i].start.xyz; let delta = segments[i].end.xyz-start;
    let length = sqrt((delta.x*delta.x+delta.y*delta.y)+delta.z*delta.z);
    var best = 2.0; var number = 0u;
    sdf_overflow = false;
    for (var v = 0u; v < arrayLength(&sdf_headers); v++) {
        let t = sdf_entry(sdf_headers[v],start,delta,length,false);
        if t < best { best = t; number = sdf_headers[v].info.x; }
    }
    hits[i].value = vec4<f32>(best,f32(number),select(0.0,1.0,best <= 1.0),0.0);
    if sdf_overflow { hits[i].value.z = 2.0; }
}
