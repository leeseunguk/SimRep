[P122] 표 9는 docs/dielectric.md의 Automatic selection at the default residual tolerance 절의 값을 합한 표다. 다른 작업과 기계를 나눠 쓰는 동안 측정했다. 3D 구에서는 GPU 데이터 전송까지 넣어도 1.41 s로 SIMION보다 18배 빠르다. GPU와 CPU 가운데 어느 쪽을 쓸지는 측정한 교차점으로 정한다. 3D 격자에서 선형 유전체는 1,048,576점 이상, 비선형 유전체는 131,072점 이상이면 GPU를 쓴다(docs/dielectric.md의 Class-aware auto thresholds 절). 그림 12는 sphere_3dp의 결과다. 왼쪽 전위 단면에서 유전율 10인 구 안의 등전위선 간격이 넓어진다. 구가 분극되어 내부 전기장이 약해졌기 때문이다. 오른쪽은 축 위의 전기장이다. 바깥에서 걸어 준 1 V/mm가 구 표면 바깥에서 커지고 구 안에서 0.2572 V/mm로 줄어든다. SIMION과 SimRep의 두 선이 겹친다.

[C123] 그림 12. 유전체 구 예제의 전위 단면(SimRep)과 축 위 전기장(SimRep과 SIMION 엄격 설정). 출처: figures/F14.png, 명령은 figure-ledger.json의 F14.

[P124] 유전율이 전기장 세기에 따라 바뀌는 비선형 유전체도 넣었다. 전위를 풀고 그 전기장으로 유전율을 다시 계산하기를 수렴할 때까지 반복한다. SIMION의 비선형 구 예제에서는 처음에 SIMION과 최대 425 V 차이가 났다. 원인을 찾아보니 두 프로그램이 유전율을 정하는 데 쓰는 전기장을 서로 다르게 계산했다. SIMION은 반 칸 떨어진 전위 차이를 쓰고 SimRep은 칸 안에서 전위를 미분한다. 격자를 2배, 4배로 줄여 보니 이 차이는 1081 V, 625 V, 425 V, 226 V로 줄었고 두 방식이 같은 연속 해로 모이는 것을 확인했다(docs/dielectric.md의 Physical grid-refinement audit 절). 그래서 같은 물리 정확도에서 비교했다. 네 개 탐침 전위, 중심 전기장, 유도 쌍극자, 판 전하를 모두 SIMION 기본 설정보다 정확하게 계산하는 설정 가운데 가장 싼 것은 SimRep 2배 격자였다. 이때 CPU 43.6 s, GPU 48.4 s로 SIMION 87.5 s보다 각각 2.0배, 1.8배 빨랐다.

[P126] 이온이 많으면 이온끼리의 쿨롱 반발이 궤적을 바꾼다. SimRep은 모든 이온을 같은 시각에 맞춰 함께 움직이며 서로의 전기장을 계산한다. 가장 직접적인 방법은 식 (9)의 쌍별 합이다.

[P128] 식 (9)의 E_i는 i번째 이온 위치의 공간전하 전기장(V/mm), Q_j는 j번째 이온이 나타내는 전하(C), r_ij는 j에서 i로 향하는 벡터(mm)다. ε는 아주 가까운 두 이온의 힘이 무한대가 되지 않게 하는 완화 길이다. 0이면 정확한 점전하 쿨롱 힘이다. 이 합은 이온 수의 제곱에 비례해 늘어난다. 이온이 많아지면 전하를 격자에 나눠 놓고 식 (1)의 푸아송 방정식을 푸는 PIC 방식을 쓴다. 두 방식은 docs/physics-space-charge.md의 Direct Coulomb 절과 PIC and Poisson 절에 있다. 쿨롱 법칙을 이 문서의 단위로 옮긴 식이다.

[P129] SIMION 예제의 이온 103개로 비교했을 때 SimRep의 도착 위치 오차는 독립 기준해에 비해 1.0e-4 mm였고 SIMION은 0.59 mm였다. 계산 시간은 0.249 s 대 0.93 s였다(bench/space-charge.md의 Compute time only 절). SIMION의 beam 모드는 이온 하나가 전류 한 줄기를 나타내는 방식이다. SIMION 문서에 적힌 대로 첫 이온이 정하는 평면에 맞춰 나머지 이온을 함께 옮기게 구현했다. 그림 13는 균일한 원통 빔이 자기 반발로 퍼지는 반경을 해석해와 겹쳐 그렸다. 두 선이 겹친다. SIMION beam 예제 두 개에서 SimRep은 더 작은 오차로 각각 3.19배(0.269 s 대 0.86 s), 1.99배(2.117 s 대 4.21 s) 빨랐다(bench/space-charge.md의 Matched-precision compute verdict and defaults 절). 이온 10만 개의 PIC는 GPU가 CPU보다 1.4배 빨랐고 두 결과는 3e-16 mm 차이였다.

[C130] 그림 13. 균일한 원통 빔의 공간전하 퍼짐: SimRep beam 모드 결과와 해석해. 출처: figures/F15.png, 명령은 figure-ledger.json의 F15.

[P132] 철심, 영구자석, 전류 도체가 있으면 자기장도 방정식을 풀어야 한다. SIMION의 자기 포텐셜 예제를 따라 두 가지 꼴을 넣었다. 철심과 자석은 식 (10)의 스칼라 포텐셜로 푼다.

[P134] 식 (10)의 μᵣ은 상대 투자율, M은 자석의 자화(A/mm)다. 앞식은 자화가 만드는 자극을 원천으로 하는 포텐셜 방정식이다. 형태가 식 (8)과 같아서 유전체 솔버의 계수 계산을 그대로 쓴다. 뒷식은 그 포텐셜에서 자기장을 얻는 식이다. 이온이 날아가는 진공에서는 μᵣ = 1이고 M = 0이므로 뒷식이 식 (6)의 앞식으로 돌아간다. 결과를 기존 자기 PA 형식으로 저장하면 그대로 날릴 수 있다. 이 식과 비행 규약은 docs/magnetostatics.md의 Equations and flight contract 절에 있다. 전류는 식 (11)의 벡터 포텐셜로 푼다.

[P136] 식 (11)의 J_z는 평면에 수직으로 흐르는 전류 밀도(A/mm²), A_z는 그 방향의 벡터 포텐셜이다. 자기장은 벡터 포텐셜의 회전(curl)으로 얻는다. 축대칭 문제는 ψ = rA_φ를, 3D 문제는 세 성분을 각각 푼다. 식 (10)과 (11)은 SIMION 도움말 magnetic_potential.html의 정의를 이 문서의 기호로 옮긴 것이며 정리는 docs/magnetostatics.md의 Equations and flight contract 절, 구현은 crates/simrep-solve/src/magnetostatics.rs다.

[P137] SIMION 자기 포텐셜 예제 20개, 계산 25건으로 비교했다. 25건 모두 우리 결과가 SIMION 기본 설정보다 기준해에 가까웠다. 계산 시간은 22건 가운데 21건에서 1.7~23배 빨랐다(docs/magnetostatics.md의 Audit results 절). 처음에 느렸던 균일장 예제는 내부를 0에서 시작해 필요 없는 반복을 27번 한 것이 원인이었다. 경계가 평면이고 재료가 없으면 정확한 선형 해에서 시작하게 고쳤더니 0.704 ms로 SIMION의 24 ms보다 34배 빨라졌다(같은 문서의 Hardware GPU and uniform-field follow-up 절). 그래서 22건 모두 SIMION보다 빠르다. 그림 14은 C자 자석 예제의 자기장 크기 지도다. 철심 안에 자기장이 모이고 간극에는 균일한 자기장이 생긴다.

[C138] 그림 14. C자 자석 예제(c_magnet_2dp)의 자기장 크기 분포(SimRep 자기정역학 계산 결과). 출처: figures/F16.png, 명령은 figure-ledger.json의 F16.

[P140] 자석 설계에는 흔히 COMSOL, Opera, CST, FEMM 같은 전자기 해석 프로그램을 쓴다. 이 프로그램들이 내보낸 자기장 격자 파일을 읽어 바로 비행에 쓰는 기능을 넣었다. COMSOL의 Spreadsheet·Grid 형식, Opera의 .table, 일반 CSV·텍스트, 축대칭 (z, r, B_z, B_r) 표를 읽고 길이 단위(mm, m, cm, inch)와 자기장 단위(G, T, mT), 위치·회전·배율 변환을 지정한다. 읽어 들인 격자는 코일 지도와 같은 C2 보간으로 재구성하고 다른 자기장 소스와 겹쳐 더한다. 사용법은 docs/bfield-import.md에 있다.

[P141] 정확도는 정확한 해를 아는 자기장을 각 형식으로 내보낸 뒤 다시 읽어 날리는 방식으로 확인했다. 표 10은 원형 코일과 솔레노이드를 3D 격자로 내보냈을 때 정확한 자기장으로 날린 결과와의 도착 위치·비행시간 차이다.

[P143] 표 10은 bench/bfield-import.md의 Cartesian full-flight errors 절에서 가져왔다. 격자 간격을 절반으로 줄일 때마다 차이가 약 10~20배 줄어든다(관측 차수 3.2~4.4). 같은 자기장을 일반 CSV, Opera, COMSOL 두 형식으로 내보내도 오차는 모두 같았다. 그림 15은 이 수렴을 그림으로 그린 것이다. 가로축은 격자 간격, 세로축은 도착 위치 차이(로그축)다. 9,261점 격자에서 지도를 읽고 재구성하는 데 약 25 ms가 걸렸다. 다만 규칙 격자 파일만 받는다. 점이 불규칙하게 흩어진 데이터는 원래 프로그램에서 규칙 격자로 다시 내보내야 한다.

[C144] 그림 15. 외부 자기장 지도의 격자 간격에 따른 도착 위치 차이(정확한 자기장 대비). bench/bfield-import-20261024.csv의 측정값을 이 문서에서 다시 그렸다. 출처: figures/F17.png.

[P145] 요점 정리

<!-- HUMANIZE-SUMMARY v1.6.1
run_id: ch05b
metrics:
  char_in: 4584
  char_out: 4563
  change_rate: 5.8%
  self_check: 6/6
  grade: B
categories:
  C-11 연결어미 뒤 쉼표: 12 → 0
  I-1 '~한 것이다' 결말: 2 → 0
  I-3 '~다는 뜻이다' 결말: 1 → 0
  A-10 '~할 수 있다': 2 → 1
  A-15 추상 주어 환원: 1 → 0
  E-2 종결 다양화·장문 분리: 7 → 0
self_check:
  - 고유명사·수치·인용 100% 보존: ✅
  - 변경률 30% 이하: ✅
  - 장르 이탈 없음: ✅
  - register 보존: ✅
  - S1 잔존 0건: ✅
  - 인공 표현 추가 없음: ✅
highlights:
  - id: C-11
    before: "전위를 풀고, 그 전기장으로 유전율을 다시 계산하는 반복을 수렴할 때까지 되풀이한다."
    after: "전위를 풀고 그 전기장으로 유전율을 다시 계산하기를 수렴할 때까지 반복한다."
  - id: I-3
    before: "등전위선 간격이 넓어지는데, 이는 구가 분극되어 내부 전기장이 약해졌다는 뜻이다."
    after: "등전위선 간격이 넓어진다. 구가 분극되어 내부 전기장이 약해졌기 때문이다."
  - id: A-15
    before: "이 측정은 다른 작업과 기계를 나눠 쓰는 동안 했다."
    after: "다른 작업과 기계를 나눠 쓰는 동안 측정했다."
  - id: E-2
    before: "25건 모두 우리 결과가 SIMION 기본 설정보다 기준해에 가까웠고, 계산 시간은 22건 가운데 21건에서 1.7~23배 빨랐다"
    after: "25건 모두 우리 결과가 SIMION 기본 설정보다 기준해에 가까웠다. 계산 시간은 22건 가운데 21건에서 1.7~23배 빨랐다"
  - id: I-1
    before: "Automatic selection at the default residual tolerance 절에 있는 값을 합한 것이다."
    after: "Automatic selection at the default residual tolerance 절의 값을 합한 표다."
residual_findings: "없음 (S1·S2 잔존 0건)"
grade_reason: "B — S1 0건, S2 0건, 자체검증 6항 통과. 단 변경률 5.8%로 10% 목표 미달: 입력이 이미 사람 문체에 가까워 근거 기반 편집 여지가 적었다."
-->
