// Run from the repository root on macOS:
// clang -fobjc-arc tests/notes-capture.m -framework AppKit -framework Vision -framework ScreenCaptureKit -framework Carbon -framework ServiceManagement -o /tmp/beside-capture-test && /tmp/beside-capture-test
#import <Foundation/Foundation.h>
#import <ApplicationServices/ApplicationServices.h>
// Exercise Apple's occasionally stale focused-element reference without
// opening a real window or asking for Accessibility permission.
static AXError fixtureAttribute(AXUIElementRef element, CFStringRef key, CFTypeRef *out) {
    if (CFGetTypeID(element)==CFDictionaryGetTypeID()) {
        id value=[(__bridge NSDictionary *)element objectForKey:(__bridge NSString *)key];
        if (!value) return kAXErrorNoValue;
        *out=CFBridgingRetain(value);
        return kAXErrorSuccess;
    }
    return AXUIElementCopyAttributeValue(element,key,out);
}
#define AXUIElementCopyAttributeValue fixtureAttribute
#import "../src-tauri/activity.m"
#undef AXUIElementCopyAttributeValue
#import "../src-tauri/bag_vision.m"

static void checkBookingAddress(void) {
    NSString *path = @"/gb/en/trip/flights/bags?tpAdults=1&tpTeens=0&tpChildren=0&tpInfants=0&tpEndDate=&tpStartDate=2026-12-02&tpDestinationlata=ATH&cartId=discard";
    for (NSString *origin in @[@"ryanair.com", @"https://www.ryanair.com", @"< > G ryanair.com", @"< > G= ryanair.com", @"< > @ = ryanair.com", @"-- ryanair.com", @"•= ryanair.com"]) {
        NSDictionary *trip = publicItinerary([origin stringByAppendingString:path]);
        NSCAssert([trip[@"tpDestinationIata"] isEqual:@"ATH"] && [trip[@"tpStartDate"] isEqual:@"2026-12-02"], @"Read the itinerary even when browser controls share the address line");
        NSCAssert(!trip[@"cartId"], @"Retain only public trip fields");
    }
    for (NSString *origin in @[@"ryanair.com.example.org", @"myryanair.com", @"https://example.org/ryanair.com", @"https://example.org/?url=https://www.ryanair.com", @"http://ryanair.com", @"< > Gryanair.com", @"< > @ = e.co/ ryanair.com", @"< > WRONG ryanair.com", @"look up ryanair.com", @"ryan\u0430ir.com"]) {
        NSCAssert(!publicItinerary([origin stringByAppendingString:path]), @"Require the visible airline origin");
    }
    NSString *url = [@"ryanair.com" stringByAppendingString:path];
    for (NSString *other in @[[url stringByReplacingOccurrencesOfString:@"/bags?" withString:@"/checkout?"],
                              [url stringByReplacingOccurrencesOfString:@"tpAdults=1" withString:@"tpAdults=2"],
                              [url stringByReplacingOccurrencesOfString:@"tpEndDate=&" withString:@"tpEndDate=2026-12-09&"],
                              [url stringByAppendingString:@"&tpDestinationIata=STN"]]) {
        NSCAssert(!publicItinerary(other), @"Require one unambiguous itinerary at the checked-bag step");
    }
}

static NSDictionary *readFixture(NSString *heading, NSString *body) {
    NSBitmapImageRep *bitmap = [[NSBitmapImageRep alloc] initWithBitmapDataPlanes:NULL
        pixelsWide:1100 pixelsHigh:750 bitsPerSample:8 samplesPerPixel:4 hasAlpha:YES isPlanar:NO
        colorSpaceName:NSCalibratedRGBColorSpace bytesPerRow:0 bitsPerPixel:0];
    [NSGraphicsContext saveGraphicsState];
    NSGraphicsContext.currentContext = [NSGraphicsContext graphicsContextWithBitmapImageRep:bitmap];
    [NSColor.whiteColor setFill];
    NSRectFill(NSMakeRect(0, 0, 1100, 750));
    NSDictionary *normal = @{NSFontAttributeName:[NSFont systemFontOfSize:20],
                             NSForegroundColorAttributeName:NSColor.blackColor};
    [@"Notes" drawAtPoint:NSMakePoint(30, 700) withAttributes:normal];
    [@"Other folders" drawAtPoint:NSMakePoint(30, 620) withAttributes:normal];
    if (heading) [heading drawAtPoint:NSMakePoint(350, 625) withAttributes:@{
        NSFontAttributeName:[NSFont boldSystemFontOfSize:34], NSForegroundColorAttributeName:NSColor.blackColor}];
    if (body) [body drawAtPoint:NSMakePoint(350, 565) withAttributes:normal];
    [NSGraphicsContext restoreGraphicsState];
    return recognize(bitmap.CGImage, @"display", @"Telescope visit", NO, YES);
}
static NSDictionary *windowRow(int pid, int wid, CGRect frame, double alpha) {
    return @{(__bridge NSString *)kCGWindowOwnerPID:@(pid), (__bridge NSString *)kCGWindowNumber:@(wid),
        (__bridge NSString *)kCGWindowLayer:@0, (__bridge NSString *)kCGWindowAlpha:@(alpha),
        (__bridge NSString *)kCGWindowBounds:CFBridgingRelease(CGRectCreateDictionaryRepresentation(frame))};
}
static NSDictionary *editorRow(NSString *text, NSString *owner, CFRange visible) {
    NSMutableDictionary *row=[@{@"AXRole":@"AXTextArea", @"AXValue":text, @"AXNumberOfCharacters":@(text.length),
        @"AXVisibleCharacterRange":CFBridgingRelease(AXValueCreate(kAXValueCFRangeType,&visible))} mutableCopy];
    if (owner) row[@"AXWindow"]=owner;
    return row;
}
int main(void) {
    @autoreleasepool {
        checkBookingAddress();
        CGRect frame=CGRectMake(140,50,640,480);
        NSDictionary *note=windowRow(3,7,frame,1);
        NSDictionary *side=windowRow(9,8,CGRectMake(CGRectGetMaxX(frame),50,420,480),1);
        NSDictionary *over=windowRow(9,8,CGRectMake(770,70,420,380),1);
        NSDictionary *invisible=windowRow(9,10,frame,0);
        NSDictionary *empty=windowRow(9,11,CGRectZero,1);
        NSCAssert(CGRectEqualToRect(unobscuredNoteFrame(@[side,empty,invisible,note,over],3,7),frame), @"A visible note can share a display with unrelated windows");
        NSCAssert(CGRectIsEmpty(unobscuredNoteFrame(@[side,over,note],3,7)), @"Reject even a partially covered note");
        NSCAssert(CGRectIsEmpty(unobscuredNoteFrame(@[note],4,7)), @"Never substitute another app's window");
        NSCAssert(CGRectIsEmpty(unobscuredNoteFrame(@[side],3,7)), @"Require the original window to remain visible");
        NSCAssert([comparableNoteTitle(@"This week’s dinners") isEqual:comparableNoteTitle(@"This week's dinners")], @"Vision may use a straight apostrophe for the exact visible Notes title");
        NSCAssert(![comparableNoteTitle(@"This week’s dinners") isEqual:comparableNoteTitle(@"Last week's dinners")], @"Similar titles still identify different notes");
        NSString *name=@"Public workshop";
        NSString *body=@"Public workshop\nBring one notebook.";
        CFRange fullyVisible=CFRangeMake(0,body.length);
        NSDictionary *other=editorRow(body,@"last window",fullyVisible), *current=editorRow(body,nil,fullyVisible);
        NSCAssert(!visibleEditorText(other,@"this window",name,YES,NO), @"The previous focused field belongs to a different window");
        NSDictionary *tree=@{@"AXRole":@"AXWindow",@"AXChildren":@[other,
            editorRow(body,nil,CFRangeMake(0,12)),editorRow(name,nil,CFRangeMake(0,name.length)),current]};
        int budget=32;
        NSString *read=matchingEditor(tree,0,&budget,^NSString *(id el) {return visibleEditorText(el,@"this window",name,YES,YES);});
        NSCAssert([read isEqual:body], @"Keep searching the verified window after stale, clipped or half-rendered editors");
        NSDictionary *blank = readFixture(nil, nil);
        NSDictionary *headingOnly = readFixture(@"Telescope visit", nil);
        NSDictionary *first = readFixture(@"Telescope visit", @"Bring two lenses on Saturday.");
        NSDictionary *changed = readFixture(@"Telescope visit", @"Bring two lenses on Sunday.");
        NSCAssert(!blank && !headingOnly, @"Partial frames must not become passages");
        NSCAssert([first[@"title"] isEqual:@"Notes · Telescope visit"], @"Expected the editor title");
        NSCAssert([first[@"body"] isEqual:@"Telescope visit\nBring two lenses on Saturday."], @"Expected only the title and its body");
        NSCAssert(!sameNoteRead(first, nil) && !sameNoteRead(first, changed), @"A changing frame must wait");
        NSCAssert(sameNoteRead(first, readFixture(@"Telescope visit", @"Bring two lenses on Saturday.")), @"Two stable reads should agree");
    }
    return 0;
}
