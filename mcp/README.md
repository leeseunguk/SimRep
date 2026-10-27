# MCP 사용법 — SimRep MCP server

AI 클라이언트가 이 저장소의 `simrep` CLI를 **직접 실행**하게 하는 MCP 서버입니다.
Claude Code, Claude Desktop, 그 밖에 Model Context Protocol을 말하는 어떤
클라이언트든 붙일 수 있습니다. 붙이고 나면 "이 GEM으로 PA를 만들고 CPU로 정제해서
이온을 날려 줘" 한마디에 실제 솔버가 돕니다. 도구 결과는 CLI `--json` 요약 객체입니다.

An MCP server that lets an AI client **actually run** the `simrep` CLI in this
repository. Works with Claude Code, Claude Desktop, or any client that speaks the
Model Context Protocol. Once it is connected, "turn this GEM into a PA, refine it
on CPU, and fly the ions" runs the real solver. Each tool returns the CLI `--json`
summary object.

---

## 1. 준비물 / What you need

| | |
|---|---|
| Node.js | 18 이상 (의존성은 없습니다 — `mcp/server.mjs` 하나입니다) |
| `simrep` | `cargo build --release -p simrep-cli` 로 만든 실행 파일 |

GPU는 필수가 아닙니다. WSL과 CI에는 보통 하드웨어 Vulkan/DX12 장치가 없으니
도구 인자 `device` 는 `cpu` 를 쓰십시오. `gpu` 는 소프트웨어 어댑터를 거절합니다.

A GPU is not required. WSL and CI usually have no hardware Vulkan/DX12 device, so
pass `device: "cpu"`. `gpu` refuses a software adapter.

바이너리가 없다면 저장소 루트에서:

```sh
cargo build --release -p simrep-cli
```

단위는 mm, 마이크로초, 볼트, 가우스, amu, 기본 전하, eV, 파스칼입니다.
전극 번호는 1..1000 이고 저장 전압이 아닙니다. PA 파일은 원점을 저장하지 않습니다.

---

## 2. 붙이기 / Connecting

### Claude Code

프로젝트 루트에서:

```sh
claude mcp add simrep -- node mcp/server.mjs
```

바이너리와 작업 폴더를 명시하려면:

```sh
claude mcp add simrep \
  -e SIMREP_BIN=/absolute/path/to/simrep \
  -e SIMREP_WORKSPACE=/absolute/path/to/workspace \
  -- node mcp/server.mjs
```

붙었는지 확인: `claude mcp list`

### Claude Desktop

`claude_desktop_config.json` 에 다음을 넣습니다.
(Windows: `%APPDATA%\Claude\claude_desktop_config.json`,
macOS: `~/Library/Application Support/Claude/claude_desktop_config.json`,
Linux: `~/.config/Claude/claude_desktop_config.json`)

```json
{
  "mcpServers": {
    "simrep": {
      "command": "node",
      "args": ["/absolute/path/to/SIMION_Rep/mcp/server.mjs"],
      "env": {
        "SIMREP_BIN": "/absolute/path/to/simrep",
        "SIMREP_WORKSPACE": "/absolute/path/to/SIMION_Rep"
      }
    }
  }
}
```

넣고 나서 Claude Desktop을 완전히 종료했다가 다시 켜야 반영됩니다.

### 그 밖의 클라이언트 / Other clients

stdio로 JSON-RPC를 주고받는 표준 MCP 서버입니다. 실행 명령은
`node mcp/server.mjs` 하나이고, 설정은 아래 환경 변수로만 합니다.

---

## 3. 환경 변수 / Environment

| 변수 | 뜻 | 기본값 |
|---|---|---|
| `SIMREP_BIN` | `simrep` 실행 파일의 절대 경로 | `mcp/` 옆 `target/release/simrep` (없으면 `target/debug`) |
| `SIMREP_WORKSPACE` | 도구가 읽고 쓸 수 있는 **유일한** 폴더 | 서버를 띄운 현재 폴더 |
| `SIMREP_MCP_TIMEOUT_S` | 도구 하나가 돌 수 있는 최대 초 | `3600` |

상대 경로는 작업 폴더 기준으로 해석한 뒤, 심볼릭 링크를 따라간 실제 경로가
작업 폴더 안인지 검사합니다. 밖이면 거절합니다.

---

## 4. 도구 목록 / The tools

| 도구 | 하는 일 |
|---|---|
| `gpu_info` | 어댑터, f64 지원, 소프트웨어 어댑터 여부 |
| `pa_info` | PA/PA#/PA0 헤더와 전위 통계 |
| `gem2pa` | GEM → 미정제 `.pa#` |
| `mesh_import` | STL/OBJ/PLY/3MF/glTF → `.pa#` |
| `refine` | `.pa` / `.pa#` 솔브. `.pa#` 는 `.pa0` + `.paN` 패밀리를 씁니다 |
| `refine_geometry` | GEM/메시/CAD 연속 표면 솔브 (`part=number:voltage`) |
| `cad_measure` | STEP/IGES complete-face gap and touch matrices (`--features cad`) |
| `fly` | 이온 비행. 입자 파일 또는 생성된 빔, 전압, 검출면, bounds, device, bfield, collision, CSV |
| `export_geometry` | 전극 표면 메시; USDA에는 궤적 CSV를 곡선으로 함께 저장 가능 |
| `export_grid` | 전위 격자 VTI/VTS |
| `export_isosurface` | 등전위면 |
| `export_trajectories` | 궤적 샘플 CSV → VTP/USDA/CSV (비행 결과 CSV가 아닙니다) |
| `read_csv_summary` | 비행 결과 CSV 요약 (CLI를 부르지 않습니다) |

`fly` 의 `device` 는 `auto`, `cpu`, `gpu` 입니다. 자기장이 있거나 빔이 작으면
`auto` 도 CPU에 남습니다. 정확한 표면 충돌, direct Coulomb, beam 모드는 CPU 전용입니다.
GPU로 계단 충돌만 쓰려면 `no_cuts: true` 와 `device: "gpu"`. SDS 충돌은
번들에 없는 SIMION `mbmr.dat` 경로(`mbmr`)가 필요합니다.

성공하면 텍스트 내용이 CLI JSON입니다 (`schema_version`, `simrep_version`,
명령별 키). 실패하면 `error.message`, `error.kind`, 그리고 `error.stderr`
(프로세스 stderr)가 들어 있습니다.

`cad_measure` takes `file` and either `gap: ["pair/left", "pair/right"]` or
`all_pairs: true`. Selectors also accept zero-based body indices and explicit
face subsets, for example `"0:faces=0,1"`. `cad_tolerance` (default 0.01 mm)
limits exact/tessellation disagreement; `touch_tolerance` (default 1e-7 mm)
classifies near-touching surfaces. Touching is not proof of electrical contact.
The result includes closest points and face indices; all-pairs results include
`distance_matrix_mm` and `touching_matrix` in the reported `parts` order.

---

## 5. 해 보기 / A first conversation

동봉된 einzel 렌즈(`examples/data/einzel.gem`, `examples/data/ions.ion`)로
시작하는 것이 가장 빠릅니다.

> `examples/data/einzel.gem` 을 PA#로 만들고 CPU로 정제한 다음,
> `examples/data/ions.ion` 을 전압 1=0, 2=60, 3=0 으로 날려 줘.
> 검출면은 `1,0,0,58` mm, 시간은 2000 us, 결과는 CSV로 남기고 TOF를 알려 줘.

클라이언트는 `gem2pa` → `refine` → `fly` → `read_csv_summary` 를 부릅니다.
이 격자는 CPU에서 1초 안쪽입니다.

> Turn `examples/data/einzel.gem` into a PA#, refine it on CPU, then fly
> `examples/data/ions.ion` at voltages 1=0, 2=60, 3=0 through the plane
> `1,0,0,58` mm for 2000 us, write a CSV, and tell me the time of flight.

자주 보는 오류:

- `--device gpu requires a hardware GPU ... use --device cpu` — `device` 를 `cpu` 로.
- `surface collisions require CPU` — `device` 를 `cpu` 로 하거나 `no_cuts` 를 켜십시오.
- `refinement did not converge` — `max_cycles` 를 올리거나 `tolerance` 를 푸십시오.
- `N particles did not reach an allowed termination` — `plane`, `bounds`, `duration_us` 를 늘리거나 `allow_time_limit` 을 켜십시오.
- `SDS requires --mbmr` — SIMION `mbmr.dat` 경로를 `mbmr` 로 넘기십시오.
- `STEP/IGES import requires cargo build -p simrep-cli --features cad` — CAD 기능으로 다시 빌드하십시오.

---

## 6. 무엇을 못 하게 되어 있는가 / What it will not do

이 서버는 모델이 부르는 것이므로, 부를 수 있는 범위를 좁게 못 박아 두었습니다.

- **셸이 없습니다.** 자식 프로세스는 언제나 인자 배열로 실행되고, 실행 파일은
  `SIMREP_BIN` 하나뿐입니다. 호출자가 쓴 문자열이 명령이 되는 경로가 없습니다.
- **작업 폴더 밖으로 나가지 못합니다.** 모든 경로는 `SIMREP_WORKSPACE` 기준으로
  해석하고, 심볼릭 링크를 따라간 실제 경로가 그 안인지 검사합니다. 아니면 거절합니다.
- **파일을 지우지 않습니다.** 지우는 도구가 없습니다.
- **시간을 무한정 쓰지 않습니다.** `SIMREP_MCP_TIMEOUT_S` 가 지나면 프로세스를 죽이고
  stderr를 오류에 넣습니다.

The server is called by a model, so what it can be made to do is pinned down:
no shell, argv arrays only, one executable from `SIMREP_BIN`, every path resolved
and checked against the workspace root (symlinks included), no delete tool, and a
timeout that kills the child and returns its stderr.

---

## 7. 점검 / Checking it works

```sh
cargo build --release -p simrep-cli
SIMREP_BIN=./target/release/simrep node mcp/smoke.mjs
```

`initialize`, `tools/list`, 작업 폴더 밖 경로 거절, 그리고
`examples/data/einzel.gem` 에 대한 `gem2pa` + `refine` + `fly` (`device: cpu`)
까지 왕복하고 JSON 필드를 확인합니다. GPU가 없는 기계에서도 CPU 경로는 통과해야 합니다.

---

## 라이선스 / Licence

`mcp/server.mjs` 와 `mcp/smoke.mjs` 의 프로토콜 루프는
[Iteration-CFD `mcp/`](https://github.com/using76/Iteration-CFD/tree/1475bbf781e08f33f681661ec40e00727f1e7021/mcp)
를 고쳐 쓴 것입니다. 저작권은 Iterations Co., Ltd. 이고 라이선스는
Prosperity Public License 3.0.0 입니다. 전문과 파일 목록은
[`licenses/THIRD_PARTY.md`](../licenses/THIRD_PARTY.md) 에 있습니다.
그 라이선스는 비상업적 이용을 허용하고, 상업적 이용은 30일 시험 후 별도 허락이 필요합니다.
SimRep 도구 정의와 이 문서는 그 저장소에서 복사한 것이 아닙니다.
