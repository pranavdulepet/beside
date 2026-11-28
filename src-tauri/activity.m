#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <Carbon/Carbon.h>
#import <CommonCrypto/CommonDigest.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import <ServiceManagement/ServiceManagement.h>
#import <Vision/Vision.h>

extern NSDictionary *besideVisualBagLayout(CGImageRef image, CGPoint normalizedCursor);

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
// Keep the user's opted-in observation responsive while they are working in
// another app. This activity allows both the display and the system to sleep.
void sumi_observing_window(int observing) {
    static id activity;
    BOOL needed=observing && sumi_activity_awake() && availableSession();
    if (needed && !activity)
        activity=[NSProcessInfo.processInfo beginActivityWithOptions:NSActivityUserInitiatedAllowingIdleSystemSleep reason:@"Remembering visible work on this Mac"];
    else if (!needed && activity) {
        [NSProcessInfo.processInfo endActivity:activity];
        activity=nil;
    }
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
static NSString *comparableNoteTitle(NSString *value) {
    NSString *title = value.precomposedStringWithCanonicalMapping;
    title = [title stringByReplacingOccurrencesOfString:@"[‘’]" withString:@"'" options:NSRegularExpressionSearch range:NSMakeRange(0,title.length)];
    return [title stringByReplacingOccurrencesOfString:@"[“”]" withString:@"\"" options:NSRegularExpressionSearch range:NSMakeRange(0,title.length)];
}
static NSArray<NSString *> *notePassage(NSArray<VNRecognizedTextObservation *> *rows,
                                         NSString *windowTitle) {
    VNRecognizedTextObservation *heading = nil;
    BOOL namedWindow = windowTitle.length && ![windowTitle isEqualToString:@"Notes"];
    NSString *wanted = comparableNoteTitle(windowTitle);
    double best = -1;
    for (VNRecognizedTextObservation *row in rows) {
        VNRecognizedText *candidate = [row topCandidates:1].firstObject;
        NSString *text = candidate.string;
        CGRect box = row.boundingBox;
        if (candidate.confidence < 0.55 || !text.length || CGRectGetMidY(box) < 0.40 ||
            CGRectGetMidY(box) > 0.97)
            continue;
        BOOL matches = wanted.length && [wanted caseInsensitiveCompare:comparableNoteTitle(text)] == NSOrderedSame;
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
    NSString *title = namedWindow ? windowTitle : [heading topCandidates:1].firstObject.string;
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

static BOOL fareWindow(NSString *title) {
    return [title rangeOfString:@"booking|reservation|flight confirmation|flight itinerary|manage trip|check-in|fare types|fare bundles|fare options|bags\\s*[|·–-]\\s*ryanair"
                        options:NSRegularExpressionSearch|NSCaseInsensitiveSearch].location != NSNotFound;
}

// The airline serves a table and a card layout. In both, the smallest shared
// accessibility ancestor must contain one counter, one price and its inclusion
// notice. An adjacent cabin upgrade or 23 kg selector must never supply a price.
static NSString *bagReceipt(NSArray<NSString *> *parts) {
    NSString *raw = [parts componentsJoinedByString:@" "];
    NSArray *words = [raw componentsSeparatedByCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
    NSString *line = [[words filteredArrayUsingPredicate:[NSPredicate predicateWithFormat:@"length > 0"]] componentsJoinedByString:@" "];
    NSString *pattern = @"^[1-3] +[£€$] *[0-9]{1,4}(?: *[.,] *[0-9]{2})? +1 bag included (?:with|in) [A-Za-z][A-Za-z -]{0,24}$";
    return [line rangeOfString:pattern options:NSRegularExpressionSearch|NSCaseInsensitiveSearch].location != NSNotFound ? line : nil;
}

static void addBagDestination(NSString *route, NSMutableSet *destinations) {
    NSString *pattern = @"^[\\p{Lu}][\\p{L}\\p{M}.'’(), -]{0,60} to [\\p{Lu}][\\p{L}\\p{M}.'’(), -]{0,60}$";
    if (route.length < 140 && [route rangeOfString:pattern options:NSRegularExpressionSearch].location != NSNotFound)
        [destinations addObject:[[route componentsSeparatedByString:@" to "] lastObject]];
}

static void findBagOffer(id node, CGRect viewport, int depth, int *budget, NSMutableSet *visited,
                         NSMutableArray *texts, NSMutableArray *offers, NSMutableSet *destinations) {
    if (!node || [visited containsObject:node] || *budget <= 0 || depth > 48) return;
    [visited addObject:node];
    (*budget)--;
    AXUIElementRef el = (__bridge AXUIElementRef)node;
    // One round trip per node matters on a cross-process browser tree.
    NSArray *keys = @[(__bridge id)kAXRoleAttribute, (__bridge id)kAXTitleAttribute,
                      (__bridge id)kAXDescriptionAttribute, (__bridge id)kAXValueAttribute, (__bridge id)kAXChildrenAttribute];
    CFArrayRef raw = NULL;
    if (AXUIElementCopyMultipleAttributeValues(el, (__bridge CFArrayRef)keys, 0, &raw) != kAXErrorSuccess || !raw) return;
    NSArray *values = CFBridgingRelease(raw);
    if (values.count != keys.count) return;
    NSString *role = clean(values[0]);
    if ((depth && [role isEqual:@"AXWebArea"]) || [role containsString:@"TextField"] ||
        [role isEqual:@"AXTextArea"] || [role isEqual:@"AXColumn"] || [role isEqual:@"AXLink"]) return;
    BOOL isText = [role isEqual:@"AXStaticText"];
    NSString *label = isText ? clean(values[3]) : clean(values[1]);
    if (!label.length) label = clean(values[2]);
    if (!label.length) label = clean(isText ? values[1] : values[3]);
    label = [label stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
    if (isText || [role isEqual:@"AXHeading"] || [role isEqual:@"AXCell"]) {
        // A general heading such as "Equipment Need to bring sports ...?" is
        // not an itinerary. Read named endpoints from the page's own route.
        addBagDestination(label,destinations);
    }
    if (isText) {
        if (label.length && CGRectIntersectsRect(viewport, bounds(el))) [texts addObject:label];
        return;
    }
    NSUInteger firstText = texts.count, firstOffer = offers.count;
    if (([role isEqual:@"AXButton"] || [role isEqual:@"AXIncrementArrow"]) &&
        [label rangeOfString:@"^Add [1-9][0-9]?\\s*kg Check-in Bag$" options:NSRegularExpressionSearch|NSCaseInsensitiveSearch].location != NSNotFound &&
        CGRectIntersectsRect(viewport, bounds(el))) {
        [offers addObject:[@{@"control":label} mutableCopy]];
    }
    // Chrome on macOS exposes an actionable AXIncrementor and hides its
    // underlying plus/minus HTML buttons. The numeric value is the live
    // quantity. Keep only the bag name, never the passenger label after it.
    if ([role isEqual:@"AXIncrementor"] && (![values[4] isKindOfClass:NSArray.class] || ![values[4] count]) &&
        CGRectIntersectsRect(viewport, bounds(el))) {
        NSRange bag = [label rangeOfString:@"^[1-9][0-9]?\\s*kg Check-in Bag(?= |$)"
                                options:NSRegularExpressionSearch|NSCaseInsensitiveSearch];
        NSString *quantity = [values[3] isKindOfClass:NSNumber.class] ? [values[3] stringValue] : clean(values[3]);
        if (bag.location != NSNotFound && [@[@"1",@"2",@"3"] containsObject:quantity]) {
            CFArrayRef actions = NULL;
            if (AXUIElementCopyActionNames(el, &actions) == kAXErrorSuccess && actions) {
                NSArray *available = CFBridgingRelease(actions);
                if ([available containsObject:(__bridge NSString *)kAXIncrementAction]) {
                    NSString *name = [@"Add " stringByAppendingString:[label substringWithRange:bag]];
                    [offers addObject:[@{@"control":name} mutableCopy]];
                    [texts addObject:quantity];
                }
            }
        }
    }
    if ([values[4] isKindOfClass:NSArray.class]) {
        for (id child in values[4]) {
            if (*budget <= 0) break;
            findBagOffer(child,viewport,depth+1,budget,visited,texts,offers,destinations);
        }
    }
    // In the card layout "London Stansted to " and "Athens" are two
    // adjacent text children of the same route label.
    NSUInteger textCount = texts.count - firstText;
    if (textCount >= 2 && textCount <= 3) {
        for (NSUInteger j=firstText+1; j<texts.count; j++) {
            if ([texts[j-1] hasSuffix:@" to"])
                addBagDestination([NSString stringWithFormat:@"%@ %@",texts[j-1],texts[j]],destinations);
        }
    }
    if (offers.count != firstOffer + 1 || offers[firstOffer][@"line"]) return;
    NSUInteger count = texts.count - firstText;
    if (!count || count > 14) return;
    NSString *receipt = bagReceipt([texts subarrayWithRange:NSMakeRange(firstText,count)]);
    if (receipt) offers[firstOffer][@"line"] = receipt;
}

static NSString *checkedBagPage(id page, CGRect viewport) {
    AXUIElementRef el = (__bridge AXUIElementRef)page;
    id raw = attribute(el,kAXURLAttribute);
    NSURL *url = [raw isKindOfClass:NSString.class] ? [NSURL URLWithString:raw] : raw;
    if (![url isKindOfClass:NSURL.class] || ![url.scheme isEqual:@"https"] || ![url.host.lowercaseString isEqual:@"www.ryanair.com"] || ![url.path hasSuffix:@"/trip/flights/bags"]) return nil;
    // Discard cart identifiers and personal details. Rates for multiple
    // people or legs must not be attributed to a single bag.
    NSSet *keys = [NSSet setWithArray:@[@"tpDestinationIata",@"tpStartDate",@"tpEndDate",@"tpAdults",@"tpTeens",@"tpChildren",@"tpInfants"]];
    NSMutableDictionary *trip = [NSMutableDictionary dictionary];
    for (NSURLQueryItem *item in [NSURLComponents componentsWithURL:url resolvingAgainstBaseURL:NO].queryItems) {
        if (![keys containsObject:item.name]) continue;
        if (trip[item.name]) return nil;
        trip[item.name] = item.value ?: @"";
    }
    NSString *code = trip[@"tpDestinationIata"], *day = trip[@"tpStartDate"];
    if (!code || !day || [code rangeOfString:@"^[A-Z]{3}$" options:NSRegularExpressionSearch].location == NSNotFound ||
        [day rangeOfString:@"^[0-9]{4}-[0-9]{2}-[0-9]{2}$" options:NSRegularExpressionSearch].location == NSNotFound ||
        ![trip[@"tpEndDate"] isEqual:@""] || ![trip[@"tpAdults"] isEqual:@"1"] || ![trip[@"tpTeens"] isEqual:@"0"] ||
        ![trip[@"tpChildren"] isEqual:@"0"] || ![trip[@"tpInfants"] isEqual:@"0"]) return nil;
    NSMutableArray *offers = [NSMutableArray array], *texts = [NSMutableArray array];
    NSMutableSet *destinations = [NSMutableSet set], *visited = [NSMutableSet set];
    int budget = 1200;
    findBagOffer(page,viewport,0,&budget,visited,texts,offers,destinations);
    if (budget <= 0 || offers.count != 1 || destinations.count != 1 || !offers.firstObject[@"line"]) return nil;
    return [NSString stringWithFormat:@"Check-in bag selection\nDestination: %@\nOutbound: %@ %@\n%@\n%@",
            destinations.anyObject,code,day,offers.firstObject[@"control"],offers.firstObject[@"line"]];
}

// A browser can switch tabs while the accessibility tree is being read.
static NSString *currentBagOffer(id page, id focused, CGRect viewport, pid_t pid, CGWindowID wid) {
    id initialURL = attribute((__bridge AXUIElementRef)page,kAXURLAttribute);
    NSString *offer = checkedBagPage(page,viewport);
    if (!offer || !availableSession() || !stillFront(pid) || frontWindow(pid)!=wid) return nil;
    int budget=220;
    id activePage=webArea(focused,0,&budget);
    return [activePage isEqual:page] && [initialURL isEqual:attribute((__bridge AXUIElementRef)page,kAXURLAttribute)] ? offer : nil;
}

// Only inspect checkout semantics on a visible airline bag page. Other display
// samples keep their usual capture path, including their original pointer line.
static NSString *frontBagOffer(NSRunningApplication *app, NSString *title, CGWindowID wid, const CGPoint *point) {
    if ([title rangeOfString:@"bags\\s*[|·–-]\\s*ryanair" options:NSRegularExpressionSearch|NSCaseInsensitiveSearch].location == NSNotFound ||
        !wid || !AXIsProcessTrusted() || !availableSession() || !stillFront(app.processIdentifier)) return nil;
    id owner=CFBridgingRelease(AXUIElementCreateApplication(app.processIdentifier));
    AXUIElementSetMessagingTimeout((__bridge AXUIElementRef)owner,0.25);
    if ([app.bundleIdentifier isEqual:@"com.google.Chrome"])
        AXUIElementSetAttributeValue((__bridge AXUIElementRef)owner,CFSTR("AXManualAccessibility"),kCFBooleanTrue);
    id focused=attribute((__bridge AXUIElementRef)owner,kAXFocusedWindowAttribute);
    int budget=220;
    id page=focused ? webArea(focused,0,&budget) : nil;
    if (!page) return nil;
    CGRect viewport=CGRectIntersection(bounds((__bridge AXUIElementRef)focused),bounds((__bridge AXUIElementRef)page));
    if (CGRectIsEmpty(viewport) || (point && !CGRectContainsPoint(viewport,*point))) return nil;
    return currentBagOffer(page,focused,viewport,app.processIdentifier,wid);
}

// Read the public itinerary from this top-level HTTPS page and the offer only
// from the control under the pointer. Never cross into another offer or iframe.
static BOOL bagTextMatches(NSString *line, NSString *pattern) {
    return line.length && line.length<=165 && ![line containsString:@"@"] &&
        [line rangeOfString:pattern options:NSRegularExpressionSearch|NSCaseInsensitiveSearch].location!=NSNotFound;
}
static BOOL bagAction(id node, NSString **word, NSNumber **quantity) {
    AXUIElementRef el=(__bridge AXUIElementRef)node;
    NSString *role=clean(attribute(el,kAXRoleAttribute));
    BOOL stepper=[role isEqual:@"AXIncrementor"];
    if (!stepper && ![role isEqual:@"AXButton"] && ![role isEqual:@"AXIncrementArrow"]) return NO;
    NSString *label=clean(attribute(el,kAXTitleAttribute));
    if (!label.length) label=clean(attribute(el,kAXDescriptionAttribute));
    if (!bagTextMatches(label,@"^(?:\\+|add|increase|increase quantity|(?:add|increase|select) (?:a |an |one |another |extra |additional )?(?:(?:[1-9][0-9]?(?:\\.[0-9])?\\s*(?:kg|lb)s? )?(?:check[- ]in|checked|hold) )?(?:bag(?:gage)?|luggage)s?(?: quantity)?|(?:[1-9][0-9]?\\s*(?:kg|lb)s? )?(?:check[- ]in|checked|hold) bag(?:gage)?(?: quantity)?)$")) return NO;
    if(word)*word=label;
    if(stepper && quantity) {
        id n=attribute(el,kAXValueAttribute);
        NSString *v=[n isKindOfClass:NSNumber.class]?[n stringValue]:clean(n);
        if(bagTextMatches(v,@"^[0-9]$"))*quantity=@(v.integerValue);
    }
    return YES;
}
static void publicBagFields(id node, CGRect viewport, BOOL itinerary, NSSet *pointed, NSMutableOrderedSet *fields,
                            BOOL *foreign, int depth, int *budget) {
    if (--*budget<0 || depth>38 || fields.count>=(itinerary?48:32)) { *foreign=YES; return; }
    AXUIElementRef el=(__bridge AXUIElementRef)node;
    NSString *role=clean(attribute(el,kAXRoleAttribute));
    if ((depth && [role isEqual:@"AXWebArea"])||[role containsString:@"TextField"]||[role isEqual:@"AXTextArea"]||[role isEqual:@"AXComboBox"]||[role isEqual:@"AXLink"]||[role isEqual:@"AXColumn"]) return;
    CGRect rect=bounds(el);
    if (!CGRectIsEmpty(rect)&&!CGRectIntersectsRect(rect,viewport)) return;
    if (!itinerary && ![pointed containsObject:node] && bagAction(node,NULL,NULL)) { *foreign=YES; return; }
    if ([role isEqual:@"AXStaticText"]||[role isEqual:@"AXHeading"]) {
        NSString *v=clean(attribute(el,[role isEqual:@"AXStaticText"]?kAXValueAttribute:kAXTitleAttribute));
        if (!v.length) v=clean(attribute(el,kAXDescriptionAttribute));
        if (!CGRectIsEmpty(rect)) {
            for (NSString *raw in [v componentsSeparatedByCharactersInSet:NSCharacterSet.newlineCharacterSet]) {
                NSString *line=[raw stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceCharacterSet];
                NSString *scope=@"^(?:(?:departure(?: airport| date)?|departing|outbound(?: flight)?|from|origin|to|arrival airport|destination|return date|return flight|travel date):.{1,115}|(?:passengers?|travell?ers?):\\s*[0-9]+(?: (?:adults?|passengers?|travell?ers?))?|(?:one[- ]way(?: flight| journey)?|round[- ]trip|multi[- ]city)|[0-9]+ (?:adults?|passengers?|travell?ers?|children|infants?)|.{1,60}\\([A-Z]{3}\\)\\s*(?:→|->| to | – | — | - )\\s*.{1,60}\\([A-Z]{3}\\))$";
                NSString *offer=@"(?:\\b(?:check[- ]in|checked|hold|cabin|personal item|carry[- ]on|overweight|oversize|bags?|allowance|included|selected|quantity|fare|price|total|discount)\\b|[£€$]|\\b(?:USD|EUR|GBP|CAD|AUD|NZD|CHF|DKK|NOK|SEK|kg|lb)s?\\b)";
                if (bagTextMatches(line,itinerary?scope:offer)) [fields addObject:line];
            }
        }
        if ([role isEqual:@"AXStaticText"]) return;
    }
    for (id child in attribute(el,kAXChildrenAttribute)) {
        if (*foreign) return;
        publicBagFields(child,viewport,itinerary,pointed,fields,foreign,depth+1,budget);
    }
}
static NSDictionary *checkoutAtPointer(NSRunningApplication *app, CGWindowID wid, CGPoint point) {
    if (!wid||!AXIsProcessTrusted()||!availableSession()||!stillFront(app.processIdentifier)||frontWindow(app.processIdentifier)!=wid) return nil;
    id owner=CFBridgingRelease(AXUIElementCreateApplication(app.processIdentifier));
    AXUIElementSetMessagingTimeout((__bridge AXUIElementRef)owner,.22);
    AXUIElementRef element=NULL;
    if (AXUIElementCopyElementAtPosition((__bridge AXUIElementRef)owner,point.x,point.y,&element)!=kAXErrorSuccess||!element) return nil;
    id hit=CFBridgingRelease(element), page=nil;
    NSMutableArray *path=[NSMutableArray array];
    NSString *control=nil;
    NSNumber *quantity=nil;
    for (id current=hit;current && path.count<34;current=attribute((__bridge AXUIElementRef)current,kAXParentAttribute)) {
        if ([path containsObject:current]) return nil;
        if ([clean(attribute((__bridge AXUIElementRef)current,kAXRoleAttribute)) isEqual:@"AXWebArea"]) {page=current;break;}
        if (path.count<5) {
            NSString *label=nil;
            if (bagAction(current,&label,&quantity) && (!control||[label length]>control.length)) {
                if (![attribute((__bridge AXUIElementRef)current,kAXEnabledAttribute) boolValue]) return nil;
                control=label;
            }
        }
        [path addObject:current];
        if (path.count==5&&!control) return nil;
    }
    if (!page||!control) return nil;
    NSString *host=hostOf(page);
    if (!host.length || [host rangeOfString:@"^[a-z0-9.-]+\\.[a-z]{2,}$" options:NSRegularExpressionSearch].location==NSNotFound) return nil;
    id before=attribute((__bridge AXUIElementRef)page,kAXURLAttribute);
    NSURL *url=[before isKindOfClass:NSString.class] ? [NSURL URLWithString:before] : before;
    if (![url isKindOfClass:NSURL.class] || url.user.length || url.password.length || url.port ||
        ([host isEqual:@"www.ryanair.com"] && [url.path hasSuffix:@"/trip/flights/bags"])) return nil;
    id focused=attribute((__bridge AXUIElementRef)owner,kAXFocusedWindowAttribute);
    int check=220;
    if (!focused||![webArea(focused,0,&check) isEqual:page]) return nil;
    CGRect viewport=CGRectIntersection(bounds((__bridge AXUIElementRef)focused),bounds((__bridge AXUIElementRef)page));
    if (!CGRectContainsPoint(viewport,point)) return nil;
    NSSet *pointed=[NSSet setWithArray:path];NSMutableArray *groups=[NSMutableArray array];
    for (id n in path) {
        if (groups.count>=6) break;
        int budget=75;BOOL foreign=NO;NSMutableOrderedSet *parts=[NSMutableOrderedSet orderedSet];
        publicBagFields(n,viewport,NO,pointed,parts,&foreign,0,&budget);
        if (foreign) break;
        if (parts.count&&![groups containsObject:parts.array]) [groups addObject:parts.array];
    }
    BOOL hasBag=bagTextMatches(control,@"\\b(?:checked|check[- ]in|hold) bag");
    for(NSArray *group in groups)for(NSString *line in group)if(bagTextMatches(line,@"\\b(?:checked|check[- ]in|hold) bag"))hasBag=YES;
    if (!hasBag) return nil;
    int budget=650;BOOL incomplete=NO;NSMutableOrderedSet *trip=[NSMutableOrderedSet orderedSet];
    publicBagFields(page,viewport,YES,pointed,trip,&incomplete,0,&budget);
    check=220;
    if(!availableSession()||!stillFront(app.processIdentifier)||frontWindow(app.processIdentifier)!=wid||
       ![webArea(focused,0,&check) isEqual:page] ||
       ![before isEqual:attribute((__bridge AXUIElementRef)page,kAXURLAttribute)])return nil;
    return @{@"origin":[@"https://" stringByAppendingString:host],@"groups":groups,@"itinerary":incomplete?@[]:trip.array,@"control":control,@"quantity":quantity?:NSNull.null};
}

// Vision returns observations in reading blocks. On a fare table, preserve
// horizontal alignment so a checked-bag value cannot slide into the cabin row.
static NSArray *fareRows(NSArray<VNRecognizedTextObservation *> *observations) {
    NSArray *sorted = [observations sortedArrayUsingComparator:^NSComparisonResult(VNRecognizedTextObservation *a, VNRecognizedTextObservation *b) {
        return [@(CGRectGetMidY(b.boundingBox)) compare:@(CGRectGetMidY(a.boundingBox))];
    }];
    NSMutableArray *groups = [NSMutableArray array];
    for (VNRecognizedTextObservation *word in sorted) {
        NSMutableArray *row = groups.lastObject;
        VNRecognizedTextObservation *first = row.firstObject;
        if (!first || fabs(CGRectGetMidY(first.boundingBox)-CGRectGetMidY(word.boundingBox)) >
            0.5 * MIN(first.boundingBox.size.height,word.boundingBox.size.height)) {
            row=[NSMutableArray array]; [groups addObject:row];
        }
        [row addObject:word];
    }
    NSMutableArray *lines=[NSMutableArray array];
    for (NSMutableArray *row in groups) {
        [row sortUsingComparator:^NSComparisonResult(VNRecognizedTextObservation *a,VNRecognizedTextObservation *b){
            return [@(a.boundingBox.origin.x) compare:@(b.boundingBox.origin.x)];
        }];
        NSMutableArray *cells=[NSMutableArray array];
        for (VNRecognizedTextObservation *cell in row) {
            NSString *value=[cell topCandidates:1].firstObject.string;
            if(value.length) [cells addObject:value];
        }
        [lines addObject:[cells componentsJoinedByString:@"\t"]];
    }
    return lines;
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
        if (fareWindow(title) && !note) lines = [fareRows(ocr.results) mutableCopy];
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
// Core Graphics orders windows front to back across every display. A
// side-by-side window can come first even when it covers none of the note.
static CGRect unobscuredNoteFrame(NSArray<NSDictionary *> *windows, pid_t pid, CGWindowID wanted) {
    CGRect frame = CGRectZero;
    for (NSDictionary *w in windows.reverseObjectEnumerator) {
        if ([w[(__bridge NSString *)kCGWindowLayer] intValue] != 0) continue;
        NSNumber *alpha = w[(__bridge NSString *)kCGWindowAlpha];
        if (alpha && alpha.doubleValue <= 0) continue;
        NSDictionary *box = w[(__bridge NSString *)kCGWindowBounds];
        CGRect rect = CGRectZero;
        BOOL known = box && CGRectMakeWithDictionaryRepresentation((__bridge CFDictionaryRef)box, &rect);
        if ([w[(__bridge NSString *)kCGWindowOwnerPID] intValue] == pid &&
            [w[(__bridge NSString *)kCGWindowNumber] unsignedIntValue] == wanted) {
            if (!known || CGRectIsEmpty(rect)) return CGRectZero;
            frame = rect;
        } else if (!CGRectIsEmpty(frame) && (!known || !CGRectIsEmpty(CGRectIntersection(frame, rect)))) {
            // A window in front of the editor hides part of it. Do not read
            // behind it with a window-only screenshot or Accessibility.
            return CGRectZero;
        }
    }
    return frame;
}

static CGRect visibleNoteWindow(pid_t pid, CGWindowID wanted) {
    NSArray *windows = CFBridgingRelease(CGWindowListCopyWindowInfo(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements, kCGNullWindowID));
    return unobscuredNoteFrame(windows, pid, wanted);
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
        BOOL textFile = [app.bundleIdentifier isEqual:@"com.apple.TextEdit"];
        NSString *heading = textFile ? window.title.stringByDeletingPathExtension : window.title;
        NSDictionary *row = noteFrame(filter, config, source, heading ?: @"", preview && previous != nil);
        if (row && textFile) {
            NSMutableDictionary *passage=[row mutableCopy];
            passage[@"title"]=[NSString stringWithFormat:@"Visible text · %@ · TextEdit active",window.title];
            row=passage;
        }
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
BOOL beside_guidance_session(void) {
    return availableSession() && screenAccess(NO);
}
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
                if (!name.length || [name isEqual:@"Notes"])
                    return wid ? packed(@{@"source":@"display", @"title":@"Notes", @"window":@(wid)}) : NULL;
                return packed(@{@"source": @"display", @"title": [@"Notes · " stringByAppendingString:name], @"window":@(wid)});
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
        return packed(@{@"source": bundle, @"title": name, @"window":@(wid)});
    }
}
// The app-wide focused field can belong to the previous note. Search the
// actual focused window for an editor whose own visible text has this title.
static NSString *matchingEditor(id node, int depth, int *budget, NSString *(^read)(id)) {
    if (!node || --*budget<0 || depth>12) return nil;
    AXUIElementRef el=(__bridge AXUIElementRef)node;
    if ([attribute(el,kAXRoleAttribute) isEqual:@"AXTextArea"]) return read(node);
    for (id child in attribute(el,kAXChildrenAttribute)) {
        NSString *text=matchingEditor(child,depth+1,budget,read);
        if (text) return text;
    }
    return nil;
}
static NSString *visibleEditorText(id node, id window, NSString *name, BOOL notes, BOOL inWindow) {
    if (!node) return nil;
    AXUIElementRef el=(__bridge AXUIElementRef)node;
    if (![attribute(el,kAXRoleAttribute) isEqual:@"AXTextArea"]) return nil;
    id owner=attribute(el,kAXWindowAttribute);
    if (owner ? ![owner isEqual:window] : !inWindow) return nil;
    NSNumber *length=attribute(el,kAXNumberOfCharactersAttribute);
    id visible=attribute(el,kAXVisibleCharacterRangeAttribute);
    CFRange range={0,0};
    if (!length || length.integerValue<1 || length.integerValue>5400 || !visible ||
        CFGetTypeID((__bridge CFTypeRef)visible)!=AXValueGetTypeID() ||
        !AXValueGetValue((__bridge AXValueRef)visible,kAXValueCFRangeType,&range) ||
        range.location!=0 || range.length<length.integerValue) return nil;
    NSString *text=clean(attribute(el,kAXValueAttribute));
    if (text.length!=length.unsignedIntegerValue) return nil;
    NSString *heading=[[text componentsSeparatedByCharactersInSet:NSCharacterSet.newlineCharacterSet] firstObject];
    if (notes && (![name isEqual:heading] ||
        ![[text substringFromIndex:heading.length] stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet].length)) return nil;
    return text;
}
static NSDictionary *visibleEditorDocument(NSRunningApplication *app, NSString *source) {
    if (!AXIsProcessTrusted() || !availableSession() || !screenAccess(NO)) return nil;
    BOOL notes=[app.bundleIdentifier isEqual:@"com.apple.Notes"];
    if (!notes && ![app.bundleIdentifier isEqual:@"com.apple.TextEdit"]) return nil;
    pid_t pid=app.processIdentifier;
    CGWindowID wid=frontWindow(pid);
    if (!wid || !stillFront(pid) || CGRectIsEmpty(visibleNoteWindow(pid,wid))) return nil;
    id owner=CFBridgingRelease(AXUIElementCreateApplication(pid));
    AXUIElementRef ax=(__bridge AXUIElementRef)owner;
    AXUIElementSetMessagingTimeout(ax,0.25);
    id window=attribute(ax,kAXFocusedWindowAttribute);
    NSString *name=window ? clean(attribute((__bridge AXUIElementRef)window,kAXTitleAttribute)) : @"";
    if (!name.length || (notes && [name isEqual:@"Notes"]) || (!notes && [name rangeOfString:@"packing (list|checklist)" options:NSRegularExpressionSearch|NSCaseInsensitiveSearch].location==NSNotFound)) return nil;
    NSString *text=visibleEditorText(attribute(ax,kAXFocusedUIElementAttribute),window,name,notes,NO);
    if (!text && notes) {
        int budget=120;
        text=matchingEditor(window,0,&budget,^NSString *(id node) {
            return visibleEditorText(node,window,name,YES,YES);
        });
    }
    if (!text || !stillFront(pid) || frontWindow(pid)!=wid || CGRectIsEmpty(visibleNoteWindow(pid,wid)) ||
        ![window isEqual:attribute(ax,kAXFocusedWindowAttribute)] || ![name isEqual:attribute((__bridge AXUIElementRef)window,kAXTitleAttribute)]) return nil;
    NSString *title=notes ? [@"Notes · " stringByAppendingString:name] : [NSString stringWithFormat:@"TextEdit · %@ · TextEdit active",name];
    return @{@"source":source,@"title":title,@"body":text};
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
        if ([bundle isEqual:@"com.apple.TextEdit"] && (displays || [allowed containsObject:bundle])) {
            NSDictionary *list=visibleEditorDocument(app, displays ? @"display" : bundle);
            if (list) return packed(list);
            CGWindowID window=frontWindow(pid);
            NSArray *windows=CFBridgingRelease(CGWindowListCopyWindowInfo(kCGWindowListOptionIncludingWindow,window));
            NSString *name=clean(windows.firstObject[(__bridge NSString *)kCGWindowName]);
            if ([name rangeOfString:@"packing (list|checklist)" options:NSRegularExpressionSearch|NSCaseInsensitiveSearch].location!=NSNotFound)
                return sampleNote(app,displays ? @"display" : bundle,keepPreview);
        }
        if ([bundle isEqual:@"com.apple.Notes"] && (displays || [allowed containsObject:bundle])) {
            NSString *source=displays ? @"display" : bundle;
            NSDictionary *note=visibleEditorDocument(app,source);
            if (note) return packed(note);
            return sampleNote(app,source,keepPreview);
        }
        if (displays && browser(bundle) && screenAccess(NO)) {
            CGWindowID wid=frontWindow(pid);
            if (wid) {
                NSArray *rows=CFBridgingRelease(CGWindowListCopyWindowInfo(kCGWindowListOptionIncludingWindow,wid));
                NSString *title=clean(rows.firstObject[(__bridge NSString *)kCGWindowName]);
                NSString *offer=frontBagOffer(app,title,wid,NULL);
                if (offer) return packed(@{@"source":@"display",@"title":@"Check-in bags · Ryanair",@"body":offer});
            }
        }
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
            NSString *offer=currentBagOffer(page,window,viewport,pid,frontWindow(pid));
            if (offer) return packed(@{@"source":source,@"title":@"Check-in bags · Ryanair",@"body":offer});
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
extern void sumi_capture_service_origin(void);
extern void sumi_watch_service_origin(void);
@interface SumiServices : NSObject
@end
@implementation SumiServices
- (void)askSumi:(NSPasteboard *)pasteboard userData:(NSString *)data error:(NSString **)error {
    NSString *text = [pasteboard stringForType:NSPasteboardTypeString];
    if (!text.length || text.length > 12000) {
        if (error) *error = @"Select between 1 and 12,000 characters to use in Beside.";
        return;
    }
    if (receivedSelection) {
        sumi_capture_service_origin();
        receivedSelection(text.UTF8String);
    }
}
@end
void sumi_register_service(void (*callback)(const char *)) {
    receivedSelection = callback;
    static SumiServices *provider;
    if (!provider) {
        sumi_watch_service_origin();
        provider = [SumiServices new];
    }
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
                 @"bundle":app.bundleIdentifier, @"pid":@(pid), @"title":clean(row[(__bridge NSString *)kCGWindowName]), @"frame":[NSValue valueWithRect:frame]};
    }
    return nil;
}

// Use the author's selection only when the pointer is actually over it in
// a supported editor. A selection elsewhere in the document is unrelated.
static NSString *selectedPassage(NSDictionary *window, CGPoint point) {
    pid_t pid = [window[@"pid"] intValue];
    CGWindowID wid = [window[@"id"] unsignedIntValue];
    if (![@[@"com.apple.TextEdit", @"com.apple.Notes", @"com.apple.iWork.Pages"] containsObject:window[@"bundle"]]
        || !AXIsProcessTrusted() || !wid || !stillFront(pid) || frontWindow(pid) != wid) return nil;
    id app = CFBridgingRelease(AXUIElementCreateApplication(pid));
    AXUIElementRef owner = (__bridge AXUIElementRef)app, raw = NULL;
    AXUIElementSetMessagingTimeout(owner, 0.15);
    if (AXUIElementCopyElementAtPosition(owner, point.x, point.y, &raw) != kAXErrorSuccess) return nil;
    id element = CFBridgingRelease(raw);
    AXUIElementRef field = (__bridge AXUIElementRef)element;
    if (![attribute(field,kAXRoleAttribute) isEqual:(__bridge NSString *)kAXTextAreaRole]) return nil;
    id editor = attribute(field,kAXWindowAttribute) ?: attribute(field,kAXTopLevelUIElementAttribute);
    if (!editor || ![editor isEqual:attribute(owner,kAXFocusedWindowAttribute)]) return nil;
    id range = attribute(field,kAXSelectedTextRangeAttribute);
    CFRange selection = {0, 0};
    if (!range || CFGetTypeID((__bridge CFTypeRef)range) != AXValueGetTypeID()
        || !AXValueGetValue((__bridge AXValueRef)range,kAXValueCFRangeType,&selection)
        || selection.length < 1 || selection.length > 1200) return nil;
    CFTypeRef rawBounds = NULL;
    if (AXUIElementCopyParameterizedAttributeValue(field,kAXBoundsForRangeParameterizedAttribute,
            (__bridge CFTypeRef)range,&rawBounds) != kAXErrorSuccess) return nil;
    id box = CFBridgingRelease(rawBounds);
    CGRect rect = CGRectZero;
    if (!box || CFGetTypeID((__bridge CFTypeRef)box) != AXValueGetTypeID()
        || !AXValueGetValue((__bridge AXValueRef)box,kAXValueCGRectType,&rect)
        || !CGRectContainsPoint(rect,point)) return nil;
    NSString *text = clean(attribute(field,kAXSelectedTextAttribute));
    return text.length > 0 && text.length <= 1200 && stillFront(pid) && frontWindow(pid) == wid ? text : nil;
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
        NSString *title=clean(attribute((__bridge AXUIElementRef)page,kAXTitleAttribute));
        CGRect viewport=CGRectIntersection(bounds((__bridge AXUIElementRef)focused),bounds((__bridge AXUIElementRef)page));
        NSString *offer=currentBagOffer(page,focused,viewport,pid,wid);
        if (offer && [host isEqual:hostOf(page)])
            return @{@"source":source,@"app":window[@"app"],@"pageTitle":title,@"line":line,@"bookingTitle":@"Check-in bags · Ryanair",@"bookingBody":offer};
        NSDictionary *generic=checkoutAtPointer([NSRunningApplication runningApplicationWithProcessIdentifier:pid],wid,point);
        if (generic && [host isEqual:hostOf(page)]) return @{@"source":source,@"app":window[@"app"],@"pageTitle":title?:@"",@"line":line?:@"",@"bookingTitle":[@"Checked bags · " stringByAppendingString:host],@"checkoutEvidence":generic};
        return line.length && line.length<1800 ? @{@"source":source,@"app":window[@"app"],@"pageTitle":title ?: @"",@"line":line} : nil;
    }
    if (!displays && ![settings[@"sources"] containsObject:source]) return nil;
    if (!availableSession() || !screenAccess(YES)) return nil;
    NSString *checkout=nil;
    NSDictionary *genericCheckout=nil;
    if (browser(window[@"bundle"])) {
        NSRunningApplication *app=[NSRunningApplication runningApplicationWithProcessIdentifier:pid];
        checkout=frontBagOffer(app,clean(window[@"title"]),wid,&point);
        if (!checkout) genericCheckout=checkoutAtPointer(app,wid,point);
    }
    NSString *selection = selectedPassage(window, point);
    dispatch_semaphore_t done=dispatch_semaphore_create(0);
    __block NSDictionary *result=nil;
    [SCShareableContent getShareableContentExcludingDesktopWindows:YES onScreenWindowsOnly:YES completionHandler:^(SCShareableContent *content,NSError *error) {
        if (error) { dispatch_semaphore_signal(done); return; }
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
                    // Also produce a compact full-window screenshot for visual context
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
                    BOOL selected = selection && [selection isEqual:selectedPassage(window, point)];
                    result=@{
                        @"source":source,
                        @"app":window[@"app"],
                        @"bundle":window[@"bundle"],
                        @"line":selected ? selection : line,
                        @"lineKind":selected ? @"selection" : @"point",
                        @"nearby":nearby,
                        @"image":[jpeg base64EncodedStringWithOptions:0] ?: @"",
                        @"fullImage":fullB64, @"wid":@(wid), @"pid":@(pid),
                        @"cursor":@{@"x_pct":@(xPct), @"y_pct":@(yPct)},
                        @"windowFrame":@{@"x":@(round(frame.origin.x)), @"y":@(round(frame.origin.y)), @"w":@(round(frame.size.width)), @"h":@(round(frame.size.height))}
                    };
                    NSString *title=w.title ?: @"";
                    if ([NSProcessInfo.processInfo.environment[@"BESIDE_BAG_DIAGNOSTICS"] isEqual:@"1"] && browser(window[@"bundle"]))
                        fprintf(stderr,"[Beside baggage] screen width=%zu height=%zu fare_title=%d ax_offer=%d\n",CGImageGetWidth(image),CGImageGetHeight(image),fareWindow(title),checkout!=nil);
                    if ([window[@"bundle"] isEqual:@"com.apple.Notes"]) {
                        // Read the editor column from the same frame as the pointer.
                        // The small pointer crop can omit the end of a list or plan.
                        NSDictionary *note=visibleEditorDocument([NSRunningApplication runningApplicationWithProcessIdentifier:pid],source);
                        if (!note) note=recognize(image,source,title,NO,YES);
                        if ([note[@"body"] length]) {
                            NSMutableDictionary *withNote=[result mutableCopy];
                            withNote[@"document"]=note[@"body"];
                            result=withNote;
                        }
                    }
                    if (checkout && [checkout isEqual:frontBagOffer(
                        [NSRunningApplication runningApplicationWithProcessIdentifier:pid],title,wid,&point)]) {
                        NSMutableDictionary *withOffer=[result mutableCopy];
                        withOffer[@"bookingTitle"]=@"Check-in bags · Ryanair";
                        withOffer[@"bookingBody"]=checkout;
                        result=withOffer;
                    } else if (genericCheckout && [genericCheckout isEqual:checkoutAtPointer(
                        [NSRunningApplication runningApplicationWithProcessIdentifier:pid],wid,point)]) {
                        NSMutableDictionary *withOffer=[result mutableCopy];
                        withOffer[@"bookingTitle"]=[@"Checked bags · " stringByAppendingString:[NSURL URLWithString:genericCheckout[@"origin"]].host];
                        withOffer[@"checkoutEvidence"]=genericCheckout;
                        result=withOffer;
                    } else if (fareWindow(title)) {
                        BOOL airline = browser(window[@"bundle"]) &&
                            [title rangeOfString:@"bags\\s*[|·–-]\\s*ryanair" options:NSRegularExpressionSearch|NSCaseInsensitiveSearch].location!=NSNotFound;
                        NSDictionary *layout = airline ? besideVisualBagLayout(image,CGPointMake(local.x/frame.size.width,local.y/frame.size.height)) : nil;
                        NSDictionary *page = layout ? nil : recognize(image,source,title,NO,NO);
                        if (layout) {
                            NSMutableDictionary *withLayout=[result mutableCopy];
                            withLayout[@"bookingTitle"]=@"Check-in bags · Ryanair";
                            withLayout[@"bagLayout"]=layout;
                            withLayout[@"bookingControlsUnavailable"]=@(!AXIsProcessTrusted());
                            result=withLayout;
                        }
                        if (page[@"body"]) {
                            NSMutableDictionary *withPage=[result mutableCopy];
                            withPage[@"bookingTitle"]=title;
                            withPage[@"bookingBody"]=page[@"body"];
                            if (browser(window[@"bundle"]) && !AXIsProcessTrusted())
                                withPage[@"bookingControlsUnavailable"]=@YES;
                            result=withPage;
                        }
                    }
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
