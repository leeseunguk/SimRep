@group(0) @binding(0) var<storage,read> cells:array<u32>;
@group(0) @binding(1) var<storage,read_write> heads:array<atomic<u32>>;
@group(0) @binding(2) var<storage,read_write> links:array<atomic<u32>>;
@compute @workgroup_size(128)
fn main(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    let i=id.x+groups.x*128u*id.y;
    if(i>=cells[0]) { return; }
    // Keep each cell in descending source-index order. A displaced smaller
    // index continues down the larger node's link, independent of scheduling.
    var node=i+1u;
    var previous=atomicMax(&heads[cells[i+1u]],node);
    loop {
        if(previous==0u) { break; }
        let parent=max(node,previous);
        node=min(node,previous);
        previous=atomicMax(&links[parent-1u],node);
    }
}
