# Security

Report a suspected vulnerability privately through [GitHub Security Advisories](https://github.com/pranavdulepet/beside/security/advisories/new). Include the Beside and macOS versions, the feature involved, and the effect you observed. Use sample data and keep notebook files, screenshots of private conversations, API keys and Keychain entries out of reports.

## What Beside protects

The working database uses SQLCipher with its key held in macOS Keychain. Screen observation starts with your permission and can be paused from the menu bar. The local agent server uses a loopback address, a bearer token and request limits. Model downloads are checked against a pinned digest.

A user or process with access to your unlocked account can read the notebook’s ordinary Markdown files, portable exports and meeting recordings. Public search terms go to Exa. A cloud provider can receive the material you submit or permit Beside to share. See the [storage and permissions guide](docs/guide.md).

## Release review

The November 25, 2026 maintainer review covered the locked npm and Cargo dependencies, public source tree, notebook and export permissions, model downloads, local agent endpoint, webview policy and Mac packaging. The updated lockfiles returned zero known vulnerabilities from npm audit and RustSec. RustSec flagged the maintenance status of Chrono, which ships on Mac. Five further RustSec warnings concern packages outside the Apple Silicon dependency tree. One also appears in GitHub’s advisory database as a moderate GLib finding. Tauri uses that GTK dependency on Linux and BSD. Both the resolved Mac dependency tree and the linked libraries of the distributed app and runtime exclude it. Reassess the finding before adding another platform. [GLib advisory](https://github.com/advisories/GHSA-wrw7-89jp-8q8g)

To repeat the dependency checks, run `npm audit` and `cargo audit --file src-tauri/Cargo.lock` with a current advisory database. Run `gitleaks dir . --redact` on a clean source checkout to scan for recognizable credentials. The [development guide](docs/development.md) covers the app tests and package verification.

This was an internal review. Its scope excludes independent penetration testing, model weights and an upstream source audit of the bundled llama.cpp runtime. The community download is ad-hoc signed and lacks Apple notarization.
