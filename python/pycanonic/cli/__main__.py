"""The `canonic` dispatcher. Exit 1 is a command's own verdict (a failed fixture, drift). Anything
that stopped it from running (a missing file, an unknown flag, a spec that doesn't compile) exits 2,
so a script can tell them apart."""

from __future__ import annotations

import importlib.util
import os
import sys
from collections.abc import Callable
from types import ModuleType

from .commands import build, diff, infer, test
from .io import Transforms, UsageError

COMMANDS: dict[str, ModuleType] = {"infer": infer, "test": test, "diff": diff, "build": build}

USAGE = f"""usage: canonic <command> [options]

{chr(10).join(f"  {m.USAGE.splitlines()[0]}" for m in COMMANDS.values())}

  --transforms <module.py>  deployment-private transforms for any command: the file's public
                            module-level functions, by name

canonic <command> --help shows the details."""


def _load_transforms(args: list[str]) -> tuple[list[str], Transforms]:
    """Takes --transforms <module.py> out of args and imports the public functions the file defines
    (not names it imports)."""
    if "--transforms" not in args:
        return args, {}
    i = args.index("--transforms")
    if i + 1 >= len(args):
        raise UsageError("--transforms needs a module path")
    file = args[i + 1]
    spec = importlib.util.spec_from_file_location(os.path.splitext(os.path.basename(file))[0], file)
    if spec is None or spec.loader is None:
        raise UsageError(f"--transforms: can't import {file}")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    fns: dict[str, Callable[..., object]] = {
        k: v
        for k, v in vars(mod).items()
        if not k.startswith("_") and callable(v) and getattr(v, "__module__", None) == mod.__name__
    }
    return args[:i] + args[i + 2 :], fns


def main(argv: list[str] | None = None) -> int:
    command, *rest = (sys.argv[1:] if argv is None else argv) or [""]
    if command in ("--help", "-h"):
        print(USAGE)
        return 0
    entry = COMMANDS.get(command)
    if entry is None:
        print(USAGE, file=sys.stderr)
        return 2
    if "--help" in rest or "-h" in rest:
        print(entry.USAGE)
        return 0
    try:
        args, transforms = _load_transforms(rest)
        code: int = entry.run(args, transforms)
        return code
    except UsageError as e:
        print(e, file=sys.stderr)
    except Exception as e:  # noqa: BLE001 - the command could not run; say why and exit 2
        print(f"canonic: {e}", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
