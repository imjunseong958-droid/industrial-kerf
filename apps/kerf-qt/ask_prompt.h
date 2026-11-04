#pragma once

#include "test_friends.h"

#include <QFrame>

class QLabel;
class QTextEdit;

namespace kerf {

namespace ui {
class Button;
class GhostButton;
} // namespace ui

// The Ask popover (#mAsk, mockup-spec 7.1): 380 wide, pad 12, a card with
// the pop shadow over the clip or playhead it is anchored to.
//
//   Sequence at 00:00:17:08                    (UI 500 13/20)
//   Scope  [This clip · A019_C001 ⌄]           (12 ink2, a ghost with its menu)
//   ┌ Tell Kerf what to change ───────────┐    (56 tall, raised, inset line2)
//   └──────────────────────────────────────┘
//   ↵ Send · ⇧↵ New line              [Ask]    (11 ink3, a primary button)
//
// Return sends; Shift+Return adds a line; Escape or a click elsewhere
// closes it. MainWindow sends the request as an agent turn with its focus.
class AskPrompt final : public QFrame {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    static constexpr int kWidth = 380;
    explicit AskPrompt(QWidget *parent = nullptr);

    // Opens above `anchor` (global coordinates). With a clip (its name), the
    // request can stay on the clip or take in the whole sequence.
    void open(const QPoint &anchor, const QString &title, const QString &clip);
    // `clip` or `sequence`.
    QString scope() const { return m_scope; }
    void setScope(const QString &scope);

    QLabel *titleLabel() const { return m_title; }
    ui::GhostButton *scopeButton() const { return m_scopeButton; }
    QTextEdit *input() const { return m_input; }
    ui::Button *askButton() const { return m_ask; }
    // The card's rect, inside the shadow's margins.
    QRect cardRect() const;

signals:
    void submitted(const QString &text, const QString &scope);

protected:
    bool eventFilter(QObject *object, QEvent *event) override;
    void paintEvent(QPaintEvent *) override;

private:
    void submit();
    void showScopeMenu();

    QLabel *m_title = nullptr;
    ui::GhostButton *m_scopeButton = nullptr;
    QWidget *m_field = nullptr;
    QTextEdit *m_input = nullptr;
    ui::Button *m_ask = nullptr;
    QString m_clip;
    QString m_scope = QStringLiteral("sequence");
};

} // namespace kerf
