#!/usr/bin/env python3
"""Flatten install/occluview.wxs into the plain XML wixl accepts.

The MSI source is a WiX 3 document with the preprocessor directives the release
build uses (`<?define?>`, `<?ifndef?>`, `<?if?>`, `<?endif?>` and `$(var.X)`
substitutions). wixl implements the WiX schema but not that preprocessor, and
the release build on Windows runs WiX itself, so this script keeps the single
source of truth: it resolves the directives exactly as WiX does for the
definitions this file uses, and writes a document wixl can compile.

Usage:
    flatten-wxs.py <input.wxs> <output.wxs> [NAME=VALUE ...]
"""

import re
import sys
from pathlib import Path


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__)
        return 2
    source_path = Path(sys.argv[1])
    source = source_path.read_text(encoding="utf-8")
    destination = Path(sys.argv[2])
    definitions = {
        "SOURCEFILEDIR": str(source_path.resolve().parent) + "/",
    }
    for assignment in sys.argv[3:]:
        name, _, value = assignment.partition("=")
        definitions[name.strip()] = value

    output: list[str] = []
    # Each entry is (parent_active, this_condition_true).
    conditions: list[tuple[bool, bool]] = []

    def active() -> bool:
        return all(parent and taken for parent, taken in conditions)

    for line in source.splitlines():
        stripped = line.strip()

        directive = re.fullmatch(r"<\?(ifndef|ifdef|if|else|endif)\s*(.*?)\s*\?>", stripped)
        if directive:
            kind, argument = directive.group(1), directive.group(2)
            if kind == "ifndef":
                conditions.append((active(), argument not in definitions))
            elif kind == "ifdef":
                conditions.append((active(), argument in definitions))
            elif kind == "if":
                # Only `$(var.X) = value` is used by this file.
                match = re.fullmatch(r"\$\(var\.([A-Za-z0-9_]+)\)\s*=\s*(.*)", argument)
                if not match:
                    raise SystemExit(f"unsupported condition: {argument!r}")
                conditions.append((active(), definitions.get(match.group(1)) == match.group(2)))
            elif kind == "else":
                parent, taken = conditions.pop()
                conditions.append((parent, not taken))
            else:
                conditions.pop()
            continue

        define = re.fullmatch(r"<\?define\s+([A-Za-z0-9_]+)\s*=\s*\"(.*)\"\s*\?>", stripped)
        if define:
            # A definition inside a false branch is not defined, which is what
            # `<?ifndef?>` above depends on.
            if active():
                definitions.setdefault(define.group(1), define.group(2))
            continue

        if not active():
            continue

        def substitute(match: re.Match[str]) -> str:
            name = match.group(1) or match.group(2)
            if name not in definitions:
                raise SystemExit(f"undefined variable: {name}")
            return definitions[name]

        output.append(re.sub(r"\$\((?:var|sys)\.([A-Za-z0-9_]+)\)", substitute, line))

    destination.write_text(
        "\n".join(wixl_compatible(output, definitions.get("ProductVersion", ""))) + "\n",
        encoding="utf-8",
    )
    return 0


def wixl_compatible(lines: list[str], product_version: str) -> list[str]:
    """Drop or adjust what wixl does not implement.

    Every change is either cosmetic, expressed another way on the wixl command
    line, or reproduces what the WiX attributes this file carries mean. Anything
    that would change what is installed must fail loudly here rather than be
    silently dropped.

    * `Package/@Platform` — wixl takes the architecture with `--arch x64`.
    * `MajorUpgrade/@Schedule` — wixl has no such attribute and schedules the
      old product's removal before `InstallInitialize`. That is the one place
      where the difference bites: the uninstall action then runs during an
      upgrade, where `UPGRADINGPRODUCTCODE` is not set yet, cannot find the file
      it names, and aborts the package with error 2753. An explicit
      `RemoveExistingProducts` row after `InstallInitialize` reproduces the
      scheduling this file asks for. Both refresh actions are made tolerant as
      well: a shell-refresh that cannot run must never roll back a working
      install, and wixl writes no `File/@Version`, so a repair that skips the
      file would put the install action in the same position.
    * `WixVariable` — installer artwork and the licence page text; wixl draws
      its own UI from `UIRef`.
    * `ShortcutProperty` — the Start Menu shortcut's AppUserModelID, which only
      affects how Windows groups the taskbar button.
    * `UIRef WixUI_InstallDir` becomes `WixUI_Minimal`: wixl ships only the
      minimal UI set. The product, its files, registry entries and shortcuts are
      identical; the operator loses the folder-chooser page.
    * `File/@Version` — WiX reads it from the executable's VERSIONINFO; wixl
      writes an empty column whatever the source says, which is why the
      scheduling fix above matters. The attribute is added anyway so the source
      states the intent.
    * Backslashes in `Source` and `SourceFile` paths — the same files, written
      the way the platform wixl runs on spells them.
    """
    cleaned: list[str] = []
    for line in lines:
        if "<WixVariable" in line or "<ShortcutProperty" in line:
            continue
        line = line.replace(' Platform="x64"', "")
        line = line.replace(' Schedule="afterInstallInitialize"', "")
        if "Source=" in line or "SourceFile=" in line:
            line = line.replace("\\", "/")
        line = line.replace("WixUI_InstallDir", "WixUI_Minimal")
        cleaned.append(line)

    text = "\n".join(cleaned)

    # A shell refresh that cannot run must never roll back an installed viewer.
    # The uninstall action cannot resolve its file during an upgrade removal, and
    # wixl writes no File/@Version, so a repair that skips the file would leave
    # the install action in the same position. The refresh is a cache hint: the
    # operator sees stale icons for a moment instead of a failed installation.
    text = re.sub(
        r'(Id="RefreshShellAssociations(?:Uninstall|Install)")(.*?)(Return=")check(")',
        r"\1\2\3ignore\4",
        text,
        flags=re.DOTALL,
    )

    # Reproduce MajorUpgrade/@Schedule="afterInstallInitialize": the removal of
    # the old product belongs after InstallInitialize, where an upgrade removal
    # is recognisable as one.
    if "RemoveExistingProducts" not in text:
        text = text.replace(
            "    <InstallExecuteSequence>",
            "    <InstallExecuteSequence>\n"
            '      <RemoveExistingProducts After="InstallInitialize" />',
            1,
        )

    if product_version:
        def versioned(match: "re.Match[str]") -> str:
            element = match.group(0)
            if "Version=" in element:
                return element
            return element[: -len("/>")].rstrip() + f' Version="{product_version}" />'

        text = re.sub(
            r'<File\b[^>]*?Id="(?:filOccluViewExe|filOccluViewShellDll)"[^>]*?/>',
            versioned,
            text,
        )
    return text.split("\n")


if __name__ == "__main__":
    raise SystemExit(main())
