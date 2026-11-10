#pragma once

// Parts the panels share: a label in one of the design's type styles, and a
// button that follows a registry action.

#include "design_tokens.h"

#include <QString>
#include <QWidget>

class QAbstractButton;
class QAction;
class QLabel;

namespace kerf::shell {

// A label in `style` and `ink`.
QLabel *text(const QString &content, const tokens::TypeStyle &style, QRgb ink);
// Gives `label` the ink, size and weight of `style` as its own sheet. The
// app's sheet (its `*` rule sets every widget's text colour and 12 px) wins
// over a palette and a font's size, so a label that must keep them, or
// change its ink later, says so here.
void setTextStyle(QLabel *label, const tokens::TypeStyle &style, QRgb ink);

// Shows `button` as the registry's `action` is: enabled with it, with its
// tool tip (the label, the shortcut of the keymap in use, and the reason it
// is unavailable). A non-empty `blocked` disables it too, with that reason
// for its tip while the action itself is available; without an action it is
// disabled. Call it again when either changes (the action's changed()).
void followAction(QAbstractButton *button, const QAction *action, const QString &blocked = {});

} // namespace kerf::shell
