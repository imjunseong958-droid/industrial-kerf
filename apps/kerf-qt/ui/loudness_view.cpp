#include "loudness_view.h"
#include "components.h"
#include "design_tokens.h"
#include "editor_bus.h"
#include "tray_shell.h"

#include <QJsonArray>
#include <QLabel>
#include <QPainter>
#include <QPainterPath>

#include <algorithm>
#include <cmath>

namespace kerf {

namespace {

// The design's history drawing: 208 x 120, the curve between x 8 and 200.
constexpr double kViewWidth = 208;
constexpr double kPlotLeft = 8;
constexpr double kPlotWidth = 192;
// Rows, in the view's coordinates (mockup-spec 12, .ld).
constexpr int kStatsHeight = 40;
constexpr int kHistoryTop = kStatsHeight + 8;
constexpr int kHistoryHeight = 120;
constexpr int kLegendTop = kHistoryTop + kHistoryHeight + 4;
constexpr int kTruePeakTop = kLegendTop + 16 + 4;
constexpr int kTruePeakRow = 24;
constexpr int kChannelWidth = 12;
constexpr int kValueWidth = 72;
// The pane's body starts 1 in from its edge; the content is 12 in.
constexpr int kInset = tokens::space::s12 - 1;

std::optional<double> number(const QJsonValue &value) {
    if (!value.isDouble() || !std::isfinite(value.toDouble())) return std::nullopt;
    return value.toDouble();
}

// "−23.1": one decimal and a true minus; a dash when nothing was measured.
QString decimal(std::optional<double> value) {
    if (!value) return QStringLiteral("—");
    const double rounded = std::round(*value * 10) / 10;
    const QString text = QString::number(std::abs(rounded), 'f', 1);
    return rounded < 0 ? QStringLiteral("−") + text : text;
}

// "−23", "−1": the targets as the design writes them.
QString whole(double value) {
    const QString text = QString::number(std::abs(value), 'g', 3);
    return value < 0 ? QStringLiteral("−") + text : text;
}

// "30 s", "2 min 05 s", "1 h 02 min": the programme's length for the legend.
QString lengthText(double seconds) {
    const long long s = std::llround(std::max(0.0, seconds));
    if (s < 120) return QStringLiteral("%1 s").arg(s);
    if (s < 3600) return QStringLiteral("%1 min %2 s").arg(s / 60).arg(s % 60, 2, 10, QLatin1Char('0'));
    return QStringLiteral("%1 h %2 min").arg(s / 3600).arg((s % 3600) / 60, 2, 10, QLatin1Char('0'));
}

} // namespace

LoudnessView::LoudnessView(QWidget *parent, EditorBus *bus) : QWidget(parent), m_bus(bus) {
    setObjectName(QStringLiteral("loudnessView"));
    setMinimumSize(200, kTruePeakTop + 2 * kTruePeakRow + 4);
    m_status = shell::text(QString(), tokens::meta, tokens::ink3);
    m_status->setObjectName(QStringLiteral("loudnessStatus"));
    m_status->setParent(this);
    m_status->setAlignment(Qt::AlignCenter);
    m_status->setWordWrap(true);
    m_measure = new ui::Button(QStringLiteral("Measure"), ui::Button::Kind::Secondary, this);
    m_measure->setObjectName(QStringLiteral("loudnessMeasure"));
    m_measure->setTip(QStringLiteral("Measure the programme to EBU R128"));
    connect(m_measure, &QAbstractButton::clicked, this, &LoudnessView::measure);
    m_again = new ui::GhostButton(QStringLiteral("Measure again"), this);
    m_again->setObjectName(QStringLiteral("loudnessMeasureAgain"));
    m_again->setTypeStyle(tokens::meta);
    m_again->setControlHeight(tokens::height::h20);
    m_again->setHorizontalPadding(tokens::space::s6);
    m_again->setTip(QStringLiteral("The document changed since this measurement"));
    connect(m_again, &QAbstractButton::clicked, this, &LoudnessView::measure);
    if (m_bus) {
        connect(m_bus, &EditorBus::event, this, [this](const QString &type, const QJsonObject &event) {
            if (type == QStringLiteral("loudness")) setLoudness(event);
            else if (type == QStringLiteral("meter")) setMeter(event);
            else if (type == QStringLiteral("connection")) connectionChanged();
            else if (type == QStringLiteral("error") && event.value(QStringLiteral("command")).toString() == QStringLiteral("loudness"))
                setLoudness(QJsonObject{{QStringLiteral("status"), QStringLiteral("failed")},
                                        {QStringLiteral("message"), event.value(QStringLiteral("message"))}});
        });
        connect(m_bus, &EditorBus::playheadChanged, this, qOverload<>(&QWidget::update));
        connect(m_bus, &EditorBus::snapshotChanged, this, &LoudnessView::refresh);
        const QJsonObject last = m_bus->lastEvent(QStringLiteral("loudness"));
        if (!last.isEmpty()) setLoudness(last);
    }
    refresh();
}

void LoudnessView::setLoudness(const QJsonObject &loudness) {
    const QString status = loudness.value(QStringLiteral("status")).toString();
    // A measurement that runs or fails keeps the last result on screen
    // only as far as saying so; a result replaces everything.
    m_loudness = loudness;
    if (status == QStringLiteral("ready")) {
        const QJsonValue revision = loudness.value(QStringLiteral("revision"));
        if (revision.isDouble()) m_measuredRevision = quint64(revision.toDouble());
        else if (m_requested) m_measuredRevision = m_requestedRevision;
        else if (m_bus) m_measuredRevision = m_bus->revision();
        m_requested = false;
    } else if (status == QStringLiteral("failed")) {
        m_requested = false;
    }
    m_sendError.clear();
    m_segmentsSize = QSize();
    refresh();
}

void LoudnessView::connectionChanged() {
    // A measurement runs on one connection: when it goes (or a new one
    // comes) its last event may never arrive, so stop waiting for it.
    const bool running = m_loudness.value(QStringLiteral("status")).toString() == QStringLiteral("running");
    if (!running && !m_requested) return;
    m_requested = false;
    if (running) {
        m_loudness = {};
        m_segmentsSize = QSize();
    }
    refresh();
}

bool LoudnessView::measured() const {
    return m_loudness.value(QStringLiteral("status")).toString() == QStringLiteral("ready");
}

bool LoudnessView::stale() const {
    return measured() && m_bus && m_bus->revision() != m_measuredRevision;
}

void LoudnessView::setMeter(const QJsonObject &meter) {
    m_meterM = number(meter.value(QStringLiteral("momentary")));
    m_meterS = number(meter.value(QStringLiteral("shortTerm")));
    update();
}

void LoudnessView::measure() {
    if (!m_bus) return;
    m_sendError.clear();
    m_requested = true;
    m_requestedRevision = m_bus->revision();
    if (!m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("loudness")}})) {
        m_requested = false;
        m_sendError = QStringLiteral("Kerf is not connected");
    }
    refresh();
}

double LoudnessView::target() const {
    return m_loudness.value(QStringLiteral("target")).toDouble(-23);
}

double LoudnessView::limit() const {
    return m_loudness.value(QStringLiteral("limit")).toDouble(-1);
}

double LoudnessView::programmeDuration() const {
    const double duration = m_loudness.value(QStringLiteral("duration")).toDouble();
    if (duration > 0) return duration;
    const QJsonArray series = m_loudness.value(QStringLiteral("series")).toArray();
    if (!series.isEmpty()) {
        const double last = series.last().toArray().at(0).toDouble();
        if (last > 0) return last;
    }
    return m_bus ? m_bus->duration() : 0;
}

std::optional<double> LoudnessView::truePeak(int channel) const {
    if (!measured()) return std::nullopt;
    return number(m_loudness.value(QStringLiteral("truePeak")).toArray().at(channel));
}

std::optional<double> LoudnessView::seriesAt(double seconds, int column) const {
    if (!measured()) return std::nullopt;
    const QJsonArray series = m_loudness.value(QStringLiteral("series")).toArray();
    if (series.isEmpty()) return std::nullopt;
    // The last reading at or before `seconds` (readings are in time order).
    int lo = 0, hi = int(series.size()) - 1, found = -1;
    while (lo <= hi) {
        const int mid = (lo + hi) / 2;
        if (series.at(mid).toArray().at(0).toDouble() <= seconds + 1e-9) {
            found = mid;
            lo = mid + 1;
        } else {
            hi = mid - 1;
        }
    }
    if (found < 0) return std::nullopt;
    return number(series.at(found).toArray().at(column));
}

QString LoudnessView::formatStat(const QString &key) const {
    const double at = m_bus ? m_bus->playhead() : 0;
    if (key == QStringLiteral("M")) return decimal(m_meterM ? m_meterM : seriesAt(at, 1));
    if (key == QStringLiteral("S")) return decimal(m_meterS ? m_meterS : seriesAt(at, 2));
    if (key == QStringLiteral("I"))
        return decimal(measured() ? number(m_loudness.value(QStringLiteral("integrated"))) : std::nullopt);
    if (key == QStringLiteral("LRA")) return decimal(measured() ? number(m_loudness.value(QStringLiteral("lra"))) : std::nullopt);
    if (key == QStringLiteral("TP L")) return decimal(truePeak(0)) + QStringLiteral(" dBTP");
    if (key == QStringLiteral("TP R")) return decimal(truePeak(1)) + QStringLiteral(" dBTP");
    return {};
}

QString LoudnessView::statText(const QString &key) const {
    return formatStat(key);
}

QString LoudnessView::statusText() const {
    return m_status->isHidden() ? QString() : m_status->text();
}

void LoudnessView::refresh() {
    const QString status = m_loudness.value(QStringLiteral("status")).toString();
    QString text;
    bool offerMeasure = false;
    if (!m_sendError.isEmpty()) {
        text = m_sendError;
        offerMeasure = true;
    } else if (status == QStringLiteral("running") || m_requested) {
        // The bridge's progress (0…1) once a running event carries it.
        const QJsonValue progress = m_loudness.value(QStringLiteral("progress"));
        text = status == QStringLiteral("running") && progress.isDouble()
                   ? QStringLiteral("Measuring the programme… %1%").arg(std::lround(std::clamp(progress.toDouble(), 0.0, 1.0) * 100))
                   : QStringLiteral("Measuring the programme…");
    } else if (status == QStringLiteral("failed")) {
        text = m_loudness.value(QStringLiteral("message")).toString();
        if (text.isEmpty()) text = QStringLiteral("The measurement failed");
        offerMeasure = true;
    } else if (!measured()) {
        text = QStringLiteral("Not measured yet");
        offerMeasure = true;
    }
    m_status->setText(text);
    m_status->setVisible(!text.isEmpty());
    m_measure->setVisible(offerMeasure);
    m_measure->setEnabled(m_bus != nullptr);
    m_again->setVisible(text.isEmpty() && stale());
    placeStatus();
    update();
}

void LoudnessView::resizeEvent(QResizeEvent *) {
    placeStatus();
}

QRectF LoudnessView::contentRect() const {
    return QRectF(kInset, 0, qMax(0, width() - 2 * kInset), height());
}

QRectF LoudnessView::historyRect() const {
    const QRectF content = contentRect();
    return QRectF(content.x(), kHistoryTop, content.width(), kHistoryHeight);
}

double LoudnessView::historyY(double lufs) {
    const double v = std::clamp(lufs, -36.0, -14.0);
    return 6 + (-14 - v) / 22 * 108;
}

double LoudnessView::targetLineY() const {
    return historyRect().y() + historyY(target());
}

std::optional<double> LoudnessView::playheadX() const {
    const double duration = programmeDuration();
    if (!measured() || !m_bus || duration <= 0) return std::nullopt;
    const QRectF well = historyRect();
    const double t = std::clamp(m_bus->playhead() / duration, 0.0, 1.0);
    return well.x() + (kPlotLeft + t * kPlotWidth) * well.width() / kViewWidth;
}

QList<QPolygonF> LoudnessView::historySegments() const {
    return segments();
}

const QList<QPolygonF> &LoudnessView::segments() const {
    if (m_segmentsSize == size()) return m_segments;
    m_segmentsSize = size();
    m_segments.clear();
    const double duration = programmeDuration();
    if (!measured() || duration <= 0) return m_segments;
    const QRectF well = historyRect();
    const double sx = well.width() / kViewWidth;
    const QJsonArray series = m_loudness.value(QStringLiteral("series")).toArray();
    // A long programme has more readings than the well has points: keep
    // the quietest and the loudest of each column, in time order.
    const int columns = qMax(1, int(std::ceil(kPlotWidth * sx)));
    const bool decimate = series.size() > 4 * columns;
    QPolygonF line;
    int column = -1, index = 0, lowAt = 0, highAt = 0;
    QPointF low, high;
    bool have = false;
    const auto flush = [&] {
        if (!have) return;
        if (low == high) line << low;
        else if (lowAt < highAt) line << low << high;
        else line << high << low;
        have = false;
    };
    // A null reading (silence) breaks the curve rather than bridging it.
    const auto endSegment = [&] {
        flush();
        if (!line.isEmpty()) m_segments << line;
        line.clear();
    };
    for (const QJsonValue &value : series) {
        const QJsonArray reading = value.toArray();
        const std::optional<double> s = number(reading.at(2));
        if (!s) {
            endSegment();
            continue;
        }
        const double t = std::clamp(reading.at(0).toDouble() / duration, 0.0, 1.0);
        const QPointF point(well.x() + (kPlotLeft + t * kPlotWidth) * sx, well.y() + historyY(*s));
        ++index;
        if (!decimate) {
            line << point;
            continue;
        }
        const int c = int(t * columns);
        if (c != column || !have) {
            flush();
            column = c;
            low = high = point;
            lowAt = highAt = index;
            have = true;
            continue;
        }
        if (point.y() > low.y()) {
            low = point;
            lowAt = index;
        }
        if (point.y() < high.y()) {
            high = point;
            highAt = index;
        }
    }
    endSegment();
    return m_segments;
}

double LoudnessView::barScale(double dbtp) const {
    return std::clamp((dbtp + 12) / 12, 0.0, 1.0);
}

QRectF LoudnessView::truePeakBar(int channel) const {
    const QRectF content = contentRect();
    const double top = kTruePeakTop + channel * (kTruePeakRow + 4);
    const double left = content.x() + kChannelWidth + tokens::space::s8;
    const double right = content.right() - kValueWidth - tokens::space::s8;
    return QRectF(left, top + (kTruePeakRow - 4) / 2.0, qMax(0.0, right - left), 4);
}

QRectF LoudnessView::truePeakFill(int channel) const {
    const std::optional<double> value = truePeak(channel);
    if (!value) return {};
    const QRectF bar = truePeakBar(channel);
    return QRectF(bar.x(), bar.y(), bar.width() * std::min(barScale(*value), barScale(limit())), bar.height());
}

QRectF LoudnessView::truePeakOvershoot(int channel) const {
    const std::optional<double> value = truePeak(channel);
    if (!value || *value <= limit()) return {};
    const QRectF bar = truePeakBar(channel);
    const double from = barScale(limit()), to = barScale(*value);
    return QRectF(bar.x() + bar.width() * from, bar.y(), bar.width() * (to - from), bar.height());
}

QRectF LoudnessView::truePeakTick(int channel) const {
    const QRectF bar = truePeakBar(channel);
    return QRectF(std::round(bar.x() + bar.width() * barScale(limit())), bar.y() - 4, 1, 12);
}

QString LoudnessView::legendText() const {
    const double duration = programmeDuration();
    return measured() && duration > 0 ? QStringLiteral("Short-term, %1").arg(lengthText(duration)) : QStringLiteral("Short-term");
}

void LoudnessView::placeStatus() {
    const QRectF well = historyRect();
    const int buttonHeight = !m_measure->isHidden() ? m_measure->height() + tokens::space::s8 : 0;
    const int labelHeight = 32;
    const int top = int(well.center().y() - (labelHeight + buttonHeight) / 2.0);
    m_status->setGeometry(int(well.x()) + 8, top, qMax(0, int(well.width()) - 16), labelHeight);
    m_measure->move(int(well.center().x() - m_measure->width() / 2.0), top + labelHeight + tokens::space::s8);
    m_again->move(int(well.right()) - m_again->width() - 4, int(well.y()) + 4);
}

void LoudnessView::paintEvent(QPaintEvent *) {
    QPainter p(this);
    const QRectF content = contentRect();

    // Stats: four columns, value over label.
    static const char *const keys[] = {"M", "S", "I", "LRA"};
    static const char *const labels[] = {"M", "S", "I", "LRA LU"};
    const double column = (content.width() - 3 * tokens::space::s4) / 4;
    for (int i = 0; i < 4; ++i) {
        const double x = content.x() + i * (column + tokens::space::s4);
        p.setFont(tokens::font(tokens::monoLarge));
        p.setPen(tokens::color(tokens::ink));
        p.drawText(QRectF(x, 2, column, 20), Qt::AlignLeft | Qt::AlignVCenter, formatStat(QString::fromLatin1(keys[i])));
        p.setFont(tokens::font(tokens::meta));
        p.setPen(tokens::color(tokens::ink3));
        p.drawText(QRectF(x, 22, column, 16), Qt::AlignLeft | Qt::AlignVCenter, QString::fromLatin1(labels[i]));
    }

    // History.
    const QRectF well = historyRect();
    p.setRenderHint(QPainter::Antialiasing, true);
    QPainterPath wellPath;
    wellPath.addRoundedRect(well, tokens::radius::small, tokens::radius::small);
    p.fillPath(wellPath, tokens::color(tokens::well));
    if (measured()) {
        p.save();
        p.setClipPath(wellPath);
        const double sx = well.width() / kViewWidth;
        QPen dashed(QColor(0x90, 0x90, 0x90), 1);
        dashed.setDashPattern({3, 3});
        p.setPen(dashed);
        p.drawLine(QPointF(well.x() + kPlotLeft * sx, targetLineY()), QPointF(well.x() + (kPlotLeft + kPlotWidth) * sx, targetLineY()));
        QPen curve(tokens::color(tokens::ink2), 1.25);
        curve.setJoinStyle(Qt::RoundJoin);
        p.setPen(curve);
        for (const QPolygonF &segment : segments()) {
            if (segment.size() == 1) p.drawPoint(segment.first());
            else p.drawPolyline(segment);
        }
        if (const std::optional<double> x = playheadX()) {
            p.setPen(QPen(tokens::color(tokens::playhead), 1));
            p.drawLine(QPointF(*x, well.y() + 2), QPointF(*x, well.y() + 118));
        }
        p.restore();
    }
    p.setRenderHint(QPainter::Antialiasing, false);

    // Legend.
    p.setFont(tokens::font(tokens::meta));
    p.setPen(tokens::color(tokens::ink3));
    const QRectF legend(content.x(), kLegendTop, content.width(), 16);
    p.drawText(legend, Qt::AlignLeft | Qt::AlignVCenter, legendText());
    p.drawText(legend, Qt::AlignRight | Qt::AlignVCenter, QStringLiteral("%1 LUFS").arg(whole(target())));

    // True peak, per channel.
    static const char *const channels[] = {"L", "R"};
    p.setFont(tokens::font(tokens::monoSmall));
    for (int channel = 0; channel < 2; ++channel) {
        const double top = kTruePeakTop + channel * (kTruePeakRow + 4);
        p.setPen(tokens::color(tokens::ink3));
        p.drawText(QRectF(content.x(), top, kChannelWidth, kTruePeakRow), Qt::AlignLeft | Qt::AlignVCenter,
                   QString::fromLatin1(channels[channel]));
        p.setRenderHint(QPainter::Antialiasing, true);
        p.setPen(Qt::NoPen);
        p.setBrush(tokens::color(tokens::track));
        p.drawRoundedRect(truePeakBar(channel), 2, 2);
        const QRectF fill = truePeakFill(channel);
        if (fill.width() > 0) {
            p.setBrush(tokens::color(tokens::track2));
            p.drawRoundedRect(fill, 2, 2);
        }
        p.setRenderHint(QPainter::Antialiasing, false);
        const QRectF over = truePeakOvershoot(channel);
        if (over.width() > 0) p.fillRect(over, tokens::color(tokens::neg));
        p.fillRect(truePeakTick(channel), tokens::color(tokens::ink));
        const std::optional<double> value = truePeak(channel);
        p.setPen(tokens::color(value && *value > limit() ? tokens::neg : tokens::ink2));
        p.drawText(QRectF(content.right() - kValueWidth, top, kValueWidth, kTruePeakRow), Qt::AlignRight | Qt::AlignVCenter,
                   formatStat(channel == 0 ? QStringLiteral("TP L") : QStringLiteral("TP R")));
    }
}

} // namespace kerf
