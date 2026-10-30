#include "macos_window.h"

#include <QGuiApplication>
#include <QWidget>

#import <AppKit/AppKit.h>

namespace kerf {

void hideNativeWindowTitle(QWidget *window) {
    if (QGuiApplication::platformName() != QLatin1String("cocoa")) return;
    auto *view = reinterpret_cast<NSView *>(window->winId());
    if (view.window) view.window.titleVisibility = NSWindowTitleHidden;
}

} // namespace kerf
