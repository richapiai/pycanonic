"""`Artifact.stream` and `pycanonic.stream.routing_tap`: canonic/docs/streaming.md, in Python."""

import asyncio

import pytest
from conftest import fixture
from test_extract import ADA, ROUTED

from pycanonic import compile_spec
from pycanonic.stream import RoutingTap, routing_tap


@pytest.fixture(scope="module")
def globex(person):
    return compile_spec(fixture("specs/globex.json"), person)


def test_stream_the_docs_example(globex):
    s = globex.stream()
    assert s.push('{"fname":"Ada","e_mail":" Ada@Glo') is False
    assert s.push(b'bex.com ","tel":"+1 415 555 0100",') is False
    assert s.push('"org_url":"https://globex.com/x"') is True
    assert s.push("ignored") is True
    assert s.result() == ROUTED


def test_a_scalar_at_the_end_of_a_chunk_is_read_on_the_next_push(globex):
    s = globex.stream()
    s.push('{"tel":4155550100')
    s.push(',"x":1')
    assert s.result()["phone"] == globex.extract('{"tel":4155550100,"x":1}')["phone"] == "4155550100"


def test_result_twice_raises(globex):
    s = globex.stream()
    assert s.result() == {"email": None, "phone": None, "company.domain": None}
    with pytest.raises(ValueError, match="stream already finished"):
        s.result()
    with pytest.raises(ValueError, match="stream already finished"):
        s.push("{}")


def test_one_byte_pushes_equal_extract(bench):
    a, _, bodies = bench
    for raw in bodies["raws"]:
        s = a.stream()
        b = raw.encode()
        for i in range(len(b)):
            s.push(b[i : i + 1])
        assert s.result() == a.extract(b)


def chunks(text, size):
    b = text.encode()
    return [b[i : i + size] for i in range(0, len(b), size)]


async def drain(tap, log):
    out = []
    async for c in tap:
        out.append(c)
        log.append(("chunk", tap.routing.done()))
    return out


def test_tap_yields_every_chunk_and_resolves_early(globex):
    body = ADA[:-1] + "," + ",".join(f'"pad{i}":"{"x" * 40}"' for i in range(50)) + "}"
    parts = chunks(body, 16)
    log = []

    async def source():
        for c in parts:
            await asyncio.sleep(0)
            log.append("source")
            yield c
        log.append("exhausted")

    async def main():
        tap = routing_tap(globex, source())
        assert isinstance(tap, RoutingTap)
        reader = asyncio.create_task(drain(tap, log))
        routing = await tap.routing
        assert "exhausted" not in log
        return routing, await reader

    routing, out = asyncio.run(main())
    assert routing == ROUTED
    assert out == parts
    assert log[-1] == "exhausted"


def test_tap_settles_at_the_end_when_no_field_appears(globex):
    parts = chunks('{"fname":"Ada","lname":"Lovelace"}', 5)

    async def source():
        for c in parts:
            yield c

    async def main():
        tap = routing_tap(globex, source())
        out = [c async for c in tap]
        return out, await tap.routing

    out, routing = asyncio.run(main())
    assert out == parts
    assert routing == {"email": None, "phone": None, "company.domain": None}


def test_tap_takes_an_async_iterable(globex):
    class Body:
        def __aiter__(self):
            async def gen():
                yield ADA.encode()

            return gen()

    async def main():
        tap = routing_tap(globex, Body())
        return [c async for c in tap], await tap.routing

    assert asyncio.run(main()) == ([ADA.encode()], ROUTED)
