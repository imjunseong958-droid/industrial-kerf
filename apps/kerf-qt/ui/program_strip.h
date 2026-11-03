#pragma once

// The Program monitor's title strip (mockup-spec section 2): 28 points on
// chrome with a bottom line, padding 0 12 0 16, gap 4. "Program" (UI 500
// 12/16 ink), then while a proposal is previewed a 6 pt dot and
// "Previewing proposal vN" (UI 12/16 ink3) with Exit preview (a ghost
// button: preview_exit); on the right the viewer's switches the window
// hands it: the mask tools (⇧R ⇧E ⇧P), a separator, Annotations, Safe zones
// and Dim surround, a separator, and the Fit ghost with its chevron (the
// viewer's zoom). Each is a kit button that follows its action: enabled,
// checked, and its tip (name · shortcut, and the reason while it is
// unavailable), so it shows the same state as the menus.
//
// Q-STAGE (W2), on Q-SKELETON's shell; the constructor and this API stay.

#include <QJsonObject>
#include <QList>
#include <QPointer>
#include <QString>
#include <QWidget>

class QAbstractButton;
class QAction;
class QHBoxLayout;
class QLabel;

namespace kerf {

class EditorBus;

namespace ui {
class Dot;
class GhostButton;
} // namespace ui

class ProgramStrip final : public QWidget {
    Q_OBJECT
public:
    explicit ProgramStrip(QWidget *parent, EditorBus *bus);

    // The actions on the strip's right, in order; a null entry is a
    // separator. Replaces the ones set before. The zoom (viewer.zoom) is a
    // ghost button that asks for its menu (zoomMenuRequested).
    void setActions(const QList<QAction *> &actions);
    QList<QAction *> actions() const { return m_actions; }
    // The button showing `action`, if the strip shows it.
    QAbstractButton *buttonFor(const QAction *action) const;
    // The zoom's ghost button, and its label: "Fit", "50%", "100%", "200%".
    QAbstractButton *zoomButton() const;
    void setZoomText(const QString &text);
    QString zoomText() const { return m_zoomText; }
    // The program's details for the title's tip ("1920 × 1080 · 25 fps").
    void setDetail(const QString &detail);
    QString detail() const { return m_detail; }

    // Whether the strip says a proposal preview is on, and which.
    bool previewShown() const;
    QString previewText() const;
    QAbstractButton *exitPreviewButton() const;

    // The kit icon an action of the strip draws, by its object name.
    static QString iconFor(const QAction *action);
    // The strip's name for an action (the mockup's tips).
    static QString tipFor(const QAction *action);

    QSize sizeHint() const override;
    QSize minimumSizeHint() const override;

signals:
    // Exit preview was pressed (the bus also sends `preview_exit`).
    void exitPreviewRequested();
    // The zoom's ghost button was pressed: open the Fit menu under it.
    void zoomMenuRequested(QAbstractButton *anchor);

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    // A `preview` event: shown while a proposal's preview runs or plays.
    void showPreview(const QJsonObject &event);
    // The button as its action is now.
    void follow(QAbstractButton *button, const QAction *action);

    EditorBus *m_bus = nullptr;
    QHBoxLayout *m_trailing = nullptr;
    QLabel *m_title = nullptr;
    ui::Dot *m_previewDot = nullptr;
    QLabel *m_preview = nullptr;
    ui::GhostButton *m_exitPreview = nullptr;
    QList<QAction *> m_actions;
    QList<QWidget *> m_trailingWidgets;
    QList<QPair<QPointer<QAbstractButton>, QPointer<QAction>>> m_buttons;
    QString m_zoomText = QStringLiteral("Fit");
    QString m_detail;
};

} // namespace kerf
