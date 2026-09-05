#include "bridge.h"

#include <QAction>
#include <QPaintEvent>
#include <QMimeData>
#include <QDragEnterEvent>
#include <QDropEvent>
#include <QUrl>
#include <QHash>
#include <QApplication>
#include <QComboBox>
#include <QDateTime>
#include <QDir>
#include <QDoubleSpinBox>
#include <QFileDialog>
#include <QFileInfo>
#include <QFormLayout>
#include <QFrame>
#include <QGroupBox>
#include <QHBoxLayout>
#include <QImage>
#include <QLinearGradient>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QKeySequence>
#include <QKeyEvent>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QMainWindow>
#include <QMenuBar>
#include <QMessageBox>
#include <QPainter>
#include <QPainterPath>
#include <QPolygonF>
#include <QProgressBar>
#include <QPushButton>
#include <QScrollArea>
#include <QSignalBlocker>
#include <QSlider>
#include <QSplitter>
#include <QStatusBar>
#include <QStyle>
#include <QTabWidget>
#include <QTextEdit>
#include <QTextCursor>
#include <QToolBar>
#include <QTimer>
#include <QToolButton>
#include <QVBoxLayout>
#include <QVector>
#include <QWidget>
#include <QFontMetrics>

#include <algorithm>
#include <cmath>
#include <memory>

namespace {

constexpr int kTimelineLabelWidth = 126;
constexpr int kRulerHeight = 28;
constexpr int kTrackHeight = 52;
constexpr int kMinimumTimelineWidth = 760;

QString prettyTime(double seconds) {
    seconds = std::max(0.0, seconds);
    const int minutes = static_cast<int>(seconds) / 60;
    const double remainder = seconds - minutes * 60.0;
    return QStringLiteral("%1:%2.%3")
        .arg(minutes, 2, 10, QLatin1Char('0'))
        .arg(static_cast<int>(remainder), 2, 10, QLatin1Char('0'))
        .arg(static_cast<int>(std::round((remainder - std::floor(remainder)) * 100.0)), 2, 10,
             QLatin1Char('0'));
}

QString clipName(const QJsonObject &clip) {
    if (clip.value(QStringLiteral("name")).isString()) {
        return clip.value(QStringLiteral("name")).toString();
    }
    if (clip.value(QStringLiteral("title")).isString()) {
        return clip.value(QStringLiteral("title")).toString();
    }
    const QString id = clip.value(QStringLiteral("id")).toString();
    return id.isEmpty() ? QStringLiteral("Untitled clip") : id.left(18);
}

QString sourceName(const QJsonObject &source) {
    const QString path = source.value(QStringLiteral("path")).toString();
    return path.isEmpty() ? QStringLiteral("Untitled media") : QFileInfo(path).fileName();
}

QColor trackAccent(const QJsonObject &track, int row) {
    const QString type = track.value(QStringLiteral("type")).toString().toLower();
    if (type.contains(QStringLiteral("audio")) || type.contains(QStringLiteral("sound"))) {
        return QColor("#65b98f");
    }
    if (type.contains(QStringLiteral("title")) || type.contains(QStringLiteral("graphic")) ||
        type.contains(QStringLiteral("overlay"))) {
        return QColor("#a486d4");
    }
    return row == 0 ? QColor("#5c9fe8") : QColor("#6689b9");
}

QJsonObject commandApply(const QJsonObject &command, quint64 revision) {
    return QJsonObject{{QStringLiteral("type"), QStringLiteral("apply")},
                       {QStringLiteral("revision"), QJsonValue::fromVariant(QVariant::fromValue(revision))},
                       {QStringLiteral("command"), command}};
}

class TimelineWidget final : public QWidget {
    Q_OBJECT
public:
    explicit TimelineWidget(QWidget *parent = nullptr) : QWidget(parent) {
        setFocusPolicy(Qt::StrongFocus);
        setMinimumHeight(kRulerHeight + kTrackHeight * 3 + 8);
        setMouseTracking(true);
    }

    void setProject(const QJsonObject &project, double duration, double fps) {
        m_project = project;
        m_duration = std::max(duration, 1.0);
        m_fps = fps > 0.0 ? fps : 24.0;
        updateGeometry();
        update();
    }

    void setSelectedClip(const QString &id) {
        m_selectedClip = id;
        update();
    }

    void setPlayhead(double time) {
        m_playhead = std::clamp(time, 0.0, m_duration);
        update();
    }

    void setZoom(double zoom) {
        m_zoom = std::clamp(zoom, 0.35, 3.5);
        updateGeometry();
        update();
    }

    QSize minimumSizeHint() const override { return QSize(kMinimumTimelineWidth, 190); }

signals:
    void seekRequested(double time);
    void clipSelected(const QString &clipId);
    void moveRequested(const QString &clipId, double timelineStart);
    void trimRequested(const QString &clipId, bool leading, double timelineFrame);

protected:
    int heightForWidth(int) const override {
        return kRulerHeight + std::max<qsizetype>(1, m_project.value(QStringLiteral("tracks")).toArray().size()) *
                   kTrackHeight + 8;
    }

    void paintEvent(QPaintEvent *event) override {
        const QRect visible = event->rect();
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing);
        QLinearGradient timelineGradient(0, 0, 0, height());
        timelineGradient.setColorAt(0.0, QColor("#111318"));
        timelineGradient.setColorAt(1.0, QColor("#0d0f12"));
        p.fillRect(rect(), timelineGradient);

        const int trackCount = static_cast<int>(std::max<qsizetype>(1, m_project.value(QStringLiteral("tracks")).toArray().size()));
        const double pxPerSecond = pixelsPerSecond();
        const int contentWidth = std::max(kMinimumTimelineWidth, kTimelineLabelWidth + static_cast<int>(m_duration * pxPerSecond) + 30);
        if (contentWidth != minimumWidth()) {
            setMinimumWidth(contentWidth);
        }

        p.fillRect(0, 0, kTimelineLabelWidth, height(), QColor("#17191e"));
        p.fillRect(kTimelineLabelWidth, 0, width() - kTimelineLabelWidth, kRulerHeight, QColor("#1a1c22"));
        p.setPen(QColor("#2f343d"));
        p.drawLine(kTimelineLabelWidth, 0, kTimelineLabelWidth, height());

        QFont rulerFont = p.font();
        rulerFont.setPointSize(9);
        p.setFont(rulerFont);
        p.setPen(QColor("#737b88"));
        p.drawText(QRect(12, 0, kTimelineLabelWidth - 20, kRulerHeight), Qt::AlignVCenter,
                   QStringLiteral("TRACKS"));

        const int majorStep = m_duration > 90.0 ? 10 : (m_duration > 36.0 ? 5 : 1);
        const int firstSecond = std::max(0, static_cast<int>(std::floor((visible.left() - kTimelineLabelWidth) / pxPerSecond)) - 1);
        const int lastSecond = std::min(static_cast<int>(std::ceil(m_duration)),
                                        static_cast<int>(std::ceil((visible.right() - kTimelineLabelWidth) / pxPerSecond)) + 1);
        for (int second = firstSecond; second <= lastSecond; ++second) {
            const int x = kTimelineLabelWidth + static_cast<int>(second * pxPerSecond);
            const bool major = second % majorStep == 0;
            p.setPen(major ? QColor("#38404d") : QColor("#272c35"));
            p.drawLine(x, major ? kRulerHeight : kRulerHeight - 5, x, height());
            p.setPen(major ? QColor("#a4abb6") : QColor("#6e7682"));
            if (major) p.drawText(x + 6, 18, prettyTime(second));
        }

        const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
        for (int row = 0; row < trackCount; ++row) {
            const int y = kRulerHeight + row * kTrackHeight;
            if (y + kTrackHeight < visible.top() || y > visible.bottom()) continue;
            p.fillRect(0, y, width(), kTrackHeight,
                       row % 2 == 0 ? QColor("#15171c") : QColor("#121419"));
            p.setPen(QColor("#2a2f38"));
            p.drawLine(0, y + kTrackHeight - 1, width(), y + kTrackHeight - 1);
            const QJsonObject track = row < tracks.size() ? tracks.at(row).toObject() : QJsonObject();
            const QColor accent = trackAccent(track, row);
            QString trackTitle = row == 0 ? QStringLiteral("Video 1") : QStringLiteral("Track %1").arg(row + 1);
            if (!track.isEmpty()) {
                trackTitle = track.value(QStringLiteral("name")).toString();
                if (trackTitle.isEmpty()) trackTitle = track.value(QStringLiteral("type")).toString().toUpper();
                if (trackTitle.isEmpty()) trackTitle = QStringLiteral("Track %1").arg(row + 1);
            }
            const QString type = track.value(QStringLiteral("type")).toString().toLower();
            const QString laneKind = type.contains(QStringLiteral("audio")) || type.contains(QStringLiteral("sound"))
                                         ? QStringLiteral("A")
                                         : type.contains(QStringLiteral("title")) || type.contains(QStringLiteral("graphic"))
                                               ? QStringLiteral("T")
                                               : QStringLiteral("V");
            p.setPen(QColor("#303640"));
            p.setBrush(QColor("#1b1e24"));
            p.drawRoundedRect(QRectF(10, y + 9, 28, 31), 4, 4);
            p.setPen(accent);
            p.setBrush(Qt::NoBrush);
            p.drawRoundedRect(QRectF(10, y + 9, 28, 31), 4, 4);
            p.setPen(QColor("#d1d6df"));
            p.drawText(QRectF(10, y + 9, 28, 31), Qt::AlignCenter, laneKind);
            p.setPen(QColor("#c8cdd5"));
            p.drawText(48, y + 23, trackTitle);
            p.setPen(QColor("#7c8490"));
            QFont trackFont = p.font();
            trackFont.setPointSize(8);
            p.setFont(trackFont);
            p.drawText(48, y + 38, type.isEmpty() ? QStringLiteral("EDITABLE") : type.toUpper());
            p.setFont(rulerFont);

            const QJsonArray clips = track.value(QStringLiteral("clips")).toArray();
            for (const QJsonValue &value : clips) {
                const QJsonObject clip = value.toObject();
                double start = clip.value(QStringLiteral("timelineStart")).toDouble();
                double duration = clip.value(QStringLiteral("duration")).toDouble();
                if (duration <= 0.0) {
                    const double in = clip.value(QStringLiteral("in")).toDouble();
                    const double out = clip.value(QStringLiteral("out")).toDouble();
                    const double speed = std::max(0.01, clip.value(QStringLiteral("speed")).toDouble(1.0));
                    duration = std::max(0.2, (out - in) / speed);
                }
                const QString id = clip.value(QStringLiteral("id")).toString();
                if (m_dragging && id == m_dragClipId) {
                    if (m_dragLeading) { duration = std::max(1.0 / m_fps, m_dragStart + m_dragDuration - m_previewTime); start = m_previewTime; }
                    else if (m_dragTrailing) duration = std::max(1.0 / m_fps, m_previewTime - m_dragStart);
                    else start = m_previewTime;
                }
                QRectF clipRect(kTimelineLabelWidth + start * pxPerSecond + 2, y + 6,
                                std::max(12.0, duration * pxPerSecond - 4), kTrackHeight - 12);
                if (!clipRect.intersects(visible)) continue;
                const bool selected = id == m_selectedClip;
                const bool hovered = id == m_hoverClipId;
                QColor body = accent.darker(selected ? 215 : hovered ? 265 : 330);
                if (body.value() < 24) body = QColor("#252930");
                QLinearGradient clipGradient(0, clipRect.top(), 0, clipRect.bottom());
                clipGradient.setColorAt(0.0, body.lighter(114));
                clipGradient.setColorAt(1.0, body);
                p.setBrush(clipGradient);
                p.setPen(selected ? QColor("#c6e5ff") : hovered ? accent.lighter(120) : QColor("#3b4652"));
                p.drawRoundedRect(clipRect, 5, 5);
                p.save();
                p.setClipRect(clipRect.adjusted(2, 2, -2, -2));
                p.setPen(QPen(QColor(accent.red(), accent.green(), accent.blue(), 80), 2));
                p.drawLine(clipRect.left() + 5, clipRect.top() + 2, clipRect.right() - 5, clipRect.top() + 2);
                if (type.contains(QStringLiteral("audio")) || type.contains(QStringLiteral("sound"))) {
                    for (int x = static_cast<int>(clipRect.left()) + 7, i = 0; x < clipRect.right() - 5; x += 7, ++i) {
                        const int amplitude = 3 + static_cast<int>(qHash(id + QString::number(i)) % 12);
                        p.setPen(QPen(QColor(accent.red(), accent.green(), accent.blue(), 85), 1));
                        p.drawLine(x, clipRect.center().y() - amplitude / 2, x, clipRect.center().y() + amplitude / 2);
                    }
                } else {
                    p.setPen(QPen(QColor(255, 255, 255, 22), 1));
                    for (int x = static_cast<int>(clipRect.left()) + 7; x < clipRect.right() - 4; x += 14) {
                        p.drawLine(x, clipRect.top() + 5, x, clipRect.bottom() - 5);
                    }
                }
                p.restore();
                if (clipRect.width() > 24.0) {
                    const QFontMetrics metrics(p.font());
                    const QString label = metrics.elidedText(clipName(clip), Qt::ElideRight,
                                                              std::max(1, static_cast<int>(clipRect.width()) - 16));
                    p.setPen(QColor("#f0f4f8"));
                    p.drawText(clipRect.adjusted(9, 0, -7, 0), Qt::AlignVCenter | Qt::TextSingleLine, label);
                }
                p.setPen(QPen(accent.lighter(selected ? 135 : 120), selected ? 2 : 1));
                p.drawLine(clipRect.left() + 5, clipRect.top() + 1, clipRect.right() - 5, clipRect.top() + 1);
                if (selected) {
                    p.setPen(QPen(QColor("#d3eaff"), 2));
                    p.drawLine(clipRect.left(), clipRect.top() + 3, clipRect.left(), clipRect.bottom() - 3);
                    p.drawLine(clipRect.right(), clipRect.top() + 3, clipRect.right(), clipRect.bottom() - 3);
                }
            }
        }

        const int playheadX = kTimelineLabelWidth + static_cast<int>(m_playhead * pxPerSecond);
        p.setPen(QPen(QColor("#ff5d73"), 2));
        p.drawLine(playheadX, 0, playheadX, height());
        p.setBrush(QColor("#ff5d73"));
        p.setPen(Qt::NoPen);
        p.drawPolygon(QPolygonF{QPointF(playheadX - 6, 0), QPointF(playheadX + 6, 0), QPointF(playheadX, 8)});
        if (hasFocus()) {
            p.setPen(QPen(QColor("#536b86"), 1));
            p.setBrush(Qt::NoBrush);
            p.drawRect(rect().adjusted(1, 1, -2, -2));
        }
    }

    void mousePressEvent(QMouseEvent *event) override {
        if (event->button() != Qt::LeftButton) return;
        setFocus();
        const QPoint pos = event->position().toPoint();
        const double time = timeAt(pos.x());
        if (pos.y() < kRulerHeight || pos.x() < kTimelineLabelWidth) {
            m_scrubbing = true;
            emit seekRequested(time);
            return;
        }
        const auto hit = clipAt(pos);
        if (hit.id.isEmpty()) {
            m_scrubbing = true;
            emit seekRequested(time);
            return;
        }
        m_dragging = true;
        m_dragClipId = hit.id;
        m_dragStart = hit.start;
        m_previewTime = hit.start;
        m_dragDuration = hit.duration;
        m_pressTime = time;
        m_dragLeading = pos.x() - hit.rect.left() < 8;
        m_dragTrailing = hit.rect.right() - pos.x() < 8;
        emit clipSelected(hit.id);
    }

    void mouseMoveEvent(QMouseEvent *event) override {
        if (m_scrubbing) { emit seekRequested(timeAt(event->position().x())); return; }
        if (!m_dragging) {
            const ClipHit hit = clipAt(event->position().toPoint());
            if (hit.id != m_hoverClipId) {
                m_hoverClipId = hit.id;
                update();
            }
            if (hit.id.isEmpty()) {
                setCursor(Qt::ArrowCursor);
            } else if (event->position().x() - hit.rect.left() < 8 || hit.rect.right() - event->position().x() < 8) {
                setCursor(Qt::SizeHorCursor);
            } else {
                setCursor(Qt::OpenHandCursor);
            }
            return;
        }
        const double delta = timeAt(event->position().x()) - m_pressTime;
        m_previewTime = std::round(std::max(0.0, (m_dragTrailing ? m_dragStart + m_dragDuration : m_dragStart) + delta) * m_fps) / m_fps;
        setCursor((m_dragLeading || m_dragTrailing) ? Qt::SizeHorCursor : Qt::ClosedHandCursor);
        update();
    }

    void mouseReleaseEvent(QMouseEvent *event) override {
        if (m_scrubbing) { m_scrubbing = false; emit seekRequested(timeAt(event->position().x())); return; }
        if (!m_dragging) return;
        m_dragging = false;
        const double delta = timeAt(event->position().x()) - m_pressTime;
        if (std::abs(delta * pixelsPerSecond()) < QApplication::startDragDistance()) { m_dragClipId.clear(); update(); return; }
        const double target = std::max(0.0, m_dragStart + delta);
        if (m_dragLeading || m_dragTrailing) {
            // Trim commands use the absolute timeline edge under the pointer.
            const auto frame = static_cast<double>(std::llround(timeAt(event->position().x()) * m_fps));
            emit trimRequested(m_dragClipId, m_dragLeading, frame);
        } else {
            emit moveRequested(m_dragClipId, target);
        }
        m_dragClipId.clear();
        setCursor(Qt::ArrowCursor);
        update();
    }

    void leaveEvent(QEvent *) override {
        if (!m_dragging) {
            m_hoverClipId.clear();
            setCursor(Qt::ArrowCursor);
            update();
        }
    }

private:
    struct ClipHit {
        QString id;
        double start = 0.0;
        double duration = 0.0;
        QRectF rect;
    };

    double pixelsPerSecond() const { return 72.0 * m_zoom; }

    double timeAt(int x) const {
        return std::clamp((x - kTimelineLabelWidth) / pixelsPerSecond(), 0.0, m_duration);
    }

    ClipHit clipAt(const QPoint &point) const {
        const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
        for (int row = 0; row < tracks.size(); ++row) {
            const int y = kRulerHeight + row * kTrackHeight;
            if (point.y() < y || point.y() > y + kTrackHeight) continue;
            for (const QJsonValue &value : tracks.at(row).toObject().value(QStringLiteral("clips")).toArray()) {
                const QJsonObject clip = value.toObject();
                const double start = clip.value(QStringLiteral("timelineStart")).toDouble();
                double duration = clip.value(QStringLiteral("duration")).toDouble();
                if (duration <= 0.0) {
                    const double speed = std::max(0.01, clip.value(QStringLiteral("speed")).toDouble(1.0));
                    duration = (clip.value(QStringLiteral("out")).toDouble() - clip.value(QStringLiteral("in")).toDouble()) / speed;
                }
                QRectF rect(kTimelineLabelWidth + start * pixelsPerSecond() + 2, y + 6,
                            std::max(12.0, duration * pixelsPerSecond() - 4), kTrackHeight - 12);
                if (rect.contains(point)) return {clip.value(QStringLiteral("id")).toString(), start, duration, rect};
            }
        }
        return {};
    }

    QJsonObject m_project;
    QString m_selectedClip;
    QString m_hoverClipId;
    QString m_dragClipId;
    double m_duration = 1.0;
    double m_fps = 24.0;
    double m_playhead = 0.0;
    double m_zoom = 1.0;
    double m_dragStart = 0.0;
    double m_dragDuration = 0.0;
    double m_pressTime = 0.0;
    double m_previewTime = 0.0;
    bool m_scrubbing = false;
    bool m_dragging = false;
    bool m_dragLeading = false;
    bool m_dragTrailing = false;
};

class ViewerWidget final : public QWidget {
    Q_OBJECT
public:
    explicit ViewerWidget(QWidget *parent = nullptr) : QWidget(parent) {
        setMinimumSize(480, 260);
        setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Expanding);
    }

    void setFrame(const QImage &frame, double time) {
        m_frame = frame;
        m_time = time;
        update();
    }

    void clearFrame() {
        m_frame = QImage();
        update();
    }

    void setCaption(const QString &caption) {
        m_caption = caption;
        update();
    }

    void setProjectInfo(double width, double height) {
        m_width = width;
        m_height = height;
        update();
    }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing);
        QLinearGradient viewerGradient(0, 0, 0, height());
        viewerGradient.setColorAt(0.0, QColor("#0d0f12"));
        viewerGradient.setColorAt(1.0, QColor("#090a0c"));
        p.fillRect(rect(), viewerGradient);
        p.setPen(QColor("#242830"));
        p.drawLine(18, 34, width() - 18, 34);
        const QRectF target = QRectF(rect()).adjusted(32, 48, -32, -34);
        if (!m_frame.isNull()) {
            QImage scaled = m_frame.scaled(target.size().toSize(), Qt::KeepAspectRatio, Qt::SmoothTransformation);
            const QPointF topLeft(target.center().x() - scaled.width() / 2.0, target.center().y() - scaled.height() / 2.0);
            p.setPen(QColor("#363b45"));
            p.setBrush(QColor("#07080a"));
            p.drawRect(QRectF(topLeft, QSizeF(scaled.size())).adjusted(-1, -1, 1, 1));
            p.drawImage(topLeft, scaled);
        } else {
            const QPointF center = target.center() + QPointF(0, 4);
            p.setPen(QColor("#303641"));
            p.setBrush(QColor("#171a20"));
            p.drawEllipse(center, 27, 27);
            p.setPen(QColor("#56606e"));
            p.setBrush(Qt::NoBrush);
            p.drawEllipse(center, 27, 27);
            p.setPen(Qt::NoPen);
            p.setBrush(QColor("#8c96a4"));
            p.drawPolygon(QPolygonF{center + QPointF(-5, -8), center + QPointF(9, 0), center + QPointF(-5, 8)});
            p.setPen(QColor("#a0a8b5"));
            p.drawText(QRectF(target.left(), center.y() + 42, target.width(), 20), Qt::AlignCenter,
                       QStringLiteral("No preview frame"));
            p.setPen(QColor("#656e7c"));
            p.drawText(QRectF(target.left(), center.y() + 64, target.width(), 20), Qt::AlignCenter,
                       QStringLiteral("Import media to begin"));
        }
        p.setPen(QColor("#a1a8b3"));
        QFont viewerFont = p.font();
        viewerFont.setPointSize(9);
        p.setFont(viewerFont);
        p.drawText(24, 22, QStringLiteral("VIEWER"));
        p.setPen(QColor("#656d7a"));
        p.drawText(92, 22, QStringLiteral("%1 × %2").arg(m_width > 0 ? QString::number(static_cast<int>(m_width)) : QStringLiteral("—"),
                                                         m_height > 0 ? QString::number(static_cast<int>(m_height)) : QStringLiteral("—")));
        p.setPen(QColor("#aab2bd"));
        p.drawText(width() - 96, 22, prettyTime(m_time));
        if (!m_caption.isEmpty()) {
            const QRectF captionRect(48, height() - 64, width() - 96, 34);
            p.setPen(Qt::NoPen);
            p.setBrush(QColor(10, 12, 16, 225));
            p.drawRoundedRect(captionRect, 4, 4);
            p.setPen(QColor("#596474"));
            p.setBrush(Qt::NoBrush);
            p.drawRoundedRect(captionRect, 4, 4);
            p.setPen(QColor("#f4f6fa"));
            p.drawText(captionRect.adjusted(10, 0, -10, 0), Qt::AlignCenter | Qt::TextSingleLine, m_caption);
        }
    }

private:
    QImage m_frame;
    double m_time = 0.0;
    double m_width = 0.0;
    double m_height = 0.0;
    QString m_caption;
};

class MainWindow final : public QMainWindow {
    Q_OBJECT
public:
    MainWindow(const QString &projectDirectory, const QString &cutdBinary, QWidget *parent = nullptr)
        : QMainWindow(parent), m_projectDirectory(projectDirectory), m_cutdBinary(cutdBinary) {
        setWindowTitle(QStringLiteral("Cut — Editor"));
        resize(1500, 940);
        setMinimumSize(1080, 700);
        buildUi();
        setupActions();
        m_bridge = cut_bridge_new(m_projectDirectory.toUtf8().constData(), m_cutdBinary.toUtf8().constData());
        if (!m_bridge) {
            showError(QStringLiteral("Could not connect to cutd. Check the project directory and cutd binary."));
        } else {
            addActivity(QStringLiteral("system"), QStringLiteral("Connected to cutd"));
            sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("refresh")}});
        }
        m_pollTimer.setInterval(16);
        connect(&m_pollTimer, &QTimer::timeout, this, &MainWindow::pollBridge);
        m_pollTimer.start();
    }

    ~MainWindow() override {
        if (m_bridge) cut_bridge_free(m_bridge);
    }

private slots:
    void pollBridge() {
        flushSeek();
        if (!m_bridge) return;
        for (int count = 0; count < 32; ++count) {
            char *raw = cut_bridge_poll(m_bridge);
            if (!raw) break;
            const QByteArray bytes(raw);
            cut_bridge_string_free(raw);
            const QJsonDocument doc = QJsonDocument::fromJson(bytes);
            if (doc.isObject()) handleEvent(doc.object());
        }
        CutFrame frame{};
        if (cut_bridge_frame(m_bridge, &frame) && frame.pixels && frame.length > 0 && frame.width > 0 && frame.height > 0) {
            const qsizetype expected = static_cast<qsizetype>(frame.width) * static_cast<qsizetype>(frame.height) * 4;
            if (expected > 0 && frame.length >= static_cast<size_t>(expected)) {
                QImage image(frame.pixels, static_cast<int>(frame.width), static_cast<int>(frame.height),
                             static_cast<int>(frame.width * 4), QImage::Format_RGBA8888);
                m_viewer->setFrame(image.copy(), frame.time);
            }
        }
    }

    void importMedia() {
        const QString path = QFileDialog::getOpenFileName(this, QStringLiteral("Import media"), QString(),
                                                           QStringLiteral("Media files (*.mov *.mp4 *.m4v *.mkv *.webm *.wav *.mp3);;All files (*)"));
        if (path.isEmpty()) return;
        sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("import")},
                               {QStringLiteral("path"), path},
                               {QStringLiteral("revision"), QJsonValue::fromVariant(QVariant::fromValue(m_revision))}});
        addActivity(QStringLiteral("import"), QStringLiteral("Importing %1").arg(QFileInfo(path).fileName()));
    }

    void exportProject() {
        const QString quality = m_quality->currentData().toString();
        sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("export")}, {QStringLiteral("quality"), quality}});
        addActivity(QStringLiteral("export"), QStringLiteral("Export started (%1)").arg(m_quality->currentText()));
    }

    void renderPreview() {
        sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("preview")}});
        addActivity(QStringLiteral("preview"), QStringLiteral("Rendering preview"));
        m_previewButton->setEnabled(false);
    }

    void addTrack() {
        const QString id = QStringLiteral("video-%1").arg(QDateTime::currentMSecsSinceEpoch());
        const QJsonObject track{{QStringLiteral("id"), id}, {QStringLiteral("type"), QStringLiteral("video")},
                                {QStringLiteral("name"), QStringLiteral("Video %1").arg(m_trackCount + 1)},
                                {QStringLiteral("clips"), QJsonArray{}}};
        sendSimple(commandApply(QJsonObject{{QStringLiteral("type"), QStringLiteral("add_track")}, {QStringLiteral("track"), track}}, m_revision));
    }

    void undo() { sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("undo")}, {QStringLiteral("revision"), revisionValue()}}); }
    void redo() { sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("redo")}, {QStringLiteral("revision"), revisionValue()}}); }
    void refresh() { sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("refresh")}}); }

    void togglePlayback() {
        if (m_previewRequired || m_duration <= 0) return;
        sendSimple(QJsonObject{{QStringLiteral("type"), m_playing ? QStringLiteral("pause") : QStringLiteral("play")} });
    }

    void queueSeek(double time) {
        m_pendingSeek = std::clamp(time, 0.0, m_duration);
        m_hasPendingSeek = true;
        m_playhead = m_pendingSeek;
        m_timeline->setPlayhead(m_playhead);
        updateTransportTime();
    }

    void selectClip(const QString &id) {
        m_selectedClip = id;
        m_timeline->setSelectedClip(id);
        updateInspector();
    }

    void moveClip(const QString &id, double start) {
        const QJsonObject command{{QStringLiteral("type"), QStringLiteral("place_clip")},
                                  {QStringLiteral("clipId"), id},
                                  {QStringLiteral("timelineFrame"), static_cast<qint64>(std::llround(start * m_fps))}};
        sendSimple(commandApply(command, m_revision));
    }

    void trimClip(const QString &id, bool leading, double frame) {
        const QJsonObject command{{QStringLiteral("type"), QStringLiteral("trim_clip")},
                                  {QStringLiteral("clipId"), id},
                                  {QStringLiteral("edge"), leading ? QStringLiteral("leading") : QStringLiteral("trailing")},
                                  {QStringLiteral("timelineFrame"), static_cast<qint64>(std::llround(frame))}};
        sendSimple(commandApply(command, m_revision));
    }

    void splitClip() {
        if (m_selectedClip.isEmpty()) return;
        const QJsonObject command{{QStringLiteral("type"), QStringLiteral("split_clip")},
                                  {QStringLiteral("clipId"), m_selectedClip},
                                  {QStringLiteral("atFrame"), static_cast<qint64>(std::llround(m_playhead * m_fps))},
                                  {QStringLiteral("newClipId"), QStringLiteral("clip-%1").arg(QDateTime::currentMSecsSinceEpoch())}};
        sendSimple(commandApply(command, m_revision));
    }

    void deleteSelected() {
        if (m_selectedClip.isEmpty()) return;
        const QJsonObject command{{QStringLiteral("type"), QStringLiteral("remove_clip")}, {QStringLiteral("clipId"), m_selectedClip}};
        sendSimple(commandApply(command, m_revision));
    }

    void sendChat() {
        const QString text = m_chatInput->text().trimmed();
        if (text.isEmpty()) return;
        if (!sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("agent")}, {QStringLiteral("text"), text}})) return;
        m_chatInput->clear();
        m_sendButton->setEnabled(false);
        appendConversation(QStringLiteral("You"), text);
        addActivity(QStringLiteral("agent"), QStringLiteral("Agent request sent"));
    }

    void acceptProposal() {
        if (m_proposalId.isEmpty()) return;
        sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("accept")}, {QStringLiteral("proposalId"), m_proposalId}, {QStringLiteral("revision"), revisionValue()}});
    }

    void rejectProposal() {
        if (m_proposalId.isEmpty()) return;
        sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("reject")}, {QStringLiteral("proposalId"), m_proposalId}, {QStringLiteral("revision"), revisionValue()}});
    }

    void clipValueEdited() {
        if (m_selectedClip.isEmpty() || m_updatingInspector) return;
        QJsonObject patch;
        const QString field = sender()->property("clipField").toString();
        const QDoubleSpinBox *spin = qobject_cast<QDoubleSpinBox *>(sender());
        if (!spin || field.isEmpty()) return;
        patch.insert(field, spin->value());
        const QJsonObject command{{QStringLiteral("type"), QStringLiteral("update_clip")},
                                  {QStringLiteral("clipId"), m_selectedClip}, {QStringLiteral("patch"), patch}};
        sendSimple(commandApply(command, m_revision));
    }

protected:
    void keyPressEvent(QKeyEvent *event) override {
        if (event->key() == Qt::Key_S && m_timeline && m_timeline->hasFocus()) {
            splitClip();
            event->accept();
            return;
        }
        QMainWindow::keyPressEvent(event);
    }

private:
    void buildUi() {
        auto *central = new QWidget(this);
        auto *root = new QHBoxLayout(central);
        root->setContentsMargins(10, 10, 10, 10);
        root->setSpacing(10);

        auto *left = new QWidget;
        left->setObjectName(QStringLiteral("panel"));
        left->setMinimumWidth(228);
        left->setMaximumWidth(300);
        auto *leftLayout = new QVBoxLayout(left);
        leftLayout->setContentsMargins(14, 12, 14, 12);
        auto *mediaHeader = new QHBoxLayout;
        auto *mediaTitle = new QLabel(QStringLiteral("MEDIA BIN"));
        mediaTitle->setObjectName(QStringLiteral("eyebrow"));
        auto *importButton = new QToolButton;
        importButton->setText(QStringLiteral("+") );
        importButton->setToolTip(QStringLiteral("Import media"));
        importButton->setObjectName(QStringLiteral("squareButton"));
        connect(importButton, &QToolButton::clicked, this, &MainWindow::importMedia);
        mediaHeader->addWidget(mediaTitle);
        mediaHeader->addStretch();
        mediaHeader->addWidget(importButton);
        leftLayout->addLayout(mediaHeader);
        m_mediaList = new QListWidget;
        m_mediaList->setObjectName(QStringLiteral("mediaList"));
        m_mediaList->setSelectionMode(QAbstractItemView::SingleSelection);
        leftLayout->addWidget(m_mediaList, 1);
        auto *mediaHint = new QLabel(QStringLiteral("Import media to add clips to the timeline."));
        mediaHint->setObjectName(QStringLiteral("muted"));
        mediaHint->setWordWrap(true);
        leftLayout->addWidget(mediaHint);
        auto *trackButton = new QPushButton(QStringLiteral("+  Add video track"));
        trackButton->setObjectName(QStringLiteral("secondaryButton"));
        connect(trackButton, &QPushButton::clicked, this, &MainWindow::addTrack);
        leftLayout->addWidget(trackButton);

        auto *center = new QWidget;
        auto *centerLayout = new QVBoxLayout(center);
        centerLayout->setContentsMargins(0, 0, 0, 0);
        centerLayout->setSpacing(10);
        auto *viewerFrame = new QFrame;
        viewerFrame->setObjectName(QStringLiteral("viewerFrame"));
        auto *viewerLayout = new QVBoxLayout(viewerFrame);
        viewerLayout->setContentsMargins(0, 0, 0, 0);
        m_viewer = new ViewerWidget;
        viewerLayout->addWidget(m_viewer);
        centerLayout->addWidget(viewerFrame, 4);

        auto *transport = new QWidget;
        transport->setObjectName(QStringLiteral("transport"));
        auto *transportLayout = new QHBoxLayout(transport);
        transportLayout->setContentsMargins(12, 6, 12, 6);
        m_playButton = new QToolButton;
        m_playButton->setObjectName(QStringLiteral("playButton"));
        m_playButton->setText(QStringLiteral("▶"));
        m_playButton->setToolTip(QStringLiteral("Play / pause (Space)"));
        connect(m_playButton, &QToolButton::clicked, this, &MainWindow::togglePlayback);
        transportLayout->addWidget(m_playButton);
        auto *back = new QToolButton;
        back->setText(QStringLiteral("−5"));
        connect(back, &QToolButton::clicked, this, [this] { queueSeek(m_playhead - 5); });
        transportLayout->addWidget(back);
        auto *forward = new QToolButton;
        forward->setText(QStringLiteral("+5"));
        connect(forward, &QToolButton::clicked, this, [this] { queueSeek(m_playhead + 5); });
        transportLayout->addWidget(forward);
        m_timeLabel = new QLabel(QStringLiteral("00:00.00 / 00:00.00"));
        m_timeLabel->setObjectName(QStringLiteral("timecode"));
        transportLayout->addWidget(m_timeLabel);
        transportLayout->addStretch();
        transportLayout->addWidget(new QLabel(QStringLiteral("Timeline zoom")));
        m_zoom = new QSlider(Qt::Horizontal);
        m_zoom->setRange(35, 350);
        m_zoom->setValue(100);
        m_zoom->setFixedWidth(130);
        connect(m_zoom, &QSlider::valueChanged, this, [this](int value) { m_timeline->setZoom(value / 100.0); });
        transportLayout->addWidget(m_zoom);
        centerLayout->addWidget(transport);

        auto *timelineFrame = new QFrame;
        timelineFrame->setObjectName(QStringLiteral("timelineFrame"));
        auto *timelineLayout = new QVBoxLayout(timelineFrame);
        timelineLayout->setContentsMargins(0, 0, 0, 0);
        m_timeline = new TimelineWidget;
        auto *timelineScroll = new QScrollArea;
        timelineScroll->setWidgetResizable(true);
        timelineScroll->setFrameShape(QFrame::NoFrame);
        timelineScroll->setHorizontalScrollBarPolicy(Qt::ScrollBarAsNeeded);
        timelineScroll->setWidget(m_timeline);
        timelineLayout->addWidget(timelineScroll);
        centerLayout->addWidget(timelineFrame, 2);

        auto *right = new QWidget;
        right->setObjectName(QStringLiteral("panel"));
        right->setMinimumWidth(326);
        right->setMaximumWidth(410);
        auto *rightLayout = new QVBoxLayout(right);
        rightLayout->setContentsMargins(14, 12, 14, 12);
        auto *agentHeader = new QHBoxLayout;
        auto *agentTitle = new QLabel(QStringLiteral("AGENT"));
        agentTitle->setObjectName(QStringLiteral("eyebrow"));
        m_agentStatus = new QLabel(QStringLiteral("READY"));
        m_agentStatus->setObjectName(QStringLiteral("statusPill"));
        agentHeader->addWidget(agentTitle);
        agentHeader->addStretch();
        agentHeader->addWidget(m_agentStatus);
        rightLayout->addLayout(agentHeader);

        auto *tabs = new QTabWidget;
        tabs->setObjectName(QStringLiteral("rightTabs"));
        rightLayout->addWidget(tabs, 1);
        auto *chatTab = new QWidget;
        auto *chatLayout = new QVBoxLayout(chatTab);
        chatLayout->setContentsMargins(0, 8, 0, 0);
        m_conversation = new QTextEdit;
        m_conversation->setReadOnly(true);
        m_conversation->setObjectName(QStringLiteral("conversation"));
        m_conversation->setHtml(QStringLiteral("<p style='color:#788595'>Describe an edit. Review the proposed changes here.</p>"));
        chatLayout->addWidget(m_conversation, 1);
        auto *chatRow = new QHBoxLayout;
        m_chatInput = new QLineEdit;
        m_chatInput->setPlaceholderText(QStringLiteral("Ask the editor to make a change…"));
        connect(m_chatInput, &QLineEdit::returnPressed, this, &MainWindow::sendChat);
        auto *sendButton = new QPushButton(QStringLiteral("Send"));
        sendButton->setObjectName(QStringLiteral("primaryButton"));
        m_sendButton = sendButton;
        connect(sendButton, &QPushButton::clicked, this, &MainWindow::sendChat);
        chatRow->addWidget(m_chatInput);
        chatRow->addWidget(sendButton);
        auto *abortButton = new QToolButton;
        abortButton->setText(QStringLiteral("Stop"));
        abortButton->setToolTip(QStringLiteral("Stop the current agent request"));
        connect(abortButton, &QToolButton::clicked, this, [this] {
            sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("abort")}});
        });
        chatRow->addWidget(abortButton);
        chatLayout->addLayout(chatRow);

        auto *activityTitle = new QLabel(QStringLiteral("ACTIVITY"));
        activityTitle->setObjectName(QStringLiteral("eyebrow"));
        chatLayout->addWidget(activityTitle);
        m_activityList = new QListWidget;
        m_activityList->setObjectName(QStringLiteral("activityList"));
        m_activityList->setMaximumHeight(96);
        chatLayout->addWidget(m_activityList);

        m_exportProgress = new QProgressBar;
        m_exportProgress->setRange(0, 100);
        m_exportProgress->setValue(0);
        m_exportProgress->setTextVisible(false);
        m_exportProgress->setToolTip(QStringLiteral("Export and preview progress"));
        chatLayout->addWidget(m_exportProgress);

        auto *proposalBox = new QGroupBox(QStringLiteral("PROPOSAL"));
        m_proposalBox = proposalBox;
        proposalBox->setObjectName(QStringLiteral("proposalBox"));
        auto *proposalLayout = new QVBoxLayout(proposalBox);
        m_proposalSummary = new QLabel(QStringLiteral("No pending changes"));
        m_proposalSummary->setWordWrap(true);
        m_proposalSummary->setObjectName(QStringLiteral("proposalSummary"));
        m_proposalDiff = new QLabel;
        m_proposalDiff->setWordWrap(true);
        m_proposalDiff->setObjectName(QStringLiteral("muted"));
        auto *proposalButtons = new QHBoxLayout;
        m_acceptButton = new QPushButton(QStringLiteral("Accept"));
        m_acceptButton->setObjectName(QStringLiteral("primaryButton"));
        m_rejectButton = new QPushButton(QStringLiteral("Reject"));
        m_rejectButton->setObjectName(QStringLiteral("dangerButton"));
        connect(m_acceptButton, &QPushButton::clicked, this, &MainWindow::acceptProposal);
        connect(m_rejectButton, &QPushButton::clicked, this, &MainWindow::rejectProposal);
        proposalButtons->addWidget(m_acceptButton);
        proposalButtons->addWidget(m_rejectButton);
        proposalLayout->addWidget(m_proposalSummary);
        proposalLayout->addWidget(m_proposalDiff);
        proposalLayout->addLayout(proposalButtons);
        chatLayout->addWidget(proposalBox);
        tabs->addTab(chatTab, QStringLiteral("Chat"));

        auto *inspectorTab = new QWidget;
        auto *inspectorLayout = new QVBoxLayout(inspectorTab);
        inspectorLayout->setContentsMargins(0, 8, 0, 0);
        auto *inspectorTitle = new QLabel(QStringLiteral("INSPECTOR"));
        inspectorTitle->setObjectName(QStringLiteral("eyebrow"));
        inspectorLayout->addWidget(inspectorTitle);
        auto *inspector = new QWidget;
        auto *form = new QFormLayout(inspector);
        form->setContentsMargins(0, 0, 0, 0);
        form->setSpacing(6);
        m_inspectorClip = new QLabel(QStringLiteral("Select a clip"));
        m_inspectorClip->setObjectName(QStringLiteral("inspectorClip"));
        form->addRow(m_inspectorClip);
        m_position = inspectorSpin(0.0, 0.0, 86400.0, 0.01, QStringLiteral("timelineStart"));
        m_durationSpin = inspectorSpin(1.0, 0.01, 86400.0, 0.01, QStringLiteral("duration"));
        m_inSpin = inspectorSpin(0.0, 0.0, 86400.0, 0.01, QStringLiteral("in"));
        m_outSpin = inspectorSpin(1.0, 0.01, 86400.0, 0.01, QStringLiteral("out"));
        m_speedSpin = inspectorSpin(1.0, 0.01, 20.0, 0.01, QStringLiteral("speed"));
        m_volumeSpin = inspectorSpin(1.0, 0.0, 2.0, 0.01, QStringLiteral("volume"));
        m_opacitySpin = inspectorSpin(1.0, 0.0, 1.0, 0.01, QStringLiteral("opacity"));
        form->addRow(QStringLiteral("Position"), m_position);
        form->addRow(QStringLiteral("Duration"), m_durationSpin);
        form->addRow(QStringLiteral("Source in"), m_inSpin);
        form->addRow(QStringLiteral("Source out"), m_outSpin);
        form->addRow(QStringLiteral("Speed"), m_speedSpin);
        form->addRow(QStringLiteral("Volume"), m_volumeSpin);
        form->addRow(QStringLiteral("Opacity"), m_opacitySpin);
        inspectorLayout->addWidget(inspector);
        inspectorLayout->addStretch(1);
        tabs->addTab(inspectorTab, QStringLiteral("Inspector"));

        root->addWidget(left, 0);
        root->addWidget(center, 1);
        root->addWidget(right, 0);
        setCentralWidget(central);

        connect(m_timeline, &TimelineWidget::seekRequested, this, &MainWindow::queueSeek);
        connect(m_timeline, &TimelineWidget::clipSelected, this, &MainWindow::selectClip);
        connect(m_timeline, &TimelineWidget::moveRequested, this, &MainWindow::moveClip);
        connect(m_timeline, &TimelineWidget::trimRequested, this, &MainWindow::trimClip);
        updateProposal(QJsonValue());
        updateInspector();
    }

    QDoubleSpinBox *inspectorSpin(double value, double minimum, double maximum, double step, const QString &field) {
        auto *spin = new QDoubleSpinBox;
        spin->setRange(minimum, maximum);
        spin->setSingleStep(step);
        spin->setDecimals(2);
        spin->setValue(value);
        spin->setProperty("clipField", field);
        connect(spin, &QDoubleSpinBox::editingFinished, this, &MainWindow::clipValueEdited);
        return spin;
    }

    void setupActions() {
        auto *file = menuBar()->addMenu(QStringLiteral("File"));
        auto *import = file->addAction(QStringLiteral("Import media…"));
        import->setShortcut(QKeySequence::Open);
        connect(import, &QAction::triggered, this, &MainWindow::importMedia);
        auto *exportAction = file->addAction(QStringLiteral("Export…"));
        exportAction->setShortcut(QKeySequence(QStringLiteral("Ctrl+E")));
        connect(exportAction, &QAction::triggered, this, &MainWindow::exportProject);
        file->addSeparator();
        auto *refreshAction = file->addAction(QStringLiteral("Refresh project"));
        connect(refreshAction, &QAction::triggered, this, &MainWindow::refresh);
        file->addSeparator();
        auto *quit = file->addAction(QStringLiteral("Quit"));
        quit->setShortcut(QKeySequence::Quit);
        connect(quit, &QAction::triggered, qApp, &QApplication::quit);

        auto *edit = menuBar()->addMenu(QStringLiteral("Edit"));
        auto *undoAction = edit->addAction(QStringLiteral("Undo"));
        m_undoAction = undoAction; undoAction->setEnabled(false);
        undoAction->setShortcut(QKeySequence::Undo);
        connect(undoAction, &QAction::triggered, this, &MainWindow::undo);
        auto *redoAction = edit->addAction(QStringLiteral("Redo"));
        m_redoAction = redoAction; redoAction->setEnabled(false);
        redoAction->setShortcut(QKeySequence::Redo);
        connect(redoAction, &QAction::triggered, this, &MainWindow::redo);
        edit->addSeparator();
        auto *deleteAction = edit->addAction(QStringLiteral("Delete selected clip"));
        deleteAction->setShortcut(QKeySequence::Delete);
        deleteAction->setShortcutContext(Qt::WidgetShortcut);
        m_timeline->addAction(deleteAction);
        connect(deleteAction, &QAction::triggered, this, &MainWindow::deleteSelected);
        auto *splitAction = edit->addAction(QStringLiteral("Split at playhead"));
        connect(splitAction, &QAction::triggered, this, &MainWindow::splitClip);

        auto *playback = menuBar()->addMenu(QStringLiteral("Playback"));
        auto *play = playback->addAction(QStringLiteral("Play / Pause"));
        play->setShortcut(Qt::Key_Space);
        play->setShortcutContext(Qt::WidgetShortcut);
        m_timeline->addAction(play);
        connect(play, &QAction::triggered, this, &MainWindow::togglePlayback);

        auto *toolbar = addToolBar(QStringLiteral("Main"));
        toolbar->setMovable(false);
        toolbar->addAction(import);
        toolbar->addSeparator();
        toolbar->addAction(undoAction);
        toolbar->addAction(redoAction);
        toolbar->addSeparator();
        auto *split = toolbar->addAction(QStringLiteral("Split"));
        connect(split, &QAction::triggered, this, &MainWindow::splitClip);
        auto *deleteToolbar = toolbar->addAction(QStringLiteral("Delete"));
        connect(deleteToolbar, &QAction::triggered, this, &MainWindow::deleteSelected);
        toolbar->addSeparator();
        toolbar->addWidget(new QLabel(QStringLiteral("Quality ")));
        m_quality = new QComboBox;
        m_quality->addItem(QStringLiteral("Preview"), QStringLiteral("preview"));
        m_quality->addItem(QStringLiteral("Full"), QStringLiteral("full"));
        toolbar->addWidget(m_quality);
        auto *exportToolbar = toolbar->addAction(QStringLiteral("Export"));
        connect(exportToolbar, &QAction::triggered, this, &MainWindow::exportProject);
        m_previewButton = toolbar->addAction(QStringLiteral("Render preview"));
        m_previewButton->setEnabled(false);
        connect(m_previewButton, &QAction::triggered, this, &MainWindow::renderPreview);
    }

    QJsonValue revisionValue() const { return QJsonValue::fromVariant(QVariant::fromValue(m_revision)); }

    bool sendSimple(const QJsonObject &value) {
        if (!m_bridge) {
            showError(QStringLiteral("Runtime is not connected"));
            return false;
        }
        const QByteArray payload = QJsonDocument(value).toJson(QJsonDocument::Compact);
        if (!cut_bridge_send(m_bridge, payload.constData())) {
            showError(QStringLiteral("Runtime command queue is full; the edit was not sent"));
            return false;
        }
        return true;
    }

    void flushSeek() {
        if (!m_hasPendingSeek) return;
        m_hasPendingSeek = false;
        sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("seek")}, {QStringLiteral("position"), m_pendingSeek}});
    }

    void handleEvent(QJsonObject event) {
        const QString type = event.value(QStringLiteral("type")).toString();
        if (type == QStringLiteral("state")) {
            const QJsonObject snapshot = event.value(QStringLiteral("snapshot")).toObject().isEmpty()
                                              ? event.value(QStringLiteral("state")).toObject()
                                              : event.value(QStringLiteral("snapshot")).toObject();
            if (!snapshot.isEmpty()) applySnapshot(snapshot);
        } else if (type == QStringLiteral("viewer_reset")) {
            m_viewer->clearFrame();
            m_viewer->setCaption(QString());
        } else if (type == QStringLiteral("error")) {
            showError(event.value(QStringLiteral("message")).toString(QStringLiteral("Runtime error")));
        } else if (type == QStringLiteral("agent")) {
            const QString kind = event.value(QStringLiteral("kind")).toString(QStringLiteral("agent"));
            const QString text = event.value(QStringLiteral("text")).toString();
            if (kind == "delta") {
                if (m_agentText.isEmpty()) appendConversation("Cut", text);
                else { QTextCursor cursor(m_conversation->document()); cursor.movePosition(QTextCursor::End); cursor.insertText(text); m_conversation->ensureCursorVisible(); }
                m_agentText += text;
            }
            else if (kind == "done") { if (m_agentText.isEmpty()) appendConversation("Cut", text); m_agentText.clear(); }
            else addActivity(kind, text.left(120));
        } else if (type == QStringLiteral("agent_busy")) {
            const bool busy = event.value(QStringLiteral("busy")).toBool();
            m_agentStatus->setText(busy ? QStringLiteral("THINKING") : QStringLiteral("READY"));
            m_chatInput->setEnabled(!busy);
            m_sendButton->setEnabled(!busy);
        } else if (type == QStringLiteral("clock")) {
            m_playhead = std::clamp(event.value(QStringLiteral("position")).toDouble(), 0.0, m_duration);
            if (event.value(QStringLiteral("duration")).isDouble()) m_duration = std::max(0.0, event.value(QStringLiteral("duration")).toDouble());
            m_timeline->setPlayhead(m_playhead);
            updateTransportTime();
        } else if (type == QStringLiteral("playback")) {
            m_playing = event.value(QStringLiteral("playing")).toBool();
            m_playButton->setText(m_playing ? QStringLiteral("Ⅱ") : QStringLiteral("▶"));
        } else if (type == QStringLiteral("export")) {
            const double progress = event.value(QStringLiteral("progress")).toDouble();
            m_exportProgress->setValue(static_cast<int>(std::clamp(progress, 0.0, 1.0) * 100.0));
            const QString status = event.value(QStringLiteral("status")).toString();
            if (!status.isEmpty()) statusBar()->showMessage(QStringLiteral("Export %1").arg(status));
            if (!event.value(QStringLiteral("path")).toString().isEmpty()) addActivity(QStringLiteral("export"), event.value(QStringLiteral("path")).toString());
        } else if (type == QStringLiteral("preview")) {
            const QString status = event.value(QStringLiteral("status")).toString();
            const QString message = event.value(QStringLiteral("message")).toString();
            if (status == QStringLiteral("required") || status == QStringLiteral("unavailable")) {
                m_previewRequired = true;
                m_previewButton->setEnabled(status == QStringLiteral("required"));
                m_playButton->setEnabled(false);
                m_viewer->clearFrame();
                statusBar()->showMessage(message.isEmpty() ? QStringLiteral("Rendered preview required") : message, 6000);
                addActivity(QStringLiteral("preview"), message.isEmpty() ? QStringLiteral("Native preview unavailable") : message);
            } else if (status == QStringLiteral("running")) {
                m_previewRequired = true;
                m_previewButton->setEnabled(false);
                m_playButton->setEnabled(false);
                m_exportProgress->setValue(static_cast<int>(std::clamp(event.value(QStringLiteral("progress")).toDouble(), 0.0, 1.0) * 100.0));
            } else if (status == QStringLiteral("ready")) {
                m_previewRequired = false;
                m_previewButton->setEnabled(false);
                m_playButton->setEnabled(true);
                m_exportProgress->setValue(0);
                statusBar()->showMessage(QStringLiteral("Preview ready"), 4000);
            }
        } else if (type == QStringLiteral("caption")) {
            m_viewer->setCaption(event.value(QStringLiteral("text")).toString());
        }
    }

    void applySnapshot(const QJsonObject &snapshot) {
        m_snapshot = snapshot;
        m_revision = static_cast<quint64>(snapshot.value(QStringLiteral("revision")).toDouble());
        m_duration = std::max(0.0, snapshot.value(QStringLiteral("duration")).toDouble());
        m_project = snapshot.value(QStringLiteral("project")).toObject();
        // Derive presentation fields once per snapshot, not on every repaint.
        QHash<QString, QJsonObject> sourceById;
        for (const auto &value : m_project.value("sources").toArray()) { const auto source = value.toObject(); sourceById.insert(source.value("id").toString(), source); }
        QJsonArray displayTracks;
        for (const auto &value : m_project.value("tracks").toArray()) {
            auto track = value.toObject(); QJsonArray displayClips;
            for (const auto &clipValue : track.value("clips").toArray()) {
                auto clip = clipValue.toObject(); const auto source = sourceById.value(clip.value("sourceId").toString());
                if (!clip.contains("duration")) clip.insert("duration", std::max(0.0, (clip.value("out").toDouble(source.value("duration").toDouble()) - clip.value("in").toDouble()) / std::max(0.01, clip.value("speed").toDouble(1.0))));
                if (!clip.contains("name")) clip.insert("name", sourceName(source));
                displayClips.append(clip);
            }
            track.insert("clips", displayClips); displayTracks.append(track);
        }
        m_project.insert("tracks", displayTracks);
        m_undoAction->setEnabled(snapshot.value("canUndo").toBool());
        m_redoAction->setEnabled(snapshot.value("canRedo").toBool());
        m_fps = m_project.value(QStringLiteral("fps")).toDouble(24.0);
        const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
        m_trackCount = tracks.size();
        if (!m_selectedClip.isEmpty() && selectedClipObject().isEmpty()) m_selectedClip.clear();
        m_timeline->setProject(m_project, m_duration, m_fps);
        m_timeline->setSelectedClip(m_selectedClip);
        m_viewer->setProjectInfo(m_project.value(QStringLiteral("width")).toDouble(), m_project.value(QStringLiteral("height")).toDouble());
        m_mediaList->clear();
        for (const QJsonValue &value : snapshot.value(QStringLiteral("sources")).toArray()) {
            const QJsonObject source = value.toObject();
            auto *item = new QListWidgetItem(sourceName(source), m_mediaList);
            item->setToolTip(source.value(QStringLiteral("path")).toString());
            item->setData(Qt::UserRole, source.value(QStringLiteral("id")).toString());
        }
        m_exportProgress->setValue(0);
        updateProposal(snapshot.value(QStringLiteral("pendingProposal")));
        updateInspector();
        updateTransportTime();
        statusBar()->showMessage(QStringLiteral("Revision %1  •  %2 tracks  •  %3").arg(m_revision).arg(m_trackCount).arg(m_project.value(QStringLiteral("name")).toString(QStringLiteral("Untitled project"))), 1500);
    }

    QJsonObject selectedClipObject() const {
        const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
        for (const QJsonValue &trackValue : tracks) {
            for (const QJsonValue &clipValue : trackValue.toObject().value(QStringLiteral("clips")).toArray()) {
                const QJsonObject clip = clipValue.toObject();
                if (clip.value(QStringLiteral("id")).toString() == m_selectedClip) return clip;
            }
        }
        return {};
    }

    void updateInspector() {
        m_updatingInspector = true;
        const QJsonObject clip = selectedClipObject();
        const bool hasClip = !clip.isEmpty();
        m_inspectorClip->setText(hasClip ? clipName(clip) : QStringLiteral("Select a clip"));
        auto setSpin = [hasClip](QDoubleSpinBox *spin, double value) {
            spin->setEnabled(hasClip);
            const QSignalBlocker blocker(spin);
            spin->setValue(value);
        };
        setSpin(m_position, clip.value(QStringLiteral("timelineStart")).toDouble());
        setSpin(m_durationSpin, std::max(0.01, clip.value(QStringLiteral("duration")).toDouble(1.0)));
        setSpin(m_inSpin, clip.value(QStringLiteral("in")).toDouble());
        setSpin(m_outSpin, std::max(0.01, clip.value(QStringLiteral("out")).toDouble(1.0)));
        setSpin(m_speedSpin, clip.value(QStringLiteral("speed")).toDouble(1.0));
        setSpin(m_volumeSpin, clip.value(QStringLiteral("volume")).toDouble(1.0));
        setSpin(m_opacitySpin, clip.value(QStringLiteral("opacity")).toDouble(1.0));
        m_updatingInspector = false;
    }

    void updateProposal(const QJsonValue &value) {
        m_proposalId.clear();
        bool pending = value.isObject() && value.toObject().value(QStringLiteral("status")).toString() == QStringLiteral("pending");
        m_acceptButton->setEnabled(pending);
        m_rejectButton->setEnabled(pending);
        m_proposalBox->setVisible(pending);
        if (!pending) {
            m_proposalSummary->setText(QStringLiteral("No pending changes"));
            m_proposalDiff->clear();
            return;
        }
        const QJsonObject proposal = value.toObject();
        m_proposalId = proposal.value(QStringLiteral("proposalId")).toString();
        const bool stale = proposal.value(QStringLiteral("freshness")).toString() == QStringLiteral("stale");
        const QJsonArray diff = proposal.value(QStringLiteral("diff")).toArray();
        m_proposalSummary->setText(QStringLiteral("%1 change%2%3")
                                       .arg(diff.size())
                                       .arg(diff.size() == 1 ? QString() : QStringLiteral("s"))
                                       .arg(stale ? QStringLiteral("  •  stale") : QString()));
        QStringList lines;
        for (int i = 0; i < static_cast<int>(std::min<qsizetype>(4, diff.size())); ++i) {
            const QJsonObject change = diff.at(i).toObject();
            lines << QStringLiteral("%1  %2").arg(change.value(QStringLiteral("kind")).toString(), change.value(QStringLiteral("path")).toString());
        }
        m_proposalDiff->setText(lines.join(QStringLiteral("\n")));
        m_acceptButton->setEnabled(!stale);
    }

    void appendConversation(const QString &speaker, const QString &text) {
        if (text.isEmpty()) return;
        if (m_conversationEmpty) {
            m_conversation->clear();
            m_conversationEmpty = false;
        }
        m_conversation->append(QStringLiteral("<b>%1</b><br/>%2").arg(speaker.toHtmlEscaped(), text.toHtmlEscaped().replace(QStringLiteral("\n"), QStringLiteral("<br/>"))));
        while (m_conversation->document()->blockCount() > 120) {
            QTextCursor cursor(m_conversation->document());
            cursor.movePosition(QTextCursor::Start);
            cursor.select(QTextCursor::BlockUnderCursor);
            cursor.removeSelectedText();
            cursor.deleteChar();
        }
        m_conversation->ensureCursorVisible();
    }

    void addActivity(const QString &kind, const QString &text) {
        if (text.isEmpty()) return;
        m_activityLog << QStringLiteral("%1  %2").arg(kind.toUpper(), text);
        while (m_activityLog.size() > 80) m_activityLog.removeFirst();
        if (m_activityList) {
            m_activityList->addItem(m_activityLog.constLast());
            while (m_activityList->count() > 8) delete m_activityList->takeItem(0);
            m_activityList->scrollToBottom();
        }
    }

    void showError(const QString &message) {
        statusBar()->showMessage(message, 6000);
        appendConversation(QStringLiteral("error"), message);
        addActivity(QStringLiteral("error"), message);
    }

    void updateTransportTime() {
        m_timeLabel->setText(QStringLiteral("%1 / %2").arg(prettyTime(m_playhead), prettyTime(m_duration)));
    }

    QString m_projectDirectory;
    QString m_cutdBinary;
    void *m_bridge = nullptr;
    QTimer m_pollTimer;
    QJsonObject m_snapshot;
    QJsonObject m_project;
    quint64 m_revision = 0;
    int m_trackCount = 0;
    double m_duration = 1.0;
    double m_fps = 24.0;
    double m_playhead = 0.0;
    double m_pendingSeek = 0.0;
    bool m_hasPendingSeek = false;
    bool m_playing = false;
    bool m_previewRequired = false;
    bool m_updatingInspector = false;
    bool m_conversationEmpty = true;
    QString m_agentText;
    QString m_selectedClip;
    QString m_proposalId;
    QStringList m_activityLog;

    ViewerWidget *m_viewer = nullptr;
    TimelineWidget *m_timeline = nullptr;
    QListWidget *m_mediaList = nullptr;
    QTextEdit *m_conversation = nullptr;
    QLineEdit *m_chatInput = nullptr;
    QPushButton *m_sendButton = nullptr;
    QListWidget *m_activityList = nullptr;
    QLabel *m_agentStatus = nullptr;
    QLabel *m_proposalSummary = nullptr;
    QLabel *m_proposalDiff = nullptr;
    QPushButton *m_acceptButton = nullptr;
    QPushButton *m_rejectButton = nullptr;
    QGroupBox *m_proposalBox = nullptr;
    QLabel *m_inspectorClip = nullptr;
    QDoubleSpinBox *m_position = nullptr;
    QDoubleSpinBox *m_durationSpin = nullptr;
    QDoubleSpinBox *m_inSpin = nullptr;
    QDoubleSpinBox *m_outSpin = nullptr;
    QDoubleSpinBox *m_speedSpin = nullptr;
    QDoubleSpinBox *m_volumeSpin = nullptr;
    QDoubleSpinBox *m_opacitySpin = nullptr;
    QToolButton *m_playButton = nullptr;
    QLabel *m_timeLabel = nullptr;
    QSlider *m_zoom = nullptr;
    QComboBox *m_quality = nullptr;
    QProgressBar *m_exportProgress = nullptr;
    QAction *m_undoAction = nullptr;
    QAction *m_redoAction = nullptr;
    QAction *m_previewButton = nullptr;
};

} // namespace

#include "main.moc"

int main(int argc, char **argv) {
    QApplication app(argc, argv);
    QApplication::setApplicationName(QStringLiteral("Cut"));
    QApplication::setOrganizationName(QStringLiteral("Cut"));
    app.setStyle(QStringLiteral("Fusion"));
    app.setStyleSheet(QStringLiteral(
        "* { color: #d8dbe1; font-family: -apple-system, BlinkMacSystemFont, 'Helvetica Neue', Arial, sans-serif; font-size: 12px; }"
        "QMainWindow, QWidget { background: #0d0e10; } QLabel { background: transparent; }"
        "QMenuBar { background: #17181b; border-bottom: 1px solid #292b30; padding: 4px 9px; }"
        "QMenuBar::item { padding: 5px 10px; border-radius: 4px; } QMenuBar::item:selected { background: #2a2d33; }"
        "QMenu { background: #1b1d21; border: 1px solid #363940; padding: 4px; }"
        "QMenu::item { padding: 7px 28px 7px 12px; border-radius: 3px; } QMenu::item:selected { background: #315b87; }"
        "QToolBar { background: #17181b; border: 0; border-bottom: 1px solid #292b30; spacing: 6px; padding: 6px 10px; }"
        "QToolBar::separator { background: #34363d; width: 1px; margin: 4px 3px; }"
        "QToolButton, QPushButton { background: #26282d; border: 1px solid #3b3e46; border-radius: 5px; padding: 6px 11px; min-height: 16px; }"
        "QToolButton:hover, QPushButton:hover { background: #30343b; border-color: #505661; }"
        "QToolButton:pressed, QPushButton:pressed { background: #344f6b; border-color: #6795bd; }"
        "QToolButton:disabled, QPushButton:disabled { color: #626873; background: #1b1d21; border-color: #292c32; }"
        "QPushButton#primaryButton { background: #356fae; border-color: #5c99cf; color: #f4f8fc; font-weight: 600; }"
        "QPushButton#primaryButton:hover { background: #4383c0; } QPushButton#dangerButton { background: #4a292e; border-color: #86505a; }"
        "QPushButton#dangerButton:hover { background: #60363d; } QPushButton#secondaryButton { background: #1b1e23; }"
        "QToolButton#squareButton { padding: 2px 9px; font-size: 18px; color: #c3ccd7; }"
        "QToolButton#playButton { background: #326fae; border-color: #66a2d8; border-radius: 6px; color: #f5f9fc; font-size: 15px; padding: 5px 16px; }"
        "QToolButton#playButton:hover { background: #4186c5; }"
        "QWidget#panel { background: #181a1e; border: 1px solid #2c2f36; border-radius: 6px; }"
        "QFrame#viewerFrame, QFrame#timelineFrame { background: #0b0c0f; border: 1px solid #2c2f36; border-radius: 6px; }"
        "QWidget#transport { background: #181a1e; border: 1px solid #2c2f36; border-radius: 6px; }"
        "QLabel#eyebrow { color: #8b929d; font-size: 10px; font-weight: 700; letter-spacing: 1px; }"
        "QLabel#muted { color: #7a818c; font-size: 11px; } QLabel#statusPill { color: #71cd99; font-size: 10px; font-weight: 700; }"
        "QLabel#timecode { color: #d5d9e0; font-family: Menlo, Monaco, monospace; font-size: 11px; }"
        "QLabel#inspectorClip { color: #f1f3f6; font-weight: 600; padding: 5px 0; }"
        "QListWidget, QTextEdit { background: #111316; border: 1px solid #30343c; border-radius: 5px; padding: 5px; selection-background-color: #294e75; }"
        "QListWidget::item { padding: 9px 8px; border-bottom: 1px solid #25282e; } QListWidget::item:hover { background: #1c2026; }"
        "QListWidget::item:selected { background: #294e75; color: #f3f7fb; }"
        "QTextEdit { selection-background-color: #315f8d; } QLineEdit, QDoubleSpinBox, QComboBox { background: #202328; border: 1px solid #3a3e47; border-radius: 4px; padding: 6px; selection-background-color: #315f8d; }"
        "QLineEdit:hover, QDoubleSpinBox:hover, QComboBox:hover { border-color: #505660; } QLineEdit:focus, QDoubleSpinBox:focus, QComboBox:focus { border-color: #6194c3; background: #232830; }"
        "QGroupBox#proposalBox { border: 1px solid #465263; border-radius: 5px; margin-top: 12px; padding: 12px 10px 10px; background: #1b2027; }"
        "QGroupBox#proposalBox::title { subcontrol-origin: margin; left: 9px; padding: 0 5px; color: #a8b5c7; font-size: 10px; font-weight: 700; }"
        "QProgressBar { background: #202329; border: 0; border-radius: 2px; height: 4px; } QProgressBar::chunk { background: #5e9fcf; border-radius: 2px; }"
        "QTabWidget::pane { border: 0; background: transparent; } QTabBar::tab { color: #838b97; padding: 8px 15px; border-bottom: 2px solid transparent; }"
        "QTabBar::tab:hover { color: #d7dce4; } QTabBar::tab:selected { color: #f1f4f8; border-bottom-color: #5f9bd0; }"
        "QSlider::groove:horizontal { background: #101215; border: 1px solid #292c32; height: 5px; border-radius: 3px; }"
        "QSlider::handle:horizontal { background: #6a9ec7; border: 1px solid #8bb7d8; width: 11px; margin: -4px 0; border-radius: 6px; }"
        "QScrollBar:vertical, QScrollBar:horizontal { background: #101215; border: 0; } QScrollBar:vertical { width: 10px; } QScrollBar:horizontal { height: 10px; }"
        "QScrollBar::handle:vertical, QScrollBar::handle:horizontal { background: #3b424d; border-radius: 4px; min-height: 20px; min-width: 20px; }"
        "QScrollBar::handle:vertical:hover, QScrollBar::handle:horizontal:hover { background: #596675; } QScrollBar::add-line, QScrollBar::sub-line { width: 0; height: 0; }"
        "QStatusBar { background: #111215; color: #89919d; border-top: 1px solid #25282e; }"
    ));

    QString project;
    QString cutd;
    QString screenshotPath;
    int quitAfterMs = 0;
    QStringList positional;
    for (int i = 1; i < argc; ++i) {
        const QString argument = QString::fromUtf8(argv[i]);
        if (argument == QStringLiteral("--screenshot") && i + 1 < argc) {
            screenshotPath = QString::fromUtf8(argv[++i]);
        } else if (argument.startsWith(QStringLiteral("--screenshot="))) {
            screenshotPath = argument.mid(13);
        } else if (argument == QStringLiteral("--quit-after-ms") && i + 1 < argc) {
            quitAfterMs = QString::fromUtf8(argv[++i]).toInt();
        } else if (argument.startsWith(QStringLiteral("--quit-after-ms="))) {
            quitAfterMs = argument.mid(16).toInt();
        } else {
            positional << argument;
        }
    }
    project = positional.value(0, qEnvironmentVariable("CUT_PROJECT_DIR"));
    if (project.isEmpty()) project = qEnvironmentVariable("CUT_PROJECT_DIRECTORY");
    if (project.isEmpty()) project = QDir::currentPath();
    cutd = positional.value(1, qEnvironmentVariable("CUTD_BIN"));
    if (cutd.isEmpty()) cutd = qEnvironmentVariable("CUTD_BINARY");
    if (cutd.isEmpty()) cutd = QStringLiteral("cutd");
    MainWindow window(project, cutd);
    window.show();
    if (!screenshotPath.isEmpty() || quitAfterMs > 0) {
        const int delay = quitAfterMs > 0 ? quitAfterMs : 500;
        QTimer::singleShot(delay, &app, [&window, &app, screenshotPath] {
            if (!screenshotPath.isEmpty()) window.grab().save(screenshotPath);
            app.quit();
        });
    }
    return app.exec();
}
