# Build Beside from source

## Run the desktop app

Start on an Apple Silicon Mac running macOS 14 or later. Install rustup for the pinned Rust toolchain, Node.js 22.12 or later, npm, Go with automatic toolchain selection (the confidential helper pins Go 1.26.11), and the Xcode command line tools. The runtime setup verifies the pinned upstream llama.cpp archive. You can pass a previously downloaded archive as its first argument to restore llama.cpp offline. Building the confidential helper offline also requires its Go modules and toolchain to have been cached.

```sh
git clone https://github.com/pranavdulepet/beside.git
cd beside
scripts/prepare-runtime.sh
npm ci
npm run desktop
```

The desktop shell is Tauri. Vite serves the React interface during development. The native app provides screen access, storage and inference.

Follow a shortcut, a background observation or a memory correction through the running app in the [architecture guide](architecture.md).

## Where to look

| Area | Entry points |
| --- | --- |
| Floating sheet and voice gesture | [React sheet](../src/Activity.tsx), [native capture](../src-tauri/activity.m), [speech input](../src-tauri/voice.m) |
| Evidence and retrieval | [Activity index](../src-tauri/src/activity.rs), [related passages and saved lists](../src-tauri/src/activity/), [file search](../src-tauri/src/activity_files.rs) |
| Model and tool selection | [Conversation](../src-tauri/src/conversation.rs), [notebook tools](../src-tauri/src/desk_tools.rs), [local runtime](../src-tauri/src/bundled.rs) |
| Memory and ownership | [Store](../src-tauri/src/store.rs), [memory](../src-tauri/src/memory.rs), [agent access](../src-tauri/src/agent_link.rs) |
| Tests | [Browser workflows](../tests/notebook.spec.ts) and Rust tests alongside their modules |

Beside uses the native on-device embedding index by default. To use an embedding model you already have in Ollama, start Beside with `BESIDE_OLLAMA_EMBED_MODEL=name:tag` in its environment. The idle worker reads the local manifest, verifies the GGUF blob and builds the new index. Inference runs inside the bundled, offline llama.cpp runtime on loopback. Model tags from remote registries are unsupported. Set `BRAVE_API_KEY` in the app’s environment to enable Brave as a web search fallback.

## Run the checks

```sh
npm run build
(cd src-tauri/confidential && go test -mod=readonly ./...)
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo build --locked --manifest-path src-tauri/Cargo.toml
# The browser suite uses the packaged archive and the model described below.
scripts/release.sh
npm run test:ui
```

The UI test uses Google Chrome, an installed Ollama llama3.2:3b model, a temporary notebook, and the debug build of the Rust core. Additional integration checks exercise local inference, the packaged command-line agent server and loopback HTTP. Build the release app before running them.

```sh
cargo test --manifest-path src-tauri/Cargo.toml -- --ignored --nocapture --test-threads=1
```

To exercise the publisher download in the browser suite, set `BESIDE_TEST_MODEL_DOWNLOAD=1`. This optional test downloads the strongest supported tier that fits the available RAM and storage from Hugging Face and removes its temporary notebook and weights afterward. On repeat runs, set `BESIDE_TEST_GGUF_PREFIX` to an existing GGUF with the same filename to reuse its leading bytes. The test still interrupts a real transfer, resumes it from the publisher, verifies the full digest and runs an inference.

For the local activity walkthrough, place the pinned Inkling model and optional vision projector in the ignored `release/` directory as `Inkling-2-Mini-UD-IQ3_XXS.gguf` and `mmproj-F16.gguf`. Run `BESIDE_TEST_INKLING=1 npm run test:ui -- -g 'ask anywhere remembers'` with both files to exercise OCR, visual inference, corrections, and forgetting against the Rust backend. The test uses a synthetic window and never needs permission to inspect your screen.

The weekend walkthrough is opt-in. With the local model and browser prerequisites above, run `BESIDE_LIVE_SEARCH=1 npm run test:ui -- -g 'a weekend plan'`. It creates a synthetic notebook, searches public listings through public web search, checks the source and memory mark, retires the preference, and reopens the notebook. Results vary with the date and available listings.

To measure a saved-line lookup on a real Mac, compile the native check with `swiftc -parse-as-library tests/recall-latency.swift -o /tmp/beside-recall-latency`. Tuck the assistant, activate another app, and run the binary with a local JSON example on standard input containing `question`, `answer`, and `quote`. The check submits one question to the open notebook. It times shortcut focus, Return acknowledgement, the new reply, and a window capture in which local OCR verifies the question, answer, and exact line. The pixel time is an upper bound. OCR processing time is excluded.

## Package and distribute

For repeated installs on your own Mac, build the app and run `scripts/install-local.sh`. It signs the bundle with your Apple Development identity and keeps the app at `~/Applications/Beside.app`. When upgrading an older installation, the installer keeps its former executable path available for connected agents. It verifies each update against the installed signature’s designated requirement before replacing the bundle. The first switch from an ad-hoc build can require a fresh macOS grant. Use this same install path and identity for subsequent updates. For distribution to other people, use the release script with a Developer ID Application identity and notarization.

To make and verify the distributable bundle and the website download, run `scripts/release.sh`. The standalone landing page is `website/index.html`. It loads its fonts and paper locally. On disk and on localhost the download uses the adjacent Mac archive. The public page links to the latest release. GitHub Pages deploys the website from `main` through the Pages workflow.

To distribute with Apple notarization, set `BESIDE_SIGN_IDENTITY` to the Developer ID Application identity and `BESIDE_NOTARY_PROFILE` to its Keychain notarytool profile before running `scripts/release.sh`. The script signs the embedded runtime, submits the app, staples the ticket, and verifies the extracted archive. With neither variable set, it builds an ad-hoc signed copy. A successful `codesign --verify --deep --strict` validates that bundle’s signature and sealed content. A Developer ID signature and Apple notarization are separate distribution checks. The first-launch guide explains what to expect from the community download. [First launch](guide.md#first-launch)

The redistributable source archive is built by `python3 scripts/source-archive.py` and written to `release/Beside-source.zip`. The release script runs this step automatically. Run it from a clean checkout at the release commit. The packager uses the tracked file list and reads the files from the working tree. It adds a file inventory for offline repackaging.

## Publishing a signed model list

Official builds include the model list and its verification key under `catalog/`. They check the signed manifest on `main` weekly while idle. To update it, increment the revision in `catalog/payload.json`, run `swift scripts/sign-catalog.swift catalog/payload.json catalog/models.json` on the release Mac, review the changes and publish the signed manifest to `main`. The signing key lives in macOS Keychain. A publisher may set `BESIDE_CATALOG_URL` to its HTTPS manifest URL and `BESIDE_CATALOG_PUBLIC_KEY` to its base64-encoded 32-byte Ed25519 public key at compile time. The envelope contains `payload` and `signature`, both standard base64. Sign the exact UTF-8 JSON payload bytes. The payload contains an integer `revision` above 1 and an ordered `models` array of Ollama `name` and byte-size `bytes` records. The app rejects invalid signatures and older revisions. Users can disable weekly checks. Models download only on request.
