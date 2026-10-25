@group(0) @binding(0) var<storage,read> cells:array<u32>;
@group(0) @binding(1) var<storage,read_write> heads:array<atomic<u32>>;
@group(0) @binding(2) var<storage,read_write> links:array<u32>;
@compute @workgroup_size(128)
fn main(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    let i=id.x+groups.x*128u*id.y;
    if(i>=cells[0]) { return; }
    links[i]=atomicExchange(&heads[cells[i+1u]],i+1u);
}
