#!/usr/bin/env python3
"""latest.json for the Tauri updater (Windows only); used by scripts/release-windows.bat.

  release-latest-json.py --check VERSION ENDPOINT         exit 1 unless VERSION is newer than the published feed
  release-latest-json.py VERSION SIG_FILE EXE_URL [NOTES]  print latest.json
  release-latest-json.py --selftest
"""
import json
import re
import sys
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


def manifest(version, signature, url, notes):
    parse(version)
    return {
        "version": version,
        "notes": notes or f"Dreamforge {version}",
        "pub_date": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "platforms": {"windows-x86_64": {"signature": signature.strip(), "url": url}},
    }


def selftest():
    assert check("0.5.21", None).startswith("ok")
    assert check("0.5.21", "0.5.20").startswith("ok")
    assert check("0.10.0", "0.9.9").startswith("ok")  # numeric, not lexical
    for bad in ("0.5.20", "0.5.19", "0.5.20-df.1"):
        try:
            check(bad, "0.5.20")
        except SystemExit:
            continue
        raise AssertionError(bad)
    m = manifest("0.5.21", "sig\n", "https://x/y.exe", "")
    assert m["platforms"]["windows-x86_64"] == {"signature": "sig", "url": "https://x/y.exe"}
    assert m["notes"] == "Dreamforge 0.5.21"
    print("selftest ok")


def main(argv):
    if argv == ["--selftest"]:
        return selftest()
    if argv[:1] == ["--check"] and len(argv) == 3:
        return print(check(argv[1], published_version(argv[2])))
    if len(argv) in (3, 4):
        version, sig_file, url = argv[:3]
        with open(sig_file, encoding="ascii") as f:
            signature = f.read()
        notes = argv[3] if len(argv) == 4 else ""
        return print(json.dumps(manifest(version, signature, url, notes), indent=2))
    sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
