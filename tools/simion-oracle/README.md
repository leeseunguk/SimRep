# simion-oracle

`run_oracle.py` runs the installed SIMION (`C:\Program Files\SIMION-2024\simion.exe`,
version `SIMION 2024-09-26-8.2.1.0`) headless from WSL to produce reference results.
SIMION is used as a black box only.

`compare_pa.py left.paN [right.paN]` independently decodes binary PA values
using only the Python standard library. With one file it prints header,
electrode-voltage, and PA0-trailer statistics. With two it reports maximum
and RMS potential differences over **all nodes**, and electrode-mask mismatches.
PA0 comparisons require equal fast-adjust voltage settings; PA1..PAn and PA_
use SIMION's 10000 V reference convention. See `bench/refine-real.md` for the
real-input CPU/Windows Vulkan comparison and timing protocol.

## Usage

```sh
python3 tools/simion-oracle/run_oracle.py <example> [--refine] [--fly IOB] [--rec REC] \
    [--simion PATH] [--oracle-root DIR] [--timeout SEC]

python3 tools/simion-oracle/run_oracle.py test_plane  --refine --fly einzel.iob
python3 tools/simion-oracle/run_oracle.py test_sphere --refine --fly surf.iob
python3 tools/simion-oracle/run_oracle.py einzel      --refine --fly einzel.iob
```

- `<example>` is a folder under `SIMION-2024/examples/` (nested names such as
  `performance_tests/5_planar_pa_tests` work).
- The examples folder is read-only, so the example is first copied fresh (the old copy is
  deleted) to `/mnt/c/Users/lsu10/simrep-oracle/<example>/` (`C:\Users\lsu10\simrep-oracle\...`).
  The copy does not keep file metadata, so the read-only attribute is not carried over.
- `--refine` refines every `*.pa#` in the example. If there is no `.pa#`, it refines every `*.pa` instead.
- `--fly IOB` flies the IOB with data recording on. The output goes to
  `<iob stem>.oracle_recording.txt`. The recording definition is `--rec`, or `<iob stem>.rec` when
  that file exists. `--rec` is passed to SIMION as given, so it can be relative to the work dir or an
  absolute Windows path.
- Every SIMION call is a direct foreground `simion.exe` call (no `cmd.exe` wrapper) with
  `--nogui --noprompt` and a timeout (default 1800 s). On timeout, every `simion.exe` started since
  the call began is tree-killed with `taskkill /T /F`, because killing the WSL wrapper alone does not
  stop the Windows process.
- The script exits with 0 only if every SIMION step exits with 0.

## oracle.json

Written to `<work dir>/oracle.json`:

| field | meaning |
| --- | --- |
| `example`, `source_dir`, `work_dir` | example name, read-only source folder, Windows working copy |
| `simion_exe`, `simion_version` | executable and the output of `--nogui --version` |
| `created_utc` | run timestamp |
| `runs[]` | one entry per SIMION call: `step` (`refine`/`fly`), `target`, `argv`, `command_line`, `cwd`, `exit_code` (`null` on timeout), `timed_out`, `seconds` (wall clock), `output_files` (files created or changed in the work dir by this call), `stdout`, `stderr`. Fly entries also carry `recording` (.rec used) and `recording_output`. |

## Discovered SIMION CLI

`simion.exe [options] [command] [commandoptions]`. Use `--nogui`. Without it, even
`simion.exe --help` opens the GUI and never returns: it was killed after a 60 s timeout. With
`--nogui`, output goes to stdout, and each line carries a tag prefix: `help,`, `status,`,
`status2,` or `error,`. Lua `print()` output from user programs is printed without a tag.

Global options (`--nogui --help`):

```
--help                      Display this help message
--version                   Display SIMION version and copyright info.
--nogui                     Disable GUI.  Command-line interface only.
--quiet                     Disable printing of status messages.
--num-threads arg           Max number of CPU threads to use
--noprompt                  Disable prompts (usually used with --nogui).
--reserved-memory arg       Number of bytes of memory to reserve for PAs.
--editor arg                Default text editor (same as SIMION_EDITOR).
--default-num-points arg    Default number of PA grid point to allocate.
--default-num-particles arg Default number of particles to allocate.
--temp-path arg             Default directory for storing temporary files.
--lua arg                   Lua statements to execute.
command    Command name (gem2pa, refine, fastadj, fly, lua) or IOB or PA filename
```

`refine [options] pafile`:

```
--max-iterations arg           Maximum number of iterations.
--convergence arg              Convergence objective in volts (or mags).
--max-overrelaxation arg       Maximum overrrelaxation value (advanced).
--historical-memory-factor arg Historical memory factor (advanced).
--skipped-point arg            Enable skipped point refine (default true)
--resume arg                   Resume refine 1=yes, 0=no (default prompt)
```

Refining `x.pa#` writes `x.pa1..x.paN` (one solution per electrode) and `x.pa0`.

`fly [options] iobfile`:

```
--trajectory-quality arg        trajectory quality control value
--recording arg                 data recording file (.REC)
--recording-enable arg          enable data recording (1=yes,0=no)
--recording-output arg          data recording output file
--grouped arg                   fly particles as grouped (1=yes,0=no)
--omit-relativistic-effects arg omit relativistic effects (1=yes,0=no)
--markers arg                   time markers (microseconds)
--marker-color arg              marker color 0-15
--repulsion arg                 repulsion type (none, beam, coulomb, factor)
--repulsion-amount arg          repulsion amount
--retain-trajectories arg       retain trajectories as file on disk
--rerun arg                     rerun enable (1=yes,0=no)
--restore-potentials arg        restore potentials (1=yes,0=no)
--particles arg                 particles file (.FLY or .ION)
--adjustable arg                set value of adjustable variable, name=value
--programs arg                  enable user programs (1=yes,0=no)
--remove-pas arg                remove all PAs from RAM (0=never, 1=enter, 2=exit, 3=enter and exit)
--disk arg                      allow refining PA# arrays to disk (0=no - use RAM, 1=yes - default)
--refine-convergence arg        Refine convergence objective in volts (or mags) if arrays to be refined.
```

`fastadj [options] pafile potentials`: `pafile` is a PA0 (or PA#) path and `potentials` looks
like `'1=200,2=300'`.

`gem2pa [options] gemfile [pafile]`: `--nx --ny --nz`, `--symmetry` (`cylindrical`, `planar`,
`planar[xyz]`), `--potential_type` (`electric`/`magnetic`), `--ng`, `--dx_mm --dy_mm --dz_mm`,
`--x --y --z`, `--scale --scalex --scaley --scalez`, `--azimuth --elevation --rotation`, `--merge`.

`lua [options] [script [args]]`: standard `lua` options (`-e stat`, `-l name`, `-i`, `-v`, `--`,
`-`). A Lua failure prints `error,FAIL: Lua returned failure exit code.` and exits with 1.
Examples in the install use `simion.exe --nogui lua pa2text.lua <file>` and
`simion.exe --nogui fly --adjustable master=0 tune_series.iob`.

Recording behaviour seen so far:
- The recording output file is appended to, not replaced. Each fly starts with
  `------ Begin Next Fly'm ------`. Because the work dir is copied fresh, each oracle run starts
  with no recording file.
- The file contains Lua `print()` output and the recorded fields, plus a
  `Begin Fly'm (<local date>)` header when a .rec is in use. Ignore that header when diffing.
- `einzel` ships no `.rec`, so its recording file is header-only (32 bytes). The per-ion splat
  data for einzel is in the fly `stdout`. To record fields there, author a `.rec` in the SIMION GUI
  (the format is binary) and pass it with `--rec`. Reusing `test_plane/einzel.rec` only adds the header.

## Reference runs (2026-10-23)

| example | step | command | exit | seconds | outputs |
| --- | --- | --- | --- | --- | --- |
| test_plane | refine | `refine einzel.pa#` | 0 | 0.166 | einzel.pa0..pa3 |
| test_plane | fly | `fly ... --recording einzel.rec einzel.iob` | 0 | 0.069 | einzel.oracle_recording.txt (2049 B, 11 splats) |
| test_sphere | refine | `refine surf.pa` | 0 | 0.064 | surf.pa (empty array, 1 iteration) |
| test_sphere | fly | `fly ... surf.iob` | 0 | 0.066 | surf.oracle_recording.txt (1723 B, 50 crossings at tof 5.25 us) |
| einzel | refine | `refine einzel.pa#` | 0 | 0.169 | einzel.pa0..pa3 |
| einzel | fly | `fly ... einzel.iob` | 0 | 0.067 | einzel.oracle_recording.txt (header only, 11 splats in stdout) |
