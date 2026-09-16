#!/usr/bin/env python3
"""latest.json for the Tauri updater; used by .github/workflows/dreamforge-desktop-release.yml.

  release-latest-json.py --check VERSION ENDPOINT                 exit 1 unless VERSION is newer than the published feed
  release-latest-json.py VERSION PLATFORM=SIG_FILE=URL [...]      print latest.json, one triple per platform
  release-latest-json.py --selftest
"""
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
    if current is not None and parse(version) <= parse(current):
        sys.exit(f"{version} is not newer than the published {current}")
    return f"ok: {version} > {current or 'nothing published yet'}"


def triple(arg):
    parts = arg.split("=", 2)  # URLs may carry '=' in a query string; platforms and paths do not
    if len(parts) != 3 or not all(parts):
        sys.exit(f"expected PLATFORM=SIG_FILE=URL, got {arg!r}")
    return parts


def read_sig(path):
    try:
        with open(path, encoding="ascii") as f:
            signature = f.read()
    except FileNotFoundError:
        sys.exit(f"missing signature: {path}")
    if not signature.strip():
        sys.exit(f"empty signature: {path}")
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

    with tempfile.TemporaryDirectory() as d:
        good, empty = os.path.join(d, "good.sig"), os.path.join(d, "empty.sig")
        with open(good, "w") as f:
            f.write("dW50cnVzdGVk\n")
        open(empty, "w").close()
        assert read_sig(good).strip() == "dW50cnVzdGVk"
        assert refuses(read_sig, empty)
        assert refuses(read_sig, os.path.join(d, "missing.sig"))
    print("selftest ok")


def main(argv):
    if argv == ["--selftest"]:
        return selftest()
    if argv[:1] == ["--check"] and len(argv) == 3:
        return print(check(argv[1], published_version(argv[2])))
    if len(argv) >= 2 and not argv[0].startswith("-"):
        platforms = []
        for arg in argv[1:]:
            name, sig_file, url = triple(arg)
            platforms.append((name, read_sig(sig_file), url))
        return print(json.dumps(manifest(argv[0], platforms), indent=2))
    sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
