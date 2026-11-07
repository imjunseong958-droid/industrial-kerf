#pragma once

// The Finish tray (mockup-spec 12): in the Finish workspace it takes the
// whole bottom band. A 28-point bar — "Scopes" and what they measure
// (Program · timecode · the clip under the playhead · its colour space when
// the file says), Ask, the R128 targets QC checks against, Run QC — over
// five panes 8 apart: Waveform, Vectorscope, RGB parade, Loudness and QC.
// Loudness and the vectorscope are 240 wide; the waveform, the parade and
// QC share the rest 352 : 352 : 368.
//
// The scopes are measured from the program frames the viewer shows
// (EditorBus::programFrameChanged), off the GUI thread, at most ten times a
// second and only while the tray is in sight (ScopeAnalyser). Loudness and
// QC are live from the bridge's `loudness`, `meter` and `check` events; Ask
// runs `ai.ask` and Run QC `qc.run`, each enabled and explained as its
// registry action is.

#include "scopes.h"

#include <QHash>
#include <QJsonObject>
#include <QPointer>
#include <QSet>
#include <QString>
#include <QWidget>

#include <array>

class QAction;
class QLabel;
class QTimer;

namespace kerf {

class EditorBus;
class LoudnessView;
class QcView;

namespace ui {
class Button;
class GhostButton;
} // namespace ui

class FinishTray final : public QWidget {
    Q_OBJECT
public:
    // How long a pane asked for keeps its accent outline.
    static constexpr int highlightMs = 1200;

    explicit FinishTray(QWidget *parent, EditorBus *bus);
    // The registry's `ai.ask` and `qc.run`, which Ask and Run QC follow:
    // enabled with them, their tips the registry's (shortcut, reason).
    // Until they are set both are disabled.
    void setActions(QAction *ask, QAction *runQc);
    void setLoudness(const QJsonObject &loudness);
    void setCheck(const QJsonObject &check);
    // Bring a pane forward with the accent outline for highlightMs:
    // "waveform", "vectorscope", "parade", "loudness", "qc", or "scopes"
    // (the three scope panes).
    void highlightPane(const QString &id);
    QString highlightedPane() const { return m_highlighted; }
    QWidget *pane(const QString &id) const { return m_panes.value(id); }
    ScopePlot *plot(const QString &id) const { return m_plots.value(id); }
    ScopeAnalyser *analyser() const { return m_analyser; }
    LoudnessView *loudnessView() const { return m_loudness; }
    QcView *qcView() const { return m_qc; }
    ui::Button *runQcButton() const { return m_runQc; }
    ui::GhostButton *askButton() const { return m_ask; }
    ui::GhostButton *waveformModeButton() const { return m_waveformMode; }
    QString scopesText() const;
    QString targetsText() const;
    QString paneMeta(const QString &id) const;
    // The waveform pane: Luma, or R′G′B′ overlaid (kept in the settings).
    WaveformMode waveformMode() const { return m_mode; }
    void setWaveformMode(WaveformMode mode);

    // Where the panes go in a tray `width` wide, in order (waveform,
    // vectorscope, parade, loudness, qc): x and width.
    struct Span {
        int x;
        int width;
    };
    static std::array<Span, 5> paneSpans(int width);

    QSize sizeHint() const override;
    QSize minimumSizeHint() const override;

protected:
    void paintEvent(QPaintEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;
    void showEvent(QShowEvent *event) override;
    void hideEvent(QHideEvent *event) override;

private:
    void mergeMediaInfo(const QJsonObject &media);
    void updateBar();
    void updateScopeMeta();
    void askForSignal();
    void followActions();
    void placePanes();
    void pushLayout();
    void showWaveformMenu();
    void applyWaveformMode(WaveformMode mode);
    EditorBus *m_bus = nullptr;
    ScopeAnalyser *m_analyser = nullptr;
    QWidget *m_bar = nullptr;
    QHash<QString, QWidget *> m_panes;
    QHash<QString, ScopePlot *> m_plots;
    QStringList m_order;
    QString m_highlighted;
    QTimer *m_highlightTimer = nullptr;
    LoudnessView *m_loudness = nullptr;
    QcView *m_qc = nullptr;
    QLabel *m_scopes = nullptr;
    QLabel *m_targets = nullptr;
    QLabel *m_qcMeta = nullptr;
    QLabel *m_qcStandards = nullptr;
    QLabel *m_waveMeta = nullptr;
    QLabel *m_paradeMeta = nullptr;
    QLabel *m_vectorMeta = nullptr;
    ui::GhostButton *m_ask = nullptr;
    ui::GhostButton *m_waveformMode = nullptr;
    ui::Button *m_runQc = nullptr;
    QPointer<QAction> m_askAction;
    QPointer<QAction> m_runQcAction;
    // Every media_info the bridge sent, by media id: it sends them in
    // pieces (new media, one id asked for), so the last event alone does
    // not know them all.
    QHash<QString, QJsonObject> m_mediaInfo;
    WaveformMode m_mode = WaveformMode::Luma;
};

} // namespace kerf
