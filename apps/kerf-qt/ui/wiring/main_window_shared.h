#pragma once

// What MainWindow's region files (main_window_*.cpp) share: the toolbar's
// metrics, whether the toolbar is the window's title bar, and the symbols
// drawn for the toolbar and the tool palette. Moved from main_window.cpp's
// anonymous namespace when MainWindow was split by region (Q-SKELETON, W1).

#include "design_tokens.h"

#include <QIcon>
#include <QString>
#include <QtGlobal>

// On macOS the toolbar is the window's title bar, as in Logic and Final
// Cut: the window runs up under a transparent title bar, the window buttons
// sit at the toolbar's leading end, and the project's name is the title.
// Elsewhere the native title bar stays.
#if defined(Q_OS_MACOS) && QT_VERSION >= QT_VERSION_CHECK(6, 9, 0)
#define KERF_EDITOR_OWN_TITLE_BAR 1
#else
#define KERF_EDITOR_OWN_TITLE_BAR 0
#endif

namespace kerf::window_parts {

#if KERF_EDITOR_OWN_TITLE_BAR
// The room the window buttons take at the toolbar's leading end (after its
// 6 points of padding) where the platform does not report them.
inline constexpr int kWindowButtonsSpace = 72;
#endif
// The toolbar's height (mockup-spec 0.1).
inline constexpr int kToolbarHeight = tokens::layout::toolbar;
// The toolbar's symbols, in points; toolbarIcon() draws them at this size.
inline constexpr int kToolbarIconSize = 15;
// The style sheet's QToolBar spacing, between every pair of its items,
// and its padding, at either end.
inline constexpr int kToolbarSpacing = 2;
inline constexpr int kToolbarPadding = 6;
// The rows of the window's chrome (the toolbar, the banners under it and
// the agent's bar) end their last control this far in from the window's
// edge, so their trailing buttons line up.
inline constexpr int kTrailingInset = 8;
// How far the title keeps from the toolbar's groups; nearer, it gives way.
inline constexpr int kTitleClearance = 20;

// Toolbar symbols drawn on a shared grid so their weight and optical size
// match; a switch that is on has its symbol in the accent.
QIcon toolbarIcon(const QString &glyph);
// A tool's icon: secondary text when not chosen, the accent when chosen.
QIcon toolIcon(const QString &id);

} // namespace kerf::window_parts
