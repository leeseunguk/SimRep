# 보고서 그림 (F01–F22)

모든 그림은 실제 앱 화면이거나 실제 실행·측정값이다. 생성 이미지, 개념도, 손으로 만든 수치는 없다.
그림마다 무엇을 보여 주는지, 출처 명령·파일·표, 대조한 수치는 `../figure-ledger.json`에 있다.

| 종류 | 그림 | 만드는 법 |
|---|---|---|
| 앱 캡처 (`app-capture`) | F01 F03 F07 F09 F18 F21 F22 | 현재 main 소스로 빌드한 Windows 앱을 CDP로 조작해 새로 찍음 |
| CLI 결과 (`cli-result`) | F04 F05 F08 F11 F12 F14 F15 F16 F19 F20 | `target/release/simrep`(와 FLY2 생성기 `target/release/examples/particles`)를 실제로 실행 |
| 저장 데이터 (`stored-data`) | F02 F06 F10 F13 F17 | bench/·docs/ 표 또는 보관된 측정 CSV를 그대로 그림 |

`%USERPROFILE%\simrep-screenshots\`의 예전 스크린샷은 하나도 다시 쓰지 않았다. 그 뒤 앱 소스
(`SimulationSections.tsx`, `ResultsPanel.tsx`, i18n 등)가 바뀌어 현재 앱과 같다고 볼 수 없기 때문이다.

## 폴더

- `F01.png` … `F22.png`: 그래프는 300 dpi, 앱 캡처는 원래 해상도(창 1280×800 CSS를 배율 2로 찍은
  픽셀을 그대로 자름. F03 띠 1–3만 1280×440 CSS 창). 인쇄 폭 15.5 cm 기준으로 글자 크기를 정했다.
- `scripts/`: 다시 만드는 코드.
  - `_style.py`: 공통 서식(맑은 고딕 `/mnt/c/Windows/Fonts/malgun.ttf`, DejaVu Sans 대체 글꼴, 색, 15.5 cm 폭).
  - `F??_*.py`: 그림 하나당 하나. `--extract …` 인자가 있는 스크립트는 원본 PA에서 `data/`를 다시 만든다.
  - `cli_runs.sh`: CLI 결과 그림의 실행 전부(약 2분). `bash docs/report/figures/scripts/cli_runs.sh`
    로 `target/report-figs/`에 다시 풀고 `data/`와 PNG를 다시 만든다. 2026-10-25에 이 스크립트로 처음부터
    다시 돌려 `data/`의 값이 첫 실행과 같음을 확인했다(F12 로그의 단일 실행 시간만 다름).
  - `pa_prep.py`: 예제 PA **작업 사본**에만 쓰는 준비(비갱신 비트 해제, max_voltage 올리기, 예제 Lua가 하는
    라벨→물리량 변환). 원본 예제 파일은 바꾸지 않는다.
  - `cdp.mjs`, `app/*.js`, `crop.py`: 앱 캡처용 CDP 구동기, 앱 안에서 실행한 스크립트, 픽셀 자르기·이어 붙이기.
- `data/`: 그림에 실제로 쓴 원자료(CSV, JSON, NPZ). 파일 이름 앞의 `F??`가 그림 ID다.

Python 환경: `uv venv ~/.cache/simrep-figvenv && uv pip install --python ~/.cache/simrep-figvenv/bin/python
matplotlib numpy scipy pillow` (확인한 버전: matplotlib 3.11.2, numpy 2.5.4, scipy 1.18.1, Pillow 13.0.0).
스크립트는 `cd docs/report/figures/scripts && ~/.cache/simrep-figvenv/bin/python F06_cutcell_order.py`처럼 실행한다.

## 앱 캡처 다시 찍기

`docs/dev/frontend.md`의 "Windows 앱을 스크립트로 조작하기 (CDP)" 절과 같은 방식이다.

```bash
# 1) 빌드 (STEP 가져오기 때문에 cad 기능 필요)
CARGO_TARGET_DIR='%USERPROFILE%\simrep-app-target-fig' \
OCCT_INCLUDE_DIR='%USERPROFILE%\occt\opencascade-7.7.0\inc' OCCT_LIB_DIR='%USERPROFILE%\occt\opencascade-7.7.0\win64\vc14\lib' \
WSLENV=CARGO_TARGET_DIR:OCCT_INCLUDE_DIR:OCCT_LIB_DIR \
  /mnt/c/Users/<user>/.cargo/bin/cargo.exe build --manifest-path "$(wslpath -w app/src-tauri/Cargo.toml)" --features cad < /dev/null
# 2) Vite (WSL)과 앱 (원격 디버깅 포트 9333)
(cd app && node_modules/.bin/vite) &
cd /mnt/c && WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333 WSLENV=WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS/w \
  /mnt/c/Users/<user>/simrep-app-target-fig/debug/simrep-app.exe < /dev/null &
# 3) cdp.mjs와 app/*.js를 %USERPROFILE%\simrep-gui-test\report-figs\ 로 복사한 뒤 Windows node로 실행
cd /mnt/c/Users/<user>/simrep-gui-test/report-figs
METRICS=1280x800 WSLENV=METRICS '/mnt/c/Program Files/nodejs/node.exe' cdp.mjs eval einzel.js '{"count":1000,"draw":300}'
METRICS=1280x800 WSLENV=METRICS '/mnt/c/Program Files/nodejs/node.exe' cdp.mjs shot 'C:\...\F01-raw.png'
# 끝나면: taskkill.exe /IM simrep-app.exe /T /F
```

그림별 순서(스크립트와 인자, 자른 영역)는 `../figure-ledger.json`의 `source`에 있다.
F09는 `cdp.mjs dragshot`으로 실제 마우스 끌기 도중에 찍었다. 앱 그림에서 계산과 무관한 표시 조작은
카메라 위치, 결과 패널 높이, 단면 숨김, F03 띠 3의 막대 전극 1 숨김뿐이다.

## 계획에서 바뀐 점

- F04: 계획은 앱 캡처였으나, 앱은 전극을 격자점 상자로만 그려 계단/정확한 표면의 차이가 보이지 않는다.
  같은 4중극을 `refine-geometry`(cut-cell)와 `refine`(계단)으로 푼 실제 출력 파일의 단면 그림(CLI 결과)으로 바꿨다.
- F05: CLI는 사이클별 잔차를 출력하지 않으므로 `--max-cycles k`로 k마다 따로 실행해 k회 뒤 잔차를 읽었다.
- F11, F15: CLI는 궤적 표본을 내보내지 않으므로 같은 비행을 시간 제한(F11) 또는 검출면 위치(F15)만 바꿔
  여러 번 실행한 끝점으로 그렸다.
- F12: 같은 폴더의 SIMION 솔레노이드 FINAL 행은 이 이온들과 다른 무작위 발사라(문서의 경고) 그리지 않았다.
- F02: 같은 정밀도 이상인 짝만, SIMION 시간은 실측만 썼다(외삽한 대형 빔 기준 제외). 행마다 근거는 `data/F02-speed.csv`.
- F17: be-bmap-import 병합(2026-10-25) 뒤 `bench/bfield-import-20261024.csv`로 만들었다. 이 CSV의 시간 열은
  contended로 표시되어 있어 그리지 않았다.
