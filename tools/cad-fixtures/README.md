# cad-fixtures (dev tool, not shipped)

Writes the `simrep_cad::write_fixture` shapes (sphere, cylinder, ring, two-sphere `pair`) as
STEP **and IGES**, then reads every file back with `read_cad` and prints its parts, so a broken
writer or reader shows immediately. The IGES files are what the app's import wizard is tested
with (`.igs`, B-rep mode, millimetres).

Standalone cargo project (`[workspace]`), never built by the root `cargo build`. It needs the
OCCT SDK at build time and its DLLs at run time (`docs/dev/windows-occt.md`). On Windows, run it
inside the `env.bat` environment from a `/mnt/c` cwd:

```bash
MAN="$(wslpath -w tools/cad-fixtures/Cargo.toml)"
OCCT='C:\occt\opencascade-7.7.0'   # Windows path to the OCCT SDK (docs/dev/windows-occt.md)
cd /mnt/c && timeout 1800 cmd.exe /c "call $OCCT\env.bat vc143 win64 release >nul & set CARGO_TARGET_DIR=%USERPROFILE%\simrep-target-cad& set OCCT_INCLUDE_DIR=$OCCT\inc& set OCCT_LIB_DIR=$OCCT\win64\vc14\lib& %USERPROFILE%\.cargo\bin\cargo.exe run --release --manifest-path $MAN -- %USERPROFILE%\simrep-cad-fixtures\out" < /dev/null 2>&1 | tr -d '\r'
```

On Linux (system OCCT 7.6): `cargo run --release --manifest-path tools/cad-fixtures/Cargo.toml -- /tmp/cad-fixtures`.
