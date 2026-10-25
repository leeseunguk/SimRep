# Windows RTX 3070 verification

Run: 2026-10-25 UTC / 2026-10-26 KST. Source:
initially `0b64131bb781f6f09bde8ff3419cb0b9a8902eea`, `main`.
This verification made no application source changes, commits, or pushes.
The lead concurrently advanced main to `d293d03e7d3fc9e8be67d5f356a21e206b8c456f`:
oracle tool path configuration, Tauri Rust formatting, and Compare-tab
colors/normalized log-axis fixes. The root simulation crates and manifests
did not change, so their native/Python/compute evidence applies to both
revisions. The live frontend was served after that advance. The installer
was rebuilt and its lifecycle repeated for the advanced main.

The requested native suites, Python hardware parity, representative compute
controls, live app measurements, and CAD installer lifecycle were exercised.
The additional historical real-einzel CLI replay failed after a Windows display
driver reset. It must not be reported as a successful timing median.

## Machine and load

Windows Vulkan and DX12 selected NVIDIA GeForce RTX 3070, DiscreteGpu.
Vulkan exposes native f64; DX12 does not. The host is the Ryzen 9 5950X.
The serial compute run started at 2026-10-25 16:22:22 UTC.
Before it, nvidia-smi reported GPU utilization 0%, memory utilization 0%,
1,180 MiB resident, and 12.65 W.

This was not an idle CPU host. Six pre-existing, live-parent conhosts
(26152, 52060, 29192, 56704, 69860, 5832) each used about 98% of one core.
Other sampled CPU loads were CompatTelRunner 93%, System 83%, vmmemWSL 67%,
and dllhost 31%, in the Windows per-core percentage convention. They were
retained; none was a task-owned process. GPU workloads were run serially after
the builds and accuracy suites had finished and the installed app was closed.

## 1. Native GPU tests

Command: Windows `cargo.exe test --release -p simrep-fly -p simrep-solve --
--nocapture`, using the root manifest and `C:\Users\lsu10\simrep-target`.
Exit code 0. Rust harness totals: **152 passed, 0 failed, 14 ignored**.
Those totals include CPU tests and exclude the custom harness checks below.

| Check | Passed | Failed | Ignored / skipped | Adapter/backend |
| --- | ---: | ---: | ---: | --- |
| simrep-fly library, all tests | 33 | 0 | 9 ignored | Ten actual GPU compute tests on RTX 3070 Vulkan; other tests include CPU/helpers |
| simrep-solve library, all tests | 10 | 0 | 0 | Includes CPU/shader-validation helpers |
| Flight integration `tests/gpu.rs` | 6 | 0 | 0 | RTX 3070 Vulkan |
| GPU PIC custom harness | 1 | 0 | 0 | RTX 3070 Vulkan |
| Dielectric custom harness | 24 | 0 | 0 | DX12 8, Vulkan 16 |
| Multigrid custom harness | 24 | 0 | 0 | DX12 8, Vulkan 16 |
| Poisson custom harness | 18 | 0 | 0 | DX12 6, Vulkan 12 |
| Residual smoke custom harness | 3 | 0 | 1 skip | DX12 f32; Vulkan f32/f64; expected DX12 f64 skip |
| Seven magnetic reference gates, separate ignored-test replay | 7 cases | 0 | 0 | RTX 3070 Vulkan native f64 |

The custom harness total is **70 checks passed, 0 failed**, plus the expected
DX12 `SHADER_F64 unavailable` smoke skip. The magnetic replay is one Rust test
containing seven case gates, not seven independently registered Rust tests.
Its command used `SIMREP_GPU_PHYSICS=C:\Users\lsu10\simrep-oracle\physics-validation-20261023`
and `--lib gpu_physics_reference_agreement -- --ignored --nocapture`; exit 0,
65.80 s. Maximum errors:

| Case | Endpoint mm | TOF us |
| --- | ---: | ---: |
| magnet1 | 2.160691333307e-8 | 6.245475248079e-9 |
| mag90 | 5.258515373950e-9 | 4.986411283880e-11 |
| sector_2dp | 3.301033862238e-9 | 7.550937652923e-11 |
| sector_2dc | 2.500263918375e-9 | 2.470024185186e-11 |
| sector_3dp | 8.776549926811e-8 | 4.639923645300e-8 |
| helmholtz | 3.521652320939e-11 | 6.516662209854e-14 |
| solenoid | 4.313371398755e-7 | 3.956928118498e-10 |

Every gate satisfied the retained old-API endpoint and TOF bounds.

## 2. Windows Python

Built the actual Windows abi3 wheel with the existing Windows venv's
`Scripts\maturin.exe build --release`, installed it with
`pip install --force-reinstall --no-deps`, and built the matching current CLI.
Wheel, pip install, and CLI build each exited 0.

- Venv: `C:\Users\lsu10\simrep-pyvenv`.
- Wheel: `C:\Users\lsu10\simrep-wheels\simrep-0.1.0-cp310-abi3-win_amd64.whl`.
- CLI: `C:\Users\lsu10\simrep-target-py\release\simrep.exe`, forwarded as `SIMREP_BIN`.
- Full `python -m pytest crates/simrep-py/tests -v -ra`: **86 passed, 0 failed,
  1 skipped**, exit 0, 104.82 s.
- The only skip was the software-adapter refusal test, which requires software
  Vulkan and is inapplicable on this hardware adapter.

All six hardware numeric parity cases ran and passed:

| Hardware numeric parity case | Result |
| --- | --- |
| GPU PIC versus CLI | PASS |
| Magnetic PA/coils, `exact=False`, GPU | PASS |
| Magnetic PA/coils, `exact=True`, GPU | PASS |
| GPU HS1 versus CLI | PASS |
| GPU SDS versus CLI | PASS |
| GPU sampled trajectories preserve CLI terminal results | PASS |

## 3. Representative compute remeasurement

No initialization, field preparation, shader compilation, or file I/O is
included in the compute-only column. Flight values are GPU pass timestamps.
Refinement uses the solver's reported solve interval; its upload/readback are
reported separately. Values are three-run medians.

| Workload | Current compute-only ms | Stored comparable value ms | Change |
| --- | ---: | ---: | --- |
| 259^3 refine, both PA_ and PA500, tolerance 1e-12 | 479.870 | Pure solve interval not separately published | See matching published interval below |
| Einzel 100k native f64, latest retained kernel control | 194.857184 | 194.637760 | +0.113% |
| Quadrupole 100k Preview, latest retained kernel control | 55.180608 | 54.886016 | +0.537% |
| HS1 100k, native f64, default field-free 50 us profile | 3814.632640 | 3841.686720 | -0.704% |

The refine comparison in `bench/refine-real.md` is explicitly **solve including
transfers**, 736.262 ms. The matching interval here is **742.495 ms (+0.847%)**,
not the 479.870 ms pure solver interval. Its three samples were
756.156 / 740.527 / 742.495 ms; pure solver sums were
484.441 / 474.699 / 479.870 ms. Both bases took 16 cycles and reproduced the
stored residuals: 5.747655981e-13 and 4.175831236e-13 relative.
The median setup was 887.983 ms, separately excluded.
Using the stored SIMION 14.23 s interval, the matching published speed ratio
is 19.17x versus README's rounded 19.3x, within 1%.

The kernel controls used the unchanged `kernel_experiments` test, solved files
under `C:\Users\lsu10\simrep-gui-test`, five warmups per candidate/population,
three measured flights, and 300 sampled trajectories. Both precision runs
passed all bitwise state/TOF/hit/step/sample comparisons against the general
kernel. Their medians are the `unroll_x` results, not the slower general control.
The native einzel 100k samples were 194.857280 / 194.857184 / 194.847360 ms.
Preview quadrupole 100k samples were 54.925408 / 55.180608 / 55.299104 ms.

HS1 used `gpu_physics_compute_benchmark`, `SIMREP_GPU_TIMING=1`,
`SIMREP_GPU_CASE=hs1`, counts 1000,10000,100000. GPU medians were
129.214272 / 436.529056 / 3814.632640 ms, respectively. The 100k GPU samples
were 3812.826688 / 4102.583520 / 3814.632640 ms. Its test exited 0.
The README's HS1 54x figure is a CPU measurement sourced through
`bench/default-flight.md`, not a 100k GPU baseline.

**No matched representative median regressed by more than 10%.**

### Additional historical real-einzel CLI failure

The distinct real-einzel workload in `bench/fly-gpu.md`'s native precision
study was also attempted, with `_benchfly/einzel.pa0`,
`einzel_100000.ion`, voltages `1=0,2=110,3=0`, and bounds
`-101,-19,-19,291,19,19`. This is not the 194.857 ms resident-template control.

| Run | GPU compute ms | Physical splats | Result |
| --- | ---: | ---: | --- |
| Warmup/first run | 2349.544 | 100000 | Exit 0; +268.8% versus stored 637.090 ms single value |
| Next run | Invalid: reported zero | 0 | Exit 1; all 100000 CSV rows `nonfinite`, zero state/steps |

Windows System log recorded nvlddmkm event 153 at 2026-10-25 16:22:35 UTC
and Display event 4101 at 16:22:36 UTC:
`Display driver nvlddmkm stopped responding and has successfully recovered.`
There is **no valid three-run median** for this CLI workload.
No invalid zero timestamp was treated as a performance improvement.
No application fix, timeout-policy change, or system driver setting was made.
Output CSVs remain in `C:\Users\lsu10\simrep-verify-20261025\einzel-{0,1}.csv`.

## 4. App live re-fly

Current release native app, `NODE_ENV=production` Vite, real WebView2 at
1440x900, 300 drawn trajectories, the app's current template beams, five-second
cosine voltage drags. Every animation frame supplied a distinct voltage;
all 278 supplied changes are recorded per case. Results count new flight
objects applied to the store, not requests sent. All measured results used
RTX 3070 Vulkan f32 with no CPU fallback; path CPU time was 0.

| Template / ions | Old updates/s | Current updates/s | Rate change | Old / current round-trip median ms | Current compute median ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| Einzel 10k | 62.2 | 55.59 | -10.62% | 20.3 / 16.6 | 4.285408 |
| Einzel 100k | 17.3 | 19.58 | +13.17% | 99.2 / 135.0 | 23.403456 |
| Quadrupole 10k | 19.2-24.9 | 55.37 | +122.35% versus old upper bound | 43-55 / 25.8 | 9.702784 |
| Quadrupole 100k | 5.0 | 16.37 | +227.36% | 362 / 167.4 | 55.975264 |

The offered animation-frame rate was **55.6 Hz**, versus about 64.8 Hz in
the historical raw record. Einzel 10k applied all 278 changes and quadrupole
10k applied 277. Their current rates are input-limited. Flag the -10.62%
observed einzel 10k rate, but do not attribute it to packet growth: its
round-trip latency improved and the input ceiling is lower.
Einzel 100k round-trip latency increased **36.09%**, despite improved throughput.
These are current-versus-recorded measurements, not an isolated 21-versus-41
B/ion A/B experiment.

An initial bounded run quantized the small quadrupole voltage range to 0.1 V
and therefore offered only about 40 distinct changes. It was rejected as an
input-capped harness run and replaced by the distinct-per-frame measurement
above. `live-results.json` contains the valid run, including all beam settings.

## 5. CAD NSIS installer

Built production frontend, staged OCCT, and ran the documented Windows Tauri
CLI build with `--features cad --bundles nsis`, CAD config, and the external
no-before-build config. All exited 0. Rust compilation had no warnings; Tauri
printed its existing `.app` identifier/macOS naming recommendation.

| Check | Observed result |
| --- | --- |
| Current installer | `C:\Users\lsu10\simrep-app-target-p6cad\release\bundle\nsis\SimRep_0.1.0_x64-setup.exe` |
| Size | **30,219,343 bytes (28.82 MiB)** for refreshed `d293d03`; initial verification build 30,225,716 bytes; historical package 29,873,560 bytes |
| Before installation | No install directory, HKCU/HKLM uninstall keys, SimRep keys, or shortcuts |
| Silent `/S` install | Refreshed package exit 0, 4.2 s; 54 files, 96.8 MiB; 39 OCCT DLLs, 3 CRT DLLs, notices/licenses |
| Per-user scope | HKCU uninstall/SimRep keys and user shortcuts only; no HKLM or ProgramData shortcuts |
| Launch | Real installed executable; WebGPU nvidia ampere and native RTX 3070 Vulkan visible |
| STEP import | `pair.step`, two closed valid B-rep solids; imported through native-path drop handler |
| Cut-cell/refine | Refreshed package: 100 V on electrode 1; 24x12x12, 268 cut nodes; build 48 ms; CPU refine 27 cycles, residual 7.5e-11, 30 ms |
| Fly | 200 ions, exact surfaces, CPU f64; 144 electrode-1 hits, 56 +X-edge hits; mean TOF 0.05848 us |
| DLL origin | All 30 loaded OCCT toolkit DLLs came from `%LOCALAPPDATA%\SimRep\occt`, not the SDK |
| Close/uninstall | Task-owned app/WebView2 tree killed with `/T /F`; silent uninstall exit 0, 1.3 s; no uninstaller processes left |
| After uninstall | Install folder, HKCU uninstall/SimRep keys, and shortcuts gone; no HKLM/ProgramData artifacts |
| Retained user data | `%LOCALAPPDATA%\dev.simrep.app` remains, as intended |

The current flight's 0.05848 us mean differs from the older prose's 0.08756 us.
The older prose does not fully specify its beam. This smoke verifies the
current default beam/exact-surface workflow, not matched-input TOF parity.
Only STEP was requested and rerun; the earlier IGES smoke was not repeated.

The refreshed installer SHA-256 is
`0d3e177cc2b7aad168136c31ba50cbc7667c1fae00e75ebe46e9c3a0234cd681`.
`installed-smoke-refreshed.json` records all four current-main UI stages:
two-part inspection, grid, completed refinement, and completed flight.

The prior native file-dialog automation stalled while sending characters.
Its task-owned dialog was cancelled; STEP import then succeeded via the app's
actual native drag/drop import handler. No application code was patched.

## Recorded files and validation

- `REPORT.md`: counts, timing samples, regressions, installer results, cleanup.
- `live-results.json`: valid four-case measurement with source settings.
- `live-quadrupole-100k.png`: real app GPU flight, inspected visually.
- `installed-final-dom.json`, `installed-step-flight.png`, and
  `installed-smoke-refreshed.json`: refreshed installed workflow,
  inspected visually.
- `cdp.mjs`, `live.js`, `installed-import.js`, `installed-refresh.js`: evidence drivers.

`node --check cdp.mjs` passed; live and import bodies are evaluated by the CDP
driver as async function bodies. The TypeScript LSP server was unavailable;
no application source was changed and the production frontend's TypeScript
check/build passed. Existing benchmark and published-claim source files were
not rewritten with these new measurements.

## Final Windows cleanup

The last Windows invocation was the mandated foreground, bounded command:

```sh
s="$(wslpath -w tools/win/winproc.ps1)"
(cd /mnt/c && timeout 30 powershell.exe -NoProfile -NonInteractive \
  -ExecutionPolicy Bypass -File "$s" cleanup < /dev/null)
```

Exit code 0. No task-owned cargo, rustc, Python, simrep, app, or WebView2 process
remains. The task-owned Vite server was also stopped. Full cleanup output:

```text
== process counts ==

Name           Count
----           -----
cmd.exe            4
conhost.exe       28
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
powershell.exe                        1
rlm.exe                               1
CompatTelRunner.exe                   1
node.exe                              1
sunshine.exe                          1
dispatcher.exe                        1
wlanext.exe                           1
codex.exe                             1
matlab-mcp-server-windows-x64.exe     1
GoogleDriveFS.exe                     1

orphan conhost: 0
busy conhost (>50% of a core): 6
  pid 26152 cpu 98% parent wsl.exe (49640) wsl.exe -d Ubuntu --exec sh -s -- /home/lsu10/orca/workspaces/SIMION/SIMION_Rep
  pid 52060 cpu 98% parent wsl.exe (33024) wsl.exe -d Ubuntu --exec sh -c "cd '/mnt/d/Private/Opera_test' && export PATH=\"$HOME/.local/bin:$PA
  pid 29192 cpu 98% parent wsl.exe (33024) wsl.exe -d Ubuntu --exec sh -c "cd '/mnt/d/Private/Opera_test' && export PATH=\"$HOME/.local/bin:$PA
  pid 56704 cpu 98% parent wsl.exe (33024) wsl.exe -d Ubuntu --exec sh -c "cd '/mnt/d/Private/Opera_test' && export PATH=\"$HOME/.local/bin:$PA
  pid 69860 cpu 98% parent wsl.exe (33024) wsl.exe -d Ubuntu --exec sh -c "cd '/mnt/d/Private/Opera_test' && export PATH=\"$HOME/.local/bin:$PA
  pid 5832 cpu 98% parent wsl.exe (33024) wsl.exe -d Ubuntu --exec sh -c "cd '/mnt/d/Private/Opera_test' && export PATH=\"$HOME/.local/bin:$PA
stopped orphan conhost: 0
```
