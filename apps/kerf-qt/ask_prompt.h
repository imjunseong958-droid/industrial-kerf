#pragma once

#include <QFrame>

class QComboBox;
class QLabel;
class QTextEdit;

namespace kerf {

// The prompt that asks Kerf about what is selected: a small popup over the
// clip or playhead it is anchored to. Return sends; Shift+Return adds a
// line; Escape or a click elsewhere closes it.
class AskPrompt final : public QFrame {
    Q_OBJECT
    friend class EditorTest;
public:
    explicit AskPrompt(QWidget *parent = nullptr);

    // Open above `anchor` (global coordinates). With a clip, the request can
    // stay on the clip or take in the whole sequence around it.
    void open(const QPoint &anchor, const QString &title, bool hasClip);
    // `clip` or `sequence`.
    QString scope() const;

signals:
    void submitted(const QString &text, const QString &scope);

protected:
    bool eventFilter(QObject *object, QEvent *event) override;

private:
    void submit();

    QLabel *m_title = nullptr;
    QTextEdit *m_input = nullptr;
    QComboBox *m_scope = nullptr;
};

} // namespace kerf
