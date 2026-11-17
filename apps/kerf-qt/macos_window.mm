#include "macos_window.h"

#include <QGuiApplication>
#include <QWidget>

#import <AppKit/AppKit.h>
#import <objc/runtime.h>

#include <cmath>

// Keeps a window's buttons where the window wants them. The system puts them
// back where it has them whenever it lays the title bar out again (a resize,
// a new title, the window becoming key), so their frames are watched and put
// right each time they move.
@interface KerfWindowButtonKeeper : NSObject
@property(nonatomic, assign) NSWindow *window;
@property(nonatomic) CGFloat left;
@property(nonatomic) CGFloat centerY;
@property(nonatomic) CGFloat step;
- (instancetype)initWithWindow:(NSWindow *)window;
- (void)place;
@end

@implementation KerfWindowButtonKeeper {
    BOOL _placing;
    BOOL _scheduled;
}

- (NSArray<NSButton *> *)buttons {
    NSMutableArray<NSButton *> *out = [NSMutableArray array];
    for (NSNumber *kind in @[@(NSWindowCloseButton), @(NSWindowMiniaturizeButton), @(NSWindowZoomButton)]) {
        NSButton *button = [self.window standardWindowButton:(NSWindowButton)kind.integerValue];
        if (button) [out addObject:button];
    }
    return out;
}

- (instancetype)initWithWindow:(NSWindow *)window {
    if ((self = [super init])) {
        _window = window;
        for (NSButton *button in [self buttons]) {
            button.postsFrameChangedNotifications = YES;
            [NSNotificationCenter.defaultCenter addObserver:self selector:@selector(moved:) name:NSViewFrameDidChangeNotification object:button];
        }
        [NSNotificationCenter.defaultCenter addObserver:self selector:@selector(closing:) name:NSWindowWillCloseNotification object:window];
    }
    return self;
}

- (void)dealloc {
    [NSNotificationCenter.defaultCenter removeObserver:self];
#if !__has_feature(objc_arc)
    [super dealloc];
#endif
}

- (void)closing:(NSNotification *)notification {
    [NSNotificationCenter.defaultCenter removeObserver:self];
    _window = nil;
}

- (void)moved:(NSNotification *)notification {
    // Not in the middle of the system's own layout, which goes on to move the
    // others: once it is done, and once for however many it moved.
    if (_placing || _scheduled) return;
    _scheduled = YES;
#if !__has_feature(objc_arc)
    [self retain];
#endif
    dispatch_async(dispatch_get_main_queue(), ^{
        self->_scheduled = NO;
        [self place];
#if !__has_feature(objc_arc)
        [self release];
#endif
    });
}

- (void)place {
    NSWindow *window = self.window;
    // In full screen the buttons are the system's, in its own bar.
    if (!window || _placing || (window.styleMask & NSWindowStyleMaskFullScreen)) return;
    _placing = YES;
    CGFloat x = self.left;
    for (NSButton *button in [self buttons]) {
        NSView *bar = button.superview;
        if (bar) {
            // By its middle, whatever size the system draws it at.
            const CGFloat width = NSWidth(button.frame);
            const CGFloat height = NSHeight(button.frame);
            const CGFloat y = bar.isFlipped ? self.centerY - height / 2 : NSHeight(bar.frame) - self.centerY - height / 2;
            const NSPoint origin = NSMakePoint(x + (14 - width) / 2, y);
            if (!NSEqualPoints(button.frame.origin, origin)) [button setFrameOrigin:origin];
        }
        x += self.step;
    }
    _placing = NO;
}

@end

namespace kerf {

namespace {

const char kKeeperKey = 0;

NSWindow *nativeWindow(QWidget *window) {
    if (QGuiApplication::platformName() != QLatin1String("cocoa")) return nil;
    auto *view = reinterpret_cast<NSView *>(window->winId());
    return view.window;
}

} // namespace

void hideNativeWindowTitle(QWidget *window) {
    NSWindow *native = nativeWindow(window);
    if (!native) return;
    native.titleVisibility = NSWindowTitleHidden;
    // A window with a file of its own gets that file's icon in the title bar,
    // title or no title: beside the window buttons, over what the window draws.
    [native standardWindowButton:NSWindowDocumentIconButton].hidden = YES;
}

void placeWindowButtons(QWidget *window, int left, int centerY, int step) {
    NSWindow *native = nativeWindow(window);
    if (!native) return;
    KerfWindowButtonKeeper *keeper = objc_getAssociatedObject(native, &kKeeperKey);
    if (!keeper) {
        keeper = [[KerfWindowButtonKeeper alloc] initWithWindow:native];
        objc_setAssociatedObject(native, &kKeeperKey, keeper, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
#if !__has_feature(objc_arc)
        [keeper release];
#endif
    }
    keeper.left = left;
    keeper.centerY = centerY;
    keeper.step = step;
    [keeper place];
}

int windowButtonsWidth(QWidget *window) {
    NSWindow *native = nativeWindow(window);
    NSButton *zoom = native ? [native standardWindowButton:NSWindowZoomButton] : nil;
    if (!zoom || zoom.hidden) return 0;
    return static_cast<int>(std::ceil(NSMaxX([zoom convertRect:zoom.bounds toView:nil])));
}

void titleBarDoubleClicked(QWidget *window) {
    NSWindow *native = nativeWindow(window);
    if (!native) return;
    NSString *action = [NSUserDefaults.standardUserDefaults stringForKey:@"AppleActionOnDoubleClick"];
    if ([action isEqualToString:@"Minimize"]) [native performMiniaturize:nil];
    else if (![action isEqualToString:@"None"]) [native performZoom:nil];
}

} // namespace kerf
