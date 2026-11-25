// Run from the repository root on macOS:
// clang -fobjc-arc tests/notes-capture.m -framework AppKit -framework Vision -framework ScreenCaptureKit -framework Carbon -framework ServiceManagement -o /tmp/sumi-notes-test && /tmp/sumi-notes-test
#import "../src-tauri/activity.m"

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
int main(void) {
    @autoreleasepool {
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
