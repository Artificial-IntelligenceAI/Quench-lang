#!/usr/bin/env python3
"""Counts what the language is made of, and writes it down.

The site used to work these numbers out in the browser, which meant every visitor
downloaded 226 KB of Rust so the page could print three integers. It does not need
to be live to be true — it needs to say what it read. So this reads the compiler
once, writes `data/language.json`, and records the commit it was looking at.

Run it after anything that changes the lexer or adds a word:

    python3 tools/count-language.py
"""

import json
import pathlib
import re
import subprocess
import sys

QUENCH = pathlib.Path("/Users/ts/Quench")
OUT = pathlib.Path(__file__).resolve().parent.parent / "data" / "language.json"

READS = [
    "crates/quench-lex/src/token.rs",
    "crates/quench-parse/src/lib.rs",
    "crates/quench-parse/src/ast.rs",
    "crates/quench-check/src/lib.rs",
]

# The words the language answers to, grouped by where a word means something.
# There is no single list inside the compiler to read — the lexer keeps none by
# design — so this is the list, and every one of them is checked against the
# source below rather than taken on trust.
# The words the language answers to, grouped by where a word means something.
# There is no single list inside the compiler to read — the lexer keeps none by
# design — so this is the list, and every word in it is checked against the source
# below rather than taken on trust. The grouping is the point, not decoration:
# a word means what it means because of where it may stand.
WORDS = [
    ("statements", "Statements", ["var", "set", "add", "print", "call", "give", "if", "else", "loop", "break"]),
    ("topLevel", "Top level", ["fn", "const", "START"]),
    ("chainLinks", "Chain links", ["mut", "immut", "arr", "grow", "temp", "perm", "range", "while", "nothing"]),
    ("visibility", "Visibility", ["file", "program", "export"]),
    ("types", "Types", ["i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64",
                        "b16", "b32", "b64", "d32", "d64", "e", "bool", "str"]),
    ("operators", "Operators", ["x", "mod", "and", "or", "not"]),
    ("beforeAValue", "Before a value", ["share", "copy"]),
    ("literals", "Literals", ["true", "false"]),
    ("streams", "Streams", ["stdout", "stderr"]),
    ("provided", "Provided", ["count", "stitch"]),
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
    every_word = [word for _, _, group in WORDS for word in group]

    def found(word: str) -> bool:
        # Quoted or backticked: most words appear as string literals, but the two
        # streams are only ever named in the tree's documentation.
        return re.search(rf"[\"`]{re.escape(word)}[\"`]", everything) is not None

    categories = [
        {
            "id": cid,
            "label": label,
            "count": len(group),
            "words": group,
            "missing": [w for w in group if not found(w)],
        }
        for cid, label, group in WORDS
    ]
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

    OUT.parent.mkdir(exist_ok=True)
    OUT.write_text(json.dumps(data, indent=2) + "\n")

    print(f"read {sum(len(s) for s in sources.values()):,} bytes of Rust at {data['readFrom']['commit']}")
    print(f"  reserved {data['reserved']}, symbols {symbols} of {len(described)} kinds, "
          f"words {data['confirmed']}/{len(every_word)} across {len(categories)} categories")
    if missing:
        print(f"  NOT FOUND IN THE SOURCE: {', '.join(missing)}", file=sys.stderr)
    print(f"wrote {OUT.relative_to(pathlib.Path.cwd())} ({OUT.stat().st_size} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
