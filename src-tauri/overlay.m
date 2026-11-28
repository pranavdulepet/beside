#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <QuartzCore/QuartzCore.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import <Vision/Vision.h>

static id ax_attr(AXUIElementRef el, CFStringRef key) {
    CFTypeRef val = NULL;
    if (AXUIElementCopyAttributeValue(el, key, &val) != kAXErrorSuccess) return nil;
    return CFBridgingRelease(val);
}

static CGRect ax_bounds(AXUIElementRef el) {
    CGPoint p;
    CGSize s;
    id pos = ax_attr(el, kAXPositionAttribute);
    id size = ax_attr(el, kAXSizeAttribute);
    if (!pos || !size) return CGRectZero;
    if (CFGetTypeID((__bridge CFTypeRef)pos) != AXValueGetTypeID() ||
        CFGetTypeID((__bridge CFTypeRef)size) != AXValueGetTypeID()) return CGRectZero;
    if (!AXValueGetValue((__bridge AXValueRef)pos, kAXValueCGPointType, &p) ||
        !AXValueGetValue((__bridge AXValueRef)size, kAXValueCGSizeType, &s)) return CGRectZero;
    return (CGRect){p, s};
}

static NSString *ax_clean(id v) {
    return [v isKindOfClass:NSString.class]
        ? [v stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet]
        : @"";
}

static BOOL interactive_role(NSString *role) {
    static NSSet *roles;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        roles = [NSSet setWithArray:@[
            @"AXButton", @"AXMenuButton", @"AXPopUpButton", @"AXCheckBox",
            @"AXRadioButton", @"AXTab", @"AXLink", @"AXTextField",
            @"AXSearchField", @"AXMenuItem", @"AXSlider", @"AXDisclosureTriangle",
            @"AXMenuBarItem", @"AXToolbarButton", @"AXSegmentedControl",
            @"AXCell", @"AXRow"
        ]];
    });
    return [roles containsObject:role];
}

static NSString *short_role(NSString *role) {
    if ([role hasPrefix:@"AX"]) return [[role substringFromIndex:2] lowercaseString];
    return role.lowercaseString ?: @"control";
}

static NSString *region_for_rect(CGRect r, CGRect windowFrame) {
    if (windowFrame.size.width <= 1 || windowFrame.size.height <= 1) return @"window";
    CGFloat rx = (CGRectGetMidX(r) - windowFrame.origin.x) / windowFrame.size.width;
    CGFloat ry = (CGRectGetMidY(r) - windowFrame.origin.y) / windowFrame.size.height;
    if (ry < 0.11) {
        if (rx < 0.25) return @"top-left bar";
        if (rx > 0.75) return @"top-right bar";
        return @"top bar";
    }
    if (ry > 0.89) {
        if (rx < 0.25) return @"bottom-left";
        if (rx > 0.75) return @"bottom-right";
        return @"bottom bar";
    }
    if (rx < 0.24) return @"left sidebar";
    if (rx > 0.78) return @"right sidebar";
    return @"main area";
}

static BOOL skip_bundle(NSString *bundle) {
    if (!bundle.length) return YES;
    NSString *selfBundle = NSBundle.mainBundle.bundleIdentifier ?: @"";
    if ([bundle isEqualToString:selfBundle]) return YES;
    static NSSet *excluded;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        excluded = [NSSet setWithArray:@[
            @"com.apple.dock",
            @"com.apple.controlcenter",
            @"com.apple.notificationcenterui",
            @"com.apple.WindowManager",
            @"com.apple.SystemUIServer",
            @"com.apple.Spotlight",
            @"com.apple.Passwords",
            @"com.apple.keychainaccess",
            @"com.apple.SecurityAgent",
            @"com.apple.loginwindow",
            @"com.1password.1password",
            @"com.agilebits.onepassword7",
            @"com.bitwarden.desktop"
        ]];
    });
    return [excluded containsObject:bundle];
}

static void collect_elements(
    AXUIElementRef node,
    CGRect windowFrame,
    int depth,
    int *budget,
    NSMutableArray *out
) {
    if (!node || depth > 14 || --(*budget) < 0 || out.count >= 30) return;
    NSString *role = ax_clean(ax_attr(node, kAXRoleAttribute));
    if (interactive_role(role)) {
        CGRect r = ax_bounds(node);
        if (r.size.width >= 10 && r.size.height >= 10 &&
            r.size.width <= windowFrame.size.width &&
            r.size.height <= windowFrame.size.height &&
            CGRectIntersectsRect(r, windowFrame)) {
            NSString *label = ax_clean(ax_attr(node, kAXTitleAttribute));
            if (!label.length) label = ax_clean(ax_attr(node, kAXDescriptionAttribute));
            if (!label.length) label = ax_clean(ax_attr(node, kAXHelpAttribute));
            if (!label.length && ![role containsString:@"TextField"]) {
                label = ax_clean(ax_attr(node, kAXValueAttribute));
            }
            if (label.length >= 2 && label.length <= 72) {
                NSString *eid = [NSString stringWithFormat:@"E%lu", (unsigned long)(out.count + 1)];
                [out addObject:@{
                    @"id": eid,
                    @"role": short_role(role),
                    @"label": label,
                    @"region": region_for_rect(r, windowFrame),
                    @"x": @(round(r.origin.x)),
                    @"y": @(round(r.origin.y)),
                    @"w": @(round(r.size.width)),
                    @"h": @(round(r.size.height))
                }];
            }
        }
    }
    NSArray *children = ax_attr(node, kAXChildrenAttribute);
    if (![children isKindOfClass:NSArray.class]) return;
    for (id child in children) {
        if (*budget <= 0 || out.count >= 30) break;
        collect_elements((__bridge AXUIElementRef)child, windowFrame, depth + 1, budget, out);
    }
}

static BOOL already_captured(NSArray *elements, NSString *label, CGRect rect) {
    NSString *needle = label.lowercaseString;
    for (NSDictionary *el in elements) {
        CGRect existing = CGRectMake(
            [el[@"x"] doubleValue],
            [el[@"y"] doubleValue],
            [el[@"w"] doubleValue],
            [el[@"h"] doubleValue]
        );
        NSString *lbl = [el[@"label"] lowercaseString];
        if ([lbl isEqualToString:needle] && CGRectIntersectsRect(CGRectInset(existing, -14, -14), rect)) {
            return YES;
        }
    }
    return NO;
}

static NSString *strip_leading_glyph(NSString *raw) {
    NSString *s = [raw stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
    // Strip common leading bullet/icon OCR artifacts like "+ ", "• ", "· ", "& ", "o ", "0 ", "Q "
    while (s.length > 2) {
        unichar c = [s characterAtIndex:0];
        BOOL iconPrefix =
            c == '+' || c == '-' || c == '*' || c == '&' || c == '>' || c == '<' ||
            c == '@' || c == 0x2022 || c == 0x00B7 || c == 0x00BF || c == 0x25CB ||
            c == 0x25CF || c == 0x25A1 || c == 0xF8FF || c == 0x00A9 || c == 0x00AE;
        BOOL singleGlyphWord =
            s.length > 3 && [s characterAtIndex:1] == ' ' &&
            (c == 'o' || c == 'O' || c == '0' || c == 'Q' || c == 'c' || c == 'C' || c == 'v');
        if (iconPrefix || singleGlyphWord) {
            NSString *rest = [[s substringFromIndex:1] stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceCharacterSet];
            if (rest.length >= 2) { s = rest; continue; }
        }
        break;
    }
    // Capitalize single-word lowercase OCR artifacts like "automations" -> "Automations"
    if (s.length >= 2 && [[NSCharacterSet lowercaseLetterCharacterSet] characterIsMember:[s characterAtIndex:0]]) {
        s = [[[s substringToIndex:1] uppercaseString] stringByAppendingString:[s substringFromIndex:1]];
    }
    return s;
}

static void run_vision_ocr_on_cgimage(CGImageRef image, CGRect frame, NSMutableArray *elements) {
    if (!image || elements.count >= 64) return;
    VNRecognizeTextRequest *ocr = [VNRecognizeTextRequest new];
    ocr.recognitionLevel = VNRequestTextRecognitionLevelAccurate;
    ocr.usesLanguageCorrection = NO;
    VNImageRequestHandler *handler = [[VNImageRequestHandler alloc] initWithCGImage:image options:@{}];
    if ([handler performRequests:@[ocr] error:nil]) {
        for (VNRecognizedTextObservation *obs in ocr.results) {
            if (elements.count >= 64) break;
            VNRecognizedText *cand = [obs topCandidates:1].firstObject;
            if (!cand || cand.confidence < 0.45) continue;
            NSString *label = strip_leading_glyph(cand.string);
            if (label.length < 2 || label.length > 56) continue;
            CGRect b = obs.boundingBox;
            CGRect r = CGRectMake(
                frame.origin.x + b.origin.x * frame.size.width,
                frame.origin.y + (1.0 - CGRectGetMaxY(b)) * frame.size.height,
                MAX(18.0, b.size.width * frame.size.width),
                MAX(16.0, b.size.height * frame.size.height)
            );
            if (already_captured(elements, label, r)) continue;
            NSString *eid = [NSString stringWithFormat:@"E%lu", (unsigned long)(elements.count + 1)];
            [elements addObject:@{
                @"id": eid,
                @"role": @"ui",
                @"label": label,
                @"region": region_for_rect(r, frame),
                @"x": @(round(r.origin.x)),
                @"y": @(round(r.origin.y)),
                @"w": @(round(r.size.width)),
                @"h": @(round(r.size.height))
            }];
        }
    }
}

static NSString *collect_vision_elements(
    CGWindowID wid,
    pid_t pid,
    CGRect fallbackFrame,
    NSMutableArray *elements
) {
    if (!wid) return nil;
    NSMutableArray *observed=[elements mutableCopy];
    __block NSString *pixels=nil;
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    [SCShareableContent getShareableContentExcludingDesktopWindows:YES
                                               onScreenWindowsOnly:YES
                                                 completionHandler:^(SCShareableContent *content, NSError *error) {
        if (error) {
            dispatch_semaphore_signal(done);
            return;
        }
        SCWindow *target = nil;
        for (SCWindow *w in content.windows) {
            if (w.windowID == wid && w.owningApplication.processID == pid) {
                target = w;
                break;
            }
        }
        if (!target) {
            dispatch_semaphore_signal(done);
            return;
        }
        CGRect frame = CGRectIsEmpty(target.frame) ? fallbackFrame : target.frame;
        SCStreamConfiguration *cfg = [SCStreamConfiguration new];
        cfg.ignoreShadowsSingleWindow = YES;
        cfg.showsCursor = NO;
        CGFloat scale = MIN(1.5, 1800.0 / MAX(1.0, MAX(frame.size.width, frame.size.height)));
        cfg.width = MAX(1, (size_t)ceil(frame.size.width * scale));
        cfg.height = MAX(1, (size_t)ceil(frame.size.height * scale));
        SCContentFilter *filter = [[SCContentFilter alloc] initWithDesktopIndependentWindow:target];
        [SCScreenshotManager captureImageWithFilter:filter
                                      configuration:cfg
                                  completionHandler:^(CGImageRef image, NSError *issue) {
            if (image && !issue) {
                run_vision_ocr_on_cgimage(image, frame, observed);
                CGImageRef visionImg = image;
                CGImageRef scaled = NULL;
                size_t iw = CGImageGetWidth(image), ih = CGImageGetHeight(image);
                if (iw > 1024 || ih > 1024) {
                    CGFloat s = 1024.0 / (CGFloat)MAX(iw, ih);
                    size_t tw = MAX(1, (size_t)llround(iw * s));
                    size_t th = MAX(1, (size_t)llround(ih * s));
                    CGColorSpaceRef cs = CGColorSpaceCreateDeviceRGB();
                    CGContextRef ctx = CGBitmapContextCreate(NULL, tw, th, 8, 0, cs, (CGBitmapInfo)kCGImageAlphaPremultipliedLast);
                    CGColorSpaceRelease(cs);
                    if (ctx) {
                        CGContextSetInterpolationQuality(ctx, kCGInterpolationHigh);
                        CGContextDrawImage(ctx, CGRectMake(0, 0, tw, th), image);
                        scaled = CGBitmapContextCreateImage(ctx);
                        CGContextRelease(ctx);
                        if (scaled) visionImg = scaled;
                    }
                }
                NSBitmapImageRep *rep=[[NSBitmapImageRep alloc] initWithCGImage:visionImg];
                if (scaled) CGImageRelease(scaled);
                NSData *jpeg=[rep representationUsingType:NSBitmapImageFileTypeJPEG properties:@{NSImageCompressionFactor:@0.70}];
                if (jpeg.length<=1600000) pixels=[jpeg base64EncodedStringWithOptions:0];
            }
            dispatch_semaphore_signal(done);
        }];
    }];
    if (dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 4 * NSEC_PER_SEC))) return nil;
    [elements setArray:observed];
    return pixels;
}

extern BOOL beside_guidance_session(void);

static NSDictionary *pick_target_window(void) {
    if (!beside_guidance_session()) return nil;
    pid_t selfPid = NSProcessInfo.processInfo.processIdentifier;
    NSArray *windows = CFBridgingRelease(CGWindowListCopyWindowInfo(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        kCGNullWindowID
    ));
    NSRunningApplication *front=NSWorkspace.sharedWorkspace.frontmostApplication;
    if (!front || (front.processIdentifier!=selfPid && skip_bundle(front.bundleIdentifier))) return nil;
    for (NSDictionary *row in windows) {
        if ([row[(__bridge NSString *)kCGWindowLayer] intValue] != 0) continue;
        pid_t pid = [row[(__bridge NSString *)kCGWindowOwnerPID] intValue];
        if (pid <= 0 || pid == selfPid) continue;
        CGRect frame = CGRectZero;
        if (!CGRectMakeWithDictionaryRepresentation(
                (__bridge CFDictionaryRef)row[(__bridge NSString *)kCGWindowBounds], &frame)) continue;
        if (frame.size.width < 180 || frame.size.height < 120) continue;
        NSRunningApplication *app = [NSRunningApplication runningApplicationWithProcessIdentifier:pid];
        if (!app || skip_bundle(app.bundleIdentifier)) return nil;
        if (front.processIdentifier!=selfPid && front.processIdentifier!=pid) return nil;
        NSString *appName = app.localizedName ?: @"App";
        NSDictionary *candidate = @{
            @"wid": row[(__bridge NSString *)kCGWindowNumber],
            @"pid": @(pid),
            @"app": appName,
            @"bundle": app.bundleIdentifier ?: @"",
            @"frame": [NSValue valueWithRect:frame]
        };
        return candidate;
    }
    return nil;
}

extern BOOL beside_overlay_allowed(const char *raw, NSString *bundle);

static BOOL matches_owner(NSDictionary *owner) {
    if (![owner isKindOfClass:NSDictionary.class]) return NO;
    NSDictionary *front=pick_target_window();
    if (!front || ![front[@"pid"] isEqual:owner[@"pid"]] || ![front[@"wid"] isEqual:owner[@"wid"]] ||
        ![front[@"bundle"] isEqual:owner[@"bundle"]]) return NO;
    CGRect r=[front[@"frame"] rectValue];
    NSDictionary *f=owner[@"window"];
    return f && round(r.origin.x)==[f[@"x"] doubleValue] && round(r.origin.y)==[f[@"y"] doubleValue] &&
        round(r.size.width)==[f[@"w"] doubleValue] && round(r.size.height)==[f[@"h"] doubleValue];
}

int beside_window_current(const char *json) {
    if (!json) return 0;
    @autoreleasepool {
        NSData *data=[NSData dataWithBytes:json length:strlen(json)];
        return matches_owner([NSJSONSerialization JSONObjectWithData:data options:0 error:nil]);
    }
}

char *beside_capture_front_elements(const char *settings) {
    @autoreleasepool {
        NSDictionary *target = pick_target_window();
        if (!target || !beside_overlay_allowed(settings,target[@"bundle"])) return strdup("{\"elements\":[]}");

        pid_t pid = [target[@"pid"] intValue];
        CGWindowID wid = [target[@"wid"] unsignedIntValue];
        CGRect frame = [target[@"frame"] rectValue];
        NSString *appName = target[@"app"] ?: @"App";
        NSString *bundle = target[@"bundle"] ?: @"";

        NSMutableArray *elements = [NSMutableArray array];

        // 1. Try Accessibility tree (enable AXManualAccessibility for Electron/Chromium apps).
        if (AXIsProcessTrusted()) {
            AXUIElementRef appRef = AXUIElementCreateApplication(pid);
            if (appRef) {
                AXUIElementSetMessagingTimeout(appRef, 0.25);
                AXUIElementSetAttributeValue(appRef, CFSTR("AXManualAccessibility"), kCFBooleanTrue);
                id win = ax_attr(appRef, kAXFocusedWindowAttribute);
                if (!win) {
                    NSArray *wins = ax_attr(appRef, kAXWindowsAttribute);
                    if ([wins isKindOfClass:NSArray.class] && wins.count > 0) win = wins.firstObject;
                }
                if (win) {
                    CGRect axFrame = ax_bounds((__bridge AXUIElementRef)win);
                    NSNumber *number=ax_attr((__bridge AXUIElementRef)win,CFSTR("AXWindowNumber"));
                    if ([number isKindOfClass:NSNumber.class] && number.unsignedIntValue==wid && CGRectEqualToRect(axFrame,frame)) {
                        int budget=220;
                        collect_elements((__bridge AXUIElementRef)win,frame,0,&budget,elements);
                    }
                }
                CFRelease(appRef);
            }
        }

        // 2. Augment with ScreenCaptureKit + Apple Vision OCR bounding boxes so Electron,
        //    web apps, Figma, Antigravity, and Safari controls always have pixel-exact boxes.
        NSString *pixels=collect_vision_elements(wid,pid,frame,elements);

        NSMutableDictionary *result = [NSMutableDictionary dictionaryWithDictionary:@{
            @"app": appName, @"wid":@(wid), @"pid":@(pid),
            @"bundle": bundle,
            @"elements": elements,
            @"window": @{
                @"x": @(round(frame.origin.x)),
                @"y": @(round(frame.origin.y)),
                @"w": @(round(frame.size.width)),
                @"h": @(round(frame.size.height))
            }
        }];
        if (!matches_owner(result) || !beside_overlay_allowed(settings,bundle)) return strdup("{\"elements\":[]}");
        if (pixels.length) result[@"image"]=pixels;
        if (!AXIsProcessTrusted()) result[@"needsAccessibility"] = @YES;

        NSData *data = [NSJSONSerialization dataWithJSONObject:result options:0 error:nil];
        return data
            ? strdup([[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding].UTF8String)
            : strdup("{\"elements\":[]}");
    }
}




static NSPanel *overlayPanel;
static uint64_t overlayGeneration = 0;
static NSArray<NSDictionary *> *walkthroughSteps;
static NSUInteger walkthroughIndex = 0;
static NSRect activeGlobalTarget = {{0, 0}, {0, 0}};
static NSPoint lastPointerGlobal = {0, 0};
static BOOL hasLastPointer = NO;
static id clickAdvanceMonitor = nil;
static NSDictionary *overlayOwner;
static NSString *overlaySettings;
static NSTimer *ownerTimer;

static void remove_click_monitor(void) {
    if (clickAdvanceMonitor) {
        [NSEvent removeMonitor:clickAdvanceMonitor];
        clickAdvanceMonitor = nil;
    }
}

static void clear_overlay_main(void) {
    overlayGeneration++;
    remove_click_monitor();
    walkthroughSteps = nil;
    overlayOwner=nil;overlaySettings=nil;
    [ownerTimer invalidate]; ownerTimer=nil;
    walkthroughIndex = 0;
    activeGlobalTarget = NSZeroRect;
    hasLastPointer = NO;
    if (overlayPanel) {
        [overlayPanel orderOut:nil];
        overlayPanel.contentView = [[NSView alloc] initWithFrame:NSZeroRect];
    }
}

void beside_overlay_clear(void) {
    dispatch_async(dispatch_get_main_queue(), ^{
        clear_overlay_main();
    });
}

static void render_step_main(
    double qx,
    double qy,
    double qw,
    double qh,
    NSString *label,
    NSString *note,
    int durationMs,
    NSUInteger stepNum,
    NSUInteger totalSteps
);

static void install_click_monitor_main(uint64_t gen) {
    remove_click_monitor();
    clickAdvanceMonitor = [NSEvent addGlobalMonitorForEventsMatchingMask:NSEventMaskLeftMouseDown
                                                                 handler:^(NSEvent *event) {
        CGPoint clicked = CGEventGetLocation(event.CGEvent);
        dispatch_async(dispatch_get_main_queue(), ^{
            if (overlayGeneration != gen || NSIsEmptyRect(activeGlobalTarget)) return;
            if (!matches_owner(overlayOwner) || !beside_overlay_allowed(overlaySettings.UTF8String,overlayOwner[@"bundle"])) {clear_overlay_main(); return;}
            NSPoint clickLoc = NSMakePoint(clicked.x,NSScreen.screens.firstObject.frame.size.height-clicked.y);
            NSRect hitZone = activeGlobalTarget;
            if (!NSPointInRect(clickLoc, hitZone)) return;
            if (walkthroughSteps && walkthroughIndex + 1 < walkthroughSteps.count) {
                remove_click_monitor();
                NSDictionary *next=walkthroughSteps[walkthroughIndex+1];
                NSString *settings=[overlaySettings copy];
                [overlayPanel orderOut:nil];
                // Navigation can replace a control without moving the window. Resolve the next
                // visible label again after the app has handled the click.
                dispatch_after(dispatch_time(DISPATCH_TIME_NOW,180*NSEC_PER_MSEC),
                    dispatch_get_global_queue(QOS_CLASS_USER_INITIATED,0), ^{
                    char *raw=beside_capture_front_elements(settings.UTF8String);
                    NSData *data=raw ? [NSData dataWithBytes:raw length:strlen(raw)] : nil;
                    free(raw);
                    NSDictionary *snapshot=data ? [NSJSONSerialization JSONObjectWithData:data options:0 error:nil] : nil;
                    dispatch_async(dispatch_get_main_queue(), ^{
                        if (overlayGeneration!=gen) return;
                        if (!matches_owner(overlayOwner) || ![snapshot[@"wid"] isEqual:overlayOwner[@"wid"]] ||
                            ![snapshot[@"pid"] isEqual:overlayOwner[@"pid"]]) {clear_overlay_main();return;}
                        NSArray *matches=[snapshot[@"elements"] filteredArrayUsingPredicate:
                            [NSPredicate predicateWithBlock:^BOOL(NSDictionary *el,NSDictionary *bindings){
                                (void)bindings; return [el[@"label"] isEqual:next[@"label"]];
                            }]];
                        if (matches.count!=1){clear_overlay_main();return;}
                        NSDictionary *item=matches.firstObject; walkthroughIndex++;
                        render_step_main([item[@"x"] doubleValue],[item[@"y"] doubleValue],
                           [item[@"w"] doubleValue],[item[@"h"] doubleValue],item[@"label"],next[@"note"],
                           15000,walkthroughIndex+1,walkthroughSteps.count);
                    });
                });
            } else {
                clear_overlay_main();
            }
        });
    }];
}

static void render_step_main(
    double qx,
    double qy,
    double qw,
    double qh,
    NSString *label,
    NSString *note,
    int durationMs,
    NSUInteger stepNum,
    NSUInteger totalSteps
) {
    if (!matches_owner(overlayOwner) || !beside_overlay_allowed(overlaySettings.UTF8String,overlayOwner[@"bundle"])) {clear_overlay_main(); return;}
    CGRect safe=[pick_target_window()[@"frame"] rectValue];
    if (!CGRectContainsRect(safe,CGRectMake(qx,qy,qw,qh))) {clear_overlay_main(); return;}
    overlayGeneration++;
    uint64_t gen = overlayGeneration;
    int clampedMs = durationMs > 500 ? MIN(durationMs, 30000) : 12000;

    NSScreen *primary = NSScreen.screens.firstObject ?: NSScreen.mainScreen;
    if (!primary) return;
    CGFloat primaryHeight = primary.frame.size.height;

    // Convert Quartz top-left global coordinates to AppKit bottom-left coordinates.
    CGFloat ax = qx;
    CGFloat ay = primaryHeight - (qy + qh);
    NSRect globalTarget = NSMakeRect(ax, ay, MAX(qw, 20), MAX(qh, 18));
    NSPoint center = NSMakePoint(NSMidX(globalTarget), NSMidY(globalTarget));
    activeGlobalTarget = globalTarget;

    NSScreen *screen = primary;
    for (NSScreen *s in NSScreen.screens) {
        if (NSPointInRect(center, s.frame)) {
            screen = s;
            break;
        }
    }

    if (!overlayPanel) {
        overlayPanel = [[NSPanel alloc]
            initWithContentRect:screen.frame
                      styleMask:NSWindowStyleMaskBorderless | NSWindowStyleMaskNonactivatingPanel
                        backing:NSBackingStoreBuffered
                          defer:NO];
        overlayPanel.opaque = NO;
        overlayPanel.backgroundColor = NSColor.clearColor;
        overlayPanel.hasShadow = NO;
        overlayPanel.ignoresMouseEvents = YES;
        overlayPanel.level = NSScreenSaverWindowLevel;
        overlayPanel.collectionBehavior =
            NSWindowCollectionBehaviorCanJoinAllSpaces |
            NSWindowCollectionBehaviorFullScreenAuxiliary |
            NSWindowCollectionBehaviorStationary |
            NSWindowCollectionBehaviorTransient;
    }

    [overlayPanel setFrame:screen.frame display:NO];
    NSView *canvas = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, screen.frame.size.width, screen.frame.size.height)];
    canvas.wantsLayer = YES;
    overlayPanel.contentView = canvas;

    NSRect local = NSMakeRect(
        globalTarget.origin.x - screen.frame.origin.x,
        globalTarget.origin.y - screen.frame.origin.y,
        globalTarget.size.width,
        globalTarget.size.height
    );
    NSRect padded = NSInsetRect(local, -7, -6);
    CGFloat radius = MIN(11, MIN(padded.size.width, padded.size.height) * 0.4);

    // Outer soft luminous halo
    CAShapeLayer *halo = [CAShapeLayer layer];
    NSRect haloRect = NSInsetRect(padded, -4, -4);
    CGPathRef haloPath = CGPathCreateWithRoundedRect(haloRect, radius + 3, radius + 3, NULL);
    halo.path = haloPath;
    halo.fillColor = NSColor.clearColor.CGColor;
    halo.strokeColor = [NSColor colorWithRed:0.43 green:0.16 blue:0.29 alpha:0.35].CGColor;
    halo.lineWidth = 4.0;
    [canvas.layer addSublayer:halo];
    CGPathRelease(haloPath);

    // Crisp focus ring around the target UI element
    CAShapeLayer *ring = [CAShapeLayer layer];
    CGPathRef ringPath = CGPathCreateWithRoundedRect(padded, radius, radius, NULL);
    ring.path = ringPath;
    ring.fillColor = [NSColor colorWithRed:0.43 green:0.16 blue:0.29 alpha:0.14].CGColor;
    ring.strokeColor = [NSColor colorWithRed:0.62 green:0.81 blue:0.80 alpha:0.98].CGColor;
    ring.lineWidth = 2.5;
    ring.shadowColor = [NSColor colorWithRed:0.17 green:0.27 blue:0.44 alpha:0.65].CGColor;
    ring.shadowRadius = 10;
    ring.shadowOpacity = 0.85;
    ring.shadowOffset = CGSizeZero;
    [canvas.layer addSublayer:ring];
    CGPathRelease(ringPath);

    // Curve toward the control while keeping the target visible.
    NSPoint originGlobal = hasLastPointer ? lastPointerGlobal : NSEvent.mouseLocation;
    NSPoint startPt = NSMakePoint(
        MAX(20, MIN(screen.frame.size.width - 20, originGlobal.x - screen.frame.origin.x)),
        MAX(20, MIN(screen.frame.size.height - 20, originGlobal.y - screen.frame.origin.y))
    );
    NSPoint endPt = NSMakePoint(NSMidX(padded), NSMidY(padded));
    lastPointerGlobal = center;
    hasLastPointer = YES;

    CAShapeLayer *dart = [CAShapeLayer layer];
    CGMutablePathRef dartPath = CGPathCreateMutable();
    CGPathMoveToPoint(dartPath, NULL, 0, 0);
    CGPathAddLineToPoint(dartPath, NULL, -14, -7);
    CGPathAddLineToPoint(dartPath, NULL, -10, 0);
    CGPathAddLineToPoint(dartPath, NULL, -14, 7);
    CGPathCloseSubpath(dartPath);
    dart.path = dartPath;
    CGPathRelease(dartPath);
    dart.fillColor = [NSColor colorWithRed:0.62 green:0.81 blue:0.80 alpha:0.98].CGColor;
    dart.strokeColor = [NSColor colorWithRed:0.15 green:0.21 blue:0.21 alpha:0.95].CGColor;
    dart.lineWidth = 1.5;
    dart.shadowColor = NSColor.blackColor.CGColor;
    dart.shadowOpacity = 0.45;
    dart.shadowRadius = 6;
    dart.position = NSMakePoint(NSMinX(padded) - 6, NSMidY(padded));
    [canvas.layer addSublayer:dart];

    if (!NSWorkspace.sharedWorkspace.accessibilityDisplayShouldReduceMotion) {
        CABasicAnimation *pulse = [CABasicAnimation animationWithKeyPath:@"opacity"];
        pulse.fromValue = @1.0;
        pulse.toValue = @0.35;
        pulse.duration = 0.85;
        pulse.autoreverses = YES;
        pulse.repeatCount = HUGE_VALF;
        [halo addAnimation:pulse forKey:@"haloPulse"];

        CGFloat dist = hypot(endPt.x - startPt.x, endPt.y - startPt.y);
        if (dist > 32.0) {
            NSPoint ctrl = NSMakePoint(
                (startPt.x + endPt.x) * 0.5,
                MAX(startPt.y, endPt.y) + MIN(120.0, dist * 0.28)
            );
            CGMutablePathRef arc = CGPathCreateMutable();
            CGPathMoveToPoint(arc, NULL, startPt.x, startPt.y);
            CGPathAddQuadCurveToPoint(arc, NULL, ctrl.x, ctrl.y, NSMinX(padded) - 6, NSMidY(padded));

            CAKeyframeAnimation *fly = [CAKeyframeAnimation animationWithKeyPath:@"position"];
            fly.path = arc;
            fly.duration = 0.46;
            fly.timingFunction = [CAMediaTimingFunction functionWithName:kCAMediaTimingFunctionEaseOut];
            [dart addAnimation:fly forKey:@"flyArc"];
            CGPathRelease(arc);
        }
    }

    // Sleek callout pill (shows "Step X of N · ..." during multi-step walkthroughs)
    NSString *baseCaption = note.length
        ? (label.length && ![note.lowercaseString containsString:label.lowercaseString]
            ? [NSString stringWithFormat:@"%@ · %@", label, note]
            : note)
        : label;
    NSString *caption = totalSteps > 1
        ? [NSString stringWithFormat:@"Step %lu of %lu · %@", (unsigned long)stepNum, (unsigned long)totalSteps, baseCaption.length ? baseCaption : @"Click here"]
        : baseCaption;

    if (caption.length) {
        NSTextField *text = [NSTextField labelWithString:caption];
        text.font = [NSFont systemFontOfSize:12.5 weight:NSFontWeightMedium];
        text.textColor = [NSColor colorWithWhite:0.98 alpha:1.0];
        text.lineBreakMode = NSLineBreakByTruncatingTail;
        [text sizeToFit];

        CGFloat pillW = MIN(360, text.frame.size.width + 34);
        CGFloat pillH = 30;
        CGFloat px, py;
        if (NSMaxX(padded) + 12 + pillW < screen.frame.size.width - 16 && NSMinX(padded) < screen.frame.size.width * 0.35) {
            px = NSMaxX(padded) + 12;
            py = NSMidY(padded) - pillH / 2.0;
        } else {
            px = NSMinX(padded);
            py = NSMaxY(padded) + 10;
            if (py + pillH > screen.frame.size.height - 16) {
                py = MAX(16, NSMinY(padded) - pillH - 10);
            }
        }
        px = MAX(14, MIN(px, screen.frame.size.width - pillW - 14));
        py = MAX(14, MIN(py, screen.frame.size.height - pillH - 14));

        NSView *pill = [[NSView alloc] initWithFrame:NSMakeRect(px, py, pillW, pillH)];
        pill.wantsLayer = YES;
        pill.layer.backgroundColor = [NSColor colorWithRed:0.15 green:0.21 blue:0.21 alpha:0.95].CGColor;
        pill.layer.cornerRadius = 15;
        pill.layer.borderWidth = 1;
        pill.layer.borderColor = [NSColor colorWithRed:0.62 green:0.81 blue:0.80 alpha:0.45].CGColor;
        pill.layer.shadowColor = NSColor.blackColor.CGColor;
        pill.layer.shadowOpacity = 0.4;
        pill.layer.shadowRadius = 12;
        pill.layer.shadowOffset = CGSizeMake(0, -4);

        NSView *dot = [[NSView alloc] initWithFrame:NSMakeRect(10, 11, 8, 8)];
        dot.wantsLayer = YES;
        dot.layer.cornerRadius = 4;
        dot.layer.backgroundColor = [NSColor colorWithRed:0.62 green:0.81 blue:0.80 alpha:1.0].CGColor;
        [pill addSubview:dot];

        text.frame = NSMakeRect(24, 6, pillW - 34, 18);
        [pill addSubview:text];
        [canvas addSubview:pill];
    }

    [overlayPanel orderFrontRegardless];
    install_click_monitor_main(gen);

    dispatch_after(
        dispatch_time(DISPATCH_TIME_NOW, (int64_t)clampedMs * NSEC_PER_MSEC),
        dispatch_get_main_queue(),
        ^{
            if (overlayGeneration == gen) {
                clear_overlay_main();
            }
        }
    );
}

void beside_overlay_highlight_steps(const char *stepsJson) {
    if (!stepsJson) return;
    NSData *data = [NSData dataWithBytes:stepsJson length:strlen(stepsJson)];
    dispatch_async(dispatch_get_main_queue(), ^{
        clear_overlay_main();
        NSDictionary *request=[NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
        if (![request isKindOfClass:NSDictionary.class] || !matches_owner(request[@"owner"])) return;
        NSArray *parsed=request[@"steps"];
        if (![parsed isKindOfClass:NSArray.class] || parsed.count == 0 || parsed.count > 8) return;
        overlayOwner=request[@"owner"];
        overlaySettings=[[NSString alloc] initWithData:[NSJSONSerialization dataWithJSONObject:request[@"config"] options:0 error:nil] encoding:NSUTF8StringEncoding];
        ownerTimer=[NSTimer scheduledTimerWithTimeInterval:0.20 repeats:YES block:^(NSTimer *timer) {
            (void)timer;
            if (!matches_owner(overlayOwner) || !beside_overlay_allowed(overlaySettings.UTF8String,overlayOwner[@"bundle"])) clear_overlay_main();
        }];
        walkthroughSteps = parsed;
        walkthroughIndex = 0;
        NSDictionary *first = parsed[0];
        if (![first isKindOfClass:NSDictionary.class]) return;
        render_step_main(
            [first[@"x"] doubleValue],
            [first[@"y"] doubleValue],
            MAX(18.0, [first[@"w"] doubleValue]),
            MAX(18.0, [first[@"h"] doubleValue]),
            [first[@"label"] isKindOfClass:NSString.class] ? first[@"label"] : @"",
            [first[@"note"] isKindOfClass:NSString.class] ? first[@"note"] : @"",
            15000,
            1,
            parsed.count
        );
    });
}
