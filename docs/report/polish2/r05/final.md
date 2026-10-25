[P189] 표 14는 docs/PLAN.md, docs/magnetostatics.md, bench/fly-gpu.md, docs/mesh-io.md, docs/bfield-import.md, docs/geometry-pipeline.md에 기록된 한계를 이 문서에 모았다. 이 가운데 자기장 GPU 비행의 성능은 배정밀도 연산이 빠른 GPU나 혼합 정밀도 방식으로 다시 볼 수 있다. 정확한 표면의 GPU 비행은 메시 교차 계산을 GPU로 옮기면 된다.
<!-- HUMANIZE-SUMMARY v1.6.1
run_id: r05
metrics:
  char_in: 259
  char_out: 255
  change_rate: 1.9%
  self_check: 6/6
  grade: A
categories:  # before → after
  C-11 연결어미 뒤 쉼표: 1 → 0
  I-1 '~것이다' 결말: 1 → 0
self_check:
  - 고유명사·수치·인용 100% 보존: ✅
  - 변경률 30% 이하: ✅
  - 장르 이탈 없음: ✅
  - register 보존(문어체 ~이다/~한다): ✅
  - S1 잔존 0건: ✅
  - 인공 표현 추가 없음: ✅
highlights:
  - id: I-1
    before: "이 문서에서 모은 것이다."
    after: "이 문서에 모았다."
  - id: C-11+E-1
    before: "다시 볼 수 있고, 정확한 표면의 GPU 비행은 메시 교차 계산을 GPU로 옮기면 된다."
    after: "다시 볼 수 있다. 정확한 표면의 GPU 비행은 메시 교차 계산을 GPU로 옮기면 된다."
residual_findings: 없음
grade_reason: "A — S1 잔존 0건, 자체검증 6항 통과. 단문 입력이라 변경률 1.9%에 그침."
-->
