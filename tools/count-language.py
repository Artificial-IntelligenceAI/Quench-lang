#!/usr/bin/env python3
"""Counts what the language is made of, and writes it down.

The site used to work these numbers out in the browser, which meant every visitor
downloaded 226 KB of Rust so the page could print three integers. It does not need
to be live to be true — it needs to say what it read. So this reads the compiler
once, writes `data/language.json`, and records the commit it was looking at.

Run it after anything that changes the lexer or adds a word:

    python3 tools/count-language.py

Or ask whether it would change anything, which is the same work without the write
and exits non-zero when the answer is yes:

    python3 tools/count-language.py --check

Both take `--repo <path>` when the compiler is not at the usual place, which is
how the workflow in `.github/workflows` runs it against a fresh clone of `main`.
"""

import json
import os
import pathlib
import re
import subprocess
import sys

def language_repo() -> pathlib.Path:
    """Where the compiler is. The checkout on this machine by default, and
    somewhere else when a checker has cloned it — `--repo <path>`."""
    if "--repo" in sys.argv:
        return pathlib.Path(sys.argv[sys.argv.index("--repo") + 1]).resolve()
    return pathlib.Path(os.environ.get("QUENCH_REPO", "/Users/ts/Quench"))


QUENCH = language_repo()
OUT = pathlib.Path(__file__).resolve().parent.parent / "data" / "language.json"

READS = [
    "crates/quench-lex/src/token.rs",
    "crates/quench-parse/src/lib.rs",
    "crates/quench-parse/src/ast.rs",
    "crates/quench-check/src/lib.rs",
]

# Groups the compiler can be asked for directly. These are not written down here
# at all — they are read out of the constant or the match that the compiler itself
# uses, so they cannot drift from the language however fast it moves.
#
# This is the direction that matters. A list that only checks its own members is
# exactly as stale as whoever last edited it: it notices a word being removed and
# is blind to one being added. `provided` went from 2 to 31 without a word of the
# old list becoming untrue, and nothing here could have seen it.
DERIVED = [
    (
        "provided", "Provided",
        "crates/quench-check/src/lib.rs",
        r"pub const PROVIDED[^=]*=\s*&\[(.*?)\n\];",
        r'\("([^"]+)"',
    ),
    (
        "types", "Types",
        "crates/quench-check/src/lib.rs",
        r"fn simple\(word: &str\) -> Option<Ty> \{(.*?)\n        \}",
        r'"([^"]+)"\s*=>\s*Some',
    ),
]

# Groups with no single place in the compiler to read. The parser, the tree and the
# checker each recognise their own, so these stay written down — and every word is
# checked against the source, which catches a removal but cannot catch an addition.
# If a canonical list ever appears for one of these, it belongs in DERIVED instead.
LISTED = [
    ("statements", "Statements", ["var", "set", "add", "print", "call", "give", "if", "else", "loop", "break"]),
    ("topLevel", "Top level", ["fn", "const", "START"]),
    ("chainLinks", "Chain links", ["mut", "immut", "arr", "grow", "temp", "perm", "range", "while", "nothing"]),
    ("visibility", "Visibility", ["file", "program", "export"]),
    ("operators", "Operators", ["x", "mod", "and", "or", "not"]),
    ("beforeAValue", "Before a value", ["share", "copy"]),
    ("literals", "Literals", ["true", "false"]),
    ("streams", "Streams", ["stdout", "stderr"]),
]


def git(*args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(QUENCH), *args], capture_output=True, text=True, check=True
    ).stdout.strip()


def main() -> int:
    if not QUENCH.is_dir():
        print(f"no checkout at {QUENCH}", file=sys.stderr)
        return 1

    sources = {path: (QUENCH / path).read_text() for path in READS}
    token = sources[READS[0]]
    everything = "\n".join(sources.values())

    # Checked, not asserted: if the lexer ever grows a keyword table this stops
    # saying nought and starts saying nothing at all.
    reserves_none = re.search(r"reserves no words|reserves none", token, re.I) is not None

    # A symbol is a token kind whose name in an error message is its own spelling
    # in backticks — which is what separates `[` from "a written value".
    described = re.findall(r"Kind::(\w+)\s*=>\s*\"([^\"]+)\"", token)
    symbols = sum(1 for _, text in described if text.startswith("`"))

    # Quoted or backticked: most words appear as string literals, but the two
    # streams are only ever named in the tree's documentation.
    def found(word: str) -> bool:
        # Quoted or backticked: most words appear as string literals, but the two
        # streams are only ever named in the tree's documentation.
        return re.search(rf"[\"`]{re.escape(word)}[\"`]", everything) is not None

    categories = []

    for cid, label, path, block, name in DERIVED:
        text = (QUENCH / path).read_text()
        region = re.search(block, text, re.S)
        if region is None:
            print(f"could not find the {cid} list in {path}", file=sys.stderr)
            return 1
        words = re.findall(name, region.group(1))
        categories.append({
            "id": cid, "label": label, "count": len(words), "words": words,
            "missing": [], "readFrom": path,
        })

    for cid, label, group in LISTED:
        categories.append({
            "id": cid, "label": label, "count": len(group), "words": group,
            "missing": [w for w in group if not found(w)], "readFrom": None,
        })

    order = ["statements", "topLevel", "chainLinks", "visibility", "types",
             "operators", "beforeAValue", "literals", "streams", "provided"]
    categories.sort(key=lambda c: order.index(c["id"]))

    every_word = [w for c in categories for w in c["words"]]
    missing = [w for c in categories for w in c["missing"]]

    data = {
        "readFrom": {
            "commit": git("rev-parse", "--short", "HEAD"),
            "date": git("log", "-1", "--format=%ad", "--date=short"),
            "files": READS,
        },
        "reserved": 0 if reserves_none else None,
        "symbols": symbols or None,
        "tokenKinds": len(described),
        "words": len(every_word),
        "confirmed": len(every_word) - len(missing),
        "missing": missing,
        "categories": categories,
    }

    written = json.dumps(data, indent=2) + "\n"

    if "--check" in sys.argv:
        # For a hook, or for anyone who would rather be told than remember. The
        # commit and date are ignored: they move with every commit to the language
        # and would report drift on work that changed none of this.
        def facts(text: str) -> str:
            loaded = json.loads(text)
            loaded.pop("readFrom", None)
            return json.dumps(loaded, sort_keys=True)

        if not OUT.exists():
            print(f"{OUT.name} does not exist yet", file=sys.stderr)
            return 1
        if facts(written) != facts(OUT.read_text()):
            print("the site's counts no longer match the compiler.", file=sys.stderr)
            was = json.loads(OUT.read_text())
            print(f"  file says {was['words']} words, {was['symbols']} symbols; "
                  f"source says {data['words']} words, {data['symbols']} symbols", file=sys.stderr)
            for now in data["categories"]:
                before = next((c for c in was["categories"] if c["id"] == now["id"]), None)
                if before is None or before["words"] != now["words"]:
                    old = set(before["words"]) if before else set()
                    added = [w for w in now["words"] if w not in old]
                    gone = [w for w in old if w not in now["words"]]
                    bits = []
                    if added:
                        bits.append(f"gained {', '.join(added)}")
                    if gone:
                        bits.append(f"lost {', '.join(gone)}")
                    print(f"  {now['label']}: {'; '.join(bits)}", file=sys.stderr)
            print("  run: python3 tools/count-language.py", file=sys.stderr)
            return 1
        print(f"the site's counts still match the compiler at {data['readFrom']['commit']}")
        return 0

    OUT.parent.mkdir(exist_ok=True)
    OUT.write_text(written)

    print(f"read {sum(len(s) for s in sources.values()):,} bytes of Rust at {data['readFrom']['commit']}")
    derived = sum(c["count"] for c in categories if c["readFrom"])
    print(f"  reserved {data['reserved']}, symbols {symbols} of {len(described)} kinds")
    print(f"  words {len(every_word)} across {len(categories)} categories "
          f"({derived} read from the compiler, {len(every_word) - derived} listed here)")
    if missing:
        print(f"  NOT FOUND IN THE SOURCE: {', '.join(missing)}", file=sys.stderr)
    print(f"wrote {OUT.relative_to(pathlib.Path.cwd())} ({OUT.stat().st_size} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
