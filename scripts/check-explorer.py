#!/usr/bin/env python3
"""Decide, by rendering, whether the LEZ explorer has indexed a transaction.

WHY THIS EXISTS

The reviewer asked for explorer links, so the honest answer needs measuring
rather than asserting.

When this script was written the explorer was a WASM application: every
`/transaction/<hash>` URL returned the same 2416-byte shell and fetched its
content client-side, so an indexed transaction and a hash that *cannot exist*
were byte-identical over `curl` and a size comparison could not tell them apart.
That is why this drives a browser at all.

That is no longer true. Re-measured 2026-08-15, the explorer server-side renders
(Leptos): a real transaction comes back around 366 kB with its `Type:` and
`Proof Size:` in the body, and a hash that cannot exist comes back as a
2416-byte page reading `Failed to load transaction: error running server
function: Transaction not found`. So `curl` does separate them now, and
docs/DEPLOYMENT.md gives that one-liner.

Rendering is kept as the second opinion, not as a necessity: it reads the DOM a
reviewer actually sees rather than a byte count, and it keeps working if the
explorer goes back to rendering client-side. The impossible hash stays as the
control — whatever the explorer shows for "this does not exist" is the baseline
every real hash is compared against, and if that control ever renders as a
*found* transaction the baseline is meaningless and the whole run is abandoned.

    ./scripts/check-explorer.py                    # the committed lifecycle
    ./scripts/check-explorer.py <hash> [<hash>...] # arbitrary hashes

Needs playwright with firefox:  pip install playwright && playwright install firefox
"""
import pathlib
import re
import sys

BASE = "https://explorer.testnet.lez.logos.co"
IMPOSSIBLE = "ff" * 32
NOT_FOUND = "Transaction not found"
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
        # The hash cell may be a bare `hash` or a markdown link to the explorer,
        # so take the first 64-hex run in it either way.
        label = cells[0].replace("`", "")
        m = re.search(r"\b[0-9a-f]{64}\b", cells[1])
        if m and m.group(0) not in seen:
            seen.add(m.group(0))
            out.append((label, m.group(0)))
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
    # The control is the whole basis of the comparison. If a hash that cannot
    # exist renders as a found transaction, the baseline is wrong and every
    # verdict derived from it is meaningless — so refuse to print any.
    if NOT_FOUND not in control:
        sys.exit(
            f"control ({IMPOSSIBLE[:8]}…) is a hash that cannot exist, but the explorer did "
            f"not render it as not-found. The baseline is invalid, so no verdict below it "
            f"would mean anything. Control rendered:\n  {control[:300]}"
        )

    print(f"explorer's most recent indexed block: {head}\n")
    print(f"{'step':<38} verdict")
    for label, h, text in results[1:]:
        print(f"{label:<38} {'NOT INDEXED (same as control)' if text == control else 'INDEXED'}")
    print(f"\ncontrol ({IMPOSSIBLE[:8]}…) renders:\n  {control}")


if __name__ == "__main__":
    main()
