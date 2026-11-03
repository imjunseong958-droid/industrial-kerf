#pragma once

// The Program monitor's title strip (mockup-spec section 2): 28 points on
// chrome with a bottom line. "Program", then while a proposal is previewed
// "Previewing proposal vN" and Exit preview; on the right the viewer's
// switches the window hands it (the mask tools, annotations, safe zones,
// dim, the zoom), each a registry action, so it shows the same enabled
// state, reason and shortcut as the menus.
//
// Shell from Q-SKELETON (W1); Q-STAGE fills it in (W2), keeping the
// constructor and this API.

#include <QJsonObject>
#include <QList>
#include <QString>
#include <QWidget>

class QAction;
class QHBoxLayout;
class QLabel;
class QToolButton;

namespace kerf {

class EditorBus;

class ProgramStrip final : public QWidget {
    Q_OBJECT
public:
    explicit ProgramStrip(QWidget *parent, EditorBus *bus);

    // The actions on the strip's right, in order; a null entry is a
    // separator. Replaces the ones set before.
    void setActions(const QList<QAction *> &actions);
    QList<QAction *> actions() const { return m_actions; }
    // The button showing `action`, if the strip shows it.
    QToolButton *buttonFor(const QAction *action) const;

    // Whether the strip says a proposal preview is on, and which.
    bool previewShown() const;
    QString previewText() const;
    QToolButton *exitPreviewButton() const { return m_exitPreview; }

    QSize sizeHint() const override;
    QSize minimumSizeHint() const override;

signals:
    // Exit preview was pressed (the bus also sends `preview_exit`).
    void exitPreviewRequested();

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    // A `preview` event: shown while a proposal's preview runs or plays.
    void showPreview(const QJsonObject &event);

    EditorBus *m_bus = nullptr;
    QHBoxLayout *m_trailing = nullptr;
    QLabel *m_title = nullptr;
    QLabel *m_previewDot = nullptr;
    QLabel *m_preview = nullptr;
    QToolButton *m_exitPreview = nullptr;
    QList<QAction *> m_actions;
    QList<QWidget *> m_trailingWidgets;
};

} // namespace kerf
