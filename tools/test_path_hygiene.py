"""Run with python -m unittest discover -s tools -p 'test_path_hygiene.py'.

Integration test of the optional OCR dependency: requires Pillow, Tesseract and
DejaVu Sans (or set OCR_TEST_FONT to an equivalent TrueType font).
"""
import os
import tempfile
import unittest
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

from tools.path_hygiene import ocr_image


class OcrTest(unittest.TestCase):
    def test_fake_path_and_original_pixel_box(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "synthetic.png"
            image = Image.new("RGB", (640, 100), "white")
            font = ImageFont.truetype(os.environ.get(
                "OCR_TEST_FONT", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"), 22)
            text = "C:" + chr(92) + "Users" + chr(92) + "qa-person" + chr(92) + "result.pa0"
            draw = ImageDraw.Draw(image)
            draw.text((20, 25), text, fill="black", font=font)
            image.save(path)
            result = ocr_image(path)
            self.assertEqual(result["size"], [640, 100])
            candidates = [hit for hit in result["hits"] if "qa-person" in hit["match"]]
            self.assertTrue(candidates, result)
            self.assertFalse(candidates[0]["placeholder"])
            x, y, X, Y = candidates[0]["bbox"]
            ink = draw.textbbox((20, 25), text, font=font)
            self.assertLessEqual(abs(x - ink[0]), 2)
            self.assertLessEqual(abs(y - ink[1]), 2)
            self.assertLessEqual(abs(X - ink[2]), 2)
            self.assertLessEqual(abs(Y - ink[3]), 2)

    def test_quad_scans_all_quadrants_and_preserves_offsets(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "synthetic-quad.png"
            image = Image.new("RGB", (1280, 200), "white")
            font = ImageFont.truetype(os.environ.get(
                "OCR_TEST_FONT", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"), 22)
            draw = ImageDraw.Draw(image)
            for x, y in [(20, 25), (660, 25), (20, 125), (660, 125)]:
                draw.text((x, y), "/home/" + "qa-person/result.pa0", fill="black", font=font)
            image.save(path)
            result = ocr_image(path)
            self.assertEqual(len(result["tiles"]), 4)
            boxes = [hit["bbox"] for hit in result["hits"] if "qa-person" in hit["match"]]
            self.assertEqual(len(boxes), 4, result)
            self.assertEqual({(box[0] // 640, box[1] // 100) for box in boxes},
                             {(0, 0), (1, 0), (0, 1), (1, 1)})


if __name__ == "__main__":
    _ = unittest.main()
