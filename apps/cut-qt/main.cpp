#include "bridge.h"
#include "editor_style.h"

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
#include <QProcess>
#include <QQueue>
#include <QSet>
#include <QPushButton>
#include <QScrollArea>
#include <QScrollBar>
#include <QSettings>
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
#include <QFontDatabase>
#ifdef CUT_QT_TESTING
#include <QTest>
#include <QTemporaryDir>
#include <QStandardPaths>
#endif

#include <algorithm>
#include <cmath>
#include <limits>
#include <memory>

namespace {

constexpr int kTimelineLabelWidth = 94;
constexpr int kRulerHeight = 28;
constexpr int kTrackHeight = 56;

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

QSize resolvedCanvasSize(const QJsonObject &project, const QJsonArray &sources) {
    const auto validDimension = [](const QJsonValue &value) {
        const double number = value.toDouble();
        return std::isfinite(number) && number > 0.0;
    };
    const auto dimension = [](double value) {
        return static_cast<int>(std::round(std::clamp(
            value, 2.0, static_cast<double>(std::numeric_limits<int>::max()))));
    };
    if (validDimension(project.value(QStringLiteral("width"))) &&
        validDimension(project.value(QStringLiteral("height")))) {
        return QSize(dimension(project.value(QStringLiteral("width")).toDouble()),
                     dimension(project.value(QStringLiteral("height")).toDouble()));
    }
    QSize largest;
    double largestArea = 0.0;
    for (const auto &value : sources) {
        const QJsonObject source = value.toObject();
        if (!validDimension(source.value(QStringLiteral("width"))) ||
            !validDimension(source.value(QStringLiteral("height")))) continue;
        const double width = source.value(QStringLiteral("width")).toDouble();
        const double height = source.value(QStringLiteral("height")).toDouble();
        if (width * height > largestArea) {
            largestArea = width * height;
            largest = QSize(dimension(width), dimension(height));
        }
    }
    return largest.isValid() ? largest : QSize(1920, 1080);
}

class QualityComboBox final : public QComboBox {
public:
    using QComboBox::QComboBox;

protected:
    void paintEvent(QPaintEvent *event) override {
        QComboBox::paintEvent(event);
        QPainter painter(this);
        painter.setRenderHint(QPainter::Antialiasing);
        painter.setPen(Qt::NoPen);
        painter.setBrush(isEnabled() ? QColor("#a7a7a7") : QColor("#727272"));
        const qreal centerX = width() - 15.0;
        const qreal centerY = height() / 2.0;
        painter.drawPolygon(QPolygonF{{centerX - 4.5, centerY - 2.5},
                                      {centerX + 4.5, centerY - 2.5},
                                      {centerX, centerY + 2.5}});
    }
};

QColor trackAccent(const QJsonObject &track, int row) {
    const QString type = track.value(QStringLiteral("type")).toString().toLower();
    const QString name = track.value(QStringLiteral("name")).toString().toLower();
    const bool music = type.contains(QStringLiteral("music")) || type.contains(QStringLiteral("score")) ||
                       name.contains(QStringLiteral("music")) || name.contains(QStringLiteral("score"));
    if (music) return QColor("#63c96b");
    if (type.contains(QStringLiteral("audio")) || type.contains(QStringLiteral("sound"))) {
        return QColor("#29b6a6");
    }
    if (type.contains(QStringLiteral("title")) || type.contains(QStringLiteral("graphic")) ||
        type.contains(QStringLiteral("overlay"))) {
        return QColor("#a879e8");
    }
    return row == 0 ? QColor("#4d9df4") : QColor("#3978c5");
}

QJsonObject commandApply(const QJsonObject &command, quint64 revision) {
    return QJsonObject{{QStringLiteral("type"), QStringLiteral("apply")},
                       {QStringLiteral("revision"), QJsonValue::fromVariant(QVariant::fromValue(revision))},
                       {QStringLiteral("command"), command}};
}

// Decode visual media references outside the paint path. Missing FFmpeg or an
// unsupported source leaves a plain clip; editing never waits for decoration.
class MediaArtwork final : public QObject {
    Q_OBJECT
public:
    explicit MediaArtwork(QObject *parent = nullptr) : QObject(parent) {
        connect(&m_process, &QProcess::finished, this, [this](int code, QProcess::ExitStatus) {
            if (code == 0) {
                const QImage result = QImage::fromData(m_process.readAllStandardOutput());
                if (!result.isNull()) {
                    (m_current.wave ? waves : frames).insert(m_current.id, result);
                    emit changed();
                }
            }
            m_timeout.stop();
            next();
        });
        connect(&m_process, &QProcess::errorOccurred, this, [this](QProcess::ProcessError error) {
            if (error == QProcess::FailedToStart) { m_timeout.stop(); next(); }
        });
        m_timeout.setSingleShot(true);
        connect(&m_timeout, &QTimer::timeout, &m_process, &QProcess::kill);
    }
    ~MediaArtwork() override {
        disconnect(&m_process, nullptr, this, nullptr);
        m_timeout.stop();
        m_process.kill();
        m_process.waitForFinished(1000);
    }
    void request(const QJsonArray &sources, const QString &directory) {
        for (const auto &value : sources) {
            const auto source = value.toObject();
            const QString id = source.value("id").toString();
            const QString path = QDir(directory).absoluteFilePath(source.value("path").toString());
            const QString key = id + path + QString::number(QFileInfo(path).lastModified().toMSecsSinceEpoch());
            if (m_requested.contains(key)) continue;
            m_requested.insert(key);
            frames.remove(id); waves.remove(id);
            m_queue.enqueue({id, path, false});
            if (source.value("hasAudio").toBool()) m_queue.enqueue({id, path, true});
        }
        if (m_process.state() == QProcess::NotRunning) next();
    }
    QHash<QString, QImage> frames;
    QHash<QString, QImage> waves;
    QIcon icon(const QString &id) const {
        if (frames.contains(id)) return QPixmap::fromImage(frames.value(id));
        QPixmap tile(96, 54);
        tile.fill(QColor("#244c49"));
        QPainter painter(&tile);
        if (waves.contains(id)) {
            painter.setOpacity(0.65);
            painter.drawImage(QRect(0, 10, 96, 34), waves.value(id));
        } else {
            painter.setPen(QColor("#7caaa4"));
            painter.drawText(tile.rect(), Qt::AlignCenter, QStringLiteral("♪"));
        }
        return tile;
    }
signals:
    void changed();
private:
    struct Job { QString id; QString path; bool wave; };
    void next() {
        if (m_queue.isEmpty()) return;
        m_current = m_queue.dequeue();
        QStringList args{"-nostdin", "-v", "error", "-i", m_current.path};
        if (m_current.wave) {
            args << "-filter_complex" << "aformat=channel_layouts=mono,showwavespic=s=2048x48:colors=white:scale=sqrt";
        } else {
            args << "-vf" << "thumbnail,scale=160:90:force_original_aspect_ratio=decrease,pad=160:90:(ow-iw)/2:(oh-ih)/2";
        }
        args << "-frames:v" << "1" << "-an" << "-f" << "image2pipe" << "-vcodec" << "png" << "pipe:1";
        m_process.start(QStringLiteral("ffmpeg"), args);
        m_timeout.start(30000);
    }
    QProcess m_process;
    QTimer m_timeout;
    QQueue<Job> m_queue;
    Job m_current;
    QSet<QString> m_requested;
};

class TimelineWidget final : public QWidget {
    Q_OBJECT
public:
    explicit TimelineWidget(QWidget *parent = nullptr) : QWidget(parent) {
        setFocusPolicy(Qt::StrongFocus);
        updateContentSize();
        setMouseTracking(true);
    }

    void setProject(const QJsonObject &project, double duration, double fps) {
        m_project = project;
        m_duration = std::max(duration, 1.0);
        m_fps = fps > 0.0 ? fps : 24.0;
        updateContentSize();
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
        m_fitWidth = 0;
        updateContentSize();
        update();
    }

    void fitToWidth(int width) {
        m_fitWidth = std::max(kTimelineLabelWidth + 32, width);
        updateContentSize();
        update();
    }

    bool isFitToWidth() const { return m_fitWidth > 0; }
    void setArtwork(MediaArtwork *artwork) {
        m_artwork = artwork;
        connect(artwork, &MediaArtwork::changed, this, QOverload<>::of(&TimelineWidget::update));
    }

    void clearSkimmer() {
        if (!m_hasSkimmer) return;
        m_hasSkimmer = false;
        emit hoverSeekEnded();
        update();
    }

    QSize minimumSizeHint() const override { return minimumSize(); }

signals:
    void seekRequested(double time);
    void hoverSeekRequested(double time);
    void hoverSeekEnded();
    void clipSelected(const QString &clipId);
    void moveRequested(const QString &clipId, double timelineStart);
    void trimRequested(const QString &clipId, bool leading, double timelineFrame);

protected:
    void paintEvent(QPaintEvent *event) override {
        const QRect visible = event->rect();
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing);
        QLinearGradient timelineGradient(0, 0, 0, height());
        timelineGradient.setColorAt(0.0, QColor("#242424"));
        timelineGradient.setColorAt(1.0, QColor("#242424"));
        p.fillRect(rect(), timelineGradient);

        const int trackCount = static_cast<int>(std::max<qsizetype>(1, m_project.value(QStringLiteral("tracks")).toArray().size()));
        const int trackBottom = kRulerHeight + trackCount * kTrackHeight;
        const double pxPerSecond = pixelsPerSecond();

        p.fillRect(0, 0, kTimelineLabelWidth, height(), QColor("#202020"));
        p.fillRect(kTimelineLabelWidth, 0, width() - kTimelineLabelWidth, kRulerHeight, QColor("#202020"));
        p.setPen(QColor("#2a2a2a"));
        p.drawLine(kTimelineLabelWidth, 0, kTimelineLabelWidth, height());

        QFont rulerFont = p.font();
        rulerFont.setPixelSize(11);
        p.setFont(rulerFont);
        p.setPen(QColor("#868686"));
        p.drawText(QRect(12, 0, kTimelineLabelWidth - 20, kRulerHeight), Qt::AlignVCenter,
                   QStringLiteral("Tracks"));

        const int majorStep = std::max(1, static_cast<int>(std::ceil(70.0 / pxPerSecond)));
        const int firstSecond = std::max(0, static_cast<int>(std::floor((visible.left() - kTimelineLabelWidth) / pxPerSecond)) - 1);
        const int lastSecond = std::min(static_cast<int>(std::ceil(m_duration)),
                                        static_cast<int>(std::ceil((visible.right() - kTimelineLabelWidth) / pxPerSecond)) + 1);
        const int tickStep = pxPerSecond >= 10.0 ? 1 : majorStep;
        for (int second = firstSecond / tickStep * tickStep; second <= lastSecond; second += tickStep) {
            const int x = kTimelineLabelWidth + static_cast<int>(second * pxPerSecond);
            const bool major = second % majorStep == 0;
            p.setPen(major ? QColor("#434343") : QColor("#2f2f2f"));
            p.drawLine(x, major ? kRulerHeight : kRulerHeight - 5, x, trackBottom);
            p.setPen(major ? QColor("#b0b0b0") : QColor("#747474"));
            if (major) p.drawText(x + 6, 18, prettyTime(second));
        }

        const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
        for (int row = 0; row < trackCount; ++row) {
            const int y = kRulerHeight + row * kTrackHeight;
            if (y + kTrackHeight < visible.top() || y > visible.bottom()) continue;
            p.fillRect(0, y, width(), kTrackHeight,
                       row % 2 == 0 ? QColor("#242424") : QColor("#202020"));
            p.setPen(QColor("#363636"));
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
            p.setPen(QColor("#393939"));
            p.setBrush(QColor("#222222"));
            p.drawRoundedRect(QRectF(8, y + 12, 16, 18), 2, 2);
            p.setPen(accent);
            p.setBrush(Qt::NoBrush);
            p.drawRoundedRect(QRectF(8, y + 12, 16, 18), 2, 2);
            p.setPen(QColor("#cecece"));
            p.drawText(QRectF(8, y + 12, 16, 18), Qt::AlignCenter, laneKind);
            p.setPen(QColor("#cbcbcb"));
            p.drawText(30, y + 25, p.fontMetrics().elidedText(trackTitle, Qt::ElideRight, 60));
            p.setPen(QColor("#808080"));
            QFont trackFont = p.font();
            trackFont.setPixelSize(10);
            p.setFont(trackFont);
            p.drawText(30, y + 40, type.toUpper());
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
                QColor body = accent.darker(selected ? 145 : hovered ? 165 : 190);
                if (body.value() < 24) body = QColor("#292929");
                QLinearGradient clipGradient(0, clipRect.top(), 0, clipRect.bottom());
                clipGradient.setColorAt(0.0, body.lighter(114));
                clipGradient.setColorAt(1.0, body);
                p.setBrush(clipGradient);
                p.setPen(selected ? QColor("#d8d8d8") : hovered ? accent.lighter(120) : QColor("#474747"));
                p.drawRoundedRect(clipRect, 3, 3);
                p.save();
                p.setClipRect(clipRect.adjusted(2, 2, -2, -2));
                p.setPen(QPen(QColor(accent.red(), accent.green(), accent.blue(), 80), 2));
                p.drawLine(clipRect.left() + 5, clipRect.top() + 2, clipRect.right() - 5, clipRect.top() + 2);
                if (type.contains(QStringLiteral("audio")) || type.contains(QStringLiteral("sound"))) {
                    const QString sourceId = clip.value("sourceId").toString();
                    if (m_artwork && m_artwork->waves.contains(sourceId)) {
                        const auto wave = m_artwork->waves.value(sourceId);
                        double sourceDuration = 1;
                        for (const auto &source : m_project.value("sources").toArray())
                            if (source.toObject().value("id").toString() == sourceId)
                                sourceDuration = std::max(0.01, source.toObject().value("duration").toDouble(1));
                        const double in = clip.value("in").toDouble();
                        const double out = clip.value("out").toDouble(sourceDuration);
                        p.setOpacity(0.55);
                        p.drawImage(clipRect.adjusted(0, 18, 0, 0), wave,
                                    QRectF(in / sourceDuration * wave.width(), 0,
                                           (out - in) / sourceDuration * wave.width(), wave.height()));
                        p.setOpacity(1);
                    }
                } else {
                    const QImage frame = m_artwork ? m_artwork->frames.value(clip.value("sourceId").toString()) : QImage();
                    if (!frame.isNull()) {
                        const double tileWidth = (clipRect.height() - 16) * 16.0 / 9.0;
                        const double firstTile = clipRect.left() + std::max(0.0, std::floor((visible.left() - clipRect.left()) / tileWidth)) * tileWidth;
                        for (double x = firstTile; x < std::min(clipRect.right(), double(visible.right())); x += tileWidth)
                            p.drawImage(QRectF(x, clipRect.top() + 16, tileWidth, clipRect.height() - 16), frame);
                    }
                }
                p.restore();
                if (clipRect.width() > 24.0) {
                    const QFontMetrics metrics(p.font());
                    const QString label = metrics.elidedText(clipName(clip), Qt::ElideRight,
                                                              std::max(1, static_cast<int>(clipRect.width()) - 16));
                    p.setPen(QColor("#eeeeee"));
                    p.drawText(QRectF(clipRect.left() + 5, clipRect.top(), clipRect.width() - 10, 16), Qt::AlignVCenter | Qt::TextSingleLine, label);
                }
                p.setPen(QPen(accent.lighter(selected ? 135 : 120), selected ? 2 : 1));
                p.drawLine(clipRect.left() + 5, clipRect.top() + 1, clipRect.right() - 5, clipRect.top() + 1);
                if (selected) {
                    p.setPen(QPen(QColor("#e0e0e0"), 2));
                    p.drawLine(clipRect.left(), clipRect.top() + 3, clipRect.left(), clipRect.bottom() - 3);
                    p.drawLine(clipRect.right(), clipRect.top() + 3, clipRect.right(), clipRect.bottom() - 3);
                }
            }
        }

        const int playheadX = kTimelineLabelWidth + static_cast<int>(m_playhead * pxPerSecond);
        p.setPen(QPen(QColor("#c7b5ed"), 1));
        p.drawLine(playheadX, 0, playheadX, height());
        p.setBrush(QColor("#c7b5ed"));
        p.setPen(Qt::NoPen);
        p.drawPolygon(QPolygonF{QPointF(playheadX - 6, 0), QPointF(playheadX + 6, 0), QPointF(playheadX, 8)});
        if (m_hasSkimmer && !m_dragging && !m_scrubbing) {
            const int skimmerX = kTimelineLabelWidth + static_cast<int>(m_skimmerTime * pxPerSecond);
            p.setPen(QPen(QColor("#ececec"), 1));
            p.drawLine(skimmerX, kRulerHeight, skimmerX, height());
            p.setBrush(QColor("#ececec"));
            p.setPen(Qt::NoPen);
            p.drawPolygon(QPolygonF{QPointF(skimmerX - 4, kRulerHeight - 1),
                                   QPointF(skimmerX + 4, kRulerHeight - 1),
                                   QPointF(skimmerX, kRulerHeight + 5)});
        }
        if (hasFocus()) {
            p.setPen(QPen(QColor("#777777"), 1));
            p.setBrush(Qt::NoBrush);
            p.drawRect(rect().adjusted(1, 1, -2, -2));
        }
    }

    void mousePressEvent(QMouseEvent *event) override {
        if (event->button() != Qt::LeftButton) return;
        setFocus();
        clearSkimmer();
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
            updateSkimmer(event->position().toPoint());
            return;
        }
        clearSkimmer();
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
            if (m_hasSkimmer) {
                m_hasSkimmer = false;
                emit hoverSeekEnded();
            }
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

    void updateContentSize() {
        const int tracks = static_cast<int>(m_project.value(QStringLiteral("tracks")).toArray().size());
        const double contentWidth = kTimelineLabelWidth + m_duration * pixelsPerSecond() + 30;
        setMinimumSize(static_cast<int>(std::min(contentWidth, 16777215.0)),
                       kRulerHeight + std::max(3, tracks) * kTrackHeight + 8);
        updateGeometry();
    }

    double pixelsPerSecond() const {
        return m_fitWidth > 0 ? (m_fitWidth - kTimelineLabelWidth - 30.0) / m_duration : 72.0 * m_zoom;
    }

    double timeAt(int x) const {
        return std::clamp((x - kTimelineLabelWidth) / pixelsPerSecond(), 0.0, m_duration);
    }

    bool hasVideoAt(double time) const {
        const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
        for (const QJsonValue &trackValue : tracks) {
            const QJsonObject track = trackValue.toObject();
            const QString type = track.value(QStringLiteral("type")).toString().toLower();
            if (type.contains(QStringLiteral("audio")) || type.contains(QStringLiteral("sound")) ||
                type.contains(QStringLiteral("title")) || type.contains(QStringLiteral("graphic")) ||
                type.contains(QStringLiteral("overlay"))) {
                continue;
            }
            for (const QJsonValue &clipValue : track.value(QStringLiteral("clips")).toArray()) {
                const QJsonObject clip = clipValue.toObject();
                const double start = clip.value(QStringLiteral("timelineStart")).toDouble();
                double duration = clip.value(QStringLiteral("duration")).toDouble();
                if (duration <= 0.0) {
                    const double speed = std::max(0.01, clip.value(QStringLiteral("speed")).toDouble(1.0));
                    duration = (clip.value(QStringLiteral("out")).toDouble() - clip.value(QStringLiteral("in")).toDouble()) / speed;
                }
                if (time >= start && time < start + duration) return true;
            }
        }
        return false;
    }

    void updateSkimmer(const QPoint &point) {
        if (point.x() < kTimelineLabelWidth || point.y() < kRulerHeight) {
            if (m_hasSkimmer) {
                m_hasSkimmer = false;
                emit hoverSeekEnded();
                update();
            }
            return;
        }
        const double time = timeAt(point.x());
        if (!hasVideoAt(time)) {
            if (m_hasSkimmer) {
                m_hasSkimmer = false;
                emit hoverSeekEnded();
                update();
            }
            return;
        }
        const double frameTime = std::round(time * m_fps) / m_fps;
        if (!m_hasSkimmer || std::abs(m_skimmerTime - frameTime) >= (0.5 / m_fps)) {
            m_skimmerTime = std::clamp(frameTime, 0.0, m_duration);
            m_hasSkimmer = true;
            emit hoverSeekRequested(m_skimmerTime);
            update();
        }
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

    MediaArtwork *m_artwork = nullptr;
    QJsonObject m_project;
    QString m_selectedClip;
    QString m_hoverClipId;
    QString m_dragClipId;
    double m_duration = 1.0;
    double m_fps = 24.0;
    double m_playhead = 0.0;
    double m_skimmerTime = 0.0;
    double m_zoom = 1.0;
    int m_fitWidth = 0;
    double m_dragStart = 0.0;
    double m_dragDuration = 0.0;
    double m_pressTime = 0.0;
    double m_previewTime = 0.0;
    bool m_scrubbing = false;
    bool m_dragging = false;
    bool m_hasSkimmer = false;
    bool m_dragLeading = false;
    bool m_dragTrailing = false;
};

class ViewerWidget final : public QWidget {
    Q_OBJECT
public:
    explicit ViewerWidget(QWidget *parent = nullptr) : QWidget(parent) {
        setMinimumSize(320, 180);
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

    QRectF videoRect() const {
        const QRectF target = QRectF(rect()).adjusted(12, 48, -12, -12);
        double aspect = 16.0 / 9.0;
        if (m_width > 0 && m_height > 0) aspect = m_width / m_height;
        else if (!m_frame.isNull()) aspect = static_cast<double>(m_frame.width()) / m_frame.height();
        QSizeF canvasSize(target.width(), target.width() / aspect);
        if (canvasSize.height() > target.height())
            canvasSize = QSizeF(target.height() * aspect, target.height());
        return QRectF(target.center() - QPointF(canvasSize.width() / 2, canvasSize.height() / 2), canvasSize);
    }

    QRectF captionRect() const {
        const QRectF canvas = videoRect();
        const double horizontalMargin = std::max(12.0, canvas.width() * 0.08);
        const double bottomMargin = std::max(12.0, canvas.height() * 0.08);
        const double captionHeight = std::clamp(canvas.height() * 0.12, 34.0, 64.0);
        return QRectF(canvas.left() + horizontalMargin,
                      canvas.bottom() - bottomMargin - captionHeight,
                      std::max(1.0, canvas.width() - horizontalMargin * 2.0), captionHeight);
    }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing);
        QPainterPath surface;
        surface.addRect(QRectF(rect()));
        p.setClipPath(surface);
        QLinearGradient viewerGradient(0, 0, 0, height());
        viewerGradient.setColorAt(0.0, QColor("#242424"));
        viewerGradient.setColorAt(1.0, QColor("#242424"));
        p.fillRect(rect(), viewerGradient);
        p.setPen(QColor("#262626"));
        p.fillRect(0, 0, width(), 36, QColor("#2a2a2a"));
        p.drawLine(12, 36, width() - 12, 36);
        const QRectF canvas = videoRect();
        p.setPen(QColor("#202020"));
        p.setBrush(QColor("#070707"));
        p.drawRect(canvas.adjusted(0.5, 0.5, -0.5, -0.5));
        if (!m_frame.isNull()) {
            QImage scaled = m_frame.scaled(canvas.size().toSize(), Qt::KeepAspectRatio, Qt::SmoothTransformation);
            const QPointF topLeft(canvas.center().x() - scaled.width() / 2.0, canvas.center().y() - scaled.height() / 2.0);
            p.drawImage(topLeft, scaled);
        } else {
            const QPointF center = canvas.center() - QPointF(0, 22);
            p.setPen(QColor("#363636"));
            p.setBrush(QColor("#181818"));
            p.drawEllipse(center, 20, 20);
            p.setPen(QColor("#626262"));
            p.setBrush(Qt::NoBrush);
            p.drawEllipse(center, 20, 20);
            p.setPen(Qt::NoPen);
            p.setBrush(QColor("#b1b1b1"));
            p.drawPolygon(QPolygonF{center + QPointF(-5, -8), center + QPointF(9, 0), center + QPointF(-5, 8)});
            p.setPen(QColor("#a0a0a0"));
            p.drawText(QRectF(canvas.left(), center.y() + 32, canvas.width(), 20), Qt::AlignCenter,
                       QStringLiteral("Your viewer"));
            p.setPen(QColor("#747474"));
            p.drawText(QRectF(canvas.left(), center.y() + 52, canvas.width(), 20), Qt::AlignCenter,
                       QStringLiteral("Import media or render a preview"));
        }
        p.setPen(QColor("#a0a0a0"));
        QFont viewerFont = p.font();
        viewerFont.setPixelSize(12);
        viewerFont.setWeight(QFont::Medium);
        p.setFont(viewerFont);
        p.drawText(14, 23, QStringLiteral("Viewer"));
        p.setPen(QColor("#747474"));
        p.drawText(76, 23, QStringLiteral("%1 × %2").arg(m_width > 0 ? QString::number(static_cast<int>(m_width)) : QStringLiteral("—"),
                                                         m_height > 0 ? QString::number(static_cast<int>(m_height)) : QStringLiteral("—")));
        p.setPen(QColor("#a6a6a6"));
        QFont timeFont = QFontDatabase::systemFont(QFontDatabase::FixedFont);
        timeFont.setPixelSize(11);
        p.setFont(timeFont);
        p.drawText(width() - 84, 23, prettyTime(m_time));
        p.setFont(viewerFont);
        if (!m_caption.isEmpty()) {
            const QRectF caption = captionRect();
            p.setPen(Qt::NoPen);
            p.setBrush(QColor(15, 15, 15, 225));
            p.drawRoundedRect(caption, 4, 4);
            p.setPen(QColor("#676767"));
            p.setBrush(Qt::NoBrush);
            p.drawRoundedRect(caption, 4, 4);
            p.setPen(QColor("#eeeeee"));
            p.drawText(caption.adjusted(10, 4, -10, -4), Qt::AlignCenter | Qt::TextWordWrap, m_caption);
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
    friend class EditorTest;
public:
    MainWindow(const QString &projectDirectory, const QString &cutdBinary, QWidget *parent = nullptr)
        : QMainWindow(parent), m_projectDirectory(projectDirectory), m_cutdBinary(cutdBinary) {
        setWindowTitle(QStringLiteral("Cut — Editor"));
        resize(1500, 940);
        setMinimumSize(1080, 700);
        buildUi();
        setupActions();
        // Runtime messages remain available to callers, but the empty status
        // bar should not reserve a strip along the bottom of the editor.
        statusBar()->hide();
        setConnection(QStringLiteral("connecting"));
#ifndef CUT_QT_TESTING
        m_bridge = cut_bridge_new(m_projectDirectory.toUtf8().constData(), m_cutdBinary.toUtf8().constData());
        if (!m_bridge) {
            setConnection(QStringLiteral("disconnected"), QStringLiteral("Could not start the runtime connection."));
        }
        m_pollTimer.setInterval(16);
        connect(&m_pollTimer, &QTimer::timeout, this, &MainWindow::pollBridge);
        m_pollTimer.start();
#endif
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
        if (cut_bridge_frame(m_bridge, &frame) && m_connected && frame.pixels && frame.length > 0 && frame.width > 0 && frame.height > 0) {
            const qsizetype expected = static_cast<qsizetype>(frame.width) * static_cast<qsizetype>(frame.height) * 4;
            if (expected > 0 && frame.length >= static_cast<size_t>(expected)) {
                // A seek can leave one already decoded frame in the bridge's
                // replaceable slot. Do not let it flash while the latest
                // hover target is still being decoded.
                if (m_hoverSkimming &&
                    std::abs(frame.time - m_pendingHoverSeek) > (1.5 / std::max(1.0, m_fps))) {
                    return;
                }
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
        if (!sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("import")},
                               {QStringLiteral("path"), path},
                               {QStringLiteral("revision"), QJsonValue::fromVariant(QVariant::fromValue(m_revision))}})) return;
        addActivity(QStringLiteral("import"), QStringLiteral("Importing %1").arg(QFileInfo(path).fileName()));
    }

    void exportProject() {
        const QString quality = m_quality->currentData().toString();
        if (!sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("export")}, {QStringLiteral("quality"), quality}})) return;
        addActivity(QStringLiteral("export"), QStringLiteral("Export started (%1)").arg(m_quality->currentText()));
    }

    void renderPreview() {
        if (!sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("preview")}})) return;
        addActivity(QStringLiteral("preview"), QStringLiteral("Rendering preview"));
        m_previewButton->setEnabled(false);
    }

    void canvasPresetChanged(int index) {
        if (m_updatingCanvasPreset || !m_connected || index < 0) return;
        const QStringList dimensions = m_canvasPreset->itemData(index).toString().split(QLatin1Char('x'));
        if (dimensions.size() != 2) {
            syncCanvasPreset(m_canvasSize);
            return;
        }
        bool widthOk = false;
        bool heightOk = false;
        const int width = dimensions.at(0).toInt(&widthOk);
        const int height = dimensions.at(1).toInt(&heightOk);
        if (!widthOk || !heightOk || width < 2 || height < 2) {
            syncCanvasPreset(m_canvasSize);
            return;
        }
        const QJsonObject command{{QStringLiteral("type"), QStringLiteral("set_canvas")},
                                  {QStringLiteral("width"), width},
                                  {QStringLiteral("height"), height}};
        if (!sendSimple(commandApply(command, m_revision))) syncCanvasPreset(m_canvasSize);
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
        if (!m_connected || m_previewRequired || m_duration <= 0) return;
        if (!m_playing && m_hoverSkimming) {
            // Skimming is a viewer-only preview. Restore the edit playhead
            // before starting playback so hover never changes its start point.
            m_hasPendingHoverSeek = false;
            m_hoverSkimming = false;
            m_timeline->clearSkimmer();
            sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("seek")},
                                   {QStringLiteral("position"), m_playhead}});
        }
        sendSimple(QJsonObject{{QStringLiteral("type"), m_playing ? QStringLiteral("pause") : QStringLiteral("play")} });
    }

    void queueSeek(double time) {
        if (!m_connected) return;
        m_hasPendingHoverSeek = false;
        m_hoverSkimming = false;
        m_timeline->clearSkimmer();
        m_pendingSeek = std::clamp(time, 0.0, m_duration);
        m_hasPendingSeek = true;
        m_playhead = m_pendingSeek;
        m_timeline->setPlayhead(m_playhead);
        updateTransportTime();
    }

    void queueHoverSeek(double time) {
        if (!m_connected || m_previewRequired || m_playing || m_duration <= 0.0) return;
        m_pendingHoverSeek = std::clamp(time, 0.0, m_duration);
        m_hasPendingHoverSeek = true;
        m_hoverSkimming = true;
    }

    void endHoverSeek() {
        m_hasPendingHoverSeek = false;
        m_hoverSkimming = false;
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
        const QString text = m_chatInput->toPlainText().trimmed();
        if (text.isEmpty()) return;
        const QJsonObject settings = agentSettingsPayload();
        if (!sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("agent")},
                                    {QStringLiteral("text"), text},
                                    {QStringLiteral("provider"), settings.value(QStringLiteral("provider"))},
                                    {QStringLiteral("apiMode"), settings.value(QStringLiteral("apiMode"))},
                                    {QStringLiteral("apiKey"), settings.value(QStringLiteral("apiKey"))},
                                    {QStringLiteral("baseUrl"), settings.value(QStringLiteral("baseUrl"))},
                                    {QStringLiteral("model"), settings.value(QStringLiteral("model"))}})) return;
        m_chatInput->clear();
        m_agentBusy = true;
        m_agentText.clear();
        m_agentStatus->setText(QStringLiteral("WORKING"));
        m_chatInput->setEnabled(false);
        m_abortButton->setEnabled(true);
        updateChatControls();
        appendConversation(QStringLiteral("You"), text);
        appendConversationNote(QStringLiteral("Progress"), QStringLiteral("Request sent"));
        addActivity(QStringLiteral("agent"), QStringLiteral("Request sent"));
    }

    void updateChatControls() {
        const bool hasText = !m_chatInput->toPlainText().trimmed().isEmpty();
        m_sendButton->setEnabled(m_connected && hasText && !m_agentBusy);
        m_abortButton->setEnabled(m_agentBusy);
        m_abortButton->setVisible(m_agentBusy);
        m_sendButton->setVisible(!m_agentBusy);
    }

    QJsonObject agentSettingsPayload() const {
        const bool openai = m_providerSelect && m_providerSelect->currentData().toString() == QStringLiteral("openai");
        const QLineEdit *key = openai ? m_openaiKey : m_anthropicKey;
        const QLineEdit *baseUrl = openai ? m_openaiBaseUrl : m_anthropicBaseUrl;
        return QJsonObject{{QStringLiteral("provider"), m_providerSelect->currentData().toString()},
                           {QStringLiteral("apiMode"), m_openaiMode->currentData().toString()},
                           {QStringLiteral("apiKey"), key ? key->text().trimmed() : QString()},
                           {QStringLiteral("baseUrl"), baseUrl ? baseUrl->text().trimmed() : QString()},
                           {QStringLiteral("model"), m_modelSelect->currentText().trimmed()}};
    }

    void updateAgentModelChoices() {
        if (!m_providerSelect || !m_modelSelect) return;
        const bool openai = m_providerSelect->currentData().toString() == QStringLiteral("openai");
        const QString current = m_modelSelect->currentText();
        const QString configured = openai ? m_openaiModel->text().trimmed() : m_anthropicModel->text().trimmed();
        const QStringList models = openai
                                        ? QStringList{QStringLiteral("gpt-5.6-luna"), QStringLiteral("gpt-4o"), QStringLiteral("gpt-4o-mini")}
                                        : QStringList{QStringLiteral("claude-sonnet-4-20250514"), QStringLiteral("claude-3-5-sonnet-latest")};
        const QSignalBlocker blocker(m_modelSelect);
        m_modelSelect->clear();
        m_modelSelect->addItems(models);
        const QString model = configured.isEmpty() ? current : configured;
        m_modelSelect->setEditText(model.isEmpty() ? models.constFirst() : model);
        if (m_agentConfigLabel) {
            m_agentConfigLabel->setText(QStringLiteral("%1  ·  %2")
                                            .arg(openai ? QStringLiteral("OpenAI") : QStringLiteral("Anthropic"),
                                                 m_modelSelect->currentText()));
        }
    }

    void loadAgentSettings() {
        const QSettings settings;
        const QString envAnthropicKey = qEnvironmentVariable("ANTHROPIC_API_KEY");
        const QString envOpenaiKey = qEnvironmentVariable("OPENAI_API_KEY");
        m_anthropicKey->setText(settings.value(QStringLiteral("agent/anthropic/apiKey"), envAnthropicKey).toString());
        m_anthropicBaseUrl->setText(settings.value(QStringLiteral("agent/anthropic/baseUrl"), QStringLiteral("https://api.anthropic.com/v1")).toString());
        m_anthropicModel->setText(settings.value(QStringLiteral("agent/anthropic/model"), QStringLiteral("claude-sonnet-4-20250514")).toString());
        m_openaiKey->setText(settings.value(QStringLiteral("agent/openai/apiKey"), envOpenaiKey).toString());
        m_openaiBaseUrl->setText(settings.value(QStringLiteral("agent/openai/baseUrl"), QStringLiteral("https://api.openai.com/v1")).toString());
        m_openaiModel->setText(settings.value(QStringLiteral("agent/openai/model"), QStringLiteral("gpt-5.6-luna")).toString());
        const QString provider = settings.value(QStringLiteral("agent/activeProvider"),
                                                 qEnvironmentVariable("CUT_PROVIDER", envOpenaiKey.isEmpty() ? QStringLiteral("anthropic") : QStringLiteral("openai")))
                                     .toString()
                                     .toLower();
        const int providerIndex = m_providerSelect->findData(provider == QStringLiteral("openai") ? QStringLiteral("openai") : QStringLiteral("anthropic"));
        m_providerSelect->setCurrentIndex(std::max(0, providerIndex));
        const QString mode = settings.value(QStringLiteral("agent/openai/apiMode"), qEnvironmentVariable("OPENAI_API_MODE", QStringLiteral("chat"))).toString().toLower();
        const int modeIndex = m_openaiMode->findData(mode == QStringLiteral("responses") ? QStringLiteral("responses") : QStringLiteral("chat"));
        m_openaiMode->setCurrentIndex(std::max(0, modeIndex));
        updateAgentModelChoices();
    }

    void saveAgentSettings() {
        if (!m_anthropicKey || !m_openaiKey) return;
        QSettings settings;
        settings.setValue(QStringLiteral("agent/anthropic/apiKey"), m_anthropicKey->text());
        settings.setValue(QStringLiteral("agent/anthropic/baseUrl"), m_anthropicBaseUrl->text().trimmed());
        settings.setValue(QStringLiteral("agent/anthropic/model"), m_anthropicModel->text().trimmed());
        settings.setValue(QStringLiteral("agent/openai/apiKey"), m_openaiKey->text());
        settings.setValue(QStringLiteral("agent/openai/baseUrl"), m_openaiBaseUrl->text().trimmed());
        settings.setValue(QStringLiteral("agent/openai/model"), m_openaiModel->text().trimmed());
        settings.setValue(QStringLiteral("agent/openai/apiMode"), m_openaiMode->currentData().toString());
        settings.setValue(QStringLiteral("agent/activeProvider"), m_providerSelect->currentData().toString());
        settings.sync();
        updateAgentModelChoices();
        statusBar()->showMessage(QStringLiteral("Agent settings saved"), 3000);
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
    bool eventFilter(QObject *object, QEvent *event) override {
        if (object == m_chatInput && event->type() == QEvent::KeyPress) {
            auto *key = static_cast<QKeyEvent *>(event);
            if ((key->key() == Qt::Key_Return || key->key() == Qt::Key_Enter) &&
                !(key->modifiers() & Qt::ShiftModifier)) {
                if (m_sendButton->isEnabled()) sendChat();
                return true;
            }
        }
        if (m_timelineScroll && object == m_timelineScroll->viewport() && event->type() == QEvent::Resize && m_timeline->isFitToWidth()) {
            m_timeline->fitToWidth(m_timelineScroll->viewport()->width());
        }
        return QMainWindow::eventFilter(object, event);
    }

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
        central->setObjectName(QStringLiteral("workspaceRoot"));
        auto *root = new QVBoxLayout(central);
        // Narrow gutters give the editing surfaces a little breathing room.
        root->setContentsMargins(2, 2, 2, 2);
        root->setSpacing(2);
        m_artwork = new MediaArtwork(this);

        m_connectionBanner = new QFrame;
        m_connectionBanner->setObjectName(QStringLiteral("connectionBanner"));
        auto *connectionLayout = new QHBoxLayout(m_connectionBanner);
        m_connectionMessage = new QLabel;
        m_connectionMessage->setWordWrap(true);
        connectionLayout->addWidget(m_connectionMessage, 1);
        m_reconnectButton = new QPushButton(QStringLiteral("Reconnect"));
        connect(m_reconnectButton, &QPushButton::clicked, this, [this] {
            setConnection(QStringLiteral("connecting"));
            if (!m_bridge) {
                m_bridge = cut_bridge_new(m_projectDirectory.toUtf8().constData(), m_cutdBinary.toUtf8().constData());
                if (!m_bridge) setConnection(QStringLiteral("disconnected"), QStringLiteral("Could not start the runtime connection."));
            } else if (!sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("reconnect")}})) {
                setConnection(QStringLiteral("disconnected"), QStringLiteral("Reconnect could not be queued. Try again."));
            }
        });
        connectionLayout->addWidget(m_reconnectButton);
        root->addWidget(m_connectionBanner);

        m_errorBanner = new QFrame;
        m_errorBanner->setObjectName(QStringLiteral("errorBanner"));
        auto *errorLayout = new QHBoxLayout(m_errorBanner);
        m_errorMessage = new QLabel;
        m_errorMessage->setWordWrap(true);
        errorLayout->addWidget(m_errorMessage, 1);
        auto *dismissError = new QPushButton(QStringLiteral("Dismiss"));
        connect(dismissError, &QPushButton::clicked, m_errorBanner, &QWidget::hide);
        errorLayout->addWidget(dismissError);
        m_errorBanner->hide();
        root->addWidget(m_errorBanner);

        auto *workspace = new QSplitter(Qt::Horizontal);
        workspace->setObjectName(QStringLiteral("workspaceSplitter"));
        workspace->setHandleWidth(2);
        workspace->setChildrenCollapsible(false);
        root->addWidget(workspace, 1);

        auto *mediaShelf = new QFrame;
        m_mediaPanel = mediaShelf;
        mediaShelf->setObjectName(QStringLiteral("mediaShelf"));
        mediaShelf->setSizePolicy(QSizePolicy::Preferred, QSizePolicy::Expanding);
        mediaShelf->setMinimumWidth(180);
        auto *mediaShelfLayout = new QVBoxLayout(mediaShelf);
        mediaShelfLayout->setContentsMargins(12, 10, 12, 12);
        mediaShelfLayout->setSpacing(8);
        auto *mediaHeader = new QHBoxLayout;
        auto *mediaTitle = new QLabel(QStringLiteral("Media"));
        mediaTitle->setObjectName(QStringLiteral("eyebrow"));
        auto *importButton = new QToolButton;
        importButton->setText(QStringLiteral("+") );
        importButton->setToolTip(QStringLiteral("Import media"));
        importButton->setObjectName(QStringLiteral("squareButton"));
        connect(importButton, &QToolButton::clicked, this, &MainWindow::importMedia);
        mediaHeader->addWidget(mediaTitle);
        mediaHeader->addStretch();
        mediaHeader->addWidget(importButton);
        auto *mediaToggle = new QToolButton;
        m_mediaToggle = mediaToggle;
        mediaToggle->setText(QStringLiteral("▾"));
        mediaToggle->setToolTip(QStringLiteral("Expand media bin"));
        mediaToggle->setObjectName(QStringLiteral("mediaToggle"));
        mediaToggle->hide();
        mediaShelfLayout->addLayout(mediaHeader);
        m_mediaList = new QListWidget;
        m_mediaList->setObjectName(QStringLiteral("mediaList"));
        m_mediaList->setSelectionMode(QAbstractItemView::SingleSelection);
        m_mediaList->setFlow(QListView::LeftToRight);
        m_mediaList->setWrapping(true);
        m_mediaList->setViewMode(QListView::IconMode);
        m_mediaList->setResizeMode(QListView::Adjust);
        m_mediaList->setIconSize(QSize(112, 63));
        m_mediaList->setGridSize(QSize(128, 88));
        m_mediaList->setMovement(QListView::Static);
        m_mediaList->setMinimumHeight(90);
        m_mediaList->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
        connect(m_artwork, &MediaArtwork::changed, this, [this] {
            for (int i = 0; i < m_mediaList->count(); ++i) {
                auto *item = m_mediaList->item(i);
                const QString id = item->data(Qt::UserRole).toString();
                item->setIcon(m_artwork->icon(id));
            }
        });
        auto *mediaList = m_mediaList;
        mediaShelfLayout->addWidget(m_mediaList, 1);
        auto *mediaHint = new QLabel(QStringLiteral("Import media to add clips to the timeline."));
        m_mediaHint = mediaHint;
        mediaHint->setObjectName(QStringLiteral("muted"));
        mediaHint->setWordWrap(true);
        mediaShelfLayout->addWidget(mediaHint);
        auto *trackButton = new QPushButton(QStringLiteral("+ Track"));
        trackButton->setObjectName(QStringLiteral("secondaryButton"));
        connect(trackButton, &QPushButton::clicked, this, &MainWindow::addTrack);
        mediaToggle->setText(QStringLiteral("▸"));
        connect(mediaToggle, &QToolButton::clicked, mediaShelf, [mediaShelf, mediaList, mediaHint, mediaToggle] {
            const bool collapsed = mediaList->isVisible();
            mediaList->setVisible(!collapsed);
            mediaHint->setVisible(!collapsed && mediaList->count() == 0);
            mediaToggle->setText(collapsed ? QStringLiteral("▸") : QStringLiteral("▾"));
            mediaToggle->setToolTip(collapsed ? QStringLiteral("Expand media bin") : QStringLiteral("Collapse media bin"));
            mediaShelf->setProperty("collapsed", collapsed);
            mediaShelf->style()->unpolish(mediaShelf);
            mediaShelf->style()->polish(mediaShelf);
        });

        auto *center = new QWidget;
        auto *centerLayout = new QVBoxLayout(center);
        centerLayout->setContentsMargins(0, 0, 0, 0);
        centerLayout->setSpacing(0);
        auto *editorSplit = new QSplitter(Qt::Vertical);
        editorSplit->setChildrenCollapsible(false);
        editorSplit->setHandleWidth(2);
        centerLayout->addWidget(editorSplit);
        auto *viewerFrame = new QFrame;
        viewerFrame->setObjectName(QStringLiteral("viewerFrame"));
        auto *viewerLayout = new QVBoxLayout(viewerFrame);
        viewerLayout->setContentsMargins(0, 0, 0, 0);
        m_viewer = new ViewerWidget;
        viewerLayout->addWidget(m_viewer);
        auto *browserViewerSplit = new QSplitter(Qt::Horizontal);
        browserViewerSplit->setObjectName(QStringLiteral("browserViewerSplitter"));
        browserViewerSplit->setChildrenCollapsible(false);
        browserViewerSplit->setHandleWidth(2);
        browserViewerSplit->addWidget(mediaShelf);
        browserViewerSplit->addWidget(viewerFrame);
        browserViewerSplit->setStretchFactor(0, 0);
        browserViewerSplit->setStretchFactor(1, 1);
        browserViewerSplit->setSizes({250, 870});
        editorSplit->addWidget(browserViewerSplit);

        auto *timelinePanel = new QWidget;
        timelinePanel->setObjectName(QStringLiteral("timelinePanel"));
        timelinePanel->setAttribute(Qt::WA_StyledBackground);
        auto *timelinePanelLayout = new QVBoxLayout(timelinePanel);
        timelinePanelLayout->setContentsMargins(2, 2, 2, 2);
        timelinePanelLayout->setSpacing(2);

        auto *transport = new QWidget;
        transport->setObjectName(QStringLiteral("transport"));
        transport->setAttribute(Qt::WA_StyledBackground);
        auto *transportLayout = new QHBoxLayout(transport);
        transportLayout->setContentsMargins(8, 4, 8, 4);
        transportLayout->setSpacing(6);
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
        transportLayout->addWidget(trackButton);
        auto *fitButton = new QToolButton;
        fitButton->setText(QStringLiteral("Fit"));
        fitButton->setToolTip(QStringLiteral("Fit the entire timeline"));
        connect(fitButton, &QToolButton::clicked, this, [this] {
            m_timeline->fitToWidth(m_timelineScroll->viewport()->width());
            m_timelineScroll->horizontalScrollBar()->setValue(0);
        });
        transportLayout->addWidget(fitButton);
        m_zoom = new QSlider(Qt::Horizontal);
        m_zoom->setRange(35, 350);
        m_zoom->setValue(100);
        m_zoom->setMaximumWidth(130);
        m_zoom->setMinimumWidth(60);
        m_zoom->setToolTip(QStringLiteral("Timeline zoom"));
        connect(m_zoom, &QSlider::valueChanged, this, [this](int value) { m_timeline->setZoom(value / 100.0); });
        transportLayout->addWidget(m_zoom);
        timelinePanelLayout->addWidget(transport);

        auto *timelineFrame = new QFrame;
        timelineFrame->setObjectName(QStringLiteral("timelineFrame"));
        auto *timelineLayout = new QVBoxLayout(timelineFrame);
        timelineLayout->setContentsMargins(0, 0, 0, 0);
        m_timeline = new TimelineWidget;
        m_timeline->setArtwork(m_artwork);
        auto *timelineScroll = new QScrollArea;
        m_timelineScroll = timelineScroll;
        timelineScroll->setWidgetResizable(true);
        timelineScroll->setFrameShape(QFrame::NoFrame);
        timelineScroll->setHorizontalScrollBarPolicy(Qt::ScrollBarAsNeeded);
        timelineScroll->setWidget(m_timeline);
        timelineScroll->viewport()->installEventFilter(this);
        timelineLayout->addWidget(timelineScroll);
        timelinePanelLayout->addWidget(timelineFrame, 1);
        editorSplit->addWidget(timelinePanel);
        editorSplit->setSizes({550, 310});
        editorSplit->setStretchFactor(0, 1);
        editorSplit->setStretchFactor(1, 1);

        auto *right = new QWidget;
        m_agentPanel = right;
        right->setObjectName(QStringLiteral("panel"));
        right->setAttribute(Qt::WA_StyledBackground);
        right->setMinimumWidth(320);
        auto *rightLayout = new QVBoxLayout(right);
        rightLayout->setContentsMargins(16, 14, 16, 14);
        rightLayout->setSpacing(12);
        auto *agentHeader = new QHBoxLayout;
        auto *agentTitle = new QLabel(QStringLiteral("Cut Assistant"));
        agentTitle->setObjectName(QStringLiteral("assistantTitle"));
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
        chatLayout->setSpacing(10);
        auto *agentContext = new QFrame(chatTab);
        agentContext->setObjectName(QStringLiteral("agentContext"));
        auto *contextLayout = new QVBoxLayout(agentContext);
        contextLayout->setContentsMargins(10, 8, 10, 8);
        contextLayout->setSpacing(5);
        auto *contextHeader = new QHBoxLayout;
        auto *contextTitle = new QLabel(QStringLiteral("CURRENT TASK"));
        contextTitle->setObjectName(QStringLiteral("eyebrow"));
        m_agentConfigLabel = new QLabel;
        m_agentConfigLabel->setObjectName(QStringLiteral("agentConfig"));
        m_agentConfigLabel->setWordWrap(true);
        contextHeader->addWidget(contextTitle);
        contextHeader->addStretch();
        contextHeader->addWidget(m_agentConfigLabel);
        contextLayout->addLayout(contextHeader);
        auto *contextHint = new QLabel(QStringLiteral("Project timeline and media are attached automatically."));
        contextHint->setWordWrap(true);
        contextHint->setObjectName(QStringLiteral("muted"));
        contextLayout->addWidget(contextHint);
        agentContext->hide();
        auto *chatHint = new QLabel(QStringLiteral("Make your next cut."));
        chatHint->setObjectName(QStringLiteral("chatHint"));
        chatHint->setWordWrap(true);
        chatLayout->addWidget(chatHint);
        m_conversation = new QTextEdit;
        m_conversation->setReadOnly(true);
        m_conversation->setObjectName(QStringLiteral("conversation"));
        m_conversation->setAcceptRichText(false);
        m_conversation->setUndoRedoEnabled(false);
        m_conversation->setTextInteractionFlags(Qt::TextSelectableByMouse | Qt::TextSelectableByKeyboard);
        m_conversation->document()->setDocumentMargin(10);
        m_conversation->setPlaceholderText(QStringLiteral("Describe the edit you have in mind.\n\nYour timeline and media are included.\nYou review changes before they apply."));
        chatLayout->addWidget(m_conversation, 1);
        auto *composer = new QFrame;
        composer->setObjectName(QStringLiteral("chatComposer"));
        auto *composerLayout = new QVBoxLayout(composer);
        composerLayout->setContentsMargins(10, 10, 10, 10);
        composerLayout->setSpacing(10);
        auto *agentPicker = new QHBoxLayout;
        agentPicker->setSpacing(5);
        auto *providerLabel = new QLabel(QStringLiteral("Provider"));
        providerLabel->setObjectName(QStringLiteral("muted"));
        m_providerSelect = new QComboBox;
        m_providerSelect->addItem(QStringLiteral("Anthropic"), QStringLiteral("anthropic"));
        m_providerSelect->addItem(QStringLiteral("OpenAI"), QStringLiteral("openai"));
        m_providerSelect->setObjectName(QStringLiteral("agentProvider"));
        m_providerSelect->setToolTip(QStringLiteral("Choose which configured provider handles this request"));
        agentPicker->addWidget(providerLabel);
        providerLabel->hide();
        agentPicker->addWidget(m_providerSelect, 1);
        auto *modelLabel = new QLabel(QStringLiteral("Model"));
        modelLabel->setObjectName(QStringLiteral("muted"));
        m_modelSelect = new QComboBox;
        m_modelSelect->setEditable(true);
        m_modelSelect->setInsertPolicy(QComboBox::NoInsert);
        m_modelSelect->setSizeAdjustPolicy(QComboBox::AdjustToMinimumContentsLengthWithIcon);
        m_modelSelect->setMinimumContentsLength(12);
        m_modelSelect->setObjectName(QStringLiteral("agentModel"));
        m_modelSelect->setToolTip(QStringLiteral("Select or type a model configured for this provider"));
        agentPicker->addWidget(modelLabel);
        modelLabel->hide();
        agentPicker->addWidget(m_modelSelect, 2);
        auto *chatRow = new QHBoxLayout;
        chatRow->setContentsMargins(0, 0, 0, 0);
        chatRow->setSpacing(6);
        m_chatInput = new QTextEdit;
        m_chatInput->setObjectName(QStringLiteral("chatInput"));
        m_chatInput->setPlaceholderText(QStringLiteral("Ask Cut to edit…"));
        m_chatInput->setAccessibleName(QStringLiteral("Message Cut"));
        m_chatInput->setAcceptRichText(false);
        m_chatInput->setFixedHeight(76);
        m_chatInput->setTabChangesFocus(true);
        m_chatInput->installEventFilter(this);
        connect(m_chatInput, &QTextEdit::textChanged, this, &MainWindow::updateChatControls);
        composerLayout->addWidget(m_chatInput);
        composerLayout->addLayout(agentPicker);
        connect(m_providerSelect, QOverload<int>::of(&QComboBox::currentIndexChanged), this, &MainWindow::updateAgentModelChoices);
        connect(m_modelSelect, &QComboBox::currentTextChanged, this, [this] {
            if (m_agentConfigLabel) m_agentConfigLabel->setText(QStringLiteral("%1  ·  %2")
                                                                    .arg(m_providerSelect->currentText(), m_modelSelect->currentText()));
        });
        auto *sendButton = new QPushButton(QStringLiteral("↑"));
        sendButton->setObjectName(QStringLiteral("chatSendButton"));
        m_sendButton = sendButton;
        sendButton->setAccessibleName(QStringLiteral("Send message"));
        sendButton->setFixedSize(28, 28);
        sendButton->setToolTip(QStringLiteral("Send message (Return)"));
        connect(sendButton, &QPushButton::clicked, this, &MainWindow::sendChat);
        auto *composerHint = new QLabel(QStringLiteral("⇧ Return for a new line"));
        composerHint->setObjectName(QStringLiteral("muted"));
        chatRow->addWidget(composerHint);
        chatRow->addStretch();
        chatRow->addWidget(sendButton);
        auto *abortButton = new QToolButton;
        abortButton->setObjectName(QStringLiteral("abortButton"));
        abortButton->setText(QStringLiteral("Stop"));
        abortButton->setToolTip(QStringLiteral("Stop the current agent request"));
        m_abortButton = abortButton;
        abortButton->setEnabled(false);
        connect(abortButton, &QToolButton::clicked, this, [this] {
            if (!sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("abort")}})) return;
            m_abortButton->setEnabled(false);
            m_agentStatus->setText(QStringLiteral("STOPPING"));
            appendConversationNote(QStringLiteral("Progress"), QStringLiteral("Stopping request…"));
        });
        chatRow->addWidget(abortButton);
        composerLayout->addLayout(chatRow);
        auto *activityTitle = new QToolButton;
        activityTitle->setText(QStringLiteral("▸ Activity"));
        activityTitle->setObjectName(QStringLiteral("eyebrow"));
        chatLayout->addWidget(activityTitle);
        m_activityList = new QListWidget;
        m_activityList->setObjectName(QStringLiteral("activityList"));
        m_activityList->setMaximumHeight(76);
        chatLayout->addWidget(m_activityList);
        m_activityList->hide();
        connect(activityTitle, &QToolButton::clicked, this, [this, activityTitle] {
            const bool visible = !m_activityList->isVisible();
            m_activityList->setVisible(visible);
            activityTitle->setText(visible ? QStringLiteral("▾ Activity") : QStringLiteral("▸ Activity"));
        });

        m_exportProgress = new QProgressBar;
        m_exportProgress->setRange(0, 100);
        m_exportProgress->setValue(0);
        m_exportProgress->setTextVisible(false);
        m_exportProgress->setToolTip(QStringLiteral("Export and preview progress"));
        chatLayout->addWidget(m_exportProgress);
        m_exportProgress->hide();

        auto *proposalBox = new QGroupBox(QStringLiteral("PROPOSAL"));
        m_proposalBox = proposalBox;
        proposalBox->setObjectName(QStringLiteral("proposalBox"));
        auto *proposalLayout = new QVBoxLayout(proposalBox);
        m_proposalSummary = new QLabel(QStringLiteral("No pending changes"));
        m_proposalSummary->setWordWrap(true);
        m_proposalSummary->setObjectName(QStringLiteral("proposalSummary"));
        m_proposalDiff = new QTextEdit;
        m_proposalDiff->setReadOnly(true);
        m_proposalDiff->setMinimumHeight(120);
        m_proposalDiff->setMaximumHeight(240);
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
        // Keep the composer last so expanding Activity or a proposal consumes
        // conversation space instead of lifting the message input upward.
        chatLayout->addWidget(composer);
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

        auto *settingsTab = new QWidget;
        auto *settingsTabLayout = new QVBoxLayout(settingsTab);
        settingsTabLayout->setContentsMargins(0, 8, 0, 0);
        auto *settingsScroll = new QScrollArea;
        settingsScroll->setWidgetResizable(true);
        settingsScroll->setFrameShape(QFrame::NoFrame);
        auto *settingsContent = new QWidget;
        auto *settingsLayout = new QVBoxLayout(settingsContent);
        settingsLayout->setContentsMargins(0, 0, 2, 0);
        settingsLayout->setSpacing(10);
        auto *settingsIntro = new QLabel(QStringLiteral("Keep multiple provider connections ready. The provider and model selected above are used for the next request."));
        settingsIntro->setObjectName(QStringLiteral("chatHint"));
        settingsIntro->setWordWrap(true);
        settingsLayout->addWidget(settingsIntro);

        auto providerSettings = [this, settingsLayout](const QString &title, QLineEdit *&key, QLineEdit *&baseUrl,
                                                        QLineEdit *&model, const QString &keyPlaceholder,
                                                        const QString &baseDefault, const QString &modelDefault) {
            auto *group = new QGroupBox(title);
            group->setObjectName(QStringLiteral("providerBox"));
            auto *form = new QFormLayout(group);
            form->setContentsMargins(10, 12, 10, 10);
            form->setSpacing(7);
            key = new QLineEdit;
            key->setEchoMode(QLineEdit::Password);
            key->setPlaceholderText(keyPlaceholder);
            key->setClearButtonEnabled(true);
            baseUrl = new QLineEdit(baseDefault);
            baseUrl->setPlaceholderText(QStringLiteral("https://…/v1"));
            model = new QLineEdit(modelDefault);
            form->addRow(QStringLiteral("API key"), key);
            form->addRow(QStringLiteral("Base URL"), baseUrl);
            form->addRow(QStringLiteral("Model"), model);
            settingsLayout->addWidget(group);
        };
        providerSettings(QStringLiteral("Anthropic"), m_anthropicKey, m_anthropicBaseUrl, m_anthropicModel,
                         QStringLiteral("sk-ant-…"), QStringLiteral("https://api.anthropic.com/v1"),
                         QStringLiteral("claude-sonnet-4-20250514"));
        providerSettings(QStringLiteral("OpenAI"), m_openaiKey, m_openaiBaseUrl, m_openaiModel,
                         QStringLiteral("sk-…"), QStringLiteral("https://api.openai.com/v1"),
                         QStringLiteral("gpt-5.6-luna"));
        auto *openaiModeRow = new QHBoxLayout;
        auto *openaiModeLabel = new QLabel(QStringLiteral("OpenAI API"));
        openaiModeLabel->setObjectName(QStringLiteral("muted"));
        openaiModeRow->addWidget(openaiModeLabel);
        m_openaiMode = new QComboBox;
        m_openaiMode->addItem(QStringLiteral("Chat Completions"), QStringLiteral("chat"));
        m_openaiMode->addItem(QStringLiteral("Responses"), QStringLiteral("responses"));
        openaiModeRow->addWidget(m_openaiMode, 1);
        settingsLayout->addLayout(openaiModeRow);
        auto *settingsNote = new QLabel(QStringLiteral("Credentials stay outside project files and are never included in chat activity."));
        settingsNote->setObjectName(QStringLiteral("muted"));
        settingsNote->setWordWrap(true);
        settingsLayout->addWidget(settingsNote);
        auto *saveSettings = new QPushButton(QStringLiteral("Save provider settings"));
        saveSettings->setObjectName(QStringLiteral("primaryButton"));
        connect(saveSettings, &QPushButton::clicked, this, &MainWindow::saveAgentSettings);
        settingsLayout->addWidget(saveSettings);
        settingsLayout->addStretch(1);
        settingsScroll->setWidget(settingsContent);
        settingsTabLayout->addWidget(settingsScroll);
        tabs->addTab(settingsTab, QStringLiteral("Settings"));

        loadAgentSettings();
        connect(m_anthropicKey, &QLineEdit::editingFinished, this, &MainWindow::saveAgentSettings);
        connect(m_anthropicBaseUrl, &QLineEdit::editingFinished, this, &MainWindow::saveAgentSettings);
        connect(m_anthropicModel, &QLineEdit::editingFinished, this, &MainWindow::saveAgentSettings);
        connect(m_openaiKey, &QLineEdit::editingFinished, this, &MainWindow::saveAgentSettings);
        connect(m_openaiBaseUrl, &QLineEdit::editingFinished, this, &MainWindow::saveAgentSettings);
        connect(m_openaiModel, &QLineEdit::editingFinished, this, &MainWindow::saveAgentSettings);
        connect(m_openaiMode, QOverload<int>::of(&QComboBox::currentIndexChanged), this, &MainWindow::saveAgentSettings);

        workspace->addWidget(center);
        workspace->addWidget(right);
        workspace->setCollapsible(0, false);
        workspace->setCollapsible(1, false);
        workspace->setStretchFactor(0, 1);
        workspace->setSizes({1120, 380});
        setCentralWidget(central);

        connect(m_timeline, &TimelineWidget::seekRequested, this, &MainWindow::queueSeek);
        connect(m_timeline, &TimelineWidget::hoverSeekRequested, this, &MainWindow::queueHoverSeek);
        connect(m_timeline, &TimelineWidget::hoverSeekEnded, this, &MainWindow::endHoverSeek);
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

    void syncCanvasPreset(const QSize &size) {
        if (!m_canvasPreset || !size.isValid()) return;
        m_canvasSize = size;
        const QSignalBlocker blocker(m_canvasPreset);
        m_updatingCanvasPreset = true;
        const QString key = QStringLiteral("%1x%2").arg(size.width()).arg(size.height());
        int index = m_canvasPreset->findData(key);
        if (index < 0) {
            index = m_canvasPreset->findData(QStringLiteral("custom"));
            const QString label = QStringLiteral("Custom  %1 × %2").arg(size.width()).arg(size.height());
            if (index < 0) {
                m_canvasPreset->addItem(label, QStringLiteral("custom"));
                index = m_canvasPreset->count() - 1;
            } else {
                m_canvasPreset->setItemText(index, label);
            }
        }
        m_canvasPreset->setCurrentIndex(index);
        m_canvasPreset->setToolTip(QStringLiteral("Project format: %1 × %2").arg(size.width()).arg(size.height()));
        m_updatingCanvasPreset = false;
    }

    void setupActions() {
        menuBar()->setNativeMenuBar(true);
        auto *view = menuBar()->addMenu(QStringLiteral("View"));
        auto addPanelToggle = [view](const QString &title, QWidget *panel) {
            auto *action = view->addAction(title);
            action->setCheckable(true);
            action->setChecked(true);
            connect(action, &QAction::toggled, panel, &QWidget::setVisible);
        };
        addPanelToggle(QStringLiteral("Media bin"), m_mediaPanel);
        addPanelToggle(QStringLiteral("Agent and inspector"), m_agentPanel);
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
        toolbar->setIconSize(QSize(18, 18));
        if (!menuBar()->isNativeMenuBar()) {
            // Keep menus accessible without dedicating a full row to them.
            auto *menuButton = new QToolButton(toolbar);
            menuButton->setText(QStringLiteral("☰"));
            menuButton->setToolTip(QStringLiteral("Application menu"));
            menuButton->setAccessibleName(QStringLiteral("Application menu"));
            menuButton->setPopupMode(QToolButton::InstantPopup);
            auto *applicationMenu = new QMenu(menuButton);
            for (auto *menu : {view, file, edit, playback}) {
                applicationMenu->addMenu(menu);
                for (auto *action : menu->actions()) {
                    if (action->shortcutContext() != Qt::WidgetShortcut)
                        addAction(action);
                }
            }
            menuButton->setMenu(applicationMenu);
            toolbar->addWidget(menuButton);
            menuBar()->hide();
        }
        // Draw toolbar symbols on a shared grid so their weight and optical size match.
        auto icon = [](const QString &glyph) {
            QPixmap pixmap(40, 40);
            pixmap.setDevicePixelRatio(2);
            pixmap.fill(Qt::transparent);
            QPainter painter(&pixmap);
            painter.setRenderHint(QPainter::Antialiasing);
            painter.setPen(QPen(QColor("#d7d7d7"), 1.5, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
            QPainterPath path;
            if (glyph == QStringLiteral("↓")) {
                path.moveTo(10, 3); path.lineTo(10, 12);
                path.moveTo(6.5, 8.5); path.lineTo(10, 12); path.lineTo(13.5, 8.5);
                path.moveTo(4, 12); path.lineTo(4, 16); path.lineTo(16, 16); path.lineTo(16, 12);
            } else if (glyph == QStringLiteral("↶") || glyph == QStringLiteral("↷")) {
                if (glyph == QStringLiteral("↷")) { painter.translate(20, 0); painter.scale(-1, 1); }
                path.moveTo(7, 4); path.lineTo(3.5, 7.5); path.lineTo(7, 11);
                path.moveTo(4, 7.5); path.lineTo(11, 7.5);
                path.cubicTo(17, 7.5, 17, 15.5, 11, 15.5);
            } else if (glyph == QStringLiteral("✂")) {
                painter.drawEllipse(QPointF(5, 5), 2.5, 2.5);
                painter.drawEllipse(QPointF(5, 15), 2.5, 2.5);
                path.moveTo(7, 6.5); path.lineTo(16, 16);
                path.moveTo(7, 13.5); path.lineTo(16, 4);
            } else {
                path.moveTo(5, 5); path.lineTo(15, 15);
                path.moveTo(15, 5); path.lineTo(5, 15);
            }
            painter.drawPath(path);
            return QIcon(pixmap);
        };
        import->setIcon(icon(QStringLiteral("↓")));
        undoAction->setIcon(icon(QStringLiteral("↶")));
        redoAction->setIcon(icon(QStringLiteral("↷")));
        toolbar->addAction(import);
        toolbar->addSeparator();
        toolbar->addAction(undoAction);
        toolbar->addAction(redoAction);
        toolbar->addSeparator();
        auto *split = toolbar->addAction(QStringLiteral("Split"));
        split->setIcon(icon(QStringLiteral("✂")));
        connect(split, &QAction::triggered, this, &MainWindow::splitClip);
        auto *deleteToolbar = toolbar->addAction(QStringLiteral("Delete"));
        deleteToolbar->setIcon(icon(QStringLiteral("×")));
        connect(deleteToolbar, &QAction::triggered, this, &MainWindow::deleteSelected);
        toolbar->addSeparator();
        auto *spacer = new QWidget;
        spacer->setStyleSheet(QStringLiteral("background: transparent"));
        spacer->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Preferred);
        toolbar->addWidget(spacer);
        auto *projectTitle = new QLabel(QStringLiteral("CUT  /  %1").arg(QFileInfo(m_projectDirectory).fileName()));
        m_projectTitle = projectTitle;
        projectTitle->setObjectName(QStringLiteral("projectTitle"));
        toolbar->addWidget(projectTitle);
        auto *spacerRight = new QWidget;
        spacerRight->setStyleSheet(QStringLiteral("background: transparent"));
        spacerRight->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Preferred);
        toolbar->addWidget(spacerRight);
        m_quality = new QualityComboBox;
        m_quality->setObjectName(QStringLiteral("qualityPicker"));
        m_quality->addItem(QStringLiteral("Preview"), QStringLiteral("preview"));
        m_quality->addItem(QStringLiteral("Full"), QStringLiteral("full"));
        toolbar->addWidget(m_quality);
        m_canvasPreset = new QualityComboBox;
        m_canvasPreset->setObjectName(QStringLiteral("canvasPicker"));
        m_canvasPreset->addItem(QStringLiteral("16:9  1920 × 1080"), QStringLiteral("1920x1080"));
        m_canvasPreset->addItem(QStringLiteral("9:16  1080 × 1920"), QStringLiteral("1080x1920"));
        m_canvasPreset->setToolTip(QStringLiteral("Project format"));
        connect(m_canvasPreset, qOverload<int>(&QComboBox::currentIndexChanged),
                this, &MainWindow::canvasPresetChanged);
        toolbar->addWidget(m_canvasPreset);
        auto *exportToolbar = toolbar->addAction(QStringLiteral("Export"));
        connect(exportToolbar, &QAction::triggered, this, &MainWindow::exportProject);
        m_previewButton = toolbar->addAction(QStringLiteral("Render preview"));
        m_previewButton->setEnabled(false);
        connect(m_previewButton, &QAction::triggered, this, &MainWindow::renderPreview);
        m_runtimeActions = {import, exportAction, refreshAction, splitAction, deleteAction, play, split, deleteToolbar, exportToolbar};
    }

    QJsonValue revisionValue() const { return QJsonValue::fromVariant(QVariant::fromValue(m_revision)); }

    bool sendSimple(const QJsonObject &value) {
        if (!m_connected && value.value(QStringLiteral("type")).toString() != QStringLiteral("reconnect") &&
            value.value(QStringLiteral("type")).toString() != QStringLiteral("abort")) {
            return false;
        }
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
        if (m_hasPendingSeek) {
            m_hasPendingSeek = false;
            sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("seek")}, {QStringLiteral("position"), m_pendingSeek}});
            return;
        }
        if (m_hasPendingHoverSeek) {
            m_hasPendingHoverSeek = false;
            sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("seek")}, {QStringLiteral("position"), m_pendingHoverSeek}});
        }
    }

    void handleEvent(QJsonObject event) {
        const QString type = event.value(QStringLiteral("type")).toString();
        if (type == QStringLiteral("connection")) {
            setConnection(event.value(QStringLiteral("status")).toString(), event.value(QStringLiteral("message")).toString());
        } else if (type == QStringLiteral("state")) {
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
            if (kind == QStringLiteral("delta")) {
                if (m_agentText.isEmpty()) appendConversation(QStringLiteral("Cut"), text);
                else {
                    QTextCursor cursor(m_conversation->document());
                    cursor.movePosition(QTextCursor::End);
                    cursor.insertText(text);
                    m_conversation->ensureCursorVisible();
                }
                m_agentText += text;
            } else if (kind == QStringLiteral("done")) {
                if (m_agentText.isEmpty()) appendConversation(QStringLiteral("Cut"), text);
                m_agentText.clear();
            } else if (kind == QStringLiteral("tool")) {
                appendConversationNote(QStringLiteral("Tool"), text);
                addActivity(QStringLiteral("tool"), text.left(120));
            } else if (kind == QStringLiteral("status")) {
                appendConversationNote(QStringLiteral("Progress"), text);
                addActivity(QStringLiteral("status"), text.left(120));
            } else {
                appendConversationNote(QStringLiteral("Agent"), text);
                addActivity(kind, text.left(120));
            }
        } else if (type == QStringLiteral("agent_busy")) {
            const bool busy = event.value(QStringLiteral("busy")).toBool();
            m_agentBusy = busy;
            if (!m_connected) m_agentStatus->setText(QStringLiteral("OFFLINE"));
            else if (busy) m_agentStatus->setText(QStringLiteral("THINKING"));
            else if (m_agentRunState == QStringLiteral("failed")) m_agentStatus->setText(QStringLiteral("FAILED"));
            else if (m_agentRunState == QStringLiteral("aborted")) m_agentStatus->setText(QStringLiteral("STOPPED"));
            else if (m_agentRunState == QStringLiteral("awaiting_review")) m_agentStatus->setText(QStringLiteral("REVIEW"));
            else m_agentStatus->setText(QStringLiteral("READY"));
            m_chatInput->setEnabled(m_connected && !busy);
            updateChatControls();
        } else if (type == QStringLiteral("agent_state")) {
            const QString state = event.value(QStringLiteral("state")).toString();
            m_agentRunState = state;
            const QString label = state == QStringLiteral("running") ? QStringLiteral("THINKING")
                                  : state == QStringLiteral("awaiting_review") ? QStringLiteral("REVIEW")
                                  : state == QStringLiteral("failed") ? QStringLiteral("FAILED")
                                  : state == QStringLiteral("aborted") ? QStringLiteral("STOPPED")
                                  : QStringLiteral("READY");
            m_agentStatus->setText(m_connected ? label : QStringLiteral("OFFLINE"));
            if (state == QStringLiteral("failed")) {
                appendConversationNote(QStringLiteral("Agent error"), event.value(QStringLiteral("message")).toString());
            }
        } else if (type == QStringLiteral("clock")) {
            if (event.value(QStringLiteral("duration")).isDouble()) m_duration = std::max(0.0, event.value(QStringLiteral("duration")).toDouble());
            if (!m_hoverSkimming || m_playing) {
                m_playhead = std::clamp(event.value(QStringLiteral("position")).toDouble(), 0.0, m_duration);
                m_timeline->setPlayhead(m_playhead);
            }
            updateTransportTime();
        } else if (type == QStringLiteral("playback")) {
            m_playing = m_connected && event.value(QStringLiteral("playing")).toBool();
            if (m_playing) {
                m_hasPendingHoverSeek = false;
                m_hoverSkimming = false;
                m_timeline->clearSkimmer();
            }
            m_playButton->setText(m_playing ? QStringLiteral("Ⅱ") : QStringLiteral("▶"));
        } else if (type == QStringLiteral("export")) {
            const double progress = event.value(QStringLiteral("progress")).toDouble();
            m_exportProgress->setValue(static_cast<int>(std::clamp(progress, 0.0, 1.0) * 100.0));
            const QString status = event.value(QStringLiteral("status")).toString();
            const bool finished = status == QStringLiteral("ready") || status == QStringLiteral("complete") ||
                                  status == QStringLiteral("completed") || status == QStringLiteral("failed") ||
                                  status == QStringLiteral("cancelled") || status == QStringLiteral("canceled");
            m_exportProgress->setVisible(!finished);
            if (!status.isEmpty()) statusBar()->showMessage(QStringLiteral("Export %1").arg(status));
            if (!event.value(QStringLiteral("path")).toString().isEmpty()) addActivity(QStringLiteral("export"), event.value(QStringLiteral("path")).toString());
        } else if (type == QStringLiteral("preview")) {
            const QString status = event.value(QStringLiteral("status")).toString();
            const QString message = event.value(QStringLiteral("message")).toString();
            if (status == QStringLiteral("required") || status == QStringLiteral("unavailable")) {
                m_previewRequired = true;
                m_previewButton->setEnabled(m_connected && status == QStringLiteral("required"));
                m_playButton->setEnabled(false);
                m_viewer->clearFrame();
                const QString detail = message.isEmpty() ? QStringLiteral("Rendered preview required") : message;
                statusBar()->setToolTip(detail);
                statusBar()->showMessage(detail.left(112) + (detail.size() > 112 ? QStringLiteral("…") : QString()), 6000);
                addActivity(QStringLiteral("preview"), message.isEmpty() ? QStringLiteral("Native preview unavailable") : message);
            } else if (status == QStringLiteral("running")) {
                m_previewRequired = true;
                m_previewButton->setEnabled(false);
                m_playButton->setEnabled(false);
                m_exportProgress->setValue(static_cast<int>(std::clamp(event.value(QStringLiteral("progress")).toDouble(), 0.0, 1.0) * 100.0));
                m_exportProgress->show();
            } else if (status == QStringLiteral("ready")) {
                m_previewRequired = false;
                m_previewButton->setEnabled(false);
                m_playButton->setEnabled(m_connected && m_duration > 0);
                m_exportProgress->setValue(0);
                m_exportProgress->hide();
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
        m_undoAction->setEnabled(m_connected && snapshot.value("canUndo").toBool());
        m_redoAction->setEnabled(m_connected && snapshot.value("canRedo").toBool());
        m_fps = m_project.value(QStringLiteral("fps")).toDouble(24.0);
        m_projectTitle->setText(m_project.value(QStringLiteral("name")).toString(QStringLiteral("Untitled project")));
        const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
        m_trackCount = tracks.size();
        if (!m_selectedClip.isEmpty() && selectedClipObject().isEmpty()) m_selectedClip.clear();
        m_timeline->setProject(m_project, m_duration, m_fps);
        m_timeline->setSelectedClip(m_selectedClip);
        const QJsonArray sources = snapshot.value(QStringLiteral("sources")).toArray();
        const QSize canvasSize = resolvedCanvasSize(m_project, sources);
        m_viewer->setProjectInfo(canvasSize.width(), canvasSize.height());
        syncCanvasPreset(canvasSize);
        m_artwork->request(sources, m_projectDirectory);
        if (sources != m_displayedSources) {
            const QString selectedSource = m_mediaList->currentItem() ? m_mediaList->currentItem()->data(Qt::UserRole).toString() : QString();
            m_mediaList->clear();
            for (const QJsonValue &value : sources) {
                const QJsonObject source = value.toObject();
                auto *item = new QListWidgetItem(sourceName(source), m_mediaList);
                item->setToolTip(source.value(QStringLiteral("path")).toString());
                item->setData(Qt::UserRole, source.value(QStringLiteral("id")).toString());
                const QString sourceId = source.value("id").toString();
                item->setIcon(m_artwork->icon(sourceId));
                if (item->data(Qt::UserRole).toString() == selectedSource) m_mediaList->setCurrentItem(item);
            }
            if (!sources.isEmpty()) {
                m_mediaList->show();
                m_mediaHint->hide();
                m_mediaToggle->setText(QStringLiteral("▾"));
                m_mediaToggle->setToolTip(QStringLiteral("Collapse media bin"));
            }
            m_displayedSources = sources;
        }
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
        const bool hasClip = m_connected && !clip.isEmpty();
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
        m_acceptButton->setEnabled(m_connected && pending);
        m_rejectButton->setEnabled(m_connected && pending);
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
        auto displayValue = [](const QJsonValue &entry) {
            if (entry.isUndefined()) return QStringLiteral("—");
            if (entry.isString()) return entry.toString();
            if (entry.isObject()) {
                const auto object = entry.toObject();
                if (object.contains(QStringLiteral("start")) && object.contains(QStringLiteral("end")))
                    return QStringLiteral("%1 – %2").arg(prettyTime(object.value(QStringLiteral("start")).toDouble()), prettyTime(object.value(QStringLiteral("end")).toDouble()));
                return QString::fromUtf8(QJsonDocument(object).toJson(QJsonDocument::Indented));
            }
            if (entry.isArray()) return QString::fromUtf8(QJsonDocument(entry.toArray()).toJson(QJsonDocument::Indented));
            const QByteArray encoded = QJsonDocument(QJsonArray{entry}).toJson(QJsonDocument::Compact);
            return QString::fromUtf8(encoded.mid(1, encoded.size() - 2));
        };
        auto entityName = [this](const QString &id) {
            for (const auto &trackValue : m_project.value(QStringLiteral("tracks")).toArray()) {
                const auto track = trackValue.toObject();
                if (track.value(QStringLiteral("id")).toString() == id) return track.value(QStringLiteral("name")).toString(id);
                for (const auto &clipValue : track.value(QStringLiteral("clips")).toArray()) {
                    const auto clip = clipValue.toObject();
                    if (clip.value(QStringLiteral("id")).toString() == id) return clipName(clip);
                }
            }
            return id;
        };
        QString html;
        if (stale) html += QStringLiteral("<p><b>This proposal is out of date. Ask Cut to prepare it again before accepting.</b></p>");
        const auto impacts = proposal.value(QStringLiteral("impacts")).toArray();
        for (const auto &entry : impacts) {
            const auto impact = entry.toObject();
            QString kind = impact.value(QStringLiteral("kind")).toString();
            kind.replace(QLatin1Char('_'), QLatin1Char(' '));
            const QString id = impact.value(QStringLiteral("clipId")).toString(impact.value(QStringLiteral("trackId")).toString());
            html += QStringLiteral("<p><b>%1%2</b>").arg(kind.toHtmlEscaped(), id.isEmpty() ? QString() : QStringLiteral(" · ") + entityName(id).toHtmlEscaped());
            if (impact.contains(QStringLiteral("property"))) html += QStringLiteral("<br/>%1").arg(impact.value(QStringLiteral("property")).toString().toHtmlEscaped());
            for (const QString &key : {QStringLiteral("timeline"), QStringLiteral("source"), QStringLiteral("from"), QStringLiteral("to"), QStringLiteral("before"), QStringLiteral("after"), QStringLiteral("sourceBefore"), QStringLiteral("sourceAfter")}) {
                if (impact.contains(key)) html += QStringLiteral("<br/>%1: %2").arg(key.toHtmlEscaped(), displayValue(impact.value(key)).toHtmlEscaped());
            }
            html += QStringLiteral("</p>");
        }
        html += QStringLiteral("<p><b>All field changes</b></p>");
        for (const auto &entry : diff) {
            const auto change = entry.toObject();
            const QString path = change.value(QStringLiteral("path")).toString();
            QStringList parts;
            QString part;
            bool escaped = false;
            for (const QChar ch : path.mid(path.startsWith(QLatin1Char('$')) ? 1 : 0)) {
                if (escaped) { part += ch; escaped = false; }
                else if (ch == QLatin1Char('\\')) escaped = true;
                else if (ch == QLatin1Char('.') || ch == QLatin1Char('[') || ch == QLatin1Char(']')) {
                    if (!part.isEmpty()) { parts << part; part.clear(); }
                } else part += ch;
            }
            if (!part.isEmpty()) parts << part;
            QStringList labels;
            QJsonValue current(m_project);
            for (const QString &part : parts) {
                if (current.isArray()) {
                    bool valid = false;
                    const int index = part.toInt(&valid);
                    const auto array = current.toArray();
                    current = valid && index >= 0 && index < array.size() ? array.at(index) : QJsonValue();
                    const auto object = current.toObject();
                    labels << (object.contains(QStringLiteral("id")) ? entityName(object.value(QStringLiteral("id")).toString()) : part);
                } else {
                    current = current.toObject().value(part);
                    labels << part;
                }
            }
            html += QStringLiteral("<p><b>%1</b><br/>Before: %2<br/>After: %3</p>")
                .arg(labels.isEmpty() ? QStringLiteral("Project") : labels.join(QStringLiteral(" › ")).toHtmlEscaped(),
                     displayValue(change.value(QStringLiteral("before"))).toHtmlEscaped(),
                     displayValue(change.value(QStringLiteral("after"))).toHtmlEscaped());
        }
        const bool truncated = proposal.value(QStringLiteral("diffTruncated")).toBool();
        if (truncated) html += QStringLiteral("<p><b>This proposal is too large to review completely. Ask Cut to split it into smaller changes.</b></p>");
        m_proposalDiff->setHtml(html);
        m_acceptButton->setEnabled(m_connected && !stale && !truncated);
        m_acceptButton->setToolTip(truncated ? QStringLiteral("Split this proposal into smaller changes to review all fields") : QString());
    }

    void appendConversation(const QString &speaker, const QString &text) {
        if (text.isEmpty()) return;
        if (m_conversationEmpty) {
            m_conversation->clear();
            m_conversationEmpty = false;
        }
        const bool user = speaker == QStringLiteral("You");
        const bool error = speaker == QStringLiteral("Error");
        const QString role = speaker.toHtmlEscaped();
        const QString roleColor = user ? QStringLiteral("#d2d2d2")
                                       : error ? QStringLiteral("#939393") : QStringLiteral("#afafaf");
        const QString bubbleColor = user ? QStringLiteral("#2b2b2b")
                                         : error ? QStringLiteral("#272727") : QStringLiteral("#262626");
        const QString bodyColor = error ? QStringLiteral("#d0d0d0") : QStringLiteral("#e2e2e2");
        const QString body = text.toHtmlEscaped().replace(QStringLiteral("\n"), QStringLiteral("<br/>"));
        m_conversation->append(QStringLiteral(
            "<p style='margin:0 0 10px 0; padding:9px 10px; background-color:%1;'>"
            "<span style='color:%2; font-size:12px; font-weight:600;'>%3</span><br/>"
            "<span style='color:%4; font-size:13px;'>%5</span></p>")
                                    .arg(bubbleColor, roleColor, role, bodyColor, body));
        m_conversation->ensureCursorVisible();
    }

    void appendConversationNote(const QString &kind, const QString &text) {
        if (text.isEmpty()) return;
        if (m_conversationEmpty) {
            m_conversation->clear();
            m_conversationEmpty = false;
        }
        const QString label = kind.toHtmlEscaped().toUpper();
        const QString body = text.toHtmlEscaped().replace(QStringLiteral("\n"), QStringLiteral("<br/>"));
        const QString color = kind == QStringLiteral("Tool") ? QStringLiteral("#acacac") : QStringLiteral("#7c7c7c");
        m_conversation->append(QStringLiteral(
            "<p style='margin:0 0 7px 0; padding:2px 4px;'>"
            "<span style='color:%1; font-size:12px; font-weight:600;'>%2</span>"
            "<span style='color:#a1a1a1; font-size:12px;'>  %3</span></p>")
                                    .arg(color, label, body));
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
        m_errorMessage->setText(message);
        m_errorBanner->show();
    }

    void setConnection(const QString &status, const QString &message = QString()) {
        const bool wasConnected = m_connected;
        m_connected = status == QStringLiteral("ready");
        m_connectionBanner->setVisible(!m_connected);
        m_reconnectButton->setEnabled(status == QStringLiteral("disconnected"));
        m_connectionMessage->setText(status == QStringLiteral("connecting") ? QStringLiteral("Connecting to project…")
            : QStringLiteral("Project disconnected. %1").arg(message));
        m_agentStatus->setText(m_connected ? (m_agentBusy ? QStringLiteral("THINKING") : QStringLiteral("READY"))
            : status == QStringLiteral("connecting") ? QStringLiteral("CONNECTING") : QStringLiteral("OFFLINE"));
        m_mediaPanel->setEnabled(m_connected);
        m_timeline->setEnabled(m_connected);
        if (m_canvasPreset) m_canvasPreset->setEnabled(m_connected);
        m_chatInput->setEnabled(m_connected && !m_agentBusy);
        for (auto *action : m_runtimeActions) action->setEnabled(m_connected);
        m_undoAction->setEnabled(m_connected && m_snapshot.value(QStringLiteral("canUndo")).toBool());
        m_redoAction->setEnabled(m_connected && m_snapshot.value(QStringLiteral("canRedo")).toBool());
        m_playButton->setEnabled(m_connected && !m_previewRequired && m_duration > 0);
        m_previewButton->setEnabled(m_connected && m_previewRequired);
        if (!m_connected) {
            m_playing = false;
            m_playButton->setText(QStringLiteral("▶"));
            m_hasPendingSeek = false;
            m_hasPendingHoverSeek = false;
            m_hoverSkimming = false;
            m_timeline->clearSkimmer();
            m_viewer->clearFrame();
            m_viewer->setCaption(QString());
        } else if (!wasConnected) {
            addActivity(QStringLiteral("system"), QStringLiteral("Project connected"));
        }
        updateInspector();
        updateProposal(m_snapshot.value(QStringLiteral("pendingProposal")));
        updateChatControls();
        QTimer::singleShot(0, this, [this] {
            if (m_timelineScroll && m_timeline && m_timelineScroll->viewport()->width() > 0) {
                m_timeline->fitToWidth(m_timelineScroll->viewport()->width());
                m_timelineScroll->horizontalScrollBar()->setValue(0);
            }
        });
    }

    void updateTransportTime() {
        m_timeLabel->setText(QStringLiteral("%1 / %2").arg(prettyTime(m_playhead), prettyTime(m_duration)));
    }

    MediaArtwork *m_artwork = nullptr;
    QLabel *m_projectTitle = nullptr;
    QString m_projectDirectory;
    QString m_cutdBinary;
    void *m_bridge = nullptr;
    QTimer m_pollTimer;
    QJsonObject m_snapshot;
    QJsonObject m_project;
    QJsonArray m_displayedSources;
    bool m_connected = false;
    QFrame *m_connectionBanner = nullptr;
    QLabel *m_connectionMessage = nullptr;
    QPushButton *m_reconnectButton = nullptr;
    QFrame *m_errorBanner = nullptr;
    QLabel *m_errorMessage = nullptr;
    QWidget *m_mediaPanel = nullptr;
    QLabel *m_mediaHint = nullptr;
    QToolButton *m_mediaToggle = nullptr;
    QWidget *m_agentPanel = nullptr;
    QScrollArea *m_timelineScroll = nullptr;
    QList<QAction *> m_runtimeActions;
    quint64 m_revision = 0;
    int m_trackCount = 0;
    double m_duration = 0.0;
    double m_fps = 24.0;
    double m_playhead = 0.0;
    double m_pendingSeek = 0.0;
    double m_pendingHoverSeek = 0.0;
    bool m_hasPendingSeek = false;
    bool m_hasPendingHoverSeek = false;
    bool m_playing = false;
    bool m_previewRequired = false;
    bool m_hoverSkimming = false;
    bool m_updatingInspector = false;
    bool m_updatingCanvasPreset = false;
    bool m_conversationEmpty = true;
    bool m_agentBusy = false;
    QString m_agentRunState = QStringLiteral("idle");
    QString m_agentText;
    QString m_selectedClip;
    QString m_proposalId;
    QStringList m_activityLog;
    QSize m_canvasSize{1920, 1080};

    ViewerWidget *m_viewer = nullptr;
    TimelineWidget *m_timeline = nullptr;
    QListWidget *m_mediaList = nullptr;
    QTextEdit *m_conversation = nullptr;
    QTextEdit *m_chatInput = nullptr;
    QComboBox *m_providerSelect = nullptr;
    QComboBox *m_modelSelect = nullptr;
    QLabel *m_agentConfigLabel = nullptr;
    QLineEdit *m_anthropicKey = nullptr;
    QLineEdit *m_anthropicBaseUrl = nullptr;
    QLineEdit *m_anthropicModel = nullptr;
    QLineEdit *m_openaiKey = nullptr;
    QLineEdit *m_openaiBaseUrl = nullptr;
    QLineEdit *m_openaiModel = nullptr;
    QComboBox *m_openaiMode = nullptr;
    QPushButton *m_sendButton = nullptr;
    QToolButton *m_abortButton = nullptr;
    QListWidget *m_activityList = nullptr;
    QLabel *m_agentStatus = nullptr;
    QLabel *m_proposalSummary = nullptr;
    QTextEdit *m_proposalDiff = nullptr;
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
    QComboBox *m_canvasPreset = nullptr;
    QProgressBar *m_exportProgress = nullptr;
    QAction *m_undoAction = nullptr;
    QAction *m_redoAction = nullptr;
    QAction *m_previewButton = nullptr;
};

#ifdef CUT_QT_TESTING
class EditorTest final : public QObject {
    Q_OBJECT
private slots:
    void viewerKeepsLandscapeAndShortsInsideOneCanvasContract() {
        ViewerWidget viewer;
        viewer.resize(800, 600);

        viewer.setProjectInfo(1920, 1080);
        const QRectF landscape = viewer.videoRect();
        QVERIFY(std::abs(landscape.width() / landscape.height() - 16.0 / 9.0) < 0.001);
        QVERIFY(viewer.rect().contains(landscape.toAlignedRect()));
        QVERIFY(landscape.contains(viewer.captionRect()));

        viewer.setProjectInfo(1080, 1920);
        const QRectF shorts = viewer.videoRect();
        QVERIFY(std::abs(shorts.width() / shorts.height() - 9.0 / 16.0) < 0.001);
        QVERIFY(viewer.rect().contains(shorts.toAlignedRect()));
        QVERIFY(shorts.contains(viewer.captionRect()));
    }

    void viewerCanvasResolutionMatchesProjectThenSourceThenDefault() {
        QCOMPARE(resolvedCanvasSize(QJsonObject{{"width", 1920}, {"height", 1080}},
                                    QJsonArray{QJsonObject{{"width", 1080}, {"height", 1920}}}),
                 QSize(1920, 1080));
        QCOMPARE(resolvedCanvasSize(QJsonObject{},
                                    QJsonArray{QJsonObject{{"width", 1280}, {"height", 720}},
                                               QJsonObject{{"width", 1080}, {"height", 1920}}}),
                 QSize(1080, 1920));
        QCOMPARE(resolvedCanvasSize(QJsonObject{}, QJsonArray{}), QSize(1920, 1080));
    }

    void editorChromeKeepsComposerAtTheBottom() {
        MainWindow window{QString(), QString()};
        window.show();
        QTest::qWait(20);

        QVERIFY(window.statusBar()->isHidden());
        auto *title = window.findChild<QLabel *>(QStringLiteral("assistantTitle"));
        QVERIFY(title);
        QCOMPARE(title->text(), QStringLiteral("Cut Assistant"));

        auto *composer = window.findChild<QFrame *>(QStringLiteral("chatComposer"));
        QVERIFY(composer);
        QToolButton *activity = nullptr;
        for (auto *button : window.findChildren<QToolButton *>()) {
            if (button->text().endsWith(QStringLiteral("Activity"))) {
                activity = button;
                break;
            }
        }
        QVERIFY(activity);
        QVERIFY(activity->mapTo(&window, QPoint{}).y() < composer->mapTo(&window, QPoint{}).y());
        QVERIFY(window.m_exportProgress->isHidden());

        window.handleEvent(QJsonObject{{"type", "export"}, {"status", "running"}, {"progress", 0.5}});
        QVERIFY(!window.m_exportProgress->isHidden());
        QCOMPARE(window.m_exportProgress->value(), 50);
        window.handleEvent(QJsonObject{{"type", "export"}, {"status", "completed"}, {"progress", 1.0}});
        QVERIFY(window.m_exportProgress->isHidden());
    }

    void chatComposerSupportsMultilineWithoutAccidentalSend() {
        MainWindow window{QString(), QString()};
        window.show();
        window.m_chatInput->setEnabled(true);
        window.m_chatInput->setFocus();
        window.m_chatInput->setPlainText(QStringLiteral("First edit"));
        window.m_chatInput->moveCursor(QTextCursor::End);
        QTest::keyClick(window.m_chatInput, Qt::Key_Return, Qt::ShiftModifier);
        QTest::keyClicks(window.m_chatInput, "Second edit");
        QCOMPARE(window.m_chatInput->toPlainText(), QStringLiteral("First edit\nSecond edit"));
        // Return must preserve a draft while sending is unavailable.
        QTest::keyClick(window.m_chatInput, Qt::Key_Return);
        QCOMPARE(window.m_chatInput->toPlainText(), QStringLiteral("First edit\nSecond edit"));
        QVERIFY(!window.m_sendButton->isEnabled());
        QVERIFY(window.m_abortButton->isHidden());
    }

    void artworkDecodesMediaWithoutBlocking() {
        if (QStandardPaths::findExecutable(QStringLiteral("ffmpeg")).isEmpty()) QSKIP("FFmpeg unavailable");
        QTemporaryDir directory;
        QVERIFY(directory.isValid());
        const QString path = directory.filePath(QStringLiteral("tone.wav"));
        QCOMPARE(QProcess::execute(QStringLiteral("ffmpeg"), {"-v", "error", "-f", "lavfi", "-i", "sine=frequency=440:duration=0.2", path}), 0);
        MediaArtwork artwork;
        artwork.request(QJsonArray{QJsonObject{{"id", "tone"}, {"path", path}, {"hasAudio", true}}}, directory.path());
        QTRY_VERIFY_WITH_TIMEOUT(artwork.waves.contains(QStringLiteral("tone")), 5000);
        QVERIFY(!artwork.waves.value(QStringLiteral("tone")).isNull());
        QVERIFY(!artwork.frames.contains(QStringLiteral("tone")));
    }

    void disconnectedControlsAndRecovery() {
        MainWindow window{QString(), QString()};
        QCOMPARE(window.m_agentStatus->text(), QStringLiteral("CONNECTING"));
        QVERIFY(!window.m_sendButton->isEnabled());
        window.handleEvent(QJsonObject{{"type", "connection"}, {"status", "disconnected"}, {"message", "Daemon unavailable"}});
        QCOMPARE(window.m_agentStatus->text(), QStringLiteral("OFFLINE"));
        QVERIFY(window.m_reconnectButton->isEnabled());
        QVERIFY(!window.m_mediaPanel->isEnabled());
        QVERIFY(!window.m_connectionBanner->isHidden());
        QVERIFY(window.m_connectionMessage->text().contains(QStringLiteral("Daemon unavailable")));
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", false}});
        QCOMPARE(window.m_agentStatus->text(), QStringLiteral("OFFLINE"));
        window.handleEvent(QJsonObject{{"type", "connection"}, {"status", "ready"}});
        QVERIFY(window.m_connectionBanner->isHidden());
        QVERIFY(window.m_mediaPanel->isEnabled());
        window.m_chatInput->setText(QStringLiteral("테스트 편집"));
        QVERIFY(window.m_sendButton->isEnabled());
        const auto conversation = window.m_conversation->toPlainText();
        const int activityCount = window.m_activityList->count();
        window.showError(QStringLiteral("Render failed"));
        window.showError(QStringLiteral("Render failed"));
        QCOMPARE(window.m_conversation->toPlainText(), conversation);
        QCOMPARE(window.m_activityList->count(), activityCount);
        QCOMPARE(window.m_errorMessage->text(), QStringLiteral("Render failed"));
    }

    void timelineFitsAndAllTracksRemainReachable() {
        QScrollArea scroll;
        scroll.resize(640, 230);
        scroll.setWidgetResizable(true);
        auto *timeline = new TimelineWidget;
        scroll.setWidget(timeline);
        scroll.show();
        QTest::qWait(20);
        QCOMPARE(scroll.horizontalScrollBar()->maximum(), 0);
        QJsonArray tracks;
        for (int i = 0; i < 12; ++i) tracks.append(QJsonObject{{"id", QString::number(i)}, {"clips", QJsonArray{}}});
        timeline->setProject(QJsonObject{{"tracks", tracks}}, 3600, 24);
        timeline->fitToWidth(scroll.viewport()->width());
        QTest::qWait(20);
        // The vertical scrollbar changes the available width.
        timeline->fitToWidth(scroll.viewport()->width());
        QTest::qWait(20);
        QCOMPARE(scroll.horizontalScrollBar()->maximum(), 0);
        QVERIFY(scroll.verticalScrollBar()->maximum() > 0);
        QVERIFY(timeline->minimumHeight() >= kRulerHeight + 12 * kTrackHeight);
        timeline->setZoom(1.0);
        QTest::qWait(20);
        QVERIFY(scroll.horizontalScrollBar()->maximum() > 0);
    }

    void proposalShowsAllChangesAndGuardsIncompleteReview() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.m_project = QJsonObject{{"tracks", QJsonArray{QJsonObject{{"id", "video"}, {"clips", QJsonArray{QJsonObject{{"id", "clip-1"}, {"name", "Opening shot"}}}}}}}};
        QJsonArray diff;
        for (int i = 0; i < 6; ++i) diff.append(QJsonObject{{"path", QStringLiteral("$.metadata.field%1").arg(i)}, {"before", i}, {"after", i + 10}});
        diff.append(QJsonObject{{"path", "$.metadata.name"}, {"before", "old"}, {"after", "<b>literal</b>"}});
        diff.append(QJsonObject{{"path", "$.tracks[0].clips[0].out"}, {"before", 8}, {"after", 4}});
        diff.append(QJsonObject{{"path", "$.tracks[1]"}, {"after", QJsonObject{{"id", "new-track"}, {"clips", QJsonArray{}}}}});
        QJsonObject proposal{{"status", "pending"}, {"proposalId", "proposal-1"}, {"diff", diff}, {"impacts", QJsonArray{QJsonObject{{"kind", "clip_trimmed"}, {"clipId", "clip-1"}, {"before", QJsonObject{{"start", 0}, {"end", 8}}}, {"after", QJsonObject{{"start", 0}, {"end", 4}}}}}}};
        window.updateProposal(proposal);
        const auto text = window.m_proposalDiff->toPlainText();
        QVERIFY(text.contains(QStringLiteral("field5")));
        QVERIFY(text.contains(QStringLiteral("After: 15")));
        QVERIFY(text.contains(QStringLiteral("Opening shot")));
        QVERIFY(text.contains(QStringLiteral("Opening shot › out")));
        QVERIFY(text.contains(QStringLiteral("new-track")));
        QVERIFY(text.contains(QStringLiteral("00:04.00")));
        QVERIFY(text.contains(QStringLiteral("<b>literal</b>")));
        QVERIFY(window.m_acceptButton->isEnabled());
        proposal.insert(QStringLiteral("freshness"), QStringLiteral("stale"));
        window.updateProposal(proposal);
        QVERIFY(!window.m_acceptButton->isEnabled());
        proposal.remove(QStringLiteral("freshness"));
        proposal.insert(QStringLiteral("diffTruncated"), true);
        window.updateProposal(proposal);
        QVERIFY(!window.m_acceptButton->isEnabled());
        QVERIFY(window.m_proposalDiff->toPlainText().contains(QStringLiteral("smaller changes")));
    }
};
#endif

} // namespace

#include "main.moc"

#ifdef CUT_QT_TESTING
QTEST_MAIN(EditorTest)
#else
int main(int argc, char **argv) {
    QApplication app(argc, argv);
    QApplication::setApplicationName(QStringLiteral("Cut"));
    QApplication::setOrganizationName(QStringLiteral("Cut"));
    QFont interfaceFont = QFontDatabase::systemFont(QFontDatabase::GeneralFont);
    if (QFontDatabase::families().contains(QStringLiteral("Avenir Next")))
        interfaceFont.setFamily(QStringLiteral("Avenir Next"));
    interfaceFont.setPixelSize(13);
    interfaceFont.setWeight(QFont::Normal);
    app.setFont(interfaceFont);
    app.setStyle(QStringLiteral("Fusion"));
    app.setStyleSheet(editorStyleSheet());

    QString project;
    QString cutd;
    QString screenshotPath;
    bool renderPreview = false;
    int quitAfterMs = 0;
    QStringList positional;
    for (int i = 1; i < argc; ++i) {
        const QString argument = QString::fromUtf8(argv[i]);
        if (argument == QStringLiteral("--render-preview")) {
            renderPreview = true;
        } else if (argument == QStringLiteral("--screenshot") && i + 1 < argc) {
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
    if (renderPreview) {
        QTimer::singleShot(1500, &window, [&window] {
            QMetaObject::invokeMethod(&window, "renderPreview", Qt::DirectConnection);
        });
    }
    if (!screenshotPath.isEmpty() || quitAfterMs > 0) {
        const int delay = quitAfterMs > 0 ? quitAfterMs : 500;
        QTimer::singleShot(delay, &app, [&window, &app, screenshotPath] {
            if (!screenshotPath.isEmpty()) window.grab().save(screenshotPath);
            app.quit();
        });
    }
    return app.exec();
}
#endif
