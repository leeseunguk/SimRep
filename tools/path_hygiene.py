#!/usr/bin/env python3
"""Inspect tracked text and document/image metadata for private paths.

PDF inspection requires pypdf. Optional --ocr additionally requires Pillow and
Tesseract (the tesseract executable with English data on PATH). It scans tracked
PNGs in overlapping <=1000x800 tiles at 2x scale, reporting matches and bounding
boxes in original-image pixels, plus per-tile text/confidence for manual review.
Neutral account names below are documented placeholders/test fixtures. OCR is
lossy: a clean scan is not proof that pixels contain no private information;
low-confidence/empty tiles are reported explicitly. No image is modified.
"""
import argparse
import csv
import io
import json
import math
import os
import re
import struct
import subprocess
import zipfile
import zlib
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from typing import TypedDict

ROOT = Path(__file__).resolve().parents[1]
PATTERN = re.compile(r"(?:(?:(?-i:/home/)|/mnt/[a-z]/Users/|[A-Za-z]:[\\/]+Users[\\/]+)([A-Za-z0-9_. -]+)|/mnt/[a-z]/Private/[^\s'\"<>]+|Opera[_]test|lsu[0-9]+)", re.I)
PLACEHOLDERS = {"user", "you", "demo", "example", "example user", "test user", "runner", "runneradmin", "public", "default", "username"}
OCR_PATTERN = re.compile(
    r"(?:[A-Za-z]:[\\/]+)?Users[\\/]+[^\s<>]*|/home/[^\s<>]*|"
    + r"lsu[0-9]+|simrep-(?:gui-test|cad-fixtures)[^\s<>]*|"
    + r"[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}", re.I)


class OcrHit(TypedDict):
    match: str
    line: str
    bbox: list[int]
    placeholder: bool


class OcrTile(TypedDict):
    bbox: tuple[int, int, int, int]
    text: str
    mean_confidence: float | None
    review_reason: str | None


class OcrResult(TypedDict):
    size: list[int]
    scale: int
    hits: list[OcrHit]
    tiles: list[OcrTile]


def png_metadata(data):
    offset = 8
    records = []
    while offset + 12 <= len(data):
        length = struct.unpack_from(">I", data, offset)[0]
        kind = data[offset + 4:offset + 8]
        payload = data[offset + 8:offset + 8 + length]
        if kind == b"tEXt":
            records.append(payload.decode("latin1"))
        elif kind == b"zTXt":
            key, value = payload.split(b"\0", 1)
            records.append(key.decode("latin1") + " " + zlib.decompress(value[1:]).decode("latin1"))
        elif kind == b"iTXt":
            key, value = payload.split(b"\0", 1)
            compressed, method = value[:2]
            language, translated, text = value[2:].split(b"\0", 2)
            if compressed:
                text = zlib.decompress(text)
            records.append(key.decode("latin1") + " " + text.decode("utf8"))
        elif kind == b"eXIf":
            records.append(payload.decode("latin1"))
        offset += length + 12
    return records


def inspect(path):
    data = path.read_bytes()
    if data.startswith(b"\x89PNG\r\n\x1a\n"):
        return [("PNG metadata", text) for text in png_metadata(data)]
    if path.suffix.lower() == ".docx":
        with zipfile.ZipFile(path) as archive:
            return [(name, archive.read(name).decode("utf8")) for name in archive.namelist() if name.endswith(".xml")]
    if path.suffix.lower() == ".pdf":
        from pypdf import PdfReader
        reader = PdfReader(path)
        records = [("PDF metadata", str(reader.metadata))]
        if reader.xmp_metadata:
            records.append(("PDF XMP", reader.xmp_metadata.stream.get_data().decode("utf8", errors="replace")))
        records.extend((f"PDF page {i + 1}", page.extract_text() or "") for i, page in enumerate(reader.pages))
        return records
    if b"\0" not in data:
        return [("text", data.decode("utf8", errors="replace"))]
    return [("binary strings", data.decode("latin1")), ("binary UTF-16", data.decode("utf-16-le", errors="ignore"))]


def ocr_image(path: Path, scale: int = 2) -> OcrResult:
    """Return OCR candidates, original-pixel boxes, and reviewable tile output."""
    from PIL import Image
    hits: list[OcrHit] = []
    tiles: list[OcrTile] = []
    with Image.open(path) as image:
        # Split quad sheets first, then use balanced overlapping tiles so a
        # narrow final strip does not unnecessarily bisect sidebar text.
        if path.stem.endswith("-quad"):
            regions = [(x, y, X, Y) for y, Y in [(0, image.height // 2), (image.height // 2, image.height)]
                       for x, X in [(0, image.width // 2), (image.width // 2, image.width)]]
        else:
            regions = [(0, 0, image.width, image.height)]
        for left, top, right, bottom in regions:
            columns, rows = math.ceil((right - left) / 920), math.ceil((bottom - top) / 720)
            for row_index in range(rows):
                for column in range(columns):
                    x = max(left, left + (right - left) * column // columns - 40)
                    y = max(top, top + (bottom - top) * row_index // rows - 40)
                    X = min(right, left + (right - left) * (column + 1) // columns + 40)
                    Y = min(bottom, top + (bottom - top) * (row_index + 1) // rows + 40)
                    box = (x, y, X, Y)
                    tile = image.crop(box).convert("RGB")
                    tile = tile.resize((tile.width * scale, tile.height * scale))
                    buffer = io.BytesIO()
                    tile.save(buffer, format="PNG")
                    result = subprocess.run(
                        ["tesseract", "stdin", "stdout", "-l", "eng", "--psm", "11", "tsv"],
                        input=buffer.getvalue(), capture_output=True, check=True,
                        env={**os.environ, "OMP_THREAD_LIMIT": "1"}, timeout=120)
                    words_by_line: dict[tuple[str, str, str], list[dict[str, str]]] = {}
                    confidence: list[float] = []
                    for word in csv.DictReader(io.StringIO(result.stdout.decode()), delimiter="\t", quoting=csv.QUOTE_NONE):
                        if word["level"] != "5" or not word["text"].strip():
                            continue
                        confidence.append(float(word["conf"]))
                        key = (word["block_num"], word["par_num"], word["line_num"])
                        words_by_line.setdefault(key, []).append(word)
                    text_lines: list[str] = []
                    for words in words_by_line.values():
                        text = " ".join(word["text"] for word in words)
                        text_lines.append(text)
                        spans: list[tuple[int, int, dict[str, str]]] = []
                        offset = 0
                        for word in words:
                            spans.append((offset, offset + len(word["text"]), word))
                            offset += len(word["text"]) + 1
                        for match in OCR_PATTERN.finditer(text):
                            selected = [word for start, end, word in spans if end > match.start() and start < match.end()]
                            bbox = [x + min(int(w["left"]) for w in selected) // scale,
                                    y + min(int(w["top"]) for w in selected) // scale,
                                    x + math.ceil(max(int(w["left"]) + int(w["width"]) for w in selected) / scale),
                                    y + math.ceil(max(int(w["top"]) + int(w["height"]) for w in selected) / scale)]
                            account = re.search(r"(?:Users[\\/]+|/home/)([^\\/\s]+)", match.group(), re.I)
                            hits.append({"match": match.group(), "line": text, "bbox": bbox,
                                         "placeholder": bool(account and account.group(1).lower() in PLACEHOLDERS)})
                    mean = sum(confidence) / len(confidence) if confidence else None
                    tiles.append({"bbox": box, "text": "\n".join(text_lines), "mean_confidence": mean,
                                  "review_reason": "no recognized text (may be non-text artwork)" if mean is None else
                                  "mean English OCR confidence below 60" if mean < 60 else None})
    return {"size": [image.width, image.height], "scale": scale, "hits": hits, "tiles": tiles}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    _ = parser.add_argument("--ocr", action="store_true", help="also OCR tracked PNG pixels (requires Pillow and tesseract)")
    args = parser.parse_args()
    files = subprocess.check_output(["git", "ls-files", "-z"], cwd=ROOT).decode().split("\0")
    hits: list[dict[str, object]] = []
    counts: dict[str, int] = {}
    errors: list[dict[str, object]] = []
    for name in filter(None, files):
        path = ROOT / name
        counts[path.suffix.lower() or "no extension"] = counts.get(path.suffix.lower() or "no extension", 0) + 1
        try:
            for surface, text in inspect(path):
                for match in PATTERN.finditer(text):
                    if match.group(1) and match.group(1).strip().lower() in PLACEHOLDERS:
                        continue
                    line = text.count("\n", 0, match.start()) + 1
                    hits.append({"path": name, "surface": surface, "line": line, "match": match.group()})
        except Exception as error:
            errors.append({"path": name, "error": str(error)})
    report: dict[str, object] = {"files": sum(counts.values()), "types": counts, "hits": hits, "errors": errors,
                               "coverage": "Tracked text, SVG/JSON, PNG text/EXIF chunks, DOCX XML, PDF metadata/XMP and extracted text, binary ASCII/UTF-16 strings. Embedded document rasters are not OCRed."}
    if args.ocr:
        def scan(name: str) -> tuple[str, OcrResult | None, str | None]:
            try:
                return name, ocr_image(ROOT / name), None
            except Exception as error:
                return name, None, str(error)
        pngs = [name for name in files if name.lower().endswith(".png")]
        with ThreadPoolExecutor(max_workers=4) as pool:
            scanned = list(pool.map(scan, pngs))
        report["ocr"] = {"files": len(scanned),
                         "images": [{"path": name, **image} if image is not None else {"path": name, "error": error}
                                    for name, image, error in scanned],
                         "coverage": "English Tesseract, sparse text, 2x overlapping tiles; quad sheets split into quadrants. Review low-confidence tiles and non-English text manually."}
        for name, image, error in scanned:
            if image is None:
                errors.append({"path": name, "surface": "OCR", "error": error})
            else:
                hits.extend({"path": name, "surface": "PNG OCR", **hit}
                            for hit in image["hits"] if not hit["placeholder"])
    print(json.dumps(report, indent=2))
    return int(bool(hits or errors))


if __name__ == "__main__":
    raise SystemExit(main())
