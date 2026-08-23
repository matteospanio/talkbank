#!/usr/bin/env python3
"""Extract the translatable strings from the Rust sources and merge them into po/.

xgettext cannot read Rust: lifetimes (`'static`) look to it like unterminated
character literals and it gives up. So we look for the `t("…")` and
`tn("…", "…", n)` calls directly, which are the only forms we use.

Usage:
    tools/extract-strings.py            list the new strings
    tools/extract-strings.py --merge    append them to po/it.po and po/en.po
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SOURCES = sorted((ROOT / "crates").glob("*/src/*.rs"))


def rust_string_at(text, i):
    """Read a Rust literal starting at `text[i] == '"'`.

    Returns (value_with_original_escapes, index_after_the_closing_quote).
    Handles escape sequences and backslash line continuation, which we use to
    break up long messages.
    """
    assert text[i] == '"'
    i += 1
    out = []
    while i < len(text):
        c = text[i]
        if c == "\\":
            nxt = text[i + 1] if i + 1 < len(text) else ""
            if nxt == "\n":
                # line continuation: swallow the newline and the indentation
                i += 2
                while i < len(text) and text[i] in " \t":
                    i += 1
                continue
            out.append(c + nxt)
            i += 2
            continue
        if c == '"':
            return "".join(out), i + 1
        out.append(c)
        i += 1
    raise ValueError("unterminated literal")


CALL = re.compile(r"\b(tn?)\s*\(\s*(?=\")")
# The catalogues (CLAN commands, Batchalign tasks) hold their texts as plain
# literals and translate them at use time with `t(cmd.title)`: without this
# second pattern they would be left out and nobody would notice.
FIELD = re.compile(r"^\s*(?:title|desc):\s*(?=\")", re.M)


def extract(path):
    text = path.read_text(encoding="utf-8")
    found = []
    for m in CALL.finditer(text):
        kind = m.group(1)
        try:
            first, after = rust_string_at(text, m.end())
        except ValueError:
            continue
        if kind == "t":
            found.append((first, None))
            continue
        # tn: find the second literal, skipping the comma and whitespace
        j = after
        while j < len(text) and text[j] in ", \t\n":
            j += 1
        if j < len(text) and text[j] == '"':
            second, _ = rust_string_at(text, j)
            found.append((first, second))

    for m in FIELD.finditer(text):
        try:
            value, _ = rust_string_at(text, m.end())
        except ValueError:
            continue
        found.append((value, None))
    return found


def po_entries(text):
    """The msgids present in a .po file."""
    return {
        "".join(re.findall(r'"(.*)"', block))
        for block in re.findall(r"^msgid ((?:\".*\"\n)+)", text, re.M)
    }


def main():
    all_found = []
    for src in SOURCES:
        all_found.extend(extract(src))

    seen = {}
    for msgid, plural in all_found:
        if msgid and msgid not in seen:
            seen[msgid] = plural

    it = (ROOT / "po/it.po").read_text(encoding="utf-8")
    known = po_entries(it)
    new = {k: v for k, v in seen.items() if k not in known}

    print(f"strings found: {len(seen)}  already translated: {len(seen) - len(new)}  new: {len(new)}")
    for k, v in new.items():
        print(f'  msgid "{k}"' + (f'\n  msgid_plural "{v}"' if v else ""))

    if "--merge" in sys.argv and new:
        for lang in ("it", "en"):
            path = ROOT / f"po/{lang}.po"
            text = path.read_text(encoding="utf-8").rstrip("\n")
            for k, v in new.items():
                if v:
                    text += f'\n\nmsgid "{k}"\nmsgid_plural "{v}"\nmsgstr[0] ""\nmsgstr[1] ""'
                else:
                    text += f'\n\nmsgid "{k}"\nmsgstr ""'
            path.write_text(text + "\n", encoding="utf-8")
        print(f"added {len(new)} empty entries to po/it.po and po/en.po")


if __name__ == "__main__":
    main()
