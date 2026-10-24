# Streaming and extraction

`artifact.extract(raw)` reads the routing fields straight out of the raw JSON text, without a full parse. It stops reading after the last one. `artifact.stream()` does the same over chunks as they arrive. `routing_tap` from `pycanonic.stream` wraps that around an async iterator of body chunks.

Routing fields are the canonical fields marked `routing: true` in your canonical schema. `artifact.routing` lists the ones the spec maps. The extractor skips a routing field that the spec leaves out.

## extract

```python
artifact = pycanonic.compile_spec(globex_spec, canonical)  # python/tests/fixtures/specs/globex.json

artifact.extract(b'{"fname":"Ada","e_mail":" Ada@Globex.com ","tel":"+1 415 555 0100","org_url":"https://globex.com/x"}')
# {'email': 'ada@globex.com', 'phone': '+14155550100', 'company.domain': 'globex.com'}
```

`extract` takes `bytes` or `str`. The result is keyed by spec field, flat (`"company.domain"`, not nested). Each value went through the field's transforms and routing validator. So it equals what `map(raw)` puts in that field, apart from the two cases under [What the extractor trusts](#what-the-extractor-trusts). `extract` does not count stats, and it runs with the GIL released.

It runs in one of two modes, chosen by the spec.

- Anchored. For a spec with `anchors`, the extractor searches for each anchored key token (`"e_mail":`). It reads the value right there. Each anchor search starts where the previous one matched. So a provider that keeps its key order costs one pass over the text. A token after an odd run of backslashes sits inside a string and is skipped.
- Scanner. A single pass reads the fields without an anchor. It also reads anchored fields whose token is not in the text as written (`{ "e_mail" : ... }` with spaces). The pass walks a trie of the wanted paths. It skips unwanted subtrees with a SIMD bracket scan and allocates nothing for them. It stops once every wanted path is resolved.

`canonic infer` proposes anchors only for keys that occur exactly once in every raw sample. See [spec-format.md](spec-format.md#anchors) for the rules the compiler enforces.

## stream

```python
s = artifact.stream()
s.push(b'{"fname":"Ada","e_mail":" Ada@Glo')     # False
s.push(b'bex.com ","tel":"+1 415 555 0100",')     # False
s.push(b'"org_url":"https://globex.com/x"')       # True
s.result()
# {'email': 'ada@globex.com', 'phone': '+14155550100', 'company.domain': 'globex.com'}
```

- `push(chunk)` takes `bytes` or `str`. It returns `True` once every routing field is available. More pushes after that are ignored.
- `result()` ends the input and returns the routing values. A field that never appeared is `None`.
- Each call to `stream()` starts a fresh reader. Use one per response.

`stream()` always uses scanner mode. Anchors apply to `extract(raw)` only.

### Chunk boundaries

The reader reads a value that spans chunks once the chunk that completes it arrives. A string is complete at its closing quote. A number, `true`, `false` or `null` is complete only with the character after it. The reader cannot know that `41555` is the whole number until it sees a `,`, `}`, `]` or whitespace. So a scalar that ends exactly at the end of a chunk is read on the next `push`:

```python
s = artifact.stream()
s.push(b'{"tel":4155550100')  # phone not resolved yet
s.push(b',"x":1')             # now it is
```

Partial input is buffered from the start of the value in progress. So memory stays at about one chunk plus that value.

## routing_tap

`routing_tap(artifact, source)` takes an async iterator of `bytes` chunks and returns a tap. The tap is itself an async iterator. It yields every chunk unchanged and pushes it into an `artifact.stream()` on the way. `tap.routing` is an `asyncio.Future`. It resolves as soon as the tap sees every routing field, or at the end of the body.

```python
import asyncio
import httpx
from pycanonic.stream import routing_tap

async def handle(client: httpx.AsyncClient, url: str):
    async with client.stream("GET", url) as res:
        tap = routing_tap(artifact, res.aiter_bytes())

        async def forward():
            async for chunk in tap:
                ...  # send the chunk on to your client, or keep it

        body = asyncio.create_task(forward())
        routing = await tap.routing  # resolves while the body is still arriving
        await body
```

Something must iterate the tap. If nothing reads it, the tap never sees a chunk and `tap.routing` never resolves. So start the reader before you await `tap.routing`.

`tap.routing` resolves at the end of the body with whatever was found, and `None` for the rest. If the reader stops early (`break`, an exception), `tap.routing` stays pending. Race it with your own timeout. The tap makes the future on the running loop at the first read of `routing` or the first iteration. So you can call `routing_tap` outside a loop.

## What the extractor trusts

The extractor does not check that its input is JSON. It trusts its input.

- Malformed or truncated input never raises. The extractor returns the fields read before the damage, and `None` for the rest. `extract(b'{"e_mail":"a@b.com", oops')` returns the email.
- Duplicate keys resolve to the first occurrence, where a path read in `map` takes the last. For `{"e_mail":"first@a.com","e_mail":"last@a.com"}`, `extract` gives `first@a.com` and `map(raw)` gives `last@a.com`.

Use `extract` and `stream` to route early: pick the next provider, fail fast on a miss, or start a downstream call. Before you store, return or bill a result, map the full body with `map`.

## Numbers

`bench/run.py` reads 3 routing fields from 5.9 KB bodies. It also reads 5 routing fields from 10.7 KB bodies, placed from 10% to 90% into the body. Every implementation must return the same output before the script times anything. The machine was an Apple M3 Max with Python 3.14.7, under load. The pycanonic and TypeScript columns come from one session at pycanonic 68197d1. The `orjson.loads` column comes from the earlier run at 8841588:

| Case | pycanonic | `orjson.loads` + read | TypeScript canonic |
|---|---:|---:|---:|
| 3 fields, 5.9 KB, scanner | 1.34 µs | 14.61 µs | 8.22 µs |
| same, anchored | 705 ns | 14.25 µs | 1.22 µs |
| 5 fields, 10.7 KB, scanner | 1.88 µs | 26.16 µs | 15.63 µs |
| same, anchored | 1.12 µs | 26.23 µs | 3.89 µs |
| 5 fields, 10.7 KB, 1 KB chunks | 8.85 µs | 26.36 µs (join + parse + read) | 18.20 µs |

[bench/README.md](../bench/README.md) has the method and the full tables. These numbers come from one laptop, and the server run is still to come.
