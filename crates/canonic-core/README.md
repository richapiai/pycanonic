# canonic-core

The Rust core of [pycanonic](https://github.com/richapiai/pycanonic). It compiles a JSON mapping spec against a canonical schema into a plan. It then runs the plan over raw JSON bytes or any document type that implements `Doc`. It also holds the extractor, the default transforms and the drift checks.

Most users want the Python package: `pip install pycanonic`. The [repository README](https://github.com/richapiai/pycanonic#readme) and [docs](https://github.com/richapiai/pycanonic/tree/main/docs) describe the spec format and the behavior.

License: MIT.
