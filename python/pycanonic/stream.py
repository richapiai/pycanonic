"""`routing_tap`: read the routing fields out of a response body while it passes through, on asyncio."""

from __future__ import annotations

import asyncio
from collections.abc import AsyncIterable, AsyncIterator
from typing import Any

from ._core import Artifact

__all__ = ["RoutingTap", "routing_tap"]


class RoutingTap:
    """An async iterator over `source` that yields each chunk unchanged and pushes it into an
    `artifact.stream()` on the way.

    `routing` resolves with the routing values as soon as the tap sees every routing field, or at
    the end of the body with `None` for the rest. Something must iterate the tap: awaiting
    `routing` before you start reading can hang. If the reader stops early (`break`, an
    exception), `routing` stays pending, so race it with your own timeout. The future is made on
    the running loop the first time `routing` is read or the tap is iterated, so `routing_tap`
    itself can be called outside a loop.
    """

    def __init__(self, artifact: Artifact, source: AsyncIterable[bytes]) -> None:
        self._stream = artifact.stream()
        self._source = source
        self._it: AsyncIterator[bytes] | None = None
        self._routing: asyncio.Future[dict[str, Any]] | None = None

    @property
    def routing(self) -> asyncio.Future[dict[str, Any]]:
        if self._routing is None:
            self._routing = asyncio.get_running_loop().create_future()
        return self._routing

    def __aiter__(self) -> RoutingTap:
        return self

    async def __anext__(self) -> bytes:
        if self._it is None:
            self._it = aiter(self._source)
        routing = self.routing
        try:
            chunk = await anext(self._it)
        except StopAsyncIteration:
            if not routing.done():
                routing.set_result(self._stream.result())
            raise
        if not routing.done() and self._stream.push(chunk):
            routing.set_result(self._stream.result())
        return chunk


def routing_tap(artifact: Artifact, source: AsyncIterable[bytes]) -> RoutingTap:
    """A pass-through over `source` (an async iterator of body chunks) whose `routing` future
    resolves with `artifact`'s routing fields while the body is still arriving."""
    return RoutingTap(artifact, source)
