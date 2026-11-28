#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <AVFoundation/AVFoundation.h>
#import <Speech/Speech.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import <QuartzCore/QuartzCore.h>
#import <WebKit/WebKit.h>

extern NSDictionary *beside_point_window(CGPoint p);
extern NSDictionary *beside_read_point(NSDictionary *window, CGPoint p, NSDictionary *settings);
extern int beside_workspace_available(void);

int beside_summon_access(void) {
    if (!beside_workspace_available()) return 0;
    if (CGPreflightScreenCaptureAccess()) return 1;
    // Some Macs grant ScreenCaptureKit while the older Core Graphics check
    // reports false. Verify that grant afresh; an earlier success can be revoked.
    dispatch_semaphore_t ready=dispatch_semaphore_create(0);
    __block BOOL allowed=NO;
    [SCShareableContent getShareableContentExcludingDesktopWindows:YES onScreenWindowsOnly:YES
        completionHandler:^(SCShareableContent *content,NSError *error) {
            allowed=!error && content.displays.count>0;
            dispatch_semaphore_signal(ready);
        }];
    return !dispatch_semaphore_wait(ready,dispatch_time(DISPATCH_TIME_NOW,NSEC_PER_SEC)) && allowed;
}
typedef void (*BesideVoiceCallback)(const char *);
@interface BesideVoice : NSObject
@property AVAudioEngine *engine;
@property SFSpeechAudioBufferRecognitionRequest *request;
@property SFSpeechRecognitionTask *task;
@property SFSpeechRecognizer *recognizer;
@property NSPanel *badge;
@property NSTextField *label;
@property NSDictionary *point, *window, *settings;
@property CGPoint position;
@property NSPoint mouse;
@property NSString *text, *issue;
@property BOOL held, released, finished, tapped, typed, pointReady, speechDone;
@property BesideVoiceCallback callback;
@end
static BesideVoice *current;
static NSValue *replyPosition;
static NSRect sourceWindow;
// A receipt for the frontmost app and its focused window. No text or image is read.
static NSRunningApplication *returnApp;
static id returnWindow;
static NSRunningApplication *serviceApp;
static id serviceWindow;
static NSRunningApplication *recentServiceSender;
static NSTimeInterval senderLostFocus;

void beside_watch_service_origin(void) {
    NSNotificationCenter *center=NSWorkspace.sharedWorkspace.notificationCenter;
    [center addObserverForName:NSWorkspaceDidDeactivateApplicationNotification object:nil
                         queue:NSOperationQueue.mainQueue usingBlock:^(NSNotification *event) {
        NSRunningApplication *app=event.userInfo[NSWorkspaceApplicationKey];
        if (!app || app.processIdentifier==NSRunningApplication.currentApplication.processIdentifier) return;
        recentServiceSender=app.activationPolicy==NSApplicationActivationPolicyRegular ? app : nil;
        senderLostFocus=NSProcessInfo.processInfo.systemUptime;
    }];
}

static id focusedWindowOf(NSRunningApplication *app) {
    if (!app || !AXIsProcessTrusted()) return nil;
    id owner=CFBridgingRelease(AXUIElementCreateApplication(app.processIdentifier));
    AXUIElementSetMessagingTimeout((__bridge AXUIElementRef)owner,0.1);
    CFTypeRef window=NULL;
    if (AXUIElementCopyAttributeValue((__bridge AXUIElementRef)owner,kAXFocusedWindowAttribute,&window)==kAXErrorSuccess)
        return CFBridgingRelease(window);
    return nil;
}

static void rememberFrontApp(void) {
    NSRunningApplication *app=NSWorkspace.sharedWorkspace.frontmostApplication;
    if (!app || app.processIdentifier==NSRunningApplication.currentApplication.processIdentifier) return;
    returnApp=app;
    returnWindow=focusedWindowOf(app);
}

// Capture the sender while its Services menu still owns the selection. The
// asynchronous WebView presentation can happen after AppKit changes focus.
void beside_capture_service_origin(void) {
    if (!NSThread.isMainThread) {
        dispatch_sync(dispatch_get_main_queue(),^{ beside_capture_service_origin(); });
        return;
    }
    NSWorkspace *workspace=NSWorkspace.sharedWorkspace;
    NSRunningApplication *app=workspace.frontmostApplication;
    pid_t ourPID=NSRunningApplication.currentApplication.processIdentifier;
    if (!app || app.processIdentifier==ourPID) app=workspace.menuBarOwningApplication;
    // Services can activate Beside before delivering the pasteboard. In that
    // transition AppKit already reports Beside as both front and menu owner.
    if ((!app || app.processIdentifier==ourPID) &&
        NSProcessInfo.processInfo.systemUptime-senderLostFocus<2 && !NSApp.keyWindow.visible)
        app=recentServiceSender;
    serviceApp=app.processIdentifier==ourPID ? nil : app;
    serviceWindow=focusedWindowOf(serviceApp);
}

void beside_prepare_panel(int fromService) {
    void (^prepare)(void)=^{
        if (fromService) {
            returnApp=serviceApp;
            returnWindow=serviceWindow;
            serviceApp=nil;
            serviceWindow=nil;
        } else rememberFrontApp();
    };
    if (NSThread.isMainThread) prepare();
    else dispatch_sync(dispatch_get_main_queue(),prepare);
}

@interface BesidePanelCover : NSView
@end
@implementation BesidePanelCover
- (NSView *)hitTest:(NSPoint)point { return nil; }
@end

static BesidePanelCover *panelCover;
static NSUInteger panelPaintGeneration;
static WKWebView *panelWebView(NSView *view) {
    if ([view isKindOfClass:WKWebView.class]) return (WKWebView *)view;
    for (NSView *child in view.subviews) {
        WKWebView *web = panelWebView(child);
        if (web) return web;
    }
    return nil;
}

// The WindowServer can briefly reuse the last surface of an ordered-out
// WKWebView, even after its hidden DOM was replaced. Keep an opaque native
// view over that surface across orderOut/orderFront. Keyboard input goes
// straight to the fresh composer underneath.
void beside_cover_panel(void *pointer) {
    NSWindow *panel = (__bridge NSWindow *)pointer;
    panelPaintGeneration++;
    panel.alphaValue = 0;
    NSView *root = panel.contentView;
    if (!panelCover) {
        panelCover = [BesidePanelCover new];
        panelCover.wantsLayer = YES;
        panelCover.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
        [panelCover setAccessibilityElement:NO];
    }
    panelCover.frame = root.bounds;
    BOOL dark = [[panel.effectiveAppearance bestMatchFromAppearancesWithNames:
        @[NSAppearanceNameAqua, NSAppearanceNameDarkAqua]] isEqual:NSAppearanceNameDarkAqua];
    NSColor *paper = dark ? [NSColor colorWithSRGBRed:28./255 green:41./255 blue:41./255 alpha:1]
                          : [NSColor colorWithSRGBRed:250./255 green:248./255 blue:241./255 alpha:1];
    panelCover.layer.backgroundColor = paper.CGColor;
    if (panelCover.superview != root)
        [root addSubview:panelCover positioned:NSWindowAbove relativeTo:nil];
    [CATransaction flush];
}

void beside_reveal_after_paint(void *pointer) {
    NSWindow *panel = (__bridge NSWindow *)pointer;
    NSUInteger generation = panelPaintGeneration;
    WKWebView *web = panelWebView(panel.contentView);
    [panel.contentView layoutSubtreeIfNeeded];
    [panel displayIfNeeded];
    [CATransaction flush];
    panel.alphaValue = 1;
    if (!web) return;

    // Order the covered window in first. requestAnimationFrame in a hidden
    // WKWebView can be suspended indefinitely. Two frames leave a complete
    // paint interval between the DOM commit and the final screen update.
    [web callAsyncJavaScript:@"await new Promise(done => requestAnimationFrame(() => requestAnimationFrame(done))); return true;"
                   arguments:@{} inFrame:nil inContentWorld:WKContentWorld.pageWorld
           completionHandler:^(id painted, NSError *scriptError) {
            (void)scriptError;
        if (generation != panelPaintGeneration || !panel.visible) return;
        WKSnapshotConfiguration *config = [WKSnapshotConfiguration new];
        config.rect = CGRectMake(0, 0, 1, 1);
        config.afterScreenUpdates = YES;
        [web takeSnapshotWithConfiguration:config completionHandler:^(NSImage *pixel, NSError *error) {
            (void)error;
            if (generation == panelPaintGeneration && panel.visible && (pixel || [painted boolValue]))
                [panelCover removeFromSuperview];
        }];
    }];
}

void beside_dismiss_panel(void *pointer) {
    NSWindow *panel=(__bridge NSWindow *)pointer;
    NSRunningApplication *origin=returnApp;
    id window=returnWindow;
    returnApp=nil;
    returnWindow=nil;
    NSRunningApplication *beside=NSRunningApplication.currentApplication;
    BOOL shouldReturn=origin && !origin.terminated && !origin.hidden && panel.visible &&
        NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier==beside.processIdentifier &&
        (panel.isKeyWindow || !NSApp.keyWindow) && !NSApp.modalWindow && !panel.attachedSheet;
    BOOL allowed=NO;
    if (shouldReturn) {
        // Transfer while we still own activation. Ordering out the key
        // window first lets the window server choose another application.
        if (@available(macOS 14.0,*)) {
            [NSApp yieldActivationToApplication:origin];
            allowed=[origin activateFromApplication:beside options:0];
        } else allowed=[origin activateWithOptions:0];
    }
    beside_cover_panel(pointer);
    [panel orderOut:nil];
    if (allowed && window && AXIsProcessTrusted()) {
        AXUIElementSetMessagingTimeout((__bridge AXUIElementRef)window,0.1);
        AXUIElementPerformAction((__bridge AXUIElementRef)window,kAXRaiseAction);
    }
}

void beside_position_reply(void *pointer) {
    if (!replyPosition) return;
    NSPoint p=replyPosition.pointValue; replyPosition=nil;
    NSWindow *w=(__bridge NSWindow *)pointer;
    NSScreen *screen=NSScreen.mainScreen;
    for(NSScreen *s in NSScreen.screens) if(NSPointInRect(p,s.frame)) screen=s;
    NSRect area=NSInsetRect(screen.visibleFrame,10,10);
    NSRect frame=w.frame;
    CGFloat width=MIN(420,NSWidth(area)), gap=14;
    CGFloat x=p.x+gap;
    CGFloat y=MAX(NSMinY(area),MIN(p.y-42,NSMaxY(area)-MIN(512,NSHeight(area))));
    // Keep compact source windows completely visible when a side margin is free.
    if (!NSIsEmptyRect(sourceWindow) && NSMaxX(sourceWindow)+gap+width<=NSMaxX(area))
        x=NSMaxX(sourceWindow)+gap;
    else if (!NSIsEmptyRect(sourceWindow) && NSMinX(sourceWindow)-gap-width>=NSMinX(area))
        x=NSMinX(sourceWindow)-gap-width;
    else {
        if (x+width>NSMaxX(area)) x=p.x-width-gap;
        // The sheet keeps its bottom edge when it grows. Reserve space above or
        // below the pointed line so streaming text doesn't obscure the source.
        CGFloat reserve=MIN(512,NSHeight(area));
        if (p.y+gap+reserve<=NSMaxY(area)) y=p.y+gap;
        else if (p.y-gap-reserve>=NSMinY(area)) y=p.y-gap-reserve;
    }
    [w setFrameOrigin:NSMakePoint(MAX(NSMinX(area),MIN(x,NSMaxX(area)-frame.size.width)),y)];
}
@implementation BesideVoice
- (void)badgeText:(NSString *)value {
    if (!self.badge) {
        NSPanel *panel=[[NSPanel alloc] initWithContentRect:NSMakeRect(0,0,108,31) styleMask:NSWindowStyleMaskBorderless|NSWindowStyleMaskNonactivatingPanel backing:NSBackingStoreBuffered defer:NO];
        panel.opaque=NO; panel.backgroundColor=NSColor.clearColor; panel.hasShadow=YES; panel.ignoresMouseEvents=YES;
        panel.level=NSStatusWindowLevel; panel.collectionBehavior=NSWindowCollectionBehaviorCanJoinAllSpaces|NSWindowCollectionBehaviorFullScreenAuxiliary;
        NSView *card=panel.contentView; card.wantsLayer=YES;
        card.layer.backgroundColor=[NSColor colorWithRed:.11 green:.16 blue:.16 alpha:.97].CGColor; card.layer.cornerRadius=15;
        NSView *dot=[[NSView alloc] initWithFrame:NSMakeRect(12,11,8,8)]; dot.wantsLayer=YES; dot.layer.cornerRadius=4; dot.layer.backgroundColor=[NSColor colorWithRed:.98 green:.92 blue:.69 alpha:1].CGColor; [card addSubview:dot];
        self.label=[NSTextField labelWithString:@""]; self.label.font=[NSFont fontWithName:@"Geist-Regular" size:13] ?: [NSFont systemFontOfSize:13]; self.label.textColor=NSColor.whiteColor; self.label.lineBreakMode=NSLineBreakByTruncatingHead;
        self.label.frame=NSMakeRect(27,6,79,19); [card addSubview:self.label];
        NSScreen *screen=NSScreen.mainScreen;
        for (NSScreen *s in NSScreen.screens) if (NSPointInRect(self.mouse,s.frame)) screen=s;
        NSRect edge=screen.visibleFrame;
        [panel setFrameOrigin:NSMakePoint(MIN(MAX(NSMinX(edge),self.mouse.x+15),NSMaxX(edge)-108),MAX(NSMinY(edge),MIN(self.mouse.y-38,NSMaxY(edge)-31)))];
        self.badge=panel;
    }
    // A single, local transcript lets the speaker catch a misheard reference.
    BOOL hasTranscript=self.held && !self.released && self.text.length>0;
    CGFloat width=hasTranscript ? 280 : 108;
    NSScreen *screen=self.badge.screen ?: NSScreen.mainScreen;
    NSRect area=screen.visibleFrame;
    width=MIN(width,NSWidth(area));
    NSRect frame=self.badge.frame;
    frame.size.width=width;
    frame.origin.x=MAX(NSMinX(area),MIN(self.mouse.x+15,NSMaxX(area)-width));
    if (!NSEqualRects(frame,self.badge.frame)) [self.badge setFrame:frame display:YES];
    self.label.frame=NSMakeRect(27,6,width-31,19);
    self.label.stringValue=value; [self.badge orderFrontRegardless];
}
- (void)stopMic {
    [self.engine stop];
    if (self.tapped) { [self.engine.inputNode removeTapOnBus:0]; self.tapped=NO; }
    [self.request endAudio]; self.engine=nil;
}
- (void)deliver {
    if (self.finished || !self.released || !self.speechDone || !self.pointReady) return;
    self.finished=YES; [self stopMic]; [self.task cancel]; self.task=nil; [self.badge orderOut:nil];
    NSDictionary *payload=@{@"typed":@(self.typed), @"text":self.text ?: @"", @"issue":self.issue ?: @"", @"point":self.point ?: @{},@"generation":self.settings[@"generation"] ?: NSNull.null,@"epoch":self.settings[@"epoch"] ?: NSNull.null,@"onDemand":@([self.settings[@"onDemand"] boolValue])};
    NSData *data=[NSJSONSerialization dataWithJSONObject:payload options:0 error:nil];
    if (current==self) {
        replyPosition=[NSValue valueWithPoint:self.mouse];
        sourceWindow=[self.window[@"frame"] rectValue];
    }
    if (current==self && data) self.callback([[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding].UTF8String);
    if (current==self) current=nil;
}
- (void)fail:(NSString *)reason {
    self.issue=reason; self.speechDone=YES; [self stopMic];
    if (!self.released) [self badgeText:@"Type on release"];
    [self deliver];
}
- (void)listen {
    if (current!=self || self.released) return;
    if ([AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio]!=AVAuthorizationStatusAuthorized) {
        [self fail:@"Allow Beside to use the microphone in System Settings, then hold the shortcut again. You can type here now."];
        if ([AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio]==AVAuthorizationStatusNotDetermined)
            [AVCaptureDevice requestAccessForMediaType:AVMediaTypeAudio completionHandler:^(BOOL granted) {
                if (granted && SFSpeechRecognizer.authorizationStatus==SFSpeechRecognizerAuthorizationStatusNotDetermined)
                    [SFSpeechRecognizer requestAuthorization:^(SFSpeechRecognizerAuthorizationStatus status){ (void)status; }];
            }];
        return;
    }
    if (SFSpeechRecognizer.authorizationStatus!=SFSpeechRecognizerAuthorizationStatusAuthorized) {
        [self fail:@"Allow Speech Recognition for Beside in System Settings, then hold the shortcut again. You can type here now."];
        if (SFSpeechRecognizer.authorizationStatus==SFSpeechRecognizerAuthorizationStatusNotDetermined)
            [SFSpeechRecognizer requestAuthorization:^(SFSpeechRecognizerAuthorizationStatus status){ (void)status; }];
        return;
    }
    self.recognizer=[[SFSpeechRecognizer alloc] initWithLocale:NSLocale.currentLocale];
    if (!self.recognizer.supportsOnDeviceRecognition) { [self fail:@"On-device dictation is unavailable for this language. Type your question here."]; return; }
    self.engine=[AVAudioEngine new];
    self.request=[SFSpeechAudioBufferRecognitionRequest new]; self.request.taskHint=SFSpeechRecognitionTaskHintDictation; self.request.requiresOnDeviceRecognition=YES; self.request.shouldReportPartialResults=YES;
    // Common references to the user's own material are often dropped from
    // short utterances. These are vocabulary hints for on-device recognition.
    self.request.contextualStrings=@[@"Beside", @"Inkling 2 Mini", @"Inkling",
        @"packing list", @"shopping list", @"reading list", @"grocery list",
        @"checklist", @"saved note", self.window[@"app"] ?: @"Mac"];
    AVAudioInputNode *input=self.engine.inputNode;
    AVAudioFormat *format=[input outputFormatForBus:0];
    if (format.channelCount==0 || format.sampleRate==0) { [self fail:@"No microphone is available. Type your question here."]; return; }
    SFSpeechAudioBufferRecognitionRequest *request=self.request;
    [input installTapOnBus:0 bufferSize:1024 format:format block:^(AVAudioPCMBuffer *buffer, AVAudioTime *when){ (void)when; [request appendAudioPCMBuffer:buffer]; }];
    self.tapped=YES;
    __weak BesideVoice *weak=self;
    self.task=[self.recognizer recognitionTaskWithRequest:self.request resultHandler:^(SFSpeechRecognitionResult *r,NSError *error){
        dispatch_async(dispatch_get_main_queue(),^{
            BesideVoice *s=weak; if (!s || s.finished) return;
            if (r.bestTranscription.formattedString.length) {
                s.text=r.bestTranscription.formattedString;
                if (!s.released) [s badgeText:s.text];
            }
            if (r.isFinal || error) {s.speechDone=YES; [s stopMic]; if (!s.text.length) s.issue=@"I didn’t catch that. Hold again or type your question."; [s deliver];}
        });
    }];
    NSError *err=nil; if (![self.engine startAndReturnError:&err]) { [self fail:@"The microphone couldn’t start. Type here or try again."]; return; }
    [self badgeText:@"Listening…"];
}
- (void)releaseKey {
    if (self.released) return;
    self.released=YES; [self stopMic]; [self badgeText:@"Reading…"]; [self deliver];
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW,1600*NSEC_PER_MSEC),dispatch_get_main_queue(),^{
        if (self.finished) return;
        self.speechDone=YES; if (!self.text.length && !self.issue.length) self.issue=@"I didn’t catch that. Hold again or type your question.";
        [self deliver];
    });
}
@end

void beside_voice_key(int down,const char *raw,BesideVoiceCallback callback) {
    NSData *data=raw ? [NSData dataWithBytes:raw length:strlen(raw)] : nil;
    dispatch_async(dispatch_get_main_queue(),^{
        if (!down) {
            BesideVoice *s=current; if (!s || s.released) return;
            if (!s.held) {
                // Quick tap = Point & Type: keep the live screen & cursor capture, skip mic.
                if (![s.settings[@"onDemand"] boolValue] && [s.window[@"bundle"] isEqual:@"com.apple.MobileSMS"]) {
                    current=nil;
                    callback("{\"tap\":true}");
                    return;
                }
                s.typed=YES;
                s.released=YES;
                s.speechDone=YES;
                [s deliver];
            } else [s releaseKey];
            return;
        }
        if (current && !current.released) return;
        if (current) { current.finished=YES; [current stopMic]; [current.task cancel]; [current.badge orderOut:nil]; }
        rememberFrontApp();
        BesideVoice *s=[BesideVoice new]; current=s; s.callback=callback;
        s.settings=data ? [NSJSONSerialization JSONObjectWithData:data options:0 error:nil] : @{};
        s.mouse=NSEvent.mouseLocation;
        s.position=CGPointMake(s.mouse.x,NSMaxY(NSScreen.screens.firstObject.frame)-s.mouse.y);
        if ([s.settings[@"enabled"] boolValue]) s.window=beside_point_window(s.position);
        // Capture pointer + full-window screenshot immediately on key-down so both
        // Tap (type) and Hold (speak) have instant visual + pointer context.
        dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED,0),^{
            NSDictionary *point=beside_read_point(s.window,s.position,s.settings);
            dispatch_async(dispatch_get_main_queue(),^{ s.point=point; s.pointReady=YES; [s deliver]; });
        });
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW,240*NSEC_PER_MSEC),dispatch_get_main_queue(),^{
            if (current!=s || s.released) return;
            s.held=YES; callback("{\"warm\":true}"); [s badgeText:@"One moment"];
            [s listen];
            dispatch_after(dispatch_time(DISPATCH_TIME_NOW,60*NSEC_PER_SEC),dispatch_get_main_queue(),^{ if (current==s && !s.released) [s releaseKey]; });
        });
    });
}

// Read the displayed answer with a system voice.
static AVSpeechSynthesizer *speechSynth;

void beside_stop_speaking(void) {
    dispatch_async(dispatch_get_main_queue(), ^{
        if (speechSynth && speechSynth.isSpeaking) {
            [speechSynth stopSpeakingAtBoundary:AVSpeechBoundaryImmediate];
        }
    });
}

void beside_speak(const char *rawText) {
    if (!rawText || !rawText[0]) return;
    NSString *text = [NSString stringWithUTF8String:rawText];
    if (!text.length) return;
    dispatch_async(dispatch_get_main_queue(), ^{
        if (!speechSynth) {
            speechSynth = [AVSpeechSynthesizer new];
        }
        if (speechSynth.isSpeaking) {
            [speechSynth stopSpeakingAtBoundary:AVSpeechBoundaryImmediate];
        }
        AVSpeechUtterance *utterance = [AVSpeechUtterance speechUtteranceWithString:text];
        utterance.rate = MIN(AVSpeechUtteranceMaximumSpeechRate, AVSpeechUtteranceDefaultSpeechRate * 1.04f);
        AVSpeechSynthesisVoice *voice = [AVSpeechSynthesisVoice voiceWithLanguage:NSLocale.currentLocale.languageCode ?: @"en-US"];
        if (voice) utterance.voice = voice;
        [speechSynth speakUtterance:utterance];
    });
}
int beside_speaking(void) {
    if (NSThread.isMainThread) return speechSynth.isSpeaking;
    __block BOOL speaking=NO;
    dispatch_sync(dispatch_get_main_queue(), ^{ speaking=speechSynth.isSpeaking; });
    return speaking;
}
