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
class QHBoxLayout;
class QLabel;
class QVariantAnimation;

namespace kerf::shell {

// A label in `style` and `ink`.
QLabel *text(const QString &content, const tokens::TypeStyle &style, QRgb ink);
// Gives `label` the ink, size and weight of `style` as its own sheet. The
// app's sheet (its `*` rule sets every widget's text colour and 12 px) wins
// over a palette and a font's size, so a label that must keep them, or
// change its ink later, says so here.
void setTextStyle(QLabel *label, const tokens::TypeStyle &style, QRgb ink);

// A tray pane (mockup-spec 12, 13): a card with a 28-point header (its name,
// then `meta` when there is one) over `body`; highlighted, its outline is
// the accent, faded in and out over 160 ms (tokens::motion::fade, none
// under reduced motion).
class TrayPane final : public QWidget {
public:
    TrayPane(const QString &objectName, const QString &name, QLabel *meta, QWidget *body, QWidget *parent = nullptr);
    void setHighlighted(bool on);
    bool highlighted() const { return m_highlighted; }
    // How far the outline is toward the accent now: 0 the line, 1 the accent.
    double highlightLevel() const { return m_level; }
    // The outline's colour as drawn now.
    QColor outlineColor() const;
    // A widget at the header's right end, after its spacer (Waveform's
    // "Luma ⌄", QC's standards).
    void addTrailing(QWidget *widget);

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    QHBoxLayout *m_header = nullptr;
    QVariantAnimation *m_fade = nullptr;
    bool m_highlighted = false;
    double m_level = 0.0;
};

// Shows `button` as the registry's `action` is: enabled with it, with its
// tool tip (the label, the shortcut of the keymap in use, and the reason it
// is unavailable). A non-empty `blocked` disables it too, with that reason
// for its tip while the action itself is available; without an action it is
// disabled. Call it again when either changes (the action's changed()).
void followAction(QAbstractButton *button, const QAction *action, const QString &blocked = {});

} // namespace kerf::shell
