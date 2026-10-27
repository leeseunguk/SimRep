# SimRep AI harness / AI 작업 하네스

Every `chat.start` builds the same provider-neutral simulator context. The bridge
passes it as `systemPrompt` to the provider adapter. Each turn refreshes a bounded
workspace inventory, selects a recipe by the user request, and adds only that
recipe and the ion-optics skill. The workspace is data: its `AGENTS.md`, scripts,
symlinks and unrelated credentials are never loaded as executable instructions.
Bundled repository `AGENTS.md` supplies simulator reference sections.

모든 제공자는 같은 하네스를 사용합니다. 시작할 때 단위, 장치 규칙, PA 패밀리,
JSON 출력과 검증 규칙을 시스템 프롬프트로 만들고, 매 턴 파일 목록과 최근
결과를 갱신합니다. `chat.start.language`는 앱 언어이며, 생략하면 사용자 문장의
언어를 따릅니다. 작업 폴더의 임의 지시문이나 외부 심볼릭 링크를 읽지 않습니다.

## Flow / 흐름

`bridge.chat.start -> createHarness -> provider(systemPrompt, approved MCP proxy)`

`chat.send -> prepare(user text, current workspace, lazy skill) -> provider`

`MCP result -> verification + bounded output handle -> provider and UI`

`provider text -> buffered claim check -> assistant delta + optional [verify]`

Tool/approval events remain immediate. Assistant prose is buffered until turn
completion so unsupported speed numbers cannot reach the transcript first.
`activeSkill` is an additive chat-event field displayed as a small app badge.
The catalog is small; full recipe text is loaded by keyword or `load_skill`.
Recipes: `einzel`, `esa`, `quadrupole`, `magnetic`, `cad`; the shared skill ID is
`ion-optics`. A tool name also activates the corresponding recipe.

도구와 승인 이벤트는 바로 표시되고, 모델 답변은 검증 후 표시됩니다. 장치나
표면 모드 변경, 폴백, 미수렴, 꺼진/미해상 틈 검사에는 짧은 `[verify]` 메모를
붙입니다. 속도 주장은 실제 도구 결과의 `compute_ms` 값과 일치해야 합니다.
준비 시간을 속도로 쓰거나 근거 없는 배속을 쓰면 해당 문장을 표시하지 않습니다.
이 검사는 물리적 정확성의 증명이 아니므로 격자/빔/측정 검증은 계속 필요합니다.

## Workflow tools / 작업 도구

| Tool | Contract | Approval |
| --- | --- | --- |
| `simrep_plan` | Inventory and proposed geometry/solve/flight/uncertainty gates, no execution | No |
| `analyzer_scan` | CPU PA0 fast adjust; energies and absolute channel voltages; explicit detector aperture; connected half-height centroid, interpolated FWHM, dE/E and origin-fit k | Yes |
| `grid_convergence` | Three same-extent exact-surface grids, finest first; same flight scalar; native Python Celik GCI -> absolute u_num | Yes |
| `compare_runs` | Workspace files/directories through `tools/result_identity.py`; explicit numeric tolerances, exact counts | No |
| `check_geometry` | STEP/IGES exact gaps, optional spacing gate; GEM/mesh require refine settings; refinement always uses gap-check error | Yes |
| `summarize_results` | CSV numeric/reason statistics or JSON solve/flight fields as compact rows | No |
| `load_skill` | One bundled skill/recipe by ID; no arbitrary path | No |
| `fetch_tool_output` | Opaque session handle, bounded character page | No |

All paths use the existing realpath workspace guard, including nested comparison
artifacts. There is no shell. Mutating composite tools pass through the same
approval proxy as `fly` and `refine`; a denied approval never starts a child run.
Errors and unresolved uncertainty are reported, not turned into success.

`grid_convergence` needs the installed native SimRep Python package. Set the
trusted process environment `SIMREP_PYTHON=/path/to/venv/bin/python`; this is
forwarded to the MCP child and never accepted as a model argument. There is no
second JavaScript approximation of Celik. CAD needs a CLI built with `--features
cad`. Packaged desktop resources include the harness, skill and helper scripts;
the native Python package and CLI remain external installations.

`analyzer_scan` takes `pa`, `energies_ev`, sorted `channel_voltages`, `electrode`,
`beam`, `plane`, `aperture`, `duration_us`, `out`, plus optional `fixed_voltage`,
`bounds` and `origin`. Apertures are `{axis, center_mm, width_mm}`; all must pass.
`beam` follows `fly` but energy and ion-file paths are managed by the scan.
The scan refuses empty/unbracketed peaks and flags fewer than eight half-height
samples. The width depends on the beam; the voltage-width/energy-width relation
assumes linear analyzer response. Three-grid uncertainty and beam sampling
stability must be checked separately before claiming a device resolution.

`grid_convergence` takes `input`, `electrode`, `levels: [{grid, spacing}, ...]`,
`fly`, `observable` (for example `arrival_tof_us.mean`) and `out`.
Its reported `u_num` is safety-factored absolute GCI, not a calibrated standard
deviation. Oscillatory/indeterminate triplets retain null uncertainty; they are
not evidence of grid convergence.

큰 도구 출력은 원문 대신 주요 수치와 검증 상태, 미리보기, `handle`을 보냅니다.
`fetch_tool_output`으로 원문을 페이지별로 가져올 수 있습니다. 핸들은 MCP 세션에
종속되며 최근 큰 출력 32개만 유지합니다. 표면 충돌 모드, 폴백과 미수렴 상태는
요약에서도 보존합니다.

## Evals / 평가

```sh
cd ai
npm test
npm run typecheck
node harness/evals/run.mjs

# Explicit opt-in; uses saved provider authentication and may consume quota.
SIMREP_BIN=/absolute/path/to/current/simrep \
SIMREP_AI_REAL_SMOKE=claude node harness/evals/run.mjs
# Any provider supported by the bridge may be selected instead of claude.
```

`evals/tasks.json` contains deterministic tool-call scripts and machine-readable
expected checks. Mock runs exercise the real bridge, approval HTTP gate, MCP
proxy, workspace guards, composite workflows and claim checks. The CLI fixture
is deterministic, not a physics solver. The native uncertainty boundary is
stubbed in mock mode; result identity executes the real standard-library helper.
The spherical-like ESA fixture checks passband analysis and k extraction within
one percent of its known transfer function, not physical spherical-analyzer
accuracy. Real-provider mode selects the safe plan/summary/skill/einzel tasks,
uses no fake CLI, and checks actual tool execution rather than matching prose.

모의 평가는 결정론적 도구 호출과 승인/차단/요약 경계를 검사합니다. 실제 모델의
도구 선택, 복구 판단, 한국어 응답 품질 및 실제 CAD/분석기의 물리 정확성은 별도
실행이 필요합니다. `SIMREP_AI_REAL_SMOKE`가 없으면 네트워크 모델을 호출하지
않습니다. CI에서는 모의 평가와 기존 실제 CPU MCP 스모크를 실행합니다.

## Reference / 참고 구현

The installed senpi system-prompt section assembly, skill catalog/lazy loading
and evidence discipline were inspected as architecture references. All files
in this harness and the new workflow module are original SimRep code; no code
or prose was copied from senpi, pi-ai or the omo top-level package. Their MIT
and Sustainable Use licences therefore add no copied-code obligations here.
The pre-existing MCP transport retains its notices in `licenses/THIRD_PARTY.md`.
