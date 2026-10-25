"""Crop raw app captures (device pixels) into report figures. Pixels are copied, never resampled.

Single crop:   crop.py IN OUT x0 y0 x1 y1
Montage:       crop.py --stack OUT IN1:x0,y0,x1,y1[:label] IN2:... 
               Crops are stacked vertically; an optional label is drawn in a thin header row
               above each crop (Malgun Gothic), e.g. "1  가져오기 마법사 — 부품".
Side by side:  crop.py --row OUT IN1:x0,y0,x1,y1[:label] IN2:...
"""
import sys

from PIL import Image, ImageDraw, ImageFont

GAP = 24
BG = (255, 255, 255)
INK = (31, 41, 51)
FONT = "/mnt/c/Windows/Fonts/malgunbd.ttf"
LABEL_H = 56
LABEL_PX = 30


def parse(spec):
    parts = spec.split(":")
    # Windows-free paths only (no drive colon); label is optional and may itself contain no colon.
    path, box = parts[0], parts[1]
    label = parts[2] if len(parts) > 2 else None
    im = Image.open(path).convert("RGB").crop(tuple(int(v) for v in box.split(",")))
    if label:
        font = ImageFont.truetype(FONT, LABEL_PX)
        head = Image.new("RGB", (im.width, LABEL_H), BG)
        ImageDraw.Draw(head).text((4, (LABEL_H - LABEL_PX) // 2 - 2), label, font=font, fill=INK)
        both = Image.new("RGB", (im.width, im.height + LABEL_H), BG)
        both.paste(head, (0, 0))
        both.paste(im, (0, LABEL_H))
        im = both
    return im


if sys.argv[1] in ("--stack", "--row"):
    out = sys.argv[2]
    tiles = [parse(s) for s in sys.argv[3:]]
    if sys.argv[1] == "--stack":
        w, h = max(t.width for t in tiles), sum(t.height for t in tiles) + GAP * (len(tiles) - 1)
    else:
        w, h = sum(t.width for t in tiles) + GAP * (len(tiles) - 1), max(t.height for t in tiles)
    canvas = Image.new("RGB", (w, h), BG)
    pos = 0
    for t in tiles:
        canvas.paste(t, (0, pos) if sys.argv[1] == "--stack" else (pos, 0))
        pos += (t.height if sys.argv[1] == "--stack" else t.width) + GAP
    canvas.save(out)
    print("wrote", out, canvas.size)
else:
    src, out, *box = sys.argv[1:]
    im = Image.open(src).convert("RGB").crop(tuple(int(v) for v in box))
    im.save(out)
    print("wrote", out, im.size)
