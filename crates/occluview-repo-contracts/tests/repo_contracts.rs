//! Repository-level contract tests.
//!
//! These assert what the repository says about itself rather than what a crate
//! does: that the CI workflows still run the artifact smokes they promise, that
//! the packaging report survives carriage returns, that the workspace, the
//! lockfile and the installer agree on one version, and that the MSRV the
//! toolchain pins is the one the manifest and clippy declare. They live outside
//! the shell crate because the COM shell is an opt-in member; the Windows-only
//! modules it gates are not what these read, and every platform's CI should run
//! them.

#![allow(clippy::expect_used, clippy::panic)] // a missing repository file is a test failure

use occluview_repo_contracts::{
    cargo_lock_package_version, clippy_msrv, toolchain_channel, wix_product_version,
    workspace_package_version, workspace_rust_version,
};
use serde_yaml_ng::Value;
use std::path::PathBuf;

fn repo_file(path: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::read_to_string(root.join(path)).expect("repository contract input is readable")
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

const CI_MACOS_PACKAGE_SMOKE: &str = r#"bash install/macos/build-app.sh --no-build
app="target/macos/OccluView.app"
log="$RUNNER_TEMP/occluview-scene-load.log"
plist="$app/Contents/Info.plist"
/usr/libexec/PlistBuddy -c 'Add :LSEnvironment dict' "$plist"
/usr/libexec/PlistBuddy -c "Add :LSEnvironment:OCCLUVIEW_SCENE_LOAD_LOG string $log" "$plist"
plutil -lint "$plist"
codesign --force --sign - "$app"
codesign --verify --deep --strict "$app"
bash install/macos/build-dmg.sh --no-build
bash install/macos/build-pkg.sh --no-build
dmg="$(find target/macos -maxdepth 1 -type f -name 'OccluView-*-aarch64.dmg' -print -quit)"
pkg="$(find target/macos -maxdepth 1 -type f -name 'OccluView-*-aarch64.pkg' -print -quit)"
test -n "$dmg" && test -n "$pkg"
hdiutil verify "$dmg"
pkgutil --payload-files "$pkg" | grep -F 'OccluView.app/Contents/MacOS/occluview'
lipo -archs target/macos/OccluView.app/Contents/MacOS/occluview | grep -Fx arm64
for notice in LICENSE NOTICE THIRD-PARTY-NOTICES.md THIRD-PARTY-NOTICES-NATIVE.md; do
  test -s "target/macos/OccluView.app/Contents/Resources/Legal/$notice"
done
version="$(sed -n '/^\[workspace\.package\]/,/^\[/p' Cargo.toml \
  | sed -n 's/^version *= *"\(.*\)"/\1/p' | head -n1)"
test -n "$version"
target/macos/OccluView.app/Contents/MacOS/occluview --version | grep -F "$version"
target/macos/OccluView.app/Contents/Helpers/occluview-cli --version | grep -F "$version"
"#;

const PORTABLE_ZIP_BUILD: &str = r#"$cargoText = Get-Content ./Cargo.toml -Raw
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
"#;

const WINDOWS_LIFECYCLE_SMOKE: &str = r#"$releaseMsi = Get-ChildItem ./dist -Filter *.msi | Sort-Object LastWriteTime -Descending | Select-Object -First 1
$upgradeMsi = Get-ChildItem (Join-Path $env:RUNNER_TEMP "occluview-msi-upgrade") -Filter *.msi | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not [string]::IsNullOrWhiteSpace($env:OCCLUVIEW_LEGACY_MSI_PATH)) {
  ./install/test-msi-lifecycle.ps1 -MsiPath $releaseMsi.FullName -LegacyUpgradeMsiPath $env:OCCLUVIEW_LEGACY_MSI_PATH -UpgradeMsiPath $upgradeMsi.FullName -DowngradeMsiPath $releaseMsi.FullName
} else {
  ./install/test-msi-lifecycle.ps1 -MsiPath $releaseMsi.FullName -UpgradeMsiPath $upgradeMsi.FullName -DowngradeMsiPath $releaseMsi.FullName
}
"#;

const DEBIAN_PACKAGE_VALIDATION: &str = r#"desktop-file-validate install/linux/ai.occlutrace.OccluView.desktop
appstreamcli validate --no-net install/linux/ai.occlutrace.OccluView.metainfo.xml
xmllint --noout install/linux/occluview-mime.xml install/linux/ai.occlutrace.OccluView.metainfo.xml
dpkg-deb --info "$DEB"
dpkg-deb --contents "$DEB"
install/linux/check-deb.sh "$DEB"
"#;

const MACOS_PACKAGE_VERIFY: &str = r#"version="$(sed -n '/^\[workspace\.package\]/,/^\[/p' Cargo.toml \
  | sed -n 's/^version *= *"\(.*\)"/\1/p' | head -n1)"
dmg="target/macos/OccluView-$version-aarch64.dmg"
pkg="target/macos/OccluView-$version-aarch64.pkg"
hdiutil verify "$dmg"
pkgutil --payload-files "$pkg" | grep -F 'OccluView.app/Contents/MacOS/occluview'
lipo -archs target/macos/OccluView.app/Contents/MacOS/occluview | grep -Fx arm64
for notice in LICENSE NOTICE THIRD-PARTY-NOTICES.md THIRD-PARTY-NOTICES-NATIVE.md; do
  test -s "target/macos/OccluView.app/Contents/Resources/Legal/$notice"
done
target/macos/OccluView.app/Contents/MacOS/occluview --version | grep -F "$version"
target/macos/OccluView.app/Contents/Helpers/occluview-cli --version | grep -F "$version"
(cd target/macos && for file in *.dmg *.pkg; do shasum -a 256 "$file" > "$file.sha256"; done)
"#;

const PACKAGE_MACOS_BUILD_SELECTION: &str = r#"if [[ "${{ steps.signing.outputs.configured }}" == true ]]; then
  OCCLUVIEW_NOTARY_KEY_PATH="$RUNNER_TEMP/notary.p8" bash install/macos/sign-and-notarize.sh
  echo "notarized=true" >> "$GITHUB_OUTPUT"
else
  bash install/macos/build-dmg.sh --no-build
  bash install/macos/build-pkg.sh --no-build
  echo "notarized=false" >> "$GITHUB_OUTPUT"
fi
"#;

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

const RELEASE_VERIFICATION_BUNDLE: &str = r#"version="${RELEASE_TAG#v}"
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
"#;

const SIGNATURE_VERIFICATION_SCRIPT: &str = r#"# Verify with the public key compiled into installed copies.
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
"#;

const WINDOWS_SBOM_GENERATION: &str = r#"cargo install cargo-cyclonedx --version 0.5.8 --locked
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
"#;

const LINUX_SBOM_GENERATION: &str = r#"cargo install cargo-cyclonedx --version 0.5.8 --locked
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
"#;

fn step_runs(workflow: &Value, job: &str, name: &str, command: &str) -> bool {
    workflow_step(workflow, job, name)
        .and_then(|step| step["run"].as_str())
        .is_some_and(|run| run.trim() == command.trim())
}

fn step_contains(workflow: &Value, job: &str, name: &str, command: &str) -> bool {
    workflow_step(workflow, job, name)
        .and_then(|step| step["run"].as_str())
        .is_some_and(|run| run.contains(command))
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
    let legacy_artifact_condition =
        "inputs.windows_configuration != 'diagnostic' && inputs.legacy_msi_run_id != '' && inputs.legacy_msi_sha256 != ''";
    assert_eq!(download["if"].as_str(), Some(legacy_artifact_condition));
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
    assert_eq!(verify["if"].as_str(), Some(legacy_artifact_condition));
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
    for command in [
        "lsregister",
        "swift install/macos/check-launch-services.swift /Applications/OccluView.app",
        "open \"$sample\"",
        "open \"$warm_sample\"",
        "pgrep -x occluview",
    ] {
        assert!(
            step_contains(
                ci,
                "macos-arm",
                "Install package and open a mesh through LaunchServices",
                command
            ),
            "macOS LaunchServices smoke must include {command:?}"
        );
    }
    let macos_open = workflow_step(
        ci,
        "macos-arm",
        "Install package and open a mesh through LaunchServices",
    )
    .and_then(|step| step["run"].as_str())
    .expect("macOS package smoke opens the registered file handler");
    assert!(
        !macos_open.contains("open -a"),
        "LaunchServices must select the registered handler"
    );
    let windows_test = workflow_step(ci, "test", "cargo test --workspace (WARP required)")
        .expect("Windows CI runs the required WARP renderer suites");
    assert_eq!(
        windows_test["env"]["OCCLUVIEW_REQUIRE_GPU_TESTS"].as_str(),
        Some("1")
    );
    assert_eq!(
        windows_test["run"].as_str(),
        Some("cargo test --workspace --all-targets --locked --no-fail-fast")
    );
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

#[test]
fn release_version_is_kept_in_sync_across_workspace_lockfile_and_installer() {
    let cargo_toml = repo_file("Cargo.toml");
    let cargo_lock = repo_file("Cargo.lock");
    let wxs = repo_file("install/occluview.wxs");

    let version = workspace_package_version(&cargo_toml);
    assert!(version.is_some(), "workspace package version is present");
    let Some(version) = version else {
        panic!("required test setup or expected result was missing");
    };
    let wix_version = wix_product_version(&wxs);
    assert!(
        wix_version.is_some(),
        "WiX fallback product version is present"
    );
    let Some(wix_version) = wix_version else {
        panic!("required test setup or expected result was missing");
    };
    assert_eq!(
        wix_version, version,
        "WiX ProductVersion fallback must match Cargo workspace version"
    );

    for package in [
        "occluview-geometry-math",
        "occluview-mesh-edit",
        "occlu-sculpt",
        "occluview-align",
        "occluview-contact",
        "occluview-core",
        "occluview-edit",
        "occluview-formats",
        "occluview-hps",
        "occluview-i18n",
        "occluview-render",
        "occluview-repo-contracts",
        "occluview-robust-csg",
        "occluview-shell",
        "occluview-surface-query",
        "occluview-thumbnail",
        "occluview-update",
        "occluview-app",
        "occluview-cli",
    ] {
        assert_eq!(
            cargo_lock_package_version(&cargo_lock, package),
            Some(version),
            "{package} version in Cargo.lock must match Cargo workspace version"
        );
    }
}

/// The major and minor parts of a version, so `1.98` and `1.98.0` compare
/// equal while a floating channel such as `stable` does not.
fn major_minor(version: &str) -> (&str, &str) {
    let mut parts = version.split('.');
    (parts.next().unwrap_or_default(), parts.next().unwrap_or("0"))
}

#[test]
fn msrv_agrees_across_toolchain_manifest_and_clippy() {
    let cargo_toml = repo_file("Cargo.toml");
    let toolchain = repo_file("rust-toolchain.toml");
    let clippy = repo_file("clippy.toml");

    let Some(rust_version) = workspace_rust_version(&cargo_toml) else {
        panic!("Cargo.toml declares the workspace MSRV");
    };
    let Some(channel) = toolchain_channel(&toolchain) else {
        panic!("rust-toolchain.toml pins a channel");
    };
    let Some(msrv) = clippy_msrv(&clippy) else {
        panic!("clippy.toml declares the MSRV clippy lints against");
    };

    assert_eq!(
        major_minor(channel),
        major_minor(rust_version),
        "rust-toolchain.toml channel and Cargo.toml rust-version disagree on the MSRV"
    );
    assert_eq!(
        major_minor(rust_version),
        major_minor(msrv),
        "Cargo.toml rust-version and clippy.toml msrv disagree on the MSRV"
    );
}

fn workspace_crates_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("crates")
}

fn collect_rust_sources(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_sources(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
}

fn workspace_crate_sources() -> Vec<PathBuf> {
    let mut sources = Vec::new();
    for entry in std::fs::read_dir(workspace_crates_dir())
        .expect("the workspace has a crates directory")
        .flatten()
    {
        let src = entry.path().join("src");
        if src.is_dir() {
            collect_rust_sources(&src, &mut sources);
        }
    }
    sources.sort();
    sources
}

fn opens_unsafe_block(code: &str) -> bool {
    let mut rest = code;
    while let Some(position) = rest.find("unsafe") {
        let after = &rest[position + "unsafe".len()..];
        if after.trim_start().starts_with('{') {
            return true;
        }
        rest = after;
    }
    false
}

fn has_safety_comment(lines: &[&str], index: usize) -> bool {
    if lines[index].contains("SAFETY") {
        return true;
    }
    let mut cursor = index;
    while cursor > 0 {
        cursor -= 1;
        let trimmed = lines[cursor].trim();
        if !trimmed.starts_with("//") {
            return false;
        }
        if trimmed.contains("SAFETY") {
            return true;
        }
    }
    false
}

fn unsafe_blocks_without_a_safety_comment(source: &std::path::Path) -> Vec<usize> {
    let text = std::fs::read_to_string(source).expect("a workspace source file is readable");
    let lines: Vec<&str> = text.lines().collect();
    let mut missing = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        if opens_unsafe_block(code) && !has_safety_comment(&lines, index) {
            missing.push(index + 1);
        }
    }
    missing
}

#[test]
fn unsafe_code_is_gated_and_every_unsafe_block_is_justified() {
    let mut unexplained = Vec::new();
    for source in workspace_crate_sources() {
        for line in unsafe_blocks_without_a_safety_comment(&source) {
            unexplained.push(format!("{}:{line}", source.display()));
        }
    }
    assert!(
        unexplained.is_empty(),
        "unsafe blocks without a SAFETY: comment:\n{}",
        unexplained.join("\n")
    );

    for entry in std::fs::read_dir(workspace_crates_dir())
        .expect("the workspace has a crates directory")
        .flatten()
    {
        let library = entry.path().join("src/lib.rs");
        let binary = entry.path().join("src/main.rs");
        let root = if library.is_file() {
            library
        } else if binary.is_file() {
            binary
        } else {
            continue;
        };
        let text = std::fs::read_to_string(&root).expect("a crate root is readable");
        assert!(
            text.contains("unsafe_code"),
            "{} must carry an unsafe_code gate",
            root.display()
        );
    }
}
