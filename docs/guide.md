# Using Beside on your Mac

Beside lives in two places: a **floating sheet** (**⌥⇧Space**) for quick questions in any app, and a **notebook** for longer work, saved preferences, and settings.

## First launch

Download **Beside-macOS.zip**, unzip it, drag **Beside** into **Applications**, and open it. You’ll need an Apple silicon Mac running macOS 14 or later. Because this build isn’t notarized by Apple yet, macOS may ask you to confirm it under **System Settings → Privacy & Security → Open Anyway**. [Download Beside](https://github.com/pranavdulepet/beside/releases/latest/download/Beside-macOS.zip) · [Apple’s guide](https://support.apple.com/en-us/102445)

1. **Choose a model.** Beside suggests a local model that fits your Mac (built-in downloads start at 16 GB of memory), or lets you connect Ollama, LM Studio, or your own GGUF file.
2. **Choose screen access.** Pick **Turn on screen memory** to remember work across windows, **Only when I ask** for a one-time look when you press the shortcut, or **Keep it off**. You can change or pause this anytime in **Sources** or the menu bar.

## Asking from any app

- **Type or speak:** Point at the line or window you’re asking about. Tap **⌥⇧Space** to type, or hold it to speak. Press **Esc** to dismiss.
- **Check selected text:** In apps that support macOS Services, select text and choose **Services → Ask Beside about selection**.
- **On-screen pointers:** Ask “Where do I click?” in an allowed window and Beside highlights the control on screen. Click **Next step →** for multi-step guidance.
- **Read aloud:** Click **Speak** under a reply to hear it with a Mac voice.
- **Quiet suggestions:** When screen memory and suggestions are on, a small dot appears in the menu bar during a pause in typing if Beside spots a matching note, error, reply draft, or upcoming meeting. Dismissing a suggestion hides repeats for seven days.

## Choosing what Beside remembers

Open **What Beside knows → Sources** to turn each source on or off:

- **Screen memory:** Pick specific apps, specific websites, or **All displays** (optional toggle to include other open windows in local answers). Screen history expires after 30 days, and optional previews expire after 24 hours.
- **Notes and Mail:** Import Apple Notes or exported Apple Mail folders so Beside can search them even when those apps are closed. Removing an import also clears its quotes from saved chats.
- **Folders:** Pick folders on your Mac to search local text, Markdown, PDF, Word, and spreadsheet files.
- **Calendar and Meetings:** Turn on upcoming meeting briefs, or use **Record a meeting** (macOS 15+) for local transcripts and action items.

## Where your data lives

Your notebook lives in `~/Beside`. You can open it in Finder from Settings, export a copy, or move it to the Trash anytime.

- **Encrypted database (`.beside/moments.db`):** Stores floating chats, saved screen passages, imports, and search indexes. Its key lives in your macOS Keychain.
- **Plain Markdown (`desk/` and `scroll/`):** Stores notebook chats, preferences, relationships, dated memories, and reusable skills so you can read or edit them in any text editor.
- **Optional files:** Daily screen copies (off by default) and meeting recordings live inside `.beside/` and can be managed or cleared from Settings.

Forgetting a passage, import, or memory also removes what Beside derived from it.

## Models, web search, and cloud sharing

Beside runs on your Mac by default.

- **Web search:** Current public questions look up public pages automatically (using Exa, Brave if configured, or DuckDuckGo) without attaching your private notes.
- **Optional cloud models:** You can connect a cloud account or API key in Settings. Nothing is sent until you explicitly enable cloud sharing, and private memories stay on your Mac.
  - **Mask identifying text** swaps names, emails, phone numbers, and home paths for temporary aliases before sending and restores them locally, while keeping images on your Mac.
  - **Tinfoil** runs through a verified confidential compute router.

## Teaching preferences and sharing with other tools

Click **Correct this reply** under any answer to teach Beside how you prefer things done. You can review, edit, or retire any preference in **Memory**.

In **Connections**, you can connect tools like **ChatGPT Work mode (Codex)**, **Claude Code**, **Claude Desktop**, and **Cursor** over a local connection on your Mac:

- Choose exactly which sections each tool can read (for example, sharing **How you decide** while leaving **People** unchecked).
- Every tool gets its own revocable token and shows up in **Morning digest** so you can inspect what it read.
- Automatic replies stay off unless you allow them, and even then only activate after at least 20 reviewed practice trials pass your confidence threshold.

## Troubleshooting

- **Shortcut does nothing:** Open Beside from the menu bar; **Sources** will flag if another app already uses **⌥⇧Space**.
- **Beside can’t see a window:** Check **Sources** and verify Screen Recording / Accessibility are enabled in **System Settings → Privacy & Security**, then reopen Beside.
- **A note isn’t showing up:** Refresh Apple Notes in **Sources**, or add the folder containing your file.
