#pragma once

class QWidget;

namespace kerf {

// Hides the title macOS draws in a window's title bar, for a window that
// draws its own under a transparent one (Qt::ExpandedClientAreaHint with
// Qt::NoTitleBarBackgroundHint). Does nothing off the cocoa platform.
void hideNativeWindowTitle(QWidget *window);

// How far the window buttons reach from the window's leading edge, in
// points: the right edge of the zoom button. 0 off the cocoa platform.
int windowButtonsWidth(QWidget *window);

// What a double click on the title bar does, as System Settings has it:
// zoom (the default), minimize, or nothing. Does nothing off cocoa.
void titleBarDoubleClicked(QWidget *window);

} // namespace kerf
