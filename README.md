# Beside

Beside is a private, local-first floating assistant for your Mac. Press **⌥⇧Space** beside whatever’s open to check a closed note, revise a draft from your own sources, ask where to click on screen, or share scoped preferences with your coding agents.

[Download Beside for Mac](https://github.com/pranavdulepet/beside/releases/latest/download/Beside-macOS.zip) · [Watch Beside on a Mac](https://pranavdulepet.github.io/beside/#demo)

## What Beside does

- **Reads what’s open, recalls what’s closed.** Searches recent screens, Apple Notes, exported Mail, and approved folders on-device with fused full-text and semantic retrieval.
- **Guides you on screen and by voice.** Points to controls in the active window for step-by-step walkthroughs, listens when you hold **⌥⇧Space**, and reads replies aloud with **Speak**.
- **Quietly proactive.** Surfaces a matching note, visible error, reply draft, or upcoming Calendar meeting brief during a pause in typing—and backs off when you dismiss it.
- **Private by default, verifiable in the cloud.** Runs locally on Apple silicon with an encrypted SQLCipher notebook and in-place forgetting. Optional cloud mode supports local alias masking and hardware-attested Tinfoil confidential compute.
- **Learns and travels with you.** Turns corrections into editable Markdown memory, checks inferred updates against past replies before keeping them, and shares scoped context with Claude Code, Codex, Claude Desktop, and Cursor over local MCP.

## Try Beside

1. **Check your Mac.** Open the Apple menu → **About This Mac**. You’ll need an Apple M-series chip and macOS 14 or later. The built-in model downloads need at least 16 GB of memory.
2. **Open Beside.** Double-click the downloaded ZIP, drag **Beside** to **Applications**, then open it. The download is ad-hoc signed. If macOS asks you to review the app, see [Apple’s guide to opening downloaded apps](https://support.apple.com/en-us/102445).
3. **Choose local answers.** The floating sheet offers a model that fits your Mac. It can use an installed copy of Inkling, download one, or open a GGUF file you already have.
4. **Choose what Beside sees.** Select **Turn on screen memory** to remember work between visits, **Only when I ask** for a one-time look, or **Keep it off**. When macOS asks, allow Screen Recording and return to Beside.
5. **Ask beside your work.** Point at the relevant part of a window. Tap **Option–Shift–Space** to type, or hold it to speak. Press **Esc** to go back. The notebook and local model live on your Mac.

## See it in use

[![A sentence selected in the TextEdit essay “Who counted the gates?”](website/media/writing.png)](https://pranavdulepet.github.io/beside/#writing)

- An essay credits archaeologists with finding 64 gates. Ask Beside to check the research notes, inspect the two passages, and revise the sentence in TextEdit. [Writing](https://pranavdulepet.github.io/beside/#writing)
- Add two dinner guests, check what’s in the pantry, and paste the revised Trader Joe’s list into Notes. [Groceries](https://pranavdulepet.github.io/beside/#groceries)
- At an airline’s checked-bag selector, ask “Should I add a bag?” See how Beside finds the packing note for the right trip, checks the arrival forecast and explains whether the extra fee is worth paying. [Flight](https://pranavdulepet.github.io/beside/#flight)

[User guide](docs/guide.md) · [Walkthroughs](docs/walkthroughs.md) · [Architecture](docs/architecture.md) · [Storage and sharing](docs/guide.md#where-your-data-lives) · [Security](SECURITY.md) · [Build from source](docs/development.md) · [Contribute](CONTRIBUTING.md) · [License](LICENSE) · [Third-party credits](licenses/)
