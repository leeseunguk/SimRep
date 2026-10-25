[P084] 이온은 서로 독립이므로 GPU에서 이온 하나를 스레드 하나로 날린다. 정밀 모드는 배정밀도로 CPU와 같은 적분기와 재구성을 쓰고 앱의 실시간 미리보기는 단정밀도 미리보기 모드를 쓴다. einzel 렌즈 10만 개를 배정밀도 GPU로 날리면 계산에 0.637 s가 걸렸다. SIMION 기본 품질의 21.37 s보다 33.5배 짧다. 이 값은 bench/fly-gpu.md의 Native precision study 절과 bench/simion-baseline.md의 Fly 절에서 가져왔다.

[P085] GPU 커널에서는 전기장 재구성의 다항식 계산이 대부분의 시간을 쓴다. 다항식의 6개 계수 반복을 풀어 쓰니 GPU 컴파일러가 더 좋은 코드를 만들었다. 그림 7은 그 전후의 계산 시간이다. 가로 묶음은 템플릿과 이온 수, 회색과 파랑 막대는 개선 전후, 왼쪽은 미리보기 모드(f32), 오른쪽은 정밀 모드(f64)다. 미리보기 모드에서 4중극 10만 개가 99.7 ms에서 54.9 ms로 줄었고 결과는 개선 전과 비트 단위로 같다.

[C086] 그림 7. GPU 비행 커널의 다항식 전개 전후 계산 시간(RTX 3070, Vulkan, 세 번 측정의 중앙값, 계산 패스 시간만). bench/fly-gpu.md의 Cached-power Horner specialization 절 표를 이 문서에서 다시 그렸다. 출처: figures/F10.png.

[P087] CPU와 GPU 가운데 무엇을 쓸지는 --device auto가 측정값으로 정한다. 전기장만 있는 비행에서 배정밀도를 지원하는 GPU는 이온 4,096개부터 CPU보다 빨랐다. 1,024개에서는 CPU 13.97 ms, GPU 43.72 ms였고 4,096개에서는 CPU 58.73 ms, GPU 51.28 ms였다. 이 값은 bench/fly-gpu.md의 Automatic device selection 절에서 가져왔다. GPU가 없거나 배정밀도를 지원하지 않으면 CPU를 쓴다.

[P089] 앱에서는 전압 슬라이더를 끄는 동안 이온을 계속 다시 날려 결과를 바로 보여 준다. 그림 8는 4중극 템플릿에서 슬라이더를 움직이는 실제 화면이다. 표 7은 슬라이더를 5초 동안 끌었을 때 초당 갱신 횟수다.

[C090] 그림 8. 4중극 템플릿에서 전극 전압 슬라이더를 움직이며 GPU로 실시간 재비행하는 앱 화면. 출처: figures/F09.png.

[P092] 표 7은 docs/dev/frontend.md의 제출/수거 파이프라인과 작은 비행 패킷 절에 기록된 앱 측정값이다. 개선은 두 가지다. 다음 요청을 앞 요청의 GPU 계산과 겹쳐 제출했고 결과 패킷을 줄였다. 이 측정은 4.7절의 커널 개선 전에 했으므로 4중극 10만 개의 현재 앱 갱신 횟수는 다시 재야 한다. 측정 당시 다른 프로그램이 CPU 코어 일부를 쓰고 있었다는 점도 기록에 남아 있다.

[P093] 요점 정리

[P094] SimRep은 식 (4)를 RK45 또는 Boris로 적분하고 칸 경계에서도 연속인 전기장으로 스텝을 크게 잡는다. einzel 렌즈에서 SIMION보다 오차가 4.9배 작으면서 11.5배 빠르고 3D 4중극의 정확한 표면 경로는 오차를 약 700배 줄이면서 71배 빠르다. GPU는 이온 4,096개부터 자동으로 쓰인다.

<!-- HUMANIZE-SUMMARY v1.6.1
run_id: ch04b
metrics:
  char_in: 1639
  char_out: 1634
  change_rate: 2.0%
  self_check: 6/6
  grade: A
categories:  # before → after
  C-11 연결어미 뒤 쉼표: 4 → 0
  I-1 '~것이다' 결말: 1 → 0
  I-3 '~것이다' 해설체(개선 내용): 1 → 0
self_check:
  - 고유명사·수치·인용 100% 보존: ✅
  - 변경률 30% 이하: ✅
  - 장르 이탈 없음: ✅
  - register 보존: ✅
  - S1 잔존 0건: ✅
  - 인공 표현 추가 없음: ✅
highlights:
  - id: C-11
    before: "같은 적분기와 재구성을 쓰고, 앱의 실시간 미리보기는"
    after: "같은 적분기와 재구성을 쓰고 앱의 실시간 미리보기는"
  - id: I-1
    before: "결과 패킷을 줄인 것이다."
    after: "개선은 두 가지다. ... 결과 패킷을 줄였다."
  - id: 중복 어미
    before: "이 측정은 bench/fly-gpu.md의 Automatic device selection 절에 있다."
    after: "이 값은 bench/fly-gpu.md의 Automatic device selection 절에서 가져왔다."
  - id: C-11
    before: "오차가 4.9배 작으면서 11.5배 빠르고, 3D 4중극의"
    after: "오차가 4.9배 작으면서 11.5배 빠르고 3D 4중극의"
residual_findings: lexical_diversity z=+3.21 S1 트리거: 대응 quick-rules ID 없음(어휘 다양도가 인간 기준보다 높은 방향이라 윤문 대상 아님)
grade_reason: "A — S1 잔존 0(C-11 4건 제거), 자체검증 6항 통과, 수치·옵션·장비명 보존. 변경률 2.0%는 근거 기반 국소 수정의 결과이며 상한(30%) 이내."
-->
