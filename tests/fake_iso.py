"""Build the one fixed test ISO and its plaintext reference."""

import mmap
import shutil
import struct
import subprocess
import tempfile
from pathlib import Path

DIRECTORY = Path(__file__).resolve().parent
ISO = DIRECTORY / "Fake.iso"
EXPECTED = DIRECTORY / "Fake.expected.iso"
KEY = (DIRECTORY / "keys/Fake.dkey").read_text().strip()
SECTOR = 2048


mkisofs = shutil.which("genisoimage") or shutil.which("mkisofs")
if not mkisofs or not shutil.which("openssl"):
    raise SystemExit("Install genisoimage (or mkisofs) and OpenSSL first.")

marker = b"PS3DEC_ENCRYPTED_TEST_PAYLOAD\x00"
payload = marker + bytes(i % 251 for i in range(4 * SECTOR - len(marker)))
with tempfile.TemporaryDirectory() as directory:
    tree = Path(directory)
    usrdir = tree / "PS3_GAME" / "USRDIR"
    usrdir.mkdir(parents=True)
    (usrdir / "PAYLOAD.BIN").write_bytes(payload)
    (tree / "PS3_GAME" / "CONTROL.TXT").write_text("This stays plaintext.\n")
    with (usrdir / "PADDING.BIN").open("wb") as padding:
        padding.truncate(64 * 1024 * 1024)
    subprocess.run(
        [mkisofs, "-quiet", "-udf", "-iso-level", "3", "-V", "PS3DEC_TEST",
         "-o", str(EXPECTED), str(tree)], check=True, capture_output=True,
    )

with EXPECTED.open("r+b") as disc:
    with mmap.mmap(disc.fileno(), 0, access=mmap.ACCESS_READ) as image:
        offset = image.find(payload)
        assert offset > 0 and offset % SECTOR == 0
        assert image.find(payload, offset + 1) == -1
        assert image[16 * SECTOR + 1:16 * SECTOR + 6] == b"CD001"
        assert b"NSR02" in image[16 * SECTOR:32 * SECTOR]
        last_sector = len(image) // SECTOR - 1
    first = offset // SECTOR
    end = first + len(payload) // SECTOR - 1
    assert end < last_sector
    header = bytearray(SECTOR)
    struct.pack_into(">6I", header, 0, 2, 0, 0, first - 1, end + 1, last_sector)
    disc.seek(0)
    disc.write(header)
    disc.write(b"PlayStation3" + bytes(4) + b"TEST-00001".ljust(32, b" ") + bytes(SECTOR - 48))

shutil.copyfile(EXPECTED, ISO)
with EXPECTED.open("rb") as plain, ISO.open("r+b") as disc:
    for sector in range(first, end + 1):
        plain.seek(sector * SECTOR)
        encrypted = subprocess.run(
            ["openssl", "enc", "-aes-128-cbc", "-nopad", "-K", KEY,
             "-iv", sector.to_bytes(16, "big").hex()],
            input=plain.read(SECTOR), check=True, capture_output=True,
        ).stdout
        disc.seek(sector * SECTOR)
        disc.write(encrypted)
print("Generated tests/Fake.iso and its plaintext reference")
