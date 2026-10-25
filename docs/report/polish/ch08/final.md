[P176] 이 장에서 배울 것

[P177] 앞 장의 비교를 한 표에 모아 SimRep이 처음 목표를 얼마나 달성했는지 확인한다.

[P178] 그림 22는 각 장에서 정밀도를 맞춰 비교한 계산 시간 배수를 한 그림에 모았다. 가로축은 SIMION 계산 시간을 SimRep 계산 시간으로 나눈 값(로그축)이고 색은 기능 분야다. 모든 막대가 1보다 오른쪽에 있으므로 비교한 모든 경우에서 SimRep이 빨랐다.

[C179] 그림 22. 정밀도를 맞춘 비교에서 SIMION 대비 계산 시간 배수(로그축). 각 막대의 두 계산 시간과 출처는 figure-ledger.json의 F02에 있다. 출처: figures/F02.png.

[P181] 표 13에는 3장~5장에서 인용한 각 표와 같은 출처를 이 문서에서 모았다. 처음 계획의 단계 목표와 비교하면 큰 3D 전위 계산은 엄격 SIMION의 5배 이상이라는 목표를 GPU에서 넘었고 비행은 정확도를 지키며 10배 이상이라는 목표를 einzel과 4중극에서 넘었으며 같은 메모리에서 오차 10배 감소라는 목표도 넘었다(docs/PLAN.md의 단계별 계획 절). 정확도는 모든 비교에서 SIMION과 같거나 더 높았다.

[P182] 요점 정리

[P183] 정밀도를 맞춘 모든 비교에서 SimRep이 SIMION보다 빨랐고 정확도는 같거나 높았다. 계획한 세 단계 목표를 모두 넘었다.

<!-- HUMANIZE-SUMMARY v1.6.1
run_id: ch08
metrics:
  char_in: 706
  char_out: 694
  change_rate: 2.4%
  self_check: 6/6
  grade: A
categories:  # before → after
  C-11 연결어미 뒤 쉼표: 4 → 0
  I-1 '~모은 것이다' 결말: 2 → 0
self_check:
  - 고유명사·수치·인용 100% 보존: ✅
  - 변경률 30% 이하: ✅
  - 장르 이탈 없음: ✅
  - register 보존: ✅
  - S1 잔존 0건: ✅
  - 인공 표현 추가 없음: ✅
highlights:
  - id: I-1
    before: "한 그림에 모은 것이다."
    after: "한 그림에 모았다."
  - id: C-11
    before: "나눈 값(로그축)이고, 색은 기능 분야다."
    after: "나눈 값(로그축)이고 색은 기능 분야다."
  - id: C-11
    before: "비교하면, 큰 3D 전위 계산은"
    after: "비교하면 큰 3D 전위 계산은"
residual_findings: lexical_diversity z=+2.26 S1 트리거: 대응 quick-rules ID 없음
grade_reason: "A — S1 잔존 0, 자체검증 6항 통과, 출처·수치·그림 번호 보존. 변경률 2.4%는 짧은 요약 장 특성상 근거 있는 수정 범위가 좁았기 때문."
-->
