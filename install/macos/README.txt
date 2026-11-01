OccluView for Apple Silicon
===========================

This local package targets arm64 Macs running macOS 14 or later. Drag
OccluView.app to Applications, then launch it from Finder. Finder document-open
integration covers STL, PLY, OBJ, GLB, and OFF files. Files can also be opened explicitly or
dragged onto the viewer.

The command-line companion is bundled at:
  OccluView.app/Contents/Helpers/occluview-cli

The local app, DMG, and package are unsigned and not notarized. They are not
release artifacts, and macOS may block them when downloaded with quarantine
metadata. A maintainer with Apple Developer credentials must sign the app with
Developer ID Application, sign the installer with Developer ID Installer,
notarize and staple the distributed artifacts, then publish them through the
release workflow. Do not remove quarantine as a distribution substitute.

Build from a native Apple Silicon Mac with Rust 1.98.0, CMake, and Xcode command-line tools:
  bash install/macos/build-app.sh
  bash install/macos/build-dmg.sh --no-build
  bash install/macos/build-pkg.sh --no-build
