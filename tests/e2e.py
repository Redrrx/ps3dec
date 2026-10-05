"""Run the actual executable against one fixed encrypted ISO."""

import hashlib
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BINARY = ROOT / "target" / "debug" / ("ps3dec.exe" if os.name == "nt" else "ps3dec")
TESTS = ROOT / "tests"


def digest(path):
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").digest()


def run(directory, *args, succeeds=True):
    result = subprocess.run(
        [str(BINARY), *map(str, args)], cwd=directory, stdin=subprocess.DEVNULL,
        capture_output=True, text=True, timeout=30, check=False,
    )
    assert (result.returncode == 0) == succeeds, result.stdout + result.stderr
    if not succeeds:
        assert "Job done" not in result.stderr, result.stderr
    return result.stderr


def check_output(output):
    assert digest(output) == EXPECTED_HASH, f"wrong output bytes: {output}"
    assert not Path(str(output) + ".part").exists()
    record = json.loads(Path(str(output) + ".resume").read_text())
    assert record["next_offset"] == EXPECTED.stat().st_size
    assert record["key"] != KEY


def single_file(directory):
    run(directory, "Fake.iso", "--dk", KEY, "--chunk-size", 1, "--tc", 2, "--skip")
    output = directory / "Fake.iso_decrypted.iso"
    check_output(output)
    assert "Already done" in run(directory, "Fake.iso", "--dk", KEY, "--skip")
    check_output(output)
    shutil.copyfile(directory / "Fake.iso", directory / "Name with spaces.iso")
    shutil.copyfile(directory / "keys/Fake.dkey", directory / "keys/Name with spaces.dkey")
    run(directory, '"Name with spaces.iso"')
    check_output(directory / "Name with spaces.iso_decrypted.iso")


def batch_files(directory):
    shutil.copyfile(directory / "Fake.iso", directory / "Second.iso")
    shutil.copyfile(directory / "keys/Fake.dkey", directory / "keys/Second.dkey")
    for mode, flags in (("default", ()), ("sequential", ("--sequential",)), ("parallel", ("--jobs", 2))):
        run(directory, "Fake.iso", "Second.iso", "--output-dir", mode,
            "--chunk-size", 1, "--tc", 2, "--skip", *flags)
        check_output(directory / mode / "Fake_decrypted.iso")
        check_output(directory / mode / "Second_decrypted.iso")
        assert "Already done" in run(directory, "Fake.iso", "Second.iso",
                                     "--output-dir", mode, "--skip", *flags)


def errors_and_orphans(directory):
    shutil.copyfile(directory / "Fake.iso", directory / "Bad.iso")
    shutil.copyfile(directory / "keys/Fake.dkey", directory / "keys/Bad.dkey")
    with (directory / "Bad.iso").open("r+b") as file:
        file.write(bytes(4))
    orphan = directory / "Missing.iso_decrypted.iso.resume"
    orphan.write_text("orphan checkpoint; leave untouched")
    run(directory, "Bad.iso", "Fake.iso", "Missing.iso", "--jobs", 2,
        "--tc", 2, "--skip", succeeds=False)
    check_output(directory / "Fake.iso_decrypted.iso")
    assert orphan.read_text() == "orphan checkpoint; leave untouched"
    run(directory, "Fake.iso", "./Fake.iso", "--skip", succeeds=False)
    existing = directory / "unrelated.iso"
    existing.write_bytes(b"do not overwrite")
    run(directory, "Fake.iso", "--dk", KEY, "--output-name", "unrelated",
        "--skip", succeeds=False)
    assert existing.read_bytes() == b"do not overwrite"
    run(directory, "Fake.iso", "Bad.iso", "--dk", KEY, "--skip", succeeds=False)
    run(directory, "Fake.iso", "--auto", "--chunk-size", 0, "--skip", succeeds=False)


def automatic_recovery(directory):
    iso = directory / "Fake.iso"
    output = directory / "Fake.iso_decrypted.iso"
    part = Path(str(output) + ".part")
    resume = Path(str(output) + ".resume")
    child = subprocess.Popen(
        [str(BINARY), iso.name, "--dk", KEY, "--chunk-size", "1", "--tc", "1", "--skip"],
        cwd=directory, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    try:
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if resume.exists() and 0 < json.loads(resume.read_text())["next_offset"] < iso.stat().st_size:
                break
            assert child.poll() is None, "fixture finished before interruption"
            time.sleep(0.002)
        else:
            raise AssertionError("no committed checkpoint within 15 seconds")
        child.send_signal(signal.SIGSTOP)
        os.waitpid(child.pid, os.WUNTRACED)
        run(directory, iso.name, "--dk", KEY, "--tc", 1, "--skip", succeeds=False)
    finally:
        if child.poll() is None:
            child.kill()
        child.wait(timeout=5)

    saved_record = resume.read_bytes()
    offset = json.loads(saved_record)["next_offset"]
    assert 0 < offset < iso.stat().st_size
    prefix = part.read_bytes()[:offset]
    with EXPECTED.open("rb") as reference:
        assert prefix == reference.read(offset)
    run(directory, iso.name, "--dk", "ff" * 16, "--skip", succeeds=False)
    assert resume.read_bytes() == saved_record
    resume.write_text("{broken checkpoint")
    run(directory, iso.name, "--dk", KEY, "--skip", succeeds=False)
    resume.write_bytes(saved_record)
    with part.open("r+b") as file:
        file.truncate(offset - 2048)
    run(directory, iso.name, "--dk", KEY, "--skip", succeeds=False)
    part.write_bytes(prefix)
    with part.open("r+b") as file:
        file.seek(offset - 1)
        file.write(bytes([prefix[-1] ^ 1]))
    run(directory, iso.name, "--dk", KEY, "--skip", succeeds=False)
    part.write_bytes(prefix)
    source_stat = iso.stat()
    os.utime(iso, ns=(source_stat.st_atime_ns, source_stat.st_mtime_ns + 1_000_000_000))
    run(directory, iso.name, "--dk", KEY, "--skip", succeeds=False)
    os.utime(iso, ns=(source_stat.st_atime_ns, source_stat.st_mtime_ns))
    with part.open("ab") as file:
        file.write(b"uncommitted bytes" * 31)
    run(directory, iso.name, "--dk", KEY, "--chunk-size", 2, "--tc", 2, "--skip")
    check_output(output)
    logs = "".join(path.read_text() for path in (directory / "log").glob("*.log"))
    assert f"from byte {offset}" in logs, "did not start at the saved checkpoint"

    output.rename(part)
    run(directory, iso.name, "--dk", KEY, "--skip")
    check_output(output)


if not BINARY.is_file():
    raise SystemExit("Run cargo build --locked --bin ps3dec first.")
subprocess.run([sys.executable, "-B", str(ROOT / "tests/fake_iso.py")], check=True)
KEY = (TESTS / "keys/Fake.dkey").read_text().strip()
EXPECTED = TESTS / "Fake.expected.iso"
EXPECTED_HASH = digest(EXPECTED)
assert digest(TESTS / "Fake.iso") != EXPECTED_HASH
checks = [single_file, batch_files, errors_and_orphans]
if os.name == "posix":
    checks.append(automatic_recovery)
for check in checks:
    with tempfile.TemporaryDirectory(prefix="ps3dec-e2e-") as directory:
        directory = Path(directory)
        shutil.copyfile(TESTS / "Fake.iso", directory / "Fake.iso")
        shutil.copytree(TESTS / "keys", directory / "keys")
        check(directory)
    print(f"PASS {check.__name__}", flush=True)
print(f"{len(checks)} end-to-end checks passed")
