"""Every json block marked `<!-- run -->` in docs/spec-format.md and docs/fit.md runs.

A block is `{"schema": ..., "spec": ...}`, optionally with `"body"`, `"context"` and `"expected"`
(the conformance case format: `{"$rejected": "<reason>"}` means map raises Rejected). The spec must
compile, and the body must map to `expected` through map(bytes), map(dict) and map_json."""

import json
import re
from pathlib import Path

import pytest

import pycanonic

DOCS = Path(__file__).parents[2] / "docs"
BLOCK = re.compile(r"<!-- run -->\s*```json\n(.*?)```", re.S)


def _blocks() -> list[object]:
    out = []
    for name in ("spec-format.md", "fit.md"):
        for i, m in enumerate(BLOCK.finditer((DOCS / name).read_text(encoding="utf-8"))):
            out.append(pytest.param(m.group(1), id=f"{name}#{i}"))
    return out


def test_docs_have_examples() -> None:
    assert len(_blocks()) >= 15


@pytest.mark.parametrize("block", _blocks())
def test_example_runs(block: str) -> None:
    ex = json.loads(block)
    art = pycanonic.compile_spec(ex["spec"], ex["schema"])
    if "body" not in ex:
        return
    raw, ctx, want = json.dumps(ex["body"]).encode(), ex.get("context"), ex["expected"]
    ways = [lambda: art.map(raw, context=ctx), lambda: art.map(ex["body"], context=ctx), lambda: json.loads(art.map_json(raw, context=ctx))]
    for way in ways:
        if isinstance(want, dict) and "$rejected" in want:
            with pytest.raises(pycanonic.Rejected) as e:
                way()
            assert e.value.reason == want["$rejected"]
        else:
            assert way() == want
