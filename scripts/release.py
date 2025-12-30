#!/usr/bin/env python3
from __future__ import annotations

import argparse
from datetime import date
from pathlib import Path
import re

VERSION_RE = re.compile(r'^version\s*=\s*"(\d+\.\d+\.\d+)"', re.MULTILINE)
PKGVER_RE = re.compile(r'^pkgver=.*$', re.MULTILINE)
LOCK_RE = re.compile(
    r'(\[\[package\]\]\nname = "hyprspaces"\nversion = ")\d+\.\d+\.\d+("\n)',
    re.MULTILINE,
)


def bump_version(version: str, bump_type: str = "patch") -> str:
    parts = version.strip().split(".")
    if len(parts) != 3:
        raise ValueError(f"Invalid version: {version}")
    major, minor, patch = (int(part) for part in parts)
    if bump_type == "major":
        return f"{major + 1}.0.0"
    if bump_type == "minor":
        return f"{major}.{minor + 1}.0"
    return f"{major}.{minor}.{patch + 1}"


def read_cargo_version(path: str | Path) -> str:
    text = Path(path).read_text(encoding="utf-8")
    match = VERSION_RE.search(text)
    if not match:
        raise ValueError("No version found in Cargo.toml")
    return match.group(1)


def update_cargo_toml(path: str | Path, version: str) -> None:
    cargo_path = Path(path)
    text = cargo_path.read_text(encoding="utf-8")
    updated = VERSION_RE.sub(f'version = "{version}"', text, count=1)
    if text == updated:
        raise ValueError("Failed to update Cargo.toml version")
    cargo_path.write_text(updated, encoding="utf-8")


def update_pkgbuild(path: str | Path, version: str) -> None:
    pkgbuild_path = Path(path)
    text = pkgbuild_path.read_text(encoding="utf-8")
    updated = PKGVER_RE.sub(f"pkgver={version}", text, count=1)
    if text == updated:
        raise ValueError("Failed to update PKGBUILD version")
    pkgbuild_path.write_text(updated, encoding="utf-8")


def update_cargo_lock(path: str | Path, version: str) -> None:
    lock_path = Path(path)
    text = lock_path.read_text(encoding="utf-8")
    updated = LOCK_RE.sub(rf'\g<1>{version}\g<2>', text, count=1)
    if text == updated:
        raise ValueError("Failed to update Cargo.lock version")
    lock_path.write_text(updated, encoding="utf-8")


def update_changelog(path: str | Path, version: str, release_date: str) -> None:
    changelog_path = Path(path)
    text = changelog_path.read_text(encoding="utf-8")
    headers = list(re.finditer(r"^## \[.*?\]", text, re.MULTILINE))
    unreleased_index = next((i for i, match in enumerate(headers) if match.group(0) == "## [Unreleased]"), None)
    if unreleased_index is None:
        raise ValueError("No Unreleased section found")

    unreleased_start = headers[unreleased_index].end()
    unreleased_end = headers[unreleased_index + 1].start() if unreleased_index + 1 < len(headers) else len(text)
    unreleased_body = text[unreleased_start:unreleased_end].strip("\n")
    release_body = unreleased_body.strip()
    has_entries = False
    for line in release_body.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("### "):
            continue
        has_entries = True
        break
    if not release_body or not has_entries:
        release_body = "### Added\n- Automated release."

    new_unreleased = "\n\n### Added\n\n### Changed\n\n### Fixed\n"
    release_section = f"\n## [{version}] - {release_date}\n\n{release_body}\n"

    updated = (
        text[: headers[unreleased_index].start()]
        + "## [Unreleased]"
        + new_unreleased
        + release_section
        + text[unreleased_end:]
    )
    changelog_path.write_text(updated, encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description="Bump project version")
    parser.add_argument(
        "--bump-type",
        choices=["patch", "minor", "major"],
        default="patch",
        help="Version bump type (default: patch)",
    )
    args = parser.parse_args()

    root = Path(__file__).resolve().parents[1]
    cargo_path = root / "Cargo.toml"
    lock_path = root / "Cargo.lock"
    pkgbuild_path = root / "PKGBUILD"
    changelog_path = root / "CHANGELOG.md"

    current_version = read_cargo_version(cargo_path)
    new_version = bump_version(current_version, args.bump_type)

    update_cargo_toml(cargo_path, new_version)
    update_cargo_lock(lock_path, new_version)
    update_pkgbuild(pkgbuild_path, new_version)
    update_changelog(changelog_path, new_version, date.today().isoformat())

    print(new_version)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
