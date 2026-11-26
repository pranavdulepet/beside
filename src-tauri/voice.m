#import <AppKit/AppKit.h>
#import <AVFoundation/AVFoundation.h>
#import <Speech/Speech.h>

extern NSDictionary *sumi_point_window(CGPoint p);
extern NSDictionary *sumi_read_point(NSDictionary *window, CGPoint p, NSDictionary *settings);
typedef void (*SumiVoiceCallback)(const char *);
@interface SumiVoice : NSObject
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
@property SumiVoiceCallback callback;
@end
static SumiVoice *current;
static NSValue *replyPosition;
static NSRect sourceWindow;
void sumi_position_reply(void *pointer) {
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
@implementation SumiVoice
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
    NSDictionary *payload=@{@"typed":@(self.typed), @"text":self.text ?: @"", @"issue":self.issue ?: @"", @"point":self.point ?: @{},@"generation":self.settings[@"generation"] ?: NSNull.null,@"epoch":self.settings[@"epoch"] ?: NSNull.null};
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
                    [SFSpeechRecognizer requestAuthorization:^(SFSpeechRecognizerAuthorizationStatus status){}];
            }];
        return;
    }
    if (SFSpeechRecognizer.authorizationStatus!=SFSpeechRecognizerAuthorizationStatusAuthorized) {
        [self fail:@"Allow Speech Recognition for Beside in System Settings, then hold the shortcut again. You can type here now."];
        if (SFSpeechRecognizer.authorizationStatus==SFSpeechRecognizerAuthorizationStatusNotDetermined)
            [SFSpeechRecognizer requestAuthorization:^(SFSpeechRecognizerAuthorizationStatus status){}];
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
    [input installTapOnBus:0 bufferSize:1024 format:format block:^(AVAudioPCMBuffer *buffer, AVAudioTime *when){ [request appendAudioPCMBuffer:buffer]; }];
    self.tapped=YES;
    __weak SumiVoice *weak=self;
    self.task=[self.recognizer recognitionTaskWithRequest:self.request resultHandler:^(SFSpeechRecognitionResult *r,NSError *error){
        dispatch_async(dispatch_get_main_queue(),^{
            SumiVoice *s=weak; if (!s || s.finished) return;
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

void sumi_voice_key(int down,const char *raw,SumiVoiceCallback callback) {
    NSData *data=raw ? [NSData dataWithBytes:raw length:strlen(raw)] : nil;
    dispatch_async(dispatch_get_main_queue(),^{
        if (!down) {
            SumiVoice *s=current; if (!s || s.released) return;
            if (!s.held) {
                // Quick tap = Point & Type: keep the live screen & cursor capture, skip mic.
                if ([s.window[@"bundle"] isEqual:@"com.apple.MobileSMS"]) {
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
        SumiVoice *s=[SumiVoice new]; current=s; s.callback=callback;
        s.settings=data ? [NSJSONSerialization JSONObjectWithData:data options:0 error:nil] : @{};
        s.mouse=NSEvent.mouseLocation;
        s.position=CGPointMake(s.mouse.x,NSMaxY(NSScreen.screens.firstObject.frame)-s.mouse.y);
        s.window=sumi_point_window(s.position);
        // Capture pointer + full-window screenshot immediately on key-down so both
        // Tap (type) and Hold (speak) have instant visual + pointer context.
        dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED,0),^{
            NSDictionary *point=sumi_read_point(s.window,s.position,s.settings);
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
