# Using Beside on your Mac

Once screen memory is on, Beside can use a passage you saw earlier even after you close its window. Choose **All displays** to cover visible work across apps. An import is useful for older material you haven’t opened while Beside was observing.

The floating sheet is for a question that comes up in another app. Use Chats to reopen a floating conversation, or open the notebook for longer writing and saved preferences. [Install Beside and try the shortcut](../README.md#try-beside)

## First launch

Download **Beside-macOS.zip**, double-click it in Finder, drag **Beside** into **Applications** and open it there. The app requires an Apple silicon Mac running macOS 14 or later. The community build has an ad-hoc signature and Apple has not notarized it. If macOS says it cannot verify the developer and you trust the copy you downloaded, follow Apple’s instructions for **Open Anyway** in **System Settings → Privacy & Security**. [Download Beside](https://github.com/pranavdulepet/beside/releases/latest/download/Beside-macOS.zip) · [Apple’s opening instructions](https://support.apple.com/en-us/102445)

Choose the suggested model in the floating sheet. Beside checks for installed weights and tests the model with a local request. If it offers to download a model, it shows the size before you begin. Managed model downloads start at 16 GB of Mac memory. You can also choose a compatible GGUF file you already have.

**Turn on screen memory** lets Beside save passages as you work. **Only when I ask** reads the pointed window for a local answer when you summon it. That conversation can retain your question, the answer and a supporting line. **Keep it off** leaves screen access disabled. You can change this choice in **Sources**.

For ongoing screen memory, macOS may also ask to let Beside “bypass the system private window picker.” Choose **Allow** if you want Beside to read the screen in the scope you selected. **Last seen** in Sources shows when it last succeeded.

## Ask from another app

Hold **Option–Shift–Space** with your pointer on the relevant line. Speak when the ink mark appears. Beside shows the words it heard as you speak. Release to see the answer, use the follow-up field to keep going, or press Escape to return to your work. Tap the shortcut to type. In apps that support macOS text services, select text and use **Services → Ask Beside about selection**.

Choose **Notebook** to open the full app. Choose **Open companion** in the notebook to return to the floating sheet. Click the Beside mark to open **What Beside knows**. Inspect **Last question** for the pointer context and passages used. A source beneath a connected answer opens the supporting line. Choose **Use this passage** on an earlier observation to ask about it again. You can refer to an item by its contents, such as “the printer’s instructions.”

When observation and suggestions are on, a dot in the menu bar can point to a useful line in Notes or Mail. Choose **View suggestion**, then **Show connection** to see the line and ask Inkling how it relates. If the conversation changes subject, Beside checks the match again when it next reads the screen. You can dismiss a suggestion or mute an app in notebook Settings.

Web lookup is available automatically. A standalone public question about current information can go straight to search. Screen-identification questions use local evidence. If Beside needs a public topic or location, it asks for that detail.

### At an airline’s bag selector

Point at the checked bag you’re considering, press **⌥⇧Space**, and ask “Should I add a bag?” Beside uses the fare on screen and looks for a saved packing note for the same destination and date. The answer names the note, labels the weight estimate and links the forecast for the arrival airport. You can open either source in the sheet. If the checkout leaves out a detail that matters, Beside asks you for it. You choose the bag on the airline’s page.

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
| Calendar | Turn on **Upcoming meeting briefs** in Sources to see an upcoming meeting and related passages from your imported notes or mail. |
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

### From a meal plan to a shopping note

With an itemized meal plan open in Apple Notes, tap **⌥⇧Space** and ask what to buy. Beside compares the ingredients with your pantry. Tell it “Actually, I already have limes” to cross an item off the current list. The other quantities carry over. Choose **Copy**, then press **⌘V** in your shopping note.

When dinners have quantities and a head count, Beside can handle changes in the same conversation. For example, “Make Thursday dinner for four” rescales that meal. “Actually, I have three cans of black beans” replaces the earlier pantry count. Beside combines the meals, subtracts what you own and gives you a list to copy.

You can also ask “Make a shopping list from my meal plan” in another app. Beside looks for a saved, itemized plan. If two fit, it asks which one you mean.

### Check a claim while you write

In TextEdit, select a sentence and choose **TextEdit → Services → Ask Beside about selection**. Ask “Do my notes actually support this?” When a reply draws on more than one note, open **What each source supports** to check the passages. Use **Copy** to paste the revision into your draft. [Try it with the Chandragupta Maurya essay](walkthroughs.md)

### Carry a correction into your next draft

Tell Beside “For project updates, I prefer a paragraph of background first.” If you later change that to “Actually, I want the risk first, then the decision,” Beside saves the new preference. From your next project note, select the brief and ask “Draft my project update. Two sentences, please.” You can check or retire the preference in **What Beside knows → Memory**.

Ask “Where do I click?” in an allowed window to point to a visible control. For a walkthrough, clicking a highlighted control advances to the next visible step. Choose **Speak** beneath a reply to hear it read aloud. **Stop** or the shortcut ends playback.

## Models, web search and sharing

The floating assistant defaults to a local model. If you explicitly enable cloud context sharing, the selected provider can receive permitted screen text, images, file excerpts and saved passages. Private memories and local-only corrections are excluded. Model downloads are separate from the app. The model picker checks available memory and disk space, and verifies a bundled download’s digest. If you already run Ollama or LM Studio, open local model setup and select an available model.

In the notebook, an eligible ChatGPT plan or an API provider is optional. A cloud notebook model can receive conversation passages needed for the answer. Private sheets stay local, and attached excerpts have their own sharing control. In the floating sheet, choosing a provider for one typed question sends that text and returns the next question to local thinking. Anthropic uses an API key. Provider usage follows the account you connect.

Ask a current public question and Beside looks it up automatically. To request a lookup yourself, type a topic and click **Search public web**. Beside sends those terms to Exa, Brave if configured, or DuckDuckGo. Click a source beneath a floating answer to read the passage Beside used, then choose **Open source** to visit the page. A question about what’s on your screen uses local context. When a lookup needs a city or a clearer public topic, Beside asks. Notebook Settings also lets you manage weekend listing searches and access for connected agents.

## Floating conversations and corrections

To sign the floating sheet, open **What Beside knows → Memory → Your ink mark**, draw one stroke, and choose **Keep my mark**. The header and the small pen stroke beside a working status use that drawing. With macOS Reduce Motion enabled, the stroke stays still.

Open **Chats** to return to a conversation. Beside stores the turns and their supporting excerpts in your notebook’s encrypted database. A portable export includes them in `records/floatingChat.json`, which is readable by anyone with the exported archive. If Chats offers **Import earlier chats**, you can move the previous local browser history into the current notebook. The earlier copy is removed after a successful import.

Click the source name beneath an answer to check its evidence. When an answer used several sources, you can open each one. Changing a pantry quantity updates the current shopping list and preserves the original meal plan.

Choose **Correct this reply** beneath an answer and write the preference you want Beside to use. The saved sentence is a private notebook memory. Deleting a chat removes the conversation. A correction you chose to keep can be retired separately in Memory. Forgetting an observation removes chats that cite it. Forgetting all screen history also clears the floating conversations.

When your Mac is idle, Beside can test a proposed memory against what you actually said later. It compares predictions with and without the change before adopting it. Dated follow-ups in floating chats take part in on-device tests only.

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

In cloud model settings, **Mask identifying text** hides recognized identifiers in companion prompts and keeps images on your Mac. Choosing **Tinfoil** verifies its confidential router before sending any text. Its **Verify confidential connection** button checks the connection without submitting a question.
