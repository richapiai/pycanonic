"""Compile declarative field-mapping specs into fast mappers from provider JSON to your canonical schema."""

from ._api import (
    Issue,
    SpecError,
    Transform,
    compile_spec,
    default_transforms,
    load,
    validate_canonical,
    validate_spec,
    validators,
)
from ._core import Artifact, Stats
from .registry import Registry

__version__ = "0.1.0"

__all__ = [
    "Artifact",
    "Issue",
    "Registry",
    "SpecError",
    "Stats",
    "Transform",
    "compile_spec",
    "default_transforms",
    "load",
    "validate_canonical",
    "validate_spec",
    "validators",
]
