#pragma once

// The parts the trays and their panes share (Q-SKELETON): a label in one of
// the design's type styles, the tray pane card, and a button that follows a
// registry action. The Finish and Deliver trays, LoudnessView, QcView and
// DeliverPane use them, so W2's owners change them in one place.

#include "design_tokens.h"

#include <QString>
#include <QWidget>

class QAbstractButton;
class QAction;
class QLabel;

namespace kerf::shell {

// A label in `style` and `ink`.
QLabel *text(const QString &content, const tokens::TypeStyle &style, QRgb ink);

// A tray pane (mockup-spec 12, 13): a card with a 28-point header (its name,
// then `meta` when there is one) over `body`; highlighted, its outline is
// the accent.
class TrayPane final : public QWidget {
public:
    TrayPane(const QString &objectName, const QString &name, QLabel *meta, QWidget *body, QWidget *parent = nullptr);
    void setHighlighted(bool on);
    bool highlighted() const { return m_highlighted; }

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    bool m_highlighted = false;
};

// Shows `button` as the registry's `action` is: enabled with it, with its
// tool tip (the label, the shortcut of the keymap in use, and the reason it
// is unavailable). A non-empty `blocked` disables it too, with that reason
// for its tip while the action itself is available; without an action it is
// disabled. Call it again when either changes (the action's changed()).
void followAction(QAbstractButton *button, const QAction *action, const QString &blocked = {});

} // namespace kerf::shell
