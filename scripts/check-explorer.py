#!/usr/bin/env python3
"""Decide, by rendering, whether the LEZ explorer has indexed a transaction.

WHY THIS EXISTS

The reviewer asked for explorer links, so the honest answer needs measuring
rather than asserting — and the obvious measurement does not work.

The explorer is a WASM application. Every `/transaction/<hash>` URL returns the
same 2416-byte shell and fetches its content client-side, so an indexed
transaction and a hash that *cannot exist* are byte-identical over `curl`. A
size comparison cannot distinguish them, and concluding anything from one is a
mistake this repository made once already.

So this renders each page in a headless browser and prints the resulting text,
with an impossible hash as the control: whatever the explorer shows for "this
does not exist" is the baseline every real hash is compared against.

    ./scripts/check-explorer.py                    # the committed lifecycle
    ./scripts/check-explorer.py <hash> [<hash>...] # arbitrary hashes

Needs playwright with firefox:  pip install playwright && playwright install firefox
"""
import pathlib
import re
import sys

BASE = "https://explorer.testnet.lez.logos.co"
IMPOSSIBLE = "ff" * 32
ROOT = pathlib.Path(__file__).resolve().parent.parent


def committed_hashes():
    """The lifecycle recorded in docs/DEPLOYMENT.md, so this never drifts from it."""
    doc = (ROOT / "docs" / "DEPLOYMENT.md").read_text()
    # Only the lifecycle table. Later tables list ImageIDs, which are also
    # 64 hex characters and are not transactions.
    doc = doc.split("## The lifecycle, on chain", 1)[-1].split("###", 1)[0]
    out, seen = [], set()
    for line in doc.splitlines():
        if not line.startswith("|"):
            continue
        cells = [c.strip() for c in line.split("|") if c.strip()]
        if len(cells) < 2:
            continue
        label, tx = cells[0].replace("`", ""), cells[1].strip("`")
        if len(tx) == 64 and all(ch in "0123456789abcdef" for ch in tx) and tx not in seen:
            seen.add(tx)
            out.append((label, tx))
    return out


def main():
    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        sys.exit("playwright is not installed: pip install playwright && playwright install firefox")

    args = sys.argv[1:]
    cases = [("control:impossible", IMPOSSIBLE)]
    cases += [(f"arg{i}", h) for i, h in enumerate(args)] if args else committed_hashes()
    if len(cases) == 1:
        sys.exit("no transaction hashes found in docs/DEPLOYMENT.md and none given")

    results = []
    with sync_playwright() as p:
        browser = p.firefox.launch(headless=True)
        page = browser.new_page(viewport={"width": 1280, "height": 1400})

        page.goto(BASE + "/", wait_until="load", timeout=60000)
        page.wait_for_timeout(4000)
        home = " ".join(page.inner_text("body").split())
        m = re.search(r"Recent Blocks Block (\d+)", home)
        head = m.group(1) if m else "?"

        for label, h in cases:
            page.goto(f"{BASE}/transaction/{h}", wait_until="load", timeout=60000)
            try:
                page.wait_for_load_state("networkidle", timeout=30000)
            except Exception:
                pass
            page.wait_for_timeout(4000)
            results.append((label, h, " ".join(page.inner_text("body").split())))
        browser.close()

    control = results[0][2]
    print(f"explorer's most recent indexed block: {head}\n")
    print(f"{'step':<38} verdict")
    for label, h, text in results[1:]:
        print(f"{label:<38} {'NOT INDEXED (same as control)' if text == control else 'INDEXED'}")
    print(f"\ncontrol ({IMPOSSIBLE[:8]}…) renders:\n  {control}")


if __name__ == "__main__":
    main()
