#include "source_monitor_panel.h"
#include "editor_common.h"
#include "editor_theme.h"

#include <QAction>
#include <QHBoxLayout>
#include <QLabel>
#include <QMouseEvent>
#include <QPainter>
#include <QToolButton>
#include <QVBoxLayout>

#include <algorithm>
#include <cmath>

namespace kerf {

namespace {

// A media rate as the snapshot prints it: `24`, `29.97`, or `30000/1001`.
double parseRate(const QString &rate) {
    const QStringList parts = rate.split(QLatin1Char('/'));
    const double value = parts.size() == 2 && parts.at(1).toDouble() > 0.0
        ? parts.at(0).toDouble() / parts.at(1).toDouble() : rate.toDouble();
    return value > 0.0 ? value : 24.0;
}

} // namespace

class SourceMonitorPanel::FrameView final : public QWidget {
public:
    using QWidget::QWidget;
    QImage frame;
    QString message = QStringLiteral("Double-click media in the Project panel to open it here");

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter painter(this);
        painter.fillRect(rect(), editorTheme().viewerSurround);
        if (frame.isNull()) {
            // On the stage, which follows the appearance.
            painter.setPen(editorTheme().textSecondary);
            painter.drawText(rect().adjusted(16, 0, -16, 0), Qt::AlignCenter | Qt::TextWordWrap, message);
            return;
        }
        const QSize size = frame.size().scaled(this->size(), Qt::KeepAspectRatio);
        const QRect target(QPoint((width() - size.width()) / 2, (height() - size.height()) / 2), size);
        painter.setRenderHint(QPainter::SmoothPixmapTransform);
        painter.drawImage(target, frame);
    }
};

// The media's length with the marked range and the position.
class SourceMonitorPanel::ScrubBar final : public QWidget {
public:
    explicit ScrubBar(SourceMonitorPanel *panel) : m_panel(panel) {
        setFixedHeight(18);
        setCursor(Qt::PointingHandCursor);
    }

protected:
    void paintEvent(QPaintEvent *) override {
        const EditorTheme &theme = editorTheme();
        QPainter painter(this);
        const QRect track = rect().adjusted(8, 6, -8, -6);
        painter.fillRect(track, theme.scrubTrack);
        const double duration = m_panel->m_duration;
        if (duration <= 0.0) return;
        const auto x = [&](double seconds) { return track.left() + seconds / duration * track.width(); };
        painter.fillRect(QRectF(x(m_panel->m_in), track.top(), x(m_panel->m_out) - x(m_panel->m_in), track.height()),
                         theme.inOutRange);
        // The position is cut out of the track with a pixel of the panel on
        // each side, so it reads over the marked range, an accent wash too.
        const qreal at = x(m_panel->m_position);
        painter.fillRect(QRectF(at - 2, 2, 4, height() - 4), theme.panel);
        painter.setPen(QPen(theme.playhead, 2));
        painter.drawLine(QPointF(at, 2), QPointF(at, height() - 2));
    }
    void mousePressEvent(QMouseEvent *event) override { seekTo(event->position().x()); }
    void mouseMoveEvent(QMouseEvent *event) override {
        if (event->buttons() & Qt::LeftButton) seekTo(event->position().x());
    }

private:
    void seekTo(qreal pointer) {
        const QRect track = rect().adjusted(8, 6, -8, -6);
        const double fraction = std::clamp((pointer - track.left()) / std::max(1, track.width()), 0.0, 1.0);
        m_panel->seek(fraction * m_panel->m_duration);
    }
    SourceMonitorPanel *m_panel;
};

SourceMonitorPanel::SourceMonitorPanel(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("sourceMonitor"));
    setAttribute(Qt::WA_StyledBackground);
    setFocusPolicy(Qt::StrongFocus);
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    m_view = new FrameView;
    m_view->setMinimumSize(160, 90);
    layout->addWidget(m_view, 1);
    m_scrub = new ScrubBar(this);
    layout->addWidget(m_scrub);
    auto *bar = new QWidget;
    bar->setObjectName(QStringLiteral("transport"));
    bar->setAttribute(Qt::WA_StyledBackground);
    bar->setFixedHeight(28);
    auto *row = new QHBoxLayout(bar);
    row->setContentsMargins(10, 0, 10, 0);
    m_time = new QLabel(timelineTimecode(0.0, m_fps));
    m_time->setObjectName(QStringLiteral("transportTime"));
    m_time->setMinimumWidth(110);
    row->addWidget(m_time);
    row->addStretch(1);
    m_buttons = new QWidget;
    auto *buttons = new QHBoxLayout(m_buttons);
    buttons->setContentsMargins(0, 0, 0, 0);
    buttons->setSpacing(2);
    row->addWidget(m_buttons);
    row->addStretch(1);
    m_range = new QLabel;
    m_range->setObjectName(QStringLiteral("transportDuration"));
    m_range->setMinimumWidth(110);
    m_range->setAlignment(Qt::AlignRight | Qt::AlignVCenter);
    m_range->setToolTip(QStringLiteral("Marked length"));
    row->addWidget(m_range);
    layout->addWidget(bar);
    connect(&m_decoder, &QProcess::finished, this, &SourceMonitorPanel::decoded);
    connect(&m_decoder, &QProcess::errorOccurred, this, [this](QProcess::ProcessError error) {
        if (error != QProcess::FailedToStart) return;
        m_view->message = QStringLiteral("FFmpeg is needed to show source frames");
        m_view->update();
    });
    updateLabels();
}

SourceMonitorPanel::~SourceMonitorPanel() {
    m_decoder.kill();
    m_decoder.waitForFinished(1000);
}

QImage SourceMonitorPanel::frame() const {
    return m_view->frame;
}

void SourceMonitorPanel::setActions(const QList<QAction *> &actions) {
    for (QAction *action : actions) {
        auto *button = new QToolButton;
        button->setObjectName(QStringLiteral("transportButton"));
        button->setDefaultAction(action);
        // The Program monitor's transport metrics: 14-point symbols in 18px buttons.
        button->setIconSize(QSize(14, 14));
        button->setFocusPolicy(Qt::NoFocus);
        m_buttons->layout()->addWidget(button);
    }
}

void SourceMonitorPanel::open(const QJsonObject &source, const QString &path) {
    m_source = source;
    m_mediaId = source.value(QStringLiteral("id")).toString();
    m_path = path;
    m_duration = std::max(0.0, source.value(QStringLiteral("duration")).toDouble());
    m_fps = parseRate(source.value(QStringLiteral("rate")).toString());
    m_hasVideo = source.contains(QStringLiteral("width"));
    m_in = 0.0;
    m_out = m_duration;
    m_view->frame = QImage();
    m_view->message = m_hasVideo ? QStringLiteral("Loading…") : QStringLiteral("%1\nAudio").arg(sourceName(source));
    m_position = -1.0;
    seek(0.0);
    setFocus();
}

void SourceMonitorPanel::close() {
    m_source = {};
    m_mediaId.clear();
    m_duration = m_position = m_in = m_out = 0.0;
    m_view->frame = QImage();
    m_view->message = QStringLiteral("Double-click media in the Project panel to open it here");
    m_view->update();
    updateLabels();
}

void SourceMonitorPanel::seek(double seconds) {
    if (!isOpen()) return;
    const double frame = 1.0 / m_fps;
    // Stay on frame starts, before the media's last frame.
    const double target = std::clamp(std::round(seconds * m_fps) / m_fps, 0.0, std::max(0.0, m_duration - frame));
    if (target == m_position) return;
    m_position = target;
    updateLabels();
    if (m_hasVideo) decode(target);
}

void SourceMonitorPanel::step(int frames) {
    seek(m_position + frames / m_fps);
}

void SourceMonitorPanel::markIn() {
    if (!isOpen()) return;
    m_in = m_position;
    if (m_out <= m_in) m_out = m_duration;
    updateLabels();
}

void SourceMonitorPanel::markOut() {
    if (!isOpen()) return;
    // The out mark includes the frame on screen.
    m_out = std::min(m_duration, m_position + 1.0 / m_fps);
    if (m_in >= m_out) m_in = 0.0;
    updateLabels();
}

void SourceMonitorPanel::clearMarks() {
    m_in = 0.0;
    m_out = m_duration;
    updateLabels();
}

void SourceMonitorPanel::updateLabels() {
    m_time->setText(timelineTimecode(std::max(0.0, m_position), m_fps));
    m_range->setText(isOpen() ? timelineTimecode(m_out - m_in, m_fps) : QString());
    m_scrub->update();
    emit changed();
}

void SourceMonitorPanel::decode(double seconds) {
    if (m_decoder.state() != QProcess::NotRunning) {
        m_hasPending = true;
        m_pending = seconds;
        return;
    }
    m_hasPending = false;
    m_decoder.start(QStringLiteral("ffmpeg"), {QStringLiteral("-nostdin"), QStringLiteral("-v"), QStringLiteral("error"),
                                               QStringLiteral("-ss"), QString::number(seconds, 'f', 6),
                                               QStringLiteral("-i"), m_path, QStringLiteral("-frames:v"), QStringLiteral("1"),
                                               QStringLiteral("-vf"), QStringLiteral("scale=960:-2"),
                                               QStringLiteral("-f"), QStringLiteral("image2pipe"), QStringLiteral("-vcodec"),
                                               QStringLiteral("png"), QStringLiteral("pipe:1")});
}

void SourceMonitorPanel::decoded(int exitCode, QProcess::ExitStatus status) {
    const QByteArray bytes = m_decoder.readAllStandardOutput();
    if (status == QProcess::NormalExit && exitCode == 0) {
        const QImage image = QImage::fromData(bytes, "PNG");
        if (!image.isNull()) m_view->frame = image;
    } else if (m_view->frame.isNull()) {
        m_view->message = QStringLiteral("This media cannot be shown");
    }
    m_view->update();
    if (m_hasPending) decode(m_pending);
}

} // namespace kerf
