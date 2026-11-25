#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <Carbon/Carbon.h>
#import <CommonCrypto/CommonDigest.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import <ServiceManagement/ServiceManagement.h>
#import <Vision/Vision.h>

// Called only by the capture worker. Accessibility is used solely for the
// visible web document of an approved site. Native windows use ScreenCaptureKit.
static id attribute(AXUIElementRef element, CFStringRef key) {
    CFTypeRef value = NULL;
    if (AXUIElementCopyAttributeValue(element, key, &value) != kAXErrorSuccess)
        return nil;
    return CFBridgingRelease(value);
}
static CGRect bounds(AXUIElementRef node) {
    CGPoint p;
    CGSize s;
    id pos = attribute(node, kAXPositionAttribute), size = attribute(node, kAXSizeAttribute);
    if (!pos || !size || CFGetTypeID((__bridge CFTypeRef)pos) != AXValueGetTypeID() ||
        CFGetTypeID((__bridge CFTypeRef)size) != AXValueGetTypeID())
        return CGRectZero;
    if (!AXValueGetValue((__bridge AXValueRef)pos, kAXValueCGPointType, &p) ||
        !AXValueGetValue((__bridge AXValueRef)size, kAXValueCGSizeType, &s))
        return CGRectZero;
    return (CGRect){p, s};
}
static NSString *clean(id value) { return [value isKindOfClass:NSString.class] ? value : @""; }
static id webArea(id node, int depth, int *budget) {
    if (--*budget < 0 || depth > 14)
        return nil;
    AXUIElementRef el = (__bridge AXUIElementRef)node;
    NSString *role = attribute(el, kAXRoleAttribute);
    if ([role isEqual:@"AXWebArea"])
        return node;
    if ([role isEqual:@"AXToolbar"] || [role isEqual:@"AXTextField"])
        return nil;
    for (id child in attribute(el, kAXChildrenAttribute)) {
        id result = webArea(child, depth + 1, budget);
        if (result)
            return result;
    }
    return nil;
}
static NSString *hostOf(id node) {
    id url = attribute((__bridge AXUIElementRef)node, kAXURLAttribute);
    if ([url isKindOfClass:NSString.class])
        url = [NSURL URLWithString:url];
    if (![url isKindOfClass:NSURL.class] || ![[url scheme] isEqual:@"https"])
        return nil;
    return [[url host] lowercaseString];
}
static void visibleText(id node, CGRect viewport, int depth, int *budget, NSMutableString *text) {
    if (--*budget < 0 || depth > 24 || text.length > 5600)
        return;
    AXUIElementRef el = (__bridge AXUIElementRef)node;
    NSString *role = attribute(el, kAXRoleAttribute);
    if ((depth && [role isEqual:@"AXWebArea"]) || [role containsString:@"TextField"] ||
        [role isEqual:@"AXTextArea"] || [role isEqual:@"AXComboBox"])
        return;
    CGRect frame = bounds(el);
    if (!CGRectIsEmpty(frame) && !CGRectIntersectsRect(frame, viewport))
        return;
    if ([role isEqual:@"AXStaticText"] || [role isEqual:@"AXImage"]) {
        if (CGRectIsEmpty(frame))
            return;
        NSString *line = clean(
            attribute(el, [role isEqual:@"AXImage"] ? kAXDescriptionAttribute : kAXValueAttribute));
        if (line.length)
            [text appendFormat:@"%@\n", line];
        return;
    }
    for (id child in attribute(el, kAXChildrenAttribute)) {
        if (*budget <= 0 || text.length > 5600)
            break;
        visibleText(child, viewport, depth + 1, budget, text);
    }
}
static char *packed(NSDictionary *row) {
    NSData *bytes = [NSJSONSerialization dataWithJSONObject:row options:0 error:nil];
    return bytes
               ? strdup(
                     [[NSString alloc] initWithData:bytes encoding:NSUTF8StringEncoding].UTF8String)
               : NULL;
}
static char *status(NSString *detail) { return packed(@{@"status" : detail}); }
static BOOL browser(NSString *bundle) {
    return [bundle isEqual:@"com.apple.Safari"] || [bundle isEqual:@"com.google.Chrome"];
}
// Consult ScreenCaptureKit itself for an opted-in observation when the legacy
// preflight disagrees with Settings. A denial is remembered until a user retry.
static BOOL checkedScreenKit, allowedScreenKit;
static BOOL screenAccess(BOOL probe) {
    if (CGPreflightScreenCaptureAccess()) return YES;
    @synchronized ([SCShareableContent class]) {
        if (!probe || checkedScreenKit) return allowedScreenKit;
        checkedScreenKit = YES;
    }
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    [SCShareableContent getShareableContentExcludingDesktopWindows:YES onScreenWindowsOnly:YES
        completionHandler:^(SCShareableContent *content, NSError *error) {
            @synchronized ([SCShareableContent class]) {
                allowedScreenKit = !error && content.displays.count > 0;
                // Retry transient service failures on a later sample.
                if (error && error.code != SCStreamErrorUserDeclined) checkedScreenKit = NO;
            }
            dispatch_semaphore_signal(done);
        }];
    dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 3 * NSEC_PER_SEC));
    @synchronized ([SCShareableContent class]) { return allowedScreenKit; }
}
static void trackScreenError(NSError *error) {
    if (error.code == SCStreamErrorUserDeclined) {
        @synchronized ([SCShareableContent class]) { allowedScreenKit = NO; checkedScreenKit = YES; }
    }
}
int sumi_activity_permission(int web, int prompt) {
    if (!web) {
        if (!prompt) return screenAccess(NO);
        if (CGRequestScreenCaptureAccess()) return YES;
        @synchronized ([SCShareableContent class]) { checkedScreenKit = NO; }
        return screenAccess(YES);
    }
    if (!prompt)
        return AXIsProcessTrusted();
    return AXIsProcessTrustedWithOptions(
        (__bridge CFDictionaryRef)
            @{(__bridge NSString *)kAXTrustedCheckOptionPrompt : @YES});
}
static BOOL availableSession(void) {
    NSDictionary *session = CFBridgingRelease(CGSessionCopyCurrentDictionary());
    return session && ![session[@"CGSSessionScreenIsLocked"] boolValue] &&
           [session[(__bridge NSString *)kCGSessionOnConsoleKey] boolValue] &&
           !IsSecureEventInputEnabled();
}
int sumi_activity_awake(void) {
    return CGEventSourceSecondsSinceLastEventType(kCGEventSourceStateCombinedSessionState,
                                                  kCGAnyInputEventType) <= 120;
}
static BOOL stillFront(pid_t pid) {
    return availableSession() &&
           NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier == pid;
}
static CGWindowID frontWindow(pid_t pid) {
    NSArray *rows = CFBridgingRelease(CGWindowListCopyWindowInfo(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements, kCGNullWindowID));
    for (NSDictionary *r in rows) {
        if ([r[(__bridge NSString *)kCGWindowOwnerPID] intValue] == pid &&
            [r[(__bridge NSString *)kCGWindowLayer] intValue] == 0)
            return [r[(__bridge NSString *)kCGWindowNumber] unsignedIntValue];
    }
    return 0;
}
// Notes OCR can include the folders and note list. Anchor on the visible heading
// and read the editor column below it. Keep Apple's original strings for citations.
static NSArray<NSString *> *notePassage(NSArray<VNRecognizedTextObservation *> *rows,
                                         NSString *windowTitle) {
    VNRecognizedTextObservation *heading = nil;
    BOOL namedWindow = windowTitle.length && ![windowTitle isEqualToString:@"Notes"];
    double best = -1;
    for (VNRecognizedTextObservation *row in rows) {
        VNRecognizedText *candidate = [row topCandidates:1].firstObject;
        NSString *text = candidate.string;
        CGRect box = row.boundingBox;
        if (candidate.confidence < 0.55 || !text.length || CGRectGetMidY(box) < 0.40 ||
            CGRectGetMidY(box) > 0.97)
            continue;
        BOOL matches = windowTitle.length && [windowTitle caseInsensitiveCompare:text] == NSOrderedSame;
        if (namedWindow && !matches) continue;
        if (!matches && (box.size.height < 0.019 || CGRectGetMinX(box) < 0.015))
            continue;
        // A heading needs at least one readable line in its own column.
        BOOL hasBody = NO;
        for (VNRecognizedTextObservation *next in rows) {
            CGRect n = next.boundingBox;
            CGFloat dx = n.origin.x - box.origin.x, dy = box.origin.y - CGRectGetMaxY(n);
            VNRecognizedText *line = [next topCandidates:1].firstObject;
            if (dx >= -0.025 && dx < 0.09 && dy >= -0.003 && dy < 0.19 &&
                line.confidence >= 0.55 && line.string.length >= 4 &&
                (matches || box.size.height > n.size.height * 1.12)) {
                hasBody = YES;
                break;
            }
        }
        if (!hasBody) continue;
        double score = (matches ? 100 : 0) + box.size.height * 90 + box.origin.x + box.origin.y * 0.3;
        if (score > best) { best = score; heading = row; }
    }
    if (!heading) return nil;
    NSString *title = [heading topCandidates:1].firstObject.string;
    NSMutableArray *body = [NSMutableArray arrayWithObject:title];
    CGRect box = heading.boundingBox;
    NSArray *ordered = [rows sortedArrayUsingComparator:^NSComparisonResult(VNRecognizedTextObservation *a, VNRecognizedTextObservation *b) {
        CGFloat ay = CGRectGetMidY(a.boundingBox), by = CGRectGetMidY(b.boundingBox);
        return ay > by ? NSOrderedAscending : (ay < by ? NSOrderedDescending : NSOrderedSame);
    }];
    for (VNRecognizedTextObservation *row in ordered) {
        CGRect n = row.boundingBox;
        VNRecognizedText *line = [row topCandidates:1].firstObject;
        if (CGRectGetMidY(n) >= CGRectGetMinY(box) || n.origin.x < box.origin.x - 0.025 ||
            n.origin.x > box.origin.x + 0.09 || line.confidence < 0.55 || !line.string.length)
            continue;
        [body addObject:line.string];
    }
    return body.count > 1 ? body : nil;
}

static NSDictionary *recognize(CGImageRef image, NSString *source, NSString *title,
                               BOOL keepPreview, BOOL note) {
    VNRecognizeTextRequest *ocr = [VNRecognizeTextRequest new];
    ocr.recognitionLevel = VNRequestTextRecognitionLevelAccurate;
    VNImageRequestHandler *handler = [[VNImageRequestHandler alloc] initWithCGImage:image
                                                                            options:@{}];
    if ([handler performRequests:@[ ocr ] error:nil]) {
        NSMutableArray *lines = [NSMutableArray array];
        for (VNRecognizedTextObservation *r in ocr.results) {
            NSString *s = [r topCandidates:1].firstObject.string;
            if (s.length)
                [lines addObject:s];
        }
        if (note) {
            NSArray *passage = notePassage(ocr.results, title);
            if (!passage) return nil;
            title = [@"Notes · " stringByAppendingString:passage.firstObject];
            lines = [passage mutableCopy];
        }
        NSMutableDictionary *record = [@{
            @"source" : source,
            @"title" : title,
            @"body" : [lines componentsJoinedByString:@"\n"]
        } mutableCopy];
        if (keepPreview) {
            CGFloat iw = CGImageGetWidth(image), ih = CGImageGetHeight(image);
            CGFloat down = iw > 960.0 ? (960.0 / iw) : 1.0;
            size_t pw = MAX(1, (size_t)round(iw * down)), ph = MAX(1, (size_t)round(ih * down));
            CGImageRef thumb = image;
            CGImageRef owned = NULL;
            if (down < 1.0) {
                CGColorSpaceRef cs = CGColorSpaceCreateDeviceRGB();
                CGContextRef ctx = CGBitmapContextCreate(
                    NULL, pw, ph, 8, 0, cs, (CGBitmapInfo)kCGImageAlphaPremultipliedLast);
                if (ctx) {
                    CGContextSetInterpolationQuality(ctx, kCGInterpolationHigh);
                    CGContextDrawImage(ctx, CGRectMake(0, 0, pw, ph), image);
                    owned = CGBitmapContextCreateImage(ctx);
                    if (owned) thumb = owned;
                    CGContextRelease(ctx);
                }
                CGColorSpaceRelease(cs);
            }
            NSBitmapImageRep *rep = [[NSBitmapImageRep alloc] initWithCGImage:thumb];
            NSData *jpeg = [rep representationUsingType:NSBitmapImageFileTypeJPEG
                                             properties:@{
                                                 NSImageCompressionFactor : @0.58
                                             }];
            if (owned) CGImageRelease(owned);
            if (jpeg.length && jpeg.length <= 420000)
                record[@"preview"] = [jpeg base64EncodedStringWithOptions:0];
        }
        return record;
    }
    return nil;
}

// One display per tick keeps OCR off the main thread and bounds work on multi-display Macs.
static BOOL protectedApp(NSString *bundle) {
    return bundle &&
           ([bundle isEqual:NSBundle.mainBundle.bundleIdentifier] || [@[
                @"com.apple.Passwords", @"com.apple.keychainaccess", @"com.apple.SecurityAgent",
                @"com.apple.loginwindow", @"com.1password.1password", @"com.agilebits.onepassword7",
                @"com.bitwarden.desktop"
            ] containsObject:bundle]);
}
static NSString *displayTitle(CGDirectDisplayID display) {
    NSRunningApplication *front = NSWorkspace.sharedWorkspace.frontmostApplication;
    NSString *name = front.localizedName ?: @"Screen";
    CGDirectDisplayID ids[32];
    uint32_t count = 0;
    CGGetActiveDisplayList(32, ids, &count);
    for (uint32_t i = 0; i < count; i++)
        if (ids[i] == display) {
            NSArray *windows = CFBridgingRelease(CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements, kCGNullWindowID));
            for (NSDictionary *w in windows) {
                if ([w[(__bridge NSString *)kCGWindowOwnerPID] intValue] != front.processIdentifier || [w[(__bridge NSString *)kCGWindowLayer] intValue] != 0) continue;
                CGRect frame = CGRectZero;
                CGRectMakeWithDictionaryRepresentation((__bridge CFDictionaryRef)w[(__bridge NSString *)kCGWindowBounds], &frame);
                if (!CGRectIntersectsRect(frame, CGDisplayBounds(display))) break;
                NSString *topic = clean(w[(__bridge NSString *)kCGWindowName]);
                if (topic.length && ![topic isEqual:name])
                    return [NSString stringWithFormat:@"Screen %u · %@ · %@ active",i+1,topic,name];
                return [NSString stringWithFormat:@"Screen %u · %@ active", i+1,name];
            }
            return [NSString stringWithFormat:@"Screen %u", i+1];
        }
    return name;
}
static CGDirectDisplayID pointerDisplay(void) {
    CGEventRef cursor = CGEventCreate(NULL);
    if (!cursor)
        return CGMainDisplayID();
    CGPoint p = CGEventGetLocation(cursor);
    CFRelease(cursor);
    CGDirectDisplayID id;
    uint32_t n = 0;
    return CGGetDisplaysWithPoint(p, 1, &id, &n) == kCGErrorSuccess && n ? id : CGMainDisplayID();
}
static NSData *fingerprint(CGImageRef image) {
    uint8_t pixels[192 * 108], digest[CC_SHA256_DIGEST_LENGTH];
    CGColorSpaceRef gray = CGColorSpaceCreateDeviceGray();
    CGContextRef context =
        CGBitmapContextCreate(pixels, 192, 108, 8, 192, gray, (CGBitmapInfo)kCGImageAlphaNone);
    CGColorSpaceRelease(gray);
    if (!context)
        return nil;
    CGContextDrawImage(context, CGRectMake(0, 0, 192, 108), image);
    CGContextRelease(context);
    CC_SHA256(pixels, sizeof(pixels), digest);
    return [NSData dataWithBytes:digest length:sizeof(digest)];
}
static CGRect visibleNoteWindow(pid_t pid, CGWindowID wanted) {
    NSArray *windows = CFBridgingRelease(CGWindowListCopyWindowInfo(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements, kCGNullWindowID));
    for (NSDictionary *w in windows) {
        if ([w[(__bridge NSString *)kCGWindowLayer] intValue] != 0) continue;
        CGRect frame = CGRectZero;
        if ([w[(__bridge NSString *)kCGWindowOwnerPID] intValue] == pid &&
            [w[(__bridge NSString *)kCGWindowNumber] unsignedIntValue] == wanted)
            CGRectMakeWithDictionaryRepresentation((__bridge CFDictionaryRef)w[(__bridge NSString *)kCGWindowBounds], &frame);
        return frame;
    }
    return CGRectZero;
}

static NSDictionary *noteFrame(SCContentFilter *filter, SCStreamConfiguration *config,
                               NSString *source, NSString *title, BOOL preview) {
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    __block NSDictionary *row = nil;
    [SCScreenshotManager captureImageWithFilter:filter configuration:config
        completionHandler:^(CGImageRef image, NSError *error) {
            if (!error && image) row = recognize(image, source, title, preview, YES);
            dispatch_semaphore_signal(done);
        }];
    if (dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 4 * NSEC_PER_SEC))) return nil;
    return row;
}

static BOOL sameNoteRead(NSDictionary *row, NSDictionary *previous) {
    return row && [row[@"title"] isEqual:previous[@"title"]] && [row[@"body"] isEqual:previous[@"body"]];
}

static char *sampleNote(NSRunningApplication *app, NSString *source, BOOL preview) {
    if (!screenAccess(YES)) return status(@"Allow Screen Recording for Notes");
    pid_t pid = app.processIdentifier;
    CGWindowID wid = frontWindow(pid);
    CGRect frame = visibleNoteWindow(pid, wid);
    char * (^waiting)(void) = ^{ return status(@"Waiting for a readable Notes window"); };
    if (!wid || !stillFront(pid) || CGRectIsEmpty(frame)) return waiting();
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    __block SCWindow *window = nil;
    [SCShareableContent getShareableContentExcludingDesktopWindows:YES onScreenWindowsOnly:YES
        completionHandler:^(SCShareableContent *content, NSError *error) {
            if (error) { trackScreenError(error); dispatch_semaphore_signal(done); return; }
            for (SCWindow *w in content.windows) {
                if (w.windowID == wid && w.owningApplication.processID == pid) { window = w; break; }
            }
            dispatch_semaphore_signal(done);
        }];
    if (dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 3 * NSEC_PER_SEC)) || !window) return waiting();
    SCContentFilter *filter = [[SCContentFilter alloc] initWithDesktopIndependentWindow:window];
    SCStreamConfiguration *config = [SCStreamConfiguration new];
    CGFloat scale = MIN(2., 3000. / MAX(1., window.frame.size.width));
    config.width = MAX(1., window.frame.size.width * scale);
    config.height = MAX(1., window.frame.size.height * scale);
    config.showsCursor = NO;
    BOOL (^sameWindow)(void) = ^BOOL{ return stillFront(pid) && CGRectEqualToRect(frame, visibleNoteWindow(pid, wid)); };
    NSDictionary *previous = nil;
    for (int attempt = 0; attempt < 3; attempt++) {
        if (attempt) [NSThread sleepForTimeInterval:0.4];
        if (!sameWindow()) return waiting();
        NSDictionary *row = noteFrame(filter, config, source, window.title ?: @"", preview && previous != nil);
        if (!sameWindow()) return waiting();
        if (sameNoteRead(row, previous))
            return packed(row);
        previous = row;
    }
    return waiting();
}

// Sample one open window per pass, including windows covered by another app.
// Window pixels are discarded immediately after local OCR.
int sumi_workspace_available(void) {
    NSString *front = NSWorkspace.sharedWorkspace.frontmostApplication.bundleIdentifier;
    return availableSession() && (!protectedApp(front) || [front isEqual:NSBundle.mainBundle.bundleIdentifier]) && screenAccess(NO);
}
char *sumi_workspace_windows(unsigned long slot) {
    @autoreleasepool {
        if (!sumi_workspace_available()) return NULL;
        dispatch_semaphore_t ready = dispatch_semaphore_create(0);
        __block SCShareableContent *content = nil;
        [SCShareableContent getShareableContentExcludingDesktopWindows:YES onScreenWindowsOnly:NO
            completionHandler:^(SCShareableContent *value, NSError *error) {
                if (!error) content = value;
                dispatch_semaphore_signal(ready);
            }];
        if (dispatch_semaphore_wait(ready, dispatch_time(DISPATCH_TIME_NOW, 2 * NSEC_PER_SEC)) || !content) return NULL;
        pid_t front = NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier;
        CGWindowID focused = frontWindow(front);
        NSMutableArray<SCWindow *> *eligible = [NSMutableArray array];
        for (SCWindow *w in content.windows) {
            NSString *bundle = w.owningApplication.bundleIdentifier;
            if (!bundle.length || protectedApp(bundle) || [bundle hasPrefix:@"studio.sumi."] ||
                w.windowLayer != 0 || w.frame.size.width < 160 || w.frame.size.height < 80) continue;
            NSRunningApplication *app = [NSRunningApplication runningApplicationWithProcessIdentifier:w.owningApplication.processID];
            if (app.activationPolicy != NSApplicationActivationPolicyRegular) continue;
            [eligible addObject:w];
        }
        [eligible sortUsingComparator:^NSComparisonResult(SCWindow *a, SCWindow *b) {
            return [@(a.windowID) compare:@(b.windowID)];
        }];
        NSMutableArray *rows = [NSMutableArray array];
        for (SCWindow *w in eligible) {
            [rows addObject:[@{@"id":@(w.windowID), @"pid":@(w.owningApplication.processID),
                @"app":w.owningApplication.applicationName ?: @"App", @"title":w.title ?: @"",
                @"front":@(w.windowID == focused)} mutableCopy]];
        }
        if (rows.count) {
            NSUInteger index = slot % rows.count;
            SCWindow *w = eligible[index];
            SCContentFilter *filter = [[SCContentFilter alloc] initWithDesktopIndependentWindow:w];
            SCStreamConfiguration *cfg = [SCStreamConfiguration new];
            CGFloat scale = MIN(1.5, 1600./MAX(1.,MAX(w.frame.size.width,w.frame.size.height)));
            cfg.width = MAX(1., w.frame.size.width * scale);
            cfg.height = MAX(1., w.frame.size.height * scale);
            cfg.showsCursor = NO;
            dispatch_semaphore_t done = dispatch_semaphore_create(0);
            __block NSDictionary *read = nil;
            [SCScreenshotManager captureImageWithFilter:filter configuration:cfg completionHandler:^(CGImageRef image, NSError *error) {
                if (image && !error && availableSession()) read = recognize(image, @"window", w.title ?: @"", NO, NO);
                dispatch_semaphore_signal(done);
            }];
            if (!dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 3 * NSEC_PER_SEC))) {
                rows[index][@"text"] = read[@"body"] ?: @"";
            }
        }
        return sumi_workspace_available() ? packed(@{@"windows":rows}) : NULL;
    }
}

static char *sampleDisplay(BOOL preview, NSString *generation) {
    if (!screenAccess(YES))
        return status(@"Allow Screen Recording for your displays");
    NSRunningApplication *front = NSWorkspace.sharedWorkspace.frontmostApplication;
    if (!availableSession() || protectedApp(front.bundleIdentifier))
        return status(@"Waiting for an available screen");
    pid_t pid = front.processIdentifier;
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    __block NSDictionary *result = nil;
    static NSUInteger nextDisplay = 0;
    NSUInteger slot = nextDisplay++;
    [SCShareableContent
        getShareableContentExcludingDesktopWindows:NO
                               onScreenWindowsOnly:YES
                                 completionHandler:^(SCShareableContent *content, NSError *error) {
                                   if (error || !content.displays.count) {
                                       trackScreenError(error);
                                       dispatch_semaphore_signal(done);
                                       return;
                                   }
                                   SCDisplay *display =
                                       content.displays[slot % content.displays.count];
                                   if (CGDisplayIsAsleep(display.displayID)) {
                                       dispatch_semaphore_signal(done);
                                       return;
                                   }
                                   NSMutableArray *excluded = [NSMutableArray array];
                                   for (SCRunningApplication *app in content.applications)
                                       if (protectedApp(app.bundleIdentifier))
                                           [excluded addObject:app];
                                   SCContentFilter *filter =
                                       [[SCContentFilter alloc] initWithDisplay:display
                                                          excludingApplications:excluded
                                                               exceptingWindows:@[]];
                                   SCStreamConfiguration *config = [SCStreamConfiguration new];
                                   double scale = MIN(2., 2400. / MAX(1., display.width));
                                   config.width = MAX(1., display.width * scale);
                                   config.height = MAX(1., display.height * scale);
                                   config.showsCursor = NO;
                                   [SCScreenshotManager
                                       captureImageWithFilter:filter
                                                configuration:config
                                            completionHandler:^(CGImageRef image, NSError *issue) {
                                              trackScreenError(issue);
                                              if (image && !issue && stillFront(pid)) {
                                                  NSString *title = displayTitle(display.displayID);
                                                  static NSCache *history;
                                                  static dispatch_once_t once;
                                                  dispatch_once(&once, ^{
                                                    history = [NSCache new];
                                                    history.countLimit = 32;
                                                  });
                                                  NSString *key = [NSString
                                                      stringWithFormat:@"%@/%u/%d", generation,
                                                                       display.displayID, pid];
                                                  NSData *hash = fingerprint(image);
                                                  NSArray *last = [history objectForKey:key];
                                                  if (hash && [last.firstObject isEqual:hash] &&
                                                      -[last.lastObject timeIntervalSinceNow] <
                                                          45) {
                                                      result = @{@"status" : @"Observing Screen", @"observed" : @YES};
                                                  } else {
                                                      NSDictionary *row = recognize(
                                                          image, @"display", title, preview, NO);
                                                      if (stillFront(pid)) {
                                                          result = row;
                                                          if (row && hash)
                                                              [history
                                                                  setObject:@[ hash, [NSDate date] ]
                                                                     forKey:key];
                                                      }
                                                  }
                                              }
                                              dispatch_semaphore_signal(done);
                                            }];
                                 }];
    if (dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 8 * NSEC_PER_SEC)) ||
        !result)
        return status(@"Waiting for a readable screen");
    return packed(result);
}
// Read a window identity only for sources the user already allowed. The shortcut
// uses this before focusing its own window; no new capture is taken here.
// Whole-window guidance needs whole-window permission. Browser-host scopes
// continue to use the web-area-only pointer reader below.
BOOL sumi_overlay_allowed(const char *raw, NSString *bundle) {
    NSDictionary *cfg = [NSJSONSerialization JSONObjectWithData:
        [[NSString stringWithUTF8String:raw] dataUsingEncoding:NSUTF8StringEncoding] options:0 error:nil];
    if (![cfg[@"enabled"] boolValue] || !bundle.length || !availableSession() ||
        protectedApp(bundle) || !screenAccess(NO)) return NO;
    return [cfg[@"scope"] isEqual:@"displays"] ||
        (!browser(bundle) && [cfg[@"sources"] containsObject:bundle]);
}

static NSString *mailWindowTitle(NSString *name) {
    return [@"Mail · " stringByAppendingString:name.length ? name : @"Message"];
}
char *sumi_activity_front(const char *raw) {
    @autoreleasepool {
        NSDictionary *settings =
            [NSJSONSerialization JSONObjectWithData:[[NSString stringWithUTF8String:raw]
                                                        dataUsingEncoding:NSUTF8StringEncoding]
                                            options:0
                                              error:nil];
        NSArray *allowed = settings[@"sources"];
        if ([settings[@"scope"] isEqual:@"displays"]) {
            if (!availableSession() ||
                protectedApp(NSWorkspace.sharedWorkspace.frontmostApplication.bundleIdentifier) ||
                !screenAccess(NO))
                return NULL;
            NSRunningApplication *front = NSWorkspace.sharedWorkspace.frontmostApplication;
            if ([front.bundleIdentifier isEqual:@"com.apple.Notes"] || [front.bundleIdentifier isEqual:@"com.apple.mail"]) {
                CGWindowID wid = frontWindow(front.processIdentifier);
                NSArray *ws = CFBridgingRelease(CGWindowListCopyWindowInfo(kCGWindowListOptionIncludingWindow, wid));
                NSString *name = clean(ws.firstObject[(__bridge NSString *)kCGWindowName]);
                if ([front.bundleIdentifier isEqual:@"com.apple.mail"]) return packed(@{@"source":@"display", @"title":mailWindowTitle(name)});
                if (!name.length || [name isEqual:@"Notes"]) return NULL;
                return packed(@{@"source": @"display", @"title": [@"Notes · " stringByAppendingString:name]});
            }
            return packed(@{@"source" : @"display", @"title" : displayTitle(pointerDisplay())});
        }
        NSRunningApplication *app = NSWorkspace.sharedWorkspace.frontmostApplication;
        NSString *bundle = app.bundleIdentifier;
        if (!availableSession() || !bundle)
            return NULL;
        if (browser(bundle)) {
            if (!
                [allowed
                    filteredArrayUsingPredicate:[NSPredicate
                                                    predicateWithFormat:@"SELF BEGINSWITH 'site:'"]]
                    .count ||
                !AXIsProcessTrusted())
                return NULL;
            id ax = CFBridgingRelease(AXUIElementCreateApplication(app.processIdentifier));
            AXUIElementSetMessagingTimeout((__bridge AXUIElementRef)ax, 0.15);
            id window = attribute((__bridge AXUIElementRef)ax, kAXFocusedWindowAttribute);
            int budget = 100;
            id page = window ? webArea(window, 0, &budget) : nil;
            NSString *host = page ? hostOf(page) : nil;
            if (!host)
                return NULL;
            NSString *source = [@"site:" stringByAppendingString:host];
            if (![allowed containsObject:source])
                return NULL;
            return packed(@{
                @"source" : source,
                @"title" : clean(attribute((__bridge AXUIElementRef)page, kAXTitleAttribute))
            });
        }
        if (![allowed containsObject:bundle] || !CGPreflightScreenCaptureAccess())
            return NULL;
        CGWindowID wid = frontWindow(app.processIdentifier);
        if (!wid)
            return NULL;
        NSArray *windows =
            CFBridgingRelease(CGWindowListCopyWindowInfo(kCGWindowListOptionIncludingWindow, wid));
        NSDictionary *row = windows.firstObject;
        NSString *name = clean(row[(__bridge NSString *)kCGWindowName]);
        if ([bundle isEqual:@"com.apple.Notes"] && name.length)
            name = [@"Notes · " stringByAppendingString:name];
        return packed(@{@"source": bundle, @"title": name});
    }
}
char *sumi_activity_sample(const char *raw) {
    @autoreleasepool {
        NSDictionary *settings =
            [NSJSONSerialization JSONObjectWithData:[[NSString stringWithUTF8String:raw]
                                                        dataUsingEncoding:NSUTF8StringEncoding]
                                            options:0
                                              error:nil];
        NSArray *allowed = settings[@"sources"];
        BOOL keepPreview = [settings[@"visuals"] boolValue];
        NSRunningApplication *app = NSWorkspace.sharedWorkspace.frontmostApplication;
        NSString *bundle = app.bundleIdentifier;
        pid_t pid = app.processIdentifier;
        BOOL displays = [settings[@"scope"] isEqual:@"displays"];
        if ([bundle isEqual:@"com.apple.Notes"] && (displays || [allowed containsObject:bundle]))
            return sampleNote(app, displays ? @"display" : bundle, keepPreview);
        BOOL scopedMail = displays && [bundle isEqual:@"com.apple.mail"];
        if (displays && !scopedMail) return sampleDisplay(keepPreview, settings[@"generation"] ?: @"");
        if (!bundle || (!scopedMail && !browser(bundle) && ![allowed containsObject:bundle]))
            return status(@"Waiting for an allowed window");
        if (!availableSession())
            return status(@"Waiting for an unlocked window");
        if (browser(bundle)) {
            if (!
                [allowed
                    filteredArrayUsingPredicate:[NSPredicate
                                                    predicateWithFormat:@"SELF BEGINSWITH 'site:'"]]
                    .count)
                return status(@"Waiting for an allowed window");
            if (!AXIsProcessTrusted())
                return status(@"Allow Accessibility for website text");
            id ax = CFBridgingRelease(AXUIElementCreateApplication(pid));
            AXUIElementSetMessagingTimeout((__bridge AXUIElementRef)ax, 0.25);
            if ([bundle isEqual:@"com.google.Chrome"])
                AXUIElementSetAttributeValue((__bridge AXUIElementRef)ax,
                                             CFSTR("AXManualAccessibility"), kCFBooleanTrue);
            id window = attribute((__bridge AXUIElementRef)ax, kAXFocusedWindowAttribute);
            if (!window)
                return status(@"Waiting for a browser page");
            int budget = 220;
            id page = webArea(window, 0, &budget);
            NSString *host = page ? hostOf(page) : nil;
            NSString *source = host ? [@"site:" stringByAppendingString:host] : nil;
            if (!source || ![allowed containsObject:source])
                return status(@"Waiting for an allowed site");
            CGRect viewport = CGRectIntersection(bounds((__bridge AXUIElementRef)window),
                                                 bounds((__bridge AXUIElementRef)page));
            if (CGRectIsEmpty(viewport))
                return status(@"Waiting for visible page text");
            NSMutableString *text = [NSMutableString string];
            budget = 520;
            visibleText(page, viewport, 0, &budget, text);
            budget = 220;
            if (!stillFront(pid) || ![page isEqual:webArea(window, 0, &budget)] ||
                ![host isEqual:hostOf(page)] ||
                ![window isEqual:attribute((__bridge AXUIElementRef)ax, kAXFocusedWindowAttribute)])
                return NULL;
            return packed(@{
                @"source" : source,
                @"title" : clean(attribute((__bridge AXUIElementRef)page, kAXTitleAttribute)),
                @"body" : text
            });
        }
        if (!screenAccess(YES))
            return status(@"Allow Screen Recording for selected apps");
        CGWindowID wanted = frontWindow(pid);
        if (!wanted)
            return NULL;
        dispatch_semaphore_t done = dispatch_semaphore_create(0);
        __block NSDictionary *row = nil;
        [SCShareableContent
            getShareableContentExcludingDesktopWindows:YES
                                   onScreenWindowsOnly:YES
                                     completionHandler:^(SCShareableContent *content,
                                                         NSError *error) {
                                       if (error) {
                                           trackScreenError(error);
                                           dispatch_semaphore_signal(done);
                                           return;
                                       }
                                       SCWindow *selected = nil;
                                       for (SCWindow *w in content.windows)
                                           if (w.windowID == wanted &&
                                               [w.owningApplication.bundleIdentifier
                                                   isEqual:bundle]) {
                                               selected = w;
                                               break;
                                           }
                                       if (!selected || !stillFront(pid)) {
                                           dispatch_semaphore_signal(done);
                                           return;
                                       }
                                       SCContentFilter *filter = [[SCContentFilter alloc]
                                           initWithDesktopIndependentWindow:selected];
                                       SCStreamConfiguration *config = [SCStreamConfiguration new];
                                       config.showsCursor = NO;
                                       double scale =
                                           MIN(1.5, 1600. / MAX(1., selected.frame.size.width));
                                       config.width = MAX(1., selected.frame.size.width * scale);
                                       config.height = MAX(1., selected.frame.size.height * scale);
                                       [SCScreenshotManager
                                           captureImageWithFilter:filter
                                                    configuration:config
                                                completionHandler:^(CGImageRef image,
                                                                    NSError *issue) {
                                                  if (!issue && image && stillFront(pid) &&
                                                      frontWindow(pid) == wanted) {
                                                      row = recognize(image, scopedMail ? @"display" : bundle,
                                                                      scopedMail ? mailWindowTitle(selected.title) : selected.title ?: @"",
                                                                      keepPreview, NO);
                                                  }
                                                  dispatch_semaphore_signal(done);
                                                }];
                                     }];
        if (dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 8 * NSEC_PER_SEC)) !=
                0 ||
            !row)
            return status(@"Waiting for a readable window");
        return packed(row);
    }
}

// Enumerate names on demand. Browser windows continue to require an exact site.
char *sumi_activity_apps(void) {
    @autoreleasepool {
        NSWorkspace *workspace = NSWorkspace.sharedWorkspace;
        NSArray *webHandlers =
            [workspace URLsForApplicationsToOpenURL:[NSURL URLWithString:@"https://example.org"]];
        NSMutableSet *browsers = [NSMutableSet set];
        for (NSURL *url in webHandlers) {
            NSString *identifier = [NSBundle bundleWithURL:url].bundleIdentifier;
            if (identifier)
                [browsers addObject:identifier];
        }
        NSMutableDictionary *apps = [NSMutableDictionary dictionary];
        for (NSRunningApplication *app in workspace.runningApplications) {
            NSString *bundle = app.bundleIdentifier;
            if (!bundle || !app.localizedName ||
                app.activationPolicy != NSApplicationActivationPolicyRegular ||
                [browsers containsObject:bundle] || browser(bundle) ||
                [bundle isEqual:NSBundle.mainBundle.bundleIdentifier] ||
                [bundle isEqual:@"com.apple.systempreferences"] ||
                [bundle isEqual:@"com.apple.Passwords"])
                continue;
            apps[bundle] = app.localizedName;
        }
        return packed(apps);
    }
}

int sumi_login_status(void) { return (int)SMAppService.mainAppService.status; }
char *sumi_login_change(int enable) {
    @autoreleasepool {
        NSError *error = nil;
        SMAppService *service = SMAppService.mainAppService;
        BOOL ok = enable ? [service registerAndReturnError:&error]
                         : [service unregisterAndReturnError:&error];
        if (!ok)
            return strdup((error.localizedDescription ?: @"Check Login Items in System Settings.")
                              .UTF8String);
        return NULL;
    }
}

// Keep the lower corner where the user put it as the panel changes between a
// tab, a composer and a reading surface. AppKit provides the native curve.
void sumi_size_panel(void *pointer, int layout, bool initial) {
    NSWindow *w = (__bridge NSWindow *)pointer;
    if (initial) {
        w.titlebarAppearsTransparent = YES;
        w.titlebarSeparatorStyle = NSTitlebarSeparatorStyleNone;
        for (NSNumber *n in @[@(NSWindowCloseButton), @(NSWindowMiniaturizeButton), @(NSWindowZoomButton)])
            [w standardWindowButton:n.integerValue].hidden = YES;
    }
    NSRect area = (w.screen ?: NSScreen.mainScreen).visibleFrame;
    CGFloat width = MIN(layout == 0 ? 204 : 420, NSWidth(area));
    CGFloat height = MIN(layout == 0 ? 46 : layout, NSHeight(area));
    NSRect old = w.frame;
    CGFloat right = initial ? NSMaxX(area)-18 : NSMaxX(old);
    CGFloat bottom = initial ? NSMinY(area)+18 : NSMinY(old);
    NSRect target = NSMakeRect(MAX(NSMinX(area), MIN(right-width, NSMaxX(area)-width)),
                               MAX(NSMinY(area), MIN(bottom, NSMaxY(area)-height)), width, height);
    if (!NSEqualRects(target, old))
        [w setFrame:target display:YES animate:!initial && w.visible && !NSWorkspace.sharedWorkspace.accessibilityDisplayShouldReduceMotion];
}

// macOS supplies just the selection when someone invokes Services > Ask Beside.
static void (*receivedSelection)(const char *);
@interface SumiServices : NSObject
@end
@implementation SumiServices
- (void)askSumi:(NSPasteboard *)pasteboard userData:(NSString *)data error:(NSString **)error {
    NSString *text = [pasteboard stringForType:NSPasteboardTypeString];
    if (!text.length || text.length > 12000) {
        if (error) *error = @"Select between 1 and 12,000 characters to use in Beside.";
        return;
    }
    if (receivedSelection) receivedSelection(text.UTF8String);
}
@end
void sumi_register_service(void (*callback)(const char *)) {
    receivedSelection = callback;
    static SumiServices *provider;
    provider = [SumiServices new];
    NSApp.servicesProvider = provider;
    NSUpdateDynamicServices();
}

// Read the open transcript on an explicit shortcut. MessageEntryView and the
// conversation list are outside the transcript and are never prompt material.
static AXUIElementRef messageElement(AXUIElementRef parent, NSString *identifier, int depth) {
    if (!parent || depth < 0) return NULL;
    NSString *name = clean(attribute(parent, kAXIdentifierAttribute));
    if ([name isEqual:identifier]) return parent;
    if ([name isEqual:@"CKConversationListCollectionView"] || [name isEqual:@"MessageEntryView"]) return NULL;
    for (id child in attribute(parent,kAXChildrenAttribute)) {
        AXUIElementRef hit = messageElement((__bridge AXUIElementRef)child,identifier,depth-1);
        if (hit) return hit;
    }
    return NULL;
}

// Messages has distinct incoming and outgoing bubble lanes. Locate those lanes
// from the compose-field label and centered conversation header on this frame.
static NSDictionary *visibleMessageTurns(CGImageRef image, CGSize size) {
    VNRecognizeTextRequest *ocr = [VNRecognizeTextRequest new];
    ocr.recognitionLevel = VNRequestTextRecognitionLevelAccurate;
    VNImageRequestHandler *handler = [[VNImageRequestHandler alloc] initWithCGImage:image options:@{}];
    if (![handler performRequests:@[ocr] error:nil]) return nil;
    NSMutableArray *lines = [NSMutableArray array];
    CGFloat composeX = -1;
    for (VNRecognizedTextObservation *row in ocr.results) {
        VNRecognizedText *candidate = [row topCandidates:1].firstObject;
        CGRect r = row.boundingBox;
        CGFloat x = r.origin.x*size.width, y = (1-CGRectGetMidY(r))*size.height;
        if (candidate.confidence < (y < 85 ? .4 : .6)) continue;
        NSString *text = candidate.string;
        if (y > size.height-44 && r.size.height*size.height<23) {
            NSString *label = [[text.lowercaseString componentsSeparatedByCharactersInSet:NSCharacterSet.letterCharacterSet.invertedSet] componentsJoinedByString:@""];
            if ([@[@"message",@"imessage",@"textmessage",@"textmessagesms",@"textmessagercs"] containsObject:label]) composeX=x;
        }
        [lines addObject:@{@"text":text,@"x":@(x),@"y":@(y),@"w":@(r.size.width*size.width),@"h":@(r.size.height*size.height)}];
    }
    if (composeX<35) return nil;
    CGFloat center=(size.width+composeX-62)/2;
    NSString *contact=nil;
    NSCharacterSet *chevrons=[NSCharacterSet characterSetWithCharactersInString:@"›>»❯"];
    for (NSDictionary *line in lines) {
        CGFloat y=[line[@"y"] doubleValue], mid=[line[@"x"] doubleValue]+[line[@"w"] doubleValue]/2;
        NSString *text=line[@"text"];
        if (y<85 && fabs(mid-center)<40 && [text rangeOfCharacterFromSet:chevrons options:NSBackwardsSearch].location==text.length-1) {
            contact=[[text substringToIndex:text.length-1] stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceCharacterSet];
            break;
        }
    }
    if (!contact.length) return nil;
    NSBitmapImageRep *rep=[[NSBitmapImageRep alloc] initWithCGImage:image];
    NSColor *(^pixel)(CGFloat,CGFloat)=^NSColor *(CGFloat x,CGFloat y){
        NSInteger px=MAX(0,MIN(rep.pixelsWide-1,(NSInteger)(x*rep.pixelsWide/size.width)));
        NSInteger py=MAX(0,MIN(rep.pixelsHigh-1,(NSInteger)(y*rep.pixelsHigh/size.height)));
        return [[rep colorAtX:px y:py] colorUsingColorSpace:NSColorSpace.deviceRGBColorSpace];
    };
    NSArray *ordered=[lines sortedArrayUsingComparator:^NSComparisonResult(NSDictionary *a,NSDictionary *b){return [a[@"y"] compare:b[@"y"]];}];
    NSMutableArray *turns=[NSMutableArray array];
    CGFloat lastY=-100,lastX=0;
    for (NSDictionary *line in ordered) {
        CGFloat x=[line[@"x"] doubleValue], y=[line[@"y"] doubleValue], h=[line[@"h"] doubleValue];
        if (x<composeX-42 || y<90 || y>size.height-55 || h<8 || h>27) continue;
        NSColor *bg=pixel(x-3,y), *outside=pixel(x-24,y);
        if (!bg || !outside) continue;
        BOOL colored=MAX(bg.blueComponent,bg.greenComponent)-bg.redComponent>.29;
        BOOL mine=colored && x>composeX && (x+[line[@"w"] doubleValue]>size.width-80 || (y-lastY<21 && fabs(x-lastX)<5));
        CGFloat diff=fabs(bg.redComponent-outside.redComponent)+fabs(bg.blueComponent-outside.blueComponent)+fabs(bg.greenComponent-outside.greenComponent);
        BOOL received=!colored && x<=composeX-15 && diff>.075;
        if (!mine && !received) continue;
        NSMutableDictionary *last=turns.lastObject;
        if (last && [last[@"mine"] boolValue]==mine && y-lastY<20 && fabs(x-lastX)<6) {
            last[@"text"]=[NSString stringWithFormat:@"%@ %@",last[@"text"],line[@"text"]];
        } else {
            [turns addObject:[@{@"speaker":mine?@"Me":contact,@"mine":@(mine),@"visible":@YES,@"text":line[@"text"]} mutableCopy]];
        }
        lastY=y;lastX=x;
    }
    return turns.count ? @{@"turns":turns} : nil;
}

// Only borrow style from before a uniquely identifiable received line.
static NSDictionary *pointedMessage(NSDictionary *point, NSArray *turns) {
    NSString *line = point[@"line"];
    if (!line.length) return nil;
    NSInteger match = NSNotFound;
    for (NSUInteger i = 0; i < turns.count; i++) {
        NSDictionary *row = turns[i];
        NSString *text = row[@"text"];
        if (![row[@"visible"] boolValue] || !text.length ||
            [text rangeOfString:line options:NSCaseInsensitiveSearch].location == NSNotFound) continue;
        if (match != NSNotFound || [row[@"mine"] boolValue]) return nil;
        match = (NSInteger)i;
    }
    if (match == NSNotFound) return nil;
    NSMutableArray *examples = [NSMutableArray array];
    for (NSInteger j = match - 1; j >= 0 && examples.count < 4; j--) {
        NSString *sample = turns[j][@"text"];
        if ([turns[j][@"mine"] boolValue] && sample.length > 0 && sample.length <= 160)
            [examples insertObject:sample atIndex:0];
    }
    NSMutableDictionary *enriched = [point mutableCopy];
    enriched[@"incoming"] = turns[match][@"text"];
    enriched[@"style"] = examples;
    return enriched;
}

static NSDictionary *messagesOnScreen(NSRunningApplication *app) {
    if (!screenAccess(YES)) return nil;
    pid_t pid=app.processIdentifier;
    CGWindowID wid=frontWindow(pid);
    if (!wid) return nil;
    dispatch_semaphore_t done=dispatch_semaphore_create(0);
    __block NSDictionary *result=nil;
    [SCShareableContent getShareableContentExcludingDesktopWindows:YES onScreenWindowsOnly:YES completionHandler:^(SCShareableContent *content,NSError *error){
        for (SCWindow *w in content.windows) {
            if (error || w.windowID!=wid || w.owningApplication.processID!=pid) continue;
            SCContentFilter *filter=[[SCContentFilter alloc] initWithDesktopIndependentWindow:w];
            SCStreamConfiguration *config=[SCStreamConfiguration new];
            config.width=MAX(1,w.frame.size.width*2);
            config.height=MAX(1,w.frame.size.height*2);
            config.ignoreShadowsSingleWindow=YES;
            config.showsCursor=NO;
            [SCScreenshotManager captureImageWithFilter:filter configuration:config completionHandler:^(CGImageRef image,NSError *issue){
                if (image && !issue && stillFront(pid)) result=visibleMessageTurns(image,w.frame.size);
                dispatch_semaphore_signal(done);
            }];
            return;
        }
        dispatch_semaphore_signal(done);
    }];
    if (dispatch_semaphore_wait(done,dispatch_time(DISPATCH_TIME_NOW,3*NSEC_PER_SEC))) return nil;
    return stillFront(pid) && frontWindow(pid)==wid ? result : nil;
}

char *sumi_message_context(void) {
    @autoreleasepool {
        NSRunningApplication *app = NSWorkspace.sharedWorkspace.frontmostApplication;
        if (![app.bundleIdentifier isEqual:@"com.apple.MobileSMS"] || !availableSession()) return NULL;
        if (!AXIsProcessTrusted()) {
            NSDictionary *screen=messagesOnScreen(app);
            return packed(screen ?: @{@"needsAccess":@YES});
        }
        id owner = CFBridgingRelease(AXUIElementCreateApplication(app.processIdentifier));
        AXUIElementSetMessagingTimeout((__bridge AXUIElementRef)owner,0.25);
        id window = attribute((__bridge AXUIElementRef)owner,kAXFocusedWindowAttribute);
        AXUIElementRef transcript = messageElement((__bridge AXUIElementRef)window,@"TranscriptCollectionView",8);
        if (!transcript) return status(@"Open a conversation in Messages, then press ⌥⇧Space.");
        NSArray *rows = attribute(transcript,kAXChildrenAttribute);
        CGRect viewport = bounds(transcript);
        NSMutableArray *turns = [NSMutableArray array];
        for (NSUInteger i = rows.count > 36 ? rows.count-36 : 0; i < rows.count; i++) {
            AXUIElementRef row = (__bridge AXUIElementRef)rows[i];
            AXUIElementRef balloon = messageElement(row,@"CKBalloonTextView",3);
            if (!balloon) continue;
            NSString *text = clean(attribute(balloon,kAXValueAttribute));
            NSString *desc = clean(attribute(row,kAXDescriptionAttribute));
            NSRange boundary = [desc rangeOfString:@", "];
            if (!text.length || text.length > 1400 || boundary.location == NSNotFound) continue;
            NSString *speaker = [desc substringToIndex:boundary.location];
            if (![desc hasPrefix:[NSString stringWithFormat:@"%@, %@",speaker,text]]) continue;
            BOOL mine = [@[@"Your iMessage",@"Your text message",@"Your SMS",@"Your RCS message"] containsObject:speaker];
            BOOL visible = !CGRectIsEmpty(viewport) && CGRectIntersectsRect(viewport,bounds(balloon));
            [turns addObject:@{@"speaker":speaker,@"mine":@(mine),@"text":text,@"visible":@(visible)}];
        }
        if (!stillFront(app.processIdentifier) || ![window isEqual:attribute((__bridge AXUIElementRef)owner,kAXFocusedWindowAttribute)]) return NULL;
        return packed(@{@"turns":turns});
    }
}

// Resolve the target in the window that was under the pointer at key-down.
// Coordinates here use the global Quartz space (origin at the main display's top left).
NSDictionary *sumi_point_window(CGPoint p) {
    if (!availableSession()) return nil;
    NSArray *windows = CFBridgingRelease(CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements, kCGNullWindowID));
    for (NSDictionary *row in windows) {
        CGRect frame;
        if ([row[(__bridge NSString *)kCGWindowLayer] intValue] != 0 ||
            !CGRectMakeWithDictionaryRepresentation((__bridge CFDictionaryRef)row[(__bridge NSString *)kCGWindowBounds], &frame) ||
            !CGRectContainsPoint(frame,p)) continue;
        pid_t pid=[row[(__bridge NSString *)kCGWindowOwnerPID] intValue];
        NSRunningApplication *app=[NSRunningApplication runningApplicationWithProcessIdentifier:pid];
        if (!app.bundleIdentifier || protectedApp(app.bundleIdentifier)) return nil;
        return @{@"id":row[(__bridge NSString *)kCGWindowNumber], @"app":app.localizedName ?: @"App",
                 @"bundle":app.bundleIdentifier, @"pid":@(pid), @"frame":[NSValue valueWithRect:frame]};
    }
    return nil;
}

NSDictionary *sumi_read_point(NSDictionary *window, CGPoint point, NSDictionary *settings) {
    if (!window || ![settings[@"enabled"] boolValue]) return nil;
    pid_t pid = [window[@"pid"] intValue];
    CGWindowID wid = [window[@"id"] unsignedIntValue];
    BOOL displays = [settings[@"scope"] isEqual:@"displays"];
    BOOL siteOnly = !displays && browser(window[@"bundle"]);
    NSString *source = displays ? @"display" : window[@"bundle"];
    if (siteOnly) {
        if (!AXIsProcessTrusted() || !stillFront(pid) || frontWindow(pid)!=wid) return nil;
        id owner = CFBridgingRelease(AXUIElementCreateApplication(pid));
        int budget=100;
        id focused=attribute((__bridge AXUIElementRef)owner,kAXFocusedWindowAttribute);
        id page=focused ? webArea(focused,0,&budget) : nil;
        NSString *host=page ? hostOf(page) : nil;
        if (!host || !CGRectContainsPoint(bounds((__bridge AXUIElementRef)page),point)) return nil;
        source=[@"site:" stringByAppendingString:host];
        if (![settings[@"sources"] containsObject:source]) return nil;
        // Resolve only accessible content belonging to this exact authorized web area.
        AXUIElementRef raw=NULL;
        if (AXUIElementCopyElementAtPosition((__bridge AXUIElementRef)owner,point.x,point.y,&raw)!=kAXErrorSuccess) return nil;
        id el=CFBridgingRelease(raw), parent=el;
        for (int i=0; i<24 && parent && ![parent isEqual:page]; i++) parent=attribute((__bridge AXUIElementRef)parent,kAXParentAttribute);
        if (![parent isEqual:page] || !stillFront(pid)) return nil;
        NSString *role=clean(attribute((__bridge AXUIElementRef)el,kAXRoleAttribute));
        if ([role containsString:@"TextField"] || [role isEqual:@"AXTextArea"]) return nil;
        NSString *line=clean(attribute((__bridge AXUIElementRef)el,kAXTitleAttribute));
        if (!line.length) line=clean(attribute((__bridge AXUIElementRef)el,kAXValueAttribute));
        if (!line.length) line=clean(attribute((__bridge AXUIElementRef)el,kAXDescriptionAttribute));
        return line.length && line.length<1800 ? @{@"source":source,@"app":window[@"app"],@"line":line} : nil;
    }
    if (!displays && ![settings[@"sources"] containsObject:source]) return nil;
    if (!availableSession() || !screenAccess(YES)) return nil;
    dispatch_semaphore_t done=dispatch_semaphore_create(0);
    __block NSDictionary *result=nil;
    [SCShareableContent getShareableContentExcludingDesktopWindows:YES onScreenWindowsOnly:YES completionHandler:^(SCShareableContent *content,NSError *error) {
        for (SCWindow *w in content.windows) {
            if (w.windowID!=wid || w.owningApplication.processID!=pid || !CGRectContainsPoint(w.frame,point)) continue;
            CGRect frame=w.frame;
            CGPoint local=CGPointMake(point.x-frame.origin.x,point.y-frame.origin.y);
            CGRect crop=CGRectIntersection((CGRect){CGPointZero,frame.size},CGRectMake(local.x-260,local.y-115,520,230));
            if (CGRectIsEmpty(crop)) break;
            SCStreamConfiguration *config=[SCStreamConfiguration new];
            config.ignoreShadowsSingleWindow=YES;
            config.showsCursor=NO;
            CGFloat scale=MIN(2.,4096./MAX(frame.size.width,frame.size.height));
            config.width=ceil(frame.size.width*scale); config.height=ceil(frame.size.height*scale);
            SCContentFilter *filter=[[SCContentFilter alloc] initWithDesktopIndependentWindow:w];
            [SCScreenshotManager captureImageWithFilter:filter configuration:config completionHandler:^(CGImageRef image,NSError *issue) {
                if (image && !issue && [sumi_point_window(point)[@"id"] unsignedIntValue]==wid) {
                    // Crop in the returned window's own pixel coordinates. Window filters can ignore sourceRect.
                    CGFloat sx=CGImageGetWidth(image)/frame.size.width, sy=CGImageGetHeight(image)/frame.size.height;
                    CGImageRef patch=CGImageCreateWithImageInRect(image,CGRectIntegral(CGRectMake(crop.origin.x*sx,crop.origin.y*sy,crop.size.width*sx,crop.size.height*sy)));
                    // OCR the full width so a long note or message keeps its first and last words.
                    CGImageRef band=CGImageCreateWithImageInRect(image,CGRectIntegral(CGRectMake(0,crop.origin.y*sy,CGImageGetWidth(image),crop.size.height*sy)));
                    if (!patch || !band) { if (patch) CGImageRelease(patch); if (band) CGImageRelease(band); dispatch_semaphore_signal(done);return; }
                    VNRecognizeTextRequest *ocr=[VNRecognizeTextRequest new];
                    ocr.recognitionLevel=VNRequestTextRecognitionLevelAccurate;
                    VNImageRequestHandler *handler=[[VNImageRequestHandler alloc] initWithCGImage:band options:@{}];
                    [handler performRequests:@[ocr] error:nil];
                    NSMutableArray *nearby=[NSMutableArray new];
                    NSString *line=@"";
                    double nearest=42;
                    for (VNRecognizedTextObservation *r in ocr.results) {
                        VNRecognizedText *candidate=[r topCandidates:1].firstObject;
                        if (!candidate.string.length || candidate.confidence<.40) continue;
                        CGRect box=r.boundingBox;
                        CGFloat x=box.origin.x*frame.size.width,y=crop.origin.y+(1-CGRectGetMaxY(box))*crop.size.height;
                        CGFloat dx=MAX(MAX(x-local.x,0),local.x-x-box.size.width*frame.size.width);
                        CGFloat dy=MAX(MAX(y-local.y,0),local.y-y-box.size.height*crop.size.height);
                        double distance=hypot(dx,dy*1.8);
                        if (distance<nearest) { nearest=distance; line=candidate.string; }
                        if (dx<220 && nearby.count<16) [nearby addObject:candidate.string];
                    }
                    NSData *jpeg=[[[NSBitmapImageRep alloc] initWithCGImage:patch] representationUsingType:NSBitmapImageFileTypeJPEG properties:@{NSImageCompressionFactor:@0.72}];
                    // Also produce a compact full-window screenshot (Clicky-style visual context)
                    // scaled cleanly to max 1280px wide so multimodal models see the full layout.
                    NSString *fullB64 = @"";
                    CGFloat iw = CGImageGetWidth(image), ih = CGImageGetHeight(image);
                    CGFloat down = iw > 1280.0 ? (1280.0 / iw) : 1.0;
                    size_t fw = MAX(1, (size_t)round(iw * down)), fh = MAX(1, (size_t)round(ih * down));
                    CGColorSpaceRef cs = CGColorSpaceCreateDeviceRGB();
                    CGContextRef ctx = CGBitmapContextCreate(NULL, fw, fh, 8, 0, cs, (CGBitmapInfo)kCGImageAlphaPremultipliedLast);
                    if (ctx) {
                        CGContextSetInterpolationQuality(ctx, kCGInterpolationHigh);
                        CGContextDrawImage(ctx, CGRectMake(0, 0, fw, fh), image);
                        CGImageRef scaled = CGBitmapContextCreateImage(ctx);
                        if (scaled) {
                            NSData *fullJpeg = [[[NSBitmapImageRep alloc] initWithCGImage:scaled] representationUsingType:NSBitmapImageFileTypeJPEG properties:@{NSImageCompressionFactor:@0.68}];
                            if (fullJpeg.length > 0 && fullJpeg.length <= 900000) {
                                fullB64 = [fullJpeg base64EncodedStringWithOptions:0] ?: @"";
                            }
                            CGImageRelease(scaled);
                        }
                        CGContextRelease(ctx);
                    }
                    CGColorSpaceRelease(cs);
                    CGFloat xPct = frame.size.width > 0 ? round((local.x / frame.size.width) * 1000.0) / 10.0 : 50.0;
                    CGFloat yPct = frame.size.height > 0 ? round((local.y / frame.size.height) * 1000.0) / 10.0 : 50.0;
                    CGImageRelease(band); CGImageRelease(patch);
                    result=@{
                        @"source":source,
                        @"app":window[@"app"],
                        @"bundle":window[@"bundle"],
                        @"line":line,
                        @"nearby":nearby,
                        @"image":[jpeg base64EncodedStringWithOptions:0] ?: @"",
                        @"fullImage":fullB64,
                        @"cursor":@{@"x_pct":@(xPct), @"y_pct":@(yPct)},
                        @"windowFrame":@{@"x":@(round(frame.origin.x)), @"y":@(round(frame.origin.y)), @"w":@(round(frame.size.width)), @"h":@(round(frame.size.height))}
                    };
                    if ([window[@"bundle"] isEqual:@"com.apple.MobileSMS"] && line.length) {
                        // Sender lanes and the pointed words come from the same saved frame.
                        NSArray *rows = visibleMessageTurns(image,frame.size)[@"turns"];
                        result = pointedMessage(result,rows) ?: result;
                    }
                }
                dispatch_semaphore_signal(done);
            }];
            return;
        }
        dispatch_semaphore_signal(done);
    }];
    if (dispatch_semaphore_wait(done,dispatch_time(DISPATCH_TIME_NOW,4*NSEC_PER_SEC))) return nil;
    if (result && ![result[@"style"] count] && [window[@"bundle"] isEqual:@"com.apple.MobileSMS"] && stillFront(pid) && frontWindow(pid)==wid) {
        char *raw=sumi_message_context();
        if (raw) {
            NSData *bytes=[NSData dataWithBytes:raw length:strlen(raw)]; free(raw);
            NSArray *rows=[NSJSONSerialization JSONObjectWithData:bytes options:0 error:nil][@"turns"];
            result=pointedMessage(result,rows) ?: result;
        }
    }
    return result;
}
