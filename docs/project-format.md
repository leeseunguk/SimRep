# SimRep project format (`.simrep`, format 2)

A desktop-app project is one JSON file `X.simrep` plus a data folder `X.simrep.d/` next to it.
The app owns both (`app/src/state/project.ts`, `projectFormat.ts`, `runs.ts`, `runFormat.ts`).
Arrays and meshes are referenced by path and never copied into the project.

## `X.simrep`

```json
{
  "format": "simrep-project",
  "version": 2,
  "savedAt": "2026-10-28T09:00:00.000Z",
  "arrays": [{
    "file": { "path": "einzel.pa0", "absolute": "C:\\...\\einzel.pa0" },
    "voltages": { "1": 0, "2": 80, "3": 0 },
    "source": { "mode": "beam", "count": 40, "...": "..." },
    "results": { "ions": 40, "tof": { "n": 40, "mean": 8.26, "...": 0 }, "...": "..." },
    "run": "20261028-090000123-a1b2"
  }],
  "meshes": [],
  "view": { "camera": null, "projection": "perspective", "grid": true },
  "runs": { "keep": 50, "capMb": 200 }
}
```

`file.path` is relative to the project file when possible; `absolute` is the fallback. Other
array fields (names, hidden electrodes, slice, output folder) are the editor's state.

Format 2 adds two fields:

- `arrays[].run`: the stored run the array showed when saved, or `null`.
- `runs`: history settings. `keep` is how many runs are kept, 50 by default. Pinned runs are
  kept on top of that. `capMb` is the size cap per run, 200 MB by default.

Format 1 files still open. They have no runs, so every array that had results is flown again.
That is what format 1 always did. A file with a newer `version` is refused.

## `X.simrep.d/runs/<run-id>/`

Every completed flight is added to the history, including the first flight after a template is
generated. Live re-flies during a slider drag count once: the run is recorded after the flights
have been quiet for 1.5 s. Untitled projects keep their runs in memory, and the first save
writes them out. "Save as" copies the stored runs into the new folder.

Run ids look like `YYYYMMDD-HHMMSSmmm-xxxx` (UTC time plus a random suffix), so they sort by
time. Each run folder has these files:

| File | Content |
| --- | --- |
| `run.json` | The record (below). It is always written last. |
| `results.bin` | Per-ion end states, SRR1 (below). Left out when it alone exceeds the cap. |
| `trajectories.bin` | The drawn trajectories as the viewer holds them (already decimated), SRT1. These are dropped first when the run exceeds the cap. |

`run.json`:

| Key | Meaning |
| --- | --- |
| `format`, `version` | `"simrep-run"`, `1` |
| `id`, `seq`, `label`, `note`, `pinned`, `createdAt` | identity and the user's edits (`label` `""` shows "Run N") |
| `array` | `{name, path}` |
| `settings.inputs` | `pa: {path, sha256}`, `voltages`, `source` (beam parameters, or `{kind: "file", path, sha256}`), `bounds`, `placement`, `durationUs`, `relativistic`, `precision` (requested), `collision`, `draw`, `surfaces`, `tolerances` (the integrator's: relative/position/velocity tolerance, initial/max step) |
| `settings.executed` | `device`, `precision`, `surfaces` (exact/grid), `deviceName`, `fallback` |
| `settings.simrepVersion`, `settings.computeMs` | app version, measured compute time |
| `settingsHash` | SHA-256 of the canonical JSON (sorted keys) of `settings.inputs`, with the PA path left out |
| `metrics` | `ions`, `arrivals`, `tof {n, min, max, mean, std}`, `keMean`, `splats` (surface key to count, e.g. `wall:+x`, `electrode:2`), `computeMs` |
| `header` | The flight header as the native core sent it |
| `stored` | `{results, trajectories, velocities, bytes, dropped: null \| "trajectories" \| "results"}` |
| `plots` | `histogram` (TOF of landed ions: `lo`, `hi`, `counts`) and `detectorMap` (density on the most-hit surface: `axes`, `x`, `y`, `nx`, `ny`, `counts`, row 0 = low y; square cells, 48 across the longer side, the shorter side at least a third of it so a flat pattern stays visible). Readers can show a run from these without its binary files. |

Binary files are little endian with a 16-byte header:

```
SRR1  u32 version=1  u32 n  u32 flags (bit 0: velocities)
f64[4n] x, y, z mm, TOF us | f64[n] KE eV | f64[3n] vx, vy, vz mm/us (if flag) | u32[n] surface index into header.surfaces

SRT1  u32 version=1  u32 n  u32 P
u32[n+1] point offsets per ion | f32[3P] x, y, z mm
```

## Opening

For each array with a `run`, the app hashes the PA file (SHA-256, native side) and rebuilds the
inputs from the current settings. The stored results are used only when both of these match:

- the PA sha256 equals `settings.inputs.pa.sha256`
- the settings hash equals `settingsHash`

Then the results panel says "Loaded saved results" and offers "Fly again". If either differs,
the run is marked "Out of date" in the history and the ions are flown again. The autosave
recovery file always flies again.

## Pruning

After each new run, the oldest unpinned runs beyond `keep` are deleted with their folders.
Pinned runs and runs an array is showing are never pruned.

## Data folder access (Tauri)

`project_dir_write` (raw body, `project-path` and `rel-path` headers), `project_dir_read`,
`project_dir_list` and `project_dir_delete` take the project file path and a `/`-separated path
inside `X.simrep.d/`. Absolute paths, drive prefixes, backslashes, `.`/`..` and empty segments
are refused. So is a symbolic link anywhere on the path, the data folder itself included.
`file_sha256` hashes a file. The TS side is `app/src/state/projectDir.ts`. Run access for other
features is `listRuns()` / `loadRun(id)` in `app/src/state/runs.ts`.
