#pragma once

// The QC pane of the Finish tray and the Deliver tray's preflight
// (mockup-spec 12, 13): one 28-point row per row of the bridge's `check`
// summary (plan section 3) — a pass or fail dot, the check's name, what it
// measured against what it allows (neg when failing), and "In thread" for a
// failing row mirrored to a comment (it opens that thread) or "Pass".
//
// Before any check it is an empty state with Run QC (the registry's qc.run,
// which sends check{}); while a check runs it shows the progress (over the
// last result's rows when there is one); a failed check says why; a
// connection that ends or begins ends the run it was waiting on. The rows
// scroll when there are more than fit.

#include <QJsonObject>
#include <QPointer>
#include <QWidget>

class QAction;
class QLabel;
class QVBoxLayout;

namespace kerf {

class EditorBus;
class QcRow;

namespace ui {
class Button;
class FadeScrollArea;
class ThinProgress;
} // namespace ui

class QcView final : public QWidget {
    Q_OBJECT
public:
    explicit QcView(QWidget *parent, EditorBus *bus);
    // A `check` event: {status, valid, findings, summary:{passed, total, blocking, rows}}.
    void setCheck(const QJsonObject &check);
    // The latest check event (running, done or failed).
    QJsonObject check() const { return m_check; }
    // The latest finished check the rows show (empty before one).
    QJsonObject result() const { return m_result; }
    // "6 of 8 pass", or empty before a check.
    QString summaryText() const;
    // What the check measures against, from its rows: "R128 · Rec.709 · captions".
    QString standardsText() const;
    // A check is running.
    bool running() const;
    // 0..1 while running.
    double progress() const;
    int rowCount() const;
    QString rowName(int row) const;
    QString rowValue(int row) const;
    bool rowPassed(int row) const;
    QString rowThread(int row) const;
    QColor rowValueColor(int row) const;
    // What stands in for the rows (empty state, progress, failure), or empty.
    QString statusText() const;
    // The empty state's Run QC; it follows `action` (qc.run) once set.
    ui::Button *runButton() const { return m_run; }
    void setRunAction(QAction *action);

signals:
    // "In thread" on a failing row: the thread its finding is mirrored to.
    void threadRequested(const QString &commentId);

private:
    void connectionChanged();
    void rebuild();
    void updateStatus();
    EditorBus *m_bus = nullptr;
    QJsonObject m_check;
    QJsonObject m_result;
    QPointer<QAction> m_runAction;
    QWidget *m_statusLine = nullptr;
    QLabel *m_lineText = nullptr;
    ui::ThinProgress *m_lineProgress = nullptr;
    QWidget *m_empty = nullptr;
    QLabel *m_status = nullptr;
    ui::ThinProgress *m_progress = nullptr;
    ui::Button *m_run = nullptr;
    ui::FadeScrollArea *m_scroll = nullptr;
    QWidget *m_rows = nullptr;
    QVBoxLayout *m_rowLayout = nullptr;
    QList<QcRow *> m_rowWidgets;
};

// One QC row: dot, name, value, then the action (64 wide). Hover raises it.
class QcRow final : public QWidget {
    Q_OBJECT
public:
    QcRow(const QJsonObject &row, QWidget *parent = nullptr);
    QString name() const { return m_name; }
    QString value() const { return m_value; }
    bool passed() const { return m_passed; }
    QString thread() const { return m_thread; }
    QColor valueColor() const;
    QSize sizeHint() const override;

signals:
    void threadRequested(const QString &commentId);

protected:
    void paintEvent(QPaintEvent *event) override;
    void enterEvent(QEnterEvent *event) override;
    void leaveEvent(QEvent *event) override;

private:
    QString m_name;
    QString m_value;
    QString m_thread;
    bool m_passed = false;
    bool m_hovered = false;
};

} // namespace kerf
