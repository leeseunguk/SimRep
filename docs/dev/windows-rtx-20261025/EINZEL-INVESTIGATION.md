# Real-einzel native-f64 replay and dispatch safety

2026-10-25 UTC / 2026-10-26 KST. Investigated `9ae2bb3`; all changes and
records are uncommitted. No commit, push, driver change, TDR registry change,
or unrelated process termination was performed.

## Verdict

**There is a reproducible code defect in the default dispatch size, not an
environmental excuse for the driver reset.** The strict native-f64 CLI
workload submits all 100,000 ions in one dispatch and measured 2308.241 ms.
The next invocation reproduced the driver-reset/zero-result failure.
Restoring the 32,768-ion native default reduces the longest measured dispatch
to 829.175 ms while preserving all 100,000 output rows byte-for-byte.

**The historical 637.090 ms throughput result is not reproduced, and its
introducing regression cannot be bisected in this repository.** Partitioning
fixes the watchdog exposure, not the throughput difference. The fixed
fresh-process CLI median is 2460.147 ms. It would be incorrect to claim the
old throughput was restored, or that the six busy CPU consoles explain it.

## History and attribution limits

`git log -S '637' -- bench/fly-gpu.md` identifies `b8ce386`, "Add design
documents and benchmark records". That commit imports the complete benchmark
document, including later entries superseding the 637 ms study. The entire
flyer implementation arrived earlier in `e9b8c58`, "Add SimRep Rust
workspace". Searches for `PREFERRED_IONS_PER_DISPATCH`, `unroll_horner`,
`EndState`, and `imported_maps` all identify that same code import.
No pre-import implementation commits are reachable, and `git fsck
--no-reflogs --unreachable` found no unreachable commits.

The production GPU files `mod.rs`, `flight.rs`, `precision.rs`, `fly.wgsl`,
and `imported_map.rs` are byte-identical between `b8ce386` and `9ae2bb3`.
`mod.rs`'s pre-fix SHA-256 at both the code import and main was
`53f7b55432cf6e9c49d6b80b3f26cd26ae6a1ef61533e3cbbdb25c0ae5efaf3d`.
Thus a Git bisect between the commit containing the record and main has no
production GPU changes to isolate. The named `be-gpu-kernel`,
`be-precision-pipe`, and `be-bmap-import` development commits are not present.
The later no-silent-fallback CLI changes do not alter field construction or
GPU flight arithmetic for this explicit hardware Vulkan request.

The independent source-level controls below exclude x-Horner unrolling,
nearest-node versus all-corner termination, and unit-basis subtraction
roundoff as explanations for the approximately 3.9x historical delta.
They do not reconstruct the unavailable pre-import kernel.

## Workload and machine state

- PA: `%USERPROFILE%\simrep-oracle\_benchfly\einzel.pa0`; 91 x 20 x 1,
  degrees `[6,6,1]`, four resident factors.
- Input: the original `einzel_100000.ion`, voltages `1=0,2=110,3=0`, bounds
  `-101,-19,-19,291,19,19`.
- CLI defaults: relativistic RK45, 100 us duration, 1e-12 position/velocity/
  relative tolerance, 0.001 us initial step, automatic maximum step.
- Adapter: NVIDIA GeForce RTX 3070, PCI `0000:0a:00.0`, DiscreteGpu, Vulkan,
  NVIDIA 591.86, native f64 and GPU pass timestamps. DX12 was not selected.
- Before the first replay: 0% GPU utilization, P8, 33 C.
- During the fixed native workload: P0, 1935 MHz SM, 7001 MHz memory,
  100% GPU utilization, 39-42 C, approximately 99-106 W. There is no observed
  thermal throttling in this interval.
- Six pre-existing conhosts each consumed approximately one CPU core.
  They have live WSL parents, were present in the preceding verification,
  and were not killed. CPU load is recorded, not silently normalized away.
- No competing GPU benchmark was run. Linux checks used CPU/llvmpipe.

All reported compute values are GPU pass timestamps, excluding preparation,
compilation, upload, combination, readback, CSV and process startup.
The ignored opt-in `real_einzel_dispatch_benchmark` uses one identical warmup
and three measured resident flights. All measured runs must reproduce the
warmup's physical terminations and every final-state/TOF/step bit.

## Compute evidence

| Variant | Three compute samples ms | Median ms |
| --- | --- | ---: |
| Current unrolled kernel, explicit 32,768 cap, before fix | 2458.923392 / 2459.060256 / 2458.931232 | 2458.931232 |
| General x-Horner loop, same cap and inputs | 2544.530560 / 2545.363040 / 2543.499776 | 2544.530560 |
| Nearest-node collisions, current kernel, same cap | 2475.120768 / 2474.406528 / 2475.015232 | 2475.015232 |
| Direct normalized PA1..3 bases, same cap | 2471.267232 / 2470.912544 / 2471.745824 | 2471.267232 |
| Fixed native default, unchanged CLI-equivalent basis construction | 2459.864000 / 2458.301216 / 2459.496928 | 2459.496928 |
| Fixed CLI, three fresh processes | 2475.301 / 2459.472 / 2460.147 | 2460.147 |

Unrolling improves the bounded workload by 3.36%, rather than causing its
slowdown. Direct basis normalization is an investigative control only; its
slightly changed step counts are not shipped.
Production construction retains 53,205,564 accepted and 24,727,011 rejected
steps. Nearest-node collision selection produces those same counts.

The fixed default's individual dispatches measured:

| Dispatch | Ions | GPU compute ms |
| --- | ---: | ---: |
| 0 | 32,768 | 812.016192 |
| 1 | 32,768 | 670.320448 |
| 2 | 32,768 | 829.174752 |
| 3 | 1,696 | 146.733184 |

The sum is separate from setup and transfer. Each partition replay compares
all final states, velocities, TOFs, termination reasons/indices and accepted/
rejected counts bitwise. Each of the three fixed CLI CSVs is byte-identical
to the successful pre-fix 100,000-ion monolithic CSV. All 100,000 ions splat.

There is no valid three-run median for the unsafe 100,000-ion dispatch.
The previous report's 2349.544 ms and this investigation's 2308.241 ms are
successful individual samples, not a median. Failed zero timestamps are
invalid, not performance samples.

## Driver reset and reporting

The second pre-fix CLI invocation (with `--nearest-node-collisions` for the
historical geometry control) exited 1 after producing 100,000 zero-state
`nonfinite` rows, zero accepted/rejected steps, and zero GPU timestamps.
Its only final error was the generic "100000 particles did not reach an
allowed termination; inspect CSV reasons". Exit status was honest, but the
device-loss cause and invalid timing were not clear.

The System log confirms a new reset, distinct from the original report:

```text
2026-10-25T16:52:12.5066372Z id=153 provider=nvlddmkm
2026-10-25T16:52:13.5487377Z id=4101 provider=Display
Display driver nvlddmkm stopped responding and has successfully recovered.
```

The minimal fix:

1. Native-f64 prepared fields now default to 32,768 ions per dispatch.
   Hardware Preview keeps 131,072; physics keeps its 4,096-ion/32-trial
   continuation limit. The explicit capacity API remains available.
2. Completed readbacks must contain terminal reason codes 1 through 7.
   Unfinished/zero-filled output returns an error before publishing timings
   or particle results:

```text
GPU flight returned unfinished terminal states (possible device loss); compute timestamps and results are invalid
```

The new regression executes a real no-write GPU pipeline and verifies that
both precisions reject the resulting unfinished readback. It does not reset
the driver deliberately. Existing valid numerical `NonFinite` terminations
remain numerical terminations; this check rejects missing terminal writes.

The cap is a measured safe default for this workload, not a universal time
bound for arbitrary durations or deliberately enlarged capacities. A hard
duration bound for all electric work would require the same kind of bounded
trial continuations already used by physics. That broader scheduling change
is not part of this minimal fix.

## Validation and reproduction

`cargo fmt --all -- --check`, `git diff --check`, and
`cargo clippy -p simrep-fly --all-targets -- -D warnings` exit 0.
The language-server diagnostic requests timed out; compiler and Clippy
diagnostics are clean.

Windows release GPU-module checks: **14 passed, 0 failed, 10 explicitly
ignored fixture-dependent benchmarks**. The new incomplete-readback check,
pending-flight ownership/reuse, batching/RNG, both-precision magnetic-map and
coil/Boris cases pass. The real-einzel benchmark was separately requested
with `--ignored` and passes. The release CLI build and all three fresh-process
CLI replays exit 0.

`RAYON_NUM_THREADS=16 cargo test -p simrep-fly --release -- --nocapture`
exits 0: **107 tests passed, 15 explicitly ignored external-fixture tests**,
and the standalone GPU PIC check passes on llvmpipe. Windows's separate
electric `--test gpu` check also passes on the RTX 3070.

The full native `kernel_experiments` replay passes every bitwise comparison
of endpoints, velocities, TOFs, hit classes/indices, step counts and 300
sampled trajectories. The safer default has a measurable throughput cost
for known-short resident flights:

| Resident native 100k workload | Previous 131,072-cap record ms | New 32,768-cap median ms |
| --- | ---: | ---: |
| Einzel | 194.637760 | 215.930048 |
| Quadrupole | 490.027552 | 551.358432 |

The respective new samples are 215.930048 / 215.958592 / 215.915712 ms and
551.358432 / 551.666336 / 551.006272 ms. This approximately 11% / 13%
native throughput cost is an intentional safety trade-off, not an unreported
performance win. Callers with measured short workloads can explicitly opt
back into a larger cap. Hardware Preview's default partition is unchanged.

Raw logs/CSVs are retained outside Git under
`%USERPROFILE%\simrep-verify-20261025\einzel-*`.
Reproduce the bounded resident control:

```sh
manifest="$(wslpath -w "$PWD/Cargo.toml")"
export CARGO_TARGET_DIR='%USERPROFILE%\simrep-target'
export SIMREP_GPU_REAL_EINZEL='%USERPROFILE%\simrep-oracle'
export WSLENV=CARGO_TARGET_DIR:SIMREP_GPU_REAL_EINZEL:SIMREP_GPU_REAL_CAP:SIMREP_GPU_REAL_GENERAL:SIMREP_GPU_REAL_NEAREST:SIMREP_GPU_REAL_DIRECT
set -o pipefail
cd /mnt/c
timeout 180 /mnt/c/Users/<user>/.cargo/bin/cargo.exe test \
  --manifest-path "$manifest" -p simrep-fly --release --lib \
  real_einzel_dispatch_benchmark -- --ignored --nocapture --test-threads=1 \
  </dev/null 2>&1 | tr -d '\r'
```

`SIMREP_GPU_REAL_CAP=32768` selects the old partition explicitly;
`SIMREP_GPU_REAL_GENERAL=1` disables specialization; `SIMREP_GPU_REAL_NEAREST=1`
selects the historical collision rule. `SIMREP_GPU_REAL_DIRECT=1` is the
non-shipping basis-normalization control.
Every Windows process ran in the foreground under a timeout from `/mnt/c`,
with closed stdin, piped output, and explicit WSLENV. No detached `start`
or `Start-Process` was used.

## Final Windows cleanup

The final exact-name scan found **0 task-owned Cargo, Rust compiler, CLI,
GPU-test, or nvidia-smi processes**. No taskkill was needed: every owned
foreground executable had exited. If a timeout had left a process behind,
only its actual PID tree would have been killed with `taskkill /T /F`.
There were **0 new nvlddmkm/Display reset events after 16:53:00 UTC**, when
the bounded replays began.

The last Windows invocation was `tools/win/winproc.ps1 cleanup`, from `/mnt/c`
with closed stdin and a foreground 45-second timeout. It exited 0. Full output:

```text
== process counts ==

Name           Count
----           -----
cmd.exe            4
conhost.exe       27
node.exe           4
powershell.exe     1

== conhost by parent ==

Name                              Count
----                              -----
wsl.exe                               7
cmd.exe                               4
wslhost.exe                           3
node_repl.exe                         2
lmgrd.exe                             2
node.exe                              1
powershell.exe                        1
rlm.exe                               1
GoogleDriveFS.exe                     1
sunshine.exe                          1
dispatcher.exe                        1
codex.exe                             1
wlanext.exe                           1
matlab-mcp-server-windows-x64.exe     1

orphan conhost: 0
busy conhost (>50% of a core): 6
  pid 26152 cpu 100% parent wsl.exe (49640) wsl.exe -d Ubuntu --exec sh -s -- $REPO
  pid 52060 cpu 100% parent wsl.exe (33024) wsl.exe -d Ubuntu --exec sh -c "cd '/mnt/d/Private/Opera_test' && export PATH=\"$HOME/.local/bin:$PA
  pid 29192 cpu 100% parent wsl.exe (33024) wsl.exe -d Ubuntu --exec sh -c "cd '/mnt/d/Private/Opera_test' && export PATH=\"$HOME/.local/bin:$PA
  pid 56704 cpu 100% parent wsl.exe (33024) wsl.exe -d Ubuntu --exec sh -c "cd '/mnt/d/Private/Opera_test' && export PATH=\"$HOME/.local/bin:$PA
  pid 69860 cpu 100% parent wsl.exe (33024) wsl.exe -d Ubuntu --exec sh -c "cd '/mnt/d/Private/Opera_test' && export PATH=\"$HOME/.local/bin:$PA
  pid 5832 cpu 100% parent wsl.exe (33024) wsl.exe -d Ubuntu --exec sh -c "cd '/mnt/d/Private/Opera_test' && export PATH=\"$HOME/.local/bin:$PA
stopped orphan conhost: 0
```
