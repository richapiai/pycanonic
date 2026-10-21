"""Smoke tests for run.py: the self-test trips on an injected slowdown, and this machine passes its
gate. They time things for ~15 s and can fail on a loaded machine, so they are opt-in:
    PYCANONIC_BENCH_TESTS=1 .venv/bin/pytest bench"""

import json
import os
import platform
import subprocess
import sys
from pathlib import Path

import pytest

RUN = Path(__file__).parent / "run.py"
PLATFORM = f"{sys.platform}-{platform.machine()}"
pytestmark = pytest.mark.skipif(os.environ.get("PYCANONIC_BENCH_TESTS") != "1", reason="timing tests; set PYCANONIC_BENCH_TESTS=1")


def run(flag):
    return subprocess.run([sys.executable, str(RUN), flag], capture_output=True, text=True)


def test_selftest_trips_the_gate():
    out = run("--selftest")
    assert out.returncode == 0, out.stdout + out.stderr


@pytest.mark.skipif(PLATFORM not in json.loads((RUN.parent / "baseline.json").read_text()), reason=f"no {PLATFORM} baseline")
def test_gate_passes_on_this_machine():
    out = run("--gate")
    assert out.returncode == 0, out.stdout + out.stderr
