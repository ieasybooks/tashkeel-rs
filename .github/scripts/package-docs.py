#!/usr/bin/env python3
"""Stage release documentation and check relative Markdown file links offline."""

import argparse
from pathlib import Path
import re
import shutil
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[2]
# Keep repository paths so the original docs work both online and unpacked.
# Supporting sources are small; never copy a build directory or model cache.
PATTERNS = (
    "README.md",
    "LICENSE",
    "docs/**/*",
    ".github/RELEASING.md",
    ".github/scripts/*.py",
    "rust-toolchain.toml",
    "licenses/*",
    "crates/tashkeel/THIRD-PARTY-NOTICES.md",
    "crates/tashkeel/licenses/*",
    "crates/tashkeel/data/*.json",
    "crates/tashkeel/examples/benchmark.rs",
    "scripts/smoke-models.py",
    "scripts/model-export/*.py",
    "scripts/model-export/*.json",
    "scripts/model-export/requirements.txt",
)
INLINE_LINK = re.compile(r"\[[^\]\n]*\]\(\s*(<[^>\n]+>|[^\s)]+)")
REFERENCE_LINK = re.compile(r"^ {0,3}\[[^\]\n]+\]:\s*(<[^>\n]+>|\S+)", re.MULTILINE)
CODE_BLOCK = re.compile(r"^(`{3,}|~{3,})[^\n]*\n.*?^\1[ \t]*$", re.MULTILINE | re.DOTALL)


def stage(bundle):
    for pattern in PATTERNS:
        sources = sorted(path for path in ROOT.glob(pattern) if path.is_file())
        if not sources:
            raise SystemExit(f"No release documentation files matched {pattern}")
        for source in sources:
            destination = bundle / source.relative_to(ROOT)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)


def check_links(bundle):
    if not (bundle / "README.md").is_file():
        raise SystemExit(f"Missing release README: {bundle}")
    errors = []
    links = 0
    documents = sorted(bundle.rglob("*.md"))
    for document in documents:
        content = CODE_BLOCK.sub("", document.read_text(encoding="utf-8"))
        for target in INLINE_LINK.findall(content) + REFERENCE_LINK.findall(content):
            url = urlsplit(target.strip("<>"))
            if url.scheme or url.netloc or not url.path:
                continue
            links += 1
            destination = (document.parent / unquote(url.path)).resolve()
            if not destination.is_relative_to(bundle) or not destination.exists():
                errors.append(f"{document.relative_to(bundle)}: {target}")
    if errors:
        raise SystemExit("Unresolved release documentation links:\n" + "\n".join(errors))
    print(f"Release docs: {len(documents)} Markdown files, {links} relative file links: OK")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=Path, help="release staging or extracted archive directory")
    parser.add_argument("--check", action="store_true", help="check an existing bundle without copying files")
    args = parser.parse_args()
    bundle = args.bundle.resolve()
    if not args.check:
        stage(bundle)
    check_links(bundle)


if __name__ == "__main__":
    main()
