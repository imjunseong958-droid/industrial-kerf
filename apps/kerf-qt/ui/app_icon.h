#pragma once

// Kerf's own icon, for the editor's windows and its entry in a taskbar.
//
// The icon is drawn once, in resources/icon/industrial-kerf.svg; tools/kerf-icon
// renders the PNG files beside it, and the sizes named here are compiled in at
// :/icon/industrial-kerf-<side>.png. The programs' own icons come from the same
// SVG another way: kerf-qt.exe carries the .ico as a resource and the macOS
// app the .icns in its Resources.

#include <QIcon>
#include <QList>

namespace kerf {

// The sides, in pixels, the application's icon is compiled in at.
QList<int> appIconSides();

// The icon at every one of those sides, so each place that shows it picks the
// one drawn for its size rather than scaling another.
QIcon appIcon();

} // namespace kerf
