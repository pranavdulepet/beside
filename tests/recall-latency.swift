// A native, user-visible test. Run with Beside tucked or hidden and another app
// active. Supply a saved-line query and its expected evidence as JSON on stdin:
// {"question":"…","answer":"…","quote":"…"}
import AppKit
import ApplicationServices
import ScreenCaptureKit
import Vision

struct Example: Decodable {
    let question: String
    let answer: String
    let quote: String
}

func stop(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}

func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
    var value: CFTypeRef?
    return AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success ? value : nil
}

func string(_ element: AXUIElement, _ name: String) -> String {
    attribute(element, name) as? String ?? ""
}

func descendants(_ element: AXUIElement) -> [AXUIElement] {
    let children = attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? []
    return [element] + children.flatMap(descendants)
}

func now() -> Double { ProcessInfo.processInfo.systemUptime }
func milliseconds(_ duration: Double) -> Int { Int(ceil(duration * 1000)) }

func wait(_ seconds: Double, _ check: () -> Bool) -> Bool {
    let until = now() + seconds
    repeat {
        if check() { return true }
        RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.008))
    } while now() < until
    return false
}

func postKey(_ code: CGKeyCode, _ down: Bool, _ flags: CGEventFlags = []) {
    guard let event = CGEvent(keyboardEventSource: CGEventSource(stateID: .combinedSessionState),
                              virtualKey: code, keyDown: down) else { stop("Cannot post a key.") }
    event.flags = flags
    event.post(tap: .cghidEventTap)
}

func shortcut() {
    postKey(58, true, .maskAlternate)
    postKey(56, true, [.maskAlternate, .maskShift])
    postKey(49, true, [.maskAlternate, .maskShift])
    postKey(49, false, [.maskAlternate, .maskShift])
    postKey(56, false, .maskAlternate)
    postKey(58, false)
}

func words(_ text: String) -> String {
    text.lowercased().split { !$0.isLetter && !$0.isNumber }.joined(separator: " ")
}

@main struct RecallLatency {
    static func main() async throws {
        let sample = try JSONDecoder().decode(Example.self, from: FileHandle.standardInput.readDataToEndOfFile())
        let workspace = NSWorkspace.shared
        let apps = workspace.runningApplications.filter { $0.bundleIdentifier == "studio.sumi.notebook" }
        guard apps.count == 1, let app = apps.first,
              workspace.frontmostApplication?.processIdentifier != app.processIdentifier,
              CGPreflightPostEventAccess(), AXIsProcessTrusted() else {
            stop("Open one Beside, switch to another app, and allow keyboard automation.")
        }
        let ui = AXUIElementCreateApplication(app.processIdentifier)
        func panel() -> AXUIElement? {
            (attribute(ui, kAXWindowsAttribute) as? [AXUIElement])?.first { string($0, kAXTitleAttribute) == "Ask Beside" }
        }
        func focusedComposer() -> AXUIElement? {
            guard workspace.frontmostApplication?.processIdentifier == app.processIdentifier,
                  let win = panel(),
                  let focused = attribute(ui, kAXFocusedUIElementAttribute) else { return nil }
            let field = focused as! AXUIElement
            guard string(field, kAXRoleAttribute) == "AXTextArea",
                  descendants(win).contains(where: { CFEqual($0, field) }) else { return nil }
            return field
        }
        let opened = now()
        shortcut()
        guard wait(5, { focusedComposer() != nil }), let field = focusedComposer(), let win = panel() else {
            stop("The shortcut did not focus the floating composer.")
        }
        let ready = now()
        guard string(field, kAXValueAttribute).isEmpty else { stop("The composer contains a draft.") }
        func staticText() -> [String] {
            descendants(win).filter { string($0, kAXRoleAttribute) == "AXStaticText" }.map { string($0, kAXValueAttribute) }
        }
        let before = staticText()
        func newText(_ value: String, in after: [String]) -> Bool {
            after.filter { $0 == value }.count > before.filter { $0 == value }.count
        }
        let windows = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: true)
        guard let window = windows.windows.first(where: {
            $0.owningApplication?.processID == app.processIdentifier && $0.title == "Ask Beside"
        }) else { stop("The floating window is off screen.") }
        let filter = SCContentFilter(desktopIndependentWindow: window)
        let config = SCStreamConfiguration()
        config.width = 1024
        config.height = 1096
        config.preservesAspectRatio = true
        config.ignoreShadowsSingleWindow = true
        config.showsCursor = false
        guard AXUIElementSetAttributeValue(field, kAXValueAttribute as CFString, sample.question as CFString) == .success,
              wait(2, { string(field, kAXValueAttribute) == sample.question &&
                  descendants(win).contains { string($0, kAXTitleAttribute) == "Ask on this Mac" &&
                      attribute($0, kAXEnabledAttribute) as? Bool == true }
              }) else { stop("The question is not ready to send.") }
        try await Task.sleep(nanoseconds: 700_000_000)
        guard let focused = focusedComposer(), CFEqual(field, focused) else { stop("The composer lost focus before Return.") }
        let submitted = now()
        postKey(36, true)
        postKey(36, false)
        guard wait(2, { string(field, kAXValueAttribute).isEmpty }) else {
            stop("Return was not accepted within two seconds. No lookup timing recorded.")
        }
        let accepted = now()
        guard wait(10, {
            let after = staticText()
            return [sample.question, sample.answer, sample.quote].allSatisfy { newText($0, in: after) }
        }) else { stop("A new reply with the expected dishes and quote did not arrive.") }
        let rendered = now()
        // Clock the captured pixels before running OCR on them. A previous
        // off-screen answer, AX update, or stale frame cannot pass this check.
        for _ in 0..<5 {
            let frame = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: config)
            let captured = now()
            let request = VNRecognizeTextRequest()
            request.recognitionLevel = .accurate
            try VNImageRequestHandler(cgImage: frame).perform([request])
            let visible = words((request.results ?? []).compactMap { $0.topCandidates(1).first?.string }.joined(separator: " "))
            if [sample.question, sample.answer, "Exact line", sample.quote].allSatisfy({ visible.contains(words($0)) }) {
                print("Shortcut to focused composer: \(milliseconds(ready - opened)) ms")
                print("Return to acknowledged submission: \(milliseconds(accepted - submitted)) ms")
                print("Return to verified new reply in accessibility tree: \(milliseconds(rendered - submitted)) ms")
                print("Return to verified window pixels (upper bound): \(milliseconds(captured - submitted)) ms")
                return
            }
        }
        stop("The new reply was returned, but its question, answer and quote could not all be verified on screen.")
    }
}
