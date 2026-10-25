# License texts for the OCCT third-party DLLs

`tools/win/stage-occt.ps1` bundles these next to the DLLs in the installer's `occt\licenses\`
folder. The OCCT 7.7.0 Windows archive (`opencascade-7.7.0-vc14-64.exe`) ships the DLLs of these
libraries without their license files, so the upstream texts of the same versions are kept here.
OCCT's own `LICENSE_LGPL_21.txt` / `OCCT_LGPL_EXCEPTION.txt` and openvr's `LICENSE` are copied
from the SDK at staging time.

| File | Library (bundled DLLs) | Source |
|---|---|---|
| `tbb-LICENSE-Apache-2.0.txt` | oneTBB 2021.5 (`tbb12.dll`, `tbbmalloc.dll`), Apache 2.0 | <https://raw.githubusercontent.com/oneapi-src/oneTBB/v2021.5.0/LICENSE.txt> |
| `freetype-LICENSE.TXT`, `freetype-FTL.TXT` | FreeType 2.5.5 (`freetype.dll`), FreeType License (dual FTL / GPLv2; used under the FTL) | <https://raw.githubusercontent.com/freetype/freetype/VER-2-5-5/docs/> |
| `freeimage-license-fi.txt` | FreeImage 3.17.0 (`FreeImage.dll`), FreeImage Public License 1.0 (dual FIPL / GPL; used under the FIPL) | <https://sourceforge.net/p/freeimage/svn/1909/tree/FreeImage/trunk/license-fi.txt> |
| `ffmpeg-LICENSE.md` | FFmpeg 3.3.4 (`avcodec-57.dll`, `avformat-57.dll`, `avutil-55.dll`, `swscale-4.dll`), LGPL 2.1 or later; the LGPL 2.1 text is copied from OCCT's `LICENSE_LGPL_21.txt` | <https://raw.githubusercontent.com/FFmpeg/FFmpeg/n3.3.4/LICENSE.md> |

Why these libraries are in the closure at all: SimRep only calls the STEP/IGES toolkits, but in
OCCT 7.7 `TKXDESTEP`/`TKXDEIGES` import `TKVCAF` → `TKV3d` → `TKService`, which imports
FreeImage, FreeType, FFmpeg and OpenVR. They are loaded but not used for import.
