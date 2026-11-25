# Contributing to Beside

Open an issue with what you were trying to do in Beside, your macOS and Beside versions, what you expected, and what happened. Use a made-up example for transcripts, screenshots and notebooks.

For a code change, build on an Apple Silicon Mac using the setup in the [development guide](docs/development.md). Keep each pull request focused on one change. Describe how you verified it and include a short recording for interaction changes. Exercise both the installed-model and model-free paths if your change affects setup or answering.

Run the Rust core tests and the frontend build before submitting. The UI and model integration suites have additional local requirements listed in the development guide. Report any checks you could not run.

Keep model weights, notebook data, API credentials, generated binaries and videos outside Git. Release downloads belong on GitHub Releases. The project is MIT licensed.
