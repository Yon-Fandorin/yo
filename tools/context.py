#!/usr/bin/env python3
"""Read-only, bounded navigation over existing Markdown owners; no index or model."""

import argparse
import json
from pathlib import Path
import re
import subprocess
import sys
from urllib.parse import unquote, urlsplit

REPOSITORY_URL = "https://github.com/Yon-Fandorin/yo/blob/develop/"
STOP = {"a", "an", "and", "the", "to", "of", "for", "in", "change", "fix", "with", "or"}
MAX_DOCUMENT_BYTES = 1024 * 1024
MAX_CORPUS_BYTES = 4 * 1024 * 1024
MAX_OUTPUT_BYTES = 12 * 1024


def documents(root):
    paths = [root / "AGENTS.md", root / "CONTRIBUTING.md"]
    paths += sorted((root / "CONTRIBUTING").glob("*.md"))
    paths += sorted((root / "docs/src").rglob("*.md"))
    total = 0
    for path in paths:
        if not path.resolve().is_relative_to(root):
            raise ValueError(f"document escapes repository: {path}")
        size = path.stat().st_size
        total += size
        if size > MAX_DOCUMENT_BYTES or total > MAX_CORPUS_BYTES:
            raise ValueError("documentation exceeds the bounded corpus; narrow its owners")
        yield path.relative_to(root).as_posix(), path.read_text(encoding="utf-8")


def local_target(root, document, target):
    if target.startswith(REPOSITORY_URL):
        path = root / unquote(target[len(REPOSITORY_URL):].split("#", 1)[0])
    else:
        parsed = urlsplit(target)
        if parsed.scheme or target.startswith("//"):
            return None
        path = root / document if not parsed.path else root / Path(document).parent / unquote(parsed.path)
    resolved = path.resolve()
    if not resolved.is_relative_to(root):
        raise ValueError(f"local reference escapes repository: {document}: {target}")
    return resolved


def links(root, document, line):
    for match in re.finditer(r"\[[^\]\n]*\]\(([^)\n]+)\)", line):
        target = local_target(root, document, match.group(1))
        if target is not None:
            yield target


def words(value):
    value = re.sub(r"([a-z])([A-Z])", r"\1 \2", value)
    return set(re.findall(r"[^\W_]+", value.lower())) - STOP


def find(root, corpus, query):
    terms = words(query)
    if not terms:
        return []
    records = []
    for document, text in corpus:
        for number, line in enumerate(text.splitlines(), 1):
            # Existing route/check tables are the index. Never manufacture a
            # second mapping, rank boilerplate paragraphs, or load source code.
            if not line.startswith("|") or not list(links(root, document, line)) and "`" not in line:
                continue
            matched = sorted(terms & words(line))
            if matched:
                records.append({"document": document, "line": number,
                                "matched": matched, "outcome_matches": sorted(terms & words(line.split("|")[1])),
                                "excerpt": line[:800], "excerpt_truncated": len(line) > 800})
    return sorted(records, key=lambda r: (-len(r["outcome_matches"]), -len(r["matched"]),
                                         0 if r["document"].endswith("find-the-change.md") else 1,
                                         r["document"], r["line"]))


def changed_paths(root):
    result = []
    for args in (["diff", "--no-renames", "--name-only", "-z", "HEAD", "--"],
                 ["ls-files", "--others", "--exclude-standard", "-z"]):
        output = subprocess.run(["git", "-C", str(root), *args], check=True,
                                capture_output=True, timeout=10).stdout
        result.extend(part.decode("utf-8") for part in output.split(b"\0") if part)
    return result


def impact(root, corpus, paths):
    changed = []
    for path in paths:
        resolved = (root / path).resolve()
        if not resolved.is_relative_to(root):
            raise ValueError(f"changed path escapes repository: {path}")
        changed.append(resolved)
    records = []
    for document, text in corpus:
        for number, line in enumerate(text.splitlines(), 1):
            affected = set()
            for target in links(root, document, line):
                for path in changed:
                    if path == target or target.is_dir() and path.is_relative_to(target):
                        affected.add(path.relative_to(root).as_posix())
            if affected:
                records.append({"document": document, "line": number,
                                "changed": sorted(affected), "excerpt": line[:800],
                                "excerpt_truncated": len(line) > 800})
    return records


def check(root, corpus):
    broken = []
    for document, text in corpus:
        for number, line in enumerate(text.splitlines(), 1):
            for target in links(root, document, line):
                if not target.exists():
                    broken.append({"document": document, "line": number,
                                   "missing": target.relative_to(root).as_posix()})
    return broken


def bounded_result(action, records, limit):
    status = "broken" if action == "check" and records else "ok" if action == "check" else "matches" if records else "no_match"
    result = {"action": action, "status": status, "total": len(records),
              "truncated": len(records) > limit, "results": records[:limit],
              "scope": "Markdown routes and local file links only; no symbol graph, semantic freshness, or completeness proof"}
    while len(json.dumps(result, ensure_ascii=False).encode("utf-8")) + 1 > MAX_OUTPUT_BYTES:
        result["results"].pop()
        result["truncated"] = True
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["find", "impact", "check"])
    parser.add_argument("inputs", nargs="*")
    parser.add_argument("--changed", action="store_true", help="include staged, unstaged, deleted and untracked paths")
    parser.add_argument("--limit", type=int, default=3)
    args = parser.parse_args(argv)
    if not 1 <= args.limit <= 20:
        parser.error("--limit must be between 1 and 20")
    if args.changed and args.action != "impact":
        parser.error("--changed is only for impact")
    if args.action == "find" and not args.inputs or args.action == "impact" and not (args.inputs or args.changed):
        parser.error("supply a query or changed paths")
    if args.action == "check" and args.inputs:
        parser.error("check takes no paths")
    root = Path(__file__).resolve().parent.parent
    try:
        corpus = list(documents(root))
        if args.action == "find":
            records = find(root, corpus, " ".join(args.inputs))
        elif args.action == "impact":
            records = impact(root, corpus, args.inputs + (changed_paths(root) if args.changed else []))
        else:
            records = check(root, corpus)
        print(json.dumps(bounded_result(args.action, records, args.limit), ensure_ascii=False))
        return int(args.action == "check" and bool(records))
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(json.dumps({"status": "error", "message": str(error)[:1000]}), file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
