#!/usr/bin/env python3
"""Extract translatable strings from the BiGame-mode sources into a .pot file.

Why this exists rather than plain xgettext: xgettext has no Rust mode. Run
against Rust with `--language=C` it reads lifetimes (`&'a str`) as unterminated
character constants and bails; with `--language=Python` it mis-parses byte and
raw strings. A previous attempt to work around that produced a template
containing 34 strings, 36 of whose source references pointed into
`src/cargo-home/registry/` — the catalogues were carrying strings from the
gettext-rs crate itself and almost none of the application's own.

The application funnels every translatable string through one helper, `i18n`,
so a focused extractor is both simpler and more accurate than a general one. It
understands the Rust string literals actually used here: normal literals with
escapes, and raw literals (`r"…"`, `r#"…"#`).

Usage:
    locale/extract-strings.py                 # write locale/bigame-mode.pot
    locale/extract-strings.py --check         # exit 1 if the .pot is stale
"""

from __future__ import annotations

import argparse
import datetime
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
POTFILES = ROOT / "locale" / "POTFILES.in"
POT = ROOT / "locale" / "bigame-mode.pot"

# `i18n(` / `i18n (` followed by a string literal.
CALL = re.compile(r"\bi18n\s*\(\s*")
# Desktop/AppStream files: Name=, Comment=, GenericName=, Keywords=
DESKTOP_KEY = re.compile(r"^(Name|GenericName|Comment|Keywords)\s*=\s*(.+)$")
XML_TAG = re.compile(r"<(name|summary|caption|p)>([^<]+)</\1>")


def read_rust_literal(text: str, i: int) -> tuple[str, int] | None:
    """Read a Rust string literal starting at `i`. Returns (value, end index)."""
    # Raw literal: r"…" or r#"…"# (any number of hashes).
    if text[i] == "r":
        j = i + 1
        hashes = 0
        while j < len(text) and text[j] == "#":
            hashes += 1
            j += 1
        if j >= len(text) or text[j] != '"':
            return None
        terminator = '"' + "#" * hashes
        end = text.find(terminator, j + 1)
        if end < 0:
            return None
        return text[j + 1 : end], end + len(terminator)

    if text[i] != '"':
        return None

    out: list[str] = []
    j = i + 1
    while j < len(text):
        c = text[j]
        if c == "\\":
            if j + 1 >= len(text):
                return None
            nxt = text[j + 1]
            out.append({"n": "\n", "t": "\t", "r": "\r", "0": "\0"}.get(nxt, nxt))
            j += 2
            continue
        if c == '"':
            return "".join(out), j + 1
        out.append(c)
        j += 1
    return None


def escape_po(value: str) -> str:
    return (
        value.replace("\\", "\\\\")
        .replace('"', '\\"')
        .replace("\n", "\\n")
        .replace("\t", "\\t")
    )


def extract_rust(path: pathlib.Path) -> list[tuple[str, int]]:
    text = path.read_text(encoding="utf-8")
    found: list[tuple[str, int]] = []
    for match in CALL.finditer(text):
        literal = read_rust_literal(text, match.end())
        if literal is None:
            continue  # i18n(variable) — nothing static to extract
        value, _ = literal
        if value:
            found.append((value, text.count("\n", 0, match.start()) + 1))
    return found


def extract_desktop(path: pathlib.Path) -> list[tuple[str, int]]:
    found = []
    for n, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        # Skip entries that are already a translation (`Name[pt_BR]=`).
        if "[" in line.split("=", 1)[0]:
            continue
        m = DESKTOP_KEY.match(line.strip())
        if m and m.group(2).strip():
            found.append((m.group(2).strip(), n))
    return found


def extract_xml(path: pathlib.Path) -> list[tuple[str, int]]:
    found = []
    for n, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        for m in XML_TAG.finditer(line):
            value = m.group(2).strip()
            if value and not value.startswith("&"):
                found.append((value, n))
    return found


def build_pot() -> str:
    entries: dict[str, list[str]] = {}
    for rel in POTFILES.read_text(encoding="utf-8").split():
        path = ROOT / rel
        if not path.is_file():
            print(f"warning: {rel} listed in POTFILES.in does not exist", file=sys.stderr)
            continue
        if path.suffix == ".rs":
            found = extract_rust(path)
        elif path.suffix == ".desktop":
            found = extract_desktop(path)
        elif path.suffix == ".xml":
            found = extract_xml(path)
        else:
            continue
        for value, line in found:
            entries.setdefault(value, []).append(f"{rel}:{line}")

    date = datetime.datetime.now().astimezone().strftime("%Y-%m-%d %H:%M%z")
    out = [
        "# Translation template for BiGame-mode.",
        "# Copyright (C) BigLinux",
        "# This file is distributed under the same license as the bigame-mode package.",
        "#",
        'msgid ""',
        'msgstr ""',
        '"Project-Id-Version: bigame-mode\\n"',
        '"Report-Msgid-Bugs-To: https://github.com/ruscher/bigamemode/issues\\n"',
        f'"POT-Creation-Date: {date}\\n"',
        '"PO-Revision-Date: YEAR-MO-DA HO:MI+ZONE\\n"',
        '"Last-Translator: FULL NAME <EMAIL@ADDRESS>\\n"',
        '"Language-Team: LANGUAGE <LL@li.org>\\n"',
        '"Language: \\n"',
        '"MIME-Version: 1.0\\n"',
        '"Content-Type: text/plain; charset=UTF-8\\n"',
        '"Content-Transfer-Encoding: 8bit\\n"',
        "",
    ]
    # Sorted so regenerating without source changes produces no diff.
    for value in sorted(entries):
        for ref in sorted(set(entries[value])):
            out.append(f"#: {ref}")
        out.append(f'msgid "{escape_po(value)}"')
        out.append('msgstr ""')
        out.append("")
    return "\n".join(out)


def strip_date(text: str) -> str:
    return "\n".join(l for l in text.splitlines() if not l.startswith('"POT-Creation-Date:'))


def translatable(text: str) -> str:
    """What a translator sees: the strings, without timestamps or `#:`
    source references.

    The check compares this, not the whole file. Comparing references made
    any edit that moved a line -- with no string changed at all -- fail the
    package build, which is how main came to need a fix commit after a merge.
    A template whose references are stale still gives translators every
    string they need; one missing a string does not.
    """
    return "\n".join(
        l for l in strip_date(text).splitlines() if not l.startswith("#:")
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="exit non-zero if the template's strings are out of date "
        "(ignoring its timestamp and source line references)",
    )
    args = parser.parse_args()

    generated = build_pot()
    if args.check:
        if not POT.is_file():
            print("locale/bigame-mode.pot is missing", file=sys.stderr)
            return 1
        if translatable(POT.read_text(encoding="utf-8")) != translatable(generated):
            print(
                "locale/bigame-mode.pot is out of date; run locale/extract-strings.py",
                file=sys.stderr,
            )
            return 1
        print("locale/bigame-mode.pot is up to date")
        return 0

    POT.write_text(generated, encoding="utf-8")
    count = generated.count("\nmsgid ") - 0
    print(f"wrote {POT.relative_to(ROOT)} with {count} strings")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
