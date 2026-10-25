# 프런트엔드(데스크톱 앱) 개발 가이드

GUI는 Tauri 2 + React + TypeScript(Vite)다. 3D 뷰는 WebGPU로 그린다.

| 경로 | 내용 |
|---|---|
| `app/` | 웹 프런트엔드 (Vite + React + TS) |
| `app/src-tauri/` | Tauri 2 Rust 크레이트 `simrep-app`. **독립 cargo workspace**(`[workspace]` 테이블)라서 루트 `cargo build`/`cargo test`(WSL, `crates/*`)에 끌려 들어가지 않는다. `crates/simrep-core`, `crates/simrep-io`를 path 의존성으로 링크한다. |

## 준비물

| 항목 | 확인한 버전 |
|---|---|
| WSL Node | `~/.local/bin/node` v24 (Node 22 이상이면 됨) |
| Windows Node | `C:\Program Files\nodejs\node.exe` v24 |
| Windows Rust | `C:\Users\lsu10\.cargo\bin\cargo.exe`, `stable-x86_64-pc-windows-msvc` |
| C++ 빌드 도구 | Visual Studio 2022 (MSVC) |
| WebView2 런타임 | 155 (WebGPU 기본 활성) |

처음 한 번(WSL):

```bash
cd app
npm install
```

## 1. 브라우저 미리보기 (WSL, 가장 빠른 UI 작업 루프)

```bash
cd app
npm run dev        # http://127.0.0.1:1420
```

Windows의 Chrome/Edge 113 이상에서 `http://127.0.0.1:1420`을 열면 된다(WSL2 localhost 포워딩).
이 모드에서는 Rust 코어 없이 UI만 돈다. 상태 표시줄에 "Browser preview"가 뜬다.

## 2. Windows 데스크톱 앱 (권장: 소스·Vite는 WSL, 창은 Windows)

터미널 1 (WSL) — Vite 개발 서버:

```bash
cd app
npm run dev
```

터미널 2 (Windows PowerShell) — Tauri 창:

```powershell
$env:CARGO_TARGET_DIR = "$env:USERPROFILE\simrep-app-target"
cargo run --manifest-path \\wsl.localhost\Ubuntu\home\lsu10\orca\workspaces\SIMION\SIMION_Rep\app\src-tauri\Cargo.toml
```

같은 일을 WSL 터미널에서 Windows 툴체인으로 실행하려면:

```bash
CARGO_TARGET_DIR='C:\Users\lsu10\simrep-app-target' WSLENV=CARGO_TARGET_DIR \
  /mnt/c/Users/lsu10/.cargo/bin/cargo.exe run \
  --manifest-path '\\wsl.localhost\Ubuntu\home\lsu10\orca\workspaces\SIMION\SIMION_Rep\app\src-tauri\Cargo.toml' < /dev/null
```

- 디버그 빌드(`custom-protocol` 기능 없음)는 `tauri.conf.json`의 `build.devUrl`(`http://127.0.0.1:1420`)을 연다. 그래서 Tauri CLI 없이 `cargo run`만으로 개발 모드가 되고, WSL에서 소스를 고치면 창이 HMR로 바로 바뀐다.
- `CARGO_TARGET_DIR`을 C: 드라이브로 두는 이유: `\\wsl.localhost` 위에 target을 두면 빌드가 매우 느리다. 첫 빌드 약 1분 20초(5950X), 이후 증분 빌드는 몇 초.
- 상태 표시줄에 `Desktop · v0.1.0`과 `WebGPU · nvidia ampere`가 보이면 IPC(`backend_info` 커맨드)와 WebGPU가 모두 정상이다.
- WSL에서 띄운 경우 창을 닫거나 Ctrl+C로 끝낸다. 남은 프로세스가 있으면 `taskkill.exe /T /F /PID <pid>`로 정리한다(`tools/win/winproc.ps1 status`로 확인).

### WSL 경로에서 `npm run tauri dev`를 Windows로 직접 돌리지 않는 이유

- `npm.cmd`는 `cmd.exe`로 실행되는데, `cmd.exe`는 `\\wsl.localhost\...` 같은 UNC 경로를 현재 디렉터리로 쓰지 못한다.
- WSL에서 설치한 `node_modules`에는 Linux용 네이티브 바이너리(esbuild/rolldown, `@tauri-apps/cli`)가 들어 있어 Windows Node에서 돌지 않는다.

## 3. Windows 쪽 체크아웃에서 `npm run tauri dev`

C: 드라이브에 체크아웃(또는 복사)이 있으면 Windows Node만으로 표준 Tauri 흐름을 쓴다. 이때는 WSL의 Vite를 끄고 실행한다(같은 1420 포트).

```powershell
cd C:\dev\SIMION_Rep\app        # 루트 Cargo.toml, crates/ 도 함께 있어야 한다 (simrep-core path 의존성)
npm install
$env:CARGO_TARGET_DIR = "$env:USERPROFILE\simrep-app-target"
npm run tauri dev               # beforeDevCommand가 Vite를 띄우고 cargo run으로 창을 연다
```

이 경로로 창이 뜨고 WebGPU(nvidia ampere)와 IPC가 정상인 것을 확인했다. 설치 패키지는 아래 "Windows 설치 패키지"를 본다.

## Windows 설치 패키지 (NSIS, 사용자별 설치)

**NSIS를 쓰는 이유**: Tauri의 NSIS 번들은 `installMode: "currentUser"`로 관리자 권한 없이 `%LOCALAPPDATA%\SimRep`에 설치하고 제거 정보도 HKCU에만 쓴다. Tauri의 WiX(MSI) 번들은 컴퓨터 전체 설치(Program Files, HKLM, UAC 승격)라 "시스템에 아무것도 남기지 않는다"는 조건에 맞지 않는다. 설치 언어는 영어/한국어.

설치 파일은 `cad` 기능을 켠 release 빌드다. 구성:

| 파일 | 역할 |
|---|---|
| `tauri.conf.json` `bundle` | NSIS, `currentUser`, 설치 훅 `windows/hooks.nsh` |
| `tauri.cad.conf.json` | `--config`로 겹치는 CAD 전용 설정: `bundle-occt\*.dll`·`*.txt`·`licenses\*` → `occt\`, `bundle-occt\crt\*.dll` → 설치 폴더(실행 파일 옆) |
| `tools/win/stage-occt.ps1` | `bundle-occt\`를 만든다(gitignore). `build.rs`의 툴킷 20개에서 시작해 `dumpbin /DEPENDENTS`로 env.bat/custom.bat과 같은 폴더(`<CASROOT>\win64\vc14\bin`, 서드파티 `<이름>\bin`, openvr `bin\win64`)를 따라 DLL 묶음 전체를 모으고, 필요한 MSVC C++ 런타임을 다시 dumpbin으로 따라 `crt\`에 넣는다. OCCT LGPL 2.1 + 예외 원문, 서드파티 라이선스(`tools/win/occt-licenses/`), DLL마다 출처를 적은 `THIRD-PARTY-NOTICES.txt`(동적 링크·교체 방법·소스 위치)를 쓴다 |
| `windows/hooks.nsh` | 제거할 때 Tauri가 남기는 `HKCU\Software\SimRep\SimRep`(설치 폴더 기억)도 지운다(업데이트 제외). 사용자 데이터 `%LOCALAPPDATA%\dev.simrep.app`(복구 파일, WebView 저장소)는 제거 화면의 "앱 데이터 삭제"를 고를 때만 지운다 |

OCCT 7.7.0 묶음(2026-10-24): OCCT 툴킷 30개 + tbb 2 + FreeImage + FreeType + FFmpeg 4 + openvr = DLL 39개, MSVC 런타임 3개(`MSVCP140`, `VCRUNTIME140`, `VCRUNTIME140_1`), 70.4 MB. 그 밖의 가져오기는 Windows 시스템 DLL과 UCRT api-set뿐이다. STEP/IGES만 쓰는데도 FreeImage·FreeType·FFmpeg·openvr가 들어오는 이유: OCCT 7.7의 `TKXDESTEP`/`TKXDEIGES`가 `TKVCAF → TKV3d → TKService`를 가져오기 때문이다(불러오기만 하고 쓰지 않는다). `simrep-app.exe`는 OCCT 툴킷 12개를 지연 로드하고 `MSVCP140.dll`(simrep-cad C++ 셰임)은 바로 가져오므로, C++ 런타임은 `occt\`가 아니라 실행 파일 옆에 있어야 VC++ 재배포 패키지가 없는 PC에서도 앱이 뜬다(나중에 불러오는 OCCT DLL은 이미 올라온 런타임을 같이 쓴다).

빌드(WSL에서, Windows Node로 Tauri CLI 실행). Tauri CLI 2.12.4 Windows 패키지는 `npm pack @tauri-apps/cli@2.12.4 @tauri-apps/cli-win32-x64-msvc@2.12.4`를 `C:\Users\lsu10\simrep-tauri-cli\node_modules\@tauri-apps\`에 풀어 둔다(WSL의 `node_modules`에는 Linux 바이너리만 있음). CLI는 `TAURI_APP_PATH`/`TAURI_FRONTEND_PATH`로 WSL 소스를 가리키게 해서 `/mnt/c`에서 실행하고, Windows npm은 UNC 현재 폴더에서 못 돌므로 `beforeBuildCommand`를 비우는 설정(`{"build":{"beforeBuildCommand":""}}`, 저장소 밖 파일)을 겹치고 `dist/`는 WSL에서 먼저 만든다.

```bash
(cd app && npm run build)
s="$(wslpath -w tools/win/stage-occt.ps1)"; d="$(wslpath -w app/src-tauri)"
(cd /mnt/c && timeout 600 powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$s" -Dest "$d\bundle-occt" < /dev/null)
export TAURI_APP_PATH="$d" TAURI_FRONTEND_PATH="$(wslpath -w app)" CARGO_TARGET_DIR='C:\Users\lsu10\simrep-app-target-p6cad' \
  OCCT_INCLUDE_DIR='C:\Users\lsu10\occt\opencascade-7.7.0\inc' OCCT_LIB_DIR='C:\Users\lsu10\occt\opencascade-7.7.0\win64\vc14\lib'
export WSLENV=TAURI_APP_PATH:TAURI_FRONTEND_PATH:CARGO_TARGET_DIR:OCCT_INCLUDE_DIR:OCCT_LIB_DIR
cd /mnt/c && timeout 3000 '/mnt/c/Program Files/nodejs/node.exe' 'C:\Users\lsu10\simrep-tauri-cli\node_modules\@tauri-apps\cli\tauri.js' build \
  --features cad --bundles nsis --config "$d\tauri.cad.conf.json" --config 'C:\Users\lsu10\simrep-tauri-cli\no-before-build.conf.json' < /dev/null
```

결과: `<CARGO_TARGET_DIR>\release\bundle\nsis\SimRep_0.1.0_x64-setup.exe`, **29,873,560 B (28.49 MiB)**, 경고 0. NSIS 도구는 `%LOCALAPPDATA%\tauri\NSIS`에 받아 둔 것을 쓴다.

확인(2026-10-24, 이 PC, 일반 사용자 권한, 상태 기록: `C:\Users\lsu10\simrep-gui-test\phase6-release\state-*.txt`, 스크립트 `install-test.ps1`):

- 설치 전: `%LOCALAPPDATA%\SimRep` 없음, HKCU/HKLM 제거 키 없음, 바로 가기 없음.
- `SimRep_0.1.0_x64-setup.exe /S`: 종료 코드 0, 5.0 s. `%LOCALAPPDATA%\SimRep`에 파일 54개 95.5 MB(`simrep-app.exe`, `uninstall.exe`, C++ 런타임 3개, `occt\` DLL 39개 + 고지 + `licenses\`). HKCU 제거 키(`InstallLocation` = 위 폴더), `HKCU\Software\SimRep\SimRep`, 사용자 시작 메뉴·바탕 화면 바로 가기. **HKLM·ProgramData에는 아무것도 없다.**
- 설치한 앱 실행 → 시작 화면 **SIMION 파일 열기** → `pair.step`(구 2개, `tools/cad-fixtures`) → 마법사: 부품 2개 정상 B-rep, 미리보기 1.1–1.3 s → 전극 1 = 100 V → cut-cell 24×12×12 만들기 0.2–0.6 s → CPU refine 27회, 잔차 7.5e-11 → 이온 200개 비행, 평균 TOF 0.08756 µs. 같은 흐름을 `pair.igs`로도 했다(부품 `pair/body-0`, `pair/body-1`, 같은 TOF 통계). OCCT는 설치 폴더의 `occt\`에서 불러왔다(빌드 때의 SDK 경로와 무관하게 동작해야 하는 경로).
- `uninstall.exe /S`(설정 > 앱과 같은 경로): 종료 코드 0, 1.3 s 만에 설치 폴더 삭제, HKCU 제거 키·`HKCU\Software\SimRep`·바로 가기 모두 삭제. 앱(또는 그 WebView2 자식 프로세스)이 아직 살아 있으면 제거 프로그램이 0을 돌려주고 아무것도 지우지 않았다(앱을 끄고 2 s 뒤에 실행했을 때 한 번 관찰): 앱을 완전히 닫은 뒤 실행한다.
- 남는 것: `%LOCALAPPDATA%\dev.simrep.app`(사용자 데이터. 개발 빌드도 같은 식별자라 설치 전부터 있었다).

## 4. 검사

```bash
cd app
npm run typecheck   # tsc --noEmit (앱) + tsconfig.test.json (테스트·설정, Node 타입)
npm run build       # 타입 검사 + 프로덕션 번들 (dist/)
npm test            # vitest: 순수 로직 단위 테스트 (Node, GPU·브라우저 불필요)
npm run e2e         # Playwright: 데모 모드 스모크 테스트 (헤드리스 Chromium)
npm run check:flight # npm test 중 STR4 → f64 분석 → CSV 검사(src/scene/flight.test.ts)만 실행
```

- **`npm test`**(`vitest.config.ts`, `src/**/*.test.ts`): 패킷 디코더(STR4·SPA1·SPQ1, STR4의 f64 끝 상태 경우 포함), 분석·CSV, 그래프 도우미(`results/plot/chart.ts`), refine 진행·잔차 그래프 모델, 스토어(`useApp`, `useResults`), i18n(키·자리표시 일치), 거부 문구(`src/refusals.ts`: Rust가 보내는 모든 `Reason::new("…")` 코드에 `reason.<코드>` 문구가 있는지 Rust 소스를 읽어 확인), 데모 픽스처 디코딩. Node 24는 `localStorage`가 없어 `src/test/setup.ts`가 메모리 버전을 둔다.
- **선택지 동기화**: UI가 고르게 하는 값(refine 장치, 비행 정밀도, 전극 충돌 규칙, 빔 모양, PA 내보내기 형식, 템플릿, 마법사 만들기 방식, CAD 표면)은 `src/scene/options.ts` 목록에서 나오고 컨트롤과 `backend.ts` 타입이 그 목록을 쓴다. Rust `src-tauri/src/options.rs`(테스트 전용 모듈)가 같은 목록을 두고, `options_match_the_commands`가 값마다 실제 커맨드가 받는지(serde 열거형, 템플릿, PA 형식) 확인하며 열거형에 변형을 더하면 이름 함수의 match가 컴파일되지 않지만 `OPTIONS` 목록과 길이 상수를 그대로 둬도 테스트는 통과한다. 즉 목록에 있는 값이 명령에 수용되고 Rust JSON/TS 목록이 동기화됨을 보증하되, 모든 enum 변형이 목록에 제공됨까지는 보증하지 않는다. `options_json_is_current`가 `src/scene/engineOptions.json`이 그 목록과 같은지 보고, `src/scene/options.test.ts`가 UI 목록을 그 JSON과 비교한다. 목록을 바꾼 뒤 JSON은 `SIMREP_WRITE_OPTIONS=1 cargo test options`(앱 크레이트)로 다시 쓴다. 앱에는 고를 수 있는 적분기(integrator)가 없어 목록에도 없다.
- **데모 모드**(`npm run dev:demo` = `vite --mode demo`, `.env.demo`의 `VITE_SIMREP_DEMO=1`): `src/demo/ipc.ts`가 `@tauri-apps/api/mocks`로 Tauri IPC를 바꿔 앱을 데스크톱처럼 띄우고, 실제 네이티브 코어에서 녹화한 패킷(`src/demo/fixtures/einzel.json`: 아인젤 템플릿 .pa#, refine 채널 메시지 52개(V-사이클마다 progress 하나, 해 3개), 풀린 .pa0, fast adjust, 실시간(GPU f32)·정밀(CPU f64) 비행과 최종 속도, 이온 시계열)을 돌려준다. 그래서 실제 디코더·스토어·화면이 GPU나 Rust 없이 돈다. refine 메시지는 60 ms 간격으로 다시 보내 수렴 그래프가 자라는 것을 볼 수 있다. 주소에 `?replay=manual`을 붙이면 타이머 대신 `window.__simrepDemo.step(job)`이 메시지를 하나씩 보낸다(e2e용). 취소하면 실제 코어처럼 다음 메시지로 `cancelled` 하나만 보내고 나머지 재생을 멈춘다. 아인젤 템플릿만 있고, 전압·이온 설정을 바꿔도 녹화한 장과 비행이 그대로 온다. 저장 대화상자는 기본 경로를 돌려주고 `write_bytes` 본문은 `window.__simrepDemo.writes`에 남는다(파일은 쓰지 않음). 상태 표시줄 버전에 `(demo)`가 붙는다. `main.tsx`가 `src/demo/boot.ts`를 가장 먼저 불러오므로(`isTauri()`를 모듈을 불러올 때 보는 `scene/simulation.ts`, `results/trace.ts`보다 앞) 플래그가 없는 프로덕션 빌드에는 데모 코드와 픽스처가 들어가지 않는다(`dist/`에 `__simrepDemo` 없음 확인). Rust 쪽은 데모와 무관하다.
- **픽스처 다시 녹화**: 위 2절처럼 Vite(1420)와 debug 앱을 CDP 포트 9333으로 띄운 뒤 `cd /mnt/c && node.exe "$(wslpath -w app/scripts/capture-demo-fixtures.mjs)" "$(wslpath -w app/src/demo/fixtures/einzel.json)" < /dev/null`. 스크립트가 템플릿을 만들고 같은 커맨드를 불러 패킷을 모으며, 헤더의 로컬 폴더는 `C:\Users\demo\Documents\SimRep\einzel`로 바꾼다. 패킷 형식을 바꾸면 다시 녹화한다(`src/demo/fixtures.test.ts`가 어긋남을 잡는다).
- **`npm run e2e`**(`playwright.config.ts`, `e2e/*.spec.ts`): 데모 모드 Vite를 1430 포트로 띄우고 헤드리스 Chromium(1280×800, en-US)으로 돈다. refine은 `?replay=manual`로 테스트가 메시지를 하나씩 보내고, 커맨드는 `__simrepDemo.answered(cmd, n)`(n번째 응답까지 기다림)로 기다리므로 실행 속도에 기대는 중간 상태나 고정 대기가 없다. `e2e/refine.spec.ts`는 정상 완료(메시지 52개, 마지막 `done`)와 첫 메시지 뒤 취소(`start`·`progress`·`cancelled`만, UI 진행 표시 사라짐, 비행 없음)를 따로 검사한다. `e2e/smoke.spec.ts`는 시작 화면 → 아인젤 템플릿 → refine(수렴 그래프가 나타났다 사라짐) → 다시 비행 → TOF 히스토그램 → 내보내기 메뉴의 끝 상태 CSV(머리 + 이온 40행) → 다크/라이트 테마를 실제 클릭으로 거치고, 페이지 오류가 없어야 통과한다. WebGPU 어댑터가 없으면 3D 뷰는 대신 카드를 보여 주고 나머지는 그대로 돈다. 처음 한 번 `npx playwright install chromium`이 필요하다. 실패하면 `test-results/`에 trace가 남는다.

`check:flight`는 별도 검사가 아니라 `npm test`에 들어 있는 `src/scene/flight.test.ts`만
골라 실행하는 바로 가기다(같은 6개 경우이므로 `npm test`와 따로 세지 않는다). 같은 f32가 되는
100 / 100.000001 µs가 패킷 디코딩, 필터, 통계, CSV 왕복 뒤에도 구별되는지 확인한다. 표면 번호 폭
1/2/4 B와 f64 시작점 정렬이 다른 패킷을 모두 검사한다.
네이티브 인코딩·속도 보관·도착 TOF 통계 회귀 검사는 별도 workspace의
`cargo test --manifest-path app/src-tauri/Cargo.toml`로 실행한다
(이 PC에서는 위 Windows 툴체인·C: target 경로를 사용한다).

## 구조 요약

```
app/src/
  styles/tokens.css     디자인 토큰 (색·간격·타이포·반경·그림자·모션, light/dark). 모든 시각 값은 여기서.
  i18n/en.ts, ko.ts     UI 문자열. ko.ts는 en.ts의 키를 타입으로 강제한다. 새 문자열은 둘 다에 추가.
                        Rust가 사용자에게 보이는 이유를 말할 때는 문장 대신 코드를 보낸다: `pa.rs`의 `Reason { code, params }`
                        (`PaError::coded`, 비행 결과의 `fallback`, `compute_device`의 `reason`) → 프런트가 `reason.<code>`로 번역
                        (`reasonMessage`, scene/backend.ts). OS·파서·솔버의 영어 원문은 `detail` 매개변수로만 실려 "기술적 원인"으로 붙는다.
  state/store.ts        zustand 스토어: 장면 객체, 선택(객체 + 전극), 프로젝트 메타, 시작 화면, 로그, 토스트, GPU 상태(WebGPU + 이온 비행 장치), 테마/언어(localStorage 저장)
  state/history.ts      undo/redo (문서 필드 스냅샷, 드래그 병합, 참조 중인 네이티브 세션 유지)
  state/project.ts      .simrep 저장/열기, 자동 저장(복구 파일), 최근 프로젝트, 창 제목, 템플릿으로 새 프로젝트
  state/tour.ts         5분 따라하기 단계·진행 상태(문서 필드 아님: undo/프로젝트에 안 들어감), 첫 실행 제안(localStorage `simrep.tour`)
  scene/                메시 타입, 원통 전극 생성(tube.ts), STL 파서(stl.ts, 워커에서 실행), 형식 인식(formats.ts),
                        PA 패킷·단면(pa.ts), PA 가져오기(importPa.ts, 데스크톱 경로 열기 importFiles.ts), 표면 생성(paSurface.ts, 워커),
                        브라우저용 디코더(paDecode.ts), Tauri 커맨드 래퍼(backend.ts), refine/비행/전압 흐름(simulation.ts), 이온 소스·궤적 패킷(flight.ts),
                        전극 라벨·목록(electrodes.ts), 템플릿 메타데이터(templates.ts)
  viewport/             WebGPU 렌더러, PA 표면/단면 레이어(paLayer.ts), 궤적 레이어(trajectoryLayer.ts), 궤도 카메라, WGSL 셰이더,
                        CPU 피킹, 축 기즈모, 툴바, 단면 범례, refine 진행 칩(RefineChip.tsx)
  components/           상단 바, 장면 트리, 속성 패널, 시작 화면(StartScreen, TemplateArt), 단축키 도움말(HelpOverlay), 상태 표시줄/로그, 토스트, 드롭 오버레이,
                        따라하기 카드와 강조 고리(Tour.tsx), ui/ 기본 컴포넌트
  results/              결과 패널(ResultsPanel, 첫 비행 결과와 함께 따로 로드되는 청크): 히스토그램·검출면 지도·이온·표·비교 탭, 분석(analysis.ts),
                        선택 이온 시계열(trace.ts), 결과 스토어(store.ts: 선택 이온·보관한 결과·보기 설정), 내보내기(exporters.ts),
                        plot/ 그래프 엔진(chart.ts 모델·축·눈금, painters.ts Canvas/SVG, ChartView.tsx 호버·확대)
app/src-tauri/
  src/main.rs           Tauri 진입점, backend_info, 커맨드 등록, 대화상자 플러그인
  src/pa.rs             open_pa(경로로 열기)/read_pa(바이트, 폴백)/read_file/path_info/close_session: PA 디코딩, 바이너리 패킷 응답
  src/session.rs        열린 PA 세션(배열, fast adjust 해, 전압별 전기장 캐시, 이온 캐시, GPU에 올린 장), 앱 GPU 장치(시작 시 1회 생성)
  src/refine.rs         refine_targets/refine_start/refine_cancel: 백그라운드 스레드 multigrid, Channel로 진행·미리보기 전송
  src/fly.rs            fly/fast_adjust/ion_trace: 실시간(GPU) / 정밀(CPU) 비행, 정확한 표면(.cuts.json) 비행(CPU), 궤적 패킷 응답, 이온 하나의 시계열(CPU f64)
  src/live.rs           compute_device, 이온 비행 GPU(refine 장치 위의 f32 미리보기 파이프라인, 앱 수명 동안 유지), 배열별 GPU 상주 장(compact, 백그라운드 준비, 메모리 예산 안에서 사본 2개 + 3D 기저 셀 다항식 캐시), GPU 궤적 기록과 CPU 대체
  src/geometry.rs       inspect_geometry/build_geometry: 가져오기 마법사(GEM·메시·CAD → .pa# + .cuts.json, cut-cell 기하 보관)
  src/cad.rs            STEP/IGES 런타임 확인(기능 `cad`), Windows OCCT DLL 찾기(지연 로드)
  src/iob.rs            open_iob: SIMION 작업대 경계와 PA 배치
  src/export.rs         export_array: 전극 기하·격자·등전위면·SIMION PA 내보내기
  src/templates.rs      create_template: 템플릿 .pa#을 상자/원통 조합으로 만들어 Documents/SimRep/<종류>[-n]/에 씀
  src/project.rs        read_text/write_text/write_bytes(임시 파일 후 rename)/remove_file/recovery_path: 프로젝트·복구 파일, 내보내기(바이트 그대로)
  tauri.conf.json       창 설정 (dragDropEnabled: true → Tauri drag-drop 이벤트로 파일 경로를 받음)
```

## 가져오기 마법사 (기하 → PA)

`components/ImportWizard.tsx`(속성 패널 자리), 흐름은 `scene/importWizard.ts`, 상태는 `state/wizard.ts`(문서 객체가 아니라 undo/프로젝트에 들어가지 않음), Rust는 `src-tauri/src/geometry.rs`.

1. **부품**: `inspect_geometry`가 파일을 읽어 미리보기 패킷(`"SGP1" | 헤더 JSON | 삼각형별 f32 위치+법선`)과 격자 추천을 준다. 부품마다 색(전극 번호별 `--vp-part-*`)으로 3D에 그리고, 표에서 이름·번호(1–1000)·전압을 정한다(번호가 같으면 한 전극, 전압도 같아야 함). 메시는 단위(파일/mm/cm/m/in, glTF는 m, 3MF·CAD는 파일에 적힌 단위)와 **수리**(겹친 꼭짓점·퇴화/중복 삼각형·감김·바깥 방향, 구멍은 메우지 않음) 스위치, 부품별 결함 보고(수리 전/후)를 보여 준다. 닫힌 고체가 아닌 부품이 있으면 만들기를 막고 이유를 적는다. GEM은 전극 값(e(2) → 2)을 번호로 시작한다.
2. **격자**: 경계 처리 **Cut-cell**(메시·CAD 기본, 부호 거리로 격자 사이 표면, `.cuts.json` 보조 파일) / **격자 정렬**(SIMION식 계단, GEM 기본: pa_define 격자). 격자 간격 추천은 가장 얇은 벽(메시: 법선 방향 두께의 10번째 백분위, CAD: 가장 깊은 안쪽 거리의 2배)과 부품 사이 가장 좁은 틈에 4점이 들어가도록 1·2·5 단위로 내리고, 400만 점을 넘으면 키운다. 영역(최소·크기), 격자 크기, 점 수, 메모리/디스크 추정(전극 해 수 포함)을 보여 주고 1천만 점 이상은 경고, 4천만 점 초과는 막는다. CAD는 **CAD 곡면**을 고른다: 메시화(기본, 현 오차 간격/100, 빠름) / 정확한 B-rep(OpenCascade 거리, 메시 오차 없음, 큰 격자는 분 단위).
3. **Refine**: 저장 폴더(기본: 원본 옆, `Program Files` 아래면 `Documents\SimRep\<이름>-import`), 파일 이름, 쓸 파일 목록, 솔버(자동/GPU/CPU). **만들고 refine**은 `build_geometry`로 `.pa#`(+ `.cuts.json`)을 쓰고 → 열고 → 전극별 해를 refine한다(진행·잔차 그래프·취소는 refine 섹션과 같음). cut-cell 기하는 `AppState::geometries`에 남아 refine이 cut-cell 연산자로 푼다. 끝나면 "비행 준비 완료"와 **비행** 버튼. 영역 원점이 0이 아니면 배열을 그 위치에 놓고(배치) 이온 소스·작업대 상자도 옮긴다.

측정(2026-10-24, Windows debug 앱, RTX 3070): einzel.gem 정렬 1,820점 만들기+refine 0.17 s, 비행 결과 즉시. 템플릿에서 내보낸 STL(부품 3개, 37,376 삼각형) cut-cell 175×60×60(63만 점) 만들기 8.8 s + GPU refine, 총 22.9 s. `pair.step`(구 2개) 검사 1.0 s, 47×23×23 cut-cell(메시화) 0.55 s, `sphere.step` 12³ 정확한 B-rep 3.3 s.

### STEP/IGES (cargo 기능 `cad`)

- `app/src-tauri`의 `cad` 기능이 `simrep-cad/occt`를 켠다. 기본 빌드에는 없고, STEP/IGES를 열면 마법사가 "STEP/IGES를 가져올 수 없습니다"와 해결책(CAD 빌드 또는 메시로 내보내기), 그 아래 기술적 원인을 보여 준다(`cadUnavailable` 오류 코드).
- Windows 빌드는 `docs/dev/windows-occt.md`의 SDK(OCCT 7.7.0)로:

  ```bash
  CARGO_TARGET_DIR='C:\Users\lsu10\simrep-app-target-p4cad' \
  OCCT_INCLUDE_DIR='C:\Users\lsu10\occt\opencascade-7.7.0\inc' \
  OCCT_LIB_DIR='C:\Users\lsu10\occt\opencascade-7.7.0\win64\vc14\lib' \
  WSLENV=CARGO_TARGET_DIR:OCCT_INCLUDE_DIR:OCCT_LIB_DIR \
    /mnt/c/Users/lsu10/.cargo/bin/cargo.exe build --manifest-path "$(wslpath -w app/src-tauri/Cargo.toml)" --features cad < /dev/null
  ```

- OCCT DLL은 **지연 로드**한다(`build.rs`의 `/DELAYLOAD:TK*.dll` + `delayimp.lib`). 그래서 DLL이 없어도 앱은 뜨고, 첫 CAD 요청 때 `src/cad.rs`가 SDK를 찾아 env.bat/custom.bat과 같은 폴더(`<CASROOT>\win64\vc14\bin`과 옆의 서드파티 `<이름>\bin`, openvr은 `bin\win64`)를 이 프로세스의 PATH 앞에 넣고 툴킷 20개를 미리 불러 본다. 못 찾거나 못 불러오면 찾아본 곳을 담은 `cadUnavailable` 오류가 된다. 찾는 순서: `SIMREP_OCCT_DIR`(CASROOT 또는 DLL 폴더) → `CASROOT` → 실행 파일 옆 `occt\`(배포 시 DLL을 여기 넣는다) → 빌드할 때의 `OCCT_LIB_DIR` → `%USERPROFILE%\occt\opencascade-7.7.0`. 배포할 때는 OCCT DLL과 LGPL 고지(docs/step-import.md)를 함께 둔다. Linux는 시스템 OCCT를 그냥 링크한다.
- 검사: 유효한 B-rep인지(아니면 메시로 대체)와 경계 상자를 읽고, 미리보기는 고운 메시화의 부호 거리를 부품마다 약 64칸 격자에서 표본으로 뽑아 0 등위면(marching cubes)으로 만든다. 부품 이름은 XCAF 제품/인스턴스 경로(`pair/left`).

## SIMION 작업대 (.iob)와 작업대 상자

- `.iob`(`src-tauri/src/iob.rs`, 바이너리 배치는 SIMION 예제 148개로 확인)를 열면 참조하는 PA를 같은 폴더에서 찾아 SIMION과 같은 배치(원점·축·배율)로 열고, 작업대 경계를 각 배열의 **작업대 상자**로 켠다. 상자 밖은 장 없음, 이온은 상자에서 끝난다.
- 상자는 3D에 파란 상자로 그리고, 배열을 고르면 면 중심에 끌기 점(`viewport/BenchHandles.tsx`)이 나온다. 점을 끌면 그 면이 축을 따라 움직이고(상자 크기의 약 1/200 단위로 맞춤, 반대 면을 넘지 않음) 값이 말풍선에 보이며, 속성 패널의 상자 최소/최대와 재비행이 따라온다. 끌기 한 번이 undo 한 단계다. 화면 위치는 렌더러의 `project()`로 매 프레임 맞춘다.
- 상자가 꺼져 있고 이온이 배열 벽에서 멈추면 비행 결과에 "이온 N개 중 M개가 배열 끝에서 멈췄습니다" 경고와 **작업대 상자 추가** 버튼이 뜬다. 실제 einzel(`einzel.iob` + `einzel_10000.ion`, E2 = 110 V): 상자 없이 평균 TOF 5.105 µs(모두 +x 벽), `.iob` 상자로 15.300 µs(+x 9,552 / ±y 224씩).

## 내보내기 대화상자

상단 **내보내기**(`components/ExportDialog.tsx`, Rust `src-tauri/src/export.rs`, 궤적은 `fly.rs`의 `export_trajectories`). 데스크톱 네이티브 저장 대화상자, 기본 폴더는 프로젝트 → 배열 폴더(`Program Files`면 `Documents\SimRep`), 이름은 `<배열>-electrodes.stl` 같은 추천값.

| 종류 | 형식 | 선택 |
|---|---|---|
| 전극 기하 | STL · OBJ · glTF · GLB | 2D 원통: 360° / 절개 270° / 직접(각도), 분할 수. 2D 평면: 돌출 깊이. 부품 이름은 장면의 전극 이름 |
| 전위 격자 | VTI · VTS | 원통: x–r 평면 `.vti` 또는 회전한 3D `.vts`(각도·분할). 전기장(−∇V) 포함 여부 |
| 등전위면 | STL · OBJ · glTF · GLB | 전위(기본: 최소·최대의 가운데 둥근 값), 범위 표시 |
| 궤적 | VTP · CSV | 그린 이온을 CPU f64로 다시 날린 전체 시계열 |
| SIMION 배열 | .pa0 묶음 · .pa · .pa# | fast adjust 묶음(.pa0 + .pa1..N, 지금 전압), 계산된 전위, 전극 번호. 쓴 뒤 **다시 열기** |

배치된 배열(.iob, 원점을 옮긴 가져오기)은 작업대 좌표로 쓴다(메시는 전체 변환, VTK 격자는 평행 이동만). 확인(2026-10-24): einzel(원통)·평면 2D GEM·STL에서 만든 3D 배열로 모든 형식 19개 파일을 쓰고 구조를 검사했다(STL 삼각형 수/크기, glTF·GLB 헤더, OBJ 그룹 이름, VTK 범위). 내보낸 `.pa`와 `.pa0`을 다시 열어 같은 이온을 날리면 평균 TOF가 원본과 같다(15.2998855 µs).

스크린샷(en/ko × dark/light): `C:\Users\lsu10\simrep-screenshots\phase4-import\`. 테스트 파일: `C:\Users\lsu10\simrep-gui-test\phase4b\`.

## 3D 뷰 조작

| 입력 | 동작 |
|---|---|
| 왼쪽 드래그 | 궤도 회전 (turntable, Y-up) |
| 오른쪽/가운데 드래그, Shift+왼쪽 드래그 | 이동(pan) |
| 휠 | 커서 위치 기준 확대/축소 |
| 클릭 / 더블클릭 | 선택 / 해당 객체(빈 곳이면 전체)에 맞추기. 궤적이나 끝점을 누르면(6 px 안) 그 이온을 고르고 결과 패널 이온 탭을 연다 |
| `1` `2` `3` `4`(`0`) | 정면 / 측면(+X) / 윗면 / 등각 |
| `F` `P` `G` | 전체 맞추기 / 원근↔직교 / 격자 |
| `Esc` `Delete` | 선택 해제(이온 선택이 있으면 그것부터) / 가져온 파일 제거 |
| `Ctrl+Z` `Ctrl+Shift+Z` `Ctrl+Y` | 실행 취소 / 다시 실행 |
| `Ctrl+N` `Ctrl+O` `Ctrl+S` `Ctrl+Shift+S` | 시작 화면 / 열기 / 저장 / 다른 이름으로 저장 |
| `F2` | 장면 트리에서 선택한 항목 이름 바꾸기 |
| `?` `F1` | 단축키 도움말 |
| 좌하단 축 기즈모 | 축 끝을 누르면 그 방향에서 본다 |

좌표계는 SIMION과 같이 X가 빔(대칭) 축, Y가 위, 단위는 mm다.

## 파일 드래그 앤 드롭

- 창 어디에나 끌어다 놓거나 상단 **열기** 버튼을 쓴다.
- `.stl`(ASCII/바이너리)은 웹 워커에서 바로 파싱해 3D 뷰에 띄우고 화면을 맞춘다. 단위(mm/cm/m/inch)는 속성 패널에서 바꾼다.
- `.pa .pa# .pa0`(SIMION 전위 배열)은 바로 열린다. 아래 "전위 배열(PA) 보기" 참고.
- `.ion .fly2`(입자 정의)는 선택한 PA의 이온 소스로 붙는다(PA가 없으면 안내 토스트).
- 데스크톱에서 `.gem .stl .obj .ply .3mf .gltf .glb .step .stp .iges .igs`는 가져오기 마법사(아래 "가져오기 마법사")를 연다. `.iob`는 작업대로 열린다(아래 "SIMION 작업대"). 브라우저 미리보기에서는 STL만 바로 보이고 나머지는 데스크톱 안내 토스트를 띄운다.
- 그 밖의 확장자, 잘린/빈 STL은 토스트와 로그에 원인과 해결책을 보여 준다.
- 데스크톱: `dragDropEnabled: true`라서 Tauri의 `onDragDropEvent`가 파일 **경로**를 넘기고(`DropOverlay.tsx`), **열기** 버튼은 `@tauri-apps/plugin-dialog`의 `open()`을 쓴다(`TopBar.tsx`, 권한 `dialog:allow-open`). Rust가 경로로 직접 읽으므로 큰 파일도 IPC로 바이트를 보내지 않는다(`importFiles.ts` → `open_pa`).
- 브라우저 미리보기: HTML5 드롭/`<input type=file>`의 File 객체를 그대로 쓴다(기존 경로).

## 전위 배열(PA) 보기

데이터 흐름 (`src/scene/importPa.ts`):

1. 파일 앞 28바이트로 점 개수를 먼저 보고 한도(4천만 점, `PA_MAX_POINTS`)를 넘으면 읽기 전에 거절한다.
2. 데스크톱: 파일 경로를 `open_pa`(`src-tauri/src/pa.rs`)에 보내고, Rust가 `simrep_io::PaFile::read`로 디코딩해 세션에 보관한 뒤 바이너리 패킷 하나로 돌려준다. JSON은 헤더뿐이다.
   `"SPA1" | u32 헤더 길이 | 헤더 JSON | f32 전위[nx·ny·nz] | u8 전극 플래그[nx·ny·nz]` (JS는 `ArrayBuffer`로 받아 복사 없이 뷰를 만든다).
   헤더에는 경로, 파일 크기, 같은 폴더의 fast adjust 해(.pa0 + .pa1..N/.pa_) 상태가 들어 있다. 세션은 refine/비행이 같은 배열을 다시 읽지 않게 한다(객체를 지우면 `close_session`).
   File 객체만 있을 때(브라우저, 폴백)는 `read_pa`에 바이트를 보낸다.
   브라우저 미리보기: 같은 패킷을 웹 워커의 TypeScript 디코더(`paDecode.ts`)가 만든다.
3. 웹 워커(`pa.worker.ts`, `paSurface.ts`)가 전극 표면을 만든다. 전극점과 비전극점 사이 면만 남기고 같은 면은 이어 붙여 인스턴스 사각형(사각형당 52바이트)으로 만든다. 300만 개를 넘으면 2, 4, … 점마다 하나씩 그리고 속성 패널에 알린다(단면과 값 읽기는 항상 모든 점).
4. 오류(빈 파일, PA가 아님, 잘림, 헤더/PA0 표 손상, 너무 큼)는 해결책을 담은 토스트로 보여 주고, 기술적 원인은 활동 로그에 남긴다.

표시 (`src/viewport/paLayer.ts`, `shaders.ts`의 `paSurfaceShader`/`sliceShader`):

| 배열 | 전극 | 단면 |
|---|---|---|
| 3D | 격자점마다 상자(반 칸씩) | X/Y/Z 축과 위치 슬라이더(격자면 단위) |
| 2D 평면 (nz = 1) | Z로 ±(배열 크기의 30%) 만큼 늘인 판. 실제로는 Z로 무한히 김 | 배열 자체의 x–y 평면(z = 0) |
| 2D 원통 | x–r 단면의 경계를 X축 둘레로 96등분 회전 | 축 방향 x–r 평면(z = 0), 반지름 방향은 위아래로 대칭 |

- 전극 색은 기존 전극과 같은 규칙(중립 회색 → 양/음 색, 배열 안 최대 |V| 기준)이다. `.pa#`은 값이 전극 번호다.
- 단면 색 지도는 viridis(`--vp-field-0..4` 토큰)이고, 등전위선 간격은 1·2·5×10ⁿ V로 자동(성기게/보통/촘촘히 = 10/20/40줄)이다. 평탄한 영역에는 선을 그리지 않는다.
- "앞쪽 잘라내기"가 켜져 있으면 카메라 쪽 전극을 셰이더에서 버려, 어느 각도에서도 단면이 보인다.
- 대칭(mirror)은 GPU에서 반사 그리기로 펼친다(메모리 추가 없음). 반대칭(anti-mirror)은 전위 부호를 뒤집는다. 속성 패널에서 끌 수 있다.
- 단면 위에 마우스를 올리면 위치(mm, 원통은 x와 r)와 전위(V)를 띄운다. CPU 쪽 `sampleSlice`가 셰이더와 같은 쌍선형 보간을 한다.
- 원점은 격자점 (0,0,0)이고 좌표는 `i·dx` mm다. 처음 PA를 열면 겹치지 않도록 예제 아인젤 전극을 숨긴다(장면 트리에서 다시 켬).

`app/src-tauri/Cargo.toml`은 dev 빌드에서도 `simrep-core`, `simrep-io`, `simrep-app`를 opt-level 3으로 빌드한다(디버그 빌드 디코딩이 너무 느림). 936만 점(75 MB) 3D 배열을 여는 데 데스크톱 dev 빌드에서 약 2.2초 걸렸고, 그중 약 1.8초가 `read_pa` 호출(75 MB 보내고 47 MB 받는 IPC 왕복 + 디코딩)이다.

## Refine (전기장 풀기)

속성 패널의 **Refine** 섹션(`components/SimulationSections.tsx`, 흐름은 `scene/simulation.ts`, Rust는 `src-tauri/src/refine.rs`).

- 경로로 연 `.pa#`/`.pa`에서만 켜진다. 저장 위치 기본값은 원본 옆, **폴더 바꾸기**로 다른 폴더를 고른다. 쓸 파일 목록과 이미 있는 파일 수를 미리 보여 주고, 덮어쓸 파일이 있으면 한 번 더 묻는다.
- 솔버: 자동(기본) / GPU / CPU. 자동은 26만(2¹⁸) 점 이상일 때 Vulkan GPU, 그보다 작으면 CPU를 쓴다(작은 배열은 GPU 준비 시간이 풀이보다 길다. einzel 1,820점: GPU 1.18 s, CPU 16 ms). GPU가 없거나 버퍼 한도를 넘으면 CPU로 이어서 풀고 이유를 알린다.
- 백그라운드 스레드가 해(전극별 .pa1..N 또는 .pa_ + 번호 전극, SIMION 규약대로 10000 V 기준)를 차례로 푼다. 솔버는 끝나야 보고하므로 웜 스타트 V-cycle 묶음으로 나눠 돌린다: 1회(곧바로 거친 해), 2회, 그다음은 측정한 호출당 준비 시간과 사이클당 시간으로 약 1.5초마다 갱신되도록(준비 시간의 4배 이상) 크기를 정한다.
- 진행은 Tauri `Channel`로 온다: JSON(`start/progress/stageDone/device/done/cancelled/error`)과 바이너리 미리보기(`"SPV1" | u32 단계 | f32 전위`). 미리보기는 단면에 바로 그려지므로 수렴하는 동안 그림이 선명해진다(미리보기는 지금 푸는 해 자체라 범례가 0–10000 V일 수 있다). 패널에 단계, 사이클, 잔차 그래프, 경과 시간, 장치가 보이고 뷰포트 아래 칩에도 같은 진행과 **취소**가 있다.
- **수렴 그래프**(속성 패널 refine 진행 영역과 가져오기 마법사): `progress`는 V-사이클마다 하나(사이클 0 = 시작 장)이고 묶음 하나의 사이클들이 그 묶음이 끝날 때 함께 온다(Rust 테스트 `every_v_cycle_residual_is_streamed`). `start`가 목표 잔차(`tolerance`)와 `maxCycles`를 싣는다. 그래프는 결과 패널과 같은 그림 엔진(`results/plot`, Canvas 2D, `--plot-*` 토큰)으로 그린다: 로그 y축(자릿수 눈금 `1e-4`), 지금 푸는 해는 강조색, 앞서 푼 해는 축 색, 목표는 점선. y 범위는 항상 목표 아래까지라 수렴하는 동안 눈금이 튀지 않는다. 높이는 토큰 `--refine-chart-h`(136 px). 예전의 축 없는 SVG 스파크라인을 대신한다. 그림 엔진이 이제 처음 화면에서 필요해서 본 청크로 옮겨졌다(결과 패널 청크 53 → 41 kB).
- **거부와 대체**(`src/refusals.ts`): 요청을 거부하거나 다르게 실행한 곳(실시간 비행의 CPU 대체, GPU refine의 CPU 대체, 시작 때 GPU 없음, 비행 불가(미해결 .pa#, 자기장 배열, refine 중, 이온 파일 없음, 브라우저), 열 수 없는 파일, 브라우저 미리보기)은 모두 이 모듈을 거친다. 거부 하나는 `what`(무슨 일이 있었나)과 `why`(원인과 할 일. 대부분 네이티브 코어의 `reason.<코드>`)이고, 한 줄이면 `refusal.line`("{what}. {why}")으로 보인다. refine이 CPU로 넘어가면 예전에는 툴팁에만 있던 이유를 진행 영역에 경고색 문장으로 보여 준다.
- 허용 오차는 상대 잔차 1e-10, 최대 200 사이클. 모든 해가 수렴한 뒤에만 파일을 쓴다(취소/오류면 아무것도 안 씀). 끝나면 .pa0을 연 것처럼 객체가 바뀌고 전압 슬라이더가 생긴다.
- 17,373,979점 3D(`_bench3d/sc3d_x2.pa#`, RTX 3070): 첫 미리보기 약 1.0 s, 풀이 약 3.5 s, 파일 쓰기 포함 4.9 s.

## 이온 비행과 전압 슬라이더

- **전극 전압**(fast adjust 배열만): 전극마다 숫자 칸과 슬라이더. 바꾸면 Rust 세션이 해를 다시 중첩하고(`fast_adjust`) 단면이 바로 바뀐다. 슬라이더 범위는 배열마다 파일 전압과 현재 전압 중 가장 큰 |V|의 약 4배(1·2·5×10ⁿ로 올림, 최소 ±1 V), 단계는 범위/1000이다(아래 "단면 패킷과 전압 슬라이더 범위"). **바꾸면 다시 비행**이 켜져 있고 한 번 비행한 뒤라면 이온도 다시 날린다. 단면 요청은 하나씩(latest-wins), 재비행은 세 개까지 겹쳐 보낸다(아래 "실시간 재비행"). 모든 전극이 0 V면 안내가 뜬다.
- **이온**: 빔(개수, 질량 amu, 전하, 에너지 eV, 위치, 방향 방위/고도, 원뿔 반각, 점/선/원판 분포) 또는 `.ion`/`.fly2` 파일. 이온 수는 1천/1만/10만 빠른 선택이 있고 최대 100만 개다. **정밀도** 카드(실시간 GPU · f32 / 정밀 CPU · f64)와 **그릴 궤적 수**(기본 300, 최대 1만)를 고른다. 작업대 상자(기본: 배열 경계)를 따로 두고, 시간 제한과 **전극 충돌** 규칙을 접이식("시간 제한과 전극 충돌")으로 둔다. 전극 충돌은 **모든 모서리**(기본, SIMION과 같음: 모서리 격자점이 모두 전극인 칸에 들어가면 멈춤) / **가까운 점**(전극 격자점마다 둘레 반 칸, 예전 SimRep 방식)이고, 요청의 `collision`(`allCorners`/`nearestNode`)으로 Rust가 `GridField::with_nearest_node_collisions()`를 모든 전압 기저에 똑같이 적용한다(CPU 장 캐시와 GPU 상주 장의 키에 포함). 프로젝트에 이 값이 없으면 모든 모서리다. 가져오기 마법사의 cut-cell refine은 `.cuts.json`을 쓰지만 앱 비행은 아직 그 연속 표면 모델(`SurfaceModel`)을 쓰지 않고 PA 격자 충돌만 쓴다.
- **비행 결과**: 정밀도 배지(실시간 미리보기 / 정밀), 비행한 장치, 이온 수(비행 / 그림), **계산 시간**(GPU는 디스패치~완료, CPU는 비행 자체)을 크게, 부가 시간(장 업로드·GPU 전송·궤적·전체 왕복)을 작게 보여 준다. 실시간이 CPU로 대신 돌았으면 이유를 경고색으로 적는다. 아래는 끝난 곳(배열 벽/전극/작업대/시간 제한)별 개수와 평균 TOF, TOF 최소/평균/최대/표준편차(µs), 평균 최종 에너지. 요약은 항상 모든 이온으로 계산한다.
- 상태 표시줄의 `이온 비행 · <장치>`가 비행 장치다(`GPU 준비 중…` → GPU 이름, 없으면 `CPU (GPU 없음)`).
- 궤적 패킷: `"STR4" | u32 헤더 길이 | 헤더 JSON(8 B 정렬) | u32 그린 이온 번호[drawn] | u32 그린 이온의 시작 오프셋[drawn + 1] | f32 점[x,y,z] | 8 B 정렬 패딩 | f64 끝점[x,y,z,TOF] | f64 최종 운동 에너지(eV) | u8/u16/u32 끝난 곳 번호`(폭은 헤더 `surfaceBytes`, 끝난 곳이 256개 이하면 1 B. 이온당 41 B; STR3의 21 B보다 20 B 증가, 궤적 뒤 패딩은 0 또는 4 B). 끝 상태는 두 모드 모두 f64로 전달·분석하며 GPU 계산 자체의 정확도는 f32다. `parseFlight`가 오프셋과 끝난 곳 번호를 이온마다 하나씩(그리지 않는 이온은 빈 구간)으로 넓힌다. 최종 속도는 끝 상태 CSV 내보내기만 쓰므로 패킷에 싣지 않고, Rust 세션이 최근 비행 4개분을 f64로 헤더의 `token`에 보관했다가 `flight_velocities`로 준다(이온당 24 B, 종전 12 B). 점이 많으면 Rust가 궤적을 솎아 보낸다(`thinned`). `precision`은 요청 모드이며 `executed: { device, precision, surfaces }`가 실제 실행 경로를 기록한다. 정확한 표면을 켠 Live 요청은 `{ device: "cpu", precision: "f64", surfaces: "exact" }`다.
- 그리기(`viewport/trajectoryLayer.ts`, `trajectoryShader`): 그리는 궤적을 한 line-strip 그리기(primitive restart)로, 모든 이온의 끝점은 인스턴스 사각형(splat, 전극에 맞으면 다른 색)으로 그린다. 선 투명도는 `2.5/√그린 궤적 수`(0.06–0.8)라서 궤적이 많으면 밀도 지도처럼 보이고 초점이 드러난다. 전체 맞추기는 궤적도 포함한다.

### 실시간 재비행 (GPU)

- **장치**: 앱이 뜨면 백그라운드 스레드가 refine용 멀티그리드 장치(`simrep_solve::gpu::Gpu`, Vulkan, `refine::probe_gpu`)를 한 번 만들고, 같은 `wgpu::Device`/큐로 이온 비행 파이프라인을 만든다(`simrep_fly::gpu::Gpu::from_device_with_precision(…, Precision::Preview)`, `live::probe`). 앱 전체가 GPU 장치 하나를 쓴다. 비행 파이프라인은 f32 미리보기다(UI의 "GPU · f32"와 같다. 그냥 `Gpu::new`는 SHADER_F64가 있으면 f64를 고른다). 둘 다 앱이 끝날 때까지 둔다. GUI refine은 이 장치를 쓰므로 첫 GPU refine에 장치 생성 시간이 없다(4중극 16.6만 점 3해: 0.56 s). refine 장치가 없으면(Vulkan 없음 등) 비행 장치도 같은 이유로 없다.
- **배열별 상주 장**: 배열(세션)과 작업대 상자마다 처음 실시간 비행할 때 PA0 기준 배열 + 조절 전극마다 단위 전압 해(해 / 기준 전압)를 `GridField::new_compact`로 만들어(3D는 셀 다항식 대신 격자점 도함수만 둔다) `Gpu::prepare`로 올린다. 백그라운드 스레드에서 하므로 그동안의 비행은 CPU로 하고 결과에 이유를 적는다(아인젤 템플릿 약 30 ms, 4중극 16.6만 점 3D 약 1 s). 그 뒤 전압이 바뀌면 `set_voltages([1, V1−V1₀, …])`로 uniform만 바꾸고 다시 날린다(3D는 이때 GPU가 기저를 셀 다항식으로 펼친다). 배열을 다시 올리지 않는다. refine 결과는 새 세션이라 다시 준비한다. 3D는 `PreparedField::cache_basis_powers`로 기저의 셀 다항식을 GPU에 상주시켜 전압이 바뀔 때의 펼치기(4중극 1만 개 비행 32 ms 중 10 ms)를 없앤다. 캐시 여부는 `live::plan`이 올린 장의 실제 GPU 메모리(`PreparedField::gpu_memory_bytes`)로 정한다: compact 3D 장은 합성된 셀 다항식 한 벌 + 훨씬 작은 격자점 기저라서 캐시를 더하면 많아야 `compact × (1 + 기저 수)` 바이트이고, 그것이 2 GiB(`LIVE_VRAM_BUDGET`) 안이면 캐시한다(장치 한도를 넘으면 compact로 남는다). 4중극 템플릿(기저 4개)은 캐시 + 비행 scratch로 0.71 GB(1만 개)–0.74 GB(10만 개)다(예전 사본 2개 + 캐시 약 1.3 GB). 장은 배열마다 **하나**이고(`live::Resident`), 동시에 온 재비행은 모두 그 장을 쓴다: 잠금은 전압 설정과 제출(`PreparedField::submit`/`submit_sampled`: 이온 변환·업로드·큐에 넣기)만 감싸고, 결과 받기(`PendingFlight::collect`: GPU 대기·읽기)는 잠금 밖에서 하므로 다음 요청이 앞 비행이 GPU에 있는 동안 자기 비행을 바로 뒤에 넣는다. 디스패치당 이온 수는 크레이트 기본값(131,072, 장치 한도로 자름, 헤더 `dispatchIons`)이라 10만 개는 한 번에 날고, 헤더 `gpuBytes`가 장의 GPU 메모리(scratch 포함)를 알려 준다.
- **궤적**: 그릴 이온(전체에서 고르게 뽑은 N개)의 궤적은 같은 GPU 비행이 기록한다(`PreparedField::fly_sampled`, `TrajectorySampling`). 이온당 표본 수는 앞 비행에서 가장 긴 그린 궤적의 단계 수(+25%)로 정하고, 그 수가 화면에 남길 점 수(`POINT_BUDGET`/그린 수, 최대 1,500)를 넘으면 N단계마다 기록한다(표본 버퍼 전체를 읽어 오므로 필요한 만큼만 잡는다). 용량을 넘으면 API가 오류를 내므로, 그 요청은 끝 상태만 다시 GPU로 날리고 궤적은 예전처럼 CPU가 같은 상주 기저를 f64로 합성(`LiveField`)해 기록한다. 다음 요청부터는 기록 간격을 늘린다(확인: 4중극 템플릿 1 eV 이온 1만 개를 모두 그리기, 첫 비행만 CPU 궤적 6.4 s, 이후 GPU). 결과 패널의 부가 시간에 "궤적은 GPU에서 기록"이 보인다.
- **흐름**(`scene/simulation.ts`): 슬라이더 변경마다 단면(`fast_adjust`, 한 번에 하나)과 재비행(최대 세 개 동시, `LIVE_DEPTH`)을 따로 보낸다. 재비행 요청은 보낼 때의 최신 전압을 싣고, 그 사이의 변경은 표시만 해 두었다가(dirty) 자리가 나면 최신 값 하나로 보낸다(오래된 값은 날지 않는다). 네이티브 쪽은 세 요청을 GPU 큐에 차례로 넣으므로 끝난 비행이 읽히고 IPC를 건너고 그려지는 동안에도 GPU가 쉬지 않는다. 결과는 이미 보인 것보다 새것만 반영한다(`claimResult`).
- **정밀**(CPU): 모든 이온을 CPU f64로(끝 상태만) 날리고 그릴 이온만 궤적을 기록한다. 실시간과 같은 슬라이더 흐름이지만 느리다. 실제 einzel 1만 개(E2 = 110 V)에서 실시간과 정밀의 평균 TOF는 15.2971979 / 15.2971967 µs, 끝난 곳 개수는 같았다.

측정(2026-10-23, release 앱 + 프로덕션 React(`NODE_ENV=production` Vite), RTX 3070, 1440×900, 그릴 궤적 300개, CDP 스크립트가 requestAnimationFrame마다(약 64 Hz) 전압을 바꾸며 5초 끌기. "갱신/s"는 스토어(→ 뷰포트)에 반영된 새 비행 결과 수, 계산은 GPU 디스패치~완료 중앙값):

| 배열 | 이온 | 갱신/s | 왕복 중앙값 ms | 계산 ms | CPU 궤적 ms |
|---|---:|---:|---:|---:|---:|
| 아인젤 템플릿 201×41 (2D) | 1,000 | 62.6 | 13.1 | 5.3 | 5.2 |
| | 10,000 | 62.2 | 20.3 | 5.0 | 5.1 |
| | 100,000 | 17.3 | 99.2 | 39.8 | 5.5 |
| 실제 `_real_einzel/einzel.pa0` 91×20 (2D, `.ion` 파일, 작업대) | 1,000 | 61.2 | 17.7 | 5.0 | 5.6 |
| | 10,000 | 60.9 | 21.1 | 5.3 | 5.8 |
| | 100,000 | 16.8 | 103.6 | 42.4 | 6.1 |
| 4중극 템플릿 121×37×37 (3D) | 1,000 | 22.0 | 26.6 | 20.0 | 13.2 |
| | 10,000 | 19.2–24.9 | 43–55 | 22.9 | 15.0 |
| | 100,000 | 5.0 | 362 | 172 | 13.3 |

- 2D 배열은 1만 개가 입력 속도(약 62/s)를 그대로 따라가고 10만 개도 10/s를 넘는다.
- 3D 4중극은 목표(1만 30/s, 10만 10/s)에 못 미친다. 원인은 GPU 커널 시간이다: 이온 100개도 약 18 ms(가장 느린 이온의 순차 RK 단계 지연), 10만 개는 약 145–170 ms(3D 경계 인식 5차 패치, 셀당 216계수)라서 앱 쪽 겹치기로는 넘을 수 없다. 전압 합성(combine) 패스는 1 ms 미만이다.
- 따로 한 번만 바꿀 때 계산 시간은 끌 때보다 길게 나온다(아인젤 1만: 40–65 ms vs 5 ms). GPU가 쉬다가 깨어나는 시간으로 보인다.
- 백엔드에서 빨라질 곳: (1) 이온 버퍼를 비행마다 새로 만들고 96 B/이온을 읽어 오는 대신 상주 이온 버퍼와 작은 끝 상태(위치·TOF·이유)만 읽는 API(10만 개 GPU 전송 약 15–20 ms), (2) 표시할 이온의 N단계마다 위치를 GPU가 기록하는 옵션(CPU 재비행 제거), (3) 3D 장의 더 가벼운 GPU 표현.
- 스크린샷: `C:\Users\lsu10\simrep-screenshots\phase2-gpu-live\`. 원자료: `C:\Users\lsu10\simrep-gui-test\gpulive\results.json`.

### 정확한 전극 표면 (.cuts.json)

- **어디서 오나**: 가져오기 마법사의 cut-cell 만들기는 `<이름>.cuts.json`을 **버전 3**으로 쓴다(`geometry.rs`). 격자 연결의 표면 위치에 더해 부품마다 원본 형상(`source_geometry`: GEM은 CSG(`SourceGeometry::gem`), 메시는 삼각형(`SourceGeometry::mesh`), CAD는 현 오차 간격/100의 고운 메시)을 담는다. 형식은 docs/geometry-pipeline.md(키는 snake_case. 예전 앱이 쓴 camelCase v1 파일은 어느 리더도 읽지 못했다). 격자 정렬로 다시 만들거나 보조 파일을 끄면 같은 이름의 옛 `.cuts.json`을 지운다. refine을 다른 폴더에 쓰면 `.cuts.json`도 `.pa0` 옆으로 복사한다.
- **비행**: 경로로 연 `.pa0`/`.pa`(`.pa#` 제외) 옆에 `<이름>.cuts.json`이 있으면(`simrep fly`와 같은 규칙) 패킷 헤더 `cuts`에 이름이 실리고, 비행은 `SurfaceModel::from_reader(…).with_tolerance(위치 허용 오차 / 배율)`을 `InstanceField::with_surface`로 붙인 장으로 한다(`Session::exact`, 전압·작업대 상자·배치마다 캐시). 장(전위)은 그대로이고 전극 충돌만 격자점 대신 표면에서 일어난다. 끝난 곳은 표면의 전극 번호다. 표면 충돌은 CPU에서만 되므로(백엔드 규칙: `simrep fly`도 CPU로 강제) 실시간 모드도 CPU로 날고 결과에 "실시간 미리보기를 CPU로 비행했습니다: 정확한 전극 표면은 CPU에서만 계산합니다…"(`reason.exactSurfaces`)가 뜬다. 이온 탭 시계열과 궤적 내보내기도 같은 장을 쓴다. 읽을 수 없거나 배열과 맞지 않는 보조 파일은 무시하지 않고 비행 오류(`reason.cutsInvalid`, 다시 만들거나 끄라는 안내)가 된다.
- **UI**: 이온 섹션의 **정확한 표면** 카드(보조 파일 이름, 켜기/끄기 스위치, 무엇이 달라지는지)와 결과 섹션의 **전극** 줄·결과 패널 머리의 배지(정확한 표면 / 근사 표면(v1·v2) / 격자점). 끄면 격자점(계단)과 전극 충돌 규칙으로 날고 실시간은 GPU를 쓴다. 설정은 이온 소스의 `exactSurfaces`로 프로젝트 파일에 남는다(없으면 켜짐).
- **GEM cut-cell 풀이**: 마법사는 `simrep refine-geometry`와 같은 공개 API로 GEM을 푼다. 부품은 `Gem::electrode_sdfs()`의 정확한 CSG(`ElectrodeSdf`, 표본 격자 없음)이고, 솔버는 CLI의 `SolverShape`와 같은 래퍼(`geometry.rs`의 `SolverShape`: 원통은 반지름 축을 x로, 표면 허용 오차 `1e-9·h_min` 안의 거리는 0으로, CSG의 `segment_distance_bounds`/`segment_single_interval`을 그대로 넘겨 구간 추적)로 `CutCells::new_with_root_tolerance`에 들어간다. 근 허용 오차와 보조 파일의 연결 위치(`surface_fraction`)도 CLI처럼 원래 거리와 `1e-9·h_min`이다. 예전에는 CSG를 4배 표본 SDF(`sample_electrode_sdfs` → `SampledSdf`, 추적 경계 ÷√축 수)로 풀었고, 닫힌 함수(closure)로 감싸 구간 경계도 잃었다. 메시·CAD 부품은 바뀌지 않았다.
- **확인**(2026-10-24, release 앱, RTX 3070): 4중극 템플릿의 물리 형상 GEM(`_quad_cut/quadrupole.gem`, 121×37×37, 0.5 mm)을 마법사에서 cut-cell로 만들고 refine(만들기 1.18 s(4배 표본일 때 16.4 s), 만들기~refine 끝 3.8 s) → `particles.ion` 1,000개, 전압 1=2.5, 2=−2.5, 3=0 V, 작업대 상자 0,−18,−18–60,18,18 mm. 같은 GEM을 이 저장소의 크레이트로 빌드한 `simrep refine-geometry --electrode 1=1:2.5 --electrode 2=2:-2.5 --electrode 3=3:0`(CPU, 기본 허용 오차)으로 만든 파일과 비교하면 `.pa0`·`.pa1`·`.pa2`·`.pa#`은 바이트까지 같고 `.pa3`은 최대 1.6e-11 V(10 kV 기저) 다르다. 전극 점 15,914 / 15,914 / 7,878, cut 노드 11,160(CLI `cut_nodes=11160`). 앱 파일을 `simrep fly --device cpu --cuts quadrupole.cuts.json --tolerance 1e-8 --max-step-us 0.1`(앱 비행 설정과 같음)로 날린 결과와 이온별로 비교:

  | | +x 경계 | 전극 3 | 전극 2 | 평균 TOF(전체) µs |
  |---|---:|---:|---:|---:|
  | 앱 정밀 / 실시간(→CPU) | 750 | 230 | 20 | 4.1145034797 |
  | CLI `fly --cuts` | 750 | 230 | 20 | 4.1145034797 |
  | 앱 표면 끔(계단) / CLI `--no-cuts` | 751 / 751 | 249 / 249 | 0 / 0 | 4.124046832 / 4.124046832 |

  앱–CLI 이온별: 끝난 곳 불일치 0, 끝점 RMS 2.3e-7 mm(최대 1.8e-6, 앱 패킷이 f32), TOF RMS 1.2e-7 µs. 독립 물리 8x 기준(`compare_quadrupole.py`): 앱 정밀 비행 자체 0.0034354 mm RMS(최대 0.0202, 예전 4배 표본 풀이 0.00444), 끝난 곳 변화 0. 앱이 만든 파일을 기준 설정(`--tolerance 1e-11 --max-step-us 0.025`)으로 CLI 비행하면 **0.003435341 mm RMS, 최대 0.02018961 mm, 끝난 곳 변화 0**으로, CLI `refine-geometry` 파일의 결과(docs/geometry-pipeline.md)와 자릿수까지 같다. 첫 정확한 표면 비행은 보조 파일(12 MB) 읽기와 장 준비로 약 4 s 걸린다(CLI 준비 3.5 s). 이후 같은 전압은 캐시된다. `.pa0`를 따로 열어도(옆의 `.cuts.json`) 같은 결과였다. STL(아인젤 템플릿에서 내보낸 3부품)과 `pair.step`(CAD 빌드)도 v3 보조 파일을 만들고 정확한 표면으로 날았다. 기록: `C:\Users\lsu10\simrep-gui-test\phase7\quad\`(`cli-*.csv`), `phase7\quad-compare.txt`. 정확한 CSG 풀이 확인: `phase7b\quad\`(앱 파일, `cli\`에 CLI 파일, `app-precise.csv`, `cli-appfiles-*.csv`), `phase7b\quad-compare.txt`(스크립트 `phase7b\quadwiz.js`).
- 스크린샷(en/ko × dark/light): `C:\Users\lsu10\simrep-screenshots\phase7-exact\`. `01` 마법사 cut-cell 보조 파일, `02` 정확한 표면 켜짐(실시간→CPU 안내), `03` 결과 섹션, `04` 끔(GPU 실시간).

### GPU API 적용 전후 실시간 갱신 (2026-10-24)

위와 같은 CDP 끌기(5초, 그릴 궤적 300개, release 앱 + 프로덕션 React, RTX 3070). **이전**은 이 변경 직전의 앱이다. 비행 크레이트의 기본 정밀도가 그사이 f64가 되어(`Gpu::new`) 위의 2단계 표보다 느리고, 궤적은 CPU가 다시 날렸다. **이후**는 장치 공유 + f32 미리보기 + compact 장 + GPU 궤적 기록이다. 계산은 GPU 타임스탬프, 전송은 GPU 왕복 중 계산 밖(업로드·합성·읽기), 네이티브는 `fly` 커맨드 전체(중앙값):

| 템플릿 | 이온 | 이전 갱신/s | 이전 계산 / CPU 궤적 / 네이티브 ms | 이후 갱신/s | 이후 계산 / 전송 / 네이티브 ms |
|---|---:|---:|---|---:|---|
| 아인젤 | 10,000 | 10.0 | 57.2 / 71.8 / 94.9 | **47.4** | 5.8 / 4.4 / 11.1 |
| 아인젤 | 100,000 | 1.8 | 477.6 / 73.9 / 864.7 | **10.6** | 36.1 / 29.5 / 84.5 |
| 4중극 | 10,000 | 8.0 | 70.7 / 70.9 / 137.5 | **8.2** | 19.1 / 15.3 / 35.3 |
| 4중극 | 100,000 | 1.2 | 640.7 / 54.2 / 1,319.8 | **5.4** | 120.5 / 41.2 / 241.9 |

- `cache_basis_powers`를 켜고 잰 4중극: 1만 개 8.2/s(전송 5.8 ms, 네이티브 29.9 ms), 10만 개 5.6/s(네이티브 224.6 ms). 아인젤(2D)은 영향 없음(47.8 / 10.4/s). 이때는 갱신/s가 같아 쓰지 않았지만, 아래처럼 이 표의 4중극 갱신/s 자체가 측정 스크립트 탓이었다.
- 원자료: `C:\Users\lsu10\simrep-gui-test\phase7\before.json`, `after.json`, `after-cache.json`(스크립트 `helper.js`, `run.js`).

### 4중극 실시간 재비행 병목 (2026-10-24)

위 표의 4중극 "약 8/s"는 앱의 한계가 아니었다. CDP 스크립트가 전압을 0.1 V로 반올림했는데(`Math.round(v * 10) / 10`), 4중극 끌기는 1.5 → 3.5 → 1.5 V라 5초 동안 서로 다른 값이 40개뿐이다. 스토어는 값이 바뀔 때만 단면·재비행을 보내므로 40번 / 5 s = 8/s가 위쪽 한계였다(아인젤 60 → 100 V 끌기는 가능한 값이 800개라 프레임마다 바뀌어 영향 없음). 요청이 "약 54 ms 간격으로 하나씩" 나가고 늘 `fast_adjust`와 짝을 이룬 것도 이 때문이다. 스크립트를 1e-4 V 단위로 바꾸니(5초에 서로 다른 값 약 270개) 같은 앱이 1만 개 23.9/s였다. 실제 슬라이더는 ±1000 V 범위에 1 V 단위(`VoltageControl`: 범위 = max(1000, …), 단계 = 범위/1000)라 ±2.5 V 전극에서는 한 픽셀이 약 8 V다(아래 "남은 일").

그 뒤의 실제 한계를 단계별 시각으로 쟀다(fetch 호출·응답 머리·본문 시각(JS) + `fly` 커맨드 진입·`run` 시작/끝(Rust, 임시 계측, 지금은 뺐다)). 4중극 1만 개, 끌기 중 중앙값:

| 구간 | 이전(장 1개, 동시 2) | 사본 2 + 캐시, 동시 2 | 사본 2 + 캐시, 동시 3 |
|---|---:|---:|---:|
| 요청 IPC (JS → 커맨드) | – | 3.3 ms | 1.1–1.9 ms |
| 네이티브 `run`(잠금 대기 포함) | 44.9 ms | 25.3 ms | 26.5–28.0 ms |
| 응답 머리까지 / 본문(579 KB) | – | 14.2 / 12.4 ms | 17–18 / 14–17 ms |

(이전 열의 IPC는 Rust 시각 없이 쟀다: fetch 호출에서 응답 머리까지 62 ms, 본문 5 ms.)

- **GPU 쪽**: 같은 크레이트 API로 만든 벤치(앱 밖, 템플릿 배열, 비행 하나의 `Timings`)에서 1만 개 비행 32 ms = 준비 1.4 + 업로드 0.3 + **기저 펼치기(combine) 10.0** + 계산 15.2 + 읽기 3.1 ms였다. 위 2단계 표 아래의 "합성 1 ms 미만"은 compact 장 이전(셀 다항식을 CPU에서 만들어 올리던 때)의 값이고, 3D compact 기저는 전압이 바뀔 때마다 셀당 216계수를 다시 펼친다. 세션의 GPU 장 하나를 잠그고 날리므로 겹쳐 보낸 두 요청도 GPU에서 차례로 돌았다. 10만 개는 크레이트가 32,768개씩 4번 나눠 차례로 디스패치하고(`MAX_IONS_PER_DISPATCH`), 그때마다 지연이 가장 느린 이온의 순차 RK 단계로 정해져 계산만 약 89 ms다. 벤치(3회 중앙값, 비행당 ms, 두 요청 동시): 사본 1개 30.8 / 129(1만 / 10만), 사본 2개 27.2 / 104, 캐시 1개 21.2 / 120, **사본 2 + 캐시 17.6 / 96**. 한 요청을 여러 사본에 나눠 동시에 날리면 오히려 느렸다(10만 개 4조각 292 ms): wgpu 큐 하나에서 다른 제출의 커널은 거의 겹치지 않는다.
- **IPC 쪽**: Tauri 응답은 쉬는 앱에서도 크기에 비례한다(`read_file` 12회 중앙값: 100 KB 2.5 ms, 580 KB 7.8 ms, 830 KB 11 ms, 4.2 MB 58 ms, 약 13 ms/MB). 갱신마다 비행 패킷(1만 개 579 KB, 10만 개 4.2 MB)과 단면 패킷(`fast_adjust`, 16.6만 점 830 KB)이 같은 길로 온다. 재비행을 두 개만 겹치면 두 비행이 같은 때 시작해 같은 때 끝나고(GPU 공유), 두 응답이 IPC를 건너는 동안 GPU가 쉬었다. 세 번째 요청을 기다리게 하면 그 동안에도 GPU가 일한다.
- 메인 스레드는 병목이 아니다: 끌기 동안 프레임 간격 중앙값 18 ms, 50 ms 넘는 long task 0개.

바꾼 것: GPU 사본 2개 + 3D 기저 캐시(메모리 예산 안, `live::plan`), 사본마다 잠금(`live::Pool`), 재비행 동시 요청 2 → 3(`LIVE_DEPTH`). 결과(같은 release 앱 + 프로덕션 React, 그릴 궤적 300개, 1e-4 V 단위 5초 끌기 3회 중앙값, RTX 3070):

| 템플릿 | 이온 | 이전(위 표, 0.1 V 반올림) | 이전(1e-4 V, 3초 1회) | **이후** | 이후 네이티브 / 계산 / 전송 ms |
|---|---:|---:|---:|---:|---|
| 아인젤 | 10,000 | 47.4 | | **53.9** (52.0–54.6) | 11.5 / 5.8 / 4.8 |
| 아인젤 | 100,000 | 10.6 | | **12.6** (12.5–13.2) | 90.0 / 36.5 / 44.7 |
| 4중극 | 10,000 | 8.2 | 23.9 | **37.8** (36.5–38.5) | 31.9 / 18.7 / 10.3 |
| 4중극 | 100,000 | 5.4 | 4.7–5.0 | **5.6** (5.2–6.4) | 330 / 121 / 113 |

- 4중극 1만 개는 목표(30/s)를 넘는다. 바꾼 단계별로: 사본 2 + 캐시(동시 2개) 28.6–31.5/s, 동시 3개로 34.5–39.9/s. 캐시와 합성 순서가 달라 f32 결과가 아주 조금 바뀐다(평균 TOF 4.1733126765 → 4.1733126593 µs, 끝난 곳 개수 같음).
- **4중극 10만 개는 목표(10/s)에 못 미친다.** 앱 안에서 비행 하나의 GPU 계산(타임스탬프)이 약 120 ms라 그것만으로 10/s(100 ms)를 넘는다. 두 비행이 4조각씩 번갈아 GPU를 써서 둘 다 늦게 끝나고, 늦게 온 옛 결과는 버려진다(`claimResult`, 3초에 비행 24개 중 16–20개 반영). 큰 비행을 사본 하나로 몰아도 5.0–6.0/s로 같았다. 앱 쪽으로는 더 줄일 곳이 없다.
- **백엔드에 필요한 공개 API**(crates/를 고치지 않았으므로 여기 적는다): (1) 10만 개를 한 디스패치로 날리기(`MAX_IONS_PER_DISPATCH` 32,768을 키우거나 설정으로). 크레이트 사본에서 131,072로만 바꾼 벤치: 10만 개 계산 89 → 65 ms, 비행 129 → 95 ms. (2) 이온을 GPU에 상주시키거나 비행 준비(입자 검사·변환, 10만 개 약 16 ms, 한 스레드)를 병렬로 하는 API. (3) 제출과 결과 받기를 나눈 비동기 비행(사본을 늘리지 않고 겹치기). (4) `PreparedField`가 쓰는 GPU 메모리를 알려 주는 함수(지금은 `GridField::gpu_data`가 `pub(crate)`라 `live::plan`이 배열 크기로 어림한다).
- **남은 일**(둘 다 아래 "단면 패킷과 전압 슬라이더 범위"에서 했다): (1) 단면 패킷(830 KB)은 전압마다 바뀌지 않는 전극 플래그(166 KB)도 매번 싣고, 1만 개 갱신당 IPC의 약 60%가 단면이다. 전압별 해를 한 번 받아 UI에서 중첩하면 갱신당 IPC가 579 KB만 남는다. (2) 전압 슬라이더의 최소 범위 ±1000 V는 4중극(±2.5 V)처럼 낮은 전압 전극에서 거의 쓸 수 없다(숫자 칸 끌기도 1 V 단위).
- 원자료: `C:\Users\lsu10\simrep-gui-test\phase7b\after.json`(스크립트 `helper.js`(전압 단위 `__quantum`), `trace.js`(IPC 시각), `setup.js`), 벤치 소스는 저장소 밖(`/home/lsu10/orca/scratch-p7b/flybench`).

### 단면 패킷과 전압 슬라이더 범위 (2026-10-24)

**단면 패킷**: 전압을 바꿀 때의 `fast_adjust` 응답은 이제 SPA1(f32 전위 + u8 전극 플래그, 점당 5 B)이 아니라 SPQ1이다(`pa::potentials_packet`, `parsePotentialsPacket` in scene/pa.ts):

```text
"SPQ1" | u32 LE 헤더 길이 | 헤더 JSON(SPA1과 같음, 8바이트로 채움) | f64 offset | f64 step | u16 코드[nx·ny·nz]     전위 = offset + 코드 · step
```

- 전극 플래그는 전압으로 바뀌지 않으므로 보내지 않는다. 프런트는 그 배열의 SPA1(열기 / refine 결과)에서 받은 `grid.electrodes`를 그대로 쓴다(점 수가 다르면 오류).
- 전위는 그 배열의 [potentialMin, potentialMax]를 65,535등분한 u16이다. 오차는 범위의 1/131,070 이하로, 단면 색 지도, 등전위선(최대 약 60줄, 줄 간격의 1/2000 이하), 마우스 값 읽기(범위/1000 자리)가 구별하는 것보다 훨씬 작다. 완전히 평탄한 영역은 코드도 하나라 등전위선이 생기지 않는다. 전극 색과 범위(`electrodeMin/Max`, `potentialMin/Max`)는 헤더의 정확한 값이다.
- 크기: 4중극 템플릿(165,649점) 828 KB → **332 KB**, 아인젤 템플릿(8,241점) 41 KB → 17 KB. 3D 1,737만 점 배열이면 87 MB → 35 MB(프런트 복원은 한 번의 루프).
- 확인(실제 앱, `phase7c\qcheck.js`): 파일 전압에서 `fast_adjust` 결과를 같은 파일을 `open_pa`로 연 f32와 비교(템플릿 파일 전압은 모두 0 V라 차이 0, 플래그 일치). 전압 0.37 / 0.74 / 1.11 V에서 전극점(4중극 39,694개, 아인젤 655개)의 값과 그 전극 전압의 최대 차이 5.65e-6 V = 이론 한도(범위/131,070 = 5.65e-6 V) + f32 반올림.
- **실시간 갱신/s는 바뀌지 않았다.** 같은 조건(같은 시각, GPU 사용률 0%인 때, release 앱 + 프로덕션 React, 그릴 궤적 300개, 1e-4 V 단위 5초 끌기 3회 중앙값)에서 이 변경만 되돌린 빌드(**이전**)와 비교했다:

  | 템플릿 | 이온 | 이전 갱신/s | **이후 갱신/s** | 비행 왕복 ms(이전 / 이후) | 단면 패킷 | `fast_adjust` 중앙값 ms(이전 / 이후) |
  |---|---:|---:|---:|---|---:|---|
  | 아인젤 | 10,000 | 55.3–55.4 | **55.2–55.4** | 23.9–24.4 / 23.9–25.7 | 42 → 17 KB | 9.7–10.2 / 10.3–12.1 |
  | 아인젤 | 100,000 | 16.0–16.2 | **16.1–16.4** | 163 / 162–164 | 42 → 17 KB | 41–51 / 48–59 |

  | 4중극 | 10,000 | 43.6 | **43.4–43.6** | 60.9 / 61.4–62.2 | 829 → 332 KB | 28.4 / 29.0–31.7 |
  | 4중극 | 100,000 | 7.2 | **7.2** | 356 / 350–355 | 829 → 332 KB | 28.2 / 18.0–19.2 |

  (여러 번 잰 중앙값의 범위. GPU가 막 비었을 때 처음 잰 세트에서는 이후 아인젤이 50.3 / 14.7이었고, 바로 다시 잰 두 세트는 위와 같았다.)

  앞 표(53.9 / 12.6 / 37.8 / 5.6)보다 높은 것은 이 변경 때문이 아니라 측정 환경 때문이다. 이전 빌드도 같은 값이 나왔다. 처음 잴 때는 Windows CPU가 100%였다: WSL에서 띄운 Windows 프로그램의 headless `conhost.exe`가 클라이언트가 끝난 뒤에도 남아 하나에 약 2 코어씩 돌고 있었고(부모 `wsl.exe`가 살아 있어 `winproc.ps1 cleanup`의 고아 기준에 걸리지 않는다), 이때는 이전/이후 모두 4중극 1만 개 19–21/s였다. 다른 작업이 GPU를 쓰는 동안에도 값이 무너진다(4중극 1만 개 1/s). 측정 전에 `nvidia-smi.exe`로 GPU 사용률 0%를 확인하고 이전/이후를 번갈아 잔다.
- 왜 그대로인가: 단면 요청은 한 번에 하나라 단면 패킷은 비행 응답과 대역폭을 다투지 않았다. 비행 왕복(4중극 1만 개 61 ms = 네이티브 31 ms + 579 KB 응답)은 단면 패킷 크기와 무관하고, 동시 3개로 약 49/s가 위쪽 한계다(실제 5초에 비행 요청 221–228개). `fast_adjust` 자체도 1만 개에서는 바이트가 아니라 네이티브 작업(16.6만 점 중첩, 헤더 통계)과 비행이 CPU를 같이 쓰는 데 든다(29 ms로 같음). 10만 개에서는 단면이 빨라졌다(28 → 18–19 ms). 남는 것은 갱신당 IPC 60% 감소와 큰 배열의 단면 지연 감소다. 갱신/s를 올리려면 비행 쪽(네이티브 시간, 이온당 36 B 끝 상태 + 궤적 패킷)을 줄여야 한다.
- 원자료: `C:\Users\lsu10\simrep-gui-test\phase7c\ab2.json`(4개 설정, 이전/이후), `ab3.json`(아인젤, 이후 → 이전 → 이후), `ab-full.json`(이후 4개 설정, 이전 열은 다른 GPU 작업과 겹쳐 무효), `ab1.json`(conhost 부하 때), 스크립트 `helper.js`/`setup.js`/`trace.js`(phase7b와 같음), `qcheck.js`.

**전압 슬라이더 범위**(`VoltageControl`, components/SimulationSections.tsx):

- 범위 = 그 배열의 조절 전극들의 파일 전압과 현재 전압 중 가장 큰 |V|의 4배를 1·2·5×10ⁿ로 올린 값, 최소 ±1 V. 한 배열의 슬라이더는 모두 같은 범위다. 단계 = 범위/1000(슬라이더 2,000칸), 숫자 칸은 단계의 자릿수로 보여 주고 끌기·화살표도 그 단계다. 예: 4중극 템플릿(±2.5 V) ±10 V, 0.01 V 단계 / 아인젤 템플릿(80 V) ±500 V, 0.5 V / 리플렉트론(150 V) ±1000 V / 이온 트랩(9 V) ±50 V. 예전에는 최소 ±1000 V, 1 V 단계라 4중극에서 한 픽셀이 약 8 V였다.
- 템플릿은 파일에 모든 전극을 0 V로 저장하고 전압을 나중에 넣는다(위 확인). 그래서 파일 전압만으로는 범위를 정할 수 없고 현재 전압도 본다. 대신 사용자가 슬라이더·숫자 칸으로 범위 안에서 바꾸는 동안에는 범위를 그 편집을 시작할 때의 전압으로 고정한다(모듈 상태 `sizedFrom`: 배열별 기준 전압과 마지막 편집이 만든 `voltages` 객체). 그래서 끌기 도중이나 끝난 뒤에 범위가 다시 잡혀 엄지가 튀지 않는다. undo·되돌리기·템플릿·프로젝트 열기·refine 결과처럼 다른 곳에서 전압이 바뀌면 `voltages` 객체가 바뀌어 새 전압으로 다시 잰다. 문서 필드가 아니라 undo·프로젝트에 들어가지 않고, 편집 자체는 예전처럼 `setElectrodeVoltage`라 undo 병합도 그대로다.
- **넓히기**: 범위보다 큰 값을 숫자 칸에 입력(또는 끌기·화살표로 넘김)하면 고정을 풀고 새 전압으로 다시 잰다(4중극에서 12 V 입력 → ±50 V). 안내는 섹션 말머리와 슬라이더·눈금의 툴팁(`volts.widenHint`)에 있다.
- 확인(release 앱, 4중극 템플릿, CDP 실제 입력): 슬라이더 3개 모두 min/max −10/10, step 0.01, 숫자 칸 `2.50`. #1 칸에 `12` 입력 + Enter → 세 슬라이더 모두 −50/50, step 0.05. 스크린샷(en/ko × light/dark, 창 전체와 섹션만 2배): `C:\Users\lsu10\simrep-screenshots\phase7c\`(`01-default-*`, `02-widened-*`).

### 제출/수거 파이프라인과 작은 비행 패킷 (2026-10-24)

비행 크레이트의 새 API(`submit`/`collect`, 기본 131,072 이온 디스패치, `gpu_memory_bytes`, 병렬 이온 변환)로 바꾼 것:

1. **파이프라인**(`live.rs`): GPU 사본 2개를 잠그고 막힌 채 날던 것을, 장 하나에 잠금은 제출까지만 두고 `PendingFlight::collect`는 잠금 밖에서 하도록 바꿨다(위 "배열별 상주 장"). 사본이 하나라 GPU 메모리가 절반쯤이다. 오래된 슬라이더 값은 프런트가 이미 합친다(위 "흐름"): 요청은 보낼 때의 최신 전압만 싣는다.
2. **디스패치 한도**: 크레이트 기본값(장치 한도로 자른 131,072)을 그대로 쓴다. 앱 비행은 전기장만 쓰므로 4,096 물리 한도에 걸리지 않고, 10만 개가 한 디스패치다(헤더 `dispatchIons` = 131072 확인). 100만 개를 한 디스패치로 키우면 4중극에서 한 디스패치가 약 1 s라 느린 GPU에서 Windows TDR(2 s)에 걸릴 수 있어 키우지 않았다.
3. **`live::plan`**: 셀 수 어림 대신 올린 장의 `gpu_memory_bytes()`로 캐시 여부를 정한다(위).
4. **패킷**: STR2(이온당 40 B: 모든 이온의 오프셋, u32 끝난 곳, 속도) → STR3(이온당 21 B: 그린 이온의 번호·오프셋만, 끝난 곳 u8, 속도는 내보낼 때 `flight_velocities`로). 크기: 아인젤 1만 719 → 532 KB(궤적 점 2.6만 개가 대부분), 10만 4.32 → 2.42 MB, 4중극 1만 579 → 392 KB, 10만 4.18 → 2.28 MB.

측정(release 앱 + 프로덕션 React, RTX 3070, 1440×900, 그릴 궤적 300개, 1e-4 V 단위 5초 끌기 3회의 중앙값을 한 실행으로, 이전/이후 실행을 번갈아 이전 3회·이후 3회). **이전**은 이 변경 직전의 앱을 같은 새 크레이트로 빌드한 것이다(그래서 이미 131,072 디스패치와 병렬 변환을 쓴다. 위 표의 7.2/s보다 빠르다). 매 끌기 전에 GPU 사용률 5% 미만을 기다렸다(이전 3회차·이후 2–3회차와 아래 동시 4개 짝은 CPU 부하 40% 미만도). 이 PC는 측정 내내 사용자 터미널(Orca의 `wsl.exe` 셸)에 붙은 `conhost.exe` 2개가 각각 약 4 코어를 쓰고 있었고(다른 작업의 터미널이라 끄지 않았다), requestAnimationFrame도 실행마다 32–60 Hz로 달라 아인젤 1만 개는 슬라이더 이벤트 수에 묶인다(이후는 이벤트의 99%를 반영):

| 템플릿 | 이온 | 이전 갱신/s(실행별) | **이후 갱신/s**(실행별) | 왕복 ms 이전 → 이후 | 네이티브 / 계산 / 호스트 전송 ms(이후) |
|---|---:|---|---|---|---|
| 아인젤 | 10,000 | 39.1 (28.8, 54.6, 39.1) | **50.3** (50.3, 32.0, 53.6) | 52 → 28 | 11.6 / 6.0 / 3.9 |
| 아인젤 | 100,000 | 13.5 (13.5, 13.1, 14.9) | **17.7** (17.7, 16.1, 19.7) | 186 → 144 | 70.7 / 32.9 / 18.8 |
| 4중극 | 10,000 | 30.9 (35.3, 27.0, 30.9) | **39.5** (39.5, 37.7, 41.6) | 71 → 66 | 30.4 / 19.4 / 3.2 |
| 4중극 | 100,000 | 7.6 (7.6, 7.4, 8.4) | **8.6** (8.9, 8.6, 8.6) | 325 → 309 | 262 / 104 / 16.1 |

- **결과는 같다**: 실행마다 고정 전압(아인젤 #2 = 75.4321 V, 4중극 #1 = 2.71828 V)으로 한 번 날려 비교했다. 네 설정 모두 끝난 곳별 개수, 평균 TOF(헤더 f64와 f32 끝점의 합), 평균 에너지가 이전과 비트까지 같다(예: 4중극 10만 +x 83,806 / 전극 3 16,194, 평균 TOF 4.169646124657704 µs).
- **4중극 10만 개는 GPU에 묶인다**: 비행 하나의 계산이 104 ms(GPU 타임스탬프)라 위쪽 한계가 약 9.6/s이고, 이후 8.6–8.9/s는 GPU가 약 90% 바쁜 상태다. 네이티브 시간(262 ms)은 대부분 앞 비행 뒤에서 GPU 큐를 기다린 시간이다. 10/s를 넘기려면 비행 하나의 GPU 시간을 줄여야 한다(크레이트 쪽).
- **호스트 전송**은 이제 `Timings`의 setup + upload + readback이다(이전 열의 85 ms(10만)는 GPU 왕복에서 계산을 뺀 값이라 잠금·큐 대기를 포함했다).
- **동시 요청 4개**(`LIVE_DEPTH` = 4, 같은 이후 앱, 3개와 바로 이어 잰 짝): 아인젤 10만 22.2 vs 18.9/s로 빨라지지만 GPU에 묶인 4중극 10만은 8.7 vs 8.5/s로 그대로이고 왕복만 312 → 413 ms로 늘어 3개로 두었다(4중극 1만 짝은 rAF가 32 Hz로 떨어진 실행이라 비교 불가).
- 원자료: `C:\Users\lsu10\simrep-gui-test\livepipe\results\`(`before-2.json`, `before-3.json`, `after-1..3.json`, `d3-1.json`, `d4-1.json`. 이전 1회차는 정리 단계 오류로 JSON이 없고 중앙값만 남았다), 스크립트 `helper.js`(`check`: 고정 전압 비교), `setup.js`, `drag.js`, 구동기 `measure.py`.

## 결과 탐색 (결과 패널)

비행 결과가 생기면 3D 뷰 아래에 결과 패널이 나타난다(`src/results/`). 위쪽 가장자리를 끌어 높이를 바꾸고(더블클릭하면 기본 `--results-h`), 탭을 다시 누르거나 오른쪽 화살표로 접는다. 상태 표시줄의 **결과** 버튼도 같은 일을 한다. 열림·높이·탭은 localStorage(`simrep.results`)에 남는다. 보여 주는 배열은 선택한 배열(비행 결과가 있으면), 아니면 처음 비행한 배열이다.

| 탭 | 내용 |
|---|---|
| 히스토그램 | TOF / 운동 에너지 / x / y / z / r. 이온 필터(도달한 이온 전체, 모든 이온, 끝난 곳 하나), 구간 수(자동 = Rice 규칙, 10만 개에 93), 로그 축. 가로로 끌면 그 범위로 확대해 다시 나누고 통계도 보이는 범위로 낸다(더블클릭으로 해제). 오른쪽에 FWHM(가장 높은 봉우리, 반값 지점은 구간 중심 사이 보간)과 TOF면 질량 분해능 m/Δm = t/(2·FWHM), 이온 수·평균·표준편차·최소·최대·정점. 선택한 이온의 값은 세로선으로 |
| 검출면 지도 | 끝난 곳 하나(기본: 가장 많이 도달한 곳)의 도달 위치. 평면은 자동(±x 면 → z–y, ±y → x–z, ±z → x–y, 전극·반지름 → x–r) 또는 직접. **밀도**(mm로 정사각형인 칸의 2D 히스토그램, viridis, 로그 가능) / **점**(TOF나 에너지로 색칠). 축 비율 1:1. 점을 누르면 그 이온을 고른다. 중심, RMS, 폭 |
| 이온 | 선택한 이온의 x, y, z, \|v\|, 운동 에너지, 전위를 시간 또는 x에 대해 작은 그래프 여러 개로. 번호 입력·이전/다음. Rust `ion_trace`가 그 이온 하나를 CPU f64로 다시 날려 모든 RK 단계(최대 5,000개로 솎음)의 시간·위치·속도·에너지·전위를 준다. 실시간 모드에서는 GPU에 올린 장의 CPU 쪽 기저(그린 궤적과 같은 장)를, 정밀 모드나 GPU 준비 전에는 fast adjust한 배열을 쓴다. 비행할 때마다 다시 계산한다(한 번에 하나, 최신 우선) |
| 표 | 모든 이온의 끝 상태(#, 끝난 곳, TOF, 에너지, x, y, z, r). 보이는 행만 그리는 가상 스크롤(10만 행), 열 머리로 정렬, 끝난 곳 필터, ↑↓/PageUp/PageDown/Home/End로 선택 이동 |
| 비교 | **이 결과 보관**으로 지금 결과를 남기고(최대 5개, 프로젝트에는 저장 안 함) 전압·소스를 바꾼 뒤 히스토그램을 겹쳐 본다. 결과마다 전압·이온 수·정밀도, μ·σ·FWHM·R, 지금 결과와의 Δμ. 정규화(비율) / 로그 / 확대 |

- **연결된 선택**: 3D에서 궤적이나 끝점 클릭, 검출면 지도의 점, 표의 행, 이온 번호 입력이 모두 같은 선택(`useResults.ion`)을 바꾼다. 3D는 그 이온의 경로(다시 계산한 시계열, 없으면 그린 궤적)를 화면 공간 두께 3 px 리본과 끝점 고리로 다른 것들 위에 그리고(`trajectoryShader`의 `vsThick`/`vsRing`, 색은 `--vp-highlight`), 나머지 궤적은 흐리게 한다. 히스토그램은 세로선, 지도는 고리, 표는 행 강조와 스크롤로 따라간다. 3D 피킹은 CPU에서 모든 끝점과 그린 궤적의 선분을 화면에 투영해 가장 가까운 것을 찾는다(클릭할 때만).
- **내보내기**(패널 머리의 **내보내기**): 모든 이온의 끝 상태 CSV(`ion,surface,x_mm,…,vz_mm_per_us`, f64를 재파싱할 때 같은 수가 되는 최단 문자열로), 선택한 이온의 시계열 CSV(f64 전체 정밀도), 지금 탭의 그래프 PNG(2배 해상도)·SVG(작은 그래프 여러 개는 화면 배치 그대로 한 장), 3D 뷰 PNG(뷰포트 배경 그라디언트 위에 합성). 데스크톱은 네이티브 저장 대화상자(`dialog:allow-save`, 기본 폴더는 프로젝트 폴더 → 배열 폴더) 뒤 `write_bytes`가 IPC 본문 바이트를 그대로 받아 쓴다(경로는 percent-encode한 `path` 헤더). 브라우저는 다운로드로 대신한다. SVG 색은 `#rrggbb` + `*-opacity`로 바꿔 써서 Inkscape 같은 SVG 1.1 도구에서도 열린다.
- **그래프 엔진**: 의존성을 더하지 않고 `results/plot/`에 직접 만들었다(약 780줄). 필요한 그림이 히스토그램·계단 겹치기·선·래스터(밀도/점)뿐이고, 하나의 모델을 Canvas 2D(화면, PNG)와 SVG(벡터 내보내기)에 같은 코드로 그리며, 색·글꼴은 CSS 토큰(`--plot-*`, viridis는 `--vp-field-*`)에서 읽어 테마를 따른다. uPlot(약 50 kB)은 빠르지만 히스토그램·히트맵·SVG 출력이 없어 결국 같은 양을 더 써야 했고, Plotly/ECharts급은 수백 kB~MB다. 번들: 결과 패널 청크 53.0 kB(gzip 18.7 kB) + CSS 13 kB(gzip 2.8 kB), 첫 비행 결과가 생길 때 `React.lazy`로 불러온다. 같이 React를 따로 청크로 나눠(`vite.config.ts` `codeSplitting`) 앱 청크 290 kB / React 219 kB가 됐다(500 kB 경고 없음)(이전 단일 청크 488 kB).
- **측정**(2026-10-23, debug 앱 + 프로덕션 React, RTX 3070, 1440×900): 실시간 재비행 결과가 스토어에 들어온 순간부터 그 탭의 그래프가 다시 그려질 때까지(탭마다 12회, 중앙값 / 최대 ms).

| 경우 | 이온 | 히스토그램 | 검출면 지도(밀도) | 표 | 비교(3개 겹침) | 이온(시계열 왕복 포함) |
|---|---:|---:|---:|---:|---:|---:|
| 아인젤 템플릿 201×41 | 100,000 | 7.7 / 11.6 | 8.6 / 10.7 | 6.9 / 8.5 | 7.3 / 8.3 | 56 / 63 |
| 실제 `einzel.pa0` 91×20, `einzel_100000.ion`, E2 = 110 V | 100,000 | 7.1 / 11.6 | 8.2 / 9.8 | 6.5 / 8.1 | 7.2 / 8.4 | 68 / 288 |
| 같은 배열, `einzel_10000.ion` | 10,000 | 2.2 / 3.7 | 2.6 / 3.2 | 2.1 / 3.0 | 2.1 / 3.0 | 5.5 / 7.8 |

  분석(필터·구간 나누기·통계) 자체는 10만 개에 1–2 ms, 표를 TOF로 정렬하면 13 ms. 이온 탭의 10만 개 시간은 대부분 같은 순간에 도착한 4 MB 비행 패킷 처리와 겹친 `ion_trace` 왕복이다(따로 부르면 1.6 ms, Rust 쪽 0.45 ms). 끝 상태 CSV(10만 행, 8.0 MB)는 만드는 데 134 ms, 쓰는 데 157 ms.
- 스크린샷: `C:\Users\lsu10\simrep-screenshots\phase5-results\`(en/ko × dark/light, 탭별, 실제 einzel 10만 개, 내보내기 메뉴). 내보낸 파일 예: `C:\Users\lsu10\simrep-gui-test\phase5\export-*`.

## 5분 따라하기 (Tour)

`components/Tour.tsx`, 상태는 `state/tour.ts`. 실제 앱에서 사용자가 직접 하는 다섯 단계이고, 화면을 막지 않는 안내 카드가 그 단계에 필요한 컨트롤(`data-tour` 속성) 옆에 붙어 고리로 강조한다.

| 단계 | 대상 (`data-tour`) | 끝난 것으로 보는 조건 |
|---|---|---|
| 1. 아인젤 템플릿 | `template-einzel`(시작 화면이 아니면 `home`) | 아인젤 배열이 비행 결과를 가짐 → 다음 단계로 자동 이동 |
| 2. 집속 전압 | `voltage-2` | #2 전압이 단계 시작 때와 다름(제안값 60 V: 초점이 출구 쪽으로. 100 V를 넘으면 100 eV 이온이 되돌아감) |
| 3. 비행 | `fly` | 단계 시작 뒤 새 비행 결과 |
| 4. TOF 히스토그램 | `results-tab-hist` → `results` | 결과 패널이 열려 있고 히스토그램 탭, 양이 TOF |
| 5. CSV 내보내기 | `results-export` | 단계 시작 뒤 `.csv` 내보내기 로그(`log.exported`) → 완료 카드로 자동 이동 |

- 첫 실행(데스크톱, `simrep.tour` 없음)에 시작 화면 오른쪽에 제안 카드가 뜬다(다른 화면이면 오른쪽 아래). **나중에**/**시작**/닫기 모두 `seen`을 저장한다. 도움말(F1)의 **5분 따라하기** 버튼과 시작 화면의 같은 버튼으로 언제든 다시 시작한다. 건너뛰기·이전·다음은 언제든 된다.
- 카드 위치: 대상의 오른쪽 → 왼쪽 → 아래 → 위 중 창에 맞는 첫 자리(내보내기 단계는 메뉴가 아래로 열리므로 위를 먼저). 대상이 패널 밖에 있으면 그 패널만 스크롤해 가운데로 가져온다(`scrollIntoView`는 `overflow: hidden`인 앱 루트까지 밀어 창 전체가 밀리므로 쓰지 않는다). 작업 중(`busy`)이거나 도움말이 열려 있으면 숨는다.
- 확인(2026-10-24, debug 앱, CDP 실제 클릭): 제안 → 아인젤 카드 → #2에 60 입력 → 다시 비행 → 히스토그램 탭 → 내보내기 → 모든 이온의 끝 상태(CSV 41줄 = 머리 + 이온 40) → 완료. 스크린샷: `C:\Users\lsu10\simrep-screenshots\phase6-release\`(`01`–`05`, en/ko × dark/light).

## 시작 화면과 템플릿

- 앱은 시작 화면(`StartScreen.tsx`)으로 열린다. 예전의 내장 아인젤 원통 예제는 장면에 넣지 않는다(템플릿이 그 역할을 한다).
- 템플릿 4종(아인젤 렌즈, 정전 4중극, 리플렉트론 TOF 거울, 이온 트랩): Rust `create_template`이 작은 `.pa#`을 `Documents\SimRep\<종류>[-n]\`에 쓰고, UI가 열고 → `auto` refine(작은 배열이라 CPU, 수십~수백 ms) → 템플릿 전압·이온 소스·단면(`scene/templates.ts`)을 넣고 → 비행한다. 아인젤은 버튼부터 궤적까지 약 0.2~0.3 s, 4중극(16.6만 점)은 약 1.3 s.
- `SIMION 파일 열기…`는 새 제목 없는 프로젝트로 파일을 연다. 창에 끌어다 놓은 파일은 지금 프로젝트에 더해진다(`.simrep`은 프로젝트로 열린다).
- 브라우저 미리보기에서는 템플릿과 프로젝트 저장/열기가 꺼진다(네이티브 코어 필요).

## 장면 트리의 전극

- **전극** 그룹은 열린 PA마다 그 전극을 나열한다: 번호 칩(전위 색 점), 이름(더블클릭/F2로 바꾸기), 전압, 보이기 토글. 선택하면 3D 표면과 단면에서 그 전극의 격자점이 강조되고, 속성 패널에 이름·표시·전압 슬라이더가 나온다.
- 격자점마다 전극 라벨(u16)이 있다(`scene/electrodes.ts`): fast adjust `.pa0`은 Rust `electrode_labels`(각 해의 0.5·peak 이상인 점), `.pa#`은 저장된 번호, 그 밖의 배열은 같은 전위끼리 묶는다. 표면 사각형에 라벨이 실려(14 float/사각형) 셰이더가 강조/숨김을 처리하고(숨김은 라벨 비트 1024개), 단면 텍스처의 g 채널은 0(숨김)/1/2(선택)이다.
- 이름과 숨김은 라벨(= 전극 번호) 기준으로 저장되므로 `.pa#`을 refine해 `.pa0`이 되어도 유지된다.

## 프로젝트 (.simrep)

- 하나의 JSON 파일(`state/project.ts`): 배열/메시 경로(프로젝트 파일 기준 상대 경로 + 절대 경로 예비), 전극 이름·전압·숨김, 이온 소스(.ion/.fly2 경로 포함), 단면, 표시 옵션, 카메라·투영·격자, 마지막 비행 요약. 배열 데이터는 복사하지 않는다. 열 때는 상대 경로 → 절대 경로 순으로 찾고, 비행 결과가 있던 배열은 다시 비행한다. 못 찾은 파일은 토스트로 알린다.
- 저장하지 않은 변경이 있으면 상단 바 프로젝트 이름 옆 점과 창 제목의 `●`로 표시한다. 새 프로젝트/열기 전에 버릴지 묻는다(`dialog:allow-ask`).
- 자동 저장: 변경 후 1.5 s 뒤 `%LOCALAPPDATA%\dev.simrep.app\recovery.simrep`에 쓴다(저장하면 지운다). 다음 실행 때 시작 화면이 복원을 제안한다.
- 최근 프로젝트는 localStorage(`simrep.recent`, 8개).

## Undo / redo

- `state/history.ts`가 스토어를 지켜보며 문서 필드(이름, 표시, 전압, 전극 이름/숨김, 이온 소스, 단면, 표시 옵션, 단위, 출력 폴더)와 객체 추가/제거를 자동으로 기록한다. 계산 결과(격자, 표면, refine, 비행)는 되돌리지 않고, undo는 스냅샷의 문서 필드를 지금 객체에 합친다.
- 같은 대상의 연속 편집은 포인터를 누르고 있는 동안(슬라이더 드래그, live 재비행 포함) 또는 1 s 안의 키 입력이면 한 단계로 합친다.
- refine 결과 반영, 템플릿 설정, 프로젝트 열기는 `quietly`로 기록하지 않는다. 전압/소스 변경에 따른 fast adjust·재비행은 스토어 구독(`simulation.ts`)이 처리하므로 undo/redo·프로젝트 열기에서도 장과 궤적이 따라온다.
- 지운 배열의 네이티브 세션은 어떤 undo 단계도 그 객체를 참조하지 않을 때 닫는다. 그래서 제거를 undo하면 파일을 다시 읽지 않는다.

## Windows 앱을 스크립트로 조작하기 (CDP)

WebView2에 원격 디버깅 포트를 열면 Chrome DevTools Protocol 클라이언트(Windows 쪽 Node 등)로 실제 앱을 조작하고 화면을 찍을 수 있다.

```bash
cd /mnt/c && WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333 \
  WSLENV=WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS/w \
  /mnt/c/Users/<you>/simrep-app-target/debug/simrep-app.exe < /dev/null
```

- `http://127.0.0.1:9333/json/list`의 `webSocketDebuggerUrl`에 붙어 `Runtime.evaluate`(awaitPromise)로 `await import('/src/state/store.ts')`, `await import('/src/scene/importFiles.ts')` 같은 dev 모듈을 불러 쓴다(debug 빌드는 Vite dev 서버를 띄운다). 예: `importPaths(['C:\\...\\einzel.pa#'])`.
- 화면은 `Page.captureScreenshot`. 첫 연결까지 수십 초 걸릴 수 있다.
- Vite가 HMR로 바꾼 모듈은 앱이 `?t=…`를 붙여 불러오므로, `import('/src/state/store.ts')`가 앱과 다른 인스턴스를 줄 수 있다. `performance.getEntriesByType('resource')`에서 앱이 처음 불러온 URL을 찾아 import한다. 특히 `scene/simulation.ts`를 다른 인스턴스로 불러오면 스토어 구독이 하나 더 생겨 단면·재비행 요청이 두 배가 된다. 측정 전에는 Vite를 새로 띄워 `?t=` 모듈이 없게 한다.
- 프로덕션 React로 재려면 `NODE_ENV=production npx vite`로 띄운다(dev 모듈 import는 그대로 된다). dev React는 슬라이더 끌기에서 렌더 비용이 몇 배라 측정을 왜곡한다.
- 키·마우스는 `Input.dispatchKeyEvent`/`Input.dispatchMouseEvent`로 보낸다(예: `Ctrl+Z`는 `modifiers: 2`). 네이티브 파일 대화상자는 CDP로 조작할 수 없다. 저장하지 않은 프로젝트에서 `newFromTemplate`/`newFromFiles`를 부르면 버릴지 묻는 네이티브 대화상자가 떠서 스크립트가 멈추므로, 먼저 `useApp.setState({ dirty: false })`를 한다. 내보내기는 대화상자 대신 `results/exporters.ts`의 CSV/그래프 함수 결과를 `scene/backend.ts`의 `writeBytes(path, bytes)`로 바로 쓴다.
- 끝나면 `taskkill.exe /IM simrep-app.exe /T /F`로 프로세스 트리를 정리한다.

## WebGPU가 없을 때

뷰포트 자리에 원인과 해결책(WebView2/드라이버 업데이트, Chrome/Edge 113+)을 담은 카드가 뜨고, 나머지 UI는 그대로 동작한다.
WSL의 헤드리스 Chromium으로 화면을 찍을 때는 SwiftShader를 쓴다:
`--enable-unsafe-webgpu --enable-features=Vulkan --use-vulkan=swiftshader --use-webgpu-adapter=swiftshader --use-angle=swiftshader --enable-unsafe-swiftshader`.
`--use-angle=swiftshader`가 빠지면 시작 1초쯤 뒤 "Device was destroyed"로 장치가 사라진다(앱 문제가 아니라 헤드리스 환경 문제).
