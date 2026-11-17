#pragma once

class QWidget;

namespace kerf {

// Hides the title macOS draws in a window's title bar, and its file's icon
// beside it, for a window that draws its own under a transparent one
// (Qt::ExpandedClientAreaHint with Qt::NoTitleBarBackgroundHint). Does
// nothing off the cocoa platform.
void hideNativeWindowTitle(QWidget *window);

// Puts the window buttons where the window's own title bar wants them: the
// first `left` points in, the row's middle `centerY` points down from the
// window's top. The system puts them back where it has them whenever the
// window is laid out again, so this is called again then. Does nothing in
// full screen, or off the cocoa platform.
void placeWindowButtons(QWidget *window, int left, int centerY);

// How far the window buttons reach from the window's leading edge, in
// points: the right edge of the zoom button. 0 off the cocoa platform.
int windowButtonsWidth(QWidget *window);

// What a double click on the title bar does, as System Settings has it:
// zoom (the default), minimize, or nothing. Does nothing off cocoa.
void titleBarDoubleClicked(QWidget *window);

} // namespace kerf
