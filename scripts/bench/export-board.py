#!/usr/bin/env python3
"""Turn an Artifact read_db dump of the claude.ai board into Bench drop files.

usage: python scripts/bench/export-board.py <dump>/items <home>/.dreamforge/bench/inbox/claude-ai

Keeps only what the Bench validator accepts. decidedBy stays verbatim: the
claude-ai writer folder is the one place "you"/"orchestrator" are allowed, and
the board prints them as "(decided on claude.ai)". updatedAt and every other
key are dropped; superseded stays superseded (folded, no card). Idempotent:
same content, same version hash, no relay write. Stdlib only.
"""
import json
import os
import re
import sys

KEEP = ("kind", "title", "body", "severity", "options", "state", "decidedBy", "validity")
# What item.rs valid_link takes: allowlisted https without userinfo, or a card deep link.
URL = re.compile(
    r"https://(github\.com|claude\.ai)(/[^\s@]*)?"
    r"|buzz://message\?channel=[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[89ab][0-9a-f]{3}-[0-9a-f]{12}&id=[0-9a-f]{64}",
    re.I,
)


def name(s):
    """The validator's [a-z0-9][a-z0-9-]* shape, for ids and areas."""
    return re.sub(r"[^a-z0-9-]+", "-", s.lower()).strip("-")


def link(l):
    label = " ".join(re.sub(r"[^A-Za-z0-9 _.,:-]", " ", l.get("label", "")).split())[:40]
    return {"label": label or "link", "url": l["url"]}


def export(src, dest):
    os.makedirs(dest, exist_ok=True)
    n = 0
    for f in sorted(os.listdir(src)):
        if not f.endswith(".json"):
            continue
        with open(os.path.join(src, f), encoding="utf-8") as fh:
            d = json.load(fh)
        # A pending needs options and a status needs a state; the page has a
        # few without (all superseded), and they stay on the page.
        if (d["kind"] == "pending" and not d.get("options")) or (d["kind"] == "status" and not d.get("state")):
            print(f"skip {f}: no options/state")
            continue
        out = {k: d[k] for k in KEEP if k in d}
        out["area"] = name(d.get("area") or "dreamforge")
        if "order" in d:
            out["order"] = round(d["order"] * 100)  # order is an i64; the page used 1.75-style floats
        links = [link(l) for l in d.get("links", []) if URL.fullmatch(l.get("url", ""))]
        if links:
            out["links"] = links[:4]
        with open(os.path.join(dest, name(f[:-5]) + ".json"), "w", encoding="utf-8", newline="\n") as fh:
            json.dump(out, fh, ensure_ascii=False, indent=2)
            fh.write("\n")
        n += 1
    print(f"exported {n} drops to {dest}")


if __name__ == "__main__":
    export(*sys.argv[1:3])
