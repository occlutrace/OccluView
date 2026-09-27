use super::{
    owns_extension, APP_EXE_NAME, DEDICATED_FILE_ICON_EXTENSIONS, OFFERED_ONLY_EXTENSIONS,
    PREVIEW_HANDLER_CATEGORY, SUPPORTED_EXTENSIONS, THUMBNAIL_PROVIDER_CATEGORY,
};
use roxmltree::{Document, Node};
use serde_yaml_ng::Value;
use std::path::PathBuf;

fn repo_file(path: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::read_to_string(root.join(path)).expect("repository contract input is readable")
}

fn registry_key<'a, 'input>(document: &'a Document<'input>, key: &str) -> Option<Node<'a, 'input>> {
    document.descendants().find(|node| {
        node.is_element()
            && node.tag_name().name() == "RegistryKey"
            && node
                .attribute("Key")
                .is_some_and(|value| expand_wix_value(document, value) == key)
    })
}

fn expand_wix_value(document: &Document<'_>, value: &str) -> String {
    let mut expanded = value.to_owned();
    for instruction in document.descendants().filter_map(|node| node.pi()) {
        let Some(definition) = instruction.value.filter(|_| instruction.target == "define") else {
            continue;
        };
        let Some((name, value)) = definition.split_once('=') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim().trim_matches('"');
        expanded = expanded.replace(&format!("$(var.{name})"), value);
    }
    expanded
}

fn registry_value(document: &Document<'_>, key: &str, name: Option<&str>) -> Option<String> {
    let key = registry_key(document, key)?;
    key.children()
        .filter(|node| node.is_element() && node.tag_name().name() == "RegistryValue")
        .find(|node| node.attribute("Name") == name)
        .and_then(|node| node.attribute("Value"))
        .map(str::to_owned)
}

fn wix_element_has_id(document: &Document<'_>, element: &str, id: &str) -> bool {
    document.descendants().any(|node| {
        node.is_element() && node.tag_name().name() == element && node.attribute("Id") == Some(id)
    })
}

fn registry_section_exists(registry: &str, key: &str) -> bool {
    let expected = format!("[{key}]");
    registry.lines().map(str::trim).any(|line| line == expected)
}

fn registry_string_value(registry: &str, key: &str, name: &str) -> Option<String> {
    let mut in_section = false;
    for line in registry.lines().map(str::trim) {
        if let Some(section) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            in_section = section == key;
            continue;
        }
        if !in_section || line.is_empty() || line.starts_with(';') {
            continue;
        }
        let Some((field, value)) = line.split_once('=') else {
            continue;
        };
        if field.trim().trim_matches('"') == name {
            return Some(value.trim().trim_matches('"').to_owned());
        }
    }
    None
}

fn package_workflow() -> Value {
    serde_yaml_ng::from_str(&repo_file(".github/workflows/package-msi.yml"))
        .expect("package workflow is valid YAML")
}

fn ci_workflow() -> Value {
    serde_yaml_ng::from_str(&repo_file(".github/workflows/ci.yml"))
        .expect("CI workflow is valid YAML")
}

fn workflow_step<'a>(workflow: &'a Value, job: &str, name: &str) -> Option<&'a Value> {
    workflow["jobs"][job]["steps"]
        .as_sequence()?
        .iter()
        .find(|step| step["name"].as_str() == Some(name))
}

const CI_MACOS_PACKAGE_SMOKE: &str = r###"bash install/macos/build-app.sh --no-build
bash install/macos/build-dmg.sh --no-build
bash install/macos/build-pkg.sh --no-build
dmg="$(find target/macos -maxdepth 1 -type f -name 'OccluView-*-aarch64.dmg' -print -quit)"
pkg="$(find target/macos -maxdepth 1 -type f -name 'OccluView-*-aarch64.pkg' -print -quit)"
test -n "$dmg" && test -n "$pkg"
hdiutil verify "$dmg"
pkgutil --payload-files "$pkg" | grep -F './OccluView.app/Contents/MacOS/occluview'
lipo -archs target/macos/OccluView.app/Contents/MacOS/occluview | grep -Fx arm64
for notice in LICENSE NOTICE THIRD-PARTY-NOTICES.md THIRD-PARTY-NOTICES-NATIVE.md; do
  test -s "target/macos/OccluView.app/Contents/Resources/Legal/$notice"
done
version="$(sed -n '/^\[workspace\.package\]/,/^\[/p' Cargo.toml \
  | sed -n 's/^version *= *"\(.*\)"/\1/p' | head -n1)"
test -n "$version"
target/macos/OccluView.app/Contents/MacOS/occluview --version | grep -F "$version"
target/macos/OccluView.app/Contents/Helpers/occluview-cli --version | grep -F "$version"
"###;

const PORTABLE_ZIP_BUILD: &str = r###"$cargoText = Get-Content ./Cargo.toml -Raw
$match = [regex]::Match($cargoText, '(?s)\[workspace\.package\].*?version\s*=\s*"([^"]+)"')
if (-not $match.Success) { throw "Could not find workspace package version." }
$version = $match.Groups[1].Value
$target = "x86_64-pc-windows-msvc"
$buildDir = Join-Path $pwd "target\$target\release-unwind"
$portableRoot = Join-Path $env:RUNNER_TEMP "OccluView"
$portableDir = Join-Path $portableRoot "OccluView"
Remove-Item $portableRoot -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $portableDir -Force | Out-Null
Copy-Item (Join-Path $buildDir "occluview.exe") $portableDir
Copy-Item (Join-Path $buildDir "occluview_shell.dll") $portableDir
Copy-Item ./LICENSE $portableDir
Copy-Item ./NOTICE $portableDir
Copy-Item ./THIRD-PARTY-NOTICES.md $portableDir
Copy-Item ./THIRD-PARTY-NOTICES-NATIVE.md $portableDir
Copy-Item ./README.md $portableDir
Compress-Archive -Path $portableDir -DestinationPath "./dist/OccluView-$version-$target-portable.zip" -CompressionLevel Optimal -Force
"###;

const WINDOWS_LIFECYCLE_SMOKE: &str = r###"$releaseMsi = Get-ChildItem ./dist -Filter *.msi | Sort-Object LastWriteTime -Descending | Select-Object -First 1
$upgradeMsi = Get-ChildItem (Join-Path $env:RUNNER_TEMP "occluview-msi-upgrade") -Filter *.msi | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not [string]::IsNullOrWhiteSpace($env:OCCLUVIEW_LEGACY_MSI_PATH)) {
  ./install/test-msi-lifecycle.ps1 -MsiPath $releaseMsi.FullName -LegacyUpgradeMsiPath $env:OCCLUVIEW_LEGACY_MSI_PATH -UpgradeMsiPath $upgradeMsi.FullName -DowngradeMsiPath $releaseMsi.FullName
} else {
  ./install/test-msi-lifecycle.ps1 -MsiPath $releaseMsi.FullName -UpgradeMsiPath $upgradeMsi.FullName -DowngradeMsiPath $releaseMsi.FullName
}
"###;

const DEBIAN_PACKAGE_VALIDATION: &str = r###"desktop-file-validate install/linux/ai.occlutrace.OccluView.desktop
appstreamcli validate --no-net install/linux/ai.occlutrace.OccluView.metainfo.xml
xmllint --noout install/linux/occluview-mime.xml install/linux/ai.occlutrace.OccluView.metainfo.xml
dpkg-deb --info "$DEB"
dpkg-deb --contents "$DEB"
install/linux/check-deb.sh "$DEB"
"###;

const MACOS_PACKAGE_VERIFY: &str = r###"version="$(sed -n '/^\[workspace\.package\]/,/^\[/p' Cargo.toml \
  | sed -n 's/^version *= *"\(.*\)"/\1/p' | head -n1)"
dmg="target/macos/OccluView-$version-aarch64.dmg"
pkg="target/macos/OccluView-$version-aarch64.pkg"
hdiutil verify "$dmg"
pkgutil --payload-files "$pkg" | grep -F './OccluView.app/Contents/MacOS/occluview'
lipo -archs target/macos/OccluView.app/Contents/MacOS/occluview | grep -Fx arm64
for notice in LICENSE NOTICE THIRD-PARTY-NOTICES.md THIRD-PARTY-NOTICES-NATIVE.md; do
  test -s "target/macos/OccluView.app/Contents/Resources/Legal/$notice"
done
target/macos/OccluView.app/Contents/MacOS/occluview --version | grep -F "$version"
target/macos/OccluView.app/Contents/Helpers/occluview-cli --version | grep -F "$version"
(cd target/macos && for file in *.dmg *.pkg; do shasum -a 256 "$file" > "$file.sha256"; done)
"###;

const PACKAGE_MACOS_BUILD_SELECTION: &str = r###"if [[ "${{ steps.signing.outputs.configured }}" == true ]]; then
  OCCLUVIEW_NOTARY_KEY_PATH="$RUNNER_TEMP/notary.p8" bash install/macos/sign-and-notarize.sh
  echo "notarized=true" >> "$GITHUB_OUTPUT"
else
  bash install/macos/build-dmg.sh --no-build
  bash install/macos/build-pkg.sh --no-build
  echo "notarized=false" >> "$GITHUB_OUTPUT"
fi
"###;

const RELEASE_NOTES_SCRIPT: &str = r###"version="${RELEASE_TAG#v}"
changelog_section="$(mktemp)"
awk -v version="$version" '
  index($0, "## " version " ") == 1 { found = 1; next }
  found && /^## / { exit }
  found { print }
' CHANGELOG.md > "$changelog_section"
if [[ ! -s "$changelog_section" ]]; then
  echo "CHANGELOG.md has no '## $version' section." >&2
  rm -f "$changelog_section"
  exit 1
fi
{
  printf 'OccluView %s\n\n' "$RELEASE_TAG"
  printf '%s\n\n' '## Download for Windows'
  printf '%s\n' '- **OccluView-Windows-Setup.msi** — recommended. Installs the viewer, Explorer previews, thumbnails, and file associations.'
  printf '%s\n\n' '- **OccluView-Windows-Portable.zip** — runs without installation; no Explorer integration.'
  if [[ "$MACOS_NOTARIZED" == true ]]; then
    printf '%s\n\n' '## Download for macOS (Apple Silicon, macOS 14 or later)'
    printf '%s\n' '- **OccluView-macOS-AppleSilicon.dmg** — open it and drag OccluView to Applications.'
    printf '%s\n\n' '- **OccluView-macOS-AppleSilicon.pkg** — installer package; the in-app updater uses this one.'
  fi
  if [[ -f dist/occluview-shell-revision.json ]]; then
    printf '%s\n\n' "- Explorer shell: $(python3 -c 'import json; print(json.load(open("dist/occluview-shell-revision.json"))["summary"])')."
  fi
  cat "$changelog_section"
  printf '\nThe verification archive is for technical checks; regular users do not need it.\n'
} > dist/release-notes.md
rm -f "$changelog_section"
"###;

const RELEASE_VERIFICATION_BUNDLE: &str = r###"version="${RELEASE_TAG#v}"
cp ./occluview.pub ./dist/occluview.pub
(
  cd ./dist
  shopt -s nullglob
  material=( *.sha256 *.minisig latest.json sbom-*.json occluview.pub )
  if [[ "${#material[@]}" -eq 0 ]]; then
    echo "No verification material was produced." >&2
    exit 1
  fi
  zip -q "OccluView-${version}-verification.zip" "${material[@]}"
)
"###;

const SIGNATURE_VERIFICATION_SCRIPT: &str = r###"# Verify with the public key compiled into installed copies.
pubkey=$(sed -n 's/^pub const UPDATE_PUBKEY: &str = "\(.*\)";$/\1/p' \
  crates/occluview-update/src/lib.rs)
if [[ -z "$pubkey" ]]; then
  echo "Could not read UPDATE_PUBKEY from crates/occluview-update/src/lib.rs." >&2
  exit 1
fi
for file in dist/latest.json $(find ./dist -type f -name '*.minisig' -not -name 'latest.json.minisig' | sed 's/\.minisig$//'); do
  minisign -V -P "$pubkey" -m "$file"
  echo "Verified against the shipped public key: $file"
done
"###;

const WINDOWS_SBOM_GENERATION: &str = r###"cargo install cargo-cyclonedx --version 0.5.8 --locked
# The virtual workspace emits one SBOM beside each member manifest.
cargo metadata --locked --format-version 1
cargo cyclonedx --format json --override-filename sbom-windows
$sbom = "crates/occluview-app/sbom-windows.json"
if (-not (Test-Path $sbom)) {
  throw "cargo-cyclonedx produced no SBOM at $sbom."
}
$described = (Get-Content $sbom -Raw | ConvertFrom-Json).metadata.component.name
if ($described -ne "occluview-app") {
  throw "SBOM describes '$described', not the shipped viewer."
}
git diff --exit-code -- Cargo.lock
Move-Item $sbom ./dist/sbom-windows.json -Force
"###;

const LINUX_SBOM_GENERATION: &str = r###"cargo install cargo-cyclonedx --version 0.5.8 --locked
cargo metadata --locked --format-version 1
cargo cyclonedx --format json --override-filename sbom-linux
sbom=crates/occluview-app/sbom-linux.json
if [[ ! -f "$sbom" ]]; then
  echo "cargo-cyclonedx produced no SBOM at $sbom." >&2
  exit 1
fi
python3 - "$sbom" <<'PY'
import json, sys
described = json.load(open(sys.argv[1], encoding="utf-8"))["metadata"]["component"]["name"]
if described != "occluview-app":
    raise SystemExit(f"SBOM describes {described!r}, not the shipped viewer.")
PY
git diff --exit-code -- Cargo.lock
cp "$sbom" target/deb/sbom-linux.json
"###;

fn step_runs(workflow: &Value, job: &str, name: &str, command: &str) -> bool {
    workflow_step(workflow, job, name)
        .and_then(|step| step["run"].as_str())
        .is_some_and(|run| run.trim() == command.trim())
}

fn any_step_runs(workflow: &Value, job: &str, command: &str) -> bool {
    workflow["jobs"][job]["steps"]
        .as_sequence()
        .is_some_and(|steps| {
            steps.iter().any(|step| {
                step["run"]
                    .as_str()
                    .is_some_and(|run| run.trim() == command.trim())
            })
        })
}

fn checkout_fetch_depth(workflow: &Value, job: &str) -> Option<u64> {
    workflow["jobs"][job]["steps"]
        .as_sequence()?
        .iter()
        .find(|step| {
            step["uses"].as_str()
                == Some("actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0")
        })
        .and_then(|step| step["with"]["fetch-depth"].as_u64())
}

fn assert_windows_package_modes_and_legacy_migration(package: &Value) {
    let inputs = &package["on"]["workflow_dispatch"]["inputs"];
    let configurations: Vec<_> = inputs["windows_configuration"]["options"]
        .as_sequence()
        .expect("the Windows profile is a workflow choice")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(configurations, ["release", "diagnostic"]);
    assert_eq!(
        inputs["windows_configuration"]["default"].as_str(),
        Some("release")
    );
    for input in [
        "windows_msi_version",
        "legacy_msi_run_id",
        "legacy_msi_sha256",
    ] {
        assert_eq!(inputs[input]["type"].as_str(), Some("string"));
        assert_eq!(inputs[input]["default"].as_str(), Some(""));
    }

    let build = workflow_step(package, "windows-package", "Build MSI")
        .expect("the Windows package job builds its configured MSI");
    assert_eq!(
        build["env"]["OCCLUVIEW_WINDOWS_CONFIGURATION"].as_str(),
        Some("${{ inputs.windows_configuration }}")
    );
    assert_eq!(
        build["env"]["OCCLUVIEW_WINDOWS_MSI_VERSION"].as_str(),
        Some("${{ inputs.windows_msi_version }}")
    );

    for name in [
        "Build portable ZIP",
        "Build upgrade smoke MSIs",
        "Validate optional legacy MSI migration inputs",
        "Download optional legacy MSI migration artifact",
        "Verify optional legacy MSI migration artifact",
        "Smoke install and uninstall",
    ] {
        let step = workflow_step(package, "windows-package", name)
            .expect("Windows packaging includes each standard package step");
        assert_eq!(
            step["if"].as_str(),
            Some("inputs.windows_configuration != 'diagnostic'")
        );
    }

    let download = workflow_step(
        package,
        "windows-package",
        "Download optional legacy MSI migration artifact",
    )
    .expect("legacy migration downloads a pinned run artifact");
    assert_eq!(
        download["uses"].as_str(),
        Some("actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c")
    );
    assert_eq!(
        download["with"]["run-id"].as_str(),
        Some("${{ inputs.legacy_msi_run_id }}")
    );
    assert_eq!(
        download["with"]["repository"].as_str(),
        Some("${{ github.repository }}")
    );
    let verify = workflow_step(
        package,
        "windows-package",
        "Verify optional legacy MSI migration artifact",
    )
    .expect("the downloaded legacy MSI is verified before installation");
    assert_eq!(
        verify["env"]["OCCLUVIEW_LEGACY_MSI_RUN_ID"].as_str(),
        Some("${{ inputs.legacy_msi_run_id }}")
    );
    assert_eq!(
        verify["env"]["OCCLUVIEW_LEGACY_MSI_SHA256"].as_str(),
        Some("${{ inputs.legacy_msi_sha256 }}")
    );
    let diagnostic = workflow_step(package, "windows-package", "Diagnostic MSI lifecycle smoke")
        .expect("the diagnostic MSI uses the lifecycle smoke");
    assert_eq!(
        diagnostic["if"].as_str(),
        Some("inputs.windows_configuration == 'diagnostic'")
    );
}

fn progid_extension(extension: &str) -> String {
    if extension == "dcm" {
        "HPS".to_owned()
    } else {
        extension.to_ascii_uppercase()
    }
}

fn assert_supported_registry_entry(wix: &Document<'_>, extension: &str) {
    assert!(
        occluview_formats::probe::by_extension(extension).is_some(),
        "Explorer must not advertise an extension without a reader: {extension}"
    );
    let class = progid_extension(extension);
    let progid = format!("MeshFile.{class}");
    let progid_key = format!("Software\\Classes\\{progid}");
    let open_with_key = format!("Software\\Classes\\.{extension}\\OpenWithProgids");
    assert_eq!(
        registry_value(wix, &open_with_key, Some(progid.as_str())).as_deref(),
        Some(""),
        "the MSI Open with entry must resolve to {progid}"
    );
    assert_eq!(
        registry_value(wix, &progid_key, None).as_deref(),
        Some(format!("{class} File").as_str()),
        "the registered file type name must describe {extension}"
    );

    for category in [THUMBNAIL_PROVIDER_CATEGORY, PREVIEW_HANDLER_CATEGORY] {
        let key = format!("{progid_key}\\ShellEx\\{category}");
        assert!(registry_key(wix, &key).is_some());
    }
    let dot_extension_key = format!("Software\\Classes\\.{extension}");
    if owns_extension(extension) {
        assert_eq!(
            registry_value(wix, &dot_extension_key, None).as_deref(),
            Some(progid.as_str()),
            "the owned extension must resolve to its Open with ProgID"
        );
        for category in [THUMBNAIL_PROVIDER_CATEGORY, PREVIEW_HANDLER_CATEGORY] {
            let direct = format!("{dot_extension_key}\\ShellEx\\{category}");
            let system = format!(
                "Software\\Classes\\SystemFileAssociations\\.{extension}\\ShellEx\\{category}"
            );
            assert!(registry_key(wix, &direct).is_some());
            assert!(registry_key(wix, &system).is_some());
        }
    } else {
        assert_eq!(registry_value(wix, &dot_extension_key, None), None);
        assert!(registry_key(wix, &format!("{dot_extension_key}\\DefaultIcon")).is_none());
        for category in [THUMBNAIL_PROVIDER_CATEGORY, PREVIEW_HANDLER_CATEGORY] {
            let direct = format!("{dot_extension_key}\\ShellEx\\{category}");
            let system = format!(
                "Software\\Classes\\SystemFileAssociations\\.{extension}\\ShellEx\\{category}"
            );
            assert!(registry_key(wix, &direct).is_none());
            assert!(registry_key(wix, &system).is_none());
        }
    }

    let edit_key =
        format!("Software\\Classes\\SystemFileAssociations\\.{extension}\\shell\\OccluView.Edit");
    assert!(registry_key(wix, &edit_key).is_some());
    assert!(registry_key(wix, &format!("{edit_key}\\command")).is_some());

    if DEDICATED_FILE_ICON_EXTENSIONS.contains(&extension) && extension != "dcm" {
        let icon_key = format!("{progid_key}\\DefaultIcon");
        assert_eq!(
            registry_value(wix, &icon_key, None).as_deref(),
            Some("[INSTALLFOLDER]occluview-3d.ico")
        );
    }
}

#[test]
fn explorer_open_with_and_icon_entries_match_the_reader_set() {
    let source = repo_file("install/occluview.wxs");
    let wix = Document::parse(&source).expect("WiX source is well-formed XML");
    assert_eq!(
        wix_variable(&wix, "ThumbnailProviderCategory"),
        Some(THUMBNAIL_PROVIDER_CATEGORY.to_owned())
    );
    assert_eq!(
        wix_variable(&wix, "PreviewHandlerCategory"),
        Some(PREVIEW_HANDLER_CATEGORY.to_owned())
    );
    for (component, file) in [
        ("cmpLicenseFile", "filLicenseFile"),
        ("cmpNoticeFile", "filNoticeFile"),
        ("cmpThirdPartyNotices", "filThirdPartyNotices"),
        ("cmpThirdPartyNoticesNative", "filThirdPartyNoticesNative"),
    ] {
        assert!(wix_element_has_id(&wix, "Component", component));
        assert!(wix_element_has_id(&wix, "File", file));
        assert!(wix_element_has_id(&wix, "ComponentRef", component));
    }
    let major_upgrade = wix
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "MajorUpgrade")
        .expect("the MSI declares its upgrade policy");
    assert_eq!(
        major_upgrade.attribute("Schedule"),
        Some("afterInstallInitialize")
    );
    assert_ne!(
        major_upgrade.attribute("AllowSameVersionUpgrades"),
        Some("yes")
    );

    for extension in SUPPORTED_EXTENSIONS {
        assert_supported_registry_entry(&wix, extension);
    }

    assert_eq!(
        OFFERED_ONLY_EXTENSIONS,
        [occluview_formats::LEGACY_HPS_EXTENSION]
    );
    assert!(!owns_extension("dcm"));
    for extension in ["stl", "ply", "obj", "glb", "hps"] {
        assert!(owns_extension(extension));
    }
    for forbidden in [
        "Software\\Classes\\.dcm",
        "Software\\Classes\\.dcm\\DefaultIcon",
        "Software\\Classes\\.dcm\\ShellEx",
        "Software\\Classes\\SystemFileAssociations\\.dcm\\ShellEx",
    ] {
        assert!(
            registry_key(&wix, forbidden).is_none(),
            "the installer must not claim medical DICOM files through {forbidden}"
        );
    }
    assert!(registry_key(
        &wix,
        "Software\\Classes\\SystemFileAssociations\\.dcm\\shell\\OccluView.Edit"
    )
    .is_some());
    assert!(
        registry_key(
            &wix,
            &format!("Software\\Classes\\Applications\\{APP_EXE_NAME}")
        )
        .is_some(),
        "the Open with executable entry must name the shipped viewer"
    );

    assert_manual_registry_contract(&wix);
}

fn assert_manual_registry_contract(wix: &Document<'_>) {
    let manual = repo_file("install/occluview-shell-registration.reg");
    let preview_app_id = "{6D2B5079-2F0B-48DD-AB7F-97CEC514D30B}";
    assert_eq!(
        wix_variable(wix, "PrevhostAppId").as_deref(),
        Some(preview_app_id)
    );
    assert_eq!(
        registry_string_value(
            &manual,
            "HKEY_CLASSES_ROOT\\CLSID\\{9F3A1B2C-4D5E-4F60-8A7B-9C0D1E2F3046}",
            "AppID"
        )
        .as_deref(),
        Some(preview_app_id),
        "manual shell registration uses Windows' Prevhost AppID"
    );
    assert!(!registry_section_exists(
        &manual,
        &format!("HKEY_CLASSES_ROOT\\AppID\\{preview_app_id}")
    ));
    for extension in SUPPORTED_EXTENSIONS {
        let dot_extension = format!("HKEY_CLASSES_ROOT\\.{extension}");
        if owns_extension(extension) {
            assert_eq!(
                registry_string_value(&manual, &dot_extension, "@"),
                Some(format!("MeshFile.{}", progid_extension(extension)))
            );
        } else {
            assert_eq!(registry_string_value(&manual, &dot_extension, "@"), None);
            assert!(!registry_section_exists(
                &manual,
                &format!("{dot_extension}\\DefaultIcon")
            ));
            for category in [THUMBNAIL_PROVIDER_CATEGORY, PREVIEW_HANDLER_CATEGORY] {
                assert!(!registry_section_exists(
                    &manual,
                    &format!("{dot_extension}\\ShellEx\\{category}")
                ));
                assert!(!registry_section_exists(
                    &manual,
                    &format!(
                        "HKEY_CLASSES_ROOT\\SystemFileAssociations\\.{extension}\\ShellEx\\{category}"
                    )
                ));
            }
        }
    }
    assert_eq!(
        registry_string_value(
            &manual,
            "HKEY_CLASSES_ROOT\\.dcm\\OpenWithProgids",
            "MeshFile.HPS"
        ),
        Some(String::new()),
        ".dcm remains available through the explicit Open With menu"
    );
    assert_eq!(
        registry_string_value(
            &manual,
            "HKEY_CLASSES_ROOT\\Applications\\occluview.exe\\SupportedTypes",
            ".dcm"
        ),
        Some(String::new()),
        ".dcm remains visible in Windows app discovery"
    );
}

fn wix_variable(document: &Document<'_>, name: &str) -> Option<String> {
    document
        .descendants()
        .filter_map(|node| node.pi())
        .filter(|instruction| instruction.target == "define")
        .filter_map(|instruction| instruction.value)
        .filter_map(|definition| definition.split_once('='))
        .find_map(|(defined_name, value)| {
            (defined_name.trim() == name).then(|| value.trim().trim_matches('"').to_owned())
        })
}

#[test]
fn linux_mime_metadata_distinguishes_hps_dcm_from_medical_dicom() {
    let source = repo_file("install/linux/occluview-mime.xml");
    let mime = Document::parse(&source).expect("shared MIME metadata is well-formed XML");
    let hps = mime
        .descendants()
        .find(|node| {
            node.is_element()
                && node.tag_name().name() == "mime-type"
                && node.attribute("type") == Some("application/x-occluview-hps")
        })
        .expect("the HPS MIME type is declared");

    for pattern in ["*.dcm", "*.DCM"] {
        assert!(hps.children().any(|node| {
            node.is_element()
                && node.tag_name().name() == "glob"
                && node.attribute("pattern") == Some(pattern)
                && node.attribute("weight") == Some("40")
        }));
    }
    for pattern in ["*.hps", "*.HPS"] {
        assert!(hps.children().any(|node| {
            node.is_element()
                && node.tag_name().name() == "glob"
                && node.attribute("pattern") == Some(pattern)
        }));
    }
    assert!(hps.descendants().any(|node| {
        node.is_element()
            && node.tag_name().name() == "magic"
            && node.attribute("priority") == Some("60")
            && node.children().any(|matcher| {
                matcher.is_element()
                    && matcher.tag_name().name() == "match"
                    && matcher.attribute("value") == Some("<HPS")
                    && matcher.attribute("offset") == Some("0:256")
            })
    }));

    let wavefront = mime
        .descendants()
        .find(|node| {
            node.is_element()
                && node.tag_name().name() == "mime-type"
                && node.attribute("type") == Some("model/obj")
        })
        .expect("the Wavefront MIME type is declared");
    for pattern in ["*.obj", "*.OBJ"] {
        assert!(wavefront.children().any(|node| {
            node.is_element()
                && node.tag_name().name() == "glob"
                && node.attribute("pattern") == Some(pattern)
                && node.attribute("weight") == Some("60")
        }));
    }

    let ply = mime
        .descendants()
        .find(|node| {
            node.is_element()
                && node.tag_name().name() == "mime-type"
                && node.attribute("type") == Some("application/x-ply")
        })
        .expect("the PLY MIME type is declared");
    for pattern in ["*.ply", "*.PLY"] {
        assert!(ply.children().any(|node| {
            node.is_element()
                && node.tag_name().name() == "glob"
                && node.attribute("pattern") == Some(pattern)
        }));
    }
}

#[test]
fn package_workflow_parses_and_runs_real_artifact_smokes() {
    let ci = ci_workflow();
    let package = package_workflow();

    assert_ci_artifact_smokes(&ci);
    assert_package_artifact_smokes(&package);
    assert_release_gate(&package);
    assert_windows_package_modes_and_legacy_migration(&package);
}

fn assert_ci_artifact_smokes(ci: &Value) {
    assert_eq!(
        checkout_fetch_depth(ci, "linux-renderer-tests"),
        Some(0),
        "the shell-pin report test needs the pinned revision in the checkout"
    );
    assert!(step_runs(
        ci,
        "macos-arm",
        "Package and verify unsigned Apple Silicon app, DMG, and PKG",
        CI_MACOS_PACKAGE_SMOKE
    ));
    assert!(step_runs(
        ci,
        "linux-package-smoke",
        "Validate the package contents",
        "dpkg-deb --info \"$DEB\"\ninstall/linux/check-deb.sh \"$DEB\""
    ));
    assert!(step_runs(
        ci,
        "third-party-notices",
        "Regenerate THIRD-PARTY-NOTICES.md",
        "./scripts/gen-third-party.sh"
    ));
    assert!(step_runs(
        ci,
        "third-party-notices",
        "Fail on drift",
        "git diff --exit-code -- THIRD-PARTY-NOTICES.md"
    ));
    assert!(any_step_runs(
        ci,
        "third-party-notices",
        "cargo install cargo-about --version 0.8.4 --locked"
    ));
    for (job, duration, limit) in [
        ("fuzz-smoke", "60", "65536"),
        ("fuzz-weekly", "300", "131072"),
    ] {
        for target in ["dispatch", "hps_parser", "stl", "ply", "glb"] {
            assert!(any_step_runs(
                ci,
                job,
                &format!("./scripts/run-fuzz.sh {target} {duration} {limit}")
            ));
        }
    }
}

fn assert_package_artifact_smokes(package: &Value) {
    assert!(step_runs(
        package,
        "windows-package",
        "Smoke install and uninstall",
        WINDOWS_LIFECYCLE_SMOKE
    ));
    assert!(step_runs(
        package,
        "windows-package",
        "Build portable ZIP",
        PORTABLE_ZIP_BUILD
    ));
    assert!(step_runs(
        package,
        "linux-package",
        "Build Debian package",
        "set -o pipefail\npackage=\"$(install/linux/build-deb.sh | tail -n 1)\"\necho \"path=$package\" >> \"$GITHUB_OUTPUT\""
    ));
    assert!(step_runs(
        package,
        "linux-package",
        "Validate Debian package",
        DEBIAN_PACKAGE_VALIDATION
    ));
    assert!(step_runs(
        package,
        "macos-package",
        "Verify the packages",
        MACOS_PACKAGE_VERIFY
    ));
    assert!(step_runs(
        package,
        "macos-package",
        "Build the app with the embedded HPS key",
        "bash install/macos/build-app.sh"
    ));
    assert!(step_runs(
        package,
        "macos-package",
        "Package, and sign and notarize when configured",
        PACKAGE_MACOS_BUILD_SELECTION
    ));
}

fn assert_release_gate(package: &Value) {
    let rehearsal_input = &package["on"]["workflow_dispatch"]["inputs"]["release_dry_run"];
    assert_eq!(rehearsal_input["type"].as_str(), Some("boolean"));
    let publish = &package["jobs"]["publish"];
    assert_eq!(
        publish["if"].as_str(),
        Some("(startsWith(github.ref, 'refs/tags/v') || inputs.release_dry_run) && inputs.windows_configuration != 'diagnostic'")
    );
    assert!(publish["needs"].as_sequence().is_some_and(|needs| {
        [
            "windows-package",
            "linux-package",
            "macos-package",
            "full-ci",
        ]
        .iter()
        .all(|expected| needs.iter().any(|job| job.as_str() == Some(expected)))
    }));
    assert_eq!(
        package["jobs"]["full-ci"]["uses"].as_str(),
        Some("./.github/workflows/ci.yml"),
        "release packaging waits for the reusable CI workflow"
    );
    assert_eq!(
        package["jobs"]["full-ci"]["if"].as_str(),
        Some("startsWith(github.ref, 'refs/tags/') || inputs.release_dry_run")
    );
    let release = workflow_step(package, "publish", "Publish GitHub Release")
        .expect("release publishing has its own guarded step");
    assert_eq!(
        release["if"].as_str(),
        Some("${{ !inputs.release_dry_run }}"),
        "a release rehearsal builds and verifies artifacts without publishing them"
    );
    assert!(step_runs(
        package,
        "publish",
        "Write release notes",
        RELEASE_NOTES_SCRIPT
    ));
    assert!(step_runs(
        package,
        "publish",
        "Bundle verification material",
        RELEASE_VERIFICATION_BUNDLE
    ));
    assert!(step_runs(
        package,
        "publish",
        "Verify the signatures against the key the updater ships",
        SIGNATURE_VERIFICATION_SCRIPT
    ));
    let attest = workflow_step(package, "publish", "Attest build provenance")
        .expect("the release artifacts have build provenance");
    assert_eq!(
        attest["uses"].as_str(),
        Some("actions/attest-build-provenance@4d101475d8b20a2381f78447822ac1eab6504dd8")
    );
    let subjects = attest["with"]["subject-path"]
        .as_str()
        .expect("provenance lists the package subjects");
    assert_eq!(
        subjects.trim(),
        "dist/*.msi\ndist/*.zip\ndist/*.deb\ndist/sbom-*.json\ndist/occluview-shell-revision.json"
    );
    for (job, step, script) in [
        (
            "windows-package",
            "Generate SBOM (Windows)",
            WINDOWS_SBOM_GENERATION,
        ),
        (
            "linux-package",
            "Generate SBOM (Linux)",
            LINUX_SBOM_GENERATION,
        ),
    ] {
        assert!(step_runs(package, job, step, script));
    }
}

#[test]
fn diagnostic_events_keep_a_fixed_privacy_safe_shape() {
    use crate::shell_diagnostics::{
        ShellDiagnosticAdapter, ShellDiagnosticComponent, ShellDiagnosticEvent,
        ShellDiagnosticEventInput, ShellDiagnosticOutcome, ShellDiagnosticProcess,
        ShellDiagnosticStage,
    };

    let event = ShellDiagnosticEvent::normal(
        ShellDiagnosticEventInput {
            component: ShellDiagnosticComponent::Preview,
            stage: ShellDiagnosticStage::BitmapPublish,
            adapter: ShellDiagnosticAdapter::Hardware,
            elapsed_ms: 18,
        },
        ShellDiagnosticOutcome::Completed,
        ShellDiagnosticProcess {
            timestamp_unix_ms: 1_725_000_001,
            process_id: 42,
        },
    )
    .json_line();
    let parsed: serde_json::Value = serde_json::from_str(&event).expect("diagnostic JSON parses");

    for (field, expected) in [
        ("component", "preview"),
        ("stage", "bitmap_publish"),
        ("outcome", "completed"),
        ("adapter", "hardware"),
    ] {
        assert_eq!(parsed[field], expected);
    }
    assert_eq!(parsed["elapsed_ms"], 18);
    for field in ["path", "filename", "driver", "error"] {
        assert!(parsed.get(field).is_none(), "diagnostics omit {field}");
    }
}

#[cfg(windows)]
#[test]
#[allow(clippy::panic)]
fn com_entry_returns_the_fallback_when_the_body_panics() {
    let value = crate::com::com_entry("test::body_returns", || 0_u32, || 7);
    assert_eq!(value, 7, "a successful COM body returns its result");

    let caught = crate::com::com_entry("test::body_panics", || 0_u32, || panic!("boom"));
    assert_eq!(caught, 0, "a COM entry converts a panic into its fallback");
}

#[cfg(unix)]
fn shell_pin_script() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("scripts/report-shell-pin.sh")
}

#[cfg(unix)]
fn shell_pin_commits(report: &serde_json::Value) -> u64 {
    report["commits_behind"]
        .as_u64()
        .expect("the shell-pin report includes its commit count")
}

#[cfg(unix)]
#[test]
fn shell_pin_report_handles_carriage_returns_in_python_fields() {
    use std::process::Command;

    let script = shell_pin_script();
    assert!(script.is_file(), "the package report script is present");
    let root = script
        .parent()
        .and_then(std::path::Path::parent)
        .expect("the report script belongs to the workspace");
    let python = Command::new("sh")
        .arg("-c")
        .arg("command -v python3")
        .output()
        .expect("the packaging runner starts a shell");
    assert!(
        python.status.success(),
        "Python 3 is installed for packaging"
    );
    let python = String::from_utf8(python.stdout)
        .expect("the Python executable path is UTF-8")
        .trim()
        .to_owned();

    let temp = std::env::temp_dir().join(format!(
        "occluview-crlf-shell-pin-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).expect("create the script test directory");
    let shim = temp.join("python3");
    std::fs::write(
        &shim,
        format!(
            "#!/bin/bash\n\"{python}\" \"$@\" | \"{python}\" -c 'import sys; sys.stdout.buffer.write(sys.stdin.buffer.read().replace(b\"\\0\", b\"\\r\\0\"))'\n"
        ),
    )
    .expect("write the Python field shim");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755))
            .expect("make the shim executable");
    }
    let path = std::env::join_paths(std::iter::once(temp.clone()).chain(std::env::split_paths(
        &std::env::var_os("PATH").expect("packaging PATH is set"),
    )))
    .expect("compose a valid executable search path");
    let crlf_path = temp.join("crlf-report.json");
    let output = Command::new("bash")
        .arg(&script)
        .arg(&crlf_path)
        .current_dir(root)
        .env("PATH", path)
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "remote.origin.url")
        .env("GIT_CONFIG_VALUE_0", root)
        .output()
        .expect("run the shell-pin report with CR-tainted Python fields");
    assert!(
        output.status.success(),
        "the report handles carriage returns in field values: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let crlf: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&crlf_path).expect("the script writes its JSON report"),
    )
    .expect("report with CR-tainted input fields is valid JSON");
    assert_eq!(crlf["revision"].as_str().map(str::len), Some(40));

    let clean_path = temp.join("clean-report.json");
    let clean = Command::new("bash")
        .arg(&script)
        .arg(&clean_path)
        .current_dir(root)
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "remote.origin.url")
        .env("GIT_CONFIG_VALUE_0", root)
        .output()
        .expect("run the shell-pin report with normal Python output");
    assert!(clean.status.success(), "the baseline report succeeds");
    let clean: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&clean_path).expect("the baseline JSON report exists"),
    )
    .expect("baseline report is valid JSON");
    assert_eq!(
        shell_pin_commits(&crlf),
        shell_pin_commits(&clean),
        "CRLF output must not truncate the shell crate path list"
    );
    let _ = std::fs::remove_dir_all(&temp);
}
