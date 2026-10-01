from pathlib import Path
import struct
import tempfile
import unittest

from perf_sweep import landscape, output_digest, record, subrecord, texture


class OutputDigestTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="merged-lands-digest-test-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        (self.root / "output").mkdir()
        (self.root / "Conflicts").mkdir()
        self.output = self.root / "output" / "Merged Lands.omwaddon"

    @staticmethod
    def plugin(timestamp=1750000000):
        description = f"Merges terrain. Generated at {timestamp} UTC.".encode()
        header = subrecord(
            b"HEDR", struct.pack("<fI32s256sI", 1.3, 0, b"perf test", description, 2)
        )
        header += subrecord(b"MAST", b"Fixture.esm\0")
        header += subrecord(b"DATA", struct.pack("<Q", 1234))
        return record(b"TES3", header) + texture(0) + landscape(0, 0, 1, 2, 8)

    def digest(self, data):
        self.output.write_bytes(data)
        return output_digest(self.root)

    def test_same_width_generation_timestamp_alone_is_ignored(self):
        expected = self.digest(self.plugin())
        for timestamp in (1750000001, 1790000000, 1999999999):
            with self.subTest(timestamp=timestamp):
                self.assertEqual(self.digest(self.plugin(timestamp)), expected)

    def test_different_timestamp_width_preserves_description_layout_change(self):
        # A sweep runs under one real clock, so timestamp width stays fixed. Preserve
        # offsets and all padding bytes instead of accepting arbitrary layout changes.
        expected = self.digest(self.plugin())
        self.assertNotEqual(self.digest(self.plugin(1)), expected)

    def test_vanilla_output_normalizes_clock_and_preserves_terrain(self):
        vanilla = self.output.with_name("Merged Lands.esp")
        vanilla.write_bytes(self.plugin())
        expected = output_digest(self.root, vanilla.name)
        vanilla.write_bytes(self.plugin(1750000001))
        self.assertEqual(output_digest(self.root, vanilla.name), expected)
        changed = bytearray(vanilla.read_bytes())
        changed[-1] ^= 1
        vanilla.write_bytes(changed)
        self.assertNotEqual(output_digest(self.root, vanilla.name), expected)

    def test_every_generated_record_subfield_affects_digest(self):
        original = self.plugin()
        expected = self.digest(original)
        fields = [
            ("header.version", 24), ("header.type", 28), ("header.author", 32),
            ("header.description", 64), ("header.record_count", 320),
        ]
        position = 0
        while position < len(original):
            tag = original[position:position + 4].decode()
            payload_size = struct.unpack_from("<I", original, position + 4)[0]
            fields.append((f"{tag}.flags", position + 12))
            subposition = position + 16
            end = subposition + payload_size
            while subposition < end:
                subtag = original[subposition:subposition + 4].decode()
                size = struct.unpack_from("<I", original, subposition + 4)[0]
                if subtag != "HEDR":
                    fields.append((f"{tag}.{subtag}", subposition + 8))
                subposition += 8 + size
            position = end
        for label, offset in fields:
            with self.subTest(field=label):
                changed = bytearray(original)
                changed[offset] ^= 1
                self.assertNotEqual(self.digest(changed), expected)

    def test_description_bytes_after_first_nul_are_preserved(self):
        original = self.plugin()
        expected = self.digest(original)
        changed = bytearray(original)
        changed[319] = 0xFF
        self.assertNotEqual(self.digest(changed), expected)

    def test_timestamp_like_text_in_other_records_is_preserved(self):
        original = self.plugin() + record(b"LTEX", subrecord(b"NAME", b"Generated at 1 UTC.\0"))
        changed = original.replace(b"Generated at 1 UTC.\0", b"Generated at 2 UTC.\0")
        self.assertNotEqual(self.digest(original), self.digest(changed))

    def test_conflict_image_content_and_name_affect_digest(self):
        original = self.plugin()
        image = self.root / "Conflicts" / "cell.png"
        image.write_bytes(b"image bytes")
        expected = self.digest(original)
        image.write_bytes(b"changed image bytes")
        self.assertNotEqual(self.digest(original), expected)
        image.write_bytes(b"image bytes")
        image.rename(image.with_name("renamed.png"))
        self.assertNotEqual(self.digest(original), expected)


if __name__ == "__main__":
    unittest.main()
