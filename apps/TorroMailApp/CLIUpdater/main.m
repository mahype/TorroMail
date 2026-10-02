// TorroMail's separate terminal updater. Sparkle's CLI drivers are compiled
// from the same pinned checkout as the framework (see build-cli-updater.sh).
#import <Foundation/Foundation.h>
#import <Sparkle/Sparkle.h>
#import "SPUCommandLineDriver.h"

static int fail(NSString *message) {
    fprintf(stderr, "torromail update: %s\n", message.UTF8String);
    return EXIT_FAILURE;
}

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        if (argc < 2) return fail(@"Missing TorroMail app path.");
        NSString *path = [NSString stringWithUTF8String:argv[1]];
        if (path.length == 0) return fail(@"Invalid TorroMail app path.");
        NSBundle *host = [NSBundle bundleWithPath:path];
        if (![host.bundleIdentifier isEqualToString:@"com.torromail.app"])
            return fail(@"The target is not a TorroMail app bundle.");
        id disabled = [host objectForInfoDictionaryKey:@"TorroMailDisableUpdates"];
        if (disabled != nil && ![disabled isKindOfClass:NSNumber.class])
            return fail(@"Invalid development update setting in the app's Info.plist.");
        if ([disabled boolValue])
            return fail(@"Updates are disabled in this development build. Use a signed release app.");
        NSString *key = [host objectForInfoDictionaryKey:@"SUPublicEDKey"];
        NSString *feed = [host objectForInfoDictionaryKey:@"SUFeedURL"];
        if (![key isKindOfClass:NSString.class] ||
            [[NSData alloc] initWithBase64EncodedString:key options:0].length != 32 ||
            ![feed isKindOfClass:NSString.class] || ![NSURL URLWithString:feed].host)
            return fail(@"The app has no valid Sparkle feed or signing key. Reinstall an official release.");

        BOOL checkOnly = NO, interactive = NO, allowMajor = NO;
        for (int i = 2; i < argc; i++) {
            if (strcmp(argv[i], "--check") == 0) checkOnly = YES;
            else if (strcmp(argv[i], "--interactive") == 0) interactive = YES;
            else if (strcmp(argv[i], "--allow-major-upgrades") == 0) allowMajor = YES;
            else return fail(@"Unknown updater option.");
        }
        if (checkOnly && interactive) return fail(@"--check cannot be combined with --interactive.");
        if (interactive && geteuid() == 0) return fail(@"Run --interactive as your normal user, without sudo.");

        SPUCommandLineDriver *driver = [[SPUCommandLineDriver alloc]
            initWithUpdateBundlePath:host.bundlePath
            applicationBundlePath:nil
            allowedChannels:[NSSet set]
            customFeedURL:nil
            userAgentName:@"TorroMail CLI"
            updatePermissionResponse:nil
            deferInstallation:NO
            interactiveInstallation:interactive
            allowMajorUpgrades:allowMajor
            verbose:YES];
        if (driver == nil) return fail(@"Unable to initialize Sparkle for this app.");
        if (checkOnly) [driver probeForUpdates];
        else [driver runAndCheckForUpdatesNow:YES];
        [[NSRunLoop currentRunLoop] run];
    }
    return EXIT_SUCCESS;
}
