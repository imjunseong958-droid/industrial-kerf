#pragma once

// The Loudness pane of the Finish and Deliver trays (mockup-spec 12): the
// programme measured to EBU R128 by the bridge's `loudness` event (plan
// section 3: integrated, LRA, true peak per channel, the momentary and
// short-term series), and the momentary and short-term loudness of the
// program as it plays (its `meter`).
//
//   Stats      M and S: the meter's while it reports them (playing), else
//              the measured series at the playhead; I and LRA measured.
//   History    the short-term polyline over the programme, the dashed
//              target, the playhead.
//   Legend     "Short-term, <programme length>" ... the target.
//   True peak  a row per channel against the limit, any overshoot in neg.
//
// Nothing shows a number the engine has not measured: an unmeasured value is
// a dash. Before a measurement (or after a failed one) the history holds
// Measure, which sends loudness{}; while it runs it says so, with the
// bridge's progress; a connection that ends or begins ends the measurement
// it was waiting on; a measurement older than the document's revision
// offers to measure again.

#include <QJsonObject>
#include <QList>
#include <QPolygonF>
#include <QRectF>
#include <QWidget>

#include <optional>

class QLabel;

namespace kerf {

class EditorBus;

namespace ui {
class Button;
class GhostButton;
} // namespace ui

class LoudnessView final : public QWidget {
    Q_OBJECT
public:
    explicit LoudnessView(QWidget *parent, EditorBus *bus);
    // A `loudness` event: {status, integrated, lra, truePeak:[l,r], series, …}.
    void setLoudness(const QJsonObject &loudness);
    QJsonObject loudness() const { return m_loudness; }
    // The programme has been measured (status ready).
    bool measured() const;
    // Measured, but at an older revision of the document than the bus's.
    bool stale() const;
    // The text of a statistic: "M", "S", "I", "LRA", "TP L", "TP R".
    QString statText(const QString &key) const;
    // Under Measure (or while measuring): what the history holds instead of
    // a curve; empty while the curve shows.
    QString statusText() const;
    ui::Button *measureButton() const { return m_measure; }
    ui::GhostButton *measureAgainButton() const { return m_again; }

    // ---- geometry (widget coordinates), for painting and tests ------------
    // The history well (120 tall), and y in its 208 x 120 drawing for a
    // loudness in LUFS: 6 + (−14 − v) / 22 · 108, the value kept in −36…−14.
    QRectF historyRect() const;
    static double historyY(double lufs);
    // The short-term curve as drawn: a polyline per run of readings, broken
    // where the short-term loudness is null (silence, the first 3 s). Built
    // when the measurement or the size changes, not on every paint.
    QList<QPolygonF> historySegments() const;
    // The target and playhead lines as drawn.
    double targetLineY() const;
    std::optional<double> playheadX() const;
    // A true-peak row's bar (channel 0 is L), its fill to min(value, limit),
    // its overshoot past the limit (empty without one), and its limit tick.
    QRectF truePeakBar(int channel) const;
    QRectF truePeakFill(int channel) const;
    QRectF truePeakOvershoot(int channel) const;
    QRectF truePeakTick(int channel) const;
    // The legend's left text: "Short-term, 30 s".
    QString legendText() const;

protected:
    void paintEvent(QPaintEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;

private:
    void setMeter(const QJsonObject &meter);
    void connectionChanged();
    const QList<QPolygonF> &segments() const;
    void refresh();
    void placeStatus();
    void measure();
    double limit() const;
    double target() const;
    double programmeDuration() const;
    std::optional<double> truePeak(int channel) const;
    std::optional<double> seriesAt(double seconds, int column) const;
    QString formatStat(const QString &key) const;
    double barScale(double dbtp) const;
    QRectF contentRect() const;

    EditorBus *m_bus = nullptr;
    QJsonObject m_loudness;
    // The meter's latest momentary and short-term readings (null: none).
    std::optional<double> m_meterM;
    std::optional<double> m_meterS;
    // The document revision the shown measurement was made at, and the one
    // the measurement asked for last was asked at.
    quint64 m_measuredRevision = 0;
    quint64 m_requestedRevision = 0;
    bool m_requested = false;
    QString m_sendError;
    ui::Button *m_measure = nullptr;
    ui::GhostButton *m_again = nullptr;
    QLabel *m_status = nullptr;
    // The curve, and the size it was built for (invalid: build it again).
    mutable QList<QPolygonF> m_segments;
    mutable QSize m_segmentsSize;
};

} // namespace kerf
