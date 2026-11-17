#include "macos_window.h"

#include <QGuiApplication>
#include <QWidget>

#import <AppKit/AppKit.h>

#include <cmath>

namespace kerf {

namespace {

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

void placeWindowButtons(QWidget *window, int left, int centerY) {
    NSWindow *native = nativeWindow(window);
    if (!native || (native.styleMask & NSWindowStyleMaskFullScreen)) return;
    NSButton *close = [native standardWindowButton:NSWindowCloseButton];
    NSButton *minimize = [native standardWindowButton:NSWindowMiniaturizeButton];
    NSButton *zoom = [native standardWindowButton:NSWindowZoomButton];
    NSView *bar = close.superview;
    if (!close || !minimize || !zoom || !bar) return;
    // As far apart as the system has them; only where the row starts and how
    // far down it sits are the window's own.
    const CGFloat step = NSMinX(minimize.frame) - NSMinX(close.frame);
    CGFloat x = left;
    for (NSButton *button in @[close, minimize, zoom]) {
        const CGFloat height = NSHeight(button.frame);
        const CGFloat y = bar.isFlipped ? centerY - height / 2 : NSHeight(bar.frame) - centerY - height / 2;
        [button setFrameOrigin:NSMakePoint(x, y)];
        x += step;
    }
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
