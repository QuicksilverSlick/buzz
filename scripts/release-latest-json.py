#!/usr/bin/env python3
"""latest.json for the Tauri updater; used by .github/workflows/dreamforge-desktop-release.yml.

  release-latest-json.py --check VERSION ENDPOINT     exit 1 unless VERSION is plain x.y.z and newer than the published feed
  release-latest-json.py --next ENDPOINT FLOOR [RELEASED...]
                                                      print one patch above the feed and RELEASED, never below FLOOR
  release-latest-json.py --live VERSION ENDPOINT      exit 1 unless the public feed serves VERSION and its files download
  release-latest-json.py --pubkey PUB_FILE VERSION PLATFORM=SIG_FILE=URL [...]
                                                      print latest.json, one triple per platform;
                                                      refuses a signature made by any key but PUB_FILE's
  release-latest-json.py --selftest
"""
import base64
import json
import os
import re
import sys
import tempfile
import urllib.error
import urllib.request
from datetime import datetime, timezone


def parse(version):
    m = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)", version)
    if not m:
        sys.exit(
            f"version must be plain x.y.z: {version!r} "
            "(the updater orders semver, and a prerelease ranks below the installed build)"
        )
    return tuple(int(n) for n in m.groups())


def published_version(endpoint):
    try:
        with urllib.request.urlopen(endpoint, timeout=30) as resp:
            return json.load(resp)["version"]
    except urllib.error.HTTPError as err:
        if err.code == 404:
            return None
        raise


def check(version, current):
    new = parse(version)
    if current is not None and new <= parse(current):
        sys.exit(f"{version} is not newer than the published {current}")
    return f"ok: {version} > {current or 'nothing published yet'}"


def next_version(current, floor, released=()):
    """One patch above everything published or released, and never below floor."""
    known = [parse(v) for v in [current, *released] if v]
    major, minor, patch = max(known, default=(0, 0, 0))
    return ".".join(map(str, max((major, minor, patch + 1), parse(floor))))


PLATFORMS = {"darwin-aarch64", "darwin-x86_64", "windows-x86_64"}


def feed_problems(feed, version):
    platforms = feed.get("platforms") or {}
    problems = [] if feed.get("version") == version else [f"version is {feed.get('version')!r}, not {version}"]
    if set(platforms) != PLATFORMS:
        problems.append(f"platforms are {sorted(platforms)}, not {sorted(PLATFORMS)}")
    for name, entry in sorted(platforms.items()):
        if not (entry.get("signature") or "").strip() or not entry.get("url"):
            problems.append(f"{name} has no signature or url")
    return problems


def downloads(url):
    # One byte, not HEAD: the release redirect is a signed GET URL that refuses HEAD.
    try:
        with urllib.request.urlopen(urllib.request.Request(url, headers={"Range": "bytes=0-0"}), timeout=60) as resp:
            return resp.status in (200, 206) and len(resp.read(1)) == 1
    except (urllib.error.URLError, TimeoutError):
        return False


def live(version, endpoint):
    """What installed apps see: the public feed names VERSION and every file it names downloads."""
    with urllib.request.urlopen(endpoint, timeout=30) as resp:
        feed = json.load(resp)
    problems = feed_problems(feed, version)
    if not problems:
        problems = [f"{name} does not download" for name, e in sorted(feed["platforms"].items()) if not downloads(e["url"])]
    if problems:
        sys.exit("live feed: " + "; ".join(problems))
    return f"ok: the live feed serves {version} for {', '.join(sorted(feed['platforms']))}"


def triple(arg):
    parts = arg.split("=", 2)  # URLs may carry '=' in a query string; platforms and paths do not
    if len(parts) != 3 or not all(parts):
        sys.exit(f"expected PLATFORM=SIG_FILE=URL, got {arg!r}")
    return parts


def keynum(text, what):
    """8-byte key ID of a Tauri public key or .sig: base64 of a minisign file whose second line holds it at [2:10]."""
    try:
        k = base64.b64decode(base64.b64decode(text).decode().splitlines()[1])[2:10]
    except (ValueError, IndexError):
        k = b""
    if len(k) != 8:
        sys.exit(f"not a minisign key or signature: {what}")
    return k


def read_sig(path, key_id):
    try:
        with open(path, encoding="ascii") as f:
            signature = f.read()
    except FileNotFoundError:
        sys.exit(f"missing signature: {path}")
    # tauri build only warns when the private key does not pair with the public key; apps reject such updates.
    if keynum(signature, path) != key_id:
        sys.exit(f"{path} was signed by a different key than the updater public key")
    return signature


def manifest(version, platforms):
    parse(version)
    names = [name for name, _, _ in platforms]
    if not names or len(set(names)) != len(names):
        sys.exit(f"need at least one platform and no duplicates: {names}")
    return {
        "version": version,
        "notes": f"Dreamforge {version}",
        "pub_date": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "platforms": {name: {"signature": sig.strip(), "url": url} for name, sig, url in platforms},
    }


def refuses(fn, *args):
    try:
        fn(*args)
    except SystemExit:
        return True
    return False


def selftest():
    assert check("0.5.21", None).startswith("ok")
    assert check("0.5.21", "0.5.20").startswith("ok")
    assert check("0.10.0", "0.9.9").startswith("ok")  # numeric, not lexical
    for bad in ("0.5.20", "0.5.19", "0.5.20-df.1"):
        assert refuses(check, bad, "0.5.20"), bad
    for bad in ("", "garbage", "1.2", "0.5.21-rc.1"):  # validated even before anything is published
        assert refuses(check, bad, None), bad

    assert next_version("0.5.30", "0.5.36") == "0.5.36"  # the floor clears test builds above the feed
    assert next_version("0.5.36", "0.5.36") == "0.5.37"
    assert next_version("0.5.36", "0.5.36", ["0.5.36", "0.5.40"]) == "0.5.41"  # a tag can lead the feed
    assert next_version("0.9.9", "0.0.1") == "0.9.10"  # numeric, not lexical
    assert next_version(None, "0.5.36") == "0.5.36"
    assert refuses(next_version, "0.5.30", "0.5")
    assert refuses(next_version, "0.5.30", "0.5.36", ["v0.5.31"])

    good = {"version": "0.5.36", "platforms": {p: {"signature": "s", "url": "https://x/" + p} for p in PLATFORMS}}
    assert feed_problems(good, "0.5.36") == []
    assert feed_problems(good, "0.5.37")
    assert feed_problems({**good, "platforms": {"darwin-aarch64": good["platforms"]["darwin-aarch64"]}}, "0.5.36")
    unsigned = {**good, "platforms": {**good["platforms"], "windows-x86_64": {"signature": " ", "url": "https://x/w"}}}
    assert feed_problems(unsigned, "0.5.36") == ["windows-x86_64 has no signature or url"]
    assert feed_problems({}, "0.5.36")

    m = manifest("0.5.21", [("windows-x86_64", "sig\n", "https://x/y.exe"), ("darwin-aarch64", "s2", "https://x/a.tar.gz")])
    assert m["platforms"] == {
        "windows-x86_64": {"signature": "sig", "url": "https://x/y.exe"},
        "darwin-aarch64": {"signature": "s2", "url": "https://x/a.tar.gz"},
    }
    assert m["notes"] == "Dreamforge 0.5.21"
    assert refuses(manifest, "0.5.21", [])
    assert refuses(manifest, "0.5.21", [("a", "s", "u"), ("a", "s", "u")])

    assert triple("darwin-x86_64=d/x.sig=https://h/p?a=b") == ["darwin-x86_64", "d/x.sig", "https://h/p?a=b"]
    for bad in ("a=b", "=s=u", "p==u", "p=s="):
        assert refuses(triple, bad), bad

    # The committed key parses to the ID minisign printed in its comment (stored little-endian).
    pub = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "desktop", "src-tauri", "dreamforge-updater.pub")
    with open(pub) as f:
        assert keynum(f.read(), pub)[::-1].hex().upper() == "D9DED66F367B9EA7"

    def fake_sig(key_id):
        line = base64.b64encode(b"ED" + key_id + bytes(64)).decode()
        return base64.b64encode(f"untrusted comment: test\n{line}\ntrusted comment: t\nAAAA\n".encode()).decode()

    ours, theirs = b"\x01" * 8, b"\x02" * 8
    with tempfile.TemporaryDirectory() as d:
        sigs = {}
        for name, text in (("good", fake_sig(ours)), ("other", fake_sig(theirs)), ("empty", ""), ("junk", "dW50cnVzdGVk")):
            sigs[name] = os.path.join(d, name + ".sig")
            with open(sigs[name], "w") as f:
                f.write(text + "\n")
        assert read_sig(sigs["good"], ours).strip() == fake_sig(ours)
        for bad in ("other", "empty", "junk"):
            assert refuses(read_sig, sigs[bad], ours), bad
        assert refuses(read_sig, os.path.join(d, "missing.sig"), ours)
    print("selftest ok")


def main(argv):
    if argv == ["--selftest"]:
        return selftest()
    if argv[:1] == ["--check"] and len(argv) == 3:
        return print(check(argv[1], published_version(argv[2])))
    if argv[:1] == ["--next"] and len(argv) >= 3:
        return print(next_version(published_version(argv[1]), argv[2], argv[3:]))
    if argv[:1] == ["--live"] and len(argv) == 3:
        return print(live(argv[1], argv[2]))
    if argv[:1] == ["--pubkey"] and len(argv) >= 4:
        with open(argv[1], encoding="ascii") as f:
            key_id = keynum(f.read(), argv[1])
        platforms = []
        for arg in argv[3:]:
            name, sig_file, url = triple(arg)
            platforms.append((name, read_sig(sig_file, key_id), url))
        return print(json.dumps(manifest(argv[2], platforms), indent=2))
    sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
