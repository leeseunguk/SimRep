"""Contact sheets and before/after pairs for the visual QA review.

usage: python sheets.py sheet <dir> <phase> <out_dir>          light|dark side by side per surface and size
       python sheets.py pair <before_dir> <after_dir> <out_dir>  before|after side by side per capture
       python sheets.py quad <before_dir> <after_dir> <out_dir>  per surface and size: before|after x light/dark grid
"""
import os
import sys

from PIL import Image, ImageDraw


def side_by_side(paths, captions, out, width=int(os.environ.get("SHEET_W", "1300"))):
    ims = [Image.open(p).convert("RGB") for p in paths]
    ims = [im.resize((width, round(im.height * width / im.width)), Image.LANCZOS) for im in ims]
    cap = 28
    sheet = Image.new("RGB", (width * len(ims) + 10 * (len(ims) - 1), max(i.height for i in ims) + cap), "white")
    d = ImageDraw.Draw(sheet)
    for k, (im, c) in enumerate(zip(ims, captions)):
        x = k * (width + 10)
        d.text((x + 6, 6), c, fill="black")
        sheet.paste(im, (x, cap))
    sheet.save(out)


def main():
    mode, a, b, out = sys.argv[1:5]
    os.makedirs(out, exist_ok=True)
    if mode == "sheet":
        files = sorted(f for f in os.listdir(a) if f.endswith(f"-light-{b}.png"))
        for f in files:
            stem = f[: -len(f"-light-{b}.png")]
            dark = os.path.join(a, f"{stem}-dark-{b}.png")
            side_by_side([os.path.join(a, f), dark], [f"{stem} light", f"{stem} dark"], os.path.join(out, f"{stem}-{b}.png"))
    elif mode == "quad":
        for f in sorted(os.listdir(a)):
            if not f.endswith("-light-before.png"):
                continue
            stem = f[: -len("-light-before.png")]
            rows = []
            for theme in ("light", "dark"):
                pb, pa = os.path.join(a, f"{stem}-{theme}-before.png"), os.path.join(b, f"{stem}-{theme}-after.png")
                if not os.path.exists(pa):
                    break
                tmp = os.path.join(out, f".{stem}-{theme}.png")
                side_by_side([pb, pa], [f"{stem} {theme} BEFORE", f"{stem} {theme} AFTER"], tmp)
                rows.append(tmp)
            if len(rows) == 2:
                ims = [Image.open(r) for r in rows]
                sheet = Image.new("RGB", (ims[0].width, ims[0].height + ims[1].height + 10), "white")
                sheet.paste(ims[0], (0, 0))
                sheet.paste(ims[1], (0, ims[0].height + 10))
                sheet.save(os.path.join(out, f"{stem}-quad.png"))
            for r in rows:
                os.remove(r)
    else:
        for f in sorted(os.listdir(a)):
            if not f.endswith("-before.png"):
                continue
            after = os.path.join(b, f.replace("-before.png", "-after.png"))
            if os.path.exists(after):
                stem = f[: -len("-before.png")]
                side_by_side([os.path.join(a, f), after], [f"{stem} BEFORE", f"{stem} AFTER"], os.path.join(out, f"{stem}-pair.png"))


main()
