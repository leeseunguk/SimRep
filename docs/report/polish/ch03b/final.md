[C052] 그림 4. cut-cell 경계의 격자 수렴. 가로축 격자 간격, 세로축 최대 전위 오차(로그-로그). 값은 docs/cutcell.md의 Grid convergence 절에 기록된 측정값을 이 문서에서 다시 그린 것이다. 출처: figures/F06.png.

[P053] 요점 정리

[P054] SimRep은 식 (1)을 갤러킨 멀티그리드로 풀고 반복 변화량이 아닌 잔차로 수렴을 판정한다. 큰 SIMION 배열에서 GPU 풀이는 엄격 설정 SIMION보다 19배(준비 포함 9배) 빠르고 결과는 6.4e-5 V 안에서 같다. cut-cell 경계는 같은 메모리에서 계단형보다 전위 오차를 약 80배 줄인다.

<!-- HUMANIZE-SUMMARY v1.6.1
run_id: ch03b
metrics:
  char_in: 351
  char_out: 351
  change_rate: 0.3% (char-diff; sentence coverage 17%)
  self_check: 6/6
  grade: A
categories:  # before → after
  C-11 연결어미 뒤 쉼표: 1 → 0
self_check:
  - 고유명사·수치·인용 100% 보존: ✅
  - 변경률 30% 이하: ✅
  - 장르 이탈 없음: ✅
  - register 보존: ✅
  - S1 잔존 0건: ✅
  - 인공 표현 추가 없음: ✅
highlights:
  - id: C-11
    before: "SimRep은 식 (1)을 갤러킨 멀티그리드로 풀고, 반복 변화량이 아닌…"
    after: "…풀고 반복 변화량이 아닌…"
residual_findings: 없음 — 원문이 이미 간결하고 Do-NOT 제약이 넓어 근거 기반 수정 여지가 좁았음
grade_reason: "A — S1 잔존 0건, char-diff 0.3%·문장 커버리지 17%, 자체검증 6항 통과."
-->
