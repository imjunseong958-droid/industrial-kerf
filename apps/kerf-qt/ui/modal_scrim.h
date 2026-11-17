#pragma once

// What one goes into opens over the window, not in its place: a panel in the
// middle of a dimmed window, the work still there under it. Esc, the panel's
// own close, or a press on the dim closes it, and the window is as it was.
//
// The scrim fills what it is given (StageShell::setCover gives it the whole
// shell) and keeps the panel in the middle of its width and a little above
// the middle of its height, where the eye takes the middle to be, as large as
// `setPanelSize` allows with a margin all round.

#include <QSize>
#include <QWidget>

namespace kerf {

// The same dim over something the scrim does not reach (the window's toolbar,
// which is not the shell's): it covers its parent, whatever size that is, and
// a press on it closes what is open like a press on the scrim's own dim.
class ModalDim final : public QWidget {
    Q_OBJECT
public:
    explicit ModalDim(QWidget *over);

signals:
    void dismissed();

protected:
    void paintEvent(QPaintEvent *) override;
    void mousePressEvent(QMouseEvent *event) override;
    bool eventFilter(QObject *watched, QEvent *event) override;
};

class ModalScrim final : public QWidget {
    Q_OBJECT
public:
    explicit ModalScrim(QWidget *parent = nullptr);

    // The panel it holds, which it takes as its child.
    void setPanel(QWidget *panel);
    QWidget *panel() const { return m_panel; }
    // The most the panel is; less where the window is smaller.
    void setPanelSize(const QSize &size);
    // Where the panel is, for the scrim's size now.
    QRect panelRect() const;

signals:
    // A press on the dim, outside the panel.
    void dismissed();

protected:
    void paintEvent(QPaintEvent *) override;
    void resizeEvent(QResizeEvent *) override;
    void mousePressEvent(QMouseEvent *event) override;

private:
    QWidget *m_panel = nullptr;
    QSize m_most{1040, 760};
};

} // namespace kerf
