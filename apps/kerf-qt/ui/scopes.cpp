#include "scopes.h"
#include "scope_math.h"

#include <QFontMetricsF>
#include <QMetaObject>
#include <QPainter>
#include <QPainterPath>

#include <algorithm>
#include <cmath>

namespace kerf {

namespace scopes {

namespace {

qreal pixelRatio(qreal dpr) { return std::clamp(dpr, 1.0, 2.0); }

int devicePixels(double points, qreal dpr) { return qMax(1, int(std::lround(points * dpr))); }

// A waveform-style trace's grid: the graticule's width, and one row more
// than its height so that code 0 lands on the bottom line itself.
QSize traceGrid(double width, double height, qreal dpr) {
    return QSize(devicePixels(width, dpr), devicePixels(height, dpr) + 1);
}

QImage withRatio(QImage image, qreal dpr) {
    image.setDevicePixelRatio(dpr);
    return image;
}

// Several traces added together, as the design draws overlaid channels.
QImage plus(const std::array<QImage, 3> &layers) {
    QImage out = layers[0];
    out.detach();
    QPainter painter(&out);
    painter.setCompositionMode(QPainter::CompositionMode_Plus);
    for (size_t i = 1; i < layers.size(); ++i) painter.drawImage(0, 0, layers[i]);
    return out;
}

constexpr std::array<QRgb, 3> kChannelTraces{redTrace, greenTrace, blueTrace};

} // namespace

int bitDepth(const QImage &frame) {
    if (frame.isNull()) return 0;
    switch (frame.format()) {
    case QImage::Format_RGB30:
    case QImage::Format_BGR30:
    case QImage::Format_A2RGB30_Premultiplied:
    case QImage::Format_A2BGR30_Premultiplied: return 10;
    case QImage::Format_RGBX64:
    case QImage::Format_RGBA64:
    case QImage::Format_RGBA64_Premultiplied:
    case QImage::Format_Grayscale16: return 16;
    case QImage::Format_RGBX16FPx4:
    case QImage::Format_RGBA16FPx4:
    case QImage::Format_RGBA16FPx4_Premultiplied: return 16;
    case QImage::Format_RGBX32FPx4:
    case QImage::Format_RGBA32FPx4:
    case QImage::Format_RGBA32FPx4_Premultiplied: return 32;
    case QImage::Format_RGB16:
    case QImage::Format_RGB444:
    case QImage::Format_RGB555: return 5;
    default: return 8;
    }
}

QSize sampleSize(QSize frame, qreal dpr) {
    if (frame.isEmpty()) return {};
    const qreal ratio = pixelRatio(dpr);
    const double area = 160.0 * 90.0 * ratio * ratio;
    const double aspect = double(frame.width()) / frame.height();
    int width = int(std::lround(std::sqrt(area * aspect)));
    int height = int(std::lround(width / aspect));
    if (width > frame.width() || height > frame.height()) return frame;
    return QSize(qMax(1, width), qMax(1, height));
}

ScopeTraces analyse(const QImage &frame, const ScopeLayout &layout) {
    ScopeTraces traces;
    if (frame.isNull()) return traces;
    const qreal dpr = pixelRatio(layout.dpr);
    traces.bitDepth = bitDepth(frame);
    traces.frameSize = frame.size();
    traces.mode = layout.mode;
    const QImage source = scope::scopeSource(frame, sampleSize(frame.size(), dpr));
    traces.sampled = source.size();

    if (!layout.waveform.isEmpty()) {
        const scope::WaveformGeometry g = scope::waveformGeometry(QSizeF(layout.waveform));
        const QSize grid = traceGrid(g.x1 - g.x0, g.bottom - g.top, dpr);
        if (layout.mode == WaveformMode::Luma) {
            traces.waveform = scope::render(scope::waveformLuma(source, grid.width(), grid.height()), lumaTrace);
        } else {
            const auto cells = scope::parade(source, grid.width(), grid.height());
            traces.waveform = plus({scope::render(cells[0], kChannelTraces[0]), scope::render(cells[1], kChannelTraces[1]),
                                    scope::render(cells[2], kChannelTraces[2])});
        }
        traces.waveform = withRatio(traces.waveform, dpr);
    }

    if (!layout.parade.isEmpty()) {
        const scope::WaveformGeometry g = scope::waveformGeometry(QSizeF(layout.parade));
        const QRectF cell = g.cell(0, 3, cellGap);
        const QSize grid = traceGrid(cell.width(), g.bottom - g.top, dpr);
        const auto cells = scope::parade(source, grid.width(), grid.height());
        for (size_t i = 0; i < cells.size(); ++i) traces.parade[i] = withRatio(scope::render(cells[i], kChannelTraces[i]), dpr);
    }

    if (!layout.vectorscope.isEmpty()) {
        const QSizeF plot(layout.vectorscope);
        scope::VectorscopeGeometry geometry = scope::vectorscopeGeometry(plot);
        geometry.centre *= dpr;
        geometry.radius *= dpr;
        geometry.scale *= dpr;
        scope::Options options;
        options.chromaJitter *= dpr;
        const QSize grid(devicePixels(plot.width(), dpr), devicePixels(plot.height(), dpr));
        traces.vectorscope =
            withRatio(scope::render(scope::vectorscope(source, grid, geometry, traceGain, options), vectorscopeTrace), dpr);
    }
    return traces;
}

} // namespace scopes

// ================================================================ ScopeAnalyser

ScopeAnalyser::ScopeAnalyser(QObject *parent) : QObject(parent) {
    // One analysis at a time; a newer frame waits for it (and replaces any
    // other that waits).
    m_pool.setMaxThreadCount(1);
    m_timer.setSingleShot(true);
    connect(&m_timer, &QTimer::timeout, this, &ScopeAnalyser::schedule);
}

ScopeAnalyser::~ScopeAnalyser() {
    m_timer.stop();
    // The task posts its result to this object; it is still alive while we
    // wait, and ~QObject drops the posted result.
    m_pool.waitForDone();
}

void ScopeAnalyser::setActive(bool active) {
    if (m_active == active) return;
    m_active = active;
    if (!active) m_timer.stop();
    else schedule();
}

void ScopeAnalyser::setLayout(const ScopeLayout &layout) {
    if (m_layout == layout) return;
    m_layout = layout;
    if (!m_frame.isNull()) m_dirty = true;
    schedule();
}

void ScopeAnalyser::setFrame(const QImage &frame) {
    if (frame.isNull()) return;
    m_frame = frame;
    m_dirty = true;
    schedule();
}

void ScopeAnalyser::schedule() {
    if (!m_active || !m_dirty || m_running) return;
    if (m_layout.waveform.isEmpty() && m_layout.vectorscope.isEmpty() && m_layout.parade.isEmpty()) return;
    const qint64 since = m_since.isValid() ? m_since.elapsed() : qint64(intervalMs);
    if (since >= intervalMs) {
        m_timer.stop();
        start();
    } else if (!m_timer.isActive()) {
        m_timer.start(int(intervalMs - since));
    }
}

void ScopeAnalyser::start() {
    m_dirty = false;
    m_running = true;
    m_since.start();
    ++m_started;
    const QImage frame = m_frame;
    const ScopeLayout layout = m_layout;
    m_pool.start([this, frame, layout] {
        const ScopeTraces traces = scopes::analyse(frame, layout);
        QMetaObject::invokeMethod(this, [this, traces] { finish(traces); }, Qt::QueuedConnection);
    });
}

void ScopeAnalyser::finish(const ScopeTraces &traces) {
    m_running = false;
    m_traces = traces;
    emit tracesChanged();
    schedule();
}

// ================================================================ ScopePlot

ScopePlot::ScopePlot(Kind kind, ScopeAnalyser *analyser, QWidget *parent)
    : QWidget(parent), m_kind(kind), m_analyser(analyser) {
    setObjectName(kind == Kind::Waveform      ? QStringLiteral("scopePlot_waveform")
                  : kind == Kind::Vectorscope ? QStringLiteral("scopePlot_vectorscope")
                                              : QStringLiteral("scopePlot_parade"));
    setAttribute(Qt::WA_OpaquePaintEvent, false);
    setMinimumSize(48, 48);
    if (analyser) connect(analyser, &ScopeAnalyser::tracesChanged, this, qOverload<>(&QWidget::update));
}

QRectF ScopePlot::plotRect() const {
    // The pane's body starts 1 in from its sides; the plot is inset 4.
    return QRectF(rect()).adjusted(3, 0, -3, 0);
}

QSize ScopePlot::plotSize() const {
    return plotRect().size().toSize();
}

QString ScopePlot::emptyText() const {
    return m_analyser && !m_analyser->traces().isNull() ? QString() : QStringLiteral("No program frame");
}

void ScopePlot::resizeEvent(QResizeEvent *) {
    emit plotResized();
}

void ScopePlot::paintWaveformGraticule(QPainter &painter, const QRectF &plot, bool parade) const {
    const scope::WaveformGeometry g = scope::waveformGeometry(plot.size());
    const QFont font = tokens::font(tokens::monoSmall);
    painter.setFont(font);
    for (int code : scope::WaveformGeometry::graticule) {
        const double y = std::round(g.y(code)) + 0.5;
        painter.setPen(QPen(tokens::color(scopes::graticule), 1));
        painter.drawLine(QPointF(g.x0, y), QPointF(g.x1, y));
        painter.setPen(tokens::color(tokens::ink3));
        painter.drawText(QRectF(0, y - 6, g.x0 - 4, 16), Qt::AlignRight | Qt::AlignVCenter, QString::number(code));
    }
    if (!parade) return;
    static const char *const names[] = {"R", "G", "B"};
    for (int i = 0; i < 3; ++i) {
        const QRectF cell = g.cell(i, 3, scopes::cellGap);
        painter.drawText(QRectF(std::round(cell.x() + 4), plot.height() - 22, 16, 16), Qt::AlignLeft | Qt::AlignVCenter,
                         QString::fromLatin1(names[i]));
    }
}

void ScopePlot::paintVectorscopeGraticule(QPainter &painter, const QRectF &plot) const {
    const scope::VectorscopeGeometry g = scope::vectorscopeGeometry(plot.size());
    painter.setRenderHint(QPainter::Antialiasing, true);
    painter.setPen(QPen(tokens::color(scopes::graticule), 1));
    painter.setBrush(Qt::NoBrush);
    painter.drawEllipse(g.centre, g.radius, g.radius);
    painter.drawEllipse(g.centre, g.radius / 2, g.radius / 2);
    painter.setRenderHint(QPainter::Antialiasing, false);
    painter.drawLine(QPointF(g.centre.x() - g.radius, g.centre.y()), QPointF(g.centre.x() + g.radius, g.centre.y()));
    painter.drawLine(QPointF(g.centre.x(), g.centre.y() - g.radius), QPointF(g.centre.x(), g.centre.y() + g.radius));
    painter.setFont(tokens::font(tokens::monoSmall));
    for (const scope::TargetMark &mark : scope::targets(plot.size(), scopes::targetGain)) {
        painter.setPen(QPen(tokens::color(scopes::targetBox), 1));
        const QPointF corner(std::round(mark.box.x()) + 0.5, std::round(mark.box.y()) + 0.5);
        painter.drawRect(QRectF(corner, QSizeF(8, 8)));
        painter.setPen(tokens::color(tokens::ink3));
        painter.drawText(QRectF(mark.labelCentre.x() - 12, mark.labelCentre.y() - 6, 24, 16), Qt::AlignCenter,
                         QString::fromLatin1(mark.label));
    }
}

void ScopePlot::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    const QRectF plot = plotRect();
    if (plot.width() <= 0 || plot.height() <= 0) return;
    painter.setRenderHint(QPainter::Antialiasing, true);
    QPainterPath well;
    well.addRoundedRect(plot, tokens::radius::small, tokens::radius::small);
    painter.fillPath(well, tokens::color(tokens::well));
    painter.setRenderHint(QPainter::Antialiasing, false);
    painter.setClipPath(well);
    painter.translate(plot.topLeft());
    const QRectF local(QPointF(), plot.size());
    if (m_kind == Kind::Vectorscope) paintVectorscopeGraticule(painter, local);
    else paintWaveformGraticule(painter, local, m_kind == Kind::Parade);

    const ScopeTraces empty;
    const ScopeTraces &traces = m_analyser ? m_analyser->traces() : empty;
    if (traces.isNull()) {
        painter.setFont(tokens::font(tokens::meta));
        painter.setPen(tokens::color(tokens::ink3));
        painter.drawText(local, Qt::AlignCenter, emptyText());
        return;
    }
    painter.setCompositionMode(QPainter::CompositionMode_Plus);
    const auto draw = [&painter](const QPointF &at, const QImage &trace) {
        if (trace.isNull()) return;
        const qreal ratio = trace.devicePixelRatio();
        painter.drawImage(QRectF(at, QSizeF(trace.width() / ratio, trace.height() / ratio)), trace);
    };
    if (m_kind == Kind::Vectorscope) {
        draw(QPointF(), traces.vectorscope);
    } else {
        const scope::WaveformGeometry g = scope::waveformGeometry(local.size());
        if (m_kind == Kind::Waveform) draw(QPointF(g.x0, g.top), traces.waveform);
        else
            for (int i = 0; i < 3; ++i) draw(QPointF(g.cell(i, 3, scopes::cellGap).x(), g.top), traces.parade[size_t(i)]);
    }
}

} // namespace kerf
