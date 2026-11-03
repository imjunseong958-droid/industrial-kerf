#pragma once

// The QC pane of the Finish tray and the Deliver tray's preflight
// (mockup-spec 12, 13): one row per check of the bridge's `check` event
// (plan section 3), a dot for pass or fail, the measured value, and for a
// failing row with a thread "In thread", which opens it.
//
// Shell from Q-SKELETON (W1); Q-FINISH gives it its final look in W2,
// keeping the constructor and this API.

#include <QJsonObject>
#include <QWidget>

class QLabel;
class QVBoxLayout;

namespace kerf {

class EditorBus;

class QcView final : public QWidget {
    Q_OBJECT
public:
    explicit QcView(QWidget *parent, EditorBus *bus);
    // A `check` event: {status, valid, findings, summary:{passed, total, blocking, rows}}.
    void setCheck(const QJsonObject &check);
    QJsonObject check() const { return m_check; }
    // "6 of 8 pass", or empty before a check.
    QString summaryText() const;
    int rowCount() const;
    QString statusText() const;

signals:
    // "In thread" on a failing row: the thread its finding is mirrored to.
    void threadRequested(const QString &commentId);

private:
    void rebuild();
    EditorBus *m_bus = nullptr;
    QJsonObject m_check;
    QLabel *m_status = nullptr;
    QWidget *m_rows = nullptr;
    QVBoxLayout *m_rowLayout = nullptr;
};

} // namespace kerf
