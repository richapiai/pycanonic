"""Runtime registry: one promoted and at most one shadow artifact per provider and endpoint."""

from __future__ import annotations

from ._core import Artifact


class Registry:
    """The artifacts being served, by provider and endpoint.

    `put` swaps a reference, so a request already holding the old artifact finishes on it.
    Readers need no lock: a dict item assignment is atomic under the GIL. Writers must be
    serial (one deploy at a time).
    """

    def __init__(self) -> None:
        self._promoted: dict[tuple[str, str], Artifact] = {}
        self._shadow: dict[tuple[str, str], Artifact] = {}

    def put(self, provider: str, endpoint: str, version: int, artifact: Artifact) -> None:
        """Serve `artifact` in the promoted or shadow slot its status names.

        Raises `ValueError` for a `draft` or `deprecated` artifact, and for one whose provider,
        endpoint or version differ from the arguments. Putting a promoted artifact with the
        version the shadow slot holds clears that slot.
        """
        at = f"{provider} {endpoint} v{version}"
        if (artifact.provider, artifact.endpoint, artifact.version) != (provider, endpoint, version):
            raise ValueError(
                f"registry: {at} does not match the artifact "
                f"({artifact.provider} {artifact.endpoint} v{artifact.version})"
            )
        if artifact.status not in ("promoted", "shadow"):
            raise ValueError(
                f"registry: {at} is {artifact.status}; only promoted and shadow artifacts are served"
            )
        key = (provider, endpoint)
        if artifact.status == "shadow":
            self._shadow[key] = artifact
            return
        self._promoted[key] = artifact
        shadow = self._shadow.get(key)
        if shadow is not None and shadow.version == version:
            del self._shadow[key]

    def get(self, provider: str, endpoint: str) -> Artifact | None:
        """The promoted artifact, or `None`."""
        return self._promoted.get((provider, endpoint))

    def get_shadow(self, provider: str, endpoint: str) -> Artifact | None:
        """The shadow artifact, or `None`."""
        return self._shadow.get((provider, endpoint))
