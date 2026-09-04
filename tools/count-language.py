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
    "crates/quench-qir/src/lib.rs",
]

# Groups the compiler can be asked for directly. These are not written down here
# at all — they are read out of the constant or the match that the compiler itself
# uses, so they cannot drift from the language however fast it moves.
#
# This is the direction that matters. A list that only checks its own members is
# exactly as stale as whoever last edited it: it notices a word being removed and
# is blind to one being added. `provided` went from 2 to 31 without a word of the
# old list becoming untrue, and nothing here could have seen it.
# The same eleven constants `quench words` reads, in the order it prints them.
# Nothing about the language is written down in this file any more: every word
# comes out of the place the compiler itself uses, so this cannot say something
# the language does not do.
#
# `quench words` is the authority, and reading the constants directly rather than
# running it is only so that this needs no build — a checker with a clone and no
# cargo can still tell whether the site has gone stale. If the shape of any
# constant changes, main's own CI runs `--check` against this and goes red.
PARSE = "crates/quench-parse/src/lib.rs"
CHECK = "crates/quench-check/src/lib.rs"
QIR = "crates/quench-qir/src/lib.rs"

LIST = r'"([^"]+)"'

DERIVED = [
    # (id, label, file, region, how a name is spelled inside it)
    ("statements", "statement|Statements", PARSE, r"pub const STATEMENTS[^=]*=\s*&\[(.*?)\];", LIST),
    ("topLevel", "top level|Top level", PARSE, r"pub const TOP_LEVEL[^=]*=\s*&\[(.*?)\];", LIST),
    ("afterABlock", "after a block|After a block", PARSE, r"pub const AFTER_A_BLOCK[^=]*=\s*&\[(.*?)\];", LIST),
    ("chainLinks", "chain link|Chain links", CHECK, r"pub const CHAIN_LINKS[^=]*=\s*&\[(.*?)\];", LIST),
    ("visibility", "visibility|Visibility", CHECK, r"pub const ALL: &\[&str\] = &\[(.*?)\];", LIST),
    ("types", "type|Types", CHECK, r"pub const NAMES: &\[&str\] = &\[(.*?)\];", LIST),
    ("operators", "operator|Operators", PARSE, r"pub const OPERATORS[^=]*=\s*&\[(.*?)\];", LIST),
    ("beforeAValue", "before a value|Before a value", PARSE, r"pub const BEFORE_A_VALUE[^=]*=\s*&\[(.*?)\];", LIST),
    ("literals", "literal|Literals", CHECK, r"pub const LITERALS[^=]*=\s*&\[(.*?)\];", LIST),
    ("streams", "stream|Streams", QIR, None, r'Stream::(?:Out|Err) => "([^"]+)"'),
]

# `provided` is not one constant any more: PROVIDED carries a module column, and
# MODULES says which modules exist. Both are read, and the groups are built from
# them rather than named here — a module added to the language becomes a group on
# the page without this file being touched.
CLI = "crates/quench-cli/src/main.rs"
MODULES_RE = r"pub const MODULES[^=]*=\s*&\[(.*?)\];"
PROVIDED_RE = r"pub const PROVIDED[^=]*=\s*&\[(.*?)\n\];"
PROVIDED_ENTRY = r'\(\s*"([^"]*)"\s*,\s*"([^"]+)"'
GROUP_CALL = r'group\(\s*"([^"]+)"'


# Nothing left. Every group has a constant behind it now, so there is no list here
# to go stale and no weak check to explain.
LISTED: list = []


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

    for cid, labels, path, block, name in DERIVED:
        cli_name, label = labels.split("|", 1)
        text = (QUENCH / path).read_text()
        if block is not None:
            region = re.search(block, text, re.S)
            if region is None:
                print(f"could not find the {cid} list in {path}", file=sys.stderr)
                return 1
            text = region.group(1)
        words = re.findall(name, text)
        if not words:
            print(f"the {cid} list in {path} came out empty", file=sys.stderr)
            return 1
        categories.append({
            "id": cid, "label": label, "cliName": cli_name, "count": len(words),
            "words": words, "missing": [], "readFrom": path,
        })

    # The provided functions, split the way the compiler splits them.
    check_src = (QUENCH / CHECK).read_text()
    mod_region = re.search(MODULES_RE, check_src, re.S)
    prov_region = re.search(PROVIDED_RE, check_src, re.S)
    if mod_region is None or prov_region is None:
        print("could not find MODULES or PROVIDED in quench-check", file=sys.stderr)
        return 1

    modules = re.findall(r'"([^"]+)"', mod_region.group(1))
    entries = re.findall(PROVIDED_ENTRY, prov_region.group(1))

    categories.append({
        "id": "providedModule", "label": "Provided modules", "cliName": "provided module",
        "count": len(modules), "words": modules, "missing": [], "readFrom": CHECK,
    })
    top = [word for held, word in entries if held == ""]
    categories.append({
        "id": "provided", "label": "Provided", "cliName": "provided",
        "count": len(top), "words": top, "missing": [], "readFrom": CHECK,
    })
    for module in modules:
        inside = [word for held, word in entries if held == module]
        categories.append({
            "id": f"provided_{module}", "label": f"Provided \u00b7 {module}",
            "cliName": f"provided {module}", "count": len(inside), "words": inside,
            "missing": [], "readFrom": CHECK,
        })

    # The guard that this file did not have, and the reason it produced nonsense
    # when the maths moved: the groups were a list kept here, so a group the
    # compiler grew was invisible. Now the CLI's own `words()` says which groups
    # exist, and anything it prints that is not built above stops the run.
    cli_src = (QUENCH / CLI).read_text()
    printed = set(re.findall(GROUP_CALL, cli_src)) | {f"provided {m}" for m in modules}
    built_names = {c["cliName"] for c in categories}
    if printed != built_names:
        for name in sorted(printed - built_names):
            print(f"the compiler prints a group this tool does not build: {name}", file=sys.stderr)
        for name in sorted(built_names - printed):
            print(f"this tool builds a group the compiler does not print: {name}", file=sys.stderr)
        return 1

    for cid, label, group in LISTED:
        categories.append({
            "id": cid, "label": label, "count": len(group), "words": group,
            "missing": [w for w in group if not found(w)], "readFrom": None,
        })

    order = [c for c in re.findall(GROUP_CALL, cli_src)]
    order += [f"provided {m}" for m in modules]
    categories.sort(key=lambda c: order.index(c["cliName"]) if c["cliName"] in order else 99)

    # A word can mean something in more than one position — `module` names a block
    # at the top level and also names how far a name reaches. It is one word the
    # language answers to, so the total counts it once and the groups both keep it.
    every_word = list(dict.fromkeys(w for c in categories for w in c["words"]))
    slots = sum(c["count"] for c in categories)
    missing = [w for c in categories for w in c["missing"]]

    # The token kinds, split the way the compiler's own error messages split them:
    # a kind whose name in a diagnostic is its own spelling in backticks is a
    # symbol, and one described in words is not. Nothing is classified here.
    tokens = [
        {
            "id": "symbols",
            "label": "Symbols",
            "words": [text.strip("`") for _, text in described if text.startswith("`")],
        },
        {
            "id": "otherKinds",
            "label": "The other kinds",
            "words": [text for _, text in described if not text.startswith("`")],
        },
    ]
    for group in tokens:
        group["count"] = len(group["words"])

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
        "placesAWordMeansSomething": slots,
        "confirmed": len(every_word) - len(missing),
        "missing": missing,
        "categories": categories,
        "tokens": tokens,
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
    dup = slots - len(every_word)
    also = f", {dup} of them in two groups" if dup else ""
    print(f"  words {len(every_word)} across {len(categories)} categories{also}")
    if missing:
        print(f"  NOT FOUND IN THE SOURCE: {', '.join(missing)}", file=sys.stderr)
    print(f"wrote {OUT.relative_to(pathlib.Path.cwd())} ({OUT.stat().st_size} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
