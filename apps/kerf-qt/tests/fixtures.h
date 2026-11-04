#pragma once

// What the Qt tests share: fixture data shaped like the bridge's, helpers that
// read painted pixels and text, and small drivers for widgets and dialogs.
// Every test file includes it; tests/test_main.cpp runs the classes.

#include "media_artwork.h"

#include <QColor>
#include <QEvent>
#include <QIcon>
#include <QImage>
#include <QJsonArray>
#include <QJsonObject>
#include <QJsonValue>
#include <QList>
#include <QPaintDevice>
#include <QPaintEngine>
#include <QPainter>
#include <QPointF>
#include <QSize>
#include <QString>

#include <climits>
#include <functional>
#include <initializer_list>

class QComboBox;
class QWidget;

namespace kerf {

class ChatPanel;
class MainWindow;

namespace test {

// A paint device that keeps each run of text painted on it with the pen it
// was painted in, so a test reads a painted text's colour from the painting
// rather than from antialiased pixels. Everything else is dropped.
class TextRecorder final : public QPaintDevice {
public:
    struct Run {
        QString text;
        QColor color;
    };
    QList<Run> runs;

    TextRecorder() : m_engine(this) {}
    QPaintEngine *paintEngine() const override { return &m_engine; }
    // The colour the first run containing `text` was painted in.
    QColor colorOf(const QString &text) const {
        for (const Run &run : runs)
            if (run.text.contains(text)) return run.color;
        return {};
    }

protected:
    int metric(PaintDeviceMetric metric) const override {
        switch (metric) {
        case PdmWidth: return 600;
        case PdmHeight: return 120;
        case PdmWidthMM: return 212;
        case PdmHeightMM: return 42;
        case PdmNumColors: return INT_MAX;
        case PdmDepth: return 32;
        case PdmDpiX:
        case PdmDpiY:
        case PdmPhysicalDpiX:
        case PdmPhysicalDpiY: return 72;
        case PdmDevicePixelRatio: return 1;
        case PdmDevicePixelRatioScaled: return int(devicePixelRatioFScale());
        default: return QPaintDevice::metric(metric);
        }
    }

private:
    class Engine final : public QPaintEngine {
    public:
        explicit Engine(TextRecorder *device) : QPaintEngine(AllFeatures), m_device(device) {}
        bool begin(QPaintDevice *) override { return true; }
        bool end() override { return true; }
        void updateState(const QPaintEngineState &) override {}
        void drawPath(const QPainterPath &) override {}
        void drawPolygon(const QPointF *, int, PolygonDrawMode) override {}
        void drawPixmap(const QRectF &, const QPixmap &, const QRectF &) override {}
        void drawTextItem(const QPointF &, const QTextItem &item) override { m_device->runs.append({item.text(), state->pen().color()}); }
        Type type() const override { return User; }

    private:
        TextRecorder *m_device;
    };
    mutable Engine m_engine;
};

// `paint` run on a clear image `logical` points in size at `ratio` device
// pixels per point, as a widget paints on a 1x, 1.25x or Retina screen.
QImage paintedAt(qreal ratio, const QSize &logical, const std::function<void(QPainter &)> &paint);

// The lightness of `image`'s pixel at the logical point `at`.
int lightnessAt(const QImage &image, const QPointF &at);

// Open Kerf's conversation in the Program monitor's stage.
void showAssistant(MainWindow &window);

// The catalog's clip properties and generators, as a snapshot carries
// them (crates/kerf-model catalog.rs), for the Inspector's tests.
QJsonObject catalogProperties();

// The catalog's generators and their parameters.
QJsonObject catalogGenerators();

// The last edit's operations, taken.
QJsonArray takeOps(MainWindow &window);

// Mouse events sent straight to `widget`, which need not be on screen.
void mouse(QWidget *widget, QEvent::Type type, const QPointF &at);

// A key press sent straight to `widget`.
void key(QWidget *widget, int code);

// Whether `values` is an array of `expected`, each to within 0.001.
bool near(const QJsonValue &values, std::initializer_list<double> expected);

// A project with a clip of every kind the Inspector names.
QJsonObject everyKindSnapshot(int revision);

// A `collab` status for a shared project on main.
QJsonObject sharedStatus();

// Answer the next modal QInputDialog with `text`.
void answerInput(QWidget *parent, const QString &text);

// Whether `combo`'s menu, opened, shows every choice whole: no scroll
// arrows and nothing to scroll.
bool popupShowsEveryRow(QComboBox *combo);

// Whether `color` is one of the graphite ramp's surfaces (surroundDim
// through hover2): opaque, neutral (|r-g| and |g-b| at most 2), dark, and
// not pure black.
bool isGraphite(const QColor &color);

// Saturated amber or gold, as the editor's old accent was.
bool isAmber(const QColor &color);

// How many amber areas `image` has, and where the first is. A pixel
// counts only when its eight neighbours are amber too, so the coloured
// fringes of subpixel-antialiased text (as on Linux) are not counted.
int amberPixels(const QImage &image, QPoint *first = nullptr);

// `chat`'s list is scrolled to its end, with the latest row whole over the
// chips (or the composer) and the rows cut cleanly at the list's edges.

// A waveform's ink (its RMS grey, rgba(144,144,144,.75), over a clip's
// card and its peak): a neutral mid grey.
bool isWaveInk(const QColor &pixel);

// The colour most of `image` is.
QRgb dominantColor(const QImage &image);

// Whether two colours match to within `tolerance` per channel: a
// translucent fill laid on its surface by the painter rounds its own way.
bool nearColor(QRgb a, QRgb b, int tolerance = 2);

// `top` as it shows laid on the opaque `under`.
QColor laidOn(const QColor &under, QColor top);

// The colour `icon`'s symbol is drawn in: its most opaque pixel's.
QRgb ink(const QIcon &icon, QIcon::Mode mode = QIcon::Normal, QIcon::State state = QIcon::Off);

// A cut with every kind of clip: two picture tracks (one of titles), a
// caption track, dialogue and music, a marker, and a pending proposal
// that trims one clip, drops another, and adds a title.
QJsonObject showcaseSnapshot(int revision, const QString &projection);

// Stand-in posters, filmstrips and peaks, so filmstrips and waves draw
// without FFmpeg: frames of the tests' procedural footage (see
// test_footage.h), one shot per source, made once per run. A strip
// pans across its shot, so its frames differ along the clip.
void fakeArtwork(MediaArtwork *artwork);

// Sound at full scale throughout, `seconds` long: its waveform fills
// the body, RMS and all.
AudioPeaks fullScalePeaks(double seconds);

} // namespace test
} // namespace kerf
