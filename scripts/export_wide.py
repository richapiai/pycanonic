"""Writes crates/canonic-core/tests/fixtures/wide_bodies.json for `cargo bench --bench wide`: the
501-field spec, schema and 64 payloads of bench/interpreters.py, so the Rust bench maps the same
bytes. Run after changing the payload rules:
    .venv/bin/python scripts/export_wide.py
"""

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))

import interpreters as I  # noqa: E402

out = {
    "spec": I.canonic_spec(True),
    "schema": I.SCHEMA,
    "raws": [r.decode() for r in I.raws],
}
path = ROOT / "crates/canonic-core/tests/fixtures/wide_bodies.json"
path.write_text(json.dumps(out, separators=(",", ":")) + "\n")
print(f"wrote {path.relative_to(ROOT)}: {len(out['raws'])} bodies")
