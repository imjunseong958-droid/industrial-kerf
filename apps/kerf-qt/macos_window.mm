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
    if (NSWindow *native = nativeWindow(window)) native.titleVisibility = NSWindowTitleHidden;
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
