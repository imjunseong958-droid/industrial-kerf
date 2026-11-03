#pragma once

// The Finish tray (mockup-spec 12): in the Finish workspace it takes the
// whole bottom band. A 28-point bar (Scopes and what they show, Ask, the R128
// targets, Run QC) over five panes: Waveform, Vectorscope, RGB parade,
// Loudness and QC.
//
// Shell from Q-SKELETON (W1): Loudness and QC are live from the bridge's
// `meter`, `loudness` and `check` events; Ask runs `ai.ask` and Run QC
// `qc.run`, each enabled and explained as its registry action is; the scope
// panes stay empty until Q-FINISH computes them from program frames in W2.
// Q-FINISH fills it in, keeping the constructor and this API.

#include <QHash>
#include <QJsonObject>
#include <QPointer>
#include <QString>
#include <QWidget>

class QAction;
class QLabel;
class QPushButton;
class QToolButton;

namespace kerf {

class EditorBus;
class LoudnessView;
class QcView;

class FinishTray final : public QWidget {
    Q_OBJECT
public:
    explicit FinishTray(QWidget *parent, EditorBus *bus);
    // The registry's `ai.ask` and `qc.run`, which Ask and Run QC follow:
    // enabled with them, their tips the registry's (shortcut, reason).
    // Until they are set both are disabled.
    void setActions(QAction *ask, QAction *runQc);
    void setLoudness(const QJsonObject &loudness);
    void setCheck(const QJsonObject &check);
    // Bring a pane forward with the accent outline for a moment:
    // "waveform", "vectorscope", "parade", "loudness", "qc".
    void highlightPane(const QString &id);
    QString highlightedPane() const { return m_highlighted; }
    QWidget *pane(const QString &id) const { return m_panes.value(id); }
    LoudnessView *loudnessView() const { return m_loudness; }
    QcView *qcView() const { return m_qc; }
    QPushButton *runQcButton() const { return m_runQc; }
    QToolButton *askButton() const { return m_ask; }
    QString scopesText() const;

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    void updateBar();
    void followActions();
    EditorBus *m_bus = nullptr;
    QHash<QString, QWidget *> m_panes;
    QString m_highlighted;
    LoudnessView *m_loudness = nullptr;
    QcView *m_qc = nullptr;
    QLabel *m_scopes = nullptr;
    QLabel *m_targets = nullptr;
    QLabel *m_qcMeta = nullptr;
    QToolButton *m_ask = nullptr;
    QPushButton *m_runQc = nullptr;
    QPointer<QAction> m_askAction;
    QPointer<QAction> m_runQcAction;
};

} // namespace kerf
