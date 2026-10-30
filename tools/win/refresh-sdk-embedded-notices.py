"""Maintainer-only: freeze embedded SDK notices from a verified upstream archive."""
import hashlib
import json
import pathlib
import re
import sys
import urllib.request
import zipfile

HERE = pathlib.Path(__file__).resolve().parent
DEST = HERE / 'notice-sources'
ARCHIVE_SHA256 = 'fbfc65e39b3d4e2cb108c4ffa8c41fd02c07d4d436c594fff8dab1a6d5297f89'
UPSTREAM = 'https://sourceforge.net/projects/freeimage/files/Source%20Distribution/3.17.0/FreeImage3170.zip'
archive = pathlib.Path(sys.argv[1])
if hashlib.sha256(archive.read_bytes()).hexdigest() != ARCHIVE_SHA256:
    raise ValueError('FreeImage 3.17.0 source archive checksum mismatch')
records = []
for name, version, license_id, file in [
    ('FreeType BDF driver', '2.5.5', 'MIT', 'src/bdf/README'),
    ('FreeType PCF driver', '2.5.5', 'MIT', 'src/pcf/README'),
    ('zlib (FreeType gzip module)', '1.1.4', 'Zlib', 'src/gzip/zlib.h'),
]:
    url = f'https://raw.githubusercontent.com/freetype/freetype/VER-2-5-5/{file}'
    with urllib.request.urlopen(url, timeout=30) as response:
        text = response.read().decode('utf-8')
    if file.endswith('zlib.h'):
        text = text[:text.index('*/') + 2] + '\n'
    records.append(dict(kind='sdk-embedded', name=name, version=version, license=license_id,
                        location=f'freetype.dll ({name})', texts=[dict(source=url, text=text)]))

with zipfile.ZipFile(archive) as package:
    for name, version, license_id, prefix in [
        ('IJG JPEG', '9a', 'IJG', 'LibJPEG/'),
        ('JPEG XR / jxrlib', '1.1 (FreeImage patch 2014-01-31)', 'BSD-2-Clause', 'LibJXR/'),
        ('OpenJPEG', '2.1.0 (FreeImage SVN patch 2748)', 'BSD-2-Clause', 'LibOpenJPEG/'),
        ('libpng', '1.6.16', 'Libpng', 'LibPNG/'),
        ('LibRaw, including dcraw, DCB/FBDD and X3F', '0.17.0-Alpha1', '(CDDL-1.0 OR LGPL-2.1-only) AND BSD-3-Clause', 'LibRawLite/'),
        ('libtiff', '4.0.4beta (20150126)', 'libtiff', 'LibTIFF4/'),
        ('libwebp', '0.4.2 (FreeImage patch 2015-03-03)', 'BSD-3-Clause', 'LibWebP/'),
        ('OpenEXR', '2.2.0', 'BSD-3-Clause', 'OpenEXR/'),
        ('IlmBase', '2.2.0', 'BSD-3-Clause', 'OpenEXR/Copyrights/ilmbase/'),
        ('zlib (FreeImage)', '1.2.8', 'Zlib', 'ZLib/'),
    ]:
        texts = []
        seen = set()
        for file in sorted(package.namelist()):
            if not file.startswith(f'FreeImage/Source/{prefix}') or file.endswith('/'):
                continue
            data = package.read(file)
            try:
                content = data.decode('utf-8')
            except UnicodeDecodeError:
                content = data.decode('cp1252')
            basename = pathlib.PurePosixPath(file).name
            if re.match(r'^(LICENSE|LICENCE|COPYING|COPYRIGHT|NOTICE|README|AUTHORS|PATENTS)([._-].*)?$', basename, re.I):
                parts = [content]
            elif pathlib.PurePosixPath(file).suffix.lower() in ('.h', '.hpp', '.c', '.cc', '.cpp', '.cxx'):
                parts = [match.group() for match in re.finditer(r'/\*[\s\S]*?\*/|(?:^[ \t]*//[^\n]*(?:\n|$))+', content, re.M)
                         if re.search(r'copyright|permission.*grant|license', match.group(), re.I)]
            else:
                parts = []
            for text in parts:
                if not text.strip():
                    continue
                digest = hashlib.sha256(text.encode()).hexdigest()
                if digest in seen:
                    continue
                seen.add(digest)
                texts.append(dict(source=f'{UPSTREAM} (SHA256 {ARCHIVE_SHA256})!/{file}', text=text))
        if not texts or not any(len(item['text']) > 500 for item in texts):
            raise ValueError(f'No complete embedded licence texts: {name}')
        records.append(dict(kind='sdk-embedded', name=name, version=version, license=license_id,
                            location=f'FreeImage.dll ({name})', texts=texts))
        print(f'{name} {version}: {len(texts)} unique notice/copyright texts')
output = json.dumps(records, indent=2, ensure_ascii=False) + '\n'
file = DEST / 'sdk-embedded.json'
file.write_text(output, encoding='utf-8')
manifest_path = DEST / 'manifest.json'
manifest = json.loads(manifest_path.read_text())
manifest['sdk-embedded'] = dict(url=UPSTREAM, file=file.name, sha256=hashlib.sha256(output.encode()).hexdigest())
manifest_path.write_text(json.dumps(manifest, indent=2) + '\n')
print(f'SDK embedded components: {len(records)}; bytes: {len(output.encode())}')
