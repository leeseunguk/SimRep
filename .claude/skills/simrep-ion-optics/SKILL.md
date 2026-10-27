---
name: simrep-ion-optics
description: SimRep(SIMION 호환 Rust 이온 광학 시뮬레이터)로 형상을 받아 전위를 풀고 이온/전자를 날려 렌즈·정전 분석기(ESA) 성능을 구할 때 따르는 절차. 형상 입수(GEM, 메시, STEP), cad-measure 틈 측정, refine/refine-geometry, 격자 수렴과 u_num, 비행 설정, 32채널 에너지 채널 레시피(k = U/E, FWHM, dE/E), MCP 사용, 실패 진단, 함정. "ESA", "분석기", "analyzer", "에너지 채널", "energy channel", "dE/E", "렌즈", "einzel", "refine-geometry", "cad-measure", "fast adjust", "검출면", "MCP", "simrep fly" 같은 말이 나오면 이 스킬을 먼저 읽는다.
---

# SimRep 이온 광학 작업 (형상 -> 전위 -> 비행 -> 성능)

배경과 근거는 `AGENTS.md`, `docs/python.md`, `docs/step-import.md`, `docs/adaptive-grid.md`, `docs/validation/GATES.md`, `mcp/README.md`, `examples/README.md`에 있다. 여기는 순서와 명령만 적는다.
CLI 이름은 `simrep`이다. 저장소에서는 `./target/release/simrep` (`cargo build --release -p simrep-cli`)이고, `cad-measure`는 `--features cad` 빌드에만 있다.

## 0. 원칙

- 속도 숫자는 `compute_ms`만 비교한다. 파일 읽기, 파싱, GPU 초기화, 기저 로딩, 내보내기는 빠진다. `prep_ms`, `setup_ms`, `total_ms`는 참고용이다.
- WSL과 CI에선 `--device cpu` (`device="cpu"`)를 쓴다. 하드웨어 Vulkan/DX12 장치가 보통 없고, `--device gpu`는 소프트웨어 어댑터를 거부한다.
- 조용한 폴백을 믿지 않는다. 모든 명령에 `--json`을 붙이고 `requested`와 `executed`(장치, 정밀도, `surface_mode`)를 비교한 뒤 `fallback_reason`을 읽는다. 정확 표면을 원했는데 `executed.surface_mode`가 `staircase`면 `.cuts.json`이 안 붙은 것이다.
- 좁은 틈은 해상한다. 전극 사이 틈이 g이면 그 주변 격자 간격 h <= g/10을 맞춘다(전역 `--spacing` 또는 `--refine-region` 패치). 틈에 노드가 두세 개뿐이면 전위는 맞아 보여도 틈 안 전기장이 틀린다.
- 값 하나는 반드시 `값 +/- u_num`으로 낸다. 격자 셋에서 구한 수치 불확도 없이 "수렴했다"고 쓰지 않는다. 솔버 잔차 수렴은 격자 수렴이 아니다.

## 1. 형상 입수

| 입력 | 명령 | 메모 |
|---|---|---|
| GEM | `simrep --json gem2pa lens.gem lens.pa#` | 격자·대칭·거울은 GEM `pa_define`이 정한다 |
| STL/OBJ/PLY/3MF/glTF | `simrep --json mesh-import in.stl out.pa# --grid nx,ny,nz --spacing sx,sy,sz --origin x,y,z --units mm --repair` | 열린 면·비다양체는 보고 후 거부, `--allow-defects`는 의도할 때만 |
| STEP/IGES | `refine-geometry part.step ...` (feature `cad`, OCCT 필요) | 빌드: `cargo build --release -p simrep-cli --features cad` |

STEP을 받으면 풀기 전에 부품 사이 거리부터 잰다.

```sh
simrep cad-measure part.step --all-pairs --json
simrep cad-measure part.step --gap 0 1 --json
simrep cad-measure part.step --gap '0:faces=0,1' '1:faces=0' --touch-tolerance 1e-7 --cad-tolerance 0.01
```

- `touching_matrix`에서 `true`인 쌍은 같은 전극 망(net)으로 묶을 후보다. 같은 번호를 준다. 다만 `touching`은 근접 분류일 뿐 전기적 접촉의 증거가 아니다. 도면이나 설계자에게 확인한다.
- `distance_matrix_mm`에서 서로 다른 망 사이 가장 작은 값이 분석기 틈 g다. 이 g로 h <= g/10을 정한다. 결과의 `closest_points_mm`가 틈 위치이고, `--refine-region` 상자를 거기에 둔다.
- 선택자는 XCAF 경로 또는 0부터 세는 body 번호다. face 번호는 그 파일 안 OCCT face-map 번호이므로 CAD를 다시 내보내면 바뀐다.

전극 배정은 부품마다 정확히 하나, `부품=번호:볼트`다.

```sh
# GEM: 부품 = GEM의 e(n) 값
--electrode 1=1:0 --electrode 2=2:110 --electrode 3=3:0
# 메시/CAD: 부품 이름 또는 0부터 세는 번호
--electrode inner=1:0 --electrode outer=2:0
```

전극 번호는 1..1000이고 저장 전압이 아니다. 쓰지 않는 선택자도 오류다. PA 파일은 원점을 저장하지 않으니 `--origin`을 형상 단계와 비행 단계에서 똑같이 쓴다.

## 2. 전위 풀기

| 경우 | 명령 | 결과 |
|---|---|---|
| 계단 PA (`.pa#`, `.pa`) | `simrep --json refine lens.pa# --device cpu` | `.pa0` + `.pa1..N` fast-adjust 기저 |
| 연속 표면 (GEM, 메시, CAD) | `simrep --json refine-geometry lens.gem --electrode 1=1:0 --electrode 2=2:110 --electrode 3=3:0 --out lens --device cpu` | `.pa#`, `.pa0`, `.pa1..N`, `.cuts.json` |
| 틈 주변만 세분 | 위에 `--refine-region xmin,ymin,zmin,xmax,ymax,zmax:2` (인수 2 또는 4, 반복 가능, 겹침·접촉 금지) | `lens.adaptive.json`, 비행은 `--pa lens.adaptive.json` |

- 분석기·렌즈는 `refine-geometry`를 쓴다. cut-cell로 실제 표면에서 경계 조건을 걸고, 비행 때 `.cuts.json`으로 정확 표면 충돌을 한다. `refine`은 계단 충돌이라 표면이 반 셀까지 안쪽으로 들어온다(`examples/exact_surface.py`: 반지름 8 mm 구에서 계단 충돌 평균 0.36 mm 안쪽).
- 기저는 단위 전압 해이므로 전압만 바꿀 땐 다시 풀지 않는다. CLI는 `fly --voltage 2=150`, Python은 `field.fast_adjust({2: 150})`.
- 수렴 실패는 `--max-cycles`를 올리거나 `--tolerance`를 푼다. 그 전에 떠 있거나 겹친 전극부터 의심한다.

### 격자 수렴 (최소 두 격자, u_num엔 세 격자)

같은 형상을 h, h/2 (가능하면 h/4)로 풀어 같은 물리량(TOF, 통과 전압, 초점 위치)을 비교한다. 두 격자 차이는 감을 잡는 용도이고, `u_num`은 세 격자가 필요하다(두 격자를 넣으면 `observed_order requires exactly three levels`).

```python
from simrep import uncertainty
h = [0.125, 0.25, 0.5]            # 고운 것부터
v = [S_fine, S_mid, S_coarse]     # 같은 물리량
r = uncertainty.grid_study(h, v)  # Eca-Hoekstra, 잡음 있는 열도 처리
print(f"{r['extrapolated']:.9g} +/- {r['u_num']:.2g} (p={r['p']})")
uncertainty.observed_order(h, v)  # Celik GCI: p, extrapolated, gci_fine(상대값)
uncertainty.validation(S, D, u_num, u_input, u_datum)  # E = S-D, u_val
```

`p`가 `None`이면 진동·발산하는 열이다. 격자를 더 곱게 하거나 틈 해상을 다시 본다. `|E| <= u_val`일 때만 실험값과 맞는다고 쓴다.

## 3. 비행

```python
import simrep
ions = simrep.particles_from_energy(
    position_mm=[[1, 0.5, 0], [1, 1.0, 0]], direction=[1, 0, 0],
    energy_ev=500, mass_amu=0.00054857990946, charge_e=-1)   # 전자
r = simrep.fly(field, ions, device="cpu", plane=[1, 0, 0, 58],
               duration_us=1, allow_time_limit=True)
a = r.arrays                       # ion, tof_us, x_mm, y_mm, z_mm, ke_ev, reason, ...
print(r.report["device"], r.report["relativistic"], r.report["compute_ms"])
```

- 종(species)은 질량과 전하로만 정한다. 전자는 `mass_amu=0.00054857990946`, `charge_e=-1`. 양이온은 amu와 +전하. `.ion` 행은 `birth_us,mass_amu,charge_e,x,y,z,azimuth_deg,elevation_deg,energy_ev[,CWF,color]`.
- 상대론이 기본이다(`relativistic=True`, CLI는 `--nonrelativistic`로 끈다). keV 전자는 꼭 켜 둔다. 충돌 모델을 쓰면 비상대론으로 바뀐다.
- 검출면은 무한 평면이다. `--plane nx,ny,nz,d`는 `nx*x+ny*y+nz*z=d` 전체에서 멈춘다. 실제 슬릿·개구 크기는 결과를 걸러서 정한다.

  ```python
  hit = (a["reason"] == "plane") & (abs(a["y_mm"] - y0) <= slit_w / 2) & (abs(a["z_mm"]) <= slit_h / 2)
  ```

- PA 밖을 날아야 하면 `--bounds xmin,ymin,zmin,xmax,ymax,zmax` (`bounds=[[...],[...]]`). 없으면 PA 경계에서 `outside`로 끝난다.
- 시간 한계는 기본이 실패다. `--duration-us`를 늘리거나, 의도한 것이면 `--allow-time-limit`. 종료 이유는 CSV `reason`과 JSON `splat_counts`로 센다.
- 정확 표면 비행은 CPU 전용이다. `--device gpu`로 쓰려면 `--no-cuts`로 계단 충돌을 받아들여야 한다.

## 4. 분석기 에너지 채널 레시피 (ESA)

목표: 채널마다 통과 전압 U_i, 비례 상수 k = U/E, 분해능 dE/E(FWHM).

1. 채널 에너지를 로그 간격 32개로 잡는다: `E_ch = np.geomspace(E_min, E_max, 32)`.
2. 분석기 전극을 기저 하나로 푼다(`refine-geometry`, 2절). 이후 전압은 전부 `fast_adjust`.
3. 채널마다 U를 스캔한다. 추정 k0로 `U = k0*E*np.linspace(0.8, 1.2, N)`. 각 U에서 빔을 날리고 슬릿 크기로 거른 통과율 T(U)를 센다.
4. U_i = T(U)가 반값 이상인 구간의 무게중심. FWHM_U는 반값 교차점 선형 보간.
5. k는 원점 통과 직선으로 맞춘다: `k = sum(U_i*E_i) / sum(E_i**2)`. 잔차 `U_i/E_i - k`가 채널에 따라 기울면 선형 분석기가 아니다(가장자리장, 상대론, 틈 해상 부족).
6. 선형이면 고정 E에서 U로 잰 상대 폭과 고정 U에서 E로 잰 상대 폭이 같다: `dE/E = FWHM_U / U_i`.

```python
import numpy as np, simrep
field = simrep.load_field("esa.pa0")          # refine-geometry --out esa
ANALYZER = 2                                  # 통과 에너지를 정하는 전극 번호

def beam(E, n=64, half_deg=2.0):
    t = np.radians(np.linspace(-half_deg, half_deg, n))
    d = np.stack([np.cos(t), np.sin(t), 0 * t], axis=1)
    return simrep.particles_from_energy(position_mm=SOURCE_MM, direction=d,
        energy_ev=E, mass_amu=0.00054857990946, charge_e=-1)

def transmission(U, E):
    f = field.fast_adjust({ANALYZER: U})
    a = simrep.fly(f, beam(E), device="cpu", cuts="esa.cuts.json",
                   plane=PLANE, duration_us=1, allow_time_limit=True).arrays
    hit = (a["reason"] == "plane") & (abs(a["y_mm"] - Y0) <= SLIT_MM / 2)
    return hit.mean()

def fwhm(x, y):
    i = int(np.argmax(y)); half = y[i] / 2
    lo = np.nonzero(y[:i] < half)[0]; hi = np.nonzero(y[i:] < half)[0]
    if lo.size == 0 or hi.size == 0:
        raise ValueError("peak is not bracketed; widen the scan")
    a, b = lo[-1], i + hi[0]
    return (np.interp(half, [y[b], y[b - 1]], [x[b], x[b - 1]])
            - np.interp(half, [y[a], y[a + 1]], [x[a], x[a + 1]]))
```

`SOURCE_MM`, `PLANE`, `Y0`, `SLIT_MM`, `k0`는 그 분석기의 값으로 채운다.

표본 주의:

- FWHM 안에 스캔 점이 최소 8~10개 들어가야 한다. 거칠면 보간이 폭을 수십 % 흔든다. 처음 넓게 훑고, 피크 주변을 다시 곱게 스캔한다.
- 빔 정의(각도 폭, 시작 위치 폭, 입자 수)가 dE/E를 정한다. 같은 분석기라도 `half_deg`를 두 배로 하면 dE/E가 크게 바뀐다. 보고할 땐 빔 정의를 같이 적는다.
- 통과율은 입자 수 n에 대한 이항 잡음 `sqrt(T(1-T)/n)`을 가진다. 반값 근처 잡음이 FWHM 오차가 된다. n을 두 배로 했을 때 dE/E가 안 바뀌는지 본다.
- 격자 수렴(2절)은 U_i와 dE/E 둘 다에 따로 한다. 하나만 수렴해도 다른 것은 안 될 수 있다.

## 5. MCP로 쓰기

```sh
cargo build --release -p simrep-cli
claude mcp add simrep -e SIMREP_BIN=/absolute/path/to/simrep -e SIMREP_WORKSPACE=/absolute/path/to/workspace -- node mcp/server.mjs
SIMREP_BIN=./target/release/simrep node mcp/smoke.mjs     # 점검: "all checks passed"
```

- 도구: `gpu_info`, `pa_info`, `gem2pa`, `mesh_import`, `refine`, `refine_geometry`, `cad_measure`, `fly`, `export_geometry`, `export_grid`, `export_isosurface`, `export_trajectories`, `read_csv_summary`. 결과는 CLI `--json` 객체 그대로다.
- `device: "cpu"`를 넘긴다. `cad_measure`는 `file`과 `gap: ["a", "b"]` 또는 `all_pairs: true`.
- 경로는 `SIMREP_WORKSPACE` 안만 된다. 시간 한계는 `SIMREP_MCP_TIMEOUT_S`(기본 3600 s).
- 자세한 것은 `mcp/README.md`.

## 6. 실패 진단

| 메시지 (실제 문구) | 원인 | 조치 |
|---|---|---|
| `--device gpu requires a hardware GPU; adapter <name> (<backend>) is a CPU/software adapter; use --device cpu` | WSL, CI, GPU 없음 | `--device cpu` |
| `surface collisions require CPU; use --device cpu or auto, or --no-cuts for staircase GPU flying` | 정확 표면 비행엔 GPU 경로가 없음 | `--device cpu` 또는 `--no-cuts` |
| `refinement did not converge after {cycles} cycles` / `electrode {number} did not converge after {cycles} cycles; increase max_cycles` | 사이클 부족, 떠 있거나 겹친 전극 | `--max-cycles` 올림, `--tolerance` 완화, 전극 제약 확인 |
| `part {i} {name:?} needs exactly one --electrode assignment` | 배정 누락·중복, 쓰지 않는 선택자 | 부품마다 `part=number:voltage` 하나 |
| `electrode {number} has no inside/on-surface node; refine the grid` | 표면 안에 격자 노드가 없음 | 더 고운 `--grid`/`--spacing`, 더 두꺼운 부품 |
| `refine input must have a .pa or .pa# extension` / `refine-geometry input must be .gem, .step, ...` | 명령과 파일 종류 불일치 | 맞는 명령 사용 |
| `STEP/IGES import requires cargo build -p simrep-cli --features cad; see docs/step-import.md` (`cad-measure`는 `CAD measurement requires ...`) | `cad` 기능 없이 빌드 | `--features cad`로 다시 빌드(OCCT 필요) |
| `{n} particles did not reach an allowed termination; inspect CSV reasons` | 시간·스텝 한계 | `--plane`, `--bounds`, `--duration-us`, 또는 `--allow-time-limit` |
| `PA is marked non-refinable (field map)` | 고정 필드 맵 | 원본 `.pa#`을 푼다 |
| `electrode voltages require --pa` | `--voltage`엔 PA0 패밀리가 필요 | `--pa x.pa0` |
| `nonlinear permittivity cannot use PA# fast-adjust bases; refine an ordinary PA at its actual voltages` | 비선형 유전체는 중첩 불가 | 실제 전압의 일반 PA로 풀기 |
| `leader-plane beam integration requires --device cpu or auto` / `direct Coulomb physics requires --device cpu or auto` | CPU 전용 물리 | `--device cpu` |
| `SDS requires --mbmr /path/to/mbmr.dat` | SDS 통계 파일 미포함 | SIMION `mbmr.dat` 경로 |
| Python `ValueError: electrode {n} has no fast-adjust basis` | 풀지 않은 번호를 조정 | `field.voltages.keys()`의 번호만 |
| `observed_order requires exactly three levels` | 격자 두 개로 u_num 요청 | 세 번째 격자를 푼다 |

## 7. 함정 (겪은 것)

- 전압 2배 규약. 분석기 문헌의 U는 판 사이 전위차(내판 -U/2, 외판 +U/2)일 때도 있고 한쪽 판 전압(접지 대비 U)일 때도 있다. 둘이 섞이면 k가 정확히 2배 어긋난다. `--voltage`/`fast_adjust`에 넣는 값은 전극별 절대 전압이다. k를 낼 때 U가 무엇인지 같이 적고, 문헌 k와 비교하기 전에 규약부터 맞춘다.
- 표본 dE/E는 스캔 간격, 입자 수, 빔 각도·위치 폭에 민감하다(4절). 한 번 잰 값을 분석기 고유값처럼 쓰지 않는다. 설정을 바꿔 안정한지 본다.
- cut-cell 표면 근처 전기장은 문서화된 차수에 못 미친다. `docs/validation/GATES.md`에서 `mms-cut-cylinder-field`(기대 1.0, 끝점 0.441, 인접 최소 -0.125), `mms-cut-sphere-field`(기대 1.0, 끝점 0.257)가 "endpoint below documented order"다. 전위 자체(`mms-cut-*-potential`, 1.7~1.8)도 2차에 못 미친다. 표면에서 셀 한두 개 안쪽을 지나는 궤적은 격자를 곱게 해도 오차가 기대만큼 줄지 않는다. 그런 양은 세 격자 `u_num`을 꼭 내고, 틈은 h <= g/10으로 해상한다.
- 정확 표면이 조용히 빠질 수 있다. `fly`와 `load_field`는 PA0 옆 같은 이름의 `.cuts.json`만 자동으로 줍는다. PA0만 옮기거나 이름을 바꾸면 계단 충돌이 된다. 다른 위치면 `--cuts`(`cuts=`)로 넘긴다. JSON `executed.surface_mode`(`source-geometry` 대 `staircase`)로 확인한다.
- WSL에서 Windows exe(`simrep.exe`, `cargo.exe`, `python.exe`, `maturin.exe`)를 부르면 stdin을 닫지 않을 때 멈춘다. 항상 `(cd /mnt/c && timeout 900 <exe> ... < /dev/null)` 꼴로 부른다. 환경 변수는 `WSLENV`로 넘긴다(경로는 `/p`). 자세한 것은 `docs/python.md`의 "Windows build with cargo.exe".

## 8. 검증된 시작점 (einzel, CPU)

```sh
simrep --json gem2pa examples/data/einzel.gem einzel.pa#
simrep --json refine einzel.pa# --device cpu
simrep --json fly --pa einzel.pa0 --particles examples/data/ions.ion \
    --voltage 1=0,2=60,3=0 --plane 1,0,0,58 --duration-us 2000 --device cpu --output results.csv
simrep --json refine-geometry examples/data/einzel.gem \
    --electrode 1=1:0 --electrode 2=2:60 --electrode 3=3:0 --out ez --device cpu
simrep --json fly --pa ez.pa0 --particles examples/data/ions.ion \
    --plane 1,0,0,58 --duration-us 2000 --device cpu --output ez.csv
simrep --json pa-info einzel.pa0
```

판정: `refine`은 `converged: true`, `executed.device: "cpu"`. 첫 `fly`는 `surface_mode: "staircase"`, `ez.pa0` 비행은 `surface_mode: "source-geometry"`(옆 `ez.cuts.json` 자동 사용). 둘 다 `arrivals: 8`, `splat_counts.plane: 8`, 평균 TOF 약 1.1922 us.
