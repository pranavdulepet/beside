#import <EventKit/EventKit.h>

static EKEventStore *calendarStore(void) {
    static EKEventStore *store;
    static dispatch_once_t once;
    dispatch_once(&once, ^{ store = [EKEventStore new]; });
    return store;
}

int beside_calendar_permitted(void) {
    return [EKEventStore authorizationStatusForEntityType:EKEntityTypeEvent] == EKAuthorizationStatusFullAccess;
}

int beside_calendar_allow(void) {
    @autoreleasepool {
        if ([EKEventStore authorizationStatusForEntityType:EKEntityTypeEvent] == EKAuthorizationStatusFullAccess) return 1;
        dispatch_semaphore_t done = dispatch_semaphore_create(0);
        __block BOOL allowed = NO;
        [calendarStore() requestFullAccessToEventsWithCompletion:^(BOOL granted, NSError *error) {
            allowed = granted && !error;
            dispatch_semaphore_signal(done);
        }];
        if (dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 60 * NSEC_PER_SEC))) return 0;
        return allowed;
    }
}

char *beside_meetings_between(int64_t earliest, int64_t until) {
    @autoreleasepool {
        if ([EKEventStore authorizationStatusForEntityType:EKEntityTypeEvent] != EKAuthorizationStatusFullAccess) return NULL;
        NSDate *from = [NSDate dateWithTimeIntervalSince1970:earliest];
        NSDate *to = [NSDate dateWithTimeIntervalSince1970:until];
        EKEventStore *store = calendarStore();
        @synchronized(store) {
            NSPredicate *range = [store predicateForEventsWithStartDate:from endDate:to calendars:nil];
            NSArray<EKEvent *> *events = [[store eventsMatchingPredicate:range] sortedArrayUsingComparator:^NSComparisonResult(EKEvent *a, EKEvent *b) { return [a.startDate compare:b.startDate]; }];
            NSMutableArray *items = [NSMutableArray array];
            for (EKEvent *event in events) {
                if (event.allDay || event.status == EKEventStatusCanceled || !event.eventIdentifier.length || !event.title.length || !event.startDate || !event.endDate) continue;
                if ([event.startDate compare:from] == NSOrderedAscending || [event.startDate compare:to] != NSOrderedAscending) continue;
                [items addObject:@{@"id":event.eventIdentifier, @"title":event.title,
                                   @"start":@((int64_t)event.startDate.timeIntervalSince1970),
                                   @"end":@((int64_t)event.endDate.timeIntervalSince1970)}];
            }
            NSData *data = [NSJSONSerialization dataWithJSONObject:items options:0 error:NULL];
            NSString *encoded = data ? [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding] : nil;
            return encoded.UTF8String ? strdup(encoded.UTF8String) : NULL;
        }
    }
}
