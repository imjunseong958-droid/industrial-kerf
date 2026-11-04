// Tests for the Finish tray (mockup-spec 12): the scopes measured from
// program frames, the R128 loudness pane, the QC pane, and the tray's bar
// and pane geometry.

#include "fixtures.h"
#include "test_registry.h"

#include "command_registry.h"
#include "components.h"
#include "design_tokens.h"
#include "editor_bus.h"
#include "finish_tray.h"
#include "loudness_view.h"
#include "main_window.h"
#include "qc_view.h"
#include "scope_math.h"
#include "scopes.h"
#include "tray_shell.h"
#include "ui/stage_shell.h"

#include <QAction>
#include <QElapsedTimer>
#include <QJsonArray>
#include <QLabel>
#include <QObject>
#include <QScopeGuard>
#include <QSettings>
#include <QSignalSpy>
#include <QTest>
#include <QToolButton>

#include <cmath>

namespace kerf {

namespace {

using namespace kerf::test;

QImage solid(QSize size, QRgb rgb) {
    QImage image(size, QImage::Format_RGBA8888);
    image.fill(QColor::fromRgb(rgb));
    return image;
}

// Black on the left half, white on the right.
QImage halves(QSize size) {
    QImage image = solid(size, qRgb(0, 0, 0));
    for (int y = 0; y < size.height(); ++y)
        for (int x = size.width() / 2; x < size.width(); ++x) image.setPixel(x, y, qRgba(255, 255, 255, 255));
    return image;
}

// The mockup's plots: waveform and parade 344 x 268, vectorscope 232 x 268.
ScopeLayout designLayout(WaveformMode mode = WaveformMode::Luma) {
    ScopeLayout layout;
    layout.waveform = QSize(344, 268);
    layout.vectorscope = QSize(232, 268);
    layout.parade = QSize(344, 268);
    layout.dpr = 1;
    layout.mode = mode;
    return layout;
}

// The mockup's eight QC rows (mockup-spec 12), as kerf_mcp's FULL_ROWS sends them.
QJsonObject mockupCheck() {
    const auto row = [](const char *code, const char *name, bool passed, const char *value, const char *limit = nullptr,
                        const char *thread = nullptr) {
        QJsonObject out{{"code", code}, {"name", name}, {"passed", passed}, {"value", QString::fromUtf8(value)}, {"count", passed ? 0 : 1},
                        {"blocking", 0}};
        if (limit) out.insert("limit", QString::fromUtf8(limit));
        if (thread) out.insert("commentId", thread);
        return out;
    };
    return QJsonObject{{"type", "check"}, {"status", "done"}, {"valid", false},
                       {"summary", QJsonObject{{"passed", 6}, {"total", 8}, {"blocking", 1}, {"rows", QJsonArray{
                           row("loudness.integrated", "Integrated loudness", true, "−23.1 LUFS", "±0.5"),
                           row("loudness.true_peak", "True peak", false, "−0.4 dBTP", "≤ −1.0", "c-2718"),
                           row("loudness.range", "Loudness range", true, "6.2 LU", "≤ 20"),
                           row("picture.out_of_range", "Gamut Rec.709", true, "legal"),
                           row("picture.luma_levels", "Luma levels", true, "Y′ 64–940"),
                           row("text.safe_area", "Title safe 90%", true, "0 outside"),
                           row("text.overlap", "Caption overlap", false, "1 at 00:00:17:08", nullptr, "c-1708"),
                           row("picture.flash_black", "Flash and black", true, "0 · 0")}}}}};
}

// A programme of 30 s measured every 0.1 s: short-term −25.5 dB with
// the mockup's swells; at 9.1 s M is −17.8 and S −20.4. S is null for
// the first 3 s, as BS.1770 leaves it.
QJsonObject mockupLoudness() {
    QJsonArray series;
    for (int i = 0; i <= 300; ++i) {
        const double t = i / 10.0;
        double s = -25.5 + 2.2 * std::sin(t / 3.1) + (t > 2 && t < 12.4 ? 2.4 : 0) + (t > 14 && t < 24 ? 2.2 : 0);
        double m = s + 1;
        if (i == 91) {
            m = -17.8;
            s = -20.4;
        }
        series.append(QJsonArray{t, m, t < 3 ? QJsonValue() : QJsonValue(s)});
    }
    return QJsonObject{{"type", "loudness"}, {"status", "ready"}, {"integrated", -23.1}, {"lra", 6.2},
                       {"truePeak", QJsonArray{-0.4, -1.3}}, {"truePeakMax", -0.4}, {"shortTermMax", -14.9},
                       {"momentaryMax", -12.0}, {"series", series}, {"target", -23}, {"limit", -1}, {"duration", 30.0}};
}

QRgb pixelAt(const QImage &image, const QPointF &at) {
    return image.pixel(QPoint(int(at.x()), int(at.y())));
}

} // namespace

class FinishTest final : public QObject {
    Q_OBJECT

private slots:
    // Each test starts with the tray's preferences as a new user has them.
    void init() {
        QSettings().remove(QStringLiteral("viewer"));
        QSettings().remove(QStringLiteral("finish"));
    }

    // ---- scopes ------------------------------------------------------------

    // A frame half black and half white puts every waveform sample on the
    // bottom (code 0) or top (1023) line, in its half, two per sampled
    // pixel; the analysis draws exactly scope_math's density.
    void waveformOfASyntheticFrameMatchesScopeMath() {
        const QImage frame = halves(QSize(1920, 1080));
        QCOMPARE(scopes::sampleSize(frame.size(), 1), QSize(160, 90));
        QCOMPARE(scopes::sampleSize(frame.size(), 2), QSize(320, 180));
        const ScopeTraces traces = scopes::analyse(frame, designLayout());
        QCOMPARE(traces.bitDepth, 8);
        QCOMPARE(traces.frameSize, QSize(1920, 1080));
        QCOMPARE(traces.sampled, QSize(160, 90));
        // The trace covers the graticule, x0 34 … W − 6, y 8 … H − 10.
        QCOMPARE(traces.waveform.size(), QSize(304, 251));

        const QImage source = scope::scopeSource(frame, traces.sampled);
        const scope::Density density = scope::waveformLuma(source, 304, 251);
        QCOMPARE(density.total, quint64(2 * 160 * 90));
        quint64 bottomLeft = 0, topRight = 0;
        for (int y = 0; y < density.height; ++y)
            for (int x = 0; x < density.width; ++x) {
                const quint32 n = density.at(x, y);
                if (!n) continue;
                // No sample strays from its line by more than the jitter.
                QVERIFY2(y <= 4 || y >= density.height - 5, qPrintable(QStringLiteral("%1,%2").arg(x).arg(y)));
                if (x < density.width / 2 - 2) QVERIFY(y >= density.height - 5);
                if (x > density.width / 2 + 2) QVERIFY(y <= 4);
                (y > density.height / 2 ? bottomLeft : topRight) += n;
            }
        QCOMPARE(bottomLeft, quint64(160 * 90));
        QCOMPARE(topRight, quint64(160 * 90));
        const QImage expected = scope::render(density, scopes::lumaTrace);
        for (int y = 0; y < expected.height(); ++y)
            for (int x = 0; x < expected.width(); ++x)
                if (traces.waveform.pixel(x, y) != expected.pixel(x, y)) QFAIL(qPrintable(QStringLiteral("trace differs at %1,%2").arg(x).arg(y)));
        // The densest cells saturate to the trace colour's channels, added.
        QVERIFY(qRed(traces.waveform.pixel(40, 250)) > 0);
        QCOMPARE(qRed(traces.waveform.pixel(150, 120)), 0);

        // The parade: each cell its channel, (304 − 2 · 6) / 3 wide.
        QCOMPARE(traces.parade[0].size(), QSize(97, 251));
        for (const QImage &cell : traces.parade) QVERIFY(qRed(cell.pixel(80, 0)) + qGreen(cell.pixel(80, 0)) + qBlue(cell.pixel(80, 0)) > 0);
        // A pure red frame leaves the green and blue cells on code 0.
        const ScopeTraces red = scopes::analyse(solid(QSize(64, 36), qRgb(255, 0, 0)), designLayout());
        QVERIFY(qAlpha(red.parade[0].pixel(50, 0)) > 0);
        QCOMPARE(qAlpha(red.parade[0].pixel(50, 250)), 0);
        QVERIFY(qAlpha(red.parade[1].pixel(50, 250)) > 0);
        QCOMPARE(qAlpha(red.parade[1].pixel(50, 0)), 0);
        // RGB overlay draws all three over the waveform's width.
        const ScopeTraces overlay = scopes::analyse(solid(QSize(64, 36), qRgb(255, 0, 0)), designLayout(WaveformMode::Rgb));
        QCOMPARE(overlay.waveform.size(), QSize(304, 251));
        QVERIFY(qRed(overlay.waveform.pixel(150, 0)) > qGreen(overlay.waveform.pixel(150, 0)));
        QVERIFY(qAlpha(overlay.waveform.pixel(150, 250)) > 0);
    }

    // Neutral grey lands every vectorscope sample at the centre; a tint
    // lands at its Rec.709 Cb/Cr times 2.2, as scope_math places it.
    void vectorscopeOfASyntheticFrameMatchesScopeMath() {
        const QSize plot(232, 268);
        const ScopeTraces grey = scopes::analyse(solid(QSize(160, 90), qRgb(128, 128, 128)), designLayout());
        QCOMPARE(grey.vectorscope.size(), plot);
        const scope::Density density =
            scope::vectorscope(solid(QSize(160, 90), qRgb(128, 128, 128)), plot, scopes::traceGain);
        QCOMPARE(density.total, quint64(2 * 160 * 90));
        const QPointF centre = scope::vectorscopeGeometry(QSizeF(plot)).centre;
        for (int y = 0; y < density.height; ++y)
            for (int x = 0; x < density.width; ++x)
                if (density.at(x, y)) QVERIFY(std::hypot(x + 0.5 - centre.x(), y + 0.5 - centre.y()) < 4);
        QVERIFY(qAlpha(pixelAt(grey.vectorscope, centre)) > 200);

        const QRgb tint = qRgb(150, 110, 100);
        const ScopeTraces warm = scopes::analyse(solid(QSize(160, 90), tint), designLayout());
        const QPointF at = scope::vectorscopeGeometry(QSizeF(plot)).point(scope::chroma(qRed(tint), qGreen(tint), qBlue(tint)), 2.2);
        QVERIFY(at.x() < centre.x() && at.y() < centre.y()); // warm: Cb below zero, Cr above
        QVERIFY(qAlpha(pixelAt(warm.vectorscope, at)) > 200);
        QCOMPARE(qAlpha(pixelAt(warm.vectorscope, centre)), 0);
        // The 75% targets stay at gain 1: the red box sits on the red bar.
        const auto marks = scope::targets(QSizeF(plot), scopes::targetGain);
        QCOMPARE(QString::fromLatin1(marks[0].label), QStringLiteral("R"));
        const QPointF red = scope::vectorscopeGeometry(QSizeF(plot)).point(scope::chroma(191, 0, 0), 1);
        QVERIFY(std::hypot(marks[0].centre.x() - red.x(), marks[0].centre.y() - red.y()) < 0.5);
        // The meta: the real bit depth of what was measured.
        QCOMPARE(scopes::bitDepth(solid(QSize(4, 4), qRgb(1, 2, 3))), 8);
        QCOMPARE(scopes::bitDepth(QImage(4, 4, QImage::Format_RGBA64)), 16);
        QCOMPARE(scopes::bitDepth(QImage()), 0);
    }

    // The tray measures program frames at most ten times a second, only the
    // newest of those that wait, and not at all while it is hidden.
    void scopesAreThrottledAndStopWhenHidden() {
        EditorBus bus;
        FinishTray tray(nullptr, &bus);
        tray.resize(1600, 344);
        ScopeAnalyser *analyser = tray.analyser();
        QVERIFY(!analyser->isActive());
        // Before it is shown nothing is measured, and the plots say so.
        bus.setProgramFrame(halves(QSize(320, 180)));
        QTest::qWait(150);
        QCOMPARE(analyser->started(), 0);
        QCOMPARE(tray.plot(QStringLiteral("waveform"))->emptyText(), QStringLiteral("No program frame"));
        tray.show();
        QVERIFY(QTest::qWaitForWindowExposed(&tray));
        QVERIFY(analyser->isActive());
        QTRY_VERIFY(!analyser->traces().isNull());
        QTRY_VERIFY(!analyser->running() && !analyser->pending());
        // The plots are the panes less 4 on each side and under the header.
        QCOMPARE(tray.plot(QStringLiteral("waveform"))->plotSize(), QSize(344, 268));
        QCOMPARE(tray.plot(QStringLiteral("vectorscope"))->plotSize(), QSize(232, 268));
        QCOMPARE(tray.plot(QStringLiteral("parade"))->plotSize(), QSize(344, 268));
        QCOMPARE(analyser->layout().waveform, QSize(344, 268));
        QCOMPARE(tray.paneMeta(QStringLiteral("waveform")), QStringLiteral("Y′ · 8-bit"));
        QCOMPARE(tray.paneMeta(QStringLiteral("parade")), QStringLiteral("8-bit"));
        QCOMPARE(tray.paneMeta(QStringLiteral("vectorscope")), QStringLiteral("75%"));
        QVERIFY(tray.plot(QStringLiteral("waveform"))->emptyText().isEmpty());

        // A burst of frames: one analysis now, then one for the newest.
        QTest::qWait(ScopeAnalyser::intervalMs + 20);
        const int base = analyser->started();
        QElapsedTimer clock;
        clock.start();
        for (int i = 0; i < 6; ++i) bus.setProgramFrame(solid(QSize(320, 180), qRgb(20 * i, 40, 60)));
        QCOMPARE(analyser->started(), base + 1);
        QTRY_COMPARE(analyser->started(), base + 2);
        QVERIFY2(clock.elapsed() >= ScopeAnalyser::intervalMs - 5, qPrintable(QString::number(clock.elapsed())));
        QTRY_VERIFY(!analyser->running());
        QTest::qWait(2 * ScopeAnalyser::intervalMs);
        QCOMPARE(analyser->started(), base + 2);
        // Frames streaming at 50 per second for half a second: about five
        // analyses, never more than one per 100 ms.
        const int before = analyser->started();
        clock.restart();
        while (clock.elapsed() < 500) {
            bus.setProgramFrame(solid(QSize(320, 180), qRgb(int(clock.elapsed() % 255), 40, 60)));
            QTest::qWait(20);
        }
        const int streamed = analyser->started() - before;
        QVERIFY2(streamed >= 2 && streamed <= 6, qPrintable(QString::number(streamed)));

        // Hidden, it stops: frames wait, and are measured when it shows again.
        QTRY_VERIFY(!analyser->running());
        tray.hide();
        QVERIFY(!analyser->isActive());
        const int hidden = analyser->started();
        for (int i = 0; i < 5; ++i) {
            bus.setProgramFrame(solid(QSize(320, 180), qRgb(200, 10 * i, 60)));
            QTest::qWait(40);
        }
        QTest::qWait(150);
        QCOMPARE(analyser->started(), hidden);
        QVERIFY(analyser->pending());
        tray.show();
        QTRY_COMPARE(analyser->started(), hidden + 1);

        // Choosing RGB overlay measures the frame again for it, and is kept.
        QTRY_VERIFY(!analyser->running());
        QTest::qWait(ScopeAnalyser::intervalMs + 20);
        const int beforeMode = analyser->started();
        tray.setWaveformMode(WaveformMode::Rgb);
        QCOMPARE(analyser->layout().mode, WaveformMode::Rgb);
        QTRY_COMPARE(analyser->started(), beforeMode + 1);
        QTRY_COMPARE(analyser->traces().mode, WaveformMode::Rgb);
        QCOMPARE(tray.waveformModeButton()->text(), QStringLiteral("RGB"));
        QCOMPARE(tray.paneMeta(QStringLiteral("waveform")), QStringLiteral("R′G′B′ · 8-bit"));
        FinishTray again(nullptr, &bus);
        QCOMPARE(again.waveformMode(), WaveformMode::Rgb);
    }

    // The plots paint their well, graticule and trace (Plus over the well).
    void scopePlotsPaintTheTrace() {
        EditorBus bus;
        FinishTray tray(nullptr, &bus);
        tray.resize(1600, 344);
        tray.show();
        QVERIFY(QTest::qWaitForWindowExposed(&tray));
        bus.setProgramFrame(halves(QSize(320, 180)));
        QTRY_VERIFY(!tray.analyser()->traces().isNull() && !tray.analyser()->running());
        const qreal dpr = std::clamp(tray.devicePixelRatioF(), 1.0, 2.0);
        QTRY_COMPARE(tray.analyser()->traces().waveform.size(), QSize(int(std::lround(304 * dpr)), int(std::lround(250 * dpr)) + 1));
        ScopePlot *waveform = tray.plot(QStringLiteral("waveform"));
        const QImage shot = waveform->grab().toImage();
        const qreal ratio = shot.devicePixelRatio();
        const auto at = [&](double x, double y) { return shot.pixel(QPoint(int(x * ratio), int(y * ratio))); };
        const QRectF plot = waveform->plotRect();
        // The well, away from the lines and the trace.
        QVERIFY(nearColor(at(plot.x() + 20, plot.y() + 100), tokens::well, 3));
        // The 512 line in the graticule colour.
        const double y512 = plot.y() + std::round(scope::waveformGeometry(plot.size()).y(512)) + 0.5;
        QVERIFY(nearColor(at(plot.x() + 200, y512 - 0.5), scopes::graticule, 3) || nearColor(at(plot.x() + 200, y512 + 0.5), scopes::graticule, 3));
        // The trace lightens the well on the white half's top line.
        QVERIFY(qGray(at(plot.x() + 300, plot.y() + 8)) > qGray(tokens::well) + 20);
    }

    // ---- loudness ----------------------------------------------------------

    // The pane from the mockup's measurement: the stats, the history's
    // mapping, the legend, and the true peak over the limit in neg.
    void loudnessPaneShowsTheMeasuredProgramme() {
        EditorBus bus;
        bus.setDuration(30);
        bus.setPlayhead(9.12);
        LoudnessView view(nullptr, &bus);
        view.resize(238, 268);
        QVERIFY(!view.measured());
        QCOMPARE(view.statText(QStringLiteral("I")), QStringLiteral("—"));
        QCOMPARE(view.statText(QStringLiteral("TP L")), QStringLiteral("— dBTP"));
        QCOMPARE(view.statusText(), QStringLiteral("Not measured yet"));
        QVERIFY(!view.measureButton()->isHidden());
        QVERIFY(view.historySegments().isEmpty());
        QCOMPARE(view.legendText(), QStringLiteral("Short-term"));

        bus.dispatch(mockupLoudness());
        QVERIFY(view.measured());
        QVERIFY(view.statusText().isEmpty());
        QVERIFY(view.measureButton()->isHidden());
        QCOMPARE(view.statText(QStringLiteral("I")), QStringLiteral("−23.1"));
        QCOMPARE(view.statText(QStringLiteral("LRA")), QStringLiteral("6.2"));
        QCOMPARE(view.statText(QStringLiteral("TP L")), QStringLiteral("−0.4 dBTP"));
        QCOMPARE(view.statText(QStringLiteral("TP R")), QStringLiteral("−1.3 dBTP"));
        // Paused: M and S are the measured series at the playhead.
        QCOMPARE(view.statText(QStringLiteral("M")), QStringLiteral("−17.8"));
        QCOMPARE(view.statText(QStringLiteral("S")), QStringLiteral("−20.4"));
        // Playing: the meter's own readings, then back to the series when it stops.
        bus.dispatch(QJsonObject{{"type", "meter"}, {"momentary", -16.04}, {"shortTerm", -19.46}, {"at", 9.2}});
        QCOMPARE(view.statText(QStringLiteral("M")), QStringLiteral("−16.0"));
        QCOMPARE(view.statText(QStringLiteral("S")), QStringLiteral("−19.5"));
        bus.dispatch(QJsonObject{{"type", "meter"}, {"momentary", QJsonValue()}, {"shortTerm", QJsonValue()}, {"at", QJsonValue()}});
        QCOMPARE(view.statText(QStringLiteral("M")), QStringLiteral("−17.8"));
        bus.setPlayhead(1.0);
        QCOMPARE(view.statText(QStringLiteral("S")), QStringLiteral("—")); // no short-term in the first 3 s
        bus.setPlayhead(9.12);

        // History: 216 wide at 1600 (the pane's 240 less 12 each side), 120 tall, 8 under the stats.
        QCOMPARE(view.historyRect(), QRectF(11, 48, 216, 120));
        QVERIFY(std::abs(LoudnessView::historyY(-23) - (6 + 9.0 / 22 * 108)) < 1e-9);
        QCOMPARE(LoudnessView::historyY(-14), 6.0);
        QCOMPARE(LoudnessView::historyY(-50), 114.0);
        QVERIFY(std::abs(view.targetLineY() - (48 + 6 + 9.0 / 22 * 108)) < 1e-9);
        const double sx = 216.0 / 208;
        QVERIFY(view.playheadX().has_value());
        QVERIFY(std::abs(*view.playheadX() - (11 + (8 + 9.12 / 30 * 192) * sx)) < 1e-9);
        QCOMPARE(view.historySegments().size(), 1);
        const QPolygonF curve = view.historySegments().first();
        QCOMPARE(curve.size(), 271); // 3.0 … 30.0 s; null short-term left out
        QVERIFY(std::abs(curve.first().x() - (11 + (8 + 3.0 / 30 * 192) * sx)) < 1e-9);
        QVERIFY(std::abs(curve.last().x() - (11 + 200 * sx)) < 1e-9);
        // Silence breaks the curve: 10.0–12.0 s null leaves two runs.
        QJsonObject gapped = mockupLoudness();
        QJsonArray series = gapped.value("series").toArray();
        for (int i = 100; i <= 120; ++i) {
            QJsonArray reading = series.at(i).toArray();
            reading[2] = QJsonValue();
            series[i] = reading;
        }
        gapped["series"] = series;
        bus.dispatch(gapped);
        QCOMPARE(view.historySegments().size(), 2);
        QCOMPARE(view.historySegments().at(0).size(), 70); // 3.0 … 9.9 s
        QCOMPARE(view.historySegments().at(1).size(), 180); // 12.1 … 30.0 s
        QVERIFY(view.historySegments().at(0).last().x() < view.historySegments().at(1).first().x());
        // The curve is built again for a new size, not on every paint.
        view.resize(298, 268);
        QVERIFY(std::abs(view.historySegments().at(1).last().x() - (11 + 200 * 276.0 / 208)) < 1e-9);
        view.resize(238, 268);
        bus.dispatch(mockupLoudness());
        QCOMPARE(view.historySegments().size(), 1);
        QCOMPARE(view.legendText(), QStringLiteral("Short-term, 30 s"));

        // True peak: L is 0.6 dB over the −1 limit, R under it.
        const QRectF bar = view.truePeakBar(0);
        QCOMPARE(bar.height(), 4.0);
        QCOMPARE(bar.x(), 11.0 + 12 + 8);
        QCOMPARE(bar.right(), 227.0 - 72 - 8);
        QVERIFY(std::abs(view.truePeakFill(0).width() - bar.width() * 11.0 / 12) < 1e-9);
        const QRectF over = view.truePeakOvershoot(0);
        QVERIFY(std::abs(over.x() - (bar.x() + bar.width() * 11.0 / 12)) < 1e-9);
        QVERIFY(std::abs(over.width() - bar.width() * 0.6 / 12) < 1e-9);
        QVERIFY(view.truePeakOvershoot(1).isEmpty());
        QVERIFY(std::abs(view.truePeakFill(1).width() - bar.width() * 10.7 / 12) < 1e-9);
        QCOMPARE(view.truePeakTick(0).size(), QSizeF(1, 12));

        const QImage shot = view.grab().toImage();
        const qreal ratio = shot.devicePixelRatio();
        const auto at = [&](const QPointF &p) { return shot.pixel(QPoint(int(p.x() * ratio), int(p.y() * ratio))); };
        QVERIFY(nearColor(at(over.center()), tokens::neg, 6));
        QVERIFY(nearColor(at(QPointF(bar.x() + 20, bar.center().y())), tokens::track2, 6));
        QVERIFY(nearColor(at(QPointF(view.truePeakBar(1).right() - 2, view.truePeakBar(1).center().y())), tokens::track, 6));
        TextRecorder recorder;
        view.render(&recorder);
        QCOMPARE(recorder.colorOf(QStringLiteral("−0.4 dBTP")), tokens::color(tokens::neg));
        QCOMPARE(recorder.colorOf(QStringLiteral("−1.3 dBTP")), tokens::color(tokens::ink2));
        QCOMPARE(recorder.colorOf(QStringLiteral("−23.1")), tokens::color(tokens::ink));
        QCOMPARE(recorder.colorOf(QStringLiteral("LRA LU")), tokens::color(tokens::ink3));
        QCOMPARE(recorder.colorOf(QStringLiteral("−23 LUFS")), tokens::color(tokens::ink3));
    }

    // Measure sends loudness{}; running, failed and stale states say so.
    void loudnessMeasureRunsTheBridgeCommand() {
        EditorBus bus;
        QList<QJsonObject> sent;
        bool online = false;
        bus.setSender([&](const QJsonObject &command) {
            if (online) sent << command;
            return online;
        });
        LoudnessView view(nullptr, &bus);
        view.resize(238, 268);
        view.measureButton()->click();
        QVERIFY(sent.isEmpty());
        QCOMPARE(view.statusText(), QStringLiteral("Kerf is not connected"));
        online = true;
        view.measureButton()->click();
        QCOMPARE(sent.size(), 1);
        QCOMPARE(sent.first(), (QJsonObject{{"type", "loudness"}}));
        QCOMPARE(view.statusText(), QStringLiteral("Measuring the programme…"));
        QVERIFY(view.measureButton()->isHidden());
        bus.dispatch(QJsonObject{{"type", "loudness"}, {"status", "running"}});
        QCOMPARE(view.statusText(), QStringLiteral("Measuring the programme…"));
        // The bridge's progress, once it sends it.
        bus.dispatch(QJsonObject{{"type", "loudness"}, {"status", "running"}, {"progress", 0.37}, {"revision", 0}});
        QCOMPARE(view.statusText(), QStringLiteral("Measuring the programme… 37%"));
        QVERIFY(view.measureButton()->isHidden());
        // The connection goes mid-run: its last event will not come, so
        // the view stops waiting and offers Measure again.
        bus.dispatch(QJsonObject{{"type", "connection"}, {"status", "disconnected"}, {"message", "kerfd exited"}});
        QCOMPARE(view.statusText(), QStringLiteral("Not measured yet"));
        QVERIFY(!view.measureButton()->isHidden());
        // So does a request no event has answered yet when a new connection comes.
        view.measureButton()->click();
        QCOMPARE(view.statusText(), QStringLiteral("Measuring the programme…"));
        bus.dispatch(QJsonObject{{"type", "connection"}, {"status", "ready"}});
        QCOMPARE(view.statusText(), QStringLiteral("Not measured yet"));
        QVERIFY(!view.measureButton()->isHidden());
        view.measureButton()->click();
        bus.dispatch(QJsonObject{{"type", "loudness"}, {"status", "running"}, {"progress", 0.0}, {"revision", 0}});
        bus.dispatch(QJsonObject{{"type", "loudness"}, {"status", "failed"}, {"message", "The sound could not be decoded"}});
        QCOMPARE(view.statusText(), QStringLiteral("The sound could not be decoded"));
        QVERIFY(!view.measureButton()->isHidden());
        bus.dispatch(QJsonObject{{"type", "error"}, {"command", "loudness"}, {"message", "Open a project first"}});
        QCOMPARE(view.statusText(), QStringLiteral("Open a project first"));
        // A result goes stale when the document moves on.
        bus.setSnapshot(QJsonObject{{"revision", 4}}, {}, 25, QSize(1920, 1080));
        view.measureButton()->click();
        bus.dispatch(mockupLoudness());
        QVERIFY(!view.stale());
        QVERIFY(view.measureAgainButton()->isHidden());
        bus.setSnapshot(QJsonObject{{"revision", 5}}, {}, 25, QSize(1920, 1080));
        QVERIFY(view.stale());
        QVERIFY(!view.measureAgainButton()->isHidden());
        sent.clear();
        view.measureAgainButton()->click();
        QCOMPARE(sent.size(), 1);
        // A view made later picks up the last measurement.
        LoudnessView later(nullptr, &bus);
        QVERIFY(later.measured());
        QCOMPARE(later.statText(QStringLiteral("I")), QStringLiteral("−23.1"));
    }

    // ---- QC ----------------------------------------------------------------

    // The rows of a fixture check: dot, name, value and limit (neg when
    // failing), In thread for a mirrored failure, which opens its thread.
    void qcPaneListsTheCheckRows() {
        EditorBus bus;
        QStringList ran;
        bus.setRunner([&](const QString &command, const QJsonObject &) {
            ran << command;
            return true;
        });
        QcView view(nullptr, &bus);
        view.resize(366, 268);
        QCOMPARE(view.rowCount(), 0);
        QCOMPARE(view.statusText(), QStringLiteral("QC has not run"));
        QVERIFY(!view.runButton()->isHidden() && view.runButton()->isEnabled());
        view.runButton()->click();
        QCOMPARE(ran, QStringList{QStringLiteral("qc.run")});

        bus.dispatch(QJsonObject{{"type", "check"}, {"status", "running"}, {"progress", 0.42}});
        QVERIFY(view.running());
        QCOMPARE(view.statusText(), QStringLiteral("Running QC… 42%"));
        QVERIFY(view.runButton()->isHidden());

        QSignalSpy threads(&bus, &EditorBus::openThread);
        bus.dispatch(mockupCheck());
        QVERIFY(!view.running());
        QCOMPARE(view.rowCount(), 8);
        QCOMPARE(view.summaryText(), QStringLiteral("6 of 8 pass"));
        QCOMPARE(view.standardsText(), QStringLiteral("R128 · Rec.709 · captions"));
        QVERIFY(view.statusText().isEmpty());
        QCOMPARE(view.rowName(0), QStringLiteral("Integrated loudness"));
        QCOMPARE(view.rowValue(0), QStringLiteral("−23.1 LUFS · ±0.5"));
        QVERIFY(view.rowPassed(0));
        QCOMPARE(view.rowValueColor(0), tokens::color(tokens::ink2));
        QCOMPARE(view.rowValue(1), QStringLiteral("−0.4 dBTP · ≤ −1.0"));
        QVERIFY(!view.rowPassed(1));
        QCOMPARE(view.rowValueColor(1), tokens::color(tokens::neg));
        QCOMPARE(view.rowThread(1), QStringLiteral("c-2718"));
        QCOMPARE(view.rowValue(6), QStringLiteral("1 at 00:00:17:08"));
        QCOMPARE(view.rowThread(6), QStringLiteral("c-1708"));
        const QList<QToolButton *> links = view.findChildren<QToolButton *>(QStringLiteral("qcInThread"));
        QCOMPARE(links.size(), 2);
        QCOMPARE(links.first()->size(), QSize(64, 28));
        links.first()->click();
        QCOMPARE(threads.size(), 1);
        QCOMPARE(threads.first().first().toString(), QStringLiteral("c-2718"));
        links.last()->click();
        QCOMPARE(threads.last().first().toString(), QStringLiteral("c-1708"));

        // The painted row: name ink, value neg on a failure, Pass ink3.
        view.show();
        QVERIFY(QTest::qWaitForWindowExposed(&view));
        const QList<QcRow *> rows = view.findChildren<QcRow *>();
        QCOMPARE(rows.size(), 8);
        QCOMPARE(rows.first()->height(), 28);
        TextRecorder recorder;
        rows.at(1)->render(&recorder);
        QCOMPARE(recorder.colorOf(QStringLiteral("True peak")), tokens::color(tokens::ink));
        QCOMPARE(recorder.colorOf(QStringLiteral("−0.4 dBTP")), tokens::color(tokens::neg));
        TextRecorder passing;
        rows.at(0)->render(&passing);
        QCOMPARE(passing.colorOf(QStringLiteral("Pass")), tokens::color(tokens::ink3));
        QCOMPARE(passing.colorOf(QStringLiteral("−23.1 LUFS")), tokens::color(tokens::ink2));

        // A new check over the result keeps the rows and shows its progress.
        bus.dispatch(QJsonObject{{"type", "check"}, {"status", "running"}, {"progress", 0.1}});
        QCOMPARE(view.rowCount(), 8);
        QCOMPARE(view.statusText(), QStringLiteral("Running QC… 10%"));
        bus.dispatch(QJsonObject{{"type", "check"}, {"status", "failed"}, {"message", "The check was cancelled"}});
        QCOMPARE(view.statusText(), QStringLiteral("The check was cancelled"));
        QCOMPARE(view.rowCount(), 8);

        // A run whose connection goes never hears its end: the view stops
        // waiting and shows the last result again, Run QC back.
        bus.dispatch(QJsonObject{{"type", "check"}, {"status", "running"}, {"progress", 0.5}});
        QVERIFY(view.running());
        bus.dispatch(QJsonObject{{"type", "connection"}, {"status", "connecting"}});
        QVERIFY(!view.running());
        QVERIFY(view.statusText().isEmpty());
        QCOMPARE(view.rowCount(), 8);
        QCOMPARE(view.summaryText(), QStringLiteral("6 of 8 pass"));
    }

    // ---- the tray ------------------------------------------------------------

    // The bar names what the scopes measure, and the targets; a pane asked
    // for over the bus is outlined for 1.2 s.
    void trayBarAndHighlight() {
        EditorBus bus;
        const QJsonObject snapshot = everyKindSnapshot(3);
        bus.setSnapshot(snapshot, snapshot.value("timeline").toObject(), 24, QSize(1080, 1920));
        bus.setPlayhead(9.125);
        FinishTray tray(nullptr, &bus);
        QCOMPARE(tray.scopesText(), QStringLiteral("Program · 00:00:09:03 · fisher-close.mov"));
        bus.dispatch(QJsonObject{{"type", "media_info"}, {"media", QJsonObject{{"close", QJsonObject{{"codec", "prores"}, {"colorSpace", "bt709"},
                                                                                              {"primaries", "bt709"}, {"bitDepth", 10}}}}}});
        QCOMPARE(tray.scopesText(), QStringLiteral("Program · 00:00:09:03 · fisher-close.mov · Rec.709"));
        // The bridge sends media_info in pieces: one for newer media keeps
        // what the earlier ones said.
        bus.dispatch(QJsonObject{{"type", "media_info"}, {"media", QJsonObject{{"score", QJsonObject{{"codec", "pcm_s24le"}}}}}});
        QCOMPARE(tray.scopesText(), QStringLiteral("Program · 00:00:09:03 · fisher-close.mov · Rec.709"));
        // A tray made later starts from the last one the bus kept, and
        // learns the rest as they come.
        bus.dispatch(QJsonObject{{"type", "media_info"}, {"media", QJsonObject{{"close", QJsonObject{{"primaries", "bt2020"}}}}}});
        FinishTray later(nullptr, &bus);
        QCOMPARE(later.scopesText(), QStringLiteral("Program · 00:00:09:03 · fisher-close.mov · Rec.2020"));
        bus.dispatch(QJsonObject{{"type", "media_info"}, {"media", QJsonObject{{"close", QJsonObject{{"primaries", "bt709"}}}}}});
        QCOMPARE(tray.scopesText(), QStringLiteral("Program · 00:00:09:03 · fisher-close.mov · Rec.709"));
        bus.setPlayhead(2.0);
        QCOMPARE(tray.scopesText(), QStringLiteral("Program · 00:00:02:00"));
        QCOMPARE(tray.targetsText(), QStringLiteral("Loudness EBU R128 · −23 LUFS · −1 dBTP"));
        bus.dispatch(mockupCheck());
        QCOMPARE(tray.paneMeta(QStringLiteral("qc")), QStringLiteral("6 of 8 pass"));
        QCOMPARE(tray.findChild<QLabel *>(QStringLiteral("finishQcStandards"))->text(), QStringLiteral("R128 · Rec.709 · captions"));

        QCOMPARE(FinishTray::highlightMs, 1200);
        emit bus.highlightPane(QStringLiteral("scopes"));
        QCOMPARE(tray.highlightedPane(), QStringLiteral("scopes"));
        for (const char *id : {"waveform", "vectorscope", "parade"})
            QVERIFY(static_cast<shell::TrayPane *>(tray.pane(QString::fromLatin1(id)))->highlighted());
        QVERIFY(!static_cast<shell::TrayPane *>(tray.pane(QStringLiteral("qc")))->highlighted());
        emit bus.highlightPane(QStringLiteral("loudness"));
        QVERIFY(static_cast<shell::TrayPane *>(tray.pane(QStringLiteral("loudness")))->highlighted());
        QVERIFY(!static_cast<shell::TrayPane *>(tray.pane(QStringLiteral("waveform")))->highlighted());
        QTest::qWait(1000);
        QCOMPARE(tray.highlightedPane(), QStringLiteral("loudness"));
        QTRY_VERIFY_WITH_TIMEOUT(tray.highlightedPane().isEmpty(), 1000);
        QVERIFY(!static_cast<shell::TrayPane *>(tray.pane(QStringLiteral("loudness")))->highlighted());
        emit bus.highlightPane(QStringLiteral("queue")); // the Deliver tray's
        QVERIFY(tray.highlightedPane().isEmpty());
    }

    // A pane's highlight fades its outline from line to accent over 160 ms
    // and back; under reduced motion (or hidden) it jumps.
    void trayPaneHighlightFades() {
        auto *body = new QWidget;
        shell::TrayPane pane(QStringLiteral("fadePane"), QStringLiteral("Loudness"), nullptr, body);
        pane.resize(320, 200);
        QCOMPARE(pane.outlineColor().rgba(), tokens::color(tokens::line).rgba());
        pane.setHighlighted(true);
        QCOMPARE(pane.highlightLevel(), 1.0);
        pane.setHighlighted(false);
        QCOMPARE(pane.highlightLevel(), 0.0);
        pane.show();
        QVERIFY(QTest::qWaitForWindowExposed(&pane));
        ui::setReducedMotionOverride(0);
        const auto restore = qScopeGuard([] { ui::setReducedMotionOverride(-1); });
        QElapsedTimer clock;
        clock.start();
        pane.setHighlighted(true);
        QVERIFY(pane.highlightLevel() < 1.0);
        QTest::qWait(60);
        QVERIFY2(pane.highlightLevel() > 0.0 && pane.highlightLevel() < 1.0, qPrintable(QString::number(pane.highlightLevel())));
        QTRY_COMPARE_WITH_TIMEOUT(pane.highlightLevel(), 1.0, 1000);
        QVERIFY(clock.elapsed() >= 140);
        QCOMPARE(pane.outlineColor().rgba(), tokens::color(tokens::accent).rgba());
        pane.setHighlighted(false);
        QTRY_COMPARE_WITH_TIMEOUT(pane.highlightLevel(), 0.0, 1000);
        ui::setReducedMotionOverride(1);
        pane.setHighlighted(true);
        QCOMPARE(pane.highlightLevel(), 1.0);
    }

    // The clip the bar names is the one the program shows: a clip or track
    // that is off is passed over, and a clip without a name of its own is
    // called by its media's name.
    void trayBarNamesTheClipTheProgramShows() {
        EditorBus bus;
        QJsonObject snapshot = everyKindSnapshot(3);
        QJsonObject timeline = snapshot.value("timeline").toObject();
        QJsonArray tracks = timeline.value("tracks").toArray();
        QJsonObject v1 = tracks.at(0).toObject();
        QJsonArray clips = v1.value("clips").toArray();
        QJsonObject media = clips.at(0).toObject();
        media.remove("name");
        clips[0] = media;
        v1["clips"] = clips;
        tracks[0] = v1;
        // A V3 over V1 with a clip of the score's that is off, then one on a track that is off.
        const auto overClip = [](const char *id, bool enabled) {
            return QJsonObject{{"id", id}, {"kind", "media"}, {"sourceId", "score"}, {"name", "over"}, {"timelineStart", 6.0},
                               {"duration", 5.0}, {"enabled", enabled}};
        };
        tracks.append(QJsonObject{{"id", "V3"}, {"type", "video"}, {"enabled", true}, {"clips", QJsonArray{overClip("off", false)}}});
        tracks.append(QJsonObject{{"id", "V4"}, {"type", "video"}, {"enabled", false}, {"clips", QJsonArray{overClip("hidden", true)}}});
        timeline["tracks"] = tracks;
        snapshot["timeline"] = timeline;
        bus.setSnapshot(snapshot, timeline, 24, QSize(1080, 1920));
        bus.setPlayhead(9.125);
        bus.dispatch(QJsonObject{{"type", "media_info"}, {"media", QJsonObject{{"close", QJsonObject{{"primaries", "bt709"}}},
                                                                              {"score", QJsonObject{{"primaries", "bt2020"}}}}}});
        FinishTray tray(nullptr, &bus);
        QCOMPARE(tray.scopesText(), QStringLiteral("Program · 00:00:09:03 · fisher-close · Rec.709"));
    }

    // Run QC waits while a check runs, and no longer than its connection.
    void trayRunQcReturnsWhenTheConnectionGoes() {
        EditorBus bus;
        QAction ask(QStringLiteral("Ask"));
        QAction runQc(QStringLiteral("Run QC"));
        FinishTray tray(nullptr, &bus);
        tray.setActions(&ask, &runQc);
        QVERIFY(tray.runQcButton()->isEnabled());
        bus.dispatch(QJsonObject{{"type", "check"}, {"status", "running"}, {"progress", 0.2}});
        QVERIFY(!tray.runQcButton()->isEnabled());
        QCOMPARE(tray.runQcButton()->toolTip(), QStringLiteral("QC is running"));
        bus.dispatch(QJsonObject{{"type", "connection"}, {"status", "disconnected"}});
        QVERIFY(tray.runQcButton()->isEnabled());
        QVERIFY(!tray.qcView()->running());
    }

    // The fluid rule: 8 apart, the vectorscope and loudness 240, the rest
    // shared 352 : 352 : 368.
    void panesFollowTheFluidRule() {
        const auto spans = FinishTray::paneSpans(1600);
        const int x[5] = {8, 368, 616, 976, 1224};
        const int w[5] = {352, 240, 352, 240, 368};
        for (int i = 0; i < 5; ++i) {
            QCOMPARE(spans[size_t(i)].x, x[i]);
            QCOMPARE(spans[size_t(i)].width, w[i]);
        }
        const auto wide = FinishTray::paneSpans(1920);
        QCOMPARE(wide[1].width, 240);
        QCOMPARE(wide[3].width, 240);
        QCOMPARE(wide[4].x + wide[4].width, 1920 - 8);
        QVERIFY(std::abs(wide[0].width * 368.0 / 352 - wide[4].width) <= 1.5);
    }

    // In the window: the panes at 1600 match spec 12, and Run QC sends check.
    void trayInTheWindowRunsQc() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(showcaseSnapshot(3, QStringLiteral("clean")));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.m_commands->action(QStringLiteral("workspace.finish"))->trigger();
        QCoreApplication::processEvents();
        FinishTray *tray = window.m_finishTray;
        QVERIFY(tray->isVisible());
        QTRY_VERIFY(tray->analyser()->isActive());
        QCOMPARE(tray->width(), 1600);
        const int x[5] = {8, 368, 616, 976, 1224};
        const int w[5] = {352, 240, 352, 240, 368};
        const char *ids[5] = {"waveform", "vectorscope", "parade", "loudness", "qc"};
        for (int i = 0; i < 5; ++i) {
            const QRect pane = tray->pane(QString::fromLatin1(ids[i]))->geometry();
            QVERIFY2(std::abs(pane.x() - x[i]) <= 1 && std::abs(pane.width() - w[i]) <= 1,
                     qPrintable(QStringLiteral("%1 %2 %3").arg(QString::fromLatin1(ids[i])).arg(pane.x()).arg(pane.width())));
            QCOMPARE(pane.y(), 36);
            QVERIFY2(std::abs(pane.height() - 300) <= 1, qPrintable(QString::number(pane.height())));
        }
        QVERIFY(tray->runQcButton()->isEnabled());
        window.m_lastCommand = {};
        tray->runQcButton()->click();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "check"}}));
        // While it runs, Run QC waits.
        window.handleEvent(QJsonObject{{"type", "check"}, {"status", "running"}, {"progress", 0.0}});
        QVERIFY(!tray->runQcButton()->isEnabled());
        QCOMPARE(tray->runQcButton()->toolTip(), QStringLiteral("QC is running"));
        window.handleEvent(mockupCheck());
        QVERIFY(tray->runQcButton()->isEnabled());
        // Measure in the Loudness pane asks the bridge for the programme.
        window.m_lastCommand = {};
        tray->loudnessView()->measureButton()->click();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "loudness"}}));
        // The tray leaves with its workspace, and its scopes stop.
        window.m_commands->action(QStringLiteral("workspace.review"))->trigger();
        QCoreApplication::processEvents();
        QTRY_VERIFY(!tray->analyser()->isActive());
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::FinishTest)

#include "finish_test.moc"
