# Optional browser portability

Default desktop features retain native GPU and Lua support. CPU arithmetic and native entry points are unchanged; browser adapters use caller-owned bytes and explicit optional features.

| Crate | Defaults | Browser selection |
| --- | --- | --- |
| simrep-io | `lua` | `default-features = false`: literal FLY2 and byte fast-adjust loader |
| simrep-gem | `lua` | `default-features = false`: literal GEM, includes and STL byte loader |
| simrep-solve | `gpu` | no defaults for CPU; add `webgpu` for async browser f32 corrections |
| simrep-fly | `gpu`, `lua` | no defaults for CPU; add `webgpu` for async packed browser f32 flight |
| simrep-core / simrep-geom / simrep-mesh | unchanged | shared types, geometry and byte mesh readers/writers |

`gpu` means the existing native GPU APIs and their coupled-physics setup. `webgpu` is independent: it enables wgpu/bytemuck and asynchronous readback, not native blocking initialization, pollster, Lua or filesystem IO. Vulkan/DX12/GLSL backend features stay target-specific to native builds. Both feature families can be enabled on native targets for portable-kernel regression tests. Browser consumers must disable defaults. `lua` remains default-on for standalone IO/GEM and native flight; the workspace IO dependency disables implicit defaults so browser consumers can opt out transitively.

On wasm, the CPU modules use web-time for Instant and disable unsupported native thread spawning. The flight-only pow symbol delegates f64 pow to the browser kernel to preserve tested RK45 step decisions. No native math kernel is replaced. Literal-only parsers reject executable Lua rather than evaluating or approximating it. Native APIs keep their original filesystem semantics; the new `*_with` loaders and stream writers are explicit alternatives.

`simrep_solve::webgpu` offers Cartesian staircase f32 V-cycle corrections with a CPU f64 outer iterate/residual. `simrep_fly::webgpu` accepts the explicit static staircase f32 packed flight ABI. It is not a replacement for the high-level desktop fly request. Unsupported GPU f64 and malformed packed inputs fail before dispatch. Exact/SDF browser GPU flight is not exposed.

SimRep-Web owns the browser wrapper, cooperative refinement Channel, worker lifecycle and frontend. The wrapper uses these shared CPU solvers and exporters rather than changing desktop command implementations. Native output identity must be checked using generated PA/basis/cuts/CSV artifacts, not executable build IDs or elapsed timings. Cross-platform trigonometric mesh export can differ by a few ULPs; no universal exported-byte identity is promised.

Reproduction:

```sh
cargo check --target wasm32-unknown-unknown -p simrep-core -p simrep-geom -p simrep-io -p simrep-gem -p simrep-mesh -p simrep-solve -p simrep-fly --no-default-features
cargo check --target wasm32-unknown-unknown -p simrep-fly --no-default-features --features webgpu
GALLIUM_OVERRIDE_CPU_CAPS=avx LP_NATIVE_VECTOR_WIDTH=256 cargo test --release -p simrep-fly --features webgpu portable_einzel_matches_native_f32_and_cpu_splats -- --nocapture
```

The native WebGPU regression may use software Vulkan for correctness only. This does not change the desktop CLI's explicit-hardware-GPU policy or imply a hardware performance measurement.
