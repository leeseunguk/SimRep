# 구현된 방정식 명세

이 문서는 SimRep 코드가 실제로 푸는 방정식을 한 곳에 모은 명세다. 각 식에는 기호와
단위, 성립 범위, 문헌 출처, 구현 위치, 그리고 그 식을 검사하는 테스트를 함께 적는다.

## 이 문서를 쓰는 방법

- 식 번호는 이 문서 안에서만 의미가 있는 고유 번호다. 기술 보고서의 식 번호와는
  다르며, 대응하는 보고서 식은 "보고서 식 (n)"으로 병기한다.
- 단위는 SimRep 비행 단위계다. 위치·격자 간격 mm, 시간 μs, 전위 V, 전기장 V/mm,
  자기장 G(가우스), 질량 amu, 전하는 기본 전하 e의 배수, 전류 A.
  `ε₀ = 8.8541878128e-15 F/mm`, `μ₀/4π = 1 G·mm/A`, `c = 299792.458 mm/μs`로
  고정해 두면 모든 식이 이 단위계에서 닫힌다.
- "구현"은 파일과 함수 이름을 적는다. "테스트"는 `crates/*/tests/` 아래 파일과
  테스트 함수 이름을 적는다. 테스트가 없거나 명명된 외부 입력을 요구하는 식은
  그 사실을 명시한다.
- 문헌 출처는 해당 문서나 보고서가 실제로 인용한 것만 적는다. 없는 페이지나
  식 번호를 덧붙이지 않는다.

---

## 1. 전위 계산 (솔버)

### 식 (1). 이산 라플라스·푸아송 연산자 (보고서 식 (1))

```text
L_h V_i = Σ_a (V_{i-e_a} − 2 V_i + V_{i+e_a}) / h_a²,   −L_h V = ρ/ε₀
```

- 기호와 단위: `V_i`는 격자점 i의 전위(V), `e_a`는 축 a 방향 한 칸 이동,
  `h_a`는 그 축의 격자 간격(mm). `ρ`는 전하 밀도(C/mm³), `ε₀`는 진공 유전율
  8.8541878128e-15 F/mm이다. `ρ/ε₀`의 단위는 V/mm²다.
- 범위와 가정: 균일 직교 격자의 2차 중심차분. `nz = 1`이면 2D다. 마스크되지
  않은 바깥 면은 활성 축마다 영 법선 도함수(대칭 경계)이며, 마스크된 점은
  디리클레 고정값이다. 미러 면에서는 유령값 반사로 안쪽 이웃 계수가 `2/h²`가
  된다. 원통 대칭(`Geometry::Cylindrical`)에서는 반지름 항이

  ```text
  L_r V = [(1 − h/(2r)) V_{r-1} − 2 V_r + (1 + h/(2r)) V_{r+1}] / h²
  ```

  이고, 축(r = 0)에서는 정칙성 조건 `L_r V = 4(V(h) − V(0))/h²`를 쓴다
  (미러 계수 2/h²가 아니다).
- 솔버 내부 형태: 멀티그리드는 물리 연산자를 행별 양의 노드 가중치 W로 곱한
  대칭 행렬 `A = −W L`로 푼다. 평면 가중치는 내부 1, 미러 면 1/2이고,
  반지름 가중치는 `r/h_r`(축에서 1/8, 바깥 면 `(r/h_r − 1/2)/2`,
  환형 안쪽 미러 `(r/h_r + 1/2)/2`)이다. 축별로 곱해진다. 해와 물리 잔차는
  바뀌지 않는다.
- 출처: 표준 정전기학의 라플라스·푸아송 방정식을 프로젝트 기호로 옮긴 것.
  정의는 `docs/cpu-solver.md`의 Discrete operator 절.
- 구현: `crates/simrep-solve/src/mg.rs`의 `solve`(라플라스), `solve_poisson`
  (푸아송), `Operator::neighbors`/`weight`/`diagonal`(계수), `Axis::new`
  (원통 계수).
- 테스트: `crates/simrep-solve/tests/mg_reference.rs`(독립 SOR 기준해와 비교,
  원통 조화 다항식, 환형 `log(r)` 2차 수렴 등), `tests/poisson.rs`
  (manufactured 평면·원통 푸아송 해와 우변 부호·단위).

### 식 (2). 잔차 수렴 판정 (보고서 식 (2))

```text
‖r‖_∞ ≤ τ_abs + τ_rel · s
```

- 기호와 단위: `r`은 자유 격자점에서의 물리적 비가중 잔차로 단위는 V/mm²다.
  `τ_abs`는 절대 허용값(V/mm², 기본 0), `τ_rel`은 상대 허용값(기본 1e-10).
  `s`는 상대 스케일로, 첫 고정 전압만큼 이동(gauge-shift)한 뒤 환산 우변
  `b_i = Σ_{고정 이웃 j} c_ij (V_j − V_first)`의 항과 소스 항 `S_i`의 절댓값을
  함께 취한 최대값이다. 이 우변이 0이면 단위 전압 스텐실 스케일로 대체한다.
- 범위와 가정: 잔차가 작다는 것은 이산 방정식이 풀렸다는 뜻이며, 실제 형상의
  연속 해에 대한 오차 상한이 아니다. SIMION의 "반복 사이 최대 변화량" 판정과
  다르다. `max_cycles`(기본 100) 소진은 명시적 비수렴이다.
- 출처: 프로젝트 수렴 규약. `docs/cpu-solver.md`의 Convergence and reporting 절.
- 구현: `crates/simrep-solve/src/mg.rs`의 `reached`, `Level::boundary_scale`,
  `Options::default`. GPU 경로는 f32 V-사이클에 f64 바깥 잔차를 두는 반복
  개선으로 같은 판정을 만족한다(`docs/gpu-solver.md`).
- 테스트: `crates/simrep-solve/tests/mg_reference.rs`의
  `convergence_budget_and_input_validation_are_explicit`,
  `fixed_and_constant_solutions_do_not_need_a_cycle`.

### 식 (3). cut-cell 경계 스텐실 (보고서 식 (3))

```text
c (V_j − V_i)  →  (c/θ)(V_Γ − V_i),   0 < θ ≤ 1
```

- 기호와 단위: 진공 점 i와 전극 안쪽 점 j를 잇는 길이 h(mm)의 연결에서, 표면이
  i로부터 비율 θ만큼 떨어져 있을 때 원래 계수 `c`의 이웃 항을 `c/θ` 계수의
  표면 전위 `V_Γ`(V)로 바꾼다.
- 범위와 가정: Gibou et al. 2002의 식 (21), 선형 대칭 ghost-fluid 이산화다.
  자유/자유 연결은 바꾸지 않으므로 `A = −W L`이 대칭 양의 정부호로 남고
  제한·보간(P^T, P^T A P)과 평활 상한이 그대로 유효하다. 같은 논문의 비대칭
  2차 공식 식 (20)은 이 성질이 없어 쓰지 않는다. 표면에 가까운 점은
  `snap_distance = 2 h_min² / L`(L은 영역의 최장 축 길이) 안이면 전극 전압으로
  고정한다. 이는 표면을 O(h²/L)만큼 옮기는 것으로, 상수 θ 클램프(O(h) 변)는
  금지다. 전위 오차는 측정상 2차, 표면 근처 전기장 오차는 약 1차다.
- 출처: Gibou, F., Fedkiw, R. P., Cheng, L.-T., & Kang, M. (2002). A
  second-order-accurate symmetric discretization of the Poisson equation on
  irregular domains. Journal of Computational Physics, 176(1), 205-227.
  (`docs/cutcell.md`의 Operator and symmetry invariant 절이 식 (21)을 명시.
  참고 원고: https://physbam.stanford.edu/papers/cam2000-37.pdf)
- 구현: `crates/simrep-solve/src/cutcell.rs`의 `CutRow`,
  `CutCells::new`/`new_with_root_tolerance`, `surface_fraction`.
- 테스트: `crates/simrep-solve/tests/cutcell.rs`
  (`curved_boundaries_have_second_order_potential_and_first_order_field`,
  `tiny_cuts_snap_with_quadratic_not_linear_displacement`,
  `a_thin_electrode_between_free_endpoints_is_not_lost`).

### 식 (4). 갤러킨 거친 격자 연산자

```text
A_coarse = Pᵀ A P
```

- 기호와 단위: `P`는 부모 격자의 첨자 좌표에서 텐서 선형 보간 행렬(자유
  보간값이고, 미세 전극 점에서는 보정이 0), `A`는 식 (1)의 가중 대칭 연산자다.
  제한 행렬은 정확히 `Pᵀ`다.
- 범위와 가정: 한 점짜리 전극, 가는 도선, 일부만 채워진 집합도 거친 대각·결합
  항에 기여하므로 전극을 거친 격자에서 잃지 않는다. 거친 스텐실은 27점(2D는
  9점)으로 저장한다. 축이 최대 4점이 될 때까지 줄이고, 최하층(최대 64점)은
  정지형 야코비 80회다.
- 출처: 표준 갤러킨 멀티그리드 구성. `docs/cpu-solver.md`의 Coarsening
  electrodes without losing constraints 절.
- 구현: `crates/simrep-solve/src/mg.rs`의 `Transfer::galerkin`,
  `Transfer::new`/`interpolate_row`/`support_row`.
- 테스트: `crates/simrep-solve/tests/mg_reference.rs`의
  `arbitrary_sizes_and_single_node_electrodes_survive_coarsening`,
  `skinny_grid_retains_tight_sor_agreement`.

---

## 2. 이온 비행

### 식 (5). 상대론 로런츠 운동방정식 (보고서 식 (4))

```text
dx/dt = v = u/γ,   γ = √(1 + |u|²/c²),   du/dt = (q/m) (a_E E + a_B v × B)
```

- 기호와 단위: `x`는 위치(mm), `t`는 시간(μs), `u = γv`는 고유 속도(mm/μs),
  `E`는 V/mm, `B`는 G다. `q`는 e 단위 전하수, `m`은 amu 단위 질량이다.
  `a_E = ELECTRIC_ACCELERATION = (1.602176634e-19 / 1.66053906892e-27)·1e-6`
  (e/amu당 V/mm → mm/μs²), `a_B = MAGNETIC_FREQUENCY = a_E·1e-4`
  (e/amu당 G → 1/μs), `c = 299792.458 mm/μs`다. `γ`는 무차원이고, 저속 한계
  γ → 1에서 뉴턴 방정식으로 돌아간다.
- 범위와 가정: 상태는 (x, u) 6차원이다. 적분기는 기본이 Dormand-Prince 5(4)
  RK45 적응 스텝이며, 사용자 선택으로 Boris를 쓴다(Boris는 균일 B에서 속력을
  보존하지만 회전 위상은 2차 오차). CLI는 상대론 모드가 기본이고, 충돌 비행은
  비상대론으로 고정된다.
- 출처: 상대론적 로런츠 힘 운동방정식(Jackson, J. D. (1999), Classical
  Electrodynamics, 3rd ed., 12장)을 이 단위계로 옮긴 것.
- 구현: `crates/simrep-fly/src/flight.rs`의 `derivative`, `velocity`, `rk45`,
  `boris`. 상수는 `crates/simrep-fly/src/lib.rs`의 `ELECTRIC_ACCELERATION`,
  `MAGNETIC_FREQUENCY`, `SPEED_OF_LIGHT`.
- 테스트: `crates/simrep-fly/tests/accuracy.rs`의
  `relativistic_acceleration_and_boris_energy`,
  `parallel_plate_tof_and_birth_time`,
  `harmonic_oscillation_frequency_and_long_flight`.

### 식 (6). 비행 스텝 크기 상한

```text
Δt ≤ min( d / ṽ,  0.2 / ω_c,  0.2 · T_v )
```

- 기호와 단위: `d = max_grid_cells · h_min`은 한 스텝에 건널 수 있는 거리로,
  `max_grid_cells` 기본값 4칸에 최소 격자 간격 `h_min`(mm)을 곱한다. `ṽ`는
  속력(mm/μs)이고 가속이 있으면 `v·h + a·h²/2 = d`의 양의 근으로 바꿔 잡는다.
  `ω_c = |q/m|·a_B·|B|/γ`는 사이클로트론 각주파수(1/μs)로 한 스텝 회전각 상한은
  0.2 rad다. `T_v`는 시간 가변 전압의 특성 시간(μs)으로 위상 변화 상한도
  0.2 rad다. `max_step_us`를 지정하면 이 상한 전체를 그 값으로 대체한다.
- 범위와 가정: 오차 추정과 별개의 물리적 상한이다. 적응 허용 오차는 1e-12에서
  시작하고 정전기 격자에서는 출발점 근처 전기장 변화로 1e-12와 1e-8 사이를
  고른다(`bench/default-flight.md`의 Policy and tolerance selection 절).
- 출처: 프로젝트 규약. `crates/simrep-fly/src/flight.rs` 주석과
  `docs/physics-magnetic-collision.md` 서두.
- 구현: `flight.rs`의 `FlightOptions::step_limit_us`,
  `step_limit_with_sample`, `StepLimits::default`.
- 테스트: `crates/simrep-fly/tests/step_control.rs`
  (`transit_limit_scales_with_grid_placement_speed_and_acceleration`,
  `cyclotron_guard_and_slow_field_free_flights_do_not_have_a_fixed_ceiling`,
  `rf_phase_scale_is_parameter_bound_and_keeps_the_fastest_component`,
  `automatic_tolerance_estimates_grid_variation_and_respects_explicit_values`).

### 식 (7). 경계 인식 6계수 에르미트 전기장 재구성

```text
V(ξ) = Σ_{k=0}^{5} c_k ξ^k   (셀 내부 좌표 ξ, 축별 6계수 텐서 곱)
```

- 기호와 단위: 각 축 방향 진공 구간마다 노드 전위(V)와 1차·2차 도함수를
  공유 노드 제트(jet)로 추정한 뒤, 셀 안을 축별 5차 에르미트 다항식
  (계수 6개)의 텐서 곱으로 채운다. 전기장은 `E = −∇V`(V/mm)다.
- 범위와 가정: 전위는 C2 연속이고 전기장은 셀 경계에서 C1 연속이며, 노드의
  원래 전위를 정확히 지난다(보간, 평활 아님). 전극이 있는 배열에서는 축 방향
  선을 진공 구간으로 나누고, 경계 기울기는 진공 쪽 한쪽 3차 적합으로 추정해
  금속 안쪽 값이 섞이지 않게 한다. 전극이 없는 배열은 텐서 3차 B-스플라인
  (global cubic)을 쓰고 `--global-cubic`으로 강제할 수 있다. GPU는 같은
  다항식 계수를 받아 셰이더에서 직접 평가한다.
- 출처: 프로젝트 재구성 방식. `crates/simrep-fly/README.md`의 Fields 절,
  `docs/geometry-pipeline.md`의 Outputs and flying 절,
  `docs/PLAN.md`의 현재 상태 메모.
- 구현: `crates/simrep-fly/src/field/boundary.rs`(`build`, `cell_powers`,
  `sample`, `differentiate`, `line_derivatives`), `field.rs`의
  `GridField`/`Reconstruction`.
- 테스트: `crates/simrep-fly/tests/accuracy.rs`의
  `vacuum_boundary_derivatives_exclude_flat_metal_interior`,
  `vacuum_boundary_reconstruction_preserves_axis_and_reflection_conditions`,
  `grid_flight_converges_at_least_cubically_against_analytic_field`.

### 식 (8). 도착 위치 RMS 판정 규약 (보고서 식 (5))

```text
RMS = √( (1/N) Σ_i |x_i − x_i^ref|² )
```

- 기호와 단위: `x_i`는 i번째 이온의 도착 위치(mm), `x_i^ref`는 같은 이온을
  더 촘촘한 격자(보통 8배)로 날린 기준해의 도착 위치, `N`은 이온 수다.
- 범위와 가정: 이 문서·보고서의 정확도 비교 규약으로, 솔버나 적분기가 푸는
  방정식이 아니다. 같은 정의를 SIMION 결과에도 그대로 적용한다. 도착 면이
  달라진 이온 수는 따로 센다.
- 출처: 프로젝트 판정 규약(`docs/PLAN.md`의 1절, 보고서 4.4절).
- 구현: 계산 코어의 방정식이 아니라 비교 도구에서 사용한다. 예:
  `crates/simrep-fly/tests/magnetic_performance.rs`의 `errors`.
- 테스트: `crates/simrep-fly/tests/magnetic_performance.rs`의
  `magnetic_compute_and_convergence`.

---

## 3. 자기장

### 식 (9). 자기 PA와 비오-사바르 (보고서 식 (6))

```text
B = −n_g ∇p,    B(x) = (μ₀/4π) ∫ I dℓ × r / |r|³
```

- 기호와 단위: `p`는 자기 PA에 저장된 스칼라 포텐셜(SIMION의 mags),
  `n_g`는 PA 머리말의 양의 배율, `B`는 G다. 비오-사바르 식에서 `I`는 전류(A),
  `dℓ`는 도선 미소 구간(mm), `r`은 관측점 벡터(mm)이며, 이 단위계에서는
  `μ₀/4π = 1 G·mm/A`다.
- 범위와 가정: 자기 PA 기울기는 식 (7)과 같은 경계 인식 C2 재구성으로 읽고,
  배치(placement)는 좌표만 다시 스케일할 뿐 B의 크기를 바꾸지 않는다.
  `Coil::segment`는 유한 직선 도선의 비오-사바르 적분, `Coil::hoop`은 완전
  타원 적분 K/E를 AGM으로 계산하고 축 위 해석 극한을 쓴다. 솔레노이드는
  균등 분할한 N개 후프의 합이다. 도선은 무한소 필라멘트라서 도선 위에서는
  발산한다.
- 출처: 자기 포텐셜 정의는 SIMION 2024 도움말 `magnetic_potential.html`
  (Scientific Instrument Services, 2024)을 따른다. 비오-사바르와 타원 적분
  코일은 표준 공식이며 SIMION의 `MField.solenoid_hoops`와 같은 분할 규약을 쓴다.
- 구현: `crates/simrep-fly/src/physics/magnetic.rs`의 `MagneticPa`,
  `Coil::field_gauss`, `Coil::segment`/`hoop`, `elliptic_ke`,
  `MagneticField::solenoid`.
- 테스트: `crates/simrep-fly/tests/physics.rs`의
  `biot_savart_segment_loop_and_solenoid_limits`,
  `magnetic_pa_ng_spacing_mirrors_and_cylindrical_expansion`,
  `magnetic_scalar_grid_converges_to_own_analytic_reference_through_eightfold_refinement`.

### 식 (10). 코일 지도의 B_r/r 저장

```text
격자값: (B_x(x, r),  B_r(x, r)/r),   축: (B_r/r)(x, 0) = 3π I a² x̄ / (a² + x̄²)^{5/2}의 합
```

- 기호와 단위: 축대칭 코일 묶음의 축방향 좌표 `x`와 반지름 `r`(mm) 격자 위에
  축성분 `B_x`(G)와 매끄러운 스칼라 `B_r/r`(G/mm)를 저장한다. 축 값은 각
  후프의 해석 극한(반지름 a mm, 전류 I A, 축 거리 x̄ mm)의 합으로 채운다.
  물리 `B_r`은 재구성된 `B_r/r`에 횡방향 좌표를 곱해 얻으므로 r = 0에서 정확히 0이다.
- 범위와 가정: 엄밀히 동축인 후프가 16개 이상일 때만 지도를 만들고, 격자
  간격은 0.0625 mm다. 채택 전에 각 영역에서 격자점 밖 512개 표본을 정확한
  계산과 비교해 최대 절대 차이 2e-9 G, 최대 상대 차이 5e-10을 넘으면 지도를
  쓰지 않고 정확한 계산으로 되돌아간다. 외부 자기장이므로 지도로 바꿔도 물리는
  같고 차이는 보간 오차뿐이다.
- 출처: 프로젝트 최적화와 보정 규약. `bench/magnetic-cpu.md`의 Implementation
  and profile 절, `docs/physics-magnetic-collision.md`의 GPU transport 절.
- 구현: `crates/simrep-fly/src/physics/magnetic/coil_map.rs`
  (`CoilMapPlan`, `CoilMapDomain::new`, `calibrate`, `probes`),
  `magnetic.rs`의 `prepare_coil_map`.
- 테스트: `coil_map.rs` 모듈 테스트
  `calibrated_map_preserves_axis_rotations_and_exact_fallback`,
  `failed_calibration_and_noncoaxial_groups_stay_exact`;
  `crates/simrep-fly/tests/magnetic_performance.rs`의
  `magnetic_cpu_map_and_profile`.

### 식 (11). 재료가 있는 자기장: 스칼라 포텐셜 (보고서 식 (10))

```text
−∇·(μ_r ∇p) = (4π/n_g)(−∇·M),    B = −n_g μ_r ∇p + 4π M
```

- 기호와 단위: `μ_r`은 상대 투자율(유한, 양의 스칼라), `M`은 자화(A/mm),
  `p`는 스칼라 포텐셜, `n_g`는 PA 배율, `B`는 G다.
- 범위와 가정: 철심·영구자석용. 형태가 식 (17)과 같아 유전체 솔버의 계수
  계산을 그대로 쓴다. 출력은 평범한 자기 PA이고 형제 `.mag.json`이 재료 셀
  마스크를 저장한다. μ_r ≠ 1이거나 M ≠ 0인 셀은 이온에게 고체다. 이온이 나는
  진공에서는 μ_r = 1, M = 0이라 B = −n_g ∇p로 돌아간다. 고체 안에서 스칼라
  재구성으로 B를 읽는 것은 재료 내부 B의 계산이 아니다. 비선형 B-H 모형은 없다.
- 출처: SIMION 2024 도움말 `magnetic_potential.html`(Scientific Instrument
  Services, 2024)의 정의를 프로젝트 기호로 옮긴 것.
- 구현: `crates/simrep-solve/src/magnetostatics.rs`의
  `MagneticProblem::scalar`, `solve`, `cell_source`. 비행 계약은
  `crates/simrep-fly/src/physics/magnetic.rs`의 `MagneticPa`와
  `with_material_cells`.
- 테스트: `crates/simrep-solve/tests/magnetostatics.rs`의
  `uniformly_magnetized_sphere_and_transverse_cylinder_converge`,
  `iron_slab_has_exact_continuous_normal_flux_and_is_solid`,
  `iron_cylindrical_shield_converges_to_analytic_attenuation`.

### 식 (12). 재료가 있는 자기장: 벡터 포텐셜 (보고서 식 (11))

```text
평면:    −∇·(μ_r⁻¹ ∇A_z) = (4π/n_g) J_z
축대칭:  −∇·((μ_r r)⁻¹ ∇ψ) = (4π/n_g) J_φ,   ψ = r A_φ
직교3D:  −μ_r⁻¹ ΔA_i = (4π/n_g) J_i
공통:    B = n_g ∇ × A
```

- 기호와 단위: `J`는 전류 밀도(A/mm²), `A_z`·`A_i`는 `B = n_g ∇×A`에 맞춘
  포텐셜로 G·mm/n_g 차원이다. 축대칭의 `ψ = r A_φ`는 반지름을 곱해 G·mm²/n_g다.
  축대칭 식의 발산은 원통 스칼라 라플라시안이
  아니라 평면 발산이고, `ψ`는 축에서 0이다.
- 범위와 가정: 직교 3D 성분별 식은 `μ_r`이 공간적으로 일정할 때만 성립한다.
  위치마다 다른 μ_r의 3D 벡터 식은 세 성분이 결합되며, 독립 스칼라 식 세 개로는
  구현되어 있지 않다(미구현). 비행에서는 B 성분을 보간하지 않고 재구성한 C2
  포텐셜의 회전을 취해 C1이고 무산인 B를 얻는다. `ψ`는 정칙 짝함수
  `f = ψ/r²`를 재구성하고 축 값 `(4f(h) − f(2h))/3`을 쓰며,

  ```text
  B_x = n_g (2f + r ∂f/∂r),   B_r = −n_g r ∂f/∂x,   B_x(축) = 2 n_g f(축)
  ```

  로 r 나눗셈과 소실 없이 축 극한을 얻는다(x는 축방향, r은 반지름 방향인
  PA 로컬 좌표 기준).
  벡터 포텐셜과 자성 재료 경로는 CPU 전용이다.
- 출처: SIMION 2024 도움말 `magnetic_potential.html`의 정의.
  `docs/magnetostatics.md`의 Equations and flight contract 절.
- 구현: `crates/simrep-solve/src/magnetostatics.rs`의 `MagneticProblem::currents`,
  `cartesian_current_component`. 비행 측은
  `crates/simrep-fly/src/physics/magnetic/vector_potential.rs`의
  `VectorPotential::new`/`cartesian`.
- 테스트: `crates/simrep-solve/tests/magnetostatics.rs`의
  `infinite_wire_and_return_coax_converge`,
  `axisymmetric_current_uses_planar_one_over_r_operator`,
  `cylindrical_cell_source_conserves_axis_adjacent_annular_volume`;
  `crates/simrep-fly/tests/vector_potential.rs`의
  `computed_uniform_psi_field_matches_analytic_flight_on_axis_and_off_axis`,
  `three_component_curl_preserves_component_parity_and_placement`.

### 식 (13). 외부 자기장 지도 재구성과 발산 진단

```text
B 성분별 경계 인식 C2 재구성(식 (7)과 같은 제트),
축대칭 지도: B_z와 B_r/r을 재구성 (식 (10)과 같은 방식)
진단:  div B ≈ ∂B_x/∂x + ∂B_y/∂y + ∂B_z/∂z   (원통: ∂B_z/∂z + ∂B_r/∂r + B_r/r)
```

- 기호와 단위: 가져온 규칙 격자의 각 데카르트 성분을 같은 6계수 에르미트 C2
  재구성으로 읽는다(벡터 B를 직접 보간하며 스칼라 포텐셜이 아니다). 축대칭
  지도는 `B_z`와 `B_r/r` 스칼라를 재구성하고, `B_r/r`의 축 값은 양의 반지름
  최대 3곳의 한쪽 짝다항 외삽이다. `div B` 진단은 중심차분(가장자리는 한쪽
  차분)으로 G/mm 단위의 최대·RMS와 `max|div B|·min_spacing/max|B|`를 출력한다.
- 범위와 가정: 규칙 격자 파일만 받는다. 흩어진 점 데이터는 원래 프로그램에서
  규칙 격자로 다시 내보내야 한다. C2 다항식은 입력 데이터를 고치지 않으며
  div B = 0을 강제하지 않는다. 정규화 최대값 0.01 초과는 경고 기준일 뿐
  정확도 게이트가 아니다. 미러 반사는 재구성 시점의 성분 패리티(축벡터 규칙:
  법선 성분 짝, 나머지 홀)로 적용한다.
- 출처: 프로젝트 규약. `docs/bfield-import.md`의 Reconstruction and
  diagnostics 절.
- 구현: `crates/simrep-fly/src/physics/magnetic/imported.rs`의
  `ImportedMagneticMap::from_nodes`, `divergence_report`.
- 테스트: `crates/simrep-fly/tests/bfield_import.rs`의
  `imported_formats_flight_convergence_and_compute_timings`,
  `axisymmetric_exports_converge_to_exact_coil_flights`,
  `imported_axis_limit_transform_mirrors_superposition_and_outside`,
  `imported_map_boundary_validation_rejects_invalid_data`.

---

## 4. 중성 기체 충돌

### 식 (14). HS1 충돌률과 충돌 확률 (보고서 식 (7))

```text
n = P/(k_B T),   ν = n σ ⟨|v − v_g|⟩,   P_coll = 1 − exp(−ν Δt)
```

- 기호와 단위(SI): `P`는 기체 압력(Pa), `T`는 온도(K), `k_B`는 볼츠만 상수,
  `n`은 수밀도(m⁻³), `σ`는 충돌 단면적(m²), `⟨|v − v_g|⟩`는 맥스웰 기체에 대한
  이온의 평균 상대 속력(m/s), `ν`는 1/s다.
- mm/μs 형태: 속력을 mm/μs로 쓰면 `ν(μs⁻¹) = 10⁻³ n σ ⟨|v − v_g|⟩`가 된다.
  구현은 `P σ/(1000 k_B T)·⟨|v − v_g|⟩` 꼴로 계산한다. 명시적 평균 자유
  행로 `--mean-free-path-mm`를 주면 `ν = |v − u_g|/λ`로 대체된다(열평균이
  아니라 실제 상대 속력).
- 범위와 가정: SIMION 예제 `collision_hs1.lua`의 법칙과 상수를 따른다. 평균
  상대 속력은 맥스웰 분포 해석식과 정지 한계를 쓴다. 이벤트 시계는 지수 광학
  깊이 `τ = −ln U`를 뽑고, 수락된 스텝에서 `ν·Δt`를 차감해 깊이가 0이 되면
  충돌한다. 그래서 스텝 동안 비율이 고정되면 확률이 정확히 `1 − exp(−ν·Δt)`다.
  Lua 예제의 끝점 베르누이 판정과 달리 긴 스텝 안에서 사건을 국소화한다.
  산란은 질량중심계 등방 탄성 충돌이고, 기체 속도는 HS1의 상대속도 거부 표본
  규칙을 그대로 따른다. 충돌 비행은 비상대론이다.
- 출처: Scientific Instrument Services, SIMION 2024 예제
  `examples/collision_hs1/collision_hs1.lua`의 규칙.
  `docs/physics-magnetic-collision.md`의 HS1 절.
- 구현: `crates/simrep-fly/src/physics/collision.rs`의
  `Hs1::collision_rate_per_us`, `mean_relative_speed`, `scatter`,
  `fly_collisional`의 시계(`Process::limit_step`/`trial`/`accepted`).
- 테스트: `crates/simrep-fly/tests/physics.rs`의
  `hs1_elastic_com_and_collision_rate_limits`,
  `hs1_seed_is_reproducible_and_parallel_order_independent`;
  GPU 분포 비교는 `crates/simrep-fly/src/gpu/physics_benchmark.rs`의
  `gpu_physics_collision_distributions`(환경 변수 지정이 필요해 기본 무시).
  SIMION 대비 KS 분포 검정 기록은 `bench/default-flight.md`의 HS1 and SDS
  statistical checks 절.

### 식 (15). HS1 저전장 이동도 (Mason-Schamp 근사)

```text
K = 3 |q| / (16 n σ) · √(2π/(μ k_B T)),   v_d = K E
```

- 기호와 단위: `K`는 m²/(V·s), `E`가 V/mm이면 `v_d`는 수치상 mm/μs다.
  `μ`는 이온-기체 환산 질량(kg), `n`은 수밀도(m⁻³), `σ`는 단면적(m²)다.
- 범위와 가정: 첫 Chapman-Enskog 극한의 경구체 이동도로, 유한 전장의 정확한
  해가 아닌 저전장 근사다. 분포 검정용 독립 기준으로 쓴다.
- 출처: `docs/physics-magnetic-collision.md`의 HS1 절(예제 문서의 명명을
  따름). 구현 주석은 Mason-Schamp라고 적는다.
- 구현: `crates/simrep-fly/src/physics/collision.rs`의
  `Hs1::low_field_mobility`.
- 테스트: `collision.rs`의 모듈 테스트
  `low_field_mobility_matches_formula_with_legacy_and_si_constants`가 같은 폐형식을
  코드의 레거시 상수(정확히 일치)와 SI 상수(문서화된 상수 차이 ~2.03e-7 이내)로
  다시 계산해 비교하고, `low_field_mobility_scales_with_absolute_charge`가
  전하 크기에 대한 비례와 부호 무관성을 본다. 이 검사는 폐형식 자체의 일관성만 보고
  HS1 수송 검증은 아니다. 저전장 드리프트 감사는
  `docs/physics-magnetic-collision.md`의 독립 HS1 저전장 검사 기록에 있다
  (예측 0.0238088528877 mm/μs 대 측정 0.0239279203405 mm/μs).

### 식 (16). SDS 통계 점프

```text
r_jump = L · ICDF(log10(m_ion/m_gas), U) · √((V_thermal/L)·Δt / 100000)
```

- 기호와 단위: `L`은 평균 자유 행로, `V_thermal`은 기체 열평균 속력,
  `ICDF`는 `mbmr.dat`의 5개 질량비(1, 10, 100, 1000, 10000) 경험적 역누적
  분포다. `U`는 균일 백분위, `Δt`는 수락된 스텝 길이다. 점프는 등방이다.
- 범위와 가정: 대기압 부근의 통계적 확산 모델로, SIMION 예제
  `collision_sds.lua`와 그 `mbmr.dat` 표를 그대로 쓴다(파일은 저장소에
  복사하지 않고 경로를 지정받는다). 가우스 확산이 아니다. 끌기(drag)는
  정확한 정수 E/B 감쇠 회전·변위로 적분하고 중점 표본과 스텝 반분으로
  국소 오차를 관리한다. SDS는 위치 확산을 모델링하며, 보고하는 운동 에너지는
  HS1의 열운동 에너지가 아니다. 전하가 0이면 SDS 이동도가 성립하지 않는다.
- 출처: Scientific Instrument Services, SIMION 2024 예제
  `examples/collision_sds/collision_sds.lua` 및 `mbmr.dat`.
  `docs/physics-magnetic-collision.md`의 SDS 절.
- 구현: `crates/simrep-fly/src/physics/collision.rs`의
  `DiffusionStatistics::from_path`/`quantile`, `Sds::transport`,
  `damped_step`/`integrals`.
- 테스트: `crates/simrep-fly/tests/physics.rs`의
  `sds_runtime_icdf_transport_and_exact_drift`. 이 테스트는
  `SIMREP_MBMR` 환경 변수로 외부 `mbmr.dat` 경로가 필요해 기본 실행에서는
  무시(`#[ignore]`)된다. GPU 분포 비교
  `gpu_physics_collision_distributions`도 `SIMREP_GPU_MBM`과 Vulkan 어댑터를
  요구해 기본 무시된다.

---

## 5. 유전체

### 식 (17). 변동 유전율의 포텐셜 방정식 (보고서 식 (8) 앞식)

```text
−∇·(ε_r ∇V) = ρ_f/ε₀
```

- 기호와 단위: `ε_r`은 위치에 따라 다른 상대 유전율, `ρ_f`는 자유 전하 밀도
  (C/mm³), `V`는 전위(V)다. `ε_r = 1`이면 식 (1)로 돌아간다.
- 범위와 가정: 전기 변위의 발산이 자유 전하와 같다는 가우스 법칙을 유전율이
  위치마다 다른 경우로 쓴 것이다. 노드가 잇는 계수에 유전율을 넣어 행렬이
  대칭으로 남고 식 (4)의 갤러킨 구조를 그대로 쓴다. 재료가 포함된 풀이는
  연속 전극 cut-cell을 함께 적용하지 않는다.
- 출처: 표준 유전체 정전기학. SIMION의 유전체 정의는 SIMION 2024 도움말
  `dielectric.html`(Scientific Instrument Services, 2024)을 따른다.
  `docs/dielectric.md`의 Discretization and interfaces 절.
- 구현: `crates/simrep-solve/src/dielectric.rs`의 `Permittivity`
  (`from_nodes`/`from_cells`/`from_regions`, `solve`, `solve_poisson`,
  `solve_gpu`).
- 테스트: `crates/simrep-solve/tests/dielectric.rs`
  (`parallel_plate_interface_flux_and_grid_convergence`,
  `coaxial_dielectric_sleeve_is_second_order`,
  `dielectric_sphere_converges_to_uniform_field_solution`,
  `variable_dielectric_free_charge_and_uniform_laplace_regression`),
  `tests/nonlinear_dielectric.rs`(비선형 구성 법칙 반복).

### 식 (18). 연결 계수의 조화평균과 셀 면적평균 (보고서 식 (8) 뒷식)

```text
노드 ε 입력:      ε_ij = 2 / (1/ε_i + 1/ε_j)             (직렬, 조화평균)
SIMION 셀 ε 입력: ε_ij = 횡단 방향으로 닿는 셀들의 산술 면적평균   (병렬)
```

- 기호와 단위: `ε_i`, `ε_j`는 이웃 노드의 상대 유전율이다. 셀 입력에서는
  노드-노드 연결이 길이 방향으로 한 셀 안에 놓이고, 계수는 그 연결을 횡단하는
  셀들의 산술 면적평균(병렬 도선)이다.
- 범위와 가정: 조화평균은 반쪽씩의 직렬 저항으로, 유전체 경계에서 전기 변위
  법선 성분의 연속을 자연스럽게 만족한다. 노드에 놓인 평면 경계는 노드 중점에
  표현되고, 임의 곡면 경계의 위치 오차는 일반적으로 1차다(곡면 계면의 2차
  정확도를 주장하지 않는다). 셀 입력에서 노드 조화평균을 쓰면 노드에 정렬된
  계면이 이동·번지므로 산술 면적평균을 쓴다. 모든 무향 연결은 양쪽 행에서 같은
  방향으로 계산해 `W_i a_ij = W_j a_ji > 0`이고, 게이지를 고정하면 대칭 양의
  정부호다.
- 출처: `docs/dielectric.md`의 Discretization and interfaces 절.
  계면 근처 전기장은 한쪽 장이나 플럭스로 평가해야 한다는 주의는 SIMION
  도움말 `dielectric.html`에도 있다.
- 구현: `crates/simrep-solve/src/dielectric.rs`의 `harmonic`,
  `Permittivity::from_cells`.
- 테스트: `crates/simrep-solve/tests/dielectric.rs`의
  `parallel_plate_interface_flux_and_grid_convergence`(계면 플럭스),
  `dielectric_free_charge_at_cylindrical_axis_uses_physical_not_weighted_source`.

---

## 6. 공간전하

### 식 (19). 직접 쿨롱 합, Plummer 완화 (보고서 식 (9))

```text
E_i = k Σ_{j≠i} Q_j r_ij / (|r_ij|² + ε²)^{3/2},   k = 1/(4πε₀)
```

- 기호와 단위: `E_i`는 i번째 이온 위치의 공간전하 전기장(V/mm), `Q_j`는 j번째
  이온이 대표하는 전하(C), `r_ij`는 j에서 i로의 벡터(mm), `ε`는 완화 길이
  (mm)다. `k = 1/(4πε₀)`는 V·mm/C다.
- 범위와 가정: `ε = 0`이면 정확한 점전하 쿨롱 법칙이고, 이때 전하가 겹치면
  명시적 특이점 오류다. `ε > 0`은 문서화된 Plummer 커널이며 SIMION의 비공개
  자동 구름 반경을 흉내 내지 않는다. O(N²) 합이고, 자유 공간 장에 전극의
  외부 장을 더할 뿐 전극 전위를 다시 강제하거나 상 전하를 넣지 않는다.
- 출처: 쿨롱 법칙을 이 단위계로 옮긴 것. 완화 커널은 Plummer 형태.
  `docs/physics-space-charge.md`의 Direct Coulomb 절.
- 구현: `crates/simrep-fly/src/physics/space_charge.rs`의
  `DirectCharge::field`, 상수 `EPSILON_0_F_PER_MM`, `COULOMB_V_MM_PER_C`.
- 테스트: `crates/simrep-fly/tests/space_charge.rs`의
  `represented_charge_weights_and_coulomb_units`,
  `direct_second_order_against_exact_two_body_expansion`,
  `direct_small_and_parallel_populations_keep_source_order`.

### 식 (20). 대표 전하 배정

```text
Factor(f):      Q_i = f · q_i · e · CWF_i
Coulombic(Q):   Q_i = Q · q_i · CWF_i / Σ_j |q_j| CWF_j
```

- 기호와 단위: `q_i`는 e 단위 부호 있는 전하수, `e`는 기본 전하(C),
  `CWF_i`는 전하 가중 계수, `Q`는 총 절대 전하(C)다. 물리적 q/m이 가속을
  정하고, 대표 전하는 별도 양이다.
- 범위와 가정: 가중치는 미래 출생을 포함한 입력 집단 전체로 고정하고, 출생이나
  소실로 다시 정규화하지 않는다. 중성 입자는 원천에 기여하지 않는다.
  `beam` 모드는 전하 대신 전류를 쓰고 식 (22)를 따른다.
- 출처: SIMION charge repulsion의 CWF/총전하 규약에 맞춘 프로젝트 규약.
  `docs/physics-space-charge.md`의 Charges and units 절.
- 구현: `crates/simrep-fly/src/physics/space_charge.rs`의
  `ChargeMode::charges`.
- 테스트: `crates/simrep-fly/tests/space_charge.rs`의
  `represented_charge_weights_and_coulomb_units`.

### 식 (21). PIC 전하 적층과 푸아송 풀이

```text
노드 전하: CIC로 인접 노드에 적층 →  −L φ_sc = ρ/ε₀  →  E = −∇φ_sc, 같은 CIC 가중으로 수집
```

- 기호와 단위: `φ_sc`는 공간전하 보정 전위(V), `ρ`는 적층된 전하 밀도(C/mm³),
  `ρ/ε₀`는 V/mm²다. 평면 이중셀 부피는 면에서 반쪽 부피를 포함하고, 축대칭
  이중 부피는 축의 유한 원판을 포함하는 환형 고리 × 축방향 셀 길이다.
- 범위와 가정: 매 힘 계산마다 이전 해를 초기값으로 식 (1)의 푸아송 방정식을
  푼다. 보정은 도체 노드에서 0이라 전극 전위가 유지된다. 미러 면의 법선 E는
  정확히 0이다. 디리클레 노드와 겹치는 구름 몫은 도체에 흡수되고 진공으로
  재분배하지 않는다. CLI 메시는 명시한 bounds 위의 3D 직교 격자이고 바깥 면은
  접지 보정 경계다. GPU PIC는 같은 메시·부피·CIC를 쓰고 f64로 적층한다.
- 출처: 표준 PIC/정전기 푸아송 절차를 이 단위계로 옮긴 것.
  `docs/physics-space-charge.md`의 PIC and Poisson 절.
- 구현: `crates/simrep-fly/src/physics/space_charge.rs`의 `PicMesh`
  (`deposit`, `potentials`, `field`), `physics/ensemble.rs`,
  `crates/simrep-fly/src/physics/gpu_pic.rs`(GPU 경로).
- 테스트: `crates/simrep-fly/tests/space_charge.rs`의
  `cic_charge_conservation_cylindrical_jacobian_and_self_force`,
  `pic_second_order_against_its_converged_time_reference`,
  `uniformly_charged_sphere_envelope_and_direct_pic_agreement`,
  `uniform_cylinder_pic_expansion_matches_envelope`;
  `crates/simrep-fly/tests/gpu_pic.rs`;
  `crates/simrep-cli/tests/space_charge_cli.rs`.

### 식 (22). SIMION beam 모드: 선 전류 장

```text
λ_j = I_j · 1e-6 / (v_j·n)   (C/mm),   E_i = 2k Σ_j λ_j (r_i − r_j)_⊥ / (|r_⊥|² + ε²)
```

- 기호와 단위: `n`은 선도 이온의 진행 단위 법선, `v_j·n`은 이온 j가 선도
  평면을 앞으로 통과하는 속력(mm/μs), `λ_j`는 그 이온이 나타내는 선전하 밀도
  (C/mm), `(·)_⊥`는 선도 위치에 수직인 횡성분이다. `k = 1/(4πε₀)`(V·mm/C),
  `ε`는 명시적 선 구름 반경(mm)이다. 자기 선은 빼고, 고리(ring) 상호작용은
  자기 자신의 불연속 절반(얇은 껍질의 대칭 극한)을 쓴다. 축방향 힘은 없다.
- 범위와 가정: SIMION 문서대로 이온 하나가 정상 상태 전류 한 줄기를 나타낸다.
  선도 이온이 정하는 평면을 추종 이온이 함께 지나가며 한 번 지나는 방식이고,
  공통 시각 묶음 적분이나 반복 정상해가 아니다. SIMION의 비공개 구름 중첩
  커널은 재현하지 않는다.
- 출처: SIMION 문서의 beam 모드 규약(Scientific Instrument Services, SIMION
  2024 문서의 charge repulsion 관련 도움말). 무한 평행 선전하 장은 표준 결과.
  `crates/simrep-fly/src/physics/beam.rs` 모듈 주석.
- 구현: `crates/simrep-fly/src/physics/beam.rs`의 `beam_field`,
  `beam_field_and_time_scale`, `BeamOptions`.
- 테스트: `crates/simrep-fly/tests/beam.rs`의
  `line_current_field_uses_current_over_plane_speed_and_has_no_axial_force`,
  `uniform_current_cylinder_envelope_and_second_order_convergence`,
  `charged_rotating_leader_retains_second_order_convergence`.
