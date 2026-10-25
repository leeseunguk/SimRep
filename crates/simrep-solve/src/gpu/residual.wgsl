// Unit-spacing 7-point Laplacian residual, r = b - Laplacian(x).
// Boundary nodes are excluded from the solve and have zero residual.
@group(0) @binding(0) var<storage, read> potential: array<f32>;
@group(0) @binding(1) var<storage, read> rhs: array<f32>;
@group(0) @binding(2) var<storage, read_write> residual: array<f32>;

@compute @workgroup_size(4, 4, 4)
fn main(@builtin(global_invocation_id) p: vec3<u32>) {
    let n = 64u;
    if any(p >= vec3<u32>(n)) {
        return;
    }
    let i = p.x + n * (p.y + n * p.z);
    if any(p == vec3<u32>(0u)) || any(p == vec3<u32>(n - 1u)) {
        residual[i] = 0.0;
        return;
    }
    let laplacian = potential[i - 1u] + potential[i + 1u]
        + potential[i - n] + potential[i + n]
        + potential[i - n * n] + potential[i + n * n]
        - 6.0 * potential[i];
    residual[i] = rhs[i] - laplacian;
}
