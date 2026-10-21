"""The `canonic` command line, ported from canonic's `packages/cli`.

Adding a command: write `commands/<name>.py` exposing `USAGE` (a string whose first line is
`canonic <name> ...`) and `run(args: list[str], transforms: Transforms) -> int`, then add it to
`COMMANDS` in `__main__.py`. The dispatcher prints `USAGE` for `--help`, strips `--transforms`
from `args` and imports that file first, and turns `UsageError` (or any other exception) into
exit 2. `run` returns 0 on success and 1 for a verdict (a failed fixture, drift). Parse `args`
with `io.parser(USAGE)`, which raises `UsageError` instead of exiting.
"""
