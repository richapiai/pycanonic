# Custom transforms

You add transforms by passing named Python functions to `compile_spec`. Specs refer to them by name, the same way they refer to the defaults.

```python
import re
import pycanonic

spec = {
    "provider": "acme", "endpoint": "person.enrich", "version": 1, "status": "promoted",
    "fields": {"full_name": {"from": "name", "transform": ["trim", "strip_honorific"]}},
}

def strip_honorific(v):
    return re.sub(r"^(Dr|Mr|Mrs|Ms)\.? ", "", v) if isinstance(v, str) else None

artifact = pycanonic.compile_spec(spec, canonical, transforms={"strip_honorific": strip_honorific})
artifact.map({"name": " Dr. Ada Lovelace "})["full_name"]  # "Ada Lovelace"
```

## The contract

A transform is `f(value)` or `f(value, arg)`.

- `value` is never `None`. The plan skips the transforms for a missing source, and a pipeline stops at the first `None`.
- `arg` is the argument from the spec, `{ "my_transform": <arg> }`. For the bare-name form, the transform is called with one argument, `f(value)`. pycanonic checks arguments only for the default transforms. Yours get whatever the spec says.
- Return `None` for input you cannot handle. This counts an error for the field in `artifact.stats`.
- Return a `str`, `int`, `float`, `bool` or `None`. Any other type, such as a dict, counts as a rejection.
- A dict or list value arrives as a JSON copy, not the original object. The copy stops at 512 levels or 1,000,000 values. Past either cap the field is `None` and one error is counted.
- If your function raises an `Exception`, pycanonic catches it. The field gets `None` or its `default`, and the error is counted. The request carries on. `KeyboardInterrupt` and `SystemExit` are not caught. They come out of `map` once it returns.
- The transform runs with the GIL held, also inside `map_json` and `extract`.

An injected transform with the same name as a default replaces it for that artifact. `compile_spec(spec, schema, transforms={"trim": my_trim})` calls `my_trim` wherever the spec says `"trim"`. The same holds for a loaded plan. `load(path, transforms={"cast": my_cast})` replaces the default `cast`, and `my_cast` gets the argument from the spec.

A name matches only a function you passed.

## Unknown names fail at compile time

A spec that names a transform nobody registered does not compile:

```
acme person.enrich v1: fields["phone"]: unknown transform "e164"
```

The same check covers transforms inside `item` maps (`fields["experiences[]"].item["start"]: unknown transform ...`).

## Plans

`canonic build` writes each spec as a compiled plan (`spec.json` becomes `spec.plan.json`). `pycanonic.load` reads a plan without a new compile. The build compiles the spec, so it needs the names of your transforms up front. Pass one `--transform` per name:

```
canonic build --schema canonical/person.enrich.json --transform e164 phone.spec.json
phone.spec.json -> phone.spec.plan.json
```

Without `--transform e164`, the build fails with the `unknown transform` error above and exits 2. Only the name matters at build time. You give the functions to `load`:

```python
import pycanonic

artifact = pycanonic.load("phone.spec.plan.json", transforms={"e164": e164})
```

`--transforms <module.py>` does the same for every public function that the file defines. Names it imports from elsewhere are left out. `canonic test`, `canonic diff` and `canonic bench` use it to run specs that have private transforms. `canonic build` uses it to declare them. `canonic infer` accepts the flag and drafts with the default transforms only.

```
canonic test --schema canonical/person.enrich.json --transforms ./transforms.py phone.spec.json fixtures/
```

## A full phone parser

The core `normalize_phone` keeps digits and a leading `+` and has no country rules. For E.164, inject a parser such as `phonenumbers` and pass the default country as the argument:

```python
import phonenumbers
import pycanonic

def e164(v, country):
    try:
        n = phonenumbers.parse(str(v), country)
    except phonenumbers.NumberParseException:
        return None
    if not phonenumbers.is_valid_number(n):
        return None
    return phonenumbers.format_number(n, phonenumbers.PhoneNumberFormat.E164)

artifact = pycanonic.compile_spec(
    {
        "provider": "acme", "endpoint": "person.enrich", "version": 1, "status": "promoted",
        "fields": {"phone": {"from": "contact.mobile", "transform": {"e164": "US"}}},
    },
    canonical,
    transforms={"e164": e164},
)
```

For a routing field `phone`, the `phone` validator still runs after `e164`. Anything it rejects counts as an error.

A Python transform costs a call into Python for each value. On a hot field, a default transform is faster.
