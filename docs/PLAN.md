# SimRep 개발 계획 (네이티브 우선)

작성일: 2026-10-23

## 현재 상태 (2026-10-25)

이 문서는 초기 설계를 보존한 것으로, 일부가 현재 구현과 다르다. 구현된 방정식의 현재 명세는 [`spec/equations.md`](spec/equations.md)를 본다. 달라진 점:

- **전기장 재구성**: 3.2의 tricubic/텍스처 설계 대신, 전극 경계를 아는 칸은 축별 6계수 Hermite 재구성(5차, C2), 전극이 없는 격자는 global cubic을 쓴다(`crates/simrep-fly/src/field.rs`). GPU는 텍스처 샘플링이 아니라 셰이더에서 다항식을 직접 평가한다.
- **GPU 셰이더 언어**: f32 커널은 WGSL이지만, WGSL에 f64 타입이 없어 f64 잔차·솔브·PIC 커널은 GLSL을 Naga로 변환한 `.comp` 파일이다(`crates/simrep-solve/src/gpu/`).
- **앱 위치**: `crates/simrep-app` 크레이트는 없다. Tauri 코어는 `app/src-tauri`, 프런트엔드는 `app/src`다.
- **완료된 항목**: 아래 2단계에서 '남음'으로 적은 `tof`와 RF 검증은 완료됐다(`bench/fly-rf-tof.md`, `bench/default-flight.md`). 현재 workspace 크레이트는 11개이며 GEM·메시·CAD가 각각 전용 크레이트(`simrep-gem`, `simrep-mesh`, `simrep-cad`)로 나뉘었다.
- 아래에서 더 이상 현재 상태를 설명하지 않는 줄에는 '(대체됨 → ...)' 표시를 붙였다.

## 1. 목표

- SIMION-2024의 핵심 기능(전위 배열 계산, 이온 비행, 기하 입력, 시간 가변 전압, 공간전하)을 수행하는 네이티브 시뮬레이터.
- **속도**: 같은 문제에서 SIMION보다 빠르다. 기준은 RTX 3070 + Ryzen 9 5950X.
  - **비교는 정밀도를 맞춰서 한다(2026-10-23 사용자 결정).** refine은 SIMION 엄격 설정(`--convergence 1e-7`, SIMION이 허용하는 최소값)과 비교한다. SIMION 기본 설정은 엄격 결과와 최대 0.92 V 차이가 나므로 속도 기준으로 쓰지 않는다.
  - 이온 비행 정확도는 우리 쪽 격자 수렴 기준(8배 격자)으로 판정한다. SIMION의 전극 근처 처리는 비공개라 SIMION 일치가 정답 기준이 아니다.
  - **속도 비교는 실제 계산 시간만으로 한다(2026-10-23 사용자 결정).** GPU 초기화, 파일 읽기, 전위장 재구성·업로드, 결과 출력 같은 준비 시간은 판정에서 뺀다. SIMION도 자체 보고하는 계산 시간(refine 내부 시간, fly compute)과 비교한다. 준비 시간은 참고용으로만 보고한다.
- **정밀도**: SIMION과 같거나 더 높다. SIMION 방식(계단형 전극 + SOR + 선형 보간)에 묶이지 않고, 정밀도가 더 좋은 수치 기법을 쓴다.
- Lua 사용자 프로그램은 지원하지 않는다. 시간 가변 전압, 검출, 기록은 설정 + 수식 DSL로 대체한다.
- **GUI는 아주 쉬워야 한다.** SIMION의 GUI(모달 창, 숫자 입력 중심, 배치 흐름)를 따라가지 않는다. 열고, 보고, 끌어서 바꾸면 바로 결과가 보이는 흐름을 목표로 한다. GUI는 마지막 단계가 아니라 계산 코어가 나오는 즉시(2단계부터) 같이 만든다.
- 웹 포팅은 나중 선택지로 남긴다. 그래서 GPU API는 wgpu(WGSL)를 쓴다. 같은 셰이더가 네이티브(Vulkan/DX12)와 WebGPU에서 돈다.

## 2. 환경과 제약 (확인한 사실)

| 항목 | 상태 |
|---|---|
| WSL2 Rust | cargo/rustc 1.95.0 설치됨 |
| Windows Rust | `%USERPROFILE%\.cargo\bin\cargo.exe` 설치됨 |
| GPU | RTX 3070 8GB, 드라이버 591.86. FP64는 FP32의 1/64 |
| WSL2 GPU API | `libcuda.so`만 있음. Vulkan(vulkaninfo) 없음, nvcc 없음 |
| SIMION | `C:\Program Files\SIMION-2024`. 문서는 `docs/simion.chm`, 예제 폴더 약 80개 |

결론:
- **GPU 실행은 Windows 네이티브 빌드로 한다.** WSL2에는 Vulkan이 없어서 wgpu가 GPU를 못 잡는다. CPU 경로 개발과 테스트는 WSL에서, GPU 벤치마크와 통합 테스트는 `cargo.exe`로 Windows에서 실행한다.
- 저장소를 `\\wsl.localhost` 경로로 Windows에서 빌드하면 매우 느리다. Windows 빌드용으로 `C:\` 쪽에 체크아웃을 하나 더 두거나 `CARGO_TARGET_DIR`을 C: 드라이브로 둔다.
- **conhost 관리**: WSL에서 띄우는 모든 Windows 실행 파일(cargo.exe, simion.exe, node, tauri)은 conhost.exe를 만든다. 분리 실행(`start`) 금지, 모든 명령에 타임아웃, 장기 실행은 `taskkill /T /F`로 트리 종료, 작업 후 `tools/win/winproc.ps1 cleanup`(고아 conhost만 정리)과 프로세스 수 보고. 앱 자체도 백엔드 작업 프로세스를 콘솔 창 없이(`CREATE_NO_WINDOW`) 띄우고 앱 종료 시 Job Object로 자식 프로세스를 함께 정리한다.
- SIMION 바이너리는 역분석하지 않는다(EULA). `simion.exe --nogui` 배치 실행과 예제 파일을 정답(oracle)으로만 쓴다. 포맷과 알고리즘은 `simion.chm`과 공개 문헌에서 가져온다.

## 3. 핵심 설계 결정

### 3.1 전위 계산 (Refine)

SIMION: 균일 격자, 계단형 전극, SOR 반복, f64.
여기서는 단계적으로 간다.

1. **기준 솔버 (1단계)**: 균일 격자 + 기하 멀티그리드(V-cycle, GPU, f32 저장).
   - 정밀도는 **반복 잔차 보정(iterative refinement)** 으로 확보한다. V-cycle은 f32로 돌리고, 잔차 `r = b - A·x`는 f64로 계산해 보정한다. 잔차 계산은 사이클당 스텐실 1회라 FP64가 느려도 전체 시간의 10% 이하다(1e8 점 기준 약 수 ms).
   - 수렴 판정은 SIMION처럼 "최대 변화량"이 아니라 **f64 잔차 노름**으로 한다. 이것이 정확도 보장의 근거다.
2. **정밀도 개선 (3단계)**: 전극 경계를 격자에 맞춰 계단형으로 깎지 않고, **cut-cell(embedded boundary) 스텐실**로 실제 표면 위치를 반영한다. SIMION의 가장 큰 오차원(계단 전극)을 제거한다.
3. **적응 격자 (3단계 후반)**: 전극 가장자리 근처만 세분하는 octree 격자. 메모리와 시간은 줄이고 전극 근처 정밀도는 올린다.
4. **선택**: 개방 기하(전극이 공간에 띄엄띄엄 있는 경우)용 BEM 솔버. 조밀 행렬이 생기므로 이때만 텐서 코어/오자키 스킴이 의미 있다. 4단계 이후 필요 시.

### 3.2 전기장 보간

- SIMION: 전위 격자에서 선형 보간 → 장이 격자 경계에서 불연속, 적분기 오차의 주원인.
- 여기서는 **tricubic(또는 3차 B-spline) 보간**을 기본으로 한다. 장이 C1 연속이라 적응형 적분기가 큰 스텝을 안정적으로 잡는다. 속도와 정밀도 둘 다 이득. (대체됨 → 현재 상태 메모: 전극 경계 인식 6계수 Hermite + global cubic)
- GPU에서는 텍스처 샘플링 + 보정 방식으로 tricubic을 구현한다(메모리 접근 8회 → 하드웨어 보간 활용). (대체됨 → WGSL 셰이더에서 다항식 직접 평가)

### 3.3 이온 비행

- 적분기: Dormand-Prince RK45 적응 스텝(정전기), **Boris 적분기**(자기장 포함, 에너지 보존), 트랩 장시간 시뮬레이션용 심플렉틱 옵션.
- 이온별 독립 → GPU에 1 이온 = 1 스레드. 수천~수십만 개 동시 비행.
- 정밀도:
  - 위치: `셀 인덱스(i32) + 셀 내 오프셋(f32)` 표현. 큰 좌표 + 작은 변위에서 생기는 f32 정밀도 손실 차단.
  - 비행 시간, 누적 에너지: df64(double-float) 누적.
  - **CPU f64 기준 경로**를 항상 유지한다(Rayon + AVX2, 16코어). GPU 결과는 이 경로와 자동 비교된다.
- 시간 가변 전압: SIMION의 fast adjust는 전극별 기준 PA의 선형 중첩이다. 기준 PA를 GPU 텍스처로 올리고 `V_i(t)` 수식을 WGSL로 컴파일해 중첩한다. 이온별 Lua 호출 없이 GPU 병렬이 유지된다.

### 3.4 사용자 로직 (Lua 대체)

| 용도 | 방식 |
|---|---|
| 전극 전압 시간 함수(RF, 펄스, 램프) | 수식 DSL → WGSL 컴파일 (`V1 = 100*sin(2*pi*1e6*t)`) |
| 검출면, 조건 종료, 데이터 기록 | 설정(JSON/TOML) |
| 파라미터 스캔, 최적화 루프 | Rust API 또는 Python 바인딩(PyO3)에서 시뮬레이션 호출 |

### 3.5 기술 스택

- Rust, cargo workspace. GPU: `wgpu` + WGSL (`SHADER_F64` 기능으로 잔차 계산). CPU 병렬: `rayon`.
- 선형대수/테스트: `nalgebra`, `approx`. 직렬화: `serde`.
- 벤치마크: `criterion` + optimization-harness 스킬 규약(기준선 보존, 직렬 벤치, 회귀 bisect).
- UI: **Tauri 2 + TypeScript(React) 프런트 + 네이티브 Rust 코어.** 3D 뷰는 WebView2의 WebGPU로 그린다. 아래 3.6 참고. CLI는 자동화와 테스트용으로 같이 유지한다.

### 3.6 GUI 설계 원칙 (유저 프렌들리의 구체 기준)

프레임워크 선택 이유:
- `egui`(순수 네이티브)는 만들기 쉽지만 결과물이 "엔지니어용 도구" 느낌을 벗어나기 어렵다. 접근성, 한글 입력, 드래그 앤 드롭, 애니메이션, 컴포넌트 생태계 전부 웹 UI가 훨씬 낫다.
- Tauri는 UI만 웹 기술이고 계산은 전부 Rust 네이티브다. 속도 목표에 영향이 없다.
- 나중에 웹 포팅을 할 때 UI는 그대로 쓰고 코어만 WASM으로 바꾸면 된다.
- 대가: 언어가 둘(Rust + TypeScript)이고, 코어↔UI 사이에 IPC가 있다. 궤적·등전위 데이터는 바이너리 IPC로 넘긴다(수 MB 수준, 문제 없음).

원칙:
1. **열면 바로 보인다.** `.gem`, `.pa`, `.stl`, `.fly2`, 프로젝트 파일을 창에 끌어다 놓으면 즉시 3D 뷰에 전극이 뜬다. 설정 창을 먼저 거치지 않는다.
2. **결과가 즉시 따라온다.** 전극 전압 슬라이더를 움직이면 fast adjust 중첩으로 수 ms 안에 장과 궤적이 다시 그려진다. Refine이 필요한 변경(기하 수정)은 멀티그리드의 거친 단계 결과를 먼저 보여주고 점점 세밀해진다(progressive). 사용자는 기다리는 동안에도 대략의 결과를 본다.
3. **되돌리기가 항상 된다.** 모든 편집은 undo/redo. 프로젝트는 단일 파일(JSON + 참조 자산)로 저장되고 자동 저장된다.
4. **시작점을 준다.** 템플릿 갤러리(아인젤 렌즈, 4중극, 리플렉트론 TOF, 이온 트랩, 이온 퍼널 등). 빈 화면에서 시작하지 않는다.
5. **설정은 기본값으로 숨긴다.** 격자 간격, 수렴 허용치, 적분기 설정은 자동 추천값으로 시작하고 "고급"에만 노출한다. 자동 추천은 전극 최소 간격과 원하는 정밀도에서 계산한다.
6. **오류는 해결책을 말한다.** "PA 크기가 VRAM을 초과합니다 → 격자 간격을 0.1→0.15mm로 바꾸거나 대칭을 켜세요"처럼 다음 행동을 제안한다. 스택 트레이스는 로그로만.
7. **한국어/영어 전환.** UI 문자열은 처음부터 i18n으로 관리한다.
8. **결과 탐색이 쉽다.** 궤적 클릭 → 그 이온의 에너지/시간 그래프. 검출면 클릭 → 도달 분포 히스토그램. 데이터는 CSV/Parquet로 한 번에 내보내기.
9. **키보드와 마우스 둘 다.** 3D 뷰는 CAD식 조작(궤도 회전, 팬, 줌, 정면/측면 단축키), 단면 슬라이스 드래그.

검증: 각 UI 마일스톤마다 "SIMION을 모르는 사용자가 템플릿에서 시작해 전압을 바꾸고 결과를 내보내기까지 설명 없이 가능한가"를 기준으로 사용성 테스트. visual-qa 스킬로 화면 체크리스트 점검.

## 4. 저장소 구조

```
crates/
  simrep-core/      격자, PA 포맷, 기하 표현, 공통 타입
  simrep-solve/     멀티그리드(GPU/CPU), cut-cell, 잔차 보정
  simrep-fly/       보간, 적분기, 이온 비행(GPU/CPU)
  simrep-io/        SIMION .pa/.pa0/.fly2/.gem/.rec 읽기, STL  (대체됨 → GEM은 simrep-gem, STL/메시는 simrep-mesh)
  simrep-cli/       배치 실행 CLI
  simrep-app/       Tauri 앱 (Rust 쪽 커맨드, IPC)  (대체됨 → 크레이트 없음, app/src-tauri)
app/                TypeScript(React) 프런트: 3D 뷰어(WebGPU), 패널, 템플릿
tools/
  simion-oracle/    SIMION --nogui 배치 실행 + 결과 비교 스크립트
bench/              벤치마크 케이스와 기준선
docs/
```

## 5. 단계별 계획

각 단계는 "완료 조건"이 수치로 검증될 때 끝난다.

### 0단계: 기반 (1주)
- cargo workspace, CI(WSL: CPU 테스트, Windows: GPU 테스트), 벤치 하네스.
- `simion.chm` 추출 → PA 포맷, .fly2, GEM 문법, fast adjust 규칙 명세를 `docs/spec/`에 정리.
- SIMION oracle 스크립트: 예제를 `--nogui`로 돌려 PA와 궤적 결과를 저장.
- 완료 조건: `examples/performance_tests`, `test_plane`, `test_sphere`, `einzel`, `quad`, `tof` 예제의 SIMION 결과 파일 확보. `.pa` 읽기/쓰기 왕복 테스트 통과.

### 1단계: 기준 전위 솔버 (2~3주)
- CPU 멀티그리드(f64) → GPU 멀티그리드(f32 + f64 잔차 보정).
- 해석해 검증: 평행판, 동축 원통, 동심 구, 2D 4중극. 상대 오차 목표 1e-6 이하(격자 수렴 차수 확인).
- SIMION 비교: 같은 `.pa#`에서 refine한 결과와 비교. 차이는 SIMION의 수렴 허용치 수준(≈1e-7 V) 이내.
- 완료 조건: 3D 배열 refine이 SIMION 엄격 설정 대비 **5배 이상 빠름**(Windows, RTX 3070). 정밀도 조건 동시 만족.
- **현황(2026-10-23)**: 259³(1,737만 점) sc3d_x2에서 GPU 2.50초 vs SIMION 엄격 14.37초 = **5.75배, 달성**. SIMION 엄격 결과와 최대 6.4e-5 V 차이. einzel은 최대 1.3e-6 V. 상세: `bench/refine-real.md`, `docs/gpu-solver.md`.

### GUI 트랙 (2단계부터 병행, 각 단계마다 UI 마일스톤)

| 시점 | UI 마일스톤 |
|---|---|
| 1단계 끝 | Tauri 뼈대, 파일 드래그 앤 드롭, 3D 전극 뷰, 등전위 단면 표시, progressive refine 표시 |
| 2단계 끝 | 궤적 표시, 전압 슬라이더 즉시 재비행, 이온 소스 편집 패널, undo/redo, 프로젝트 저장 |
| 3단계 끝 | 격자 자동 추천, 정밀도 표시(잔차·수렴 차수), 고급 설정 패널 |
| 4단계 끝 | 가져오기 마법사(드래그 앤 드롭, 단위·전극 번호 지정, 메시 수리 미리보기), 내보내기 대화상자, 템플릿 갤러리 |
| 5단계 끝 | 결과 탐색(히스토그램, 그래프), 내보내기, i18n 완성 |
| 6단계 | 마감: 사용성 테스트 반영, 설치 패키지, 문서/튜토리얼 |

### 2단계: 이온 비행 (2~3주)
- tricubic 보간(대체됨 → 경계 인식 6계수 Hermite 재구성), RK45, Boris. CPU f64 경로 먼저, GPU 경로 다음.
- 해석해 검증: 균일 자기장 사이클로트론 궤도(반경·주기), 조화 트랩 진동수, 평행판 TOF. 상대 오차 1e-7(CPU), 1e-5(GPU) 목표.
- SIMION 비교: `einzel`, `quad`, `tof` 예제의 최종 위치/비행 시간. SIMION의 자체 오차 범위 안에서 일치.
- fast adjust 중첩 + 수식 DSL(RF 4중극 `quad` 예제로 검증).
- 완료 조건: 이온 비행이 SIMION 대비 **10배 이상 빠름**. 정밀도 조건 동시 만족.
- **현황(2026-10-23, einzel)**: 정확도는 8배 격자 기준 위치 오차 RMS 0.225 mm vs SIMION 1.106 mm로 **달성**. GPU 비행 계산 시간 1만 개 8.9 ms / 10만 개 29 ms vs SIMION 2.2초 / 21.5초로 **달성**. CLI 프로세스 전체 10만 개 0.58초(37배). `tof`와 RF(`performance_tests/8_rf_tests`, `quad`는 Lua 미연결이라 부적합) 검증은 남음. (대체됨 → 완료, `bench/fly-rf-tof.md`·`bench/default-flight.md` 참조) 상세: `bench/fly-real.md`, `bench/fly-gpu.md`.

### 3단계: SIMION 넘어서는 정밀도 (3~4주)
- cut-cell 경계 스텐실. 구/원통 해석해로 격자 수렴 차수 2차 확인(계단형은 1차).
- octree 적응 격자와 멀티그리드 결합. **보류(2026-10-24 사용자 결정)**: cut-cell만으로 같은 메모리에서 계단형 대비 78~84배 정확해 3단계 기준을 넘었고, 4중극 사용자 경로는 SIMION 대비 약 700배 정확. 적응 격자는 2배 패치만으로 노드 메모리 4.1~5.7배, 새 대칭 경계 연산자(hanging-node) 설계 필요. 실제로 부족한 사례가 나오면 재검토.
- 완료 조건: 동심 구 문제에서 같은 메모리로 SIMION보다 오차 10배 이상 작음. 속도 조건은 1단계 수준 유지.

### 4단계: 기하와 3D 모델 가져오기/내보내기 (3주)
- GEM 부분집합(기본 도형, 불리언, 전극 번호), STL → 부호 거리장(SDF) → cut-cell.
- 3D 모델 가져오기(import):
  - 메시: STL(ASCII/바이너리), OBJ, PLY, 3MF, glTF/GLB
  - CAD: STEP/IGES (OpenCascade 바인딩. 곡면을 SDF로 직접 변환해 메시화 오차 없이 cut-cell에 반영)
  - SIMION: `.gem`, `.pa`/`.pa#`/`.pa0`, `.iob`(워크벤치 구성), `.fly2`
  - 가져올 때 단위(mm/inch/m) 선택, 메시 결함 검사·자동 수리(구멍, 뒤집힌 법선, 비다양체), 부품별 전극 번호 지정 UI
- 3D 모델 내보내기(export):
  - 전극 기하: STL, OBJ, glTF/GLB, STEP(CAD 입력이었던 경우)
  - SIMION 호환: `.pa`/`.pa#`/`.pa0`(SIMION에서 바로 열기), `.gem`(가능한 경우), `.fly2`
  - 결과: 전위/전기장 VTK(`.vti`/`.vtu`, ParaView용), 등전위면 STL/glTF, 궤적 CSV/Parquet/VTK 폴리라인, 화면 PNG/SVG
- 완료 조건: SIMION 예제 STL/GEM을 가져와 refine한 결과가 SIMION과 일치. 내보낸 `.pa`를 SIMION에서 열어 같은 궤적이 나옴. STEP 부품 하나를 가져와 cut-cell로 refine 가능.
- .fly2 입자 정의 전부, 분포 생성(가우시안, 균일, 원뿔).
- 완료 조건: SIMION `geometry` 예제 5개를 GEM에서 읽어 refine한 결과가 SIMION과 일치.

### 5단계: 고급 물리 (3~4주, 우선순위는 사용 분야에 따라 조정)
- 자기장 PA 및 외부 자기장. 공간전하: Coulomb 직접 합(소수) + PIC(다수). 충돌 모델 HS1 상당. 유전체.
- 완료 조건: SIMION `repulsion`, `collision_hs1`, `magnet` 예제와 통계적으로 일치.

### 6단계: 마감과 배포 (2~3주)
- GUI 사용성 테스트 결과 반영, 튜토리얼(템플릿 기반 5분 가이드).
- Python 바인딩(PyO3). Windows 설치 패키지(Tauri 번들러).
- 이후 선택: 코어를 WASM으로 빌드해 같은 UI로 웹 버전 제공.

## 6. 검증 체계 (전 단계 공통)

1. **해석해 테스트**: 격자 수렴 차수를 매번 확인. 정밀도 회귀를 잡는 첫 번째 장치.
2. **CPU f64 기준 경로 대 GPU 혼합 정밀도**: 모든 GPU 결과는 CPU 결과와 자동 비교.
3. **SIMION oracle**: 같은 입력을 `simion.exe --nogui`로 돌린 결과와 비교. 허용 오차는 SIMION의 자체 수렴 허용치를 기준으로 둔다.
4. **벤치 기준선**: 단계 완료 시점의 성능을 저장. 이후 변경은 회귀 여부를 bisect로 추적.

## 7. 위험과 대응

| 위험 | 대응 |
|---|---|
| WSL2에서 GPU 실행 불가 | Windows 빌드 경로를 0단계에서 확립. CPU 경로는 WSL에서 개발 |
| wgpu f64 지원이 백엔드별로 다름 | DX12/Vulkan 둘 다 0단계에서 f64 스모크 테스트. 안 되면 잔차 계산만 CPU로. **결과(2026-10-23, RTX 3070, wgpu 30): DX12는 SHADER_F64 없음, Vulkan은 있음(f64 잔차 커널 오차 0). Windows 기본 백엔드는 Vulkan.** WGSL에는 f64 타입이 없어 f64 커널은 GLSL(Naga)로 작성 |
| 8GB VRAM 초과 배열 | 블록 단위 스트리밍(호스트 RAM 32GB 활용). 2단계 이후 |
| cut-cell 스텐실 안정성(작은 셀 문제) | 셀 병합(cell merging) 기법. 3단계 초반에 프로토타입으로 위험 제거 |
| SIMION 예제 다수가 Lua 의존 | Lua 없는 예제와 fast adjust 수준의 단순 Lua 예제만 oracle로 선택 |

## 8. 즉시 할 일 (0단계 시작)

1. `simion.chm` 추출(7z)해서 PA 포맷/.fly2/GEM 명세 정리.
2. cargo workspace 생성, Windows에서 wgpu 어댑터 + `SHADER_F64` 확인하는 스모크 테스트.
   Tauri 뼈대와 WebView2 WebGPU 가용성(삼각형 하나 그리기)도 같이 확인.
3. SIMION oracle 스크립트로 `test_sphere`, `test_plane`, `performance_tests` 결과 확보.
