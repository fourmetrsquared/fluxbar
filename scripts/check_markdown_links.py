#!/usr/bin/env python3
"""Check relative links in repository Markdown files.

External URLs, anchors, and mailto links are skipped. Fenced code blocks are
ignored so command examples do not produce false positives.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path
from urllib.parse import unquote

ROOT = Path(__file__).resolve().parent.parent
SKIP_DIRS = {".git", "target", "dist", "node_modules"}
FENCE_RE = re.compile(r"^(```|~~~)")
LINK_RE = re.compile(
    r"!?\[[^\]]*\]\(\s*(?:<([^>]+)>|([^\s)]+))"
    r"(?:\s+[\"'][^\"']*[\"'])?\s*\)"
)
SCHEME_RE = re.compile(r"^[A-Za-z][A-Za-z0-9+.-]*:")


def strip_fenced_code(text: str) -> str:
    """Return Markdown text with fenced code blocks removed."""
    result: list[str] = []
    fence: str | None = None
    for line in text.splitlines():
        match = FENCE_RE.match(line.lstrip())
        if match:
            marker = match.group(1)
            if fence is None:
                fence = marker
            elif marker == fence:
                fence = None
            continue
        if fence is None:
            result.append(line)
    return "\n".join(result)


def markdown_files() -> list[Path]:
    files: list[Path] = []
    for path in ROOT.rglob("*.md"):
        relative = path.relative_to(ROOT)
        if any(part in SKIP_DIRS for part in relative.parts):
            continue
        files.append(path)
    return sorted(files)


def link_targets(text: str) -> list[str]:
    targets: list[str] = []
    for match in LINK_RE.finditer(strip_fenced_code(text)):
        targets.append(match.group(1) or match.group(2))
    return targets


def check_file(path: Path) -> list[str]:
    errors: list[str] = []
    text = path.read_text(encoding="utf-8")
    for target in link_targets(text):
        target = target.strip()
        if not target or target.startswith("#"):
            continue
        if SCHEME_RE.match(target) or target.startswith("//"):
            continue

        path_part = target.split("#", 1)[0].split("?", 1)[0]
        if not path_part:
            continue
        path_part = unquote(path_part)
        candidate = (
            ROOT / path_part.lstrip("/")
            if path_part.startswith("/")
            else path.parent / path_part
        )
        try:
            candidate.resolve(strict=False).relative_to(ROOT.resolve())
        except ValueError:
            errors.append(f"{path.relative_to(ROOT)}: link escapes the repository: {target}")
            continue
        if not candidate.exists():
            errors.append(f"{path.relative_to(ROOT)}: broken link: {target}")
    return errors


def main() -> int:
    files = markdown_files()
    errors: list[str] = []
    for path in files:
        errors.extend(check_file(path))

    if errors:
        print("Markdown link check failed:", file=sys.stderr)
        for error in errors:
            print(f"  {error}", file=sys.stderr)
        return 1

    print(f"Markdown links OK ({len(files)} files)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
