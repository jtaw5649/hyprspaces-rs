import os
import sys
import tempfile
import textwrap
import unittest

SCRIPT_DIR = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, SCRIPT_DIR)

import release  # noqa: E402


class ReleaseScriptTests(unittest.TestCase):
    def test_bump_version_from_zero(self):
        self.assertEqual(release.bump_version("0.9.0"), "1.0.0")

    def test_bump_version_patch(self):
        self.assertEqual(release.bump_version("1.2.3"), "1.2.4")

    def test_update_cargo_toml_version(self):
        content = textwrap.dedent(
            """
            [package]
            name = "hyprspaces"
            version = "0.9.0"
            edition = "2021"
            """
        ).lstrip()
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "Cargo.toml")
            with open(path, "w", encoding="utf-8") as handle:
                handle.write(content)
            release.update_cargo_toml(path, "1.0.0")
            with open(path, "r", encoding="utf-8") as handle:
                updated = handle.read()
        self.assertIn('version = "1.0.0"', updated)

    def test_update_pkgbuild_version(self):
        content = textwrap.dedent(
            """
            pkgname=hyprspaces
            pkgver=0.9.0
            pkgrel=1
            """
        ).lstrip()
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "PKGBUILD")
            with open(path, "w", encoding="utf-8") as handle:
                handle.write(content)
            release.update_pkgbuild(path, "1.0.0")
            with open(path, "r", encoding="utf-8") as handle:
                updated = handle.read()
        self.assertIn("pkgver=1.0.0", updated)

    def test_update_cargo_lock_version(self):
        content = textwrap.dedent(
            """
            [[package]]
            name = "hyprspaces"
            version = "0.9.0"
            dependencies = [
             "serde",
            ]
            """
        ).lstrip()
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "Cargo.lock")
            with open(path, "w", encoding="utf-8") as handle:
                handle.write(content)
            release.update_cargo_lock(path, "1.0.0")
            with open(path, "r", encoding="utf-8") as handle:
                updated = handle.read()
        self.assertIn('version = "1.0.0"', updated)

    def test_update_changelog_moves_unreleased(self):
        content = textwrap.dedent(
            """
            # Changelog

            ## [Unreleased]

            ### Added
            - Something new.

            ## [0.8.0] - 2024-01-01
            """
        ).lstrip()
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "CHANGELOG.md")
            with open(path, "w", encoding="utf-8") as handle:
                handle.write(content)
            release.update_changelog(path, "1.0.0", "2025-01-01")
            with open(path, "r", encoding="utf-8") as handle:
                updated = handle.read()
        self.assertIn("## [1.0.0] - 2025-01-01", updated)
        self.assertIn("- Something new.", updated)
        self.assertIn("## [Unreleased]", updated)

    def test_update_changelog_when_empty(self):
        content = textwrap.dedent(
            """
            # Changelog

            ## [Unreleased]

            ### Added

            ### Changed

            ### Fixed
            """
        ).lstrip()
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "CHANGELOG.md")
            with open(path, "w", encoding="utf-8") as handle:
                handle.write(content)
            release.update_changelog(path, "1.0.0", "2025-01-01")
            with open(path, "r", encoding="utf-8") as handle:
                updated = handle.read()
        self.assertIn("## [1.0.0] - 2025-01-01", updated)
        self.assertIn("- Automated release.", updated)


if __name__ == "__main__":
    unittest.main()
