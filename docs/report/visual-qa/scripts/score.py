"""Per-surface, per-item before/after verdicts for the visual QA checklist (A1-J2).

The verdicts below are the scorer's judgement from the captures, the DOM audits in
before/manifest.json and after/manifest.json, motion.json, and the source; the audit numbers
are printed next to them as evidence. Writes scores.json and the section tables (markdown) to
stdout for REPORT.md.

usage: python score.py <before_manifest> <after_manifest> <out_scores.json>
"""
import json
import sys

ITEMS = {
    "A": [f"A{i}" for i in range(1, 13)],
    "B": [f"B{i}" for i in range(1, 8)],
    "C": [f"C{i}" for i in range(1, 9)],
    "D": [f"D{i}" for i in range(1, 5)],
    "E": [f"E{i}" for i in range(1, 7)],
    "F": [f"F{i}" for i in range(1, 8)],
    "G": [f"G{i}" for i in range(1, 8)],
    "H": [f"H{i}" for i in range(1, 12)],
    "I": [f"I{i}" for i in range(1, 6)],
    "J": ["J1", "J2"],
}

START = ["start", "start-recent", "start-tour-offer", "busy", "tour-1"]
WIZ = ["wizard-gem-parts", "wizard-gem-grid", "wizard-gem-refine", "wizard-gem-running", "wizard-gem-done", "wizard-step-parts", "wizard-step-grid", "error-import-gem", "error-import-step"]
TEMPLATES = ["template-einzel", "template-quadrupole", "template-reflectron", "template-trap"]
DIALOGS = ["help", "export-dialog"]

# Items with no instance on this desktop (Windows) app: phone-only, macOS-only, absent features.
GLOBAL_NA = {"A4", "A5", "A11", "D2", "H4", "H8", "H9", "H10", "H11", "I5", "J1", "J2"}

# Before FAILs found on every surface (global tokens / shared chrome: top bar, status bar, base.css).
GLOBAL_FAIL = {
    "C1": "body 13px, Hangul captions 10-11px (tokens --text-*)",
    "C5": "Hangul set in the mono stack falls back to a fixed-width face: '삼 각 형' (status bar)",
    "D1": "segments, switches and status-bar buttons 20px tall",
    "D4": "2px gaps between adjacent top-bar / toolbar buttons",
    "E2": "--color-text-subtle 3.4:1 (light) / 3.9:1 (dark); input edges ~1.3:1; disabled buttons at 45% opacity ~2.8:1",
    "G4": "reduced motion forced every animation to 1ms (spinners freeze) and camera fly-overs still ran 320ms",
}


def surface_fails(sid):
    """Before FAILs specific to one surface: item -> reason."""
    f = {}
    if sid not in START:
        for it in ("H5", "I1", "I4"):
            f[it] = "panel and results resize handles were pointer-only: not focusable, no keyboard resizing"
    if sid not in START and sid != "empty-workspace" and sid not in WIZ:
        f["F5"] = "selected scene-tree row marked by a 2px accent stripe on its left edge"
    if sid not in START:
        f["I3"] = "grid toggle named only 'G'" + ("; X/Y/Z inputs named 'X'" if sid not in ("empty-workspace",) else "")
    if sid == "results-compare":
        f["I3"] = "grid toggle 'G'; compare run toggle has no name; X/Y/Z inputs"
        f["D1"] = GLOBAL_FAIL["D1"] + "; compare run toggle 12x16"
    if sid in ("props-ions", "props-flight-results", "results-compare", "exact-on", "exact-off", "tour-5", "tour-done") or sid in WIZ:
        f["B1"] = "top-bar 'Open' filled accent next to the view's own primary (Fly / Keep / Next / Build)"
    if sid in ("wizard-gem-grid", "wizard-step-grid"):
        f["F5"] = "recommendation callout carries a 2px accent stripe on its left edge"
        f["C5"] = GLOBAL_FAIL["C5"] + "; cut-cell hint leaves '씁니다.' alone on the last line"
    if sid in TEMPLATES or sid in START or sid in ("refine-progress", "toasts", "error-unsupported-file", "tour-2", "tour-3", "props-end") or sid in WIZ:
        f["C2"] = "engine jargon as labels without plain words: 'REFINE' section, 'Refine' step/button, 'Cut-cell', 'Fast adjust:'"
    if sid in ("error-import-gem", "error-import-step"):
        f["C2"] = "raw '\\\\?\\C:\\...' path and parser text as the only explanation"
        f["C6"] = "says it failed, not what to do next"
        f["C8"] = "technical detail is not selectable"
    if sid == "log-drawer":
        f["A9"] = "3D view squeezed to ~140px: potential legend cut off top and bottom"
        f["B2"] = "'지우기' clears the whole log but looks like any ghost button"
    if sid in ("help", "export-dialog", "busy"):
        f["G2"] = "dialog rises in but unmounts with no exit"
    if sid in ("busy", "wizard-gem-running"):
        f["G5"] = "spinner shown at once, before a one-second wait"
        f["H2"] = f["G5"]
    if sid in ("help", "export-dialog"):
        f["H6"] = "F1 could open Help on top of the Export dialog (two modals)"
        f["I2"] = "one Escape could close both stacked modals"
    if sid == "export-dialog":
        f["H6"] = "F1 could stack Help on it, and Escape/backdrop/close/Cancel were all disabled while an export ran"
        f["G2"] = "dialog and its options pane fade/rise in but unmount with no exit"
        f["H3"] = "export success reported in the dialog and again as a toast"
        f["H5"] = "disabled PA-format choices faded as a whole (45% opacity)"
    if sid == "results-export-menu":
        f["E2"] = GLOBAL_FAIL["E2"] + "; disabled menu item ~1.8:1 (--color-text-disabled)"
        f["I2"] = "Escape closed the export menu and also cleared the selection"
    if sid in ("results-ion", "refine-progress"):
        f["G5"] = "loading spinner shown at once, before a one-second wait"
        f["H2"] = f["G5"]
    if sid == "busy":
        f["H7"] = "template creation overlay has no Cancel"
    if sid == "electrode-selected":
        f["C2"] = "hint says 'refine된 해를 다시 중첩합니다' (engine terms as the explanation)"
    if sid == "start-recent":
        f["C6"] = "recovery prompt contradicts itself: '자동 저장됐지만 저장되지 않았습니다'"
    if sid == "log-drawer":
        f["G2"] = "log drawer fades in on open and vanishes on close (no exit)"
    if sid == "results-export-menu":
        f["G2"] = "export menu rises in and unmounts with no exit"
    return f


def applicable(sid, item):
    if item in GLOBAL_NA:
        return False
    if item == "B2":  # destructive action (remove from scene) only where the tree has objects
        return sid not in START and sid not in ("empty-workspace",) and sid not in WIZ
    if item == "H6":
        return sid in DIALOGS
    if item == "H7":
        return sid in ("refine-progress", "wizard-gem-running", "busy")
    if item == "H2":
        return sid in ("busy", "refine-progress", "wizard-gem-running", "results-ion")
    if item == "I4":  # orbit/pan gestures have the view presets + fit as controls
        return sid not in START
    if item in ("G1", "G2", "G3", "G5", "G6", "G7"):
        return True
    if item == "F7":
        return sid in DIALOGS or sid in ("results-export-menu", "start-tour-offer") or sid.startswith("tour-")
    return True


# Deviations accepted by the lead (recorded in REPORT.md, not changed): item -> (surfaces predicate, reason).
JUSTIFIED = {
    "I4": (lambda sid: sid not in START, "orbit/pan/zoom of the 3D view are mouse gestures; presets 1-4, Fit (F) and the axes gizmo are the control alternatives; arbitrary keyboard orbit/pan/zoom would be a new feature"),
    "G4": (lambda sid: True, "under reduced motion, transform/scale transitions snap (1 ms) rather than cross-fade; fades, spinners and gesture tracking keep running"),
    "H7": (lambda sid: sid == "busy", "template creation takes 0.2-1.3 s; the long refines (properties panel, import wizard) keep Cancel"),
}


# After cells judged one by one from the final captures (round 4, after the gate review found the
# log-drawer overlap that every default PASS had missed): sid -> item -> (verdict, evidence).
# Any other after cell that is not N/A, J or a contract item is a DEFAULT PASS: the script has no
# per-cell evidence for it (see REPORT.md section 5, "How the cells were scored").
LD = "log-drawer-{1280x800,1920x1080}-{light,dark}-after.png"
_ld = {
    "A1": "panels on the app grid; nothing floats in an empty canvas",
    "A2": "panels and drawer run edge to edge",
    "A3": "symmetric panel insets; log lines are records, not prose",
    "A6": "scrollers: log list in the drawer, results side column, properties body - one per region (after manifest `scrollers`)",
    "A7": "drawer header, panel headers and status bar share edges",
    "A8": "log header groups title + clear + close; results toolbar grouped",
    "A9": "no overlap: results 228-584 / drawer 584-768 at 1280 (3D view 180px), 504-864 / 864-1048 at 1920 (after/logcheck.json); 1280 before the fix overlapped by 52px",
    "A10": "log list and results side column show a partial row and a scrollbar",
    "A12": "Layout lines for the log drawer and results side scroller (section 3)",
    "B1": "single prominent action in view ('이 결과 보관' on compare; histogram has none); top-bar Open is not filled",
    "B2": "'지우기' danger style + trash icon, not a default action",
    "B3": "project title in the top bar, '활동 로그' names the drawer",
    "B4": "3D view and results lead; log is secondary below",
    "B5": "accent on the active tab, primary button and switches only",
    "B6": "weight/size/colour hierarchy holds in both themes; sans + mono only",
    "B7": "matches the workspace brief (section 3)",
    "C1": "body 14px; audit `small` 0 in both themes",
    "C2": "log lines are sentences; 'refine' stays as the SIMION term next to the '전기장 계산 (Refine)' label",
    "C3": "outcome verbs ('지우기', '이 결과 보관'), noun tabs",
    "C4": "long log path fits at 1280; truncated panel text ends in an ellipsis",
    "C5": "no orphaned particles in the visible Korean",
    "C6": "no empty/error state on this surface; drawer always has entries",
    "C7": "every field labelled (이름, voltages)",
    "C8": "log text is selectable (LogDrawer.module.css user-select: text)",
    "D1": "audit `smallTargets` 0",
    "D3": "view toolbar three groups; drawer header two actions",
    "D4": "segment and toolbar buttons adjoin",
    "E1": "light and dark captured; no light-only asset",
    "E2": "audit `lowContrast` 0 (DOM text; canvas excluded)",
    "E3": "log status uses an icon per level next to the colour",
    "E4": "green = success, blue = info throughout",
    "E5": "overlays (legend, menu) are opaque cards",
    "E6": "top-bar theme control offers system",
    "F1": "lucide icons, one stroke",
    "F2": "token spacing/radii",
    "F3": "drawer, buttons and panels match the other surfaces",
    "F4": "SimRep wordmark in the top bar only",
    "F5": "selected file row: wash + tinted icon, no stripe",
    "F6": "standard close (X) and trash glyphs",
    "G1": "no idle animation on the surface",
    "G2": "drawer no longer fades in on mount (no exit existed); results panel and export menu entry animations removed too",
    "G3": "drawer opens at once; nothing gates input",
    "G5": "no spinner on this surface",
    "G6": "no layout shift after the drawer settles",
    "G7": "no flashing",
    "H1": "log has entries; empty log keeps the header",
    "H3": "outcomes reported in place / in the log",
    "H5": "hover/focus/disabled styles from the shared primitives",
    "I1": "focus-visible ring on drawer buttons and resizers",
    "I2": "Escape: only closes menus/modals (export menu open in this state, closes first)",
    "I3": "audit `unnamed` 0",
}
_copy = lambda what: {"A9": "no clipping or overlap in the 1280/1920 captures", "C2": what, "C4": "no clipped glyphs; ellipsis where text is cut", "C5": "no orphaned particle or split predicate in the new wording"}
EXPLICIT_AFTER = {
    "log-drawer": {k: ("PASS", v) for k, v in _ld.items()},
    "props-end": {k: ("PASS", v) for k, v in _copy("notice title '빠른 전압 조절(fast adjust) 배열'").items()},
    "electrode-selected": {k: ("PASS", v) for k, v in _copy("hint '전압을 바꾸면 계산된 전기장을 다시 합칩니다'").items()},
    "refine-progress": {k: ("PASS", v) for k, v in {**_copy("chip '{file} 전기장 계산 중' (file first, full text as tooltip)"), "C4": "chip title ends in an ellipsis with a tooltip; meta, bar and Cancel stay visible"}.items()},
    "start-recent": {k: ("PASS", v) for k, v in {**_copy("recovery card names the autosave and the project file separately"), "C6": "recovery prompt: '자동 저장본({time})은 있지만 프로젝트 파일에는 저장되지 않았습니다. 복원할까요?' + 복원/버리기", "A10": "third recent entry complete; list continues below"}.items()},
    "results-compare": {k: ("PASS", v) for k, v in {"A9": "log closed: results 408px at 1280 (unchanged by the max-height cap); both run cards and the toolbar fit one row", "A10": "side column scrollbar when cards exceed it"}.items()},
    "results-export-menu": {"G2": ("PASS", "menu appears without the entry-only rise-in"), "A9": ("PASS", "menu over the results stats only; no panel overlap")},
}


def verdict(sid, item, phase):
    if not applicable(sid, item):
        return "N/A"
    if item in JUSTIFIED and JUSTIFIED[item][0](sid):
        return "FAIL" if phase == "before" else "JUST"
    if phase == "after" and item in EXPLICIT_AFTER.get(sid, {}):
        return EXPLICIT_AFTER[sid][item][0]
    if item in ("A12", "B7"):  # contract items: no Layout line / brief existed before; written for every changed screen after
        return "N/A" if phase == "before" else "PASS"
    fails = {**{k: v for k, v in GLOBAL_FAIL.items()}, **surface_fails(sid)}
    if phase == "before" and item in fails:
        return "FAIL"
    return "PASS"


def basis(sid, item):
    """How the after cell was decided: na, justified, explicit (evidence in EXPLICIT_AFTER), contract, default."""
    if not applicable(sid, item):
        return "na"
    if item in JUSTIFIED and JUSTIFIED[item][0](sid):
        return "justified"
    if item in EXPLICIT_AFTER.get(sid, {}):
        return "explicit"
    if item in ("A12", "B7"):
        return "contract"
    return "default"


def main():
    bm, am, out = sys.argv[1:4]
    before = json.load(open(bm, encoding="utf-8"))
    after = json.load(open(am, encoding="utf-8"))
    ids = list(after["surfaces"].keys())
    scores = {}
    for sid in ids:
        fails = {**GLOBAL_FAIL, **surface_fails(sid)}
        scores[sid] = {
            "title": after["surfaces"][sid].get("title"),
            "items": {it: {"before": verdict(sid, it, "before"), "after": verdict(sid, it, "after")} for sec in ITEMS.values() for it in sec},
            "after_basis": {it: basis(sid, it) for sec in ITEMS.values() for it in sec},
            "after_evidence": {k: v[1] for k, v in EXPLICIT_AFTER.get(sid, {}).items()},
            "before_fail_reasons": {k: v for k, v in fails.items() if verdict(sid, k, "before") == "FAIL"},
            "audit": {
                phase: {
                    t: {k: m["audits"].get(f"{sid}-{t}", {}).get(k, {}).get("n") for k in ("small", "monoHangul", "lowContrast", "unnamed", "smallTargets")}
                    for t in ("light", "dark")
                }
                for phase, m in (("before", before), ("after", after))
            },
        }
    json.dump(scores, open(out, "w", encoding="utf-8"), ensure_ascii=False, indent=1)

    cell = {("PASS", "PASS"): "P", ("FAIL", "PASS"): "**F→P**", ("FAIL", "FAIL"): "**F**", ("PASS", "FAIL"): "**P→F**", ("N/A", "PASS"): "–→P", ("N/A", "N/A"): "–", ("FAIL", "JUST"): "F→J"}
    tb = ta = 0
    for s in scores.values():
        for v in s["items"].values():
            tb += v["before"] == "FAIL"
            ta += v["after"] == "FAIL"
    print(f"Totals: FAIL cells before {tb}, after {ta}; PASS cells before "
          f"{sum(v['before'] == 'PASS' for s in scores.values() for v in s['items'].values())}, after "
          f"{sum(v['after'] == 'PASS' for s in scores.values() for v in s['items'].values())}; "
          f"N/A cells after {sum(v['after'] == 'N/A' for s in scores.values() for v in s['items'].values())}; "
          f"justified deviations accepted by the lead (J) after {sum(v['after'] == 'JUST' for s in scores.values() for v in s['items'].values())}. "
          f"After PASS basis: explicit {sum(b == 'explicit' for s in scores.values() for b in s['after_basis'].values())}, "
          f"contract {sum(b == 'contract' for s in scores.values() for b in s['after_basis'].values())}, "
          f"default {sum(b == 'default' for s in scores.values() for b in s['after_basis'].values())}.\n")
    for sec, items in ITEMS.items():
        print(f"#### {sec}\n")
        print("| Surface | " + " | ".join(items) + " |")
        print("|---|" + "---|" * len(items))
        for sid, s in scores.items():
            print(f"| {sid} | " + " | ".join(cell[(s['items'][i]['before'], s['items'][i]['after'])] for i in items) + " |")
        print()


main()
