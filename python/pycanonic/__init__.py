"""Compile declarative field-mapping specs into fast mappers from provider JSON to your canonical schema."""

from ._api import (
    FieldAlert,
    FillMonitor,
    GateResult,
    Issue,
    SpecError,
    Transform,
    compile_spec,
    default_transforms,
    load,
    merge_gate,
    validate_canonical,
    validate_spec,
    validators,
)
from ._core import Artifact, Rejected, ShadowDiff, Stats, fingerprint, shape
from .registry import Registry

__version__ = "0.2.0"

__all__ = [
    "Artifact",
    "FieldAlert",
    "FillMonitor",
    "GateResult",
    "Issue",
    "Registry",
    "Rejected",
    "ShadowDiff",
    "SpecError",
    "Stats",
    "Transform",
    "compile_spec",
    "default_transforms",
    "fingerprint",
    "load",
    "merge_gate",
    "shape",
    "validate_canonical",
    "validate_spec",
    "validators",
]
