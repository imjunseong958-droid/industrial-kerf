#pragma once

// What MainWindow's region files (main_window_*.cpp) share: the toolbar's
// metrics, whether the toolbar is the window's title bar, and the symbols
// drawn for the toolbar and the tool palette. Moved from main_window.cpp's
// anonymous namespace when MainWindow was split by region (Q-SKELETON, W1).

#include "design_tokens.h"

#include <QIcon>
#include <QJsonObject>
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
// The toolbar is one line with one set of numbers, at both ends and in the
// middle, so that nothing in it is placed by eye:
//   - everything sits on the middle of its height;
//   - the first thing's middle is 16 from the window's left edge and the last
//     thing's 16 from its right: the first window button on macOS (14 across,
//     so it starts 9 in) and the panel's button;
//   - the window buttons stand 23 apart, middle to middle (16, 39, 62);
//   - the toolbar's own controls are 28 across and stand 28 apart, middle to
//     middle, the first of them 28 after the last window button's (90);
//   - the title starts 16 past the sidebar's edge, as the row starts 16 past
//     the window's.
inline constexpr int kToolbarEdge = 16;
inline constexpr int kWindowButton = 14;
inline constexpr int kWindowButtonsLeft = kToolbarEdge - kWindowButton / 2;
inline constexpr int kWindowButtonStep = 23;
inline constexpr int kToolbarControl = 28;
inline constexpr int kToolbarStep = 28;
// Where the first control starts, after the window buttons.
inline constexpr int kFirstControlAfterWindowButtons =
    kWindowButtonsLeft + 2 * kWindowButtonStep + kWindowButton / 2 + kToolbarStep - kToolbarControl / 2;
// What the row keeps clear at its right end, so the last control's middle is
// kToolbarEdge from the window's edge.
inline constexpr int kToolbarTrailing = kToolbarEdge - kToolbarControl / 2;
// The toolbar's own icons, at its two ends: as large as the sidebar's.
inline constexpr int kToolbarIcon = 18;
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

// The timeline proposal `id` would make: the snapshot's proposalTimelines
// entry, else (an older kerfd) proposalTimeline when `id` is the pending one.
inline QJsonObject proposalTimelineFor(const QJsonObject &snapshot, const QString &id) {
    if (id.isEmpty()) return {};
    const QJsonObject listed = snapshot.value(QStringLiteral("proposalTimelines")).toObject().value(id).toObject();
    if (!listed.isEmpty()) return listed;
    const QJsonObject pending = snapshot.value(QStringLiteral("pendingProposal")).toObject();
    return pending.value(QStringLiteral("proposalId")).toString() == id ? snapshot.value(QStringLiteral("proposalTimeline")).toObject()
                                                                       : QJsonObject{};
}

} // namespace kerf::window_parts
