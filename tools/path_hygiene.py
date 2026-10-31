#!/usr/bin/env python3
"""Inspect tracked text and embedded document/image metadata for private paths.

PDF inspection requires pypdf; rasterized screenshot text needs separate visual
inspection. Neutral account names below are documented placeholders/test fixtures.
"""
import json
import re
import struct
import subprocess
import zipfile
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PATTERN = re.compile(r"(?:(?:(?-i:/home/)|/mnt/[a-z]/Users/|[A-Za-z]:[\\/]+Users[\\/]+)([A-Za-z0-9_. -]+)|/mnt/[a-z]/Private/[^\s'\"<>]+|Opera[_]test|lsu[0-9]+)", re.I)
PLACEHOLDERS = {"user", "you", "demo", "example", "example user", "test user", "runner", "runneradmin", "public", "default", "username"}


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


def main():
    files = subprocess.check_output(["git", "ls-files", "-z"], cwd=ROOT).decode().split("\0")
    hits = []
    counts = {}
    errors = []
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
    print(json.dumps({"files": sum(counts.values()), "types": counts, "hits": hits, "errors": errors, "coverage": "Tracked text, SVG/JSON, PNG text/EXIF chunks, DOCX XML, PDF metadata/XMP and extracted text, binary ASCII/UTF-16 strings. Visible raster pixels require separate OCR/inspection."}, indent=2))
    return int(bool(hits or errors))


if __name__ == "__main__":
    raise SystemExit(main())
