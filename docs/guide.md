# Using Beside on your Mac

Once screen memory is on, Beside can use a passage you saw earlier even after you close its window. Choose **All displays** to cover visible work across apps. An import is useful for older material you haven’t opened while Beside was observing.

The floating sheet is for a question that comes up in another app. Use Chats to reopen a floating conversation, or open the notebook for longer writing and saved preferences. [Install Beside and try the shortcut](../README.md#try-beside)

## Ask from another app

Hold **Option–Shift–Space** with your pointer on the relevant line. Speak when the ink mark appears. Beside shows the words it heard as you speak. Release to see the answer, use the follow-up field to keep going, or press Escape to return to your work. Tap the shortcut to type. In apps that support macOS text services, select text and use **Services → Ask Beside about selection**.

Choose **Notebook** to open the full app. Choose **Open companion** in the notebook to return to the floating sheet. Click the Beside mark to open **What Beside knows**. Inspect **Last question** for the pointer context and passages used. A source beneath a connected answer opens the supporting line. Choose **Use this passage** on an earlier observation to ask about it again. Descriptions such as “my packing list” can match a saved note by subject and contents. If several lists fit, a destination or another detail can help distinguish them.

When suggestions are enabled and observation is running, Beside can match a visible passage to a saved note. A dot in the menu bar and **View suggestion** let you inspect the match. Choose **Show connection** to ask about the screen that produced it, or dismiss it. Manage suggestions in notebook Settings. Beside can surface a matching line from Mail as well as Notes. Opening the suggestion keeps the screen and the source line paired.

Web lookup is available automatically. A standalone public question about current information can go straight to search. Screen-identification questions use local evidence. If Beside needs a public topic or location, it asks for that detail.

## Choose what Beside remembers

Open **What Beside knows → Sources**. Observation starts when you select **Start** and can be paused from the sheet or the menu bar.

| Source or feature | Access and behavior |
| --- | --- |
| Selected native apps | Screen Recording lets Beside read visible windows in the apps you enable. Messages can use Accessibility for sender labels. |
| Selected websites | Accessibility provides visible page text on the exact HTTPS hostnames you add for Safari or Chrome. Approved hosts can also appear in private windows. |
| All displays | Screen Recording covers visible content across connected displays, including visible private browsing windows. Only use this scope if that fits what you want to remember. |
| Other open windows | Under **All displays**, enable **Include other open windows in local answers**. Beside lists eligible macOS windows, including covered windows. It rotates through them for on-device text recognition. The live list and excerpts stay in memory, and the model can use them for local answers. Open Memory to see **In front** and **Also open**. Closing a window removes it on the next refresh. Minimized windows can lack readable content, and inactive browser tabs have no independent window to capture. |
| Folders and local files | Choose folders explicitly. Beside searches them with Spotlight and a bounded fallback scan and can read text, Markdown, PDF, RTF, Word and spreadsheet files. Remove a folder to revoke future searches. |
| Voice | Microphone and Speech Recognition support hold-to-talk. On-device recognition needs a supported language. Typed questions work independently of the microphone. |
| Meetings | On macOS 15 or later, select **Record a meeting**, choose whether to include your microphone, and pick the window to share. Stop using the sheet or menu bar. **Get action items from last meeting** uses local transcription and a local model. |

The activity index retains passages for up to 30 days. Optional visual recall retains up to 150 encrypted previews for 24 hours. Apple Notes capture waits for the visible title and nearby text to settle. Use **Last seen** to check observation, even when the screen hasn’t changed. Forgetting a source also removes observations and corrections derived from it.

## Where your data lives

New notebooks use `~/Beside`. Upgrades keep using the existing notebook folder and Keychain key. Its working database uses SQLCipher. macOS Keychain holds its key and optional provider credentials. Keep your login Keychain with a full notebook backup. Use Settings for a portable export, or to move the notebook to the Trash.

| Data | What to expect on disk |
| --- | --- |
| Floating conversations | Saved in the encrypted database. |
| Workspace conversations, dated memories and skills | Readable Markdown copies in the notebook. Anyone with access to those files can read them. |
| Saved screen passages and optional previews | Stored in the encrypted database. **Keep daily Markdown copies** also writes readable copies in the notebook’s hidden activity-journal folder. Turning this off removes the managed daily files. They are excluded from notebook exports and the agent interface. |
| Meeting recordings | Standard MP4 files in the notebook’s hidden meetings folder, outside the encrypted database. Use **Show recording in Finder** to manage them. |

### Look inside your notebook

In notebook Settings, choose **Open notebook folder**. In Finder, `desk/conversations/` holds workspace chats, `desk/core.md` lists core memories and `desk/pages/` groups dated memories by subject. The `scroll/` folder holds notebook entries. Saved methods live under `scroll/skills/`.

You can open the Markdown in a text editor. If you prefer a shell, run these from the notebook folder once you have saved a conversation or memory.

```sh
ls desk
cat desk/core.md
find scroll -name '*.md'
grep -rin 'ferry' desk scroll
```

These commands search the readable files. Beside also searches its encrypted screen history and imported passages, which a Markdown search cannot see. Use Memory to correct or retire a dated fact. Beside regenerates its Markdown summary from those records.


### While booking a flight

Beside prepares pointers to the observed booking and a packing list whose title matches the trip. These pointers live in the encrypted notebook. When you ask about an extra bag, Beside checks the original sources and compares kilograms locally. If the packed weight is missing, it asks you to weigh the bag. A changed fare can require a new allowance. The originals stay where you put them.

## Models, web search and sharing

The floating assistant defaults to a local model. If you explicitly enable cloud context sharing, the selected provider can receive permitted screen text, images, file excerpts and saved passages. Private memories and local-only corrections are excluded. Model downloads are separate from the app. The model picker checks available memory and disk space, and verifies a bundled download’s digest. If you already run Ollama or LM Studio, open local model setup and select an available model.

In the notebook, an eligible ChatGPT plan or an API provider is optional. A cloud notebook model can receive conversation passages needed for the answer. Private sheets stay local, and attached excerpts have their own sharing control. In the floating sheet, choosing a provider for one typed question sends that text and returns the next question to local thinking. Anthropic uses an API key. Provider usage follows the account you connect.

Ask a current public question and Beside looks it up automatically. Questions about what you’re looking at use local screen memory. Exa receives public search terms. When a lookup needs a location or a private reference cannot be resolved to public terms, Beside asks for the missing detail. The selected model reads the results for a floating answer. In notebook settings you can also manage search approval for weekend listings. Agent access is separate: review the notebook entries each connected tool can read, and revoke access in Settings.

## Floating conversations and corrections

To sign the floating sheet, open **What Beside knows → Memory → Your ink mark**, draw one stroke, and choose **Keep my mark**. The header and the small pen stroke beside a working status use that drawing. With macOS Reduce Motion enabled, the stroke stays still.


Open **Chats** to return to a conversation. Beside stores the turns and their supporting excerpts in your notebook’s encrypted database. A portable export includes them in `records/floatingChat.json`, which is readable by anyone with the exported archive. If Chats offers **Import earlier chats**, you can move the previous local browser history into the current notebook. The earlier copy is removed after a successful import.

Choose **Correct this reply** beneath an answer and write the preference you want Beside to use. The saved sentence is a private notebook memory. Deleting a chat removes the conversation. A correction you chose to keep can be retired separately in Memory. Forgetting an observation removes chats that cite it. Forgetting all screen history also clears the floating conversations.

## Work in the notebook

Use **Sheets** (⌘O) to reopen a conversation. Return sends a message and Shift–Return inserts a line. Attach a document and ask a follow-up on the same sheet. Tables, charts and checklists appear when the answer calls for them. **Behind this reply** shows the model and tools used.

Use **Correct this reply** to teach Beside a preference. A red memory mark opens the supporting memory and its date. **Not true anymore** retires it. **What Beside knows** (⌘K) includes current and earlier memories, people, projects and saved methods. A standing instruction or repeated task can become a named Markdown skill. Idle processing can review proposed memories against later replies. Direct corrections take effect immediately.

## If something doesn’t work

- **The shortcut does nothing.** Open **Ask about this Mac** from the menu bar. Sources reports a shortcut conflict if another app has registered the same combination.
- **Beside can’t see the page.** Check **Sources**, the selected scope, and **Last seen**. In macOS **System Settings → Privacy & Security**, grant Screen Recording or Accessibility to the copy of Beside you actually launch. Quit and reopen if macOS requests it.
- **A note can’t be found.** Check the source scope, retention window and whether Beside ever observed the note. For a document outside the activity history, add its containing folder or attach the file in the notebook.
- **Voice comes through incorrectly.** Check the live transcript, macOS input device and Microphone and Speech Recognition permissions. Tap the shortcut to enter the exact wording.
- **There’s no model that fits.** The managed downloads require at least 16 GB of memory. Connect an existing compatible local model for floating questions. A connected cloud provider can handle an explicitly submitted typed question.


### Use Notes and Mail in proactive suggestions

Under **What Beside knows → Sources → Earlier notes and mail**, choose **Import Apple Notes**. macOS may ask for Automation access to Notes. Beside copies unlocked note titles and text into the encrypted notebook. It can then find a passage under a matching heading while Notes is closed. Choose **Refresh Apple Notes** after editing or locking notes to replace the snapshot. **Remove imported notes** clears the index. A passage you already accepted can also appear in your saved conversation. Inkling explains these suggestions locally.

In Apple Mail choose **Mailbox → Export Mailbox**, then open **Earlier notes and mail**, choose **Import Mail export** in Beside and select the export folder. Beside streams the mbox files locally and replaces its previous Mail snapshot. It keeps up to 12,000 characters of message text, preferring plain text and extracting readable text from HTML locally. It skips attachments and raw messages over 24 MiB. The import reports how many messages it kept and skipped. Apple Notes and Mail each have a removal control.
