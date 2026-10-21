"""Pure-Python ports of the transforms and validators the person.enrich bench uses (from the proof
of concept), for the `hand-written py (pure-Python transforms)` reference. They cover the bench
inputs, which test_payloads.py checks against the TypeScript outputs; they are not a full port."""

from datetime import datetime, timezone

def trim(v):
    return v.strip() if isinstance(v, str) else None


def lower(v):
    return v.lower() if isinstance(v, str) else None


def split_name(v, part):
    if not isinstance(v, str):
        return None
    s = v.strip()
    if not s:
        return None
    if part == "first":
        i = s.find(" ")
        return s if i == -1 else s[:i]
    i = s.rfind(" ")
    return None if i == -1 else s[i + 1 :]


def _is_label(label):
    if not label or len(label) > 63 or label.startswith("-") or label.endswith("-"):
        return False
    return all(c.islower() and c.isascii() or c.isdigit() or c == "-" for c in label)


def is_domain(s):
    return len(s) <= 253 and "." in s and all(_is_label(x) for x in s.split("."))


def _strip_scheme(s):
    i = s.find("://")
    return s[i + 3 :] if 0 < i < 10 else s


def _cut(s, stops):
    end = len(s)
    for c in stops:
        i = s.find(c)
        if i != -1 and i < end:
            end = i
    return s[:end]


def to_domain(v):
    if not isinstance(v, str):
        return None
    s = v.strip().lower()
    if "@" in s and "/" not in s:
        s = s[s.rfind("@") + 1 :]
    host = _cut(_strip_scheme(s), "/?#:")
    if host.startswith("www."):
        host = host[4:]
    if host.endswith("."):
        host = host[:-1]
    return host if is_domain(host) else None


def normalize_phone(v):
    if isinstance(v, bool):
        return None
    if isinstance(v, int):
        s = str(v) if v > 0 else None
    elif isinstance(v, str):
        s = v.strip()
    else:
        s = None
    if s is None:
        return None
    plus = s.startswith("+")
    digits = []
    for c in s[1:] if plus else s:
        if c.isdigit():
            digits.append(c)
        elif c not in " -().":
            return None
    d = "".join(digits)
    if len(d) < 7 or len(d) > 15:
        return None
    return "+" + d if plus else d


def normalize_linkedin(v):
    if not isinstance(v, str):
        return None
    s = _strip_scheme(v.strip().lower())
    host = _cut(s, "/?#:")
    if host != "linkedin.com" and not host.endswith(".linkedin.com"):
        return None
    rest = s[len(host) :]
    if not rest.startswith("/"):
        return None
    parts = _cut(rest, "?#").split("/")[1:]
    kind = parts[0] if parts else ""
    slug = parts[1] if len(parts) > 1 else ""
    if kind not in ("in", "company", "school") or not slug:
        return None
    return f"https://www.linkedin.com/{kind}/{slug}"


def cast_integer(v):
    if isinstance(v, bool):
        return None
    if isinstance(v, int):
        return v
    if isinstance(v, float):
        return int(v) if v.is_integer() else None
    if isinstance(v, str):
        t = v.strip()
        if not t or (v[0] == "0" and len(v) > 2 and v[1] in "xXbBoO"):
            return None
        try:
            n = float(t)
        except ValueError:
            return None
        return int(n) if n.is_integer() else None
    return None


def parse_date(v):
    if isinstance(v, bool):
        return None
    if isinstance(v, (int, float)):
        if v <= 0 or v != int(v):
            return None
        ms = v * 1000 if v < 1e11 else v
        return datetime.fromtimestamp(ms / 1000, tz=timezone.utc).strftime("%Y-%m-%d")
    if not isinstance(v, str):
        return None
    s = v.strip()
    end = 10 if len(s) > 10 and s[10] in "T " else len(s)
    if end not in (4, 7, 10):
        return None
    try:
        y = int(s[0:4])
        m = int(s[5:7]) if end > 4 and s[4] == "-" else (1 if end == 4 else None)
        d = int(s[8:10]) if end > 7 and s[7] == "-" else (1 if end <= 7 else None)
        if m is None or d is None:
            return None
        datetime(y, m, d)
    except ValueError:
        return None
    return s[:end]


def valid_email(v):
    if not isinstance(v, str):
        return None
    at = v.find("@")
    if at < 1 or at != v.rfind("@") or " " in v:
        return None
    return v if is_domain(v[at + 1 :].lower()) else None


def valid_phone(v):
    return None if normalize_phone(v) is None else v


def valid_domain(v):
    return v if isinstance(v, str) and is_domain(v.lower()) else None



def cast(v, to):
    if to != "integer":
        raise ValueError(f"only cast integer is ported, not {to}")
    return cast_integer(v)


TRANSFORMS = {
    "trim": trim,
    "lower": lower,
    "split_name": split_name,
    "normalize_phone": normalize_phone,
    "normalize_linkedin": normalize_linkedin,
    "to_domain": to_domain,
    "cast": cast,
    "parse_date": parse_date,
}
VALIDATORS = {"email": valid_email, "phone": valid_phone, "domain": valid_domain}
