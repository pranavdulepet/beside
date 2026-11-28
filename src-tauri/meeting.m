#import <AppKit/AppKit.h>
#import <AVFoundation/AVFoundation.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import <Speech/Speech.h>

// All recorder state and picker callbacks are serialized on the main queue.
API_AVAILABLE(macos(15.0))
@interface BesideMeeting : NSObject <SCContentSharingPickerObserver, SCStreamDelegate, SCRecordingOutputDelegate>
@property(nonatomic, copy) NSString *phase, *issue;
@property(nonatomic, strong) NSURL *destination;
@property BOOL microphone;
@property(nonatomic, strong) SCStream *stream;
@property(nonatomic, strong) SCRecordingOutput *output;
@end

@implementation BesideMeeting
- (void)end:(NSString *)error {
    SCContentSharingPicker *picker = SCContentSharingPicker.sharedPicker;
    [picker removeObserver:self];
    picker.active = NO;
    self.phase = error.length ? @"failed" : @"idle";
    self.issue = error;
    self.stream = nil;
    self.output = nil;
}
- (void)begin:(NSURL *)url mic:(BOOL)mic {
    if (mic && [AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio] != AVAuthorizationStatusAuthorized) {
        self.phase = @"authorizing";
        self.issue = nil;
        [AVCaptureDevice requestAccessForMediaType:AVMediaTypeAudio completionHandler:^(BOOL granted) {
            dispatch_async(dispatch_get_main_queue(), ^{
                if (![self.phase isEqual:@"authorizing"]) return;
                if (granted) [self begin:url mic:YES];
                else [self end:@"Microphone access was declined. Turn off Include my microphone or allow Beside in macOS Microphone settings."];
            });
        }];
        return;
    }
    self.destination = url;
    self.microphone = mic;
    self.phase = @"choosing";
    self.issue = nil;
    SCContentSharingPicker *picker = SCContentSharingPicker.sharedPicker;
    SCContentSharingPickerConfiguration *configuration = [SCContentSharingPickerConfiguration new];
    configuration.allowedPickerModes = SCContentSharingPickerModeSingleWindow;
    configuration.allowsChangingSelectedContent = NO;
    configuration.excludedBundleIDs = @[NSBundle.mainBundle.bundleIdentifier ?: @"studio.beside.notebook"];
    picker.defaultConfiguration = configuration;
    [picker addObserver:self];
    picker.active = YES;
    [picker presentPickerUsingContentStyle:SCShareableContentStyleWindow];
}
- (void)contentSharingPicker:(SCContentSharingPicker *)picker didCancelForStream:(SCStream *)stream {
    dispatch_async(dispatch_get_main_queue(), ^{ if ([self.phase isEqual:@"choosing"]) [self end:nil]; });
}
- (void)contentSharingPickerStartDidFailWithError:(NSError *)error {
    dispatch_async(dispatch_get_main_queue(), ^{ if ([self.phase isEqual:@"choosing"]) [self end:error.localizedDescription]; });
}
- (void)contentSharingPicker:(SCContentSharingPicker *)picker didUpdateWithFilter:(SCContentFilter *)filter forStream:(SCStream *)prior {
    dispatch_async(dispatch_get_main_queue(), ^{
        if (![self.phase isEqual:@"choosing"] || prior) return;
        self.phase = @"starting";
        SCStreamConfiguration *config = [SCStreamConfiguration new];
        config.capturesAudio = YES;
        config.captureMicrophone = self.microphone;
        config.excludesCurrentProcessAudio = YES;
        config.showsCursor = NO;
        config.minimumFrameInterval = CMTimeMake(1, 12);
        CGRect rect = filter.contentRect;
        CGFloat scale = MIN(1., 1440. / MAX(1., rect.size.width));
        config.width = MAX(2, 2 * (NSInteger)(rect.size.width * scale / 2));
        config.height = MAX(2, 2 * (NSInteger)(rect.size.height * scale / 2));
        SCStream *stream = [[SCStream alloc] initWithFilter:filter configuration:config delegate:self];
        SCRecordingOutputConfiguration *file = [SCRecordingOutputConfiguration new];
        file.outputURL = [[self.destination URLByDeletingLastPathComponent] URLByAppendingPathComponent:[@"." stringByAppendingString:self.destination.lastPathComponent]];
        file.outputFileType = AVFileTypeMPEG4;
        file.videoCodecType = AVVideoCodecTypeH264;
        self.stream = stream;
        self.output = [[SCRecordingOutput alloc] initWithConfiguration:file delegate:self];
        NSError *err = nil;
        if (![stream addRecordingOutput:self.output error:&err]) {
            [self end:err.localizedDescription ?: @"Cannot prepare the recording."];
            return;
        }
        [stream startCaptureWithCompletionHandler:^(NSError *error) {
            dispatch_async(dispatch_get_main_queue(), ^{
                if (self.stream == stream && error) [self end:error.localizedDescription];
            });
        }];
    });
}
- (void)recordingOutputDidStartRecording:(SCRecordingOutput *)output {
    dispatch_async(dispatch_get_main_queue(), ^{ if (self.output == output && [self.phase isEqual:@"starting"]) self.phase = @"recording"; });
}
- (void)recordingOutputDidFinishRecording:(SCRecordingOutput *)output {
    dispatch_async(dispatch_get_main_queue(), ^{ if (self.output == output) {
        NSURL *temp = [[self.destination URLByDeletingLastPathComponent] URLByAppendingPathComponent:[@"." stringByAppendingString:self.destination.lastPathComponent]];
        NSError *error = nil;
        [NSFileManager.defaultManager moveItemAtURL:temp toURL:self.destination error:&error];
        [self end:error.localizedDescription];
    } });
}
- (void)recordingOutput:(SCRecordingOutput *)output didFailWithError:(NSError *)error {
    dispatch_async(dispatch_get_main_queue(), ^{ if (self.output == output) [self end:error.localizedDescription]; });
}
- (void)stream:(SCStream *)stream didStopWithError:(NSError *)error {
    dispatch_async(dispatch_get_main_queue(), ^{
        if (self.stream == stream && error.code != SCStreamErrorUserStopped) [self end:error.localizedDescription];
    });
}
- (void)stop {
    if ([self.phase isEqual:@"choosing"] || [self.phase isEqual:@"authorizing"]) { [self end:nil]; return; }
    if (![self.phase isEqual:@"recording"]) return;
    SCStream *stream = self.stream;
    self.phase = @"saving";
    [stream stopCaptureWithCompletionHandler:^(NSError *error) {
        dispatch_async(dispatch_get_main_queue(), ^{ if (self.stream == stream && error) [self end:error.localizedDescription]; });
    }];
}
@end

char *beside_meeting(const char *operation, const char *path, int mic) {
    @autoreleasepool {
        __block NSDictionary *result = @{@"phase":@"unavailable"};
        if (@available(macOS 15.0, *)) {
            void (^work)(void) = ^{
                static BesideMeeting *meeting;
                if (!meeting) meeting = [BesideMeeting new];
                if (!strcmp(operation, "start") && (!meeting.phase || [meeting.phase isEqual:@"idle"] || [meeting.phase isEqual:@"failed"])) {
                    [meeting begin:[NSURL fileURLWithPath:@(path)] mic:mic != 0];
                } else if (!strcmp(operation, "stop")) [meeting stop];
                result = @{@"phase":meeting.phase ?: @"idle", @"error":meeting.issue ?: @""};
            };
            if (NSThread.isMainThread) work(); else dispatch_sync(dispatch_get_main_queue(), work);
        }
        NSData *json = [NSJSONSerialization dataWithJSONObject:result options:0 error:nil];
        return strdup([[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding].UTF8String);
    }
}

// The request is local-only, including when no language pack is installed.
char *beside_meeting_transcript(const char *path) {
    @autoreleasepool {
        dispatch_semaphore_t permission = dispatch_semaphore_create(0);
        __block BOOL granted = NO;
        [SFSpeechRecognizer requestAuthorization:^(SFSpeechRecognizerAuthorizationStatus status) {
            granted = status == SFSpeechRecognizerAuthorizationStatusAuthorized;
            dispatch_semaphore_signal(permission);
        }];
        if (dispatch_semaphore_wait(permission, dispatch_time(DISPATCH_TIME_NOW, 90 * NSEC_PER_SEC)) || !granted)
            return strdup("{\"error\":\"Allow Speech Recognition in macOS to transcribe this recording.\"}");
        SFSpeechRecognizer *recognizer = [[SFSpeechRecognizer alloc] initWithLocale:NSLocale.currentLocale];
        if (!recognizer.supportsOnDeviceRecognition) return strdup("{\"error\":\"On-device speech recognition is unavailable for this language on this Mac.\"}");
        SFSpeechURLRecognitionRequest *request = [[SFSpeechURLRecognitionRequest alloc] initWithURL:[NSURL fileURLWithPath:@(path)]];
        request.requiresOnDeviceRecognition = YES;
        request.shouldReportPartialResults = NO;
        dispatch_semaphore_t done = dispatch_semaphore_create(0);
        __block NSDictionary *result;
        SFSpeechRecognitionTask *task = [recognizer recognitionTaskWithRequest:request resultHandler:^(SFSpeechRecognitionResult *r, NSError *e) {
            if (e || r.isFinal) {
                @synchronized(request) {
                    if (result) return;
                    result = e ? @{@"error":e.localizedDescription} : @{@"text":r.bestTranscription.formattedString ?: @""};
                }
                dispatch_semaphore_signal(done);
            }
        }];
        if (dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 600 * NSEC_PER_SEC))) { [task cancel];return strdup("{\"error\":\"Local transcription timed out. The recording is saved.\"}"); }
        NSData *json = [NSJSONSerialization dataWithJSONObject:result options:0 error:nil];
        return strdup([[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding].UTF8String);
    }
}
