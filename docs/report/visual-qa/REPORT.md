# SimRep desktop app - visual QA report

Verdict: **GOOD after round 4.** The independent gate review (st_01a17256) found a real 1280x800 regression that the three review rounds and the scorer's default PASS had missed: with the log drawer open, the results panel overlapped the activity log by 52px. It is fixed and measured (overlap 0 at both sizes), and log-drawer was re-scored item by item from the new captures. The round-4 fixes are verified by the author (captures, geometry probe, audits); no fresh independent review has run on them. Most after PASS cells are scorer defaults, not cell-level evidence; see section 5.

Scope: the SimRep Tauri 2 + React + WebGPU app (Windows, WebView2), Korean UI, 42 surfaces x light/dark x 1280x800 CSS (2x) and 1920x1080 CSS (1x) = 168 captures per phase. Procedure: the `visual-qa` skill (Before/after mode; checklist A1-J2; two independent read-only reviewers per round; budget 3 rounds for a product surface). Only frontend files under `app/src` changed; no Rust, physics or computed results changed (`app/src-tauri` and `crates/` untouched; before/after manifests record the same voltages and flights, e.g. `results-*` at 40 V, compare 40 V kept vs 60 V current).

## 1. Result in numbers

| | before | after |
|---|---:|---:|
| Checklist cells FAIL (42 surfaces x 69 items = 2,898) | 497 | 0 |
| Checklist cells PASS | 1644 | 2145 |
| … of which after PASS: explicitly evidenced per cell / contract (A12, B7) / scorer default | – | 73 / 82 / 1,990 |
| Justified deviations accepted by the lead (J) | – | 80 (I4 x 37, G4 x 42, H7 x 1 surface) |
| N/A (phone-/mac-only items, absent features) | – | 673 |
| DOM audit: visible text < 12px | 6472 | 0 |
| DOM audit: Hangul set in a monospace face | 228 | 0 |
| DOM audit: text below WCAG AA (approx.) | 3005 | 0 |
| DOM audit: interactive element without a usable name | 172 | 0 |
| DOM audit: pointer target < 24px | 1182 | 0 |

DOM audit = `scripts/audit.js`, 84 runs per phase (each surface, both themes, 1280x800). It aims the review; the verdict comes from the captures and the reviewers.

## 2. The lead's suspects

| Suspect | Finding |
|---|---|
| Very small text/icons in toolbar and side panels at 1280x800 | **Confirmed.** Body 13px, captions 10-11px (6,472 sub-12px text nodes), icons 12-14px, controls 20-24px. Fixed in the tokens: type 11/12/13/14/16/20 (Hangul captions >= 12px), icons 14/16/18, controls 24/28/32/36 (short windows 24/28/28/32). |
| Dense results panel, key numbers do not stand out | **Partly confirmed.** The headline figure (m/Δm) already led the side column; the density came from 10-11px labels at 3.4:1 and, after the type bump, from cramped narrow layouts. Fixed: readable stats, context on its own row when narrow, chart ticks spaced, small multiples 3+2 / 5-across, compare cards and map legend fit. |
| Expert jargon as labels (cut-cell, PA#, refine, f32/f64, T.Qual) | **Partly confirmed.** "REFINE"/"Refine"/"Cut-cell"/"Fast adjust:" were bare labels → "전기장 계산 (Refine)", "계산", "만들고 계산", "표면 맞춤 (cut-cell)", "빠른 전압 조절(fast adjust)", onboarding "계산된 전기장". f32/f64 are already secondary sub-labels with a plain explanation under the precision cards (kept). PA# appears only as file extensions. **T.Qual does not exist in the UI** (refuted). |
| Electrodes drawn only as grid boxes in 3D | **Verified, not changed.** The PA view draws the potential array's grid cells (that is the solver's data; the import wizard preview shows the smooth source geometry). Smooth electrode surfaces would be a new renderer feature, out of scope; both final reviewers agreed. |

## Briefs, required content, Layout lines
1. Start screen (start, start-recent, start-tour-offer, busy, tour-1)
   - Primary action: pick a template (template card); top-bar "열기" is the only filled button.
   - Priorities: 1 template gallery, 2 open SIMION file / open project, 3 recent projects (+ recovery offer).
   - Left out on purpose: feature marketing, stats, a second filled button.
   - Required content: 4 template cards (art, title, one-line description, meta line), "SIMION 파일 열기…",
     "프로젝트 열기…", drop hint with extensions, "5분 따라하기", recent list or its empty placeholder, recovery
     offer when present; top bar (home, undo, redo, open, save, export, language, theme incl. system, help); status bar.
   - Layout: start screen - centred gallery with a side column -> center + card-grid (https://github.com/changeroa/StyleGallery/blob/main/patterns/centering/center.md, https://github.com/changeroa/StyleGallery/blob/main/patterns/grid-repetition/card-grid.md); scroll owner: the start overlay; anti-patterns: content glued to the top-left of a wide window, cards stretched to full width.
2. Workspace (empty-workspace, template-*, props-*, electrode-selected, live-slider, toasts, error-unsupported-file, refine-progress, exact-*, tour-2..tour-done)
   - Primary action: "다시 비행" (fly) in the selected array's properties; with an empty scene: top-bar "열기".
   - Priorities: 1 the 3D view (section + trajectories), 2 electrode voltages (tree + properties sliders), 3 flight results (results panel, properties flight section).
   - Left out on purpose: decorative stat tiles, duplicate open buttons, accent stripes.
   - Required content: scene tree (electrodes: number chip, name, voltage; files: dims, show/hide, remove); viewport
     toolbar (front/side/top/iso, fit, projection, grid), potential legend, axes gizmo, mouse hints; properties
     (general, field computation status, electrode voltages + sliders + ticks, ion source, precision cards, draw count,
     advanced, fly, workbench, flight results, section view, array info); results panel; status bar (WebGPU, ion-flight
     device, runtime, objects, triangles, trajectories, units, results toggle, log + count).
   - Layout: workspace shell - top bar + scene / 3D+results / properties + status bar, each region scrolls on its own -> panel-layout (https://github.com/changeroa/StyleGallery/blob/main/patterns/viewport-shell/panel-layout.md); scroll owner: `aside > div[class*=body]` of the properties and scene panels, results table rows in `div[class*=viewport]`; anti-patterns: document scroll, side panels fixed while a 1920 window grows only the 3D view, nested same-direction scroll inside a panel.
   - Layout: 3D view overlays - legend, hints, gizmo layered over the canvas -> imposter (https://github.com/changeroa/StyleGallery/blob/main/patterns/overlay-exception/imposter.md); scroll owner: none; anti-patterns: an overlay taller than the view (cut-off legend), overlays covering one another on a short view.
3. Results panel (results-*, and the bottom of every flown workspace capture)
   - Primary action: none filled except "이 결과 보관" on the compare tab; export menu.
   - Priorities: 1 headline figure (mass resolution m/Δm) + plot, 2 stats (n, mean, σ, min, max, peak), 3 controls.
   - Required content: tabs 히스토그램/검출면 지도/이온/표/비교, precision + surface badges, array name, ion count,
     export menu (CSV end states, ion time series CSV, plot PNG/SVG, 3D PNG), collapse.
   - Layout: results header - tabs + context in one fixed-height row when the panel is at least 900px wide; narrower (1280 window) the context (badges, array name, ion count) takes a second row of its own under the tabs, deliberately, because every context item is required content -> tab-strip + cluster (https://github.com/changeroa/StyleGallery/blob/main/patterns/in-line-grouping/tab-strip.md, https://github.com/changeroa/StyleGallery/blob/main/patterns/in-line-grouping/cluster.md); scroll owner: tabs scroll horizontally only on overflow; table rows in `div[class*=viewport]`; anti-patterns: half-clipped badge text, required context hidden, a context row on a wide panel.
4. Import wizard (wizard-*, error-import-*)
   - Primary action: one per step - "다음", "만들고 계산", "완료".
   - Priorities: 1 the step content, 2 the step indicator, 3 the 3D preview.
   - Required content: file card (name, kind, badge), steps 부품/격자/계산, parts table (colour, name, number,
     voltage), boundary cards 표면 맞춤 (cut-cell)/격자 정렬, CAD surface cards (STEP), spacing + recommendation,
     domain, cost (grid, points, memory, disk), output (folder, name, files), solver, progress, done + fly; error:
     title, next step, technical cause.
   - Layout: import wizard - replaces the properties panel with a wider stepped panel -> panel-layout (https://github.com/changeroa/StyleGallery/blob/main/patterns/viewport-shell/panel-layout.md); scroll owner: wizard body; anti-patterns: footer actions scrolling away.
5. Help (shortcuts) dialog, export dialog, log drawer, toasts, tour coach card: one primary each ("5분 따라하기",
   "내보내기…", none, none, "다음"/"마치기"); always a close control; Escape closes the dialog.

6. Layout: help / export dialogs - one modal centred over a scrim -> imposter (https://github.com/changeroa/StyleGallery/blob/main/patterns/overlay-exception/imposter.md); scroll owner: the dialog body; anti-patterns: two modals stacked, page scrolling behind the scrim.
   Layout: log drawer - full-width drawer between workspace and status bar -> panel-layout (https://github.com/changeroa/StyleGallery/blob/main/patterns/viewport-shell/panel-layout.md); scroll owner: the log list `ol`; anti-patterns: the drawer pushing the 3D view below its minimum, the list scrolling the page.
   Layout: toasts - stack over the bottom of the scene panel -> overlay-stack (https://github.com/changeroa/StyleGallery/blob/main/patterns/overlay-exception/overlay-stack.md); scroll owner: none; anti-patterns: toasts over the 3D view, more than three at once.
   Layout: tour coach card - card anchored beside its target control -> imposter; scroll owner: none (the target's panel scrolls to bring it into view); anti-patterns: card covering its target, card off-screen.
   Layout: results side column (histogram / map / compare stats) - narrow column beside the plot -> main-with-rail (https://github.com/changeroa/StyleGallery/blob/main/patterns/split-sidebar/main-with-rail.md); scroll owner: `aside[class*=side]` scrolls only its own stats when they exceed the panel; anti-patterns: stats cut with no cue, the rail stealing the plot's height.

Not capturable through CDP (recorded, not reviewed): the native Windows dialogs (open/save/discard confirm), the
file-drop overlay (Tauri native drag events), the WebGPU-unavailable card. There is no settings window; preferences
are the top-bar language and theme controls (in every capture).


## 4. Round findings and how each was resolved

Round 1 = two read-only reviewers dispatched here plus four reviewer passes run by the lead on the same build; round 2 and round 3 = two fresh read-only reviewers each (Pass A: design system, behaviour, G-J; Pass B: A-F, before/after pairs, CJK).

| # | Finding (round 1) | Resolution | Evidence |
|---|---|---|---|
| 1 | Ion-tab charts: y ticks overlap at 1280; 4+1 grid at 1920 | Tick labels that would touch are skipped (grid lines stay); small multiples min 200px (`--plot-multiple-min-w`): 3+2 at 1280, 5 in one row at 1920 | `results-ion-*` pairs; `results/plot/chart.ts` |
| 2 | Compare: run cards cut, toolbar wrap shrinks plot | Short windows: results panel 408px (`--results-h`); run cards/side column tightened (side column fits both cards: 270/270 px at 1280, 265/265 px at 1920); toolbar on one row (R2-1) | `results-compare-*`; CDP measure (client/scroll height) |
| 3 | Detector map legend endpoints below the fold at 1280 | Narrow panel: toolbar hint stays on the toolbar row (ellipsis + tooltip), tighter body | `results-map-1280*` |
| 4 | Table headers overlap | Header cells wrap (`.thead` auto height) | `results-table-*` |
| 5 | Log drawer: legend/gizmo/hints disappear | Short 3D view keeps a compact legend (title + scale sized with `cqh`), the gizmo and the hints | `log-drawer-*` |
| 6 | Exact-surface badge missing at 1280 | Narrow results panel (<900px, container query) puts badges, array name and ion count on their own row | `exact-on-1280*` |
| 7 | Array name truncated / ion count gone at 1280 | Same as 6 | `template-*-1280*`, `results-*-1280*` |
| 8 | Wizard building: Cancel below the fold | Empty wizard footer removed while building/refining; short-window rhythm | `wizard-gem-running-1280*` |
| 9 | Sidecar label truncated | Label "표면 맞춤 보조 파일" | `wizard-gem-running-*` |
| 10 | Orphan "씁니다." in cut-cell hint | Rephrased, object and predicate bound with no-break spaces | `wizard-step-grid-*` |
| 11 | Disabled buttons ~2.8:1 (45% opacity) | Disabled = `--color-text-subtle` text (>=4.6:1), primary loses its fill, no hover; other disabled text via R2-2 | `start-*`, `wizard-*`; `ui/Button.module.css` |
| 12 | "지우기" (clear log) has no destructive cue | New `danger` button variant + trash icon | `log-drawer-*` |
| 13 | Accent stripe on selected scene-tree row | Stripe removed; selection = wash + accent chip | all workspace captures; `SceneTree.module.css` |
| 14 | "refine된" in onboarding/tour copy | "계산된 전기장", "전기장을 다시 계산하지 않아도" (ko) | `start*`, `tour-*` |
| 15 | Layout lines missing for dialogs, log, toasts, tour, results side scroller | Added (section 3) | this report |
| 16 | Camera fly-over under reduced motion | `animateTo` jumps when `prefers-reduced-motion: reduce` | `after/funccheck.json` camera: settled within 60 ms |
| 17 | Spinners at once | Every spinner fades in after `--dur-spinner-delay` (1 s); the status text is immediate (see R2-3) | CSS modules |
| 18 | Dialogs rise in but unmount with no exit | Entrance motion removed (help, export incl. its options pane, busy) | `Dialog.module.css`, `ExportDialog.module.css` |
| 19 | Discard confirmation is 3 sentences | 2 sentences (ko/en), Cancel guidance kept | `i18n` `project.discard.body` (native dialog, not capturable) |
| 20 | F1 stacks Help over Export; one Escape closes both | App shortcuts ignored while the export dialog is open | `after/funccheck.json` modal: help false, export closed, selection kept |
| 21 | No Cancel on the template busy overlay | **Justified, not changed**: template creation takes 0.2-1.3 s (einzel 229-248 ms in the logs; quadrupole ~1.3 s per docs/dev/frontend.md); with the 1 s spinner gate the overlay is gone before a Cancel could be used. The long refines (properties panel, import wizard) keep Cancel. | logs in captures |
| 22 | Resize handles pointer-only | Focusable (`tabIndex`), focus ring, arrows +-16 px (Shift +-64), Home = default results height | `after/funccheck.json` panel/results |
| – | Before/after voltages differed | Plan sets voltages explicitly; both sets re-captured with it (before = git HEAD frontend) | `surfaces.*.info.voltages` identical in both manifests |
| R2-1 | Compare toolbar still wraps "로그" at 1280 | Narrow results panel: body padding 8px, toolbar gaps 6px, denser segments inside that toolbar only; all five controls on one row (measured: rows at y 424-427) | `results-compare-1280*` |
| R2-2 | Disabled export choices / menu item low contrast | `--color-text-disabled` raised to >=4.5:1 (light #666a72, dark #868d98); export-dialog choices use it instead of 45% opacity | `results-export-menu-*`, `ExportDialog.module.css` |
| R2-3 | Other spinners (scene tree, ion trace, viewport) immediate | All spinners fade in after the `--dur-spinner-delay` token (1 s) | CSS modules |
| R2-4 | Export options pane fades in without exit | Removed | `ExportDialog.module.css` |
| R2-5 | Export success shown twice (dialog + toast) | Toast removed; in-place notice and log entry kept | `ExportDialog.tsx` |
| R2-6 | Escape on the results export menu also clears the selection | App ignores Escape while a `role="menu"` is open | `App.tsx` |
| R2-7 | Magic numbers (1 s delay, tick gap) | Tokens `--dur-spinner-delay`, `--plot-tick-gap` (read into the plot theme) | `tokens.css`, `chart.ts` |
| R2-8 | Orphaned "을" after "(.cuts.json)" | Word joiner after ")" plus no-break spaces in the tail | `wizard-step-grid-*` |
| R2-9 | Results header Layout line contradicts the two-row narrow capture | Layout line now documents the deliberate second context row below 900px panel width | section 3 |
| R2-10 | start-recent 1280: third recent entry's path/date below the fold | Start screen tighter on short windows (padding, gaps, recent rows) | `start-recent-1280*` |
| R2-11 | Selected file row: wash but no selection glyph | Selected row tints its leading icon (accent) | workspace captures |
| R3-1 | H6: export dialog not dismissible while an export runs (Escape, backdrop, close, Cancel disabled) | Always dismissible; while busy the footer button reads "닫기"; the export finishes in the background and reports to the log (no backend cancellation added) | `after/h6check.json`: busy seen, close/Cancel enabled, Escape closed the dialog mid-export, log `log.exportedFiles` (einzel-field.vti) |
| R3-2 | Korean strings showed literal particle alternatives ("을(를)", "은(는)", "이(가)", "(으)로") after inserted names | All 37 rewritten so no particle follows a variable (e.g. "열 수 없는 파일: {file}", "전극 {list}의 전압은 …", "{file} 파일의 전기장을 계산해 …"); `grep` for the patterns in ko.ts finds none | `i18n/ko.ts`; fresh captures (e.g. `wizard-gem-done`, `props-end`, `error-unsupported-file`) |
| R4-1 | **Gate review B1**: log drawer open at 1280x800 → results panel (408px, `flex: none`) overflowed the 536px column by 52px and covered the log header and first entry (both themes); 1920 unaffected | Results panel capped at `max-height: calc(100% - var(--viewport-min-h))` (`ResultsPanel.module.css`): with the log open it yields to the 3D view's 180px minimum (356px at 1280); log closed it keeps 408px (1280) / 360px (1920) | `after/logcheck.json`: overlap 0 at both sizes (results 228-584, drawer 584-768 at 1280; 504-864 / 864-1048 at 1920); `log-drawer-*` re-captured |
| R4-2 | Recovery prompt "자동 저장됐지만 저장되지 않았습니다" contradicts itself | "{name} 프로젝트의 자동 저장본({time})은 있지만 프로젝트 파일에는 저장되지 않았습니다. 복원할까요?" | `start-recent-*` re-captured |
| R4-3 | Jargon left in ko.ts:191, 276, 475 | "빠른 전압 조절(fast adjust) 배열"; refine chip "{file} 전기장 계산 중" (file first, so the truncated chip still shows which array; full text as tooltip); "전압을 바꾸면 계산된 전기장을 다시 합칩니다" | `props-end-*`, `refine-progress-*`, `electrode-selected-*` re-captured |
| R4-4 | Found while re-scoring log-drawer (G2, exit mirrors entry): log drawer, results panel and results export menu animated in on mount and vanished with no exit (same pattern as item 18) | Entry animations removed (`LogDrawer.module.css`, `ResultsPanel.module.css` x2) | `log-drawer-*`, `results-export-menu-*`, `results-compare-*` re-captured |
| R2-12 | I4 orbit/pan/zoom keyboard alternatives | **Accepted by the lead as out of scope** (new feature; presets 1-4, Fit, gizmo exist) | – |
| R2-13 | G4 transform transitions snap under reduced motion | **Accepted by the lead** (snapping is acceptable reduced-motion behaviour; fades/spinners keep running) | `after/motion.json` |

## 5. Per-surface, per-item verdicts (before → after)

### How the cells were scored (read this before the totals)

`scripts/score.py` turns recorded judgements into cells; it does not detect defects by itself.
- **before FAIL** cells are explicit: every one is listed with its reason in `GLOBAL_FAIL` / `surface_fails()` (from the before captures, the DOM audit and the source), and `scores.json` keeps the reason per cell. Any before cell not listed defaults to PASS.
- **after** cells: N/A by `applicable()`, J for the three lead-accepted deviations, contract PASS for A12/B7 (Layout lines and briefs written in section 3), and **explicit** where `EXPLICIT_AFTER` holds a per-cell verdict and evidence (73 cells, all from round 4). **Every other after cell (1,990) is a default PASS.** Its support is surface-level, not per cell: the round-2/3 reviewers (Pass B opened all 84 quad sheets for A-F; Pass A judged G-J from source, captures and the functional JSON), the DOM audit (which skips canvas/SVG text, disabled elements and the 3D legend, and does not size-check separators), and the functional checks. `scores.json` records the basis of every after cell (`after_basis`).
- So "FAIL 0 after" means: no reviewer, audit or re-score has a FAIL left on the final captures. It is not an independent proof. The gate review showed the limit: the 1280 log-drawer overlap (R4-1) sat under default PASS cells through three rounds.

### Explicit re-score from the round-4 captures

**log-drawer**, every applicable item, from `log-drawer-{1280x800,1920x1080}-{light,dark}-after.png`, `after/logcheck.json`, the after audits and the source (I4 and G4 stay J; 16 items are N/A). The export menu is open in this state in both phases: the plan runs `results-export-menu` just before `log-drawer` and does not close it, so the pair is like-for-like.

| Item | before | after | evidence (after) |
|---|---|---|---|
| A1 | PASS | PASS | panels on the app grid; nothing floats in an empty canvas |
| A2 | PASS | PASS | panels and drawer run edge to edge |
| A3 | PASS | PASS | symmetric panel insets; log lines are records, not prose |
| A6 | PASS | PASS | scrollers: log list in the drawer, results side column, properties body - one per region (after manifest `scrollers`) |
| A7 | PASS | PASS | drawer header, panel headers and status bar share edges |
| A8 | PASS | PASS | log header groups title + clear + close; results toolbar grouped |
| A9 | FAIL | PASS | no overlap: results 228-584 / drawer 584-768 at 1280 (3D view 180px), 504-864 / 864-1048 at 1920 (after/logcheck.json); 1280 before the fix overlapped by 52px |
| A10 | PASS | PASS | log list and results side column show a partial row and a scrollbar |
| A12 | N/A | PASS | Layout lines for the log drawer and results side scroller (section 3) |
| B1 | PASS | PASS | single prominent action in view ('이 결과 보관' on compare; histogram has none); top-bar Open is not filled |
| B2 | FAIL | PASS | '지우기' danger style + trash icon, not a default action |
| B3 | PASS | PASS | project title in the top bar, '활동 로그' names the drawer |
| B4 | PASS | PASS | 3D view and results lead; log is secondary below |
| B5 | PASS | PASS | accent on the active tab, primary button and switches only |
| B6 | PASS | PASS | weight/size/colour hierarchy holds in both themes; sans + mono only |
| B7 | N/A | PASS | matches the workspace brief (section 3) |
| C1 | FAIL | PASS | body 14px; audit `small` 0 in both themes |
| C2 | PASS | PASS | log lines are sentences; 'refine' stays as the SIMION term next to the '전기장 계산 (Refine)' label |
| C3 | PASS | PASS | outcome verbs ('지우기', '이 결과 보관'), noun tabs |
| C4 | PASS | PASS | long log path fits at 1280; truncated panel text ends in an ellipsis |
| C5 | FAIL | PASS | no orphaned particles in the visible Korean |
| C6 | PASS | PASS | no empty/error state on this surface; drawer always has entries |
| C7 | PASS | PASS | every field labelled (이름, voltages) |
| C8 | PASS | PASS | log text is selectable (LogDrawer.module.css user-select: text) |
| D1 | FAIL | PASS | audit `smallTargets` 0 |
| D3 | PASS | PASS | view toolbar three groups; drawer header two actions |
| D4 | FAIL | PASS | segment and toolbar buttons adjoin |
| E1 | PASS | PASS | light and dark captured; no light-only asset |
| E2 | FAIL | PASS | audit `lowContrast` 0 (DOM text; canvas excluded) |
| E3 | PASS | PASS | log status uses an icon per level next to the colour |
| E4 | PASS | PASS | green = success, blue = info throughout |
| E5 | PASS | PASS | overlays (legend, menu) are opaque cards |
| E6 | PASS | PASS | top-bar theme control offers system |
| F1 | PASS | PASS | lucide icons, one stroke |
| F2 | PASS | PASS | token spacing/radii |
| F3 | PASS | PASS | drawer, buttons and panels match the other surfaces |
| F4 | PASS | PASS | SimRep wordmark in the top bar only |
| F5 | FAIL | PASS | selected file row: wash + tinted icon, no stripe |
| F6 | PASS | PASS | standard close (X) and trash glyphs |
| G1 | PASS | PASS | no idle animation on the surface |
| G2 | FAIL | PASS | drawer no longer fades in on mount (no exit existed); results panel and export menu entry animations removed too |
| G3 | PASS | PASS | drawer opens at once; nothing gates input |
| G5 | PASS | PASS | no spinner on this surface |
| G6 | PASS | PASS | no layout shift after the drawer settles |
| G7 | PASS | PASS | no flashing |
| H1 | PASS | PASS | log has entries; empty log keeps the header |
| H3 | PASS | PASS | outcomes reported in place / in the log |
| H5 | FAIL | PASS | hover/focus/disabled styles from the shared primitives |
| I1 | FAIL | PASS | focus-visible ring on drawer buttons and resizers |
| I2 | PASS | PASS | Escape: only closes menus/modals (export menu open in this state, closes first) |
| I3 | FAIL | PASS | audit `unnamed` 0 |

Surfaces whose copy or CSS changed in round 4; the items the change touches were re-scored explicitly (the rest keep the basis above):

| Surface | Item | after | evidence |
|---|---|---|---|
| props-end | A9 | PASS | no clipping or overlap in the 1280/1920 captures |
| props-end | C2 | PASS | notice title '빠른 전압 조절(fast adjust) 배열' |
| props-end | C4 | PASS | no clipped glyphs; ellipsis where text is cut |
| props-end | C5 | PASS | no orphaned particle or split predicate in the new wording |
| electrode-selected | A9 | PASS | no clipping or overlap in the 1280/1920 captures |
| electrode-selected | C2 | PASS | hint '전압을 바꾸면 계산된 전기장을 다시 합칩니다' |
| electrode-selected | C4 | PASS | no clipped glyphs; ellipsis where text is cut |
| electrode-selected | C5 | PASS | no orphaned particle or split predicate in the new wording |
| refine-progress | A9 | PASS | no clipping or overlap in the 1280/1920 captures |
| refine-progress | C2 | PASS | chip '{file} 전기장 계산 중' (file first, full text as tooltip) |
| refine-progress | C4 | PASS | chip title ends in an ellipsis with a tooltip; meta, bar and Cancel stay visible |
| refine-progress | C5 | PASS | no orphaned particle or split predicate in the new wording |
| start-recent | A9 | PASS | no clipping or overlap in the 1280/1920 captures |
| start-recent | C2 | PASS | recovery card names the autosave and the project file separately |
| start-recent | C4 | PASS | no clipped glyphs; ellipsis where text is cut |
| start-recent | C5 | PASS | no orphaned particle or split predicate in the new wording |
| start-recent | C6 | PASS | recovery prompt: '자동 저장본({time})은 있지만 프로젝트 파일에는 저장되지 않았습니다. 복원할까요?' + 복원/버리기 |
| start-recent | A10 | PASS | third recent entry complete; list continues below |
| results-compare | A9 | PASS | log closed: results 408px at 1280 (unchanged by the max-height cap); both run cards and the toolbar fit one row |
| results-compare | A10 | PASS | side column scrollbar when cards exceed it |
| results-export-menu | G2 | PASS | menu appears without the entry-only rise-in |
| results-export-menu | A9 | PASS | menu over the results stats only; no panel overlap |

Legend: `P` pass both; `**F→P**` failed before, passes after; `–→P` contract item (A12 Layout line, B7 brief) that did not exist before and passes after; `F→J` failed before, accepted deviation after (see section 4: I4, G4, H7); `–` not applicable (A4, A5, D2, J2 phone; A11, J1 macOS; H4 in-app alerts (the discard confirm is a native dialog); H8-H11 absent features; I5 no media). Scores with before-FAIL reasons, after basis and explicit evidence per cell: `scores.json`; script `scripts/score.py`.

Totals: FAIL cells before 497, after 0; PASS cells before 1644, after 2145; N/A cells after 673; justified deviations accepted by the lead (J) after 80. After PASS basis: explicit 73, contract 82, default 1990.

#### A

| Surface | A1 | A2 | A3 | A4 | A5 | A6 | A7 | A8 | A9 | A10 | A11 | A12 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| start | P | P | P | – | – | P | P | P | P | P | – | –→P |
| start-recent | P | P | P | – | – | P | P | P | P | P | – | –→P |
| start-tour-offer | P | P | P | – | – | P | P | P | P | P | – | –→P |
| busy | P | P | P | – | – | P | P | P | P | P | – | –→P |
| empty-workspace | P | P | P | – | – | P | P | P | P | P | – | –→P |
| template-einzel | P | P | P | – | – | P | P | P | P | P | – | –→P |
| template-quadrupole | P | P | P | – | – | P | P | P | P | P | – | –→P |
| template-reflectron | P | P | P | – | – | P | P | P | P | P | – | –→P |
| template-trap | P | P | P | – | – | P | P | P | P | P | – | –→P |
| props-ions | P | P | P | – | – | P | P | P | P | P | – | –→P |
| props-flight-results | P | P | P | – | – | P | P | P | P | P | – | –→P |
| props-end | P | P | P | – | – | P | P | P | P | P | – | –→P |
| electrode-selected | P | P | P | – | – | P | P | P | P | P | – | –→P |
| live-slider | P | P | P | – | – | P | P | P | P | P | – | –→P |
| results-map | P | P | P | – | – | P | P | P | P | P | – | –→P |
| results-ion | P | P | P | – | – | P | P | P | P | P | – | –→P |
| results-table | P | P | P | – | – | P | P | P | P | P | – | –→P |
| results-compare | P | P | P | – | – | P | P | P | P | P | – | –→P |
| results-export-menu | P | P | P | – | – | P | P | P | P | P | – | –→P |
| help | P | P | P | – | – | P | P | P | P | P | – | –→P |
| log-drawer | P | P | P | – | – | P | P | P | **F→P** | P | – | –→P |
| export-dialog | P | P | P | – | – | P | P | P | P | P | – | –→P |
| toasts | P | P | P | – | – | P | P | P | P | P | – | –→P |
| error-unsupported-file | P | P | P | – | – | P | P | P | P | P | – | –→P |
| refine-progress | P | P | P | – | – | P | P | P | P | P | – | –→P |
| exact-on | P | P | P | – | – | P | P | P | P | P | – | –→P |
| exact-off | P | P | P | – | – | P | P | P | P | P | – | –→P |
| wizard-gem-parts | P | P | P | – | – | P | P | P | P | P | – | –→P |
| wizard-gem-grid | P | P | P | – | – | P | P | P | P | P | – | –→P |
| wizard-gem-refine | P | P | P | – | – | P | P | P | P | P | – | –→P |
| wizard-gem-running | P | P | P | – | – | P | P | P | P | P | – | –→P |
| wizard-gem-done | P | P | P | – | – | P | P | P | P | P | – | –→P |
| wizard-step-parts | P | P | P | – | – | P | P | P | P | P | – | –→P |
| wizard-step-grid | P | P | P | – | – | P | P | P | P | P | – | –→P |
| error-import-gem | P | P | P | – | – | P | P | P | P | P | – | –→P |
| error-import-step | P | P | P | – | – | P | P | P | P | P | – | –→P |
| tour-1 | P | P | P | – | – | P | P | P | P | P | – | –→P |
| tour-2 | P | P | P | – | – | P | P | P | P | P | – | –→P |
| tour-3 | P | P | P | – | – | P | P | P | P | P | – | –→P |
| tour-4 | P | P | P | – | – | P | P | P | P | P | – | –→P |
| tour-5 | P | P | P | – | – | P | P | P | P | P | – | –→P |
| tour-done | P | P | P | – | – | P | P | P | P | P | – | –→P |

#### B

| Surface | B1 | B2 | B3 | B4 | B5 | B6 | B7 |
|---|---|---|---|---|---|---|---|
| start | P | – | P | P | P | P | –→P |
| start-recent | P | – | P | P | P | P | –→P |
| start-tour-offer | P | – | P | P | P | P | –→P |
| busy | P | – | P | P | P | P | –→P |
| empty-workspace | P | – | P | P | P | P | –→P |
| template-einzel | P | P | P | P | P | P | –→P |
| template-quadrupole | P | P | P | P | P | P | –→P |
| template-reflectron | P | P | P | P | P | P | –→P |
| template-trap | P | P | P | P | P | P | –→P |
| props-ions | **F→P** | P | P | P | P | P | –→P |
| props-flight-results | **F→P** | P | P | P | P | P | –→P |
| props-end | P | P | P | P | P | P | –→P |
| electrode-selected | P | P | P | P | P | P | –→P |
| live-slider | P | P | P | P | P | P | –→P |
| results-map | P | P | P | P | P | P | –→P |
| results-ion | P | P | P | P | P | P | –→P |
| results-table | P | P | P | P | P | P | –→P |
| results-compare | **F→P** | P | P | P | P | P | –→P |
| results-export-menu | P | P | P | P | P | P | –→P |
| help | P | P | P | P | P | P | –→P |
| log-drawer | P | **F→P** | P | P | P | P | –→P |
| export-dialog | P | P | P | P | P | P | –→P |
| toasts | P | P | P | P | P | P | –→P |
| error-unsupported-file | P | P | P | P | P | P | –→P |
| refine-progress | P | P | P | P | P | P | –→P |
| exact-on | **F→P** | P | P | P | P | P | –→P |
| exact-off | **F→P** | P | P | P | P | P | –→P |
| wizard-gem-parts | **F→P** | – | P | P | P | P | –→P |
| wizard-gem-grid | **F→P** | – | P | P | P | P | –→P |
| wizard-gem-refine | **F→P** | – | P | P | P | P | –→P |
| wizard-gem-running | **F→P** | – | P | P | P | P | –→P |
| wizard-gem-done | **F→P** | – | P | P | P | P | –→P |
| wizard-step-parts | **F→P** | – | P | P | P | P | –→P |
| wizard-step-grid | **F→P** | – | P | P | P | P | –→P |
| error-import-gem | **F→P** | – | P | P | P | P | –→P |
| error-import-step | **F→P** | – | P | P | P | P | –→P |
| tour-1 | P | – | P | P | P | P | –→P |
| tour-2 | P | P | P | P | P | P | –→P |
| tour-3 | P | P | P | P | P | P | –→P |
| tour-4 | P | P | P | P | P | P | –→P |
| tour-5 | **F→P** | P | P | P | P | P | –→P |
| tour-done | **F→P** | P | P | P | P | P | –→P |

#### C

| Surface | C1 | C2 | C3 | C4 | C5 | C6 | C7 | C8 |
|---|---|---|---|---|---|---|---|---|
| start | **F→P** | **F→P** | P | P | **F→P** | P | P | P |
| start-recent | **F→P** | **F→P** | P | P | **F→P** | **F→P** | P | P |
| start-tour-offer | **F→P** | **F→P** | P | P | **F→P** | P | P | P |
| busy | **F→P** | **F→P** | P | P | **F→P** | P | P | P |
| empty-workspace | **F→P** | P | P | P | **F→P** | P | P | P |
| template-einzel | **F→P** | **F→P** | P | P | **F→P** | P | P | P |
| template-quadrupole | **F→P** | **F→P** | P | P | **F→P** | P | P | P |
| template-reflectron | **F→P** | **F→P** | P | P | **F→P** | P | P | P |
| template-trap | **F→P** | **F→P** | P | P | **F→P** | P | P | P |
| props-ions | **F→P** | P | P | P | **F→P** | P | P | P |
| props-flight-results | **F→P** | P | P | P | **F→P** | P | P | P |
| props-end | **F→P** | **F→P** | P | P | **F→P** | P | P | P |
| electrode-selected | **F→P** | **F→P** | P | P | **F→P** | P | P | P |
| live-slider | **F→P** | P | P | P | **F→P** | P | P | P |
| results-map | **F→P** | P | P | P | **F→P** | P | P | P |
| results-ion | **F→P** | P | P | P | **F→P** | P | P | P |
| results-table | **F→P** | P | P | P | **F→P** | P | P | P |
| results-compare | **F→P** | P | P | P | **F→P** | P | P | P |
| results-export-menu | **F→P** | P | P | P | **F→P** | P | P | P |
| help | **F→P** | P | P | P | **F→P** | P | P | P |
| log-drawer | **F→P** | P | P | P | **F→P** | P | P | P |
| export-dialog | **F→P** | P | P | P | **F→P** | P | P | P |
| toasts | **F→P** | **F→P** | P | P | **F→P** | P | P | P |
| error-unsupported-file | **F→P** | **F→P** | P | P | **F→P** | P | P | P |
| refine-progress | **F→P** | **F→P** | P | P | **F→P** | P | P | P |
| exact-on | **F→P** | P | P | P | **F→P** | P | P | P |
| exact-off | **F→P** | P | P | P | **F→P** | P | P | P |
| wizard-gem-parts | **F→P** | **F→P** | P | P | **F→P** | P | P | P |
| wizard-gem-grid | **F→P** | **F→P** | P | P | **F→P** | P | P | P |
| wizard-gem-refine | **F→P** | **F→P** | P | P | **F→P** | P | P | P |
| wizard-gem-running | **F→P** | **F→P** | P | P | **F→P** | P | P | P |
| wizard-gem-done | **F→P** | **F→P** | P | P | **F→P** | P | P | P |
| wizard-step-parts | **F→P** | **F→P** | P | P | **F→P** | P | P | P |
| wizard-step-grid | **F→P** | **F→P** | P | P | **F→P** | P | P | P |
| error-import-gem | **F→P** | **F→P** | P | P | **F→P** | **F→P** | P | **F→P** |
| error-import-step | **F→P** | **F→P** | P | P | **F→P** | **F→P** | P | **F→P** |
| tour-1 | **F→P** | **F→P** | P | P | **F→P** | P | P | P |
| tour-2 | **F→P** | **F→P** | P | P | **F→P** | P | P | P |
| tour-3 | **F→P** | **F→P** | P | P | **F→P** | P | P | P |
| tour-4 | **F→P** | P | P | P | **F→P** | P | P | P |
| tour-5 | **F→P** | P | P | P | **F→P** | P | P | P |
| tour-done | **F→P** | P | P | P | **F→P** | P | P | P |

#### D

| Surface | D1 | D2 | D3 | D4 |
|---|---|---|---|---|
| start | **F→P** | – | P | **F→P** |
| start-recent | **F→P** | – | P | **F→P** |
| start-tour-offer | **F→P** | – | P | **F→P** |
| busy | **F→P** | – | P | **F→P** |
| empty-workspace | **F→P** | – | P | **F→P** |
| template-einzel | **F→P** | – | P | **F→P** |
| template-quadrupole | **F→P** | – | P | **F→P** |
| template-reflectron | **F→P** | – | P | **F→P** |
| template-trap | **F→P** | – | P | **F→P** |
| props-ions | **F→P** | – | P | **F→P** |
| props-flight-results | **F→P** | – | P | **F→P** |
| props-end | **F→P** | – | P | **F→P** |
| electrode-selected | **F→P** | – | P | **F→P** |
| live-slider | **F→P** | – | P | **F→P** |
| results-map | **F→P** | – | P | **F→P** |
| results-ion | **F→P** | – | P | **F→P** |
| results-table | **F→P** | – | P | **F→P** |
| results-compare | **F→P** | – | P | **F→P** |
| results-export-menu | **F→P** | – | P | **F→P** |
| help | **F→P** | – | P | **F→P** |
| log-drawer | **F→P** | – | P | **F→P** |
| export-dialog | **F→P** | – | P | **F→P** |
| toasts | **F→P** | – | P | **F→P** |
| error-unsupported-file | **F→P** | – | P | **F→P** |
| refine-progress | **F→P** | – | P | **F→P** |
| exact-on | **F→P** | – | P | **F→P** |
| exact-off | **F→P** | – | P | **F→P** |
| wizard-gem-parts | **F→P** | – | P | **F→P** |
| wizard-gem-grid | **F→P** | – | P | **F→P** |
| wizard-gem-refine | **F→P** | – | P | **F→P** |
| wizard-gem-running | **F→P** | – | P | **F→P** |
| wizard-gem-done | **F→P** | – | P | **F→P** |
| wizard-step-parts | **F→P** | – | P | **F→P** |
| wizard-step-grid | **F→P** | – | P | **F→P** |
| error-import-gem | **F→P** | – | P | **F→P** |
| error-import-step | **F→P** | – | P | **F→P** |
| tour-1 | **F→P** | – | P | **F→P** |
| tour-2 | **F→P** | – | P | **F→P** |
| tour-3 | **F→P** | – | P | **F→P** |
| tour-4 | **F→P** | – | P | **F→P** |
| tour-5 | **F→P** | – | P | **F→P** |
| tour-done | **F→P** | – | P | **F→P** |

#### E

| Surface | E1 | E2 | E3 | E4 | E5 | E6 |
|---|---|---|---|---|---|---|
| start | P | **F→P** | P | P | P | P |
| start-recent | P | **F→P** | P | P | P | P |
| start-tour-offer | P | **F→P** | P | P | P | P |
| busy | P | **F→P** | P | P | P | P |
| empty-workspace | P | **F→P** | P | P | P | P |
| template-einzel | P | **F→P** | P | P | P | P |
| template-quadrupole | P | **F→P** | P | P | P | P |
| template-reflectron | P | **F→P** | P | P | P | P |
| template-trap | P | **F→P** | P | P | P | P |
| props-ions | P | **F→P** | P | P | P | P |
| props-flight-results | P | **F→P** | P | P | P | P |
| props-end | P | **F→P** | P | P | P | P |
| electrode-selected | P | **F→P** | P | P | P | P |
| live-slider | P | **F→P** | P | P | P | P |
| results-map | P | **F→P** | P | P | P | P |
| results-ion | P | **F→P** | P | P | P | P |
| results-table | P | **F→P** | P | P | P | P |
| results-compare | P | **F→P** | P | P | P | P |
| results-export-menu | P | **F→P** | P | P | P | P |
| help | P | **F→P** | P | P | P | P |
| log-drawer | P | **F→P** | P | P | P | P |
| export-dialog | P | **F→P** | P | P | P | P |
| toasts | P | **F→P** | P | P | P | P |
| error-unsupported-file | P | **F→P** | P | P | P | P |
| refine-progress | P | **F→P** | P | P | P | P |
| exact-on | P | **F→P** | P | P | P | P |
| exact-off | P | **F→P** | P | P | P | P |
| wizard-gem-parts | P | **F→P** | P | P | P | P |
| wizard-gem-grid | P | **F→P** | P | P | P | P |
| wizard-gem-refine | P | **F→P** | P | P | P | P |
| wizard-gem-running | P | **F→P** | P | P | P | P |
| wizard-gem-done | P | **F→P** | P | P | P | P |
| wizard-step-parts | P | **F→P** | P | P | P | P |
| wizard-step-grid | P | **F→P** | P | P | P | P |
| error-import-gem | P | **F→P** | P | P | P | P |
| error-import-step | P | **F→P** | P | P | P | P |
| tour-1 | P | **F→P** | P | P | P | P |
| tour-2 | P | **F→P** | P | P | P | P |
| tour-3 | P | **F→P** | P | P | P | P |
| tour-4 | P | **F→P** | P | P | P | P |
| tour-5 | P | **F→P** | P | P | P | P |
| tour-done | P | **F→P** | P | P | P | P |

#### F

| Surface | F1 | F2 | F3 | F4 | F5 | F6 | F7 |
|---|---|---|---|---|---|---|---|
| start | P | P | P | P | P | P | – |
| start-recent | P | P | P | P | P | P | – |
| start-tour-offer | P | P | P | P | P | P | P |
| busy | P | P | P | P | P | P | – |
| empty-workspace | P | P | P | P | P | P | – |
| template-einzel | P | P | P | P | **F→P** | P | – |
| template-quadrupole | P | P | P | P | **F→P** | P | – |
| template-reflectron | P | P | P | P | **F→P** | P | – |
| template-trap | P | P | P | P | **F→P** | P | – |
| props-ions | P | P | P | P | **F→P** | P | – |
| props-flight-results | P | P | P | P | **F→P** | P | – |
| props-end | P | P | P | P | **F→P** | P | – |
| electrode-selected | P | P | P | P | **F→P** | P | – |
| live-slider | P | P | P | P | **F→P** | P | – |
| results-map | P | P | P | P | **F→P** | P | – |
| results-ion | P | P | P | P | **F→P** | P | – |
| results-table | P | P | P | P | **F→P** | P | – |
| results-compare | P | P | P | P | **F→P** | P | – |
| results-export-menu | P | P | P | P | **F→P** | P | P |
| help | P | P | P | P | **F→P** | P | P |
| log-drawer | P | P | P | P | **F→P** | P | – |
| export-dialog | P | P | P | P | **F→P** | P | P |
| toasts | P | P | P | P | **F→P** | P | – |
| error-unsupported-file | P | P | P | P | **F→P** | P | – |
| refine-progress | P | P | P | P | **F→P** | P | – |
| exact-on | P | P | P | P | **F→P** | P | – |
| exact-off | P | P | P | P | **F→P** | P | – |
| wizard-gem-parts | P | P | P | P | P | P | – |
| wizard-gem-grid | P | P | P | P | **F→P** | P | – |
| wizard-gem-refine | P | P | P | P | P | P | – |
| wizard-gem-running | P | P | P | P | P | P | – |
| wizard-gem-done | P | P | P | P | P | P | – |
| wizard-step-parts | P | P | P | P | P | P | – |
| wizard-step-grid | P | P | P | P | **F→P** | P | – |
| error-import-gem | P | P | P | P | P | P | – |
| error-import-step | P | P | P | P | P | P | – |
| tour-1 | P | P | P | P | P | P | P |
| tour-2 | P | P | P | P | **F→P** | P | P |
| tour-3 | P | P | P | P | **F→P** | P | P |
| tour-4 | P | P | P | P | **F→P** | P | P |
| tour-5 | P | P | P | P | **F→P** | P | P |
| tour-done | P | P | P | P | **F→P** | P | P |

#### G

| Surface | G1 | G2 | G3 | G4 | G5 | G6 | G7 |
|---|---|---|---|---|---|---|---|
| start | P | P | P | F→J | P | P | P |
| start-recent | P | P | P | F→J | P | P | P |
| start-tour-offer | P | P | P | F→J | P | P | P |
| busy | P | **F→P** | P | F→J | **F→P** | P | P |
| empty-workspace | P | P | P | F→J | P | P | P |
| template-einzel | P | P | P | F→J | P | P | P |
| template-quadrupole | P | P | P | F→J | P | P | P |
| template-reflectron | P | P | P | F→J | P | P | P |
| template-trap | P | P | P | F→J | P | P | P |
| props-ions | P | P | P | F→J | P | P | P |
| props-flight-results | P | P | P | F→J | P | P | P |
| props-end | P | P | P | F→J | P | P | P |
| electrode-selected | P | P | P | F→J | P | P | P |
| live-slider | P | P | P | F→J | P | P | P |
| results-map | P | P | P | F→J | P | P | P |
| results-ion | P | P | P | F→J | **F→P** | P | P |
| results-table | P | P | P | F→J | P | P | P |
| results-compare | P | P | P | F→J | P | P | P |
| results-export-menu | P | **F→P** | P | F→J | P | P | P |
| help | P | **F→P** | P | F→J | P | P | P |
| log-drawer | P | **F→P** | P | F→J | P | P | P |
| export-dialog | P | **F→P** | P | F→J | P | P | P |
| toasts | P | P | P | F→J | P | P | P |
| error-unsupported-file | P | P | P | F→J | P | P | P |
| refine-progress | P | P | P | F→J | **F→P** | P | P |
| exact-on | P | P | P | F→J | P | P | P |
| exact-off | P | P | P | F→J | P | P | P |
| wizard-gem-parts | P | P | P | F→J | P | P | P |
| wizard-gem-grid | P | P | P | F→J | P | P | P |
| wizard-gem-refine | P | P | P | F→J | P | P | P |
| wizard-gem-running | P | P | P | F→J | **F→P** | P | P |
| wizard-gem-done | P | P | P | F→J | P | P | P |
| wizard-step-parts | P | P | P | F→J | P | P | P |
| wizard-step-grid | P | P | P | F→J | P | P | P |
| error-import-gem | P | P | P | F→J | P | P | P |
| error-import-step | P | P | P | F→J | P | P | P |
| tour-1 | P | P | P | F→J | P | P | P |
| tour-2 | P | P | P | F→J | P | P | P |
| tour-3 | P | P | P | F→J | P | P | P |
| tour-4 | P | P | P | F→J | P | P | P |
| tour-5 | P | P | P | F→J | P | P | P |
| tour-done | P | P | P | F→J | P | P | P |

#### H

| Surface | H1 | H2 | H3 | H4 | H5 | H6 | H7 | H8 | H9 | H10 | H11 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| start | P | – | P | – | P | – | – | – | – | – | – |
| start-recent | P | – | P | – | P | – | – | – | – | – | – |
| start-tour-offer | P | – | P | – | P | – | – | – | – | – | – |
| busy | P | **F→P** | P | – | P | – | F→J | – | – | – | – |
| empty-workspace | P | – | P | – | **F→P** | – | – | – | – | – | – |
| template-einzel | P | – | P | – | **F→P** | – | – | – | – | – | – |
| template-quadrupole | P | – | P | – | **F→P** | – | – | – | – | – | – |
| template-reflectron | P | – | P | – | **F→P** | – | – | – | – | – | – |
| template-trap | P | – | P | – | **F→P** | – | – | – | – | – | – |
| props-ions | P | – | P | – | **F→P** | – | – | – | – | – | – |
| props-flight-results | P | – | P | – | **F→P** | – | – | – | – | – | – |
| props-end | P | – | P | – | **F→P** | – | – | – | – | – | – |
| electrode-selected | P | – | P | – | **F→P** | – | – | – | – | – | – |
| live-slider | P | – | P | – | **F→P** | – | – | – | – | – | – |
| results-map | P | – | P | – | **F→P** | – | – | – | – | – | – |
| results-ion | P | **F→P** | P | – | **F→P** | – | – | – | – | – | – |
| results-table | P | – | P | – | **F→P** | – | – | – | – | – | – |
| results-compare | P | – | P | – | **F→P** | – | – | – | – | – | – |
| results-export-menu | P | – | P | – | **F→P** | – | – | – | – | – | – |
| help | P | – | P | – | **F→P** | **F→P** | – | – | – | – | – |
| log-drawer | P | – | P | – | **F→P** | – | – | – | – | – | – |
| export-dialog | P | – | **F→P** | – | **F→P** | **F→P** | – | – | – | – | – |
| toasts | P | – | P | – | **F→P** | – | – | – | – | – | – |
| error-unsupported-file | P | – | P | – | **F→P** | – | – | – | – | – | – |
| refine-progress | P | **F→P** | P | – | **F→P** | – | P | – | – | – | – |
| exact-on | P | – | P | – | **F→P** | – | – | – | – | – | – |
| exact-off | P | – | P | – | **F→P** | – | – | – | – | – | – |
| wizard-gem-parts | P | – | P | – | **F→P** | – | – | – | – | – | – |
| wizard-gem-grid | P | – | P | – | **F→P** | – | – | – | – | – | – |
| wizard-gem-refine | P | – | P | – | **F→P** | – | – | – | – | – | – |
| wizard-gem-running | P | **F→P** | P | – | **F→P** | – | P | – | – | – | – |
| wizard-gem-done | P | – | P | – | **F→P** | – | – | – | – | – | – |
| wizard-step-parts | P | – | P | – | **F→P** | – | – | – | – | – | – |
| wizard-step-grid | P | – | P | – | **F→P** | – | – | – | – | – | – |
| error-import-gem | P | – | P | – | **F→P** | – | – | – | – | – | – |
| error-import-step | P | – | P | – | **F→P** | – | – | – | – | – | – |
| tour-1 | P | – | P | – | P | – | – | – | – | – | – |
| tour-2 | P | – | P | – | **F→P** | – | – | – | – | – | – |
| tour-3 | P | – | P | – | **F→P** | – | – | – | – | – | – |
| tour-4 | P | – | P | – | **F→P** | – | – | – | – | – | – |
| tour-5 | P | – | P | – | **F→P** | – | – | – | – | – | – |
| tour-done | P | – | P | – | **F→P** | – | – | – | – | – | – |

#### I

| Surface | I1 | I2 | I3 | I4 | I5 |
|---|---|---|---|---|---|
| start | P | P | P | – | – |
| start-recent | P | P | P | – | – |
| start-tour-offer | P | P | P | – | – |
| busy | P | P | P | – | – |
| empty-workspace | **F→P** | P | **F→P** | F→J | – |
| template-einzel | **F→P** | P | **F→P** | F→J | – |
| template-quadrupole | **F→P** | P | **F→P** | F→J | – |
| template-reflectron | **F→P** | P | **F→P** | F→J | – |
| template-trap | **F→P** | P | **F→P** | F→J | – |
| props-ions | **F→P** | P | **F→P** | F→J | – |
| props-flight-results | **F→P** | P | **F→P** | F→J | – |
| props-end | **F→P** | P | **F→P** | F→J | – |
| electrode-selected | **F→P** | P | **F→P** | F→J | – |
| live-slider | **F→P** | P | **F→P** | F→J | – |
| results-map | **F→P** | P | **F→P** | F→J | – |
| results-ion | **F→P** | P | **F→P** | F→J | – |
| results-table | **F→P** | P | **F→P** | F→J | – |
| results-compare | **F→P** | P | **F→P** | F→J | – |
| results-export-menu | **F→P** | **F→P** | **F→P** | F→J | – |
| help | **F→P** | **F→P** | **F→P** | F→J | – |
| log-drawer | **F→P** | P | **F→P** | F→J | – |
| export-dialog | **F→P** | **F→P** | **F→P** | F→J | – |
| toasts | **F→P** | P | **F→P** | F→J | – |
| error-unsupported-file | **F→P** | P | **F→P** | F→J | – |
| refine-progress | **F→P** | P | **F→P** | F→J | – |
| exact-on | **F→P** | P | **F→P** | F→J | – |
| exact-off | **F→P** | P | **F→P** | F→J | – |
| wizard-gem-parts | **F→P** | P | **F→P** | F→J | – |
| wizard-gem-grid | **F→P** | P | **F→P** | F→J | – |
| wizard-gem-refine | **F→P** | P | **F→P** | F→J | – |
| wizard-gem-running | **F→P** | P | **F→P** | F→J | – |
| wizard-gem-done | **F→P** | P | **F→P** | F→J | – |
| wizard-step-parts | **F→P** | P | **F→P** | F→J | – |
| wizard-step-grid | **F→P** | P | **F→P** | F→J | – |
| error-import-gem | **F→P** | P | **F→P** | F→J | – |
| error-import-step | **F→P** | P | **F→P** | F→J | – |
| tour-1 | P | P | P | – | – |
| tour-2 | **F→P** | P | **F→P** | F→J | – |
| tour-3 | **F→P** | P | **F→P** | F→J | – |
| tour-4 | **F→P** | P | **F→P** | F→J | – |
| tour-5 | **F→P** | P | **F→P** | F→J | – |
| tour-done | **F→P** | P | **F→P** | F→J | – |

#### J

| Surface | J1 | J2 |
|---|---|---|
| start | – | – |
| start-recent | – | – |
| start-tour-offer | – | – |
| busy | – | – |
| empty-workspace | – | – |
| template-einzel | – | – |
| template-quadrupole | – | – |
| template-reflectron | – | – |
| template-trap | – | – |
| props-ions | – | – |
| props-flight-results | – | – |
| props-end | – | – |
| electrode-selected | – | – |
| live-slider | – | – |
| results-map | – | – |
| results-ion | – | – |
| results-table | – | – |
| results-compare | – | – |
| results-export-menu | – | – |
| help | – | – |
| log-drawer | – | – |
| export-dialog | – | – |
| toasts | – | – |
| error-unsupported-file | – | – |
| refine-progress | – | – |
| exact-on | – | – |
| exact-off | – | – |
| wizard-gem-parts | – | – |
| wizard-gem-grid | – | – |
| wizard-gem-refine | – | – |
| wizard-gem-running | – | – |
| wizard-gem-done | – | – |
| wizard-step-parts | – | – |
| wizard-step-grid | – | – |
| error-import-gem | – | – |
| error-import-step | – | – |
| tour-1 | – | – |
| tour-2 | – | – |
| tour-3 | – | – |
| tour-4 | – | – |
| tour-5 | – | – |
| tour-done | – | – |

## 6. DOM audit per surface (light + dark, 1280x800), before → after

| Surface | text <12px | Hangul in mono | low contrast | unnamed control | target <24px |
|---|---|---|---|---|---|
| start | 80→0 | 2→0 | 37→0 | 0→0 | 12→0 |
| start-recent | 90→0 | 2→0 | 45→0 | 0→0 | 12→0 |
| start-tour-offer | 84→0 | 2→0 | 39→0 | 0→0 | 12→0 |
| busy | 80→0 | 2→0 | 37→0 | 0→0 | 12→0 |
| empty-workspace | 62→0 | 2→0 | 21→0 | 2→0 | 12→0 |
| template-einzel | 182→0 | 6→0 | 85→0 | 2→0 | 32→0 |
| template-quadrupole | 180→0 | 6→0 | 83→0 | 2→0 | 32→0 |
| template-reflectron | 196→0 | 6→0 | 91→0 | 2→0 | 30→0 |
| template-trap | 170→0 | 6→0 | 75→0 | 2→0 | 32→0 |
| props-ions | 178→0 | 6→0 | 85→0 | 8→0 | 40→0 |
| props-flight-results | 206→0 | 10→0 | 86→0 | 2→0 | 40→0 |
| props-end | 152→0 | 8→0 | 57→0 | 2→0 | 28→0 |
| electrode-selected | 160→0 | 6→0 | 67→0 | 2→0 | 30→0 |
| live-slider | 192→0 | 6→0 | 99→0 | 8→0 | 40→0 |
| results-map | 196→0 | 8→0 | 105→0 | 8→0 | 32→0 |
| results-ion | 186→0 | 10→0 | 89→0 | 8→0 | 42→0 |
| results-table | 370→0 | 6→0 | 99→0 | 8→0 | 42→0 |
| results-compare | 178→0 | 6→0 | 91→0 | 10→0 | 46→0 |
| results-export-menu | 206→0 | 6→0 | 111→0 | 8→0 | 40→0 |
| help | 320→0 | 6→0 | 225→0 | 8→0 | 40→0 |
| log-drawer | 276→0 | 6→0 | 182→0 | 8→0 | 40→0 |
| export-dialog | 222→0 | 10→0 | 125→0 | 8→0 | 54→0 |
| toasts | 184→0 | 6→0 | 85→0 | 2→0 | 34→0 |
| error-unsupported-file | 184→0 | 6→0 | 85→0 | 2→0 | 34→0 |
| refine-progress | 112→0 | 6→0 | 46→0 | 2→0 | 32→0 |
| exact-on | 178→0 | 6→0 | 85→0 | 14→0 | 48→0 |
| exact-off | 174→0 | 6→0 | 83→0 | 14→0 | 48→0 |
| wizard-gem-parts | 88→0 | 2→0 | 37→0 | 2→0 | 12→0 |
| wizard-gem-grid | 90→0 | 8→0 | 43→0 | 2→0 | 12→0 |
| wizard-gem-refine | 74→0 | 2→0 | 27→0 | 2→0 | 18→0 |
| wizard-gem-running | 114→0 | 6→0 | 42→0 | 2→0 | 20→0 |
| wizard-gem-done | 114→0 | 2→0 | 40→0 | 2→0 | 20→0 |
| wizard-step-parts | 94→0 | 2→0 | 37→0 | 2→0 | 22→0 |
| wizard-step-grid | 114→0 | 12→0 | 69→0 | 14→0 | 12→0 |
| error-import-gem | 70→0 | 2→0 | 21→0 | 2→0 | 12→0 |
| error-import-step | 70→0 | 2→0 | 21→0 | 2→0 | 12→0 |
| tour-1 | 82→0 | 2→0 | 37→0 | 0→0 | 12→0 |
| tour-2 | 148→0 | 6→0 | 65→0 | 2→0 | 18→0 |
| tour-3 | 136→0 | 6→0 | 52→0 | 2→0 | 22→0 |
| tour-4 | 136→0 | 6→0 | 52→0 | 2→0 | 22→0 |
| tour-5 | 172→0 | 6→0 | 72→0 | 2→0 | 36→0 |
| tour-done | 172→0 | 6→0 | 72→0 | 2→0 | 36→0 |

## 7. Surfaces captured

| id | surface |
|---|---|
| start | Start screen: templates and open file (no recent projects) |
| start-recent | Start screen with recent projects and a recovery offer |
| start-tour-offer | First-launch tour offer card on the start screen |
| busy | Busy overlay while a template is created |
| empty-workspace | Empty workspace (no objects, start screen closed) |
| template-einzel | Einzel lens template after refine + fly, results (TOF histogram) |
| template-quadrupole | Quadrupole template after refine + fly, results (TOF histogram) |
| template-reflectron | Reflectron TOF template after refine + fly, results (TOF histogram) |
| template-trap | Ion trap template after refine + fly, results (TOF histogram) |
| props-ions | Einzel: properties panel scrolled to the ion source section |
| props-flight-results | Einzel: properties panel scrolled to the flight results section (TOF stats, hits) |
| props-end | Einzel: properties panel scrolled to the end (flight results section) |
| electrode-selected | Einzel: electrode #2 selected in the scene tree (electrode properties) |
| live-slider | Einzel: dragging the #2 voltage slider (live re-fly during the drag) |
| results-map | Results: detector map tab |
| results-ion | Results: ion tab with ion #5 selected (time series) |
| results-table | Results: end-position table |
| results-compare | Results: compare tab with one kept run and a changed voltage |
| results-export-menu | Results: export menu open |
| help | Help overlay (shortcuts, start tour) |
| log-drawer | Log drawer open |
| export-dialog | Export dialog (geometry tab) |
| toasts | Toasts after creating a template |
| error-unsupported-file | Error: opening an unsupported file (toast) |
| refine-progress | Refine in progress (properties refine section + viewport chip) |
| exact-on | Exact surface card ON (quadrupole cut-cell .pa0 with .cuts.json) + results badge |
| exact-off | Exact surface card OFF (grid-point collision, GPU live) |
| wizard-gem-parts | Import wizard (GEM): step 1 parts |
| wizard-gem-grid | Import wizard (GEM): step 2 grid |
| wizard-gem-refine | Import wizard (GEM): step 3 output + refine (before running) |
| wizard-gem-running | Import wizard (GEM): build + refine in progress |
| wizard-gem-done | Import wizard (GEM): done, ready to fly |
| wizard-step-parts | Import wizard (STEP pair.step): step 1 parts (CAD B-rep check) |
| wizard-step-grid | Import wizard (STEP): step 2 grid (cut-cell, CAD surface) |
| error-import-gem | Error: import wizard on a broken GEM file |
| error-import-step | Error: import wizard on a broken STEP file |
| tour-1 | Tour step 1/5: pick the einzel template (start screen) |
| tour-2 | Tour step 2/5: change the focus voltage |
| tour-3 | Tour step 3/5: fly |
| tour-4 | Tour step 4/5: TOF histogram |
| tour-5 | Tour step 5/5: export CSV |
| tour-done | Tour finished card |

Not capturable through CDP (recorded, not scored): native Windows dialogs (open/save, discard confirmation - its copy was shortened to two sentences), the file-drop overlay (Tauri native drag events), the WebGPU-unavailable card. There is no settings window; preferences are the top-bar language and theme (incl. "system") controls, present in every capture.

## 8. Changed files

```
 app/src/App.tsx                                   |   5 +-
 app/src/components/Dialog.module.css              |   8 +-
 app/src/components/ExportDialog.module.css        |   3 +-
 app/src/components/ExportDialog.tsx               |  14 +--
 app/src/components/ImportWizard.module.css        |  14 +--
 app/src/components/ImportWizard.tsx               |  56 +++++-----
 app/src/components/LogDrawer.module.css           |   1 -
 app/src/components/LogDrawer.tsx                  |   4 +-
 app/src/components/PanelResizer.tsx               |  21 +++-
 app/src/components/SceneTree.module.css           |  19 ++--
 app/src/components/SimulationSections.module.css  |   4 +-
 app/src/components/SimulationSections.tsx         |   6 +-
 app/src/components/StartScreen.module.css         |  20 ++++
 app/src/components/StatusBar.tsx                  |   4 +-
 app/src/components/TopBar.module.css              |   1 -
 app/src/components/TopBar.tsx                     |   5 +-
 app/src/components/ui/Button.module.css           |  17 +++-
 app/src/components/ui/Button.tsx                  |   2 +-
 app/src/components/ui/SegmentedControl.module.css |   4 +-
 app/src/components/ui/Switch.module.css           |  13 +--
 app/src/components/ui/Vec3Field.tsx               |   5 +-
 app/src/components/ui/inputs.module.css           |   4 +-
 app/src/i18n/en.ts                                |   3 +-
 app/src/i18n/ko.ts                                | 113 ++++++++++----------
 app/src/results/CompareTab.tsx                    |   1 +
 app/src/results/DetectorTab.tsx                   |   2 +-
 app/src/results/ResultsPanel.module.css           | 119 ++++++++++++++++++++--
 app/src/results/ResultsPanel.tsx                  |  39 +++++--
 app/src/results/TableTab.tsx                      |   2 +-
 app/src/results/plot/chart.ts                     |   6 ++
 app/src/styles/base.css                           |  17 +++-
 app/src/styles/tokens.css                         | 113 +++++++++++++-------
 app/src/viewport/RefineChip.tsx                   |   6 +-
 app/src/viewport/Viewport.module.css              |  45 ++++++--
 app/src/viewport/ViewportToolbar.module.css       |   1 -
 app/src/viewport/ViewportToolbar.tsx              |   2 +-
 app/src/viewport/camera.ts                        |   2 +
 37 files changed, 486 insertions(+), 215 deletions(-)
```

Evidence and tooling (new, under `docs/report/visual-qa/`): `before/` and `after/` (168 PNGs + `manifest.json` each; `motion.json`; `after/funccheck.json`), `pairs/` (42 before|after x light/dark sheets at 1280x800), `scores.json`, `scripts/` (`vqa.mjs` capture driver, `plan.mjs` surface plan, `audit.js`, `motion.mjs`, `funccheck.js/.mjs`, `logcheck.js`, `h6watch.js`/`h6result.js`/`h6w32.ps1` (H6 check; the .ps1 holds this run's window handles), `sheets.py`, `score.py`). The PNG sets are ~380 MB; the lead may prefer to keep them out of git (or in LFS).

## 9. How it was captured

- Debug app `C:\Users\lsu10\simrep-app-target-fig\debug\simrep-app.exe` (cad build) with `--remote-debugging-port=9333`; it loads the frontend from Vite (`app/`, 127.0.0.1:1420), so frontend edits need no Rust rebuild (no `src-tauri` change was made). Driven over CDP from Windows node (`scripts/vqa.mjs`), viewport emulated at 1280x800@2x and 1920x1080@1x, theme set through the store, Korean UI.
- **before** = the pre-change frontend: `git archive HEAD app` (untouched source) served by Vite with the same binary, captured with the same plan (`scripts/plan.mjs`, scripted voltages). The very first before set (captured before any edit) is kept outside the repo at `C:\Users\lsu10\simrep-gui-test\visual-qa\before-initial`; it was replaced because its drag step left a different voltage than the after run.
- **after** = one complete fresh capture of the round-3 build (all 168 PNGs, audits, motion and functional checks), then in round 4 the surfaces the round-4 changes touch were re-captured with their audits on the final source: `log-drawer`, `results-compare`, `results-export-menu`, `props-end`, `electrode-selected`, `refine-progress`, `start-recent` (both themes, both sizes). The round-4 changes do not touch any other surface: the results-panel cap only binds when the log drawer is open (only `log-drawer` opens it), and the removed entry animations do not change a settled capture.
- Geometry probe (`scripts/logcheck.js`, `after/logcheck.json`): results panel, 3D view and log drawer rectangles with the log closed and open at 1280x800 and 1920x1080.
- Functional checks (`after/funccheck.json`, reduced motion emulated): side-panel resizer focusable, ArrowLeft +16px, Shift+ArrowRight -64px; results resizer ArrowUp +16px, Home = default; with the export dialog open F1 does not open Help and one Escape closes only the dialog with the selection kept; a camera preset settles within 60 ms. Reduced motion (`motion.json`): before, the spinner ran with a 1 ms duration (frozen/flicker); after, 900 ms and advancing (progress 0.08 → 0.40 in 300 ms; sampled on the round-3 build, whose CSS is identical to the final one); `rise-in` becomes a pure fade. Export dialog dismissed mid-export: `after/h6check.json`.

## 10. Commands run

| Command | Result |
|---|---|
| `cd app && npm run typecheck` | exit 0, 0 errors |
| `cd app && npm run build` (tsc + vite build) | exit 0 |
| `bun …/frontend/scripts/perfection/check-draft-guard.ts app/src` | clean, 124 files |
| lint / unit tests | none exist in `app/package.json` (only `dev`, `build`, `preview`, `typecheck`, `tauri`) and none are listed in docs/dev/frontend.md |
| `node scripts/vqa.mjs <dir> before|after` (+ `AUDIT_ONLY=1`) | 168 PNGs + 84 audits per phase, 0 surface errors |
| `node scripts/motion.mjs`, `node scripts/funccheck.mjs` | see section 9 |
| round 4: `node vqa.mjs <after> after <7 ids>` (+ `AUDIT_ONLY=1`), `node cdp.mjs eval logcheck.js` at both sizes | 28 PNGs + 14 audits, all audit counts 0, overlap 0 |

## 11. Review rounds

| Round | Build | Reviewers | Verdicts | Blocking findings → resolution |
|---|---|---|---|---|
| 1 (~15:30-16:25) | first after set | 2 dispatched here (Pass A, Pass B) + 4 passes run by the lead | FAIL / REVISE | items 1-22 and the voltage mismatch (section 4) |
| 2 (~16:55-17:20) | complete fresh set after round-1 fixes; before re-captured with the same plan | 2 fresh (A, B) | REVISE / REVISE | R2-1 … R2-11 fixed; R2-12 (I4), R2-13 (G4) accepted by the lead |
| 3 (~17:22-17:55) | complete fresh set after round-2 fixes | 2 fresh (A, B) | REVISE (only H6) / REVISE (only Korean particle notation) | R3-1, R3-2 fixed afterwards; budget spent, so author-verified only |
| 4, gate | round-3 set + R3 fixes | independent gate review run by the lead (st_01a17256, `.omo/evidence/simrep-visual-qa-gate-review.md`) | FAIL | B1 log/results overlap → R4-1; N1 jargon → R4-3; N2 recovery wording → R4-2; N3 scoring basis → section 5 method; R4-4 found while re-scoring. Author-verified only |

After R3-1/R3-2 the after set was captured once more, completely (168 PNGs, 84 audits, motion and functional checks), on the final source; `after/h6check.json` records the H6 runtime check. Reviewer reports are in the task log of this session (task ids st_01a171c0/c1, st_01a171f6/f7, st_01a1721f/st_01a17220; gate review st_01a17256).

## 12. Recorded taste findings and accepted deviations

- Accepted by the lead, not implemented: I4 keyboard orbit/pan/zoom of the 3D view; G4 transform transitions snap (not cross-fade) under reduced motion; H7 no Cancel on the 0.2-1.3 s template busy overlay.
- Taste (round 1): the 1920 ion grid balance and 1280 table header tightness were raised as taste and fixed with the defects.
- Electrodes as grid cells in the PA view: by design (see section 2).
- Templates and refines during capture wrote throwaway projects under `Documents\SimRep\<template>-<n>` (as earlier report captures did); they can be deleted.

## 13. Windows cleanup

Round 4 (final). No `simrep-app.exe` or Vite (port 1420) left: the app and Vite ran as tool-managed sessions, tree-killed at the end (`taskkill /IM simrep-app.exe /T /F` → "not found"; 0 listeners on :1420). `tools/win/winproc.ps1 cleanup`:

```
process counts: cmd.exe 4, conhost.exe 25, node.exe 4, powershell.exe 1
orphan conhost: 0
busy conhost (>50% of a core): 4   (pre-existing, other sessions' wsl.exe shells, same pids as before the work)
  pid 26152 cpu 98% parent wsl.exe (49640) wsl.exe -d Ubuntu --exec sh -s -- /home/lsu10/orca/workspaces/SIMION/SIMION_Rep
  pid 52060 / 29192 / 56704 cpu 98% parent wsl.exe (33024) ... /mnt/d/Private/Opera_test ...
stopped orphan conhost: 0
```
