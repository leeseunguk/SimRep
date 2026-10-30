"""Maintainer helper: extract the complete Microsoft runtime licence DOCX as text."""
import sys
import zipfile
import xml.etree.ElementTree as ET

with zipfile.ZipFile(sys.argv[1]) as archive:
    document = ET.fromstring(archive.read('word/document.xml'))
ns = {'w': 'http://schemas.openxmlformats.org/wordprocessingml/2006/main'}
for paragraph in document.findall('.//w:p', ns):
    print(''.join(element.text or '' for element in paragraph.findall('.//w:t', ns)))
