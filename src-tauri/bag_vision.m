#import <Foundation/Foundation.h>
#import <CoreGraphics/CoreGraphics.h>
#import <Vision/Vision.h>

// Keep only public fare labels from this already authorized, foreground image.
// The input is the browser window Beside captured; it never queries the browser.
static BOOL matches(NSString *text, NSString *pattern) {
    return [text rangeOfString:pattern options:NSRegularExpressionSearch].location != NSNotFound;
}

static NSRange airlineOrigin(NSString *address) {
    return [address rangeOfString:@"(?:https://)?(?:www\\.)?ryanair\\.com/[a-z]{2}/[a-z]{2}/trip/flights/bags\\?" options:NSRegularExpressionSearch];
}

static NSDictionary *publicItinerary(NSString *address) {
    NSRange domain = airlineOrigin(address);
    if (domain.location == NSNotFound) return nil;
    // Vision sometimes joins the browser's Back, Forward, Reload and site
    // controls to the address. Only accept these glyphs before a separate URL.
    NSString *prefix = [[address substringToIndex:domain.location] stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceCharacterSet];
    if (prefix.length) {
        BOOL siteIcon = matches(prefix, @"^(?:co|oo|[•○◉ⓘ◎∞−–=≡-]{1,3})$");
        // The tiny Reload icon can resemble a letter or symbol. After the
        // paired arrows, permit a short cluster with at most one isolated
        // letter and no host/path delimiters, digits or words.
        BOOL navigation = matches(prefix, @"^[<‹←]\\s*[>›→](?=.{0,9}$)[^\\p{L}\\p{N}./:\\\\?#%]*[A-Z]?[^\\p{L}\\p{N}./:\\\\?#%]*$");
        if (![NSCharacterSet.whitespaceCharacterSet characterIsMember:[address characterAtIndex:domain.location-1]] || (!siteIcon && !navigation)) return nil;
    }
    NSString *query = [address substringFromIndex:NSMaxRange(domain)];
    NSSet *allowed = [NSSet setWithArray:@[@"tpAdults",@"tpTeens",@"tpChildren",@"tpInfants",@"tpEndDate",@"tpStartDate",@"tpDestinationIata"]];
    NSMutableDictionary *params = [NSMutableDictionary dictionary];
    for (NSString *item in [query componentsSeparatedByString:@"&"]) {
        NSRange eq = [item rangeOfString:@"="];
        if (eq.location == NSNotFound) continue;
        NSString *key = [item substringToIndex:eq.location];
        // The printed parameter name has a capital I. Vision can read it as l.
        if ([key isEqual:@"tpDestinationlata"]) key=@"tpDestinationIata";
        if (![allowed containsObject:key]) continue;
        if (params[key]) return nil;
        NSString *value = [[item substringFromIndex:NSMaxRange(eq)] stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceCharacterSet];
        if (value.length > 10 || !matches(value,@"^[A-Z0-9-]*$")) return nil;
        params[key] = value;
    }
    if (![params[@"tpAdults"] isEqual:@"1"] || ![params[@"tpTeens"] isEqual:@"0"] ||
        ![params[@"tpChildren"] isEqual:@"0"] || ![params[@"tpInfants"] isEqual:@"0"] ||
        ![params[@"tpEndDate"] isEqual:@""]) return nil;
    return params;
}

static NSDictionary *itineraryInAddressBar(VNRecognizedText *word) {
    NSString *address=word.string;
    NSDictionary *trip=publicItinerary(address);
    if (!trip) return nil;
    NSRange domain=airlineOrigin(address);
    VNRectangleObservation *url=[word boundingBoxForRange:NSMakeRange(domain.location,address.length-domain.location) error:nil];
    if (!url || CGRectGetMinX(url.boundingBox)>.28) return nil;
    NSString *prefix=[[address substringToIndex:domain.location] stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceCharacterSet];
    if (prefix.length) {
        NSRange range=[address rangeOfString:prefix options:NSLiteralSearch range:NSMakeRange(0,domain.location)];
        VNRectangleObservation *icons=[word boundingBoxForRange:range error:nil];
        CGFloat gap=CGRectGetMinX(url.boundingBox)-CGRectGetMaxX(icons.boundingBox);
        if (!icons || gap<=0 || gap>.065) return nil;
    }
    return trip;
}

static NSString *publicKind(NSString *text) {
    if (matches(text,@"(?i)^[1-9][0-9]?\\s*kg check[- ]in bag$")) return @"heading";
    if (matches(text,@"(?i)^1 bag included (with|in) [a-z][a-z -]{0,24}$")) return @"included";
    if (matches(text,@"(?i)^(?:add\\s+)?[£€$]\\s*[0-9]{1,4}(?:\\s*[.,]\\s*[0-9]{2})?$")) return @"price";
    if (matches(text,@"^[0-3]$")) return @"quantity";
    if (matches(text,@"^[^\\p{L}\\p{N}]{0,5}\\p{Lu}[\\p{L}\\p{M} .'’(),-]{0,55} to \\p{Lu}[\\p{L}\\p{M} .'’(),-]{0,60}$")) return @"route";
    return nil;
}

static NSArray<VNRecognizedTextObservation *> *readFareImage(CGImageRef image) {
    VNRecognizeTextRequest *read = [VNRecognizeTextRequest new];
    read.recognitionLevel = VNRequestTextRecognitionLevelAccurate;
    read.usesLanguageCorrection = NO;
    read.recognitionLanguages = @[@"en-US"];
    VNImageRequestHandler *handler = [[VNImageRequestHandler alloc] initWithCGImage:image options:@{}];
    return [handler performRequests:@[read] error:nil] ? read.results : nil;
}

static NSDictionary *label(NSString *kind, NSString *text, CGRect rect) {
    return @{@"kind":kind, @"text":text, @"x":@(CGRectGetMidX(rect)), @"y":@(CGRectGetMidY(rect)),
             @"w":@(rect.size.width), @"h":@(rect.size.height)};
}

static CGRect topLeftBox(VNRectangleObservation *word) {
    CGRect rect=word.boundingBox;
    rect.origin.y=1-CGRectGetMaxY(rect);
    return rect;
}

// At some zoom levels Vision groups the counter and price into one line, for
// example "1 ⊕ £41.49". Split at real character boxes and reject another amount.
static void addCounterLine(VNRecognizedText *word, NSMutableArray *labels) {
    NSString *text=word.string;
    NSRange price=[text rangeOfString:@"[£€$]\\s*[0-9]{1,4}(?:\\s*[.,]\\s*[0-9]{2})?$" options:NSRegularExpressionSearch];
    if (price.location==NSNotFound || !price.location) return;
    NSString *prefix=[text substringToIndex:price.location];
    // On a captured pointer the plus icon can read as one capital letter.
    // It must follow a separately printed quantity; never join two amounts.
    if (!matches(prefix,@"^[+⊕⊖•○◯©®$–−() -]*(?:[0-3][+⊕⊖•○◯©®$–−() -]*(?: [A-Z] *)?)?$") ||
        matches(prefix,@"[£€$]\\s*[0-9]")) return;
    VNRectangleObservation *priceBox=[word boundingBoxForRange:price error:nil];
    if (!priceBox) return;
    [labels addObject:label(@"price",[text substringWithRange:price],topLeftBox(priceBox))];
    NSRange count=[prefix rangeOfString:@"[0-3]" options:NSRegularExpressionSearch];
    if (count.location!=NSNotFound) {
        VNRectangleObservation *countBox=[word boundingBoxForRange:count error:nil];
        if (countBox) [labels addObject:label(@"quantity",[prefix substringWithRange:count],topLeftBox(countBox))];
    }
}

static CGImageRef enlargeCounter(CGImageRef image) {
    size_t width=3*CGImageGetWidth(image), height=3*CGImageGetHeight(image);
    CGColorSpaceRef space=CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    CGContextRef context=CGBitmapContextCreate(NULL,width,height,8,0,space,(CGBitmapInfo)kCGImageAlphaNoneSkipLast);
    CGColorSpaceRelease(space);
    if (!context) return NULL;
    CGContextSetInterpolationQuality(context,kCGInterpolationHigh);
    CGContextDrawImage(context,CGRectMake(0,0,width,height),image);
    CGImageRef scaled=CGBitmapContextCreateImage(context);
    CGContextRelease(context);
    return scaled;
}

static BOOL readQuantityStrip(CGImageRef image, CGRect region, CGFloat priceY, CGFloat priceHeight, BOOL magnify, NSMutableArray *labels) {
    size_t width=CGImageGetWidth(image), height=CGImageGetHeight(image);
    CGRect crop=CGRectIntegral(CGRectIntersection(CGRectMake(0,0,width,height),
        CGRectMake(region.origin.x*width,region.origin.y*height,region.size.width*width,region.size.height*height)));
    if (CGRectIsEmpty(crop)) return NO;
    CGImageRef strip=CGImageCreateWithImageInRect(image,crop);
    if (!strip) return NO;
    CGImageRef scaled=magnify ? enlargeCounter(strip) : NULL;
    BOOL found=NO;
    for (VNRecognizedTextObservation *row in readFareImage(scaled ?: strip)) {
        VNRecognizedText *word=[row topCandidates:1].firstObject;
        if (word.confidence<.80 || ![publicKind(word.string) isEqual:@"quantity"]) continue;
        CGRect r=row.boundingBox;
        CGRect normalized=CGRectMake((crop.origin.x+r.origin.x*crop.size.width)/width,
            (crop.origin.y+(1-CGRectGetMaxY(r))*crop.size.height)/height,
            r.size.width*crop.size.width/width,r.size.height*crop.size.height/height);
        // An isolated “1” from the inclusion notice below is not the counter.
        if (fabs(CGRectGetMidY(normalized)-priceY) > .65*MAX(normalized.size.height,priceHeight)) continue;
        [labels addObject:label(@"quantity",word.string,normalized)];
        found=YES;
    }
    if (scaled) CGImageRelease(scaled);
    CGImageRelease(strip);
    return found;
}

NSDictionary *besideVisualBagLayout(CGImageRef image, CGPoint cursor) {
    if (!image || !CGRectContainsPoint(CGRectMake(0,0,1,1),cursor)) return nil;
    BOOL trace=[NSProcessInfo.processInfo.environment[@"BESIDE_BAG_DIAGNOSTICS"] isEqual:@"1"];
    NSMutableArray *labels = [NSMutableArray array];
    NSDictionary *itinerary = nil;
    CGFloat tabY = -1, addressY = -1;
    for (VNRecognizedTextObservation *row in readFareImage(image)) {
        VNRecognizedText *candidate = [row topCandidates:1].firstObject;
        NSString *text = [candidate.string stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
        if (candidate.confidence < .80 || !text.length) continue;
        CGRect rect = row.boundingBox;
        rect.origin.y = 1-CGRectGetMaxY(rect);
        if (CGRectGetMidY(rect)<.15 && matches(text,@"(?i)\\bbags\\s*[|·–-]\\s*ryanair\\s*$"))
            tabY = CGRectGetMidY(rect);
        if (CGRectGetMidY(rect)<.20 && rect.origin.x<.25 && [text containsString:@"ryanair.com/"]) {
            NSDictionary *params = itineraryInAddressBar(candidate);
            if (params) {
                if (itinerary) return nil;
                itinerary=params;
                addressY=CGRectGetMidY(rect);
            }
        }
        NSString *kind = publicKind(text);
        if (kind) [labels addObject:label(kind,text,rect)];
        else addCounterLine(candidate,labels);
    }
    // A matching page title, tab and browser address bar establish the origin.
    // An address in page content and a browser with hidden chrome cannot qualify.
    BOOL verified=itinerary && tabY>=0 && addressY>tabY && addressY-tabY<=.085 && labels.count<=110;
    if (trace) fprintf(stderr,"[Beside baggage] chrome tab=%d address=%d verified=%d\n",tabY>=0,itinerary!=nil,verified);
    if (!verified) return nil;
    // Vision can miss the single digit inside a +/- control at full-window scale.
    // Re-read only the strip immediately left of the price beside the pointer.
    // Read the digit itself; "1 bag included" does not establish the current count.
    NSArray *initial = [labels copy];
    for (NSDictionary *price in initial) {
        if (![price[@"kind"] isEqual:@"price"] || [price[@"text"] rangeOfString:@"^Add" options:NSRegularExpressionSearch|NSCaseInsensitiveSearch].location!=NSNotFound) continue;
        CGFloat px=[price[@"x"] doubleValue],py=[price[@"y"] doubleValue],h=[price[@"h"] doubleValue];
        if (fabs(cursor.x-px)>.13 || fabs(cursor.y-py)>.11) continue;
        CGFloat left=px-[price[@"w"] doubleValue]/2;
        if (readQuantityStrip(image,CGRectMake(left-.095,py-1.2*h,.093,2.4*h),py,h,NO,labels)) continue;
        // On compact counters the decrement and increment icons can obscure
        // a single digit in the wider scan. Read the gap to the left of the +.
        if (readQuantityStrip(image,CGRectMake(cursor.x-.067,cursor.y-.027,.062,.054),py,h,NO,labels)) continue;
        CGRect digit=CGRectMake(cursor.x-.028,cursor.y-.014,.018,.028);
        if (!readQuantityStrip(image,digit,py,h,NO,labels))
            readQuantityStrip(image,digit,py,h,YES,labels);
    }
    if (trace) {
        NSCountedSet *kinds=[[NSCountedSet alloc] initWithArray:[labels valueForKey:@"kind"]];
        fprintf(stderr,"[Beside baggage] labels heading=%lu route=%lu quantity=%lu included=%lu price=%lu\n",
            (unsigned long)[kinds countForObject:@"heading"],(unsigned long)[kinds countForObject:@"route"],
            (unsigned long)[kinds countForObject:@"quantity"],(unsigned long)[kinds countForObject:@"included"],
            (unsigned long)[kinds countForObject:@"price"]);
    }
    return @{@"trip":itinerary,@"labels":labels,@"x":@(cursor.x),@"y":@(cursor.y)};
}
