#pragma once

class QWidget;

namespace kerf {

// Hides the title macOS draws in a window's title bar, for a window that
// draws its own under a transparent one (Qt::ExpandedClientAreaHint with
// Qt::NoTitleBarBackgroundHint). Does nothing off the cocoa platform.
void hideNativeWindowTitle(QWidget *window);

} // namespace kerf
