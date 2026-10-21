"""What the commands share: usage errors, reading JSON files, the `--transforms` table."""

from __future__ import annotations

import argparse
import json
import os
from typing import Any, NoReturn

from .._api import Transform

Transforms = dict[str, Transform]
"""Deployment-private transforms loaded by --transforms."""


class UsageError(Exception):
    """The command could not run as asked; the message (usually the usage text) goes to stderr."""


class _Parser(argparse.ArgumentParser):
    def error(self, message: str) -> NoReturn:
        raise UsageError(f"{message}\n\n{self.usage}")


def parser(usage: str) -> argparse.ArgumentParser:
    """An argparse parser that raises `UsageError` with `usage` instead of exiting."""
    return _Parser(usage=usage, add_help=False, allow_abbrev=False)


def read_json(path: str) -> Any:
    with open(path, encoding="utf-8") as f:
        text = f.read()
    try:
        return json.loads(text)
    except json.JSONDecodeError as e:
        raise ValueError(f"{path}: {e}") from None


def json_files(args: list[str]) -> list[str]:
    """The files named on the command line; a directory stands for the .json files in it, sorted."""
    return [
        p
        for a in args
        for p in (
            [os.path.join(a, f) for f in sorted(os.listdir(a)) if f.endswith(".json")]
            if os.path.isdir(a)
            else [a]
        )
    ]
