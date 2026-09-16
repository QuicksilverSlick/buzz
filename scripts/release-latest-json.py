#!/usr/bin/env python3
"""latest.json for the Tauri updater; used by .github/workflows/dreamforge-desktop-release.yml.

  release-latest-json.py --check VERSION ENDPOINT     exit 1 unless VERSION is plain x.y.z and newer than the published feed
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
