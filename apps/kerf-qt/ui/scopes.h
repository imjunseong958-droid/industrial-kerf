#pragma once

// The Finish tray's scopes (mockup-spec 12, engine gap 7): the waveform, the
// vectorscope and the RGB parade of the program frame the viewer shows
// (EditorBus::programFrameChanged, the RGBA kerf_bridge_frame hands over).
//
// ScopeAnalyser measures frames off the GUI thread, in a QThreadPool task,
// at most ten times a second and only while it is active (the tray is in
// sight): a frame that arrives sooner waits, and a newer one replaces it.
// Each analysis turns the frame into the additive traces each plot paints
// (scope_math does the counting); ScopePlot paints a plot's well, graticule,
// labels and its trace, or says honestly that there is no frame yet.

#include "design_tokens.h"

#include <QElapsedTimer>
#include <QImage>
#include <QObject>
#include <QPointer>
#include <QRectF>
#include <QSize>
#include <QThreadPool>
#include <QTimer>
#include <QWidget>

#include <array>

namespace kerf {

// What the waveform pane traces: Y′ alone (Luma), or R′, G′ and B′ over one
// another in the parade's colours (RGB overlay).
enum class WaveformMode { Luma, Rgb };

// The plots an analysis draws for, in points, and the screen's pixel ratio
// (the traces are counted at device pixels, capped at 2).
struct ScopeLayout {
    QSize waveform;
    QSize vectorscope;
    QSize parade;
    qreal dpr = 1;
    WaveformMode mode = WaveformMode::Luma;
    // The frame is an HDR sequence's signal in BT.2020 on this curve: 1 for PQ, 2 for HLG; 0 for SDR.
    int curve = 0;
    bool operator==(const ScopeLayout &other) const {
        return waveform == other.waveform && vectorscope == other.vectorscope && parade == other.parade && qFuzzyCompare(dpr, other.dpr) &&
               mode == other.mode && curve == other.curve;
    }
};

// One analysis: each plot's trace, premultiplied, its device pixel ratio set,
// to be drawn with QPainter::CompositionMode_Plus over the plot's well.
struct ScopeTraces {
    QImage waveform;               // over the waveform's graticule (x0..x1, top..bottom)
    std::array<QImage, 3> parade;  // over each of the parade's R, G and B cells
    QImage vectorscope;            // over the whole vectorscope plot
    int bitDepth = 0;              // bits per channel of the frame measured (8 for RGBA8)
    QSize frameSize;               // the program frame measured
    QSize sampled;                 // the grid it was sampled at
    WaveformMode mode = WaveformMode::Luma;
    int curve = 0;                 // the signal measured, as in ScopeLayout
    bool isNull() const { return frameSize.isEmpty(); }
};

namespace scopes {

// The scope colours of mockup-spec 12.
constexpr QRgb lumaTrace = qRgba(208, 208, 208, tokens::alpha(0.09));
constexpr QRgb vectorscopeTrace = qRgba(208, 208, 208, tokens::alpha(0.18));
constexpr QRgb redTrace = qRgba(212, 106, 106, tokens::alpha(0.12));
constexpr QRgb greenTrace = qRgba(106, 196, 138, tokens::alpha(0.12));
constexpr QRgb blueTrace = qRgba(106, 143, 212, tokens::alpha(0.12));
constexpr QRgb graticule = tokens::track; // lines, circles, crosshair (#3a3a3a)
constexpr QRgb targetBox = tokens::dis;     // the 75% boxes (#5c5c5c)
constexpr double traceGain = 2.2;  // the vectorscope trace
constexpr double targetGain = 1.0; // the 75% targets
constexpr int cellGap = 6;         // between parade cells

// Where SDR white falls in a signal on `curve`, as a 10-bit code: 203 cd/m² on PQ, 75% on HLG.
constexpr int referenceWhiteCode(int curve) { return curve == 1 ? 594 : 767; }

// Bits per channel of a frame as it is stored: 8 for RGBA8 (what the bridge
// hands over), 10 for the 10-bit formats, 16 for RGBA64; 0 for none.
int bitDepth(const QImage &frame);

// The frame the scopes sample: the design measures a 160 x 90 grab, so a
// frame is scaled (keeping its aspect) to about that many pixels per point
// of plot, times dpr squared, and so each device pixel of a trace gets as
// many hits as the design's.
QSize sampleSize(QSize frame, qreal dpr);

// The whole analysis, a pure function: any thread may call it.
ScopeTraces analyse(const QImage &frame, const ScopeLayout &layout);

} // namespace scopes

class ScopeAnalyser final : public QObject {
    Q_OBJECT
public:
    // The least time between two analyses: ten a second at most.
    static constexpr int intervalMs = 100;

    explicit ScopeAnalyser(QObject *parent = nullptr);
    // Waits for an analysis still running.
    ~ScopeAnalyser() override;

    // Frames are measured only while active (the Finish tray in sight). A
    // frame that came while inactive is measured when it becomes active.
    void setActive(bool active);
    bool isActive() const { return m_active; }
    // The plots to draw for; the last frame is measured again for them.
    void setLayout(const ScopeLayout &layout);
    ScopeLayout layout() const { return m_layout; }
    // A program frame: measured now when the last analysis started at least
    // intervalMs ago and none runs, otherwise when that time is up (only the
    // newest frame waiting is measured).
    void setFrame(const QImage &frame);
    // An HDR sequence's frame as its signal: from now on what is measured, the frames a monitor gets
    // left out, until clearSignal. 1 for PQ, 2 for HLG.
    void setSignal(const QImage &frame, int curve);
    void clearSignal();
    // The newest analysis (null before the first).
    const ScopeTraces &traces() const { return m_traces; }
    // Analyses started so far, and whether one runs now.
    int started() const { return m_started; }
    bool running() const { return m_running; }
    // A frame waits to be measured.
    bool pending() const { return m_dirty; }

signals:
    void tracesChanged();

private:
    void schedule();
    void start();
    void finish(const ScopeTraces &traces);

    QThreadPool m_pool;
    QTimer m_timer;
    QElapsedTimer m_since;
    ScopeLayout m_layout;
    // The last frame a monitor gets, which an HDR signal stands in for.
    QImage m_monitorFrame;
    ScopeTraces m_traces;
    QImage m_frame;
    bool m_dirty = false;
    bool m_running = false;
    bool m_active = false;
    int m_started = 0;
};

// One scope's plot (the body of its tray pane): the well inset 4 from the
// pane's sides, the graticule and labels, then the trace with Plus.
class ScopePlot final : public QWidget {
    Q_OBJECT
public:
    enum class Kind { Waveform, Vectorscope, Parade };
    ScopePlot(Kind kind, ScopeAnalyser *analyser, QWidget *parent = nullptr);
    Kind kind() const { return m_kind; }
    // The well, in this widget's coordinates (its size is the plot's).
    QRectF plotRect() const;
    QSize plotSize() const;
    // What the plot shows while there is no trace: "No program frame".
    QString emptyText() const;

signals:
    void plotResized();

protected:
    void paintEvent(QPaintEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;

private:
    void paintWaveformGraticule(QPainter &painter, const QRectF &plot, bool parade) const;
    void paintVectorscopeGraticule(QPainter &painter, const QRectF &plot) const;
    Kind m_kind;
    QPointer<ScopeAnalyser> m_analyser;
};

} // namespace kerf
