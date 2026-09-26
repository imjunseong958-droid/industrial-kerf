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
#include <QClipboard>
#include <QCheckBox>
#include <QComboBox>
#include <QDateTime>
#include <QDir>
#include <QDialog>
#include <QDialogButtonBox>
#include <QDoubleSpinBox>
#include <QFileDialog>
#include <QFile>
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
#include <QMenu>
#include <QMenuBar>
#include <QMouseEvent>
#include <QMessageBox>
#include <QPainter>
#include <QPainterPath>
#include <QPixmap>
#include <QPolygonF>
#include <QProgressBar>
#include <QProcess>
#include <QQueue>
#include <QSet>
#include <QPushButton>
#include <QRadioButton>
#include <QRegularExpression>
#include <QResizeEvent>
#include <QScrollArea>
#include <QScrollBar>
#include <QSettings>
#include <QSaveFile>
#include <QSignalBlocker>
#include <QSlider>
#include <QSplitter>
#include <QStatusBar>
#include <QStyle>
#include <QTabWidget>
#include <QTextEdit>
#include <QTextCursor>
#include <QTextFrame>
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
#include <QSignalSpy>
#include <QTemporaryDir>
#include <QStandardPaths>
#endif

#include <algorithm>
#include <cmath>
#include <limits>
#include <memory>

namespace {

constexpr int kTimelineLabelWidth = 94;
constexpr int kTimelineTail = 30;
constexpr int kRulerHeight = 28;
constexpr int kTrackHeight = 52;
constexpr int kClipInset = 4;
// One pixel stays under a frame, so the pointer time cannot step in multi-second jumps.
constexpr double kMinPixelsPerSecond = 80.0;

// While the pointer is driving the viewer, keep a frame when it is closer to
// the target than the one already on screen. Equal distance still replaces so
// an exact decode can take over a keyframe labeled at the same time.
inline bool nearerScrubFrame(double shownTime, bool hasShown, double frameTime, double target, double fps) {
    if (!hasShown) return true;
    const double slack = 0.5 / std::max(1.0, fps);
    return std::abs(frameTime - target) <= std::abs(shownTime - target) + slack;
}

QString validateInboundMcpConfig(const QByteArray &raw) {
    QJsonParseError error;
    const QJsonDocument document = QJsonDocument::fromJson(raw, &error);
    if (error.error != QJsonParseError::NoError)
        return QStringLiteral("Invalid JSON at offset %1: %2").arg(error.offset).arg(error.errorString());
    if (!document.isObject()) return QStringLiteral("The JSON root must be an object.");
    const QJsonValue servers = document.object().value(QStringLiteral("mcpServers"));
    if (!servers.isObject()) return QStringLiteral("mcpServers must be an object.");
    const QJsonObject object = servers.toObject();
    for (auto it = object.begin(); it != object.end(); ++it) {
        if (!it.value().isObject()) return QStringLiteral("Each server must be an object.");
        const QJsonObject server = it.value().toObject();
        const QJsonValue command = server.value(QStringLiteral("command"));
        if (!command.isString() || command.toString().trimmed().isEmpty())
            return QStringLiteral("Each server needs a nonempty local command string. URL-only remote servers are not supported in Cut chat.");
        for (const QString &field : {QStringLiteral("args"), QStringLiteral("tools")}) {
            if (!server.contains(field) || (field == QStringLiteral("tools") && server.value(field).isNull())) continue;
            if (!server.value(field).isArray()) return QStringLiteral("%1 must be an array of strings.").arg(field);
            for (const QJsonValue &value : server.value(field).toArray())
                if (!value.isString()) return QStringLiteral("%1 must contain only strings.").arg(field);
        }
        if (server.contains(QStringLiteral("env"))) {
            if (!server.value(QStringLiteral("env")).isObject()) return QStringLiteral("env must be an object of string values.");
            const QJsonObject env = server.value(QStringLiteral("env")).toObject();
            for (auto entry = env.begin(); entry != env.end(); ++entry)
                if (!entry.value().isString()) return QStringLiteral("env must contain only string values.");
        }
    }
    return {};
}

QString mcpExecutable(const QString &projectDirectory) {
    const QString override = qEnvironmentVariable("CUT_MCP_BIN");
    if (!override.isEmpty()) {
        return QFileInfo(override).isRelative() && !override.contains(QLatin1Char('/'))
            && !override.contains(QLatin1Char('\\'))
            ? override : QFileInfo(override).absoluteFilePath();
    }
#ifdef Q_OS_WIN
    const QString binary = QStringLiteral("cut-mcp.exe");
#else
    const QString binary = QStringLiteral("cut-mcp");
#endif
    QStringList candidates{QDir(QCoreApplication::applicationDirPath()).filePath(binary)};
    const QStringList roots{QCoreApplication::applicationDirPath(), QDir::currentPath(),
                            QFileInfo(QString::fromUtf8(__FILE__)).absolutePath(), projectDirectory};
    for (const QString &root : roots) {
        QDir directory(root);
        do {
            if (directory.exists(QStringLiteral("crates/cut-mcp/Cargo.toml"))) {
                candidates << directory.filePath(QStringLiteral("target/release/") + binary)
                           << directory.filePath(QStringLiteral("target/debug/") + binary);
                break;
            }
        } while (directory.cdUp());
    }
    for (const QString &candidate : candidates) {
        const QFileInfo info(candidate);
        if (info.isFile() && info.isExecutable()) return info.absoluteFilePath();
    }
    return QStringLiteral("cut-mcp");
}

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

QString timelineTimecode(double seconds, double fps) {
    const double rate = std::max(1.0, fps);
    const int frameRate = std::max(1, static_cast<int>(std::lround(rate)));
    const long long rounded = std::llround(std::max(0.0, seconds) * rate);
    const int frames = static_cast<int>(rounded % frameRate);
    long long whole = rounded / frameRate;
    const int secs = static_cast<int>(whole % 60);
    whole /= 60;
    const int mins = static_cast<int>(whole % 60);
    const int hours = static_cast<int>(whole / 60);
    return QStringLiteral("%1:%2:%3:%4")
        .arg(hours, 2, 10, QLatin1Char('0'))
        .arg(mins, 2, 10, QLatin1Char('0'))
        .arg(secs, 2, 10, QLatin1Char('0'))
        .arg(frames, 2, 10, QLatin1Char('0'));
}

QString timelineRulerLabel(double seconds, double fps, double duration) {
    const QString full = timelineTimecode(seconds, fps);
    return duration >= 3600.0 ? full : full.mid(3);
}

struct RulerScale {
    double majorSeconds = 1.0;
    int divisions = 10;
};

RulerScale rulerScale(double pixelsPerSecond, double fps) {
    const double pps = std::max(pixelsPerSecond, 0.001);
    const double frame = 1.0 / std::max(1.0, fps);
    const double steps[] = {frame,      2 * frame, 5 * frame, 10 * frame, 15 * frame, 1,    2,   5,
                            10,         15,        30,        60,         120,        300, 600, 1800,
                            3600,       7200};
    double major = steps[std::size(steps) - 1];
    for (double step : steps) {
        if (step * pps >= 72.0) {
            major = step;
            break;
        }
    }
    int divisions = 1;
    if (major > frame * 1.01) {
        for (int candidate : {10, 5, 4, 2}) {
            if ((major / candidate) * pps >= 7.0) {
                divisions = candidate;
                break;
            }
        }
    }
    return {major, std::max(1, divisions)};
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

QJsonObject proposalCropEdit(const QJsonObject &proposal) {
    QQueue<QJsonObject> pending;
    pending.enqueue(proposal.value(QStringLiteral("command")).toObject());
    QJsonObject cropEdit;
    while (!pending.isEmpty()) {
        const QJsonObject command = pending.dequeue();
        if (command.value(QStringLiteral("type")).toString() == QStringLiteral("batch")) {
            for (const QJsonValue &child : command.value(QStringLiteral("commands")).toArray())
                pending.enqueue(child.toObject());
        } else if (command.value(QStringLiteral("type")).toString() == QStringLiteral("update_clip") &&
                   command.value(QStringLiteral("patch")).toObject().value(QStringLiteral("crop")).isObject()) {
            if (!cropEdit.isEmpty()) return {};
            cropEdit = command;
        }
    }
    return cropEdit;
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

// Cache source coordinates use device pixels while the paint target uses logical pixels.
void blitLaneCache(QPainter &painter, const QPixmap &cache, const QRegion &clip) {
    const qreal dpr = std::max<qreal>(1, cache.devicePixelRatio());
    painter.save();
    painter.setClipRegion(clip, Qt::IntersectClip);
    for (const QRect &logical : clip) {
        const QRectF source(logical.x() * dpr, logical.y() * dpr, logical.width() * dpr, logical.height() * dpr);
        painter.drawPixmap(QRectF(logical), cache, source);
    }
    painter.restore();
}

class TimelineWidget final : public QWidget {
    Q_OBJECT
public:
    explicit TimelineWidget(QWidget *parent = nullptr) : QWidget(parent) {
        setFocusPolicy(Qt::StrongFocus);
        setAttribute(Qt::WA_OpaquePaintEvent);
        setSizePolicy(QSizePolicy::Minimum, QSizePolicy::MinimumExpanding);
        updateContentSize();
        setMouseTracking(true);
    }

    void setProject(const QJsonObject &project, double duration, double fps) {
        m_project = project;
        m_duration = std::max(duration, 1.0);
        m_fps = fps > 0.0 ? fps : 24.0;
        invalidateLanes();
        updateContentSize();
        update();
    }

    void setSelectedClip(const QString &id) {
        if (m_selectedClip == id) return;
        m_selectedClip = id;
        invalidateLanes();
        update();
    }

    void setPlayhead(double time) {
        // The pointer already owns the line while scrubbing. A quantized seek
        // must not pull it back onto a frame boundary.
        if (m_scrubbing) return;
        const double previous = m_playhead;
        m_playhead = std::clamp(time, 0.0, m_duration);
        repaintAt(timeToX(previous));
        repaintAt(timeToX(m_playhead));
    }

    void showSkimmer(double time) {
        const double previous = m_skimmerTime;
        const bool had = m_hasSkimmer;
        const qreal previousX = m_feedbackPoint.x();
        m_skimmerTime = std::clamp(time, 0.0, m_duration);
        m_hasSkimmer = true;
        m_feedbackPoint = QPointF(timeToX(m_skimmerTime), kRulerHeight + kTrackHeight / 2.0);
        if (had) repaintAt(timeToX(previous));
        repaintAt(timeToX(m_skimmerTime));
        repaintAt(previousX);
        repaintAt(m_feedbackPoint.x());
    }

    double skimmerTime() const { return m_skimmerTime; }

    void setZoom(double zoom) {
        m_zoom = std::clamp(zoom, 0.35, 3.5);
        m_fitWidth = 0;
        invalidateLanes();
        updateContentSize();
        update();
    }

    void fitToWidth(int width) {
        m_fitWidth = std::max(kTimelineLabelWidth + 32, width);
        invalidateLanes();
        updateContentSize();
        update();
    }

    bool isFitToWidth() const { return m_fitWidth > 0; }
    void setArtwork(MediaArtwork *artwork) {
        m_artwork = artwork;
        connect(artwork, &MediaArtwork::changed, this, [this] {
            invalidateLanes();
            update();
        });
    }

    void clearSkimmer() {
        if (!m_hasSkimmer) return;
        const double previous = m_skimmerTime;
        m_hasSkimmer = false;
        m_hoverFrame = -1.0;
        repaintAt(timeToX(previous));
        repaintAt(timeToX(m_playhead));
        repaintAt(m_feedbackPoint.x());
    }

    QSize minimumSizeHint() const override { return minimumSize(); }

    bool addLaneVisible() const { return m_addLaneVisible; }
    QPoint addLaneButtonCenter() const { return addLaneButtonRect().center(); }
    QString skimLabel() const { return m_hasSkimmer ? prettyTime(m_skimmerTime) : QString(); }

signals:
    void seekRequested(double time);
    void hoverSeekRequested(double time);
    void hoverSeekEnded();
    void clipSelected(const QString &clipId);
    void moveRequested(const QString &clipId, double timelineStart);
    void trimRequested(const QString &clipId, bool leading, double timelineFrame);
    void addLaneRequested();

protected:
    void paintEvent(QPaintEvent *event) override {
        QPainter p(this);
        // Corner timecode and the playhead are separate strips. Paint each one
        // so a long timeline does not rebuild everything between them.
        if (lanesAreCacheable()) {
            const bool rebuilt = m_laneCache.isNull();
            ensureLaneCache();
            if (!m_laneCache.isNull()) {
                blitLaneCache(p, m_laneCache, event->region());
                p.setClipRegion(event->region());
                paintOverlay(p);
                // A resize can expose only the new edge. The rest would keep the
                // previous scale until the pointer moved across it.
                if (rebuilt && event->region() != QRegion(rect())) update();
                return;
            }
        }
        for (const QRect &visible : event->region())
            paintLanes(p, visible);
        p.setClipRegion(event->region());
        paintOverlay(p);
    }

    void resizeEvent(QResizeEvent *event) override {
        invalidateLanes();
        QWidget::resizeEvent(event);
        update();
    }

    void mousePressEvent(QMouseEvent *event) override {
        if (event->button() != Qt::LeftButton) return;
        setFocus();
        clearSkimmer();
        const QPoint pos = event->position().toPoint();
        if (addLaneHitRect().contains(pos)) {
            m_addLaneVisible = false;
            update();
            emit addLaneRequested();
            return;
        }
        m_addLaneVisible = false;
        const double time = timeAt(event->position().x());
        if (pos.y() < kRulerHeight || pos.x() < kTimelineLabelWidth) {
            beginScrub(event->position());
            return;
        }
        const auto hit = clipAt(pos);
        if (hit.id.isEmpty()) {
            beginScrub(event->position());
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
        if (m_scrubbing) {
            const double previous = m_playhead;
            m_feedbackPoint = event->position();
            m_playhead = std::clamp(timeAt(m_feedbackPoint.x()), 0.0, m_duration);
            repaintAt(timeToX(previous));
            repaintAt(timeToX(m_playhead));
            emit seekRequested(m_playhead);
            return;
        }
        if (!m_dragging) {
            const QPoint pos = event->position().toPoint();
            updateAddLane(pos);
            if (m_addLaneVisible && addLaneHitRect().contains(pos)) {
                if (!m_hoverClipId.isEmpty()) {
                    m_hoverClipId.clear();
                    invalidateLanes();
                }
                if (m_hasSkimmer) {
                    m_hasSkimmer = false;
                    m_hoverFrame = -1.0;
                    emit hoverSeekEnded();
                }
                setCursor(Qt::PointingHandCursor);
                update();
                return;
            }
            const ClipHit hit = clipAt(pos);
            if (hit.id != m_hoverClipId) {
                m_hoverClipId = hit.id;
                invalidateLanes();
                update();
            }
            if (hit.id.isEmpty()) {
                setCursor(Qt::ArrowCursor);
            } else if (event->position().x() - hit.rect.left() < 8 || hit.rect.right() - event->position().x() < 8) {
                setCursor(Qt::SizeHorCursor);
            } else {
                setCursor(Qt::OpenHandCursor);
            }
            updateSkimmer(event->position());
            return;
        }
        clearSkimmer();
        const double delta = timeAt(event->position().x()) - m_pressTime;
        m_previewTime = std::round(std::max(0.0, (m_dragTrailing ? m_dragStart + m_dragDuration : m_dragStart) + delta) * m_fps) / m_fps;
        setCursor((m_dragLeading || m_dragTrailing) ? Qt::SizeHorCursor : Qt::ClosedHandCursor);
        invalidateLanes();
        update();
    }

    void mouseReleaseEvent(QMouseEvent *event) override {
        if (m_scrubbing) {
            m_scrubbing = false;
            releaseMouse();
            const double previous = m_playhead;
            m_feedbackPoint = event->position();
            m_playhead = std::clamp(timeAt(m_feedbackPoint.x()), 0.0, m_duration);
            repaintAt(timeToX(previous));
            repaintAt(timeToX(m_playhead));
            emit seekRequested(m_playhead);
            return;
        }
        if (!m_dragging) return;
        m_dragging = false;
        const double delta = timeAt(event->position().x()) - m_pressTime;
        if (std::abs(delta * pixelsPerSecond()) < QApplication::startDragDistance()) {
            m_dragClipId.clear();
            invalidateLanes();
            update();
            return;
        }
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
        invalidateLanes();
        update();
    }

    void leaveEvent(QEvent *) override {
        if (m_scrubbing || m_dragging) return;
        {
            if (!m_hoverClipId.isEmpty()) invalidateLanes();
            m_hoverClipId.clear();
            m_addLaneVisible = false;
            m_hoverFrame = -1.0;
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

    void paintLanes(QPainter &p, const QRect &visible) {
        const int trackCount = static_cast<int>(m_project.value(QStringLiteral("tracks")).toArray().size());
        const double pxPerSecond = pixelsPerSecond();
        p.setRenderHint(QPainter::Antialiasing, false);
        p.fillRect(visible, QColor("#141414"));
        p.fillRect(0, kRulerHeight, kTimelineLabelWidth, std::max(0, height() - kRulerHeight), QColor("#1c1c1c"));
        for (int y = kRulerHeight, row = 0; y < height(); y += kTrackHeight, ++row) {
            const int laneHeight = std::min(kTrackHeight, height() - y);
            if (laneHeight <= 0) break;
            p.fillRect(kTimelineLabelWidth, y, std::max(0, width() - kTimelineLabelWidth), laneHeight,
                       QColor(row % 2 == 0 ? "#1a1a1a" : "#161616"));
            if (y + kTrackHeight <= height()) {
                p.setPen(QColor("#101010"));
                p.drawLine(0, y + kTrackHeight - 1, width(), y + kTrackHeight - 1);
            }
        }

        QFont laneFont = p.font();
        laneFont.setPixelSize(11);
        p.setFont(laneFont);

        const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
        for (int row = 0; row < trackCount; ++row) {
            const int y = kRulerHeight + row * kTrackHeight;
            if (y + kTrackHeight < visible.top() || y > visible.bottom()) continue;
            const QJsonObject track = tracks.at(row).toObject();
            const QColor accent = trackAccent(track, row);
            const QString type = track.value(QStringLiteral("type")).toString().toLower();
            const bool audio = type.contains(QStringLiteral("audio")) || type.contains(QStringLiteral("sound"));
            const QJsonArray clips = track.value(QStringLiteral("clips")).toArray();
            if (m_artwork && !clips.isEmpty()) {
                const QString sourceId = clips.first().toObject().value(QStringLiteral("sourceId")).toString();
                const QImage image = audio ? m_artwork->waves.value(sourceId) : m_artwork->frames.value(sourceId);
                if (!image.isNull()) {
                    const QRect thumbnail(8, y + 4, 78, kTrackHeight - 8);
                    p.drawImage(thumbnail, image);
                }
            }

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
                QRectF clipRect(kTimelineLabelWidth + start * pxPerSecond + 2, y + kClipInset,
                                std::max(12.0, duration * pxPerSecond - 4), kTrackHeight - kClipInset * 2);
                if (!clipRect.intersects(visible)) continue;
                const bool selected = id == m_selectedClip;
                const bool hovered = id == m_hoverClipId;
                QColor body = accent.darker(selected ? 145 : hovered ? 160 : 185);
                if (body.value() < 28) body = QColor("#243044");
                const qreal titleBand = std::min(16.0, std::max(10.0, clipRect.height() * 0.34));
                p.save();
                p.setClipRect(visible);
                QPainterPath clipPath;
                clipPath.addRoundedRect(clipRect, 2, 2);
                p.setClipPath(clipPath, Qt::IntersectClip);
                p.fillRect(clipRect, body);
                p.fillRect(QRectF(clipRect.left(), clipRect.top(), clipRect.width(), titleBand), accent);
                if (audio) {
                    const QString sourceId = clip.value("sourceId").toString();
                    if (m_artwork && m_artwork->waves.contains(sourceId)) {
                        const auto wave = m_artwork->waves.value(sourceId);
                        double sourceDuration = 1;
                        for (const auto &source : m_project.value("sources").toArray())
                            if (source.toObject().value("id").toString() == sourceId)
                                sourceDuration = std::max(0.01, source.toObject().value("duration").toDouble(1));
                        const double in = clip.value("in").toDouble();
                        const double out = clip.value("out").toDouble(sourceDuration);
                        const QRectF film(clipRect.left(), clipRect.top() + titleBand, clipRect.width(),
                                          std::max(1.0, clipRect.height() - titleBand));
                        p.setOpacity(0.9);
                        p.drawImage(film, wave,
                                    QRectF(in / sourceDuration * wave.width(), 0,
                                           (out - in) / sourceDuration * wave.width(), wave.height()));
                        p.setOpacity(1);
                    }
                } else {
                    const QImage frame = m_artwork ? m_artwork->frames.value(clip.value("sourceId").toString()) : QImage();
                    if (!frame.isNull()) {
                        const double tileHeight = std::max(1.0, clipRect.height() - titleBand);
                        const double tileWidth = tileHeight * 16.0 / 9.0;
                        const double firstTile = clipRect.left() + std::max(0.0, std::floor((visible.left() - clipRect.left()) / tileWidth)) * tileWidth;
                        for (double x = firstTile; x < std::min(clipRect.right(), double(visible.right())); x += tileWidth)
                            p.drawImage(QRectF(x, clipRect.top() + titleBand, tileWidth, tileHeight), frame);
                    }
                }
                if (clipRect.width() > 28.0) {
                    QFont nameFont = p.font();
                    nameFont.setPixelSize(11);
                    p.setFont(nameFont);
                    const QFontMetrics metrics(nameFont);
                    const QString label = metrics.elidedText(clipName(clip), Qt::ElideRight,
                                                              std::max(1, static_cast<int>(clipRect.width()) - 12));
                    p.setPen(QColor("#f7f7f7"));
                    p.drawText(QRectF(clipRect.left() + 6, clipRect.top(), clipRect.width() - 10, titleBand),
                               Qt::AlignVCenter | Qt::TextSingleLine, label);
                }
                p.restore();
                p.setRenderHint(QPainter::Antialiasing, true);
                p.setBrush(Qt::NoBrush);
                p.setPen(QPen(selected ? QColor("#f0b429") : QColor(0, 0, 0, 170), selected ? 2 : 1));
                p.drawRoundedRect(clipRect, 2, 2);
                if (selected) {
                    p.setPen(QPen(QColor("#f0b429"), 2, Qt::SolidLine, Qt::RoundCap));
                    p.drawLine(QPointF(clipRect.left() + 3, clipRect.center().y() - 6), QPointF(clipRect.left() + 3, clipRect.center().y() + 6));
                    p.drawLine(QPointF(clipRect.right() - 3, clipRect.center().y() - 6), QPointF(clipRect.right() - 3, clipRect.center().y() + 6));
                }
                p.setRenderHint(QPainter::Antialiasing, false);
            }
        }

        p.setPen(QColor("#0c0c0c"));
        p.drawLine(kTimelineLabelWidth, kRulerHeight, kTimelineLabelWidth, height());
        paintRuler(p, visible, pxPerSecond);
    }

    void paintRuler(QPainter &p, const QRect &visible, double pps) {
        p.setRenderHint(QPainter::Antialiasing, false);
        p.fillRect(QRect(visible.left(), 0, visible.width(), kRulerHeight), QColor("#2a2a2a"));
        p.fillRect(0, 0, kTimelineLabelWidth, kRulerHeight, QColor("#232323"));
        p.setPen(QColor("#101010"));
        p.drawLine(visible.left(), kRulerHeight - 1, visible.right(), kRulerHeight - 1);
        p.drawLine(kTimelineLabelWidth, 0, kTimelineLabelWidth, kRulerHeight - 1);

        QFont font(QStringLiteral("Menlo"));
        font.setPixelSize(9);
        font.setStyleHint(QFont::Monospace);
        p.setFont(font);
        const RulerScale scale = rulerScale(pps, m_fps);
        const double major = std::max(scale.majorSeconds, 1e-6);
        const double minor = major / scale.divisions;
        const double startTime = (visible.left() - kTimelineLabelWidth) / pps;
        const double endTime = (visible.right() - kTimelineLabelWidth) / pps;
        int first = std::max(0, static_cast<int>(std::floor(startTime / major)) - 1);
        const int limit = static_cast<int>(std::ceil(std::max(m_duration, 0.0) / major)) + 1;
        const int last = std::min(limit, static_cast<int>(std::ceil(endTime / major)) + 1);
        for (int index = first; index <= last; ++index) {
            const double time = index * major;
            const qreal x = timeToX(time);
            p.setPen(QColor("#8e8e8e"));
            p.drawLine(QPointF(x, kRulerHeight - 10), QPointF(x, kRulerHeight - 1));
            if (x >= kTimelineLabelWidth - 1.0 && time <= m_duration + 0.0001) {
                p.setPen(QColor("#b5b5b5"));
                p.drawText(QRectF(x + 4, 1, std::max(12.0, major * pps - 8.0), 14),
                           Qt::AlignLeft | Qt::AlignVCenter,
                           timelineRulerLabel(std::min(time, m_duration), m_fps, m_duration));
            }
            for (int step = 1; step < scale.divisions; ++step) {
                const double tick = time + minor * step;
                if (tick > m_duration) break;
                const qreal mx = timeToX(tick);
                p.setPen(QColor("#5c5c5c"));
                p.drawLine(QPointF(mx, kRulerHeight - 5), QPointF(mx, kRulerHeight - 1));
            }
        }
    }

    void paintOverlay(QPainter &p) {
        const auto drawHead = [&](double time, const QColor &color) {
            const qreal x = std::floor(timeToX(time)) + 0.5;
            p.setRenderHint(QPainter::Antialiasing, false);
            p.setPen(QPen(color, 1));
            p.drawLine(QPointF(x, kRulerHeight), QPointF(x, height()));
            p.setRenderHint(QPainter::Antialiasing, true);
            p.setPen(Qt::NoPen);
            p.setBrush(color);
            QPainterPath head;
            head.moveTo(x - 6.0, kRulerHeight - 11.0);
            head.lineTo(x + 6.0, kRulerHeight - 11.0);
            head.lineTo(x, kRulerHeight);
            head.closeSubpath();
            p.drawPath(head);
            p.setRenderHint(QPainter::Antialiasing, false);
        };
        if (m_hasSkimmer && !m_dragging && !m_scrubbing) {
            drawHead(m_playhead, QColor("#6a6a6a"));
            const qreal skimX = std::floor(timeToX(m_skimmerTime)) + 0.5;
            p.setRenderHint(QPainter::Antialiasing, false);
            p.setPen(QPen(QColor("#f3f3f3"), 1));
            p.drawLine(QPointF(skimX, kRulerHeight), QPointF(skimX, height()));
            paintCornerTime(p, m_skimmerTime, QColor("#f4f4f4"));
        } else {
            drawHead(m_playhead, QColor("#f0b429"));
            paintCornerTime(p, m_playhead, QColor("#f0b429"));
        }
        if (m_addLaneVisible) {
            const QRect button = addLaneButtonRect();
            p.setRenderHint(QPainter::Antialiasing, true);
            p.setPen(QPen(QColor("#5c5c5c"), 1));
            p.setBrush(QColor("#2c2c2c"));
            p.drawRoundedRect(button, 3, 3);
            p.setPen(QColor("#f2f2f2"));
            QFont addFont = p.font();
            addFont.setPixelSize(16);
            addFont.setWeight(QFont::Medium);
            p.setFont(addFont);
            p.drawText(button, Qt::AlignCenter, QStringLiteral("+"));
            p.setRenderHint(QPainter::Antialiasing, false);
        }
    }

    void paintCornerTime(QPainter &p, double time, const QColor &color) const {
        const QRect corner(0, 0, kTimelineLabelWidth, kRulerHeight - 1);
        p.setRenderHint(QPainter::Antialiasing, false);
        p.fillRect(corner, QColor("#232323"));
        QFont font(QStringLiteral("Menlo"));
        font.setPixelSize(11);
        font.setStyleHint(QFont::Monospace);
        p.setFont(font);
        p.setPen(color);
        p.drawText(corner.adjusted(4, 0, -4, 0), Qt::AlignCenter, timelineRulerLabel(time, m_fps, m_duration));
    }

    void repaintAt(qreal x) {
        update(pointerBand(x));
        update(QRect(0, 0, kTimelineLabelWidth, kRulerHeight));
    }

    void ensureLaneCache() {
        const qreal dpr = std::max<qreal>(1, devicePixelRatioF());
        if (!m_laneCache.isNull()) {
            const qreal cachedW = m_laneCache.width() / m_laneCache.devicePixelRatio();
            const qreal cachedH = m_laneCache.height() / m_laneCache.devicePixelRatio();
            if (qFuzzyCompare(m_laneCache.devicePixelRatio(), dpr) && qFuzzyCompare(cachedW, qreal(width())) &&
                qFuzzyCompare(cachedH, qreal(height())))
                return;
        }
        const int physicalW = std::max(1, static_cast<int>(std::ceil(width() * dpr)));
        const int physicalH = std::max(1, static_cast<int>(std::ceil(height() * dpr)));
        QPixmap cache(physicalW, physicalH);
        if (cache.isNull()) {
            m_laneCache = QPixmap();
            return;
        }
        cache.setDevicePixelRatio(dpr);
        cache.fill(QColor("#141414"));
        QPainter painter(&cache);
        if (!painter.isActive()) {
            m_laneCache = QPixmap();
            return;
        }
        paintLanes(painter, QRect(0, 0, width(), height()));
        m_laneCache = cache;
    }

    bool lanesAreCacheable() const {
        if (width() <= 0 || height() <= 0) return false;
        const double dpr = std::max(1.0, devicePixelRatioF());
        return double(width()) * double(height()) * dpr * dpr <= 6000000.0;
    }

    void invalidateLanes() { m_laneCache = QPixmap(); }

    qreal timeToX(double time) const {
        return kTimelineLabelWidth + time * pixelsPerSecond();
    }

    QRect pointerBand(qreal x) const {
        const int ix = static_cast<int>(std::lround(x));
        return QRect(ix - 120, 0, 240, std::max(1, height()));
    }

    void updateContentSize() {
        const int tracks = static_cast<int>(m_project.value(QStringLiteral("tracks")).toArray().size());
        const double contentWidth = kTimelineLabelWidth + m_duration * pixelsPerSecond() + kTimelineTail;
        setMinimumSize(static_cast<int>(std::min(contentWidth, 16777215.0)),
                       kRulerHeight + std::max(3, tracks) * kTrackHeight + 8);
        updateGeometry();
    }

    double pixelsPerSecond() const {
        if (m_fitWidth <= 0) return 72.0 * m_zoom;
        const double fitted = (m_fitWidth - kTimelineLabelWidth - kTimelineTail) / std::max(m_duration, 0.001);
        return std::max(fitted, kMinPixelsPerSecond);
    }

    double timeAt(qreal x) const {
        return std::clamp((x - kTimelineLabelWidth) / pixelsPerSecond(), 0.0, m_duration);
    }

    void beginScrub(const QPointF &point) {
        const double previous = m_playhead;
        m_scrubbing = true;
        grabMouse();
        m_feedbackPoint = point;
        m_playhead = std::clamp(timeAt(point.x()), 0.0, m_duration);
        repaintAt(timeToX(previous));
        repaintAt(timeToX(m_playhead));
        emit seekRequested(m_playhead);
    }

    void updateSkimmer(const QPointF &point) {
        if (point.x() < kTimelineLabelWidth) {
            if (m_hasSkimmer) {
                const double previous = m_skimmerTime;
                const qreal previousX = m_feedbackPoint.x();
                m_hasSkimmer = false;
                m_hoverFrame = -1.0;
                emit hoverSeekEnded();
                repaintAt(timeToX(previous));
                repaintAt(timeToX(m_playhead));
                repaintAt(previousX);
            }
            return;
        }
        const qreal previousX = m_feedbackPoint.x();
        const double previous = m_skimmerTime;
        const bool had = m_hasSkimmer;
        m_feedbackPoint = point;
        const double exact = std::clamp(timeAt(point.x()), 0.0, m_duration);
        const double frameTime = std::clamp(std::round(exact * m_fps) / m_fps, 0.0, m_duration);
        const bool frameChanged = !had || std::abs(m_hoverFrame - frameTime) >= (0.5 / m_fps);
        m_skimmerTime = exact;
        m_hasSkimmer = true;
        if (had) repaintAt(timeToX(previous));
        repaintAt(timeToX(exact));
        repaintAt(timeToX(m_playhead));
        repaintAt(previousX);
        if (frameChanged) {
            m_hoverFrame = frameTime;
            emit hoverSeekRequested(frameTime);
        }
    }

    QRect addLaneButtonRect() const {
        const int row = m_project.value(QStringLiteral("tracks")).toArray().size();
        return QRect((kTimelineLabelWidth - 22) / 2,
                     kRulerHeight + row * kTrackHeight + (kTrackHeight - 22) / 2, 22, 22);
    }

    QRect addLaneHitRect() const {
        const int row = m_project.value(QStringLiteral("tracks")).toArray().size();
        return QRect(0, kRulerHeight + row * kTrackHeight, kTimelineLabelWidth, kTrackHeight);
    }

    void updateAddLane(const QPoint &point) {
        const bool visible = addLaneHitRect().contains(point);
        if (m_addLaneVisible != visible) {
            m_addLaneVisible = visible;
            update(addLaneHitRect());
        }
        if (visible) setCursor(Qt::PointingHandCursor);
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
                QRectF rect(kTimelineLabelWidth + start * pixelsPerSecond() + 2, y + kClipInset,
                            std::max(12.0, duration * pixelsPerSecond() - 4), kTrackHeight - kClipInset * 2);
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
    double m_hoverFrame = -1.0;
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
    bool m_addLaneVisible = false;
    QPointF m_feedbackPoint;
    QPixmap m_laneCache;
};

class ViewerWidget final : public QWidget {
    Q_OBJECT
public:
    explicit ViewerWidget(QWidget *parent = nullptr) : QWidget(parent) {
        setMinimumSize(320, 180);
        setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Expanding);
        setMouseTracking(true);
    }

    void setFrame(const QImage &frame, double time) {
        m_frame = frame;
        m_time = time;
        m_scaled = QImage();
        update();
    }

    QImage frame() const { return m_frame; }

    void clearFrame() {
        m_frame = QImage();
        m_scaled = QImage();
        update();
    }

    void setLiveScrub(bool live) {
        if (m_liveScrub == live) return;
        m_liveScrub = live;
        m_scaled = QImage();
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
        const QRectF target = QRectF(rect()).adjusted(8, 8, -8, -8);
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

signals:
    void scrubbed(double fraction);
    void scrubEnded();
    void seekRequested(double fraction);

protected:
    void mousePressEvent(QMouseEvent *event) override {
        if (event->button() != Qt::LeftButton) return;
        emit seekRequested(fractionAt(event->position().x()));
    }

    void mouseMoveEvent(QMouseEvent *event) override {
        const double fraction = fractionAt(event->position().x());
        if (m_skimming && std::abs(fraction - m_skimFraction) < 0.0005) return;
        m_skimming = true;
        m_skimFraction = fraction;
        emit scrubbed(fraction);
    }

    void leaveEvent(QEvent *) override {
        if (!m_skimming) return;
        m_skimming = false;
        emit scrubEnded();
    }

    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing);
        p.fillRect(rect(), QColor("#141414"));
        const QRectF canvas = videoRect();
        QPainterPath canvasPath;
        canvasPath.addRoundedRect(canvas.adjusted(0.5, 0.5, -0.5, -0.5), 2, 2);
        p.setPen(QPen(QColor("#050505"), 1));
        p.setBrush(QColor("#050505"));
        p.drawPath(canvasPath);
        if (!m_frame.isNull()) {
            p.save();
            p.setClipPath(canvasPath);
            const qreal dpr = m_liveScrub ? 1.0 : std::max(1.0, devicePixelRatioF());
            const QSize target(std::max(1, static_cast<int>(std::round(canvas.width() * dpr))),
                               std::max(1, static_cast<int>(std::round(canvas.height() * dpr))));
            if (m_scaled.isNull() || m_scaled.size() != target || m_scaledFast != m_liveScrub) {
                m_scaled = m_frame.scaled(target, Qt::KeepAspectRatio,
                                          m_liveScrub ? Qt::FastTransformation : Qt::SmoothTransformation);
                m_scaled.setDevicePixelRatio(dpr);
                m_scaledFast = m_liveScrub;
            }
            const QPointF topLeft(canvas.center().x() - m_scaled.width() / dpr / 2.0,
                                  canvas.center().y() - m_scaled.height() / dpr / 2.0);
            p.drawImage(topLeft, m_scaled);
            p.restore();
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
                       QStringLiteral("The playhead frame shows here."));
        }
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
    double fractionAt(int x) const {
        return std::clamp(static_cast<double>(x) / std::max(1, width()), 0.0, 1.0);
    }

    QImage m_frame;
    QImage m_scaled;
    double m_time = 0.0;
    double m_width = 0.0;
    double m_height = 0.0;
    double m_skimFraction = -1.0;
    bool m_skimming = false;
    bool m_liveScrub = false;
    bool m_scaledFast = false;
    QString m_caption;
};

class RegionPicker final : public QWidget {
public:
    explicit RegionPicker(const QImage &frame, const QRectF &region, QWidget *parent = nullptr)
        : QWidget(parent), m_frame(frame), m_region(region) {
        setMinimumSize(480, 270);
        setMouseTracking(true);
    }

    QRectF region() const { return m_region; }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter painter(this);
        painter.fillRect(rect(), QColor("#141414"));
        const QRectF image = imageRect();
        painter.drawImage(image, m_frame);
        if (m_region.isEmpty()) return;
        const QRectF selected(image.x() + m_region.x() * image.width(),
                              image.y() + m_region.y() * image.height(),
                              m_region.width() * image.width(), m_region.height() * image.height());
        painter.setPen(QPen(QColor("#f2bd69"), 2));
        painter.setBrush(QColor(242, 189, 105, 35));
        painter.drawRect(selected);
    }

    void mousePressEvent(QMouseEvent *event) override {
        if (event->button() != Qt::LeftButton || !imageRect().contains(event->position())) return;
        m_start = normalizedPoint(event->position());
        m_dragging = true;
        m_region = QRectF();
        update();
    }

    void mouseMoveEvent(QMouseEvent *event) override {
        if (!m_dragging) return;
        const QPointF end = normalizedPoint(event->position());
        m_region = QRectF(m_start, end).normalized();
        update();
    }

    void mouseReleaseEvent(QMouseEvent *event) override {
        if (event->button() != Qt::LeftButton || !m_dragging) return;
        m_dragging = false;
        const QPointF end = normalizedPoint(event->position());
        m_region = QRectF(m_start, end).normalized();
        if (m_region.width() < 0.01 || m_region.height() < 0.01) m_region = QRectF();
        update();
    }

private:
    QRectF imageRect() const {
        const QSizeF fitted = m_frame.size().scaled(size(), Qt::KeepAspectRatio);
        return QRectF((width() - fitted.width()) / 2.0, (height() - fitted.height()) / 2.0,
                      fitted.width(), fitted.height());
    }

    QPointF normalizedPoint(const QPointF &point) const {
        const QRectF image = imageRect();
        return QPointF(std::clamp((point.x() - image.x()) / image.width(), 0.0, 1.0),
                       std::clamp((point.y() - image.y()) / image.height(), 0.0, 1.0));
    }

    QImage m_frame;
    QRectF m_region;
    QPointF m_start;
    bool m_dragging = false;
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
        m_exactSeekTimer.setSingleShot(true);
        connect(&m_exactSeekTimer, &QTimer::timeout, this, &MainWindow::settlePointerSeek);
        setConnection(QStringLiteral("connecting"));
#ifndef CUT_QT_TESTING
        m_bridge = cut_bridge_new(m_projectDirectory.toUtf8().constData(), m_cutdBinary.toUtf8().constData());
        if (!m_bridge) {
            setConnection(QStringLiteral("disconnected"), QStringLiteral("Could not start the runtime connection."));
        }
        m_pollTimer.setInterval(8);
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
                const double target = m_hoverSkimming ? m_pendingHoverSeek : m_playhead;
                if ((m_hoverSkimming || m_pointerOwnsPlayhead) &&
                    !nearerScrubFrame(m_shownFrameTime, m_hasShownFrame, frame.time, target, m_fps)) {
                    return;
                }
                QImage image(frame.pixels, static_cast<int>(frame.width), static_cast<int>(frame.height),
                             static_cast<int>(frame.width * 4), QImage::Format_RGBA8888);
                m_shownFrameTime = frame.time;
                m_hasShownFrame = true;
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
        QMenu menu(this);
        auto *video = menu.addAction(QStringLiteral("Add video track"));
        auto *audio = menu.addAction(QStringLiteral("Add audio track"));
        const QAction *choice = menu.exec(m_timeline->mapToGlobal(m_timeline->addLaneButtonCenter()));
        if (choice != video && choice != audio) return;
        const QString type = choice == audio ? QStringLiteral("audio") : QStringLiteral("video");
        int count = 0;
        for (const QJsonValue &value : m_project.value(QStringLiteral("tracks")).toArray())
            if (value.toObject().value(QStringLiteral("type")).toString() == type) ++count;
        const QString id = QStringLiteral("%1-%2").arg(type).arg(QDateTime::currentMSecsSinceEpoch());
        const QString name = choice == audio ? QStringLiteral("Audio %1").arg(count + 1)
                                              : QStringLiteral("Video %1").arg(count + 1);
        const QJsonObject track{{QStringLiteral("id"), id}, {QStringLiteral("type"), type},
                                {QStringLiteral("name"), name}, {QStringLiteral("clips"), QJsonArray{}}};
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
        m_pendingSeekScrub = scrubFromShownFrame(m_pendingSeek);
        m_hasPendingSeek = true;
        m_pointerOwnsPlayhead = true;
        m_playhead = m_pendingSeek;
        m_timeline->setPlayhead(m_playhead);
        m_viewer->setLiveScrub(true);
        if (m_pendingSeekScrub) m_exactSeekTimer.start(50);
        else m_exactSeekTimer.stop();
        flushSeek();
    }

    void queueHoverSeek(double time) {
        if (!m_connected || m_previewRequired || m_playing || m_duration <= 0.0) return;
        m_pendingHoverSeek = std::clamp(time, 0.0, m_duration);
        m_pendingHoverScrub = scrubFromShownFrame(m_pendingHoverSeek);
        m_hasPendingHoverSeek = true;
        m_hoverSkimming = true;
        m_pointerOwnsPlayhead = true;
        m_viewer->setLiveScrub(true);
        if (m_pendingHoverScrub) m_exactSeekTimer.start(50);
        else m_exactSeekTimer.stop();
        flushSeek();
    }

    void endHoverSeek() {
        const bool wasSkimming = m_hoverSkimming || m_hasPendingHoverSeek;
        m_hasPendingHoverSeek = false;
        m_hoverSkimming = false;
        m_exactSeekTimer.stop();
        if (!wasSkimming || !m_connected || m_playing) {
            m_pointerOwnsPlayhead = false;
            m_viewer->setLiveScrub(false);
            return;
        }
        // Keep the pointer in charge until the edit playhead's frame arrives,
        // so a late skim picture cannot jump back onto the viewer.
        m_pointerOwnsPlayhead = true;
        m_viewer->setLiveScrub(true);
        m_pendingSeek = m_playhead;
        m_pendingSeekScrub = false;
        m_hasPendingSeek = true;
        m_exactSeekTimer.start(50);
        flushSeek();
    }

    void settlePointerSeek() {
        if (m_hoverSkimming) {
            m_pendingHoverSeek = m_timeline->skimmerTime();
            m_pendingHoverScrub = false;
            m_hasPendingHoverSeek = true;
            flushSeek();
            return;
        }
        if (!m_pointerOwnsPlayhead) return;
        m_pendingSeek = m_playhead;
        m_pendingSeekScrub = false;
        m_hasPendingSeek = true;
        m_pointerOwnsPlayhead = false;
        m_viewer->setLiveScrub(false);
        flushSeek();
    }

    bool scrubFromShownFrame(double time) const {
        if (!m_hasShownFrame) return true;
        const double delta = time - m_shownFrameTime;
        return delta < -(1.0 / std::max(1.0, m_fps)) || delta > 0.40;
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

    QImage sourceFrame(const QJsonObject &clip, double timelineTime) const {
        const QString sourceId = clip.value(QStringLiteral("sourceId")).toString();
        QString path;
        for (const QJsonValue &value : m_project.value(QStringLiteral("sources")).toArray()) {
            const QJsonObject source = value.toObject();
            if (source.value(QStringLiteral("id")).toString() == sourceId) {
                path = QDir(m_projectDirectory).absoluteFilePath(source.value(QStringLiteral("path")).toString());
                break;
            }
        }
        if (path.isEmpty()) return {};
        const double clipStart = clip.value(QStringLiteral("timelineStart")).toDouble();
        const double sourceTime = clip.value(QStringLiteral("in")).toDouble() +
                                  (timelineTime - clipStart) * clip.value(QStringLiteral("speed")).toDouble(1.0);
        QProcess process;
        process.start(QStringLiteral("ffmpeg"), {QStringLiteral("-nostdin"), QStringLiteral("-v"), QStringLiteral("error"),
                                                 QStringLiteral("-ss"), QString::number(std::max(0.0, sourceTime), 'f', 6),
                                                 QStringLiteral("-i"), path, QStringLiteral("-frames:v"), QStringLiteral("1"),
                                                 QStringLiteral("-vf"), QStringLiteral("scale=960:-2"),
                                                 QStringLiteral("-f"), QStringLiteral("image2pipe"), QStringLiteral("-vcodec"),
                                                 QStringLiteral("png"), QStringLiteral("pipe:1")});
        if (!process.waitForStarted(3000) || !process.waitForFinished(15000)) {
            process.kill();
            process.waitForFinished();
            return {};
        }
        return process.exitStatus() == QProcess::NormalExit && process.exitCode() == 0
                   ? QImage::fromData(process.readAllStandardOutput(), "PNG") : QImage();
    }

    double firstSceneCut(const QJsonObject &clip, double start, double end) const {
        QString path;
        for (const QJsonValue &value : m_project.value(QStringLiteral("sources")).toArray()) {
            const QJsonObject source = value.toObject();
            if (source.value(QStringLiteral("id")).toString() == clip.value(QStringLiteral("sourceId")).toString()) {
                path = QDir(m_projectDirectory).absoluteFilePath(source.value(QStringLiteral("path")).toString());
                break;
            }
        }
        if (path.isEmpty()) return std::numeric_limits<double>::quiet_NaN();
        const double speed = clip.value(QStringLiteral("speed")).toDouble(1.0);
        const double sourceStart = clip.value(QStringLiteral("in")).toDouble() +
                                   (start - clip.value(QStringLiteral("timelineStart")).toDouble()) * speed;
        QProcess process;
        process.start(QStringLiteral("ffmpeg"), {QStringLiteral("-nostdin"), QStringLiteral("-hide_banner"),
                                                 QStringLiteral("-nostats"), QStringLiteral("-ss"),
                                                 QString::number(sourceStart, 'f', 6), QStringLiteral("-t"),
                                                 QString::number((end - start) * speed, 'f', 6),
                                                 QStringLiteral("-i"), path, QStringLiteral("-vf"),
                                                 QStringLiteral("select='gt(scene,0.08)',metadata=print"),
                                                 QStringLiteral("-an"), QStringLiteral("-f"), QStringLiteral("null"),
                                                 QStringLiteral("-")});
        if (!process.waitForStarted(3000) || !process.waitForFinished(30000)) {
            process.kill();
            process.waitForFinished();
            return std::numeric_limits<double>::quiet_NaN();
        }
        if (process.exitStatus() != QProcess::NormalExit || process.exitCode() != 0)
            return std::numeric_limits<double>::quiet_NaN();
        const QString output = QString::fromUtf8(process.readAllStandardError());
        const QRegularExpression times(QStringLiteral(R"(pts_time:([0-9]+(?:\.[0-9]+)?))"));
        auto match = times.globalMatch(output);
        while (match.hasNext()) {
            const double cut = start + match.next().captured(1).toDouble() / speed;
            if (cut > start + 1.0 / m_fps && cut < end - 1.0 / m_fps) return cut;
        }
        return -1.0;
    }

    QVector<QPair<qint64, QRectF>> trackRegion(const QJsonObject &clip, double start, double end,
                                                const QRectF &seed, double *reliableEnd) const {
        constexpr int width = 160;
        constexpr int height = 90;
        constexpr int samplesPerSecond = 2;
        QVector<QPair<qint64, QRectF>> boxes;
        QString path;
        for (const QJsonValue &value : m_project.value(QStringLiteral("sources")).toArray()) {
            const QJsonObject source = value.toObject();
            if (source.value(QStringLiteral("id")).toString() == clip.value(QStringLiteral("sourceId")).toString()) {
                path = QDir(m_projectDirectory).absoluteFilePath(source.value(QStringLiteral("path")).toString());
                break;
            }
        }
        if (path.isEmpty() || end - start > 120.0) return boxes;
        const double speed = clip.value(QStringLiteral("speed")).toDouble(1.0);
        const double sourceStart = clip.value(QStringLiteral("in")).toDouble() +
                                   (start - clip.value(QStringLiteral("timelineStart")).toDouble()) * speed;
        QProcess process;
        process.start(QStringLiteral("ffmpeg"), {QStringLiteral("-nostdin"), QStringLiteral("-v"), QStringLiteral("error"),
                                                 QStringLiteral("-ss"), QString::number(sourceStart, 'f', 6),
                                                 QStringLiteral("-t"), QString::number((end - start) * speed, 'f', 6),
                                                 QStringLiteral("-i"), path, QStringLiteral("-vf"),
                                                 QStringLiteral("fps=%1,scale=160:90,format=gray").arg(samplesPerSecond * speed),
                                                 QStringLiteral("-f"), QStringLiteral("rawvideo"), QStringLiteral("-pix_fmt"),
                                                 QStringLiteral("gray"), QStringLiteral("pipe:1")});
        if (!process.waitForStarted(3000) || !process.waitForFinished(30000)) {
            process.kill();
            process.waitForFinished();
            return boxes;
        }
        const QByteArray frames = process.readAllStandardOutput();
        constexpr qsizetype frameBytes = width * height;
        const qsizetype count = frames.size() / frameBytes;
        if (process.exitCode() != 0 || count < 2) return boxes;
        const int boxWidth = std::clamp(static_cast<int>(std::round(seed.width() * width)), 1, width);
        const int boxHeight = std::clamp(static_cast<int>(std::round(seed.height() * height)), 1, height);
        if (boxWidth < 6 || boxHeight < 6) return boxes;
        int x = std::clamp(static_cast<int>(std::round(seed.x() * width)), 0, width - boxWidth);
        int y = std::clamp(static_cast<int>(std::round(seed.y() * height)), 0, height - boxHeight);
        const auto *reference = reinterpret_cast<const uchar *>(frames.constData());
        int darkest = 255;
        int brightest = 0;
        for (int row = 0; row < boxHeight; row += 2) {
            for (int column = 0; column < boxWidth; column += 2) {
                const int pixel = reference[(y + row) * width + x + column];
                darkest = std::min(darkest, pixel);
                brightest = std::max(brightest, pixel);
            }
        }
        if (brightest - darkest < 20) return boxes;
        auto difference = [&](const uchar *image, int candidateX, int candidateY) {
            int sum = 0;
            int pixels = 0;
            for (int row = 0; row < boxHeight; row += 2) {
                for (int column = 0; column < boxWidth; column += 2) {
                    sum += std::abs(int(reference[(y + row) * width + x + column]) -
                                    int(image[(candidateY + row) * width + candidateX + column]));
                    ++pixels;
                }
            }
            return double(sum) / pixels;
        };
        const qint64 firstFrame = std::llround(start * m_fps);
        boxes.append({firstFrame, seed});
        *reliableEnd = end;
        int previousX = x;
        int previousY = y;
        for (qsizetype index = 1; index < count; ++index) {
            const auto *image = reinterpret_cast<const uchar *>(frames.constData() + index * frameBytes);
            double best = std::numeric_limits<double>::infinity();
            int bestX = previousX;
            int bestY = previousY;
            const int radius = std::max(8, std::min(boxWidth, boxHeight) / 2);
            for (int candidateY = std::max(0, previousY - radius);
                 candidateY <= std::min(height - boxHeight, previousY + radius); ++candidateY) {
                for (int candidateX = std::max(0, previousX - radius);
                     candidateX <= std::min(width - boxWidth, previousX + radius); ++candidateX) {
                    const double score = difference(image, candidateX, candidateY);
                    if (score < best) { best = score; bestX = candidateX; bestY = candidateY; }
                }
            }
            const double time = start + double(index) / samplesPerSecond;
            if (best > 28.0) {
                *reliableEnd = std::min(end, time);
                break;
            }
            const qint64 frame = std::llround(time * m_fps);
            if (frame >= std::llround(end * m_fps)) break;
            if (frame > boxes.last().first && (std::abs(bestX - previousX) > 1 || std::abs(bestY - previousY) > 1))
                boxes.append({frame, QRectF(std::min(double(bestX) / width, 1.0 - seed.width()),
                                            std::min(double(bestY) / height, 1.0 - seed.height()),
                                            seed.width(), seed.height())});
            previousX = bestX;
            previousY = bestY;
        }
        return boxes;
    }

    void focusRegion() {
        if (!m_proposalId.isEmpty()) {
            QMessageBox::information(this, QStringLiteral("Focus region"),
                                     QStringLiteral("Review the pending suggestion before changing the crop."));
            return;
        }
        const QJsonObject clip = selectedClipObject();
        if (clip.isEmpty()) return;
        bool video = false;
        for (const QJsonValue &value : m_project.value(QStringLiteral("tracks")).toArray()) {
            const QJsonObject track = value.toObject();
            if (track.value(QStringLiteral("type")).toString() != QStringLiteral("video")) continue;
            for (const QJsonValue &entry : track.value(QStringLiteral("clips")).toArray())
                video |= entry.toObject().value(QStringLiteral("id")).toString() == m_selectedClip;
        }
        if (!video) return;
        const double clipStart = clip.value(QStringLiteral("timelineStart")).toDouble();
        const double sourceIn = clip.value(QStringLiteral("in")).toDouble();
        const double sourceOut = clip.value(QStringLiteral("out")).toDouble();
        const double speed = clip.value(QStringLiteral("speed")).toDouble(1.0);
        const double clipDuration = clip.value(QStringLiteral("duration")).toDouble(
            speed > 0.0 ? (sourceOut - sourceIn) / speed : 0.0);
        const double clipEnd = clipStart + clipDuration;
        if (!(clipDuration > 1.0 / m_fps)) return;

        const QJsonObject crop = clip.value(QStringLiteral("crop")).toObject();
        const double representativeTime = std::clamp(m_playhead, clipStart, clipEnd - 1.0 / m_fps);
        const QImage frame = sourceFrame(clip, representativeTime);
        if (frame.isNull()) {
            QMessageBox::information(this, QStringLiteral("Focus region"),
                                     QStringLiteral("Could not read a source frame from this clip."));
            return;
        }

        const QRectF current(crop.value(QStringLiteral("x")).toDouble(), crop.value(QStringLiteral("y")).toDouble(),
                             crop.value(QStringLiteral("width")).toDouble(), crop.value(QStringLiteral("height")).toDouble());
        QDialog dialog(this);
        dialog.setWindowTitle(QStringLiteral("Focus region"));
        auto *layout = new QVBoxLayout(&dialog);
        layout->addWidget(new QLabel(QStringLiteral("Drag a box over the area to enlarge.")));
        auto *picker = new RegionPicker(frame, current, &dialog);
        layout->addWidget(picker);
        auto *form = new QFormLayout;
        auto *start = new QDoubleSpinBox(&dialog);
        auto *end = new QDoubleSpinBox(&dialog);
        for (QDoubleSpinBox *spin : {start, end}) {
            spin->setDecimals(3);
            spin->setSingleStep(1.0 / m_fps);
            spin->setSuffix(QStringLiteral(" s"));
        }
        start->setRange(clipStart, clipEnd - 1.0 / m_fps);
        end->setRange(clipStart + 1.0 / m_fps, clipEnd);
        const double suggestedStart = crop.isEmpty() ? representativeTime : clipStart;
        start->setValue(suggestedStart);
        end->setValue(crop.isEmpty() ? std::min(clipEnd, suggestedStart + 5.0) : clipEnd);
        form->addRow(QStringLiteral("Start"), start);
        form->addRow(QStringLiteral("End"), end);
        layout->addLayout(form);
        auto *follow = new QCheckBox(QStringLiteral("Follow a moving subject (up to 2 minutes)"), &dialog);
        layout->addWidget(follow);
        connect(follow, &QCheckBox::toggled, &dialog, [start, representativeTime](bool enabled) {
            if (enabled) start->setValue(representativeTime);
            start->setEnabled(!enabled);
        });
        auto *buttons = new QDialogButtonBox(QDialogButtonBox::Ok | QDialogButtonBox::Cancel, &dialog);
        connect(buttons, &QDialogButtonBox::accepted, &dialog, &QDialog::accept);
        connect(buttons, &QDialogButtonBox::rejected, &dialog, &QDialog::reject);
        layout->addWidget(buttons);
        if (dialog.exec() != QDialog::Accepted) return;
        const QRectF region = picker->region();
        const qint64 clipStartFrame = std::llround(clipStart * m_fps);
        const qint64 clipEndFrame = std::llround(clipEnd * m_fps);
        const qint64 startFrame = std::llround(start->value() * m_fps);
        qint64 endFrame = std::llround(end->value() * m_fps);
        if (region.isEmpty() || startFrame < clipStartFrame || endFrame > clipEndFrame || startFrame >= endFrame) {
            QMessageBox::information(this, QStringLiteral("Focus region"),
                                     QStringLiteral("Select a box and a valid time range within the clip."));
            return;
        }
        const double cut = firstSceneCut(clip, start->value(), end->value());
        if (!std::isfinite(cut)) {
            QMessageBox::information(this, QStringLiteral("Focus region"),
                                     QStringLiteral("Could not check this range for scene cuts. Choose a shorter range and try again."));
            return;
        }
        if (cut > 0.0) {
            endFrame = std::llround(cut * m_fps);
            QMessageBox::information(this, QStringLiteral("Focus region"),
                                     QStringLiteral("A scene cut was found. This box ends at the cut; select a new box for the next scene."));
        }

        double reliableEnd = double(endFrame) / m_fps;
        const QVector<QPair<qint64, QRectF>> tracked = follow->isChecked()
            ? trackRegion(clip, double(startFrame) / m_fps, reliableEnd, region, &reliableEnd)
            : QVector<QPair<qint64, QRectF>>{};
        if (follow->isChecked() && tracked.isEmpty()) {
            QMessageBox::information(this, QStringLiteral("Focus region"),
                                     QStringLiteral("Tracking could not start. Choose a larger box or a range under two minutes."));
            return;
        }
        if (reliableEnd < double(endFrame) / m_fps) {
            endFrame = std::llround(reliableEnd * m_fps);
            QMessageBox::information(this, QStringLiteral("Focus region"),
                                     QStringLiteral("The subject could no longer be tracked. The crop ends at the last reliable frame."));
        }

        const QString suffix = QString::number(QDateTime::currentMSecsSinceEpoch());
        QString focusId = m_selectedClip;
        QJsonArray commands;
        if (startFrame > clipStartFrame) {
            focusId = m_selectedClip + QStringLiteral("-focus-") + suffix;
            commands.append(QJsonObject{{QStringLiteral("type"), QStringLiteral("split_clip")},
                                        {QStringLiteral("clipId"), m_selectedClip},
                                        {QStringLiteral("atFrame"), startFrame},
                                        {QStringLiteral("newClipId"), focusId}});
        }
        const QJsonObject box{{QStringLiteral("x"), region.x()}, {QStringLiteral("y"), region.y()},
                              {QStringLiteral("width"), region.width()}, {QStringLiteral("height"), region.height()}};
        commands.append(QJsonObject{{QStringLiteral("type"), QStringLiteral("update_clip")},
                                    {QStringLiteral("clipId"), focusId},
                                    {QStringLiteral("patch"), QJsonObject{{QStringLiteral("crop"), box}}}});
        for (int index = 1; index < tracked.size(); ++index) {
            const auto &step = tracked.at(index);
            if (step.first >= endFrame) break;
            const QString nextId = m_selectedClip + QStringLiteral("-track-%1-").arg(index) + suffix;
            commands.append(QJsonObject{{QStringLiteral("type"), QStringLiteral("split_clip")},
                                        {QStringLiteral("clipId"), focusId}, {QStringLiteral("atFrame"), step.first},
                                        {QStringLiteral("newClipId"), nextId}});
            focusId = nextId;
            const QRectF moved = step.second;
            commands.append(QJsonObject{{QStringLiteral("type"), QStringLiteral("update_clip")},
                                        {QStringLiteral("clipId"), focusId},
                                        {QStringLiteral("patch"), QJsonObject{{QStringLiteral("crop"),
                                            QJsonObject{{QStringLiteral("x"), moved.x()}, {QStringLiteral("y"), moved.y()},
                                                        {QStringLiteral("width"), moved.width()}, {QStringLiteral("height"), moved.height()}}}}}});
        }
        if (endFrame < clipEndFrame) {
            const QString afterId = m_selectedClip + QStringLiteral("-after-") + suffix;
            commands.append(QJsonObject{{QStringLiteral("type"), QStringLiteral("split_clip")},
                                        {QStringLiteral("clipId"), focusId},
                                        {QStringLiteral("atFrame"), endFrame},
                                        {QStringLiteral("newClipId"), afterId}});
            commands.append(QJsonObject{{QStringLiteral("type"), QStringLiteral("update_clip")},
                                        {QStringLiteral("clipId"), afterId},
                                        {QStringLiteral("patch"), QJsonObject{{QStringLiteral("crop"),
                                            clip.contains(QStringLiteral("crop")) ? clip.value(QStringLiteral("crop")) : QJsonValue()}}}});
        }
        if (sendSimple(commandApply(QJsonObject{{QStringLiteral("type"), QStringLiteral("batch")},
                                               {QStringLiteral("commands"), commands}}, m_revision)))
            m_selectedClip = focusId;
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
        const QString state = !m_connected ? QStringLiteral("offline")
                              : m_agentBusy ? QStringLiteral("sending")
                              : m_chatInput->hasFocus() ? QStringLiteral("focused") : QStringLiteral("ready");
        if (m_chatComposer->property("state").toString() != state) {
            m_chatComposer->setProperty("state", state);
            m_chatComposer->style()->unpolish(m_chatComposer);
            m_chatComposer->style()->polish(m_chatComposer);
            m_chatComposer->update();
        }
        m_composerStatus->setText(!m_connected ? QStringLiteral("Connect to start editing")
                                 : m_agentBusy ? QStringLiteral("Cut is working…")
                                                : QStringLiteral("Return to send · ⇧ Return for a new line"));
        m_chatInput->setPlaceholderText(!m_connected ? QStringLiteral("Your next edit starts here…")
                                       : m_agentBusy ? QStringLiteral("Working on your edit…")
                                                     : QStringLiteral("Describe your next edit…"));
    }

    QString activeProviderId() const {
        return m_openaiActiveRadio && m_openaiActiveRadio->isChecked() ? QStringLiteral("openai")
                                                                       : QStringLiteral("anthropic");
    }

    QJsonObject agentSettingsPayload() const {
        const bool openai = activeProviderId() == QStringLiteral("openai");
        const QLineEdit *key = openai ? m_openaiKey : m_anthropicKey;
        const QLineEdit *baseUrl = openai ? m_openaiBaseUrl : m_anthropicBaseUrl;
        return QJsonObject{{QStringLiteral("provider"), activeProviderId()},
                           {QStringLiteral("apiMode"), m_openaiMode->currentData().toString()},
                           {QStringLiteral("apiKey"), key ? key->text().trimmed() : QString()},
                           {QStringLiteral("baseUrl"), baseUrl ? baseUrl->text().trimmed() : QString()},
                           {QStringLiteral("model"), m_modelSelect->currentText().trimmed()},
                           {QStringLiteral("mcpEnabled"), m_mcpEnabled && m_mcpEnabled->isChecked()},
                           {QStringLiteral("mcpConfigVersion"), static_cast<qint64>(m_mcpConfigVersion)}};
    }

    void loadInboundMcpConfig(const QString &path, bool imported) {
        QFile file(path);
        if (!file.open(QIODevice::ReadOnly)) {
            m_inboundMcpStatus->setText(QStringLiteral("Could not read %1: %2").arg(path, file.errorString()));
            return;
        }
        const QByteArray raw = file.readAll();
        if (file.error() != QFileDevice::NoError) {
            m_inboundMcpStatus->setText(QStringLiteral("Could not read %1: %2").arg(path, file.errorString()));
            return;
        }
        m_inboundMcpJson->setPlainText(QString::fromUtf8(raw));
        const QString error = validateInboundMcpConfig(raw);
        m_inboundMcpStatus->setText(error.isEmpty()
            ? (imported ? QStringLiteral("Loaded %1 into the editor only. Review, then Save and apply.").arg(path)
                        : QStringLiteral("Loaded project config. Changes apply on the next chat request."))
            : QStringLiteral("Loaded %1. %2").arg(path, error));
    }

    void saveInboundMcpConfig() {
        const QByteArray raw = m_inboundMcpJson->toPlainText().toUtf8();
        const QString error = validateInboundMcpConfig(raw);
        if (!error.isEmpty()) {
            m_inboundMcpStatus->setText(error);
            return;
        }
        const QDir project(m_projectDirectory);
        if (!project.mkpath(QStringLiteral(".cut"))) {
            m_inboundMcpStatus->setText(QStringLiteral("Could not create the project's .cut directory."));
            return;
        }
        QSaveFile file(project.filePath(QStringLiteral(".cut/mcp.json")));
        if (!file.open(QIODevice::WriteOnly) || file.write(raw) != raw.size() || !file.commit()) {
            m_inboundMcpStatus->setText(QStringLiteral("Could not save MCP config: %1").arg(file.errorString()));
            return;
        }
        ++m_mcpConfigVersion;
        m_inboundMcpStatus->setText(QStringLiteral("Saved project .cut/mcp.json. Applies on the next chat request when MCP is enabled."));
    }

    void updateAgentModelChoices() {
        if (!m_modelSelect) return;
        const bool openai = activeProviderId() == QStringLiteral("openai");
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
        if (provider == QStringLiteral("openai"))
            m_openaiActiveRadio->setChecked(true);
        else
            m_anthropicActiveRadio->setChecked(true);
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
        settings.setValue(QStringLiteral("agent/activeProvider"), activeProviderId());
        settings.sync();
        updateAgentModelChoices();
        statusBar()->showMessage(QStringLiteral("Agent settings saved"), 3000);
    }

    void acceptProposal() {
        if (m_proposalId.isEmpty()) return;
        sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("accept")}, {QStringLiteral("proposalId"), m_proposalId}, {QStringLiteral("revision"), revisionValue()}});
    }

    void adjustProposalCrop() {
        const QJsonObject proposal = m_snapshot.value(QStringLiteral("pendingProposal")).toObject();
        const QString proposalId = proposal.value(QStringLiteral("proposalId")).toString();
        const quint64 baseRevision = m_revision;
        const QJsonObject edit = proposalCropEdit(proposal);
        if (edit.isEmpty() || proposal.value(QStringLiteral("freshness")).toString() == QStringLiteral("stale")) return;
        const QString focusId = edit.value(QStringLiteral("clipId")).toString();
        QString originalId = focusId;
        QQueue<QJsonObject> pending;
        pending.enqueue(proposal.value(QStringLiteral("command")).toObject());
        QList<QJsonObject> splits;
        while (!pending.isEmpty()) {
            const QJsonObject command = pending.dequeue();
            if (command.value(QStringLiteral("type")).toString() == QStringLiteral("batch")) {
                for (const QJsonValue &child : command.value(QStringLiteral("commands")).toArray())
                    pending.enqueue(child.toObject());
            } else if (command.value(QStringLiteral("type")).toString() == QStringLiteral("split_clip")) {
                splits.append(command);
            }
        }
        for (int pass = 0; pass < splits.size(); ++pass) {
            bool changed = false;
            for (const QJsonObject &split : splits) {
                if (split.value(QStringLiteral("newClipId")).toString() == originalId) {
                    originalId = split.value(QStringLiteral("clipId")).toString();
                    changed = true;
                    break;
                }
            }
            if (!changed) break;
        }
        QJsonObject original;
        for (const QJsonValue &track : m_project.value(QStringLiteral("tracks")).toArray()) {
            for (const QJsonValue &clip : track.toObject().value(QStringLiteral("clips")).toArray()) {
                if (clip.toObject().value(QStringLiteral("id")).toString() == originalId) original = clip.toObject();
            }
        }
        if (original.isEmpty()) return;
        const double clipStart = original.value(QStringLiteral("timelineStart")).toDouble();
        const double clipEnd = clipStart + original.value(QStringLiteral("duration")).toDouble(
            (original.value(QStringLiteral("out")).toDouble() - original.value(QStringLiteral("in")).toDouble()) /
            original.value(QStringLiteral("speed")).toDouble(1.0));
        const QImage frame = sourceFrame(original, std::clamp(m_playhead, clipStart, clipEnd - 1.0 / m_fps));
        if (frame.isNull()) {
            QMessageBox::information(this, QStringLiteral("Adjust crop"),
                                     QStringLiteral("Could not read a source frame from this clip."));
            return;
        }
        const QJsonObject crop = edit.value(QStringLiteral("patch")).toObject().value(QStringLiteral("crop")).toObject();
        const QRectF proposed(crop.value(QStringLiteral("x")).toDouble(), crop.value(QStringLiteral("y")).toDouble(),
                              crop.value(QStringLiteral("width")).toDouble(), crop.value(QStringLiteral("height")).toDouble());
        QDialog dialog(this);
        dialog.setWindowTitle(QStringLiteral("Adjust proposed crop"));
        auto *layout = new QVBoxLayout(&dialog);
        layout->addWidget(new QLabel(QStringLiteral("Drag a replacement box, then apply the edited proposal.")));
        auto *picker = new RegionPicker(frame, proposed, &dialog);
        layout->addWidget(picker);
        auto *buttons = new QDialogButtonBox(QDialogButtonBox::Ok | QDialogButtonBox::Cancel, &dialog);
        connect(buttons, &QDialogButtonBox::accepted, &dialog, &QDialog::accept);
        connect(buttons, &QDialogButtonBox::rejected, &dialog, &QDialog::reject);
        layout->addWidget(buttons);
        if (dialog.exec() != QDialog::Accepted) return;
        const QRectF region = picker->region();
        if (region.isEmpty()) return;
        const QJsonObject box{{QStringLiteral("x"), region.x()}, {QStringLiteral("y"), region.y()},
                              {QStringLiteral("width"), region.width()}, {QStringLiteral("height"), region.height()}};
        sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("accept")},
                               {QStringLiteral("proposalId"), proposalId},
                               {QStringLiteral("revision"), QJsonValue::fromVariant(QVariant::fromValue(baseRevision))},
                               {QStringLiteral("cropOverride"), QJsonObject{{QStringLiteral("clipId"), focusId},
                                                                             {QStringLiteral("crop"), box}}}});
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
        if (object == m_chatInput && (event->type() == QEvent::FocusIn || event->type() == QEvent::FocusOut))
            updateChatControls();
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

        auto *editorSplit = new QSplitter(Qt::Vertical);
        editorSplit->setObjectName(QStringLiteral("editorSplitter"));
        editorSplit->setHandleWidth(2);
        editorSplit->setChildrenCollapsible(false);
        workspace->addWidget(editorSplit);

        auto *viewerFrame = new QFrame;
        viewerFrame->setObjectName(QStringLiteral("viewerFrame"));
        auto *viewerLayout = new QVBoxLayout(viewerFrame);
        viewerLayout->setContentsMargins(0, 0, 0, 0);
        m_viewer = new ViewerWidget;
        viewerLayout->addWidget(m_viewer);
        editorSplit->addWidget(viewerFrame);

        auto *bottomBand = new QWidget;
        bottomBand->setObjectName(QStringLiteral("bottomBand"));
        bottomBand->setAttribute(Qt::WA_StyledBackground);
        auto *bottomLayout = new QVBoxLayout(bottomBand);
        bottomLayout->setContentsMargins(0, 0, 0, 0);
        bottomLayout->setSpacing(0);

        auto *timelinePanel = new QWidget;
        timelinePanel->setObjectName(QStringLiteral("timelinePanel"));
        timelinePanel->setAttribute(Qt::WA_StyledBackground);
        auto *timelinePanelLayout = new QVBoxLayout(timelinePanel);
        timelinePanelLayout->setContentsMargins(0, 0, 0, 0);
        timelinePanelLayout->setSpacing(0);

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
        timelinePanel->setMinimumHeight(280);
        bottomLayout->addWidget(timelinePanel, 1);
        editorSplit->addWidget(bottomBand);
        editorSplit->setStretchFactor(0, 1);
        editorSplit->setStretchFactor(1, 1);
        editorSplit->setSizes({420, 460});

        auto *right = new QWidget;
        m_agentPanel = right;
        right->setObjectName(QStringLiteral("assistantPanel"));
        right->setAttribute(Qt::WA_StyledBackground);
        right->setMinimumWidth(300);
        auto *rightLayout = new QVBoxLayout(right);
        rightLayout->setContentsMargins(12, 12, 12, 12);
        rightLayout->setSpacing(10);
        auto *agentHeader = new QHBoxLayout;
        auto *agentTitle = new QLabel(QStringLiteral("Cut Assistant"));
        agentTitle->setObjectName(QStringLiteral("assistantTitle"));
        m_agentStatus = new QLabel(QStringLiteral("READY"));
        m_agentStatus->setObjectName(QStringLiteral("statusPill"));
        agentHeader->addWidget(agentTitle);
        agentHeader->addStretch();
        agentHeader->addWidget(m_agentStatus);
        rightLayout->addLayout(agentHeader);
        auto *agentSubtitle = new QLabel(QStringLiteral("Shape your story, one edit at a time."));
        agentSubtitle->setObjectName(QStringLiteral("muted"));
        rightLayout->addWidget(agentSubtitle);

        auto *tabs = new QTabWidget;
        tabs->setObjectName(QStringLiteral("rightTabs"));
        rightLayout->addWidget(tabs, 1);
        auto *chatTab = new QWidget;
        auto *chatTabLayout = new QVBoxLayout(chatTab);
        chatTabLayout->setContentsMargins(0, 8, 0, 0);
        chatTabLayout->setSpacing(10);
        // Review and activity can overflow on short windows; the composer stays fixed.
        auto *chatScroll = new QScrollArea;
        chatScroll->setFrameShape(QFrame::NoFrame);
        chatScroll->setWidgetResizable(true);
        chatScroll->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
        auto *chatContent = new QWidget;
        auto *chatLayout = new QVBoxLayout(chatContent);
        chatLayout->setContentsMargins(0, 0, 0, 0);
        chatLayout->setSizeConstraint(QLayout::SetMinimumSize);
        chatLayout->setSpacing(10);
        chatScroll->setWidget(chatContent);
        chatTabLayout->addWidget(chatScroll, 1);
        m_conversation = new QTextEdit;
        m_conversation->setReadOnly(true);
        m_conversation->setObjectName(QStringLiteral("conversation"));
        m_conversation->setAcceptRichText(false);
        m_conversation->setUndoRedoEnabled(false);
        m_conversation->setTextInteractionFlags(Qt::TextSelectableByMouse | Qt::TextSelectableByKeyboard);
        m_conversation->setAccessibleName(QStringLiteral("Conversation with Cut"));
        m_conversation->setMinimumHeight(80);
        m_conversation->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
        m_conversation->document()->setDocumentMargin(2);
        m_conversation->setHtml(QStringLiteral(
            "<p style='margin-top:4px; color:#c6a15a; font-size:11px;'>YOUR EDITING PARTNER</p>"
            "<p style='margin-top:4px; color:#f0f0f0; font-size:18px; font-weight:600;'>Make your next cut.</p>"
            "<p style='margin-top:6px; color:#b6b6b6;'>Describe the change. Cut uses this timeline.</p>"
            "<p style='margin-top:14px; color:#888888; font-size:11px;'>TRY ASKING</p>"
            "<p style='margin-top:2px; color:#dddddd;'>“Trim the opening to three seconds.”</p>"
            "<p style='margin-top:2px; color:#dddddd;'>“Lower the music under the dialogue.”</p>"
            "<p style='margin-top:14px; color:#999999; font-size:12px;'>Review a proposal before it changes the cut.</p>"));
        chatLayout->addWidget(m_conversation, 1);
        auto *composer = new QFrame;
        m_chatComposer = composer;
        composer->setObjectName(QStringLiteral("chatComposer"));
        auto *composerLayout = new QVBoxLayout(composer);
        composerLayout->setContentsMargins(10, 6, 8, 6);
        composerLayout->setSpacing(4);
        auto *agentPicker = new QHBoxLayout;
        agentPicker->setSpacing(5);
        auto *modelLabel = new QLabel(QStringLiteral("Model"));
        modelLabel->setObjectName(QStringLiteral("muted"));
        m_modelSelect = new QComboBox;
        m_modelSelect->setEditable(true);
        m_modelSelect->setInsertPolicy(QComboBox::NoInsert);
        m_modelSelect->setSizeAdjustPolicy(QComboBox::AdjustToMinimumContentsLengthWithIcon);
        m_modelSelect->setMinimumContentsLength(12);
        m_modelSelect->setObjectName(QStringLiteral("agentModel"));
        m_modelSelect->setToolTip(QStringLiteral("Model ID sent with each chat request. Suggestions follow the active connection in Settings."));
        agentPicker->addWidget(modelLabel);
        agentPicker->addWidget(m_modelSelect, 2);
        auto *chatRow = new QHBoxLayout;
        chatRow->setContentsMargins(0, 0, 0, 0);
        chatRow->setSpacing(6);
        m_chatInput = new QTextEdit;
        m_chatInput->setObjectName(QStringLiteral("chatInput"));
        m_chatInput->setPlaceholderText(QStringLiteral("Ask Cut to edit…"));
        m_chatInput->setAccessibleName(QStringLiteral("Message Cut"));
        m_chatInput->setAcceptRichText(false);
        m_chatInput->setFixedHeight(32);
        m_chatInput->document()->setDocumentMargin(2);
        m_chatInput->setVerticalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
        m_chatInput->setTabChangesFocus(true);
        m_chatInput->installEventFilter(this);
        connect(m_chatInput, &QTextEdit::textChanged, this, &MainWindow::updateChatControls);
        composerLayout->addWidget(m_chatInput);
        composerLayout->addLayout(agentPicker);
        auto *sendButton = new QPushButton(QStringLiteral("↑"));
        sendButton->setObjectName(QStringLiteral("chatSendButton"));
        m_sendButton = sendButton;
        sendButton->setAccessibleName(QStringLiteral("Send message"));
        sendButton->setFixedSize(32, 32);
        sendButton->setToolTip(QStringLiteral("Send message (Return)"));
        connect(sendButton, &QPushButton::clicked, this, &MainWindow::sendChat);
        m_composerStatus = new QLabel;
        m_composerStatus->setObjectName(QStringLiteral("composerStatus"));
        m_composerStatus->setWordWrap(true);
        chatRow->addWidget(m_composerStatus, 1);
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
        m_chatActivity = new QLabel;
        m_chatActivity->setObjectName(QStringLiteral("chatActivity"));
        m_chatActivity->setTextFormat(Qt::PlainText);
        m_chatActivity->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Fixed);
        m_chatActivity->setFixedHeight(28);
        m_chatActivity->hide();
        chatLayout->addWidget(m_chatActivity);
        auto *activityTitle = new QToolButton;
        activityTitle->setText(QStringLiteral("▸ Activity"));
        activityTitle->setObjectName(QStringLiteral("eyebrow"));
        chatLayout->addWidget(activityTitle);
        m_activityList = new QListWidget;
        m_activityList->setObjectName(QStringLiteral("activityList"));
        m_activityList->setMaximumHeight(60);
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
        proposalLayout->setSizeConstraint(QLayout::SetMinimumSize);
        m_proposalSummary = new QLabel(QStringLiteral("No pending changes"));
        m_proposalSummary->setWordWrap(true);
        m_proposalSummary->setObjectName(QStringLiteral("proposalSummary"));
        m_proposalDiff = new QTextEdit;
        m_proposalDiff->setReadOnly(true);
        m_proposalDiff->setMinimumHeight(80);
        m_proposalDiff->setMaximumHeight(240);
        m_proposalDiff->setObjectName(QStringLiteral("muted"));
        auto *proposalButtons = new QHBoxLayout;
        m_acceptButton = new QPushButton(QStringLiteral("Accept"));
        m_acceptButton->setObjectName(QStringLiteral("primaryButton"));
        m_adjustCropButton = new QPushButton(QStringLiteral("Adjust box…"));
        m_rejectButton = new QPushButton(QStringLiteral("Reject"));
        m_rejectButton->setObjectName(QStringLiteral("dangerButton"));
        connect(m_acceptButton, &QPushButton::clicked, this, &MainWindow::acceptProposal);
        connect(m_adjustCropButton, &QPushButton::clicked, this, &MainWindow::adjustProposalCrop);
        connect(m_rejectButton, &QPushButton::clicked, this, &MainWindow::rejectProposal);
        proposalButtons->addWidget(m_acceptButton);
        proposalButtons->addWidget(m_adjustCropButton);
        proposalButtons->addWidget(m_rejectButton);
        proposalLayout->addWidget(m_proposalSummary);
        proposalLayout->addWidget(m_proposalDiff);
        proposalLayout->addLayout(proposalButtons);
        chatLayout->addWidget(proposalBox);
        chatTabLayout->addWidget(composer);
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
        auto *settingsIntro = new QLabel(QStringLiteral("Two protocol slots. One active connection for chat. Gemini, Groq, and OpenRouter use the OpenAI-compatible slot with that vendor's compatible base URL."));
        settingsIntro->setObjectName(QStringLiteral("chatHint"));
        settingsIntro->setWordWrap(true);
        settingsLayout->addWidget(settingsIntro);
        auto *activeConnection = new QGroupBox(QStringLiteral("Active connection"));
        auto *activeLayout = new QVBoxLayout(activeConnection);
        activeLayout->setContentsMargins(10, 12, 10, 10);
        activeLayout->setSpacing(6);
        m_anthropicActiveRadio = new QRadioButton(QStringLiteral("Anthropic Messages"));
        m_anthropicActiveRadio->setObjectName(QStringLiteral("agentActiveAnthropic"));
        m_openaiActiveRadio = new QRadioButton(QStringLiteral("OpenAI-compatible"));
        m_openaiActiveRadio->setObjectName(QStringLiteral("agentActiveOpenai"));
        activeLayout->addWidget(m_anthropicActiveRadio);
        activeLayout->addWidget(m_openaiActiveRadio);
        settingsLayout->addWidget(activeConnection);

        auto *anthropicGroup = new QGroupBox(QStringLiteral("Anthropic Messages"));
        anthropicGroup->setObjectName(QStringLiteral("providerBox"));
        auto *anthropicForm = new QFormLayout(anthropicGroup);
        anthropicForm->setContentsMargins(10, 12, 10, 10);
        anthropicForm->setSpacing(7);
        m_anthropicKey = new QLineEdit;
        m_anthropicKey->setEchoMode(QLineEdit::Password);
        m_anthropicKey->setPlaceholderText(QStringLiteral("sk-ant-…"));
        m_anthropicKey->setClearButtonEnabled(true);
        m_anthropicBaseUrl = new QLineEdit(QStringLiteral("https://api.anthropic.com/v1"));
        m_anthropicBaseUrl->setPlaceholderText(QStringLiteral("https://…/v1"));
        m_anthropicModel = new QLineEdit(QStringLiteral("claude-sonnet-4-20250514"));
        anthropicForm->addRow(QStringLiteral("API key"), m_anthropicKey);
        anthropicForm->addRow(QStringLiteral("Base URL"), m_anthropicBaseUrl);
        anthropicForm->addRow(QStringLiteral("Model"), m_anthropicModel);
        settingsLayout->addWidget(anthropicGroup);

        auto *openaiGroup = new QGroupBox(QStringLiteral("OpenAI-compatible"));
        openaiGroup->setObjectName(QStringLiteral("providerBox"));
        auto *openaiForm = new QFormLayout(openaiGroup);
        openaiForm->setContentsMargins(10, 12, 10, 10);
        openaiForm->setSpacing(7);
        m_openaiKey = new QLineEdit;
        m_openaiKey->setEchoMode(QLineEdit::Password);
        m_openaiKey->setPlaceholderText(QStringLiteral("sk-…"));
        m_openaiKey->setClearButtonEnabled(true);
        m_openaiBaseUrl = new QLineEdit(QStringLiteral("https://api.openai.com/v1"));
        m_openaiBaseUrl->setPlaceholderText(QStringLiteral("https://…/v1"));
        auto *openaiBaseUrlHint = new QLabel(QStringLiteral("Must serve /chat/completions or /responses. Native Gemini generateContent URLs return 404. Use a root that ends in /openai."));
        openaiBaseUrlHint->setObjectName(QStringLiteral("muted"));
        openaiBaseUrlHint->setWordWrap(true);
        m_openaiModel = new QLineEdit(QStringLiteral("gpt-5.6-luna"));
        m_openaiMode = new QComboBox;
        m_openaiMode->addItem(QStringLiteral("Chat Completions"), QStringLiteral("chat"));
        m_openaiMode->addItem(QStringLiteral("Responses"), QStringLiteral("responses"));
        openaiForm->addRow(QStringLiteral("API key"), m_openaiKey);
        openaiForm->addRow(QStringLiteral("Base URL"), m_openaiBaseUrl);
        openaiForm->addRow(QString(), openaiBaseUrlHint);
        openaiForm->addRow(QStringLiteral("Model"), m_openaiModel);
        openaiForm->addRow(QStringLiteral("API shape"), m_openaiMode);
        settingsLayout->addWidget(openaiGroup);
        auto *settingsNote = new QLabel(QStringLiteral("Credentials stay outside project files and are never included in chat activity."));
        settingsNote->setObjectName(QStringLiteral("muted"));
        settingsNote->setWordWrap(true);
        settingsLayout->addWidget(settingsNote);
        auto *saveSettings = new QPushButton(QStringLiteral("Save agent connection"));
        saveSettings->setObjectName(QStringLiteral("primaryButton"));
        connect(saveSettings, &QPushButton::clicked, this, &MainWindow::saveAgentSettings);
        settingsLayout->addWidget(saveSettings);

        auto *inboundSection = new QGroupBox(QStringLiteral("Use MCP tools in Cut chat"));
        inboundSection->setObjectName(QStringLiteral("inboundMcpSection"));
        auto *inboundLayout = new QVBoxLayout(inboundSection);
        m_mcpEnabled = new QCheckBox(QStringLiteral("Enable external MCP tools"));
        m_mcpEnabled->setObjectName(QStringLiteral("inboundMcpEnabled"));
        const bool environmentMcpEnabled = qEnvironmentVariable("CUT_ENABLE_MCP") == QStringLiteral("1");
        m_mcpEnabled->setChecked(QSettings().value(QStringLiteral("agent/mcpEnabled"), environmentMcpEnabled).toBool());
        inboundLayout->addWidget(m_mcpEnabled);
        auto *inboundIntro = new QLabel(QStringLiteral("Cut chat calls external MCP servers directly; Cursor is not needed. Your AI provider above still powers the chat. Cut starts local commands using stdio; URL-only remote servers are not supported. CUT_ENABLE_MCP=1 also enables tools even if this box is unchecked."));
        inboundIntro->setWordWrap(true);
        inboundIntro->setObjectName(QStringLiteral("chatHint"));
        inboundLayout->addWidget(inboundIntro);
        const QString inboundPath = QDir(m_projectDirectory).absoluteFilePath(QStringLiteral(".cut/mcp.json"));
        auto *inboundPathLabel = new QLabel(QStringLiteral("Project config: %1\nAlso reads ~/.cut/mcp.json; project entries override matching server names. Use command, optional args, env, and tools (an allowlist). Commands must be installed locally; env values are literal strings. This file may contain server credentials.").arg(inboundPath));
        inboundPathLabel->setWordWrap(true);
        inboundPathLabel->setObjectName(QStringLiteral("muted"));
        inboundLayout->addWidget(inboundPathLabel);
        m_inboundMcpJson = new QTextEdit;
        m_inboundMcpJson->setObjectName(QStringLiteral("inboundMcpConfigJson"));
        m_inboundMcpJson->setAcceptRichText(false);
        m_inboundMcpJson->setFont(QFontDatabase::systemFont(QFontDatabase::FixedFont));
        m_inboundMcpJson->setMinimumHeight(160);
        m_inboundMcpJson->setPlainText(QStringLiteral("{\n  \"mcpServers\": {}\n}\n"));
        m_inboundMcpJson->setPlaceholderText(QStringLiteral("{\"mcpServers\": {\"example\": {\"command\": \"/path/to/server\", \"args\": [], \"env\": {}}}}"));
        inboundLayout->addWidget(m_inboundMcpJson);
        for (const auto &source : {qMakePair(QStringLiteral("Load project Cursor config"), QDir(m_projectDirectory).absoluteFilePath(QStringLiteral(".cursor/mcp.json"))),
                                   qMakePair(QStringLiteral("Load home Cursor config"), QDir::home().filePath(QStringLiteral(".cursor/mcp.json")))}) {
            auto *load = new QPushButton(source.first);
            load->setObjectName(source.first.contains(QStringLiteral("project")) ? QStringLiteral("loadProjectCursorMcp") : QStringLiteral("loadHomeCursorMcp"));
            load->setToolTip(source.second);
            connect(load, &QPushButton::clicked, this, [this, path = source.second]() { loadInboundMcpConfig(path, true); });
            inboundLayout->addWidget(load);
        }
        auto *saveInbound = new QPushButton(QStringLiteral("Save and apply MCP config"));
        saveInbound->setObjectName(QStringLiteral("saveInboundMcpConfig"));
        connect(saveInbound, &QPushButton::clicked, this, &MainWindow::saveInboundMcpConfig);
        inboundLayout->addWidget(saveInbound);
        m_inboundMcpStatus = new QLabel(QStringLiteral("No project config yet. Add servers here or load a Cursor config, then Save and apply. Existing home Cut config is used automatically."));
        m_inboundMcpStatus->setObjectName(QStringLiteral("inboundMcpStatus"));
        m_inboundMcpStatus->setWordWrap(true);
        inboundLayout->addWidget(m_inboundMcpStatus);
        if (QFileInfo::exists(inboundPath)) loadInboundMcpConfig(inboundPath, false);
        connect(m_mcpEnabled, &QCheckBox::toggled, this, [this](bool enabled) {
            QSettings settings;
            settings.setValue(QStringLiteral("agent/mcpEnabled"), enabled);
            settings.sync();
            m_inboundMcpStatus->setText(QStringLiteral("MCP preference saved. Applies on the next chat request. CUT_ENABLE_MCP=1 also enables tools."));
        });
        settingsLayout->addWidget(inboundSection);

        auto *mcpSection = new QGroupBox(QStringLiteral("Use Cut from Cursor / Claude"));
        mcpSection->setObjectName(QStringLiteral("mcpSection"));
        auto *mcpLayout = new QVBoxLayout(mcpSection);
        auto *mcpIntro = new QLabel(QStringLiteral("This is the opposite direction: Cursor, Claude, or Codex calls Cut through cut-mcp to work on this project. It does not enable external tools in Cut chat. Cut provider API keys are not required. Copy this config into your external client's MCP settings."));
        mcpIntro->setWordWrap(true);
        mcpIntro->setObjectName(QStringLiteral("chatHint"));
        mcpLayout->addWidget(mcpIntro);
        const QString projectPath = QDir(m_projectDirectory).absolutePath();
        auto *mcpProject = new QLineEdit(projectPath);
        mcpProject->setReadOnly(true);
        mcpProject->setObjectName(QStringLiteral("mcpProjectPath"));
        auto *mcpProjectForm = new QFormLayout;
        mcpProjectForm->addRow(QStringLiteral("Project"), mcpProject);
        mcpLayout->addLayout(mcpProjectForm);
        const QJsonObject config{{QStringLiteral("mcpServers"), QJsonObject{
            {QStringLiteral("cut"), QJsonObject{
                {QStringLiteral("command"), mcpExecutable(projectPath)},
                {QStringLiteral("args"), QJsonArray{QStringLiteral("--dir"), projectPath}}
            }}}}};
        auto *mcpJson = new QTextEdit;
        mcpJson->setObjectName(QStringLiteral("mcpConfigJson"));
        mcpJson->setReadOnly(true);
        mcpJson->setFont(QFontDatabase::systemFont(QFontDatabase::FixedFont));
        mcpJson->setPlainText(QString::fromUtf8(QJsonDocument(config).toJson(QJsonDocument::Indented)));
        mcpLayout->addWidget(mcpJson);
        auto *copyMcp = new QPushButton(QStringLiteral("Copy MCP config"));
        copyMcp->setObjectName(QStringLiteral("copyMcpConfig"));
        auto *mcpCopyStatus = new QLabel;
        mcpCopyStatus->setObjectName(QStringLiteral("muted"));
        connect(copyMcp, &QPushButton::clicked, this, [mcpJson, mcpCopyStatus]() {
            QApplication::clipboard()->setText(mcpJson->toPlainText());
            mcpCopyStatus->setText(QStringLiteral("MCP config copied"));
        });
        mcpLayout->addWidget(copyMcp);
        mcpLayout->addWidget(mcpCopyStatus);
        settingsLayout->addWidget(mcpSection);
        settingsLayout->addStretch(1);
        settingsScroll->setWidget(settingsContent);
        settingsTabLayout->addWidget(settingsScroll);
        tabs->addTab(settingsTab, QStringLiteral("Settings"));

        loadAgentSettings();
        const auto onActiveConnectionChanged = [this](bool checked) {
            if (!checked) return;
            updateAgentModelChoices();
            saveAgentSettings();
        };
        connect(m_anthropicActiveRadio, &QRadioButton::toggled, this, onActiveConnectionChanged);
        connect(m_openaiActiveRadio, &QRadioButton::toggled, this, onActiveConnectionChanged);
        connect(m_anthropicKey, &QLineEdit::editingFinished, this, &MainWindow::saveAgentSettings);
        connect(m_anthropicBaseUrl, &QLineEdit::editingFinished, this, &MainWindow::saveAgentSettings);
        connect(m_anthropicModel, &QLineEdit::editingFinished, this, &MainWindow::saveAgentSettings);
        connect(m_openaiKey, &QLineEdit::editingFinished, this, &MainWindow::saveAgentSettings);
        connect(m_openaiBaseUrl, &QLineEdit::editingFinished, this, &MainWindow::saveAgentSettings);
        connect(m_openaiModel, &QLineEdit::editingFinished, this, &MainWindow::saveAgentSettings);
        connect(m_openaiMode, QOverload<int>::of(&QComboBox::currentIndexChanged), this, &MainWindow::saveAgentSettings);

        workspace->addWidget(right);
        workspace->setStretchFactor(0, 1);
        workspace->setStretchFactor(1, 0);
        workspace->setSizes({1168, 332});
        setCentralWidget(central);

        connect(m_timeline, &TimelineWidget::seekRequested, this, &MainWindow::queueSeek);
        connect(m_timeline, &TimelineWidget::addLaneRequested, this, &MainWindow::addTrack);
        connect(m_timeline, &TimelineWidget::hoverSeekRequested, this, &MainWindow::queueHoverSeek);
        connect(m_timeline, &TimelineWidget::hoverSeekEnded, this, &MainWindow::endHoverSeek);
        connect(m_viewer, &ViewerWidget::scrubbed, this, [this](double fraction) {
            if (m_duration <= 0.0) return;
            const double time = fraction * m_duration;
            m_timeline->showSkimmer(time);
            queueHoverSeek(time);
        });
        connect(m_viewer, &ViewerWidget::scrubEnded, this, [this] {
            m_timeline->clearSkimmer();
            endHoverSeek();
        });
        connect(m_viewer, &ViewerWidget::seekRequested, this, [this](double fraction) {
            if (m_duration <= 0.0) return;
            queueSeek(fraction * m_duration);
        });
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
        auto *focusAction = edit->addAction(QStringLiteral("Focus region…"));
        connect(focusAction, &QAction::triggered, this, &MainWindow::focusRegion);

        auto *playback = menuBar()->addMenu(QStringLiteral("Playback"));
        auto *play = playback->addAction(QStringLiteral("Play / Pause"));
        play->setShortcut(Qt::Key_Space);
        play->setShortcutContext(Qt::WidgetShortcut);
        m_timeline->addAction(play);
        connect(play, &QAction::triggered, this, &MainWindow::togglePlayback);

        auto *toolbar = addToolBar(QStringLiteral("Main"));
        toolbar->setMovable(false);
        toolbar->setIconSize(QSize(15, 15));
        toolbar->setFixedHeight(34);
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
        m_quality->setSizeAdjustPolicy(QComboBox::AdjustToContents);
        m_quality->setToolTip(QStringLiteral("Export quality"));
        m_quality->addItem(QStringLiteral("Preview"), QStringLiteral("preview"));
        m_quality->addItem(QStringLiteral("Full"), QStringLiteral("full"));
        toolbar->addWidget(m_quality);
        m_canvasPreset = new QualityComboBox;
        m_canvasPreset->setObjectName(QStringLiteral("canvasPicker"));
        m_canvasPreset->setSizeAdjustPolicy(QComboBox::AdjustToContents);
        m_canvasPreset->addItem(QStringLiteral("16:9"), QStringLiteral("1920x1080"));
        m_canvasPreset->addItem(QStringLiteral("9:16"), QStringLiteral("1080x1920"));
        m_canvasPreset->setToolTip(QStringLiteral("Project format"));
        connect(m_canvasPreset, qOverload<int>(&QComboBox::currentIndexChanged),
                this, &MainWindow::canvasPresetChanged);
        toolbar->addWidget(m_canvasPreset);
        auto *exportToolbar = toolbar->addAction(QStringLiteral("Export"));
        exportToolbar->setToolTip(QStringLiteral("Export the timeline"));
        connect(exportToolbar, &QAction::triggered, this, &MainWindow::exportProject);
        if (auto *exportButton = qobject_cast<QToolButton *>(toolbar->widgetForAction(exportToolbar)))
            exportButton->setObjectName(QStringLiteral("toolbarPrimary"));
        m_previewButton = toolbar->addAction(QStringLiteral("Render"));
        m_previewButton->setToolTip(QStringLiteral("Render a preview of the timeline"));
        m_previewButton->setEnabled(false);
        connect(m_previewButton, &QAction::triggered, this, &MainWindow::renderPreview);
        if (auto *renderButton = qobject_cast<QToolButton *>(toolbar->widgetForAction(m_previewButton)))
            renderButton->setObjectName(QStringLiteral("toolbarQuiet"));
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

    bool sendSeek(double position, bool scrub) {
        if (!m_connected || !m_bridge) return false;
        const QJsonObject payload{{QStringLiteral("type"), QStringLiteral("seek")},
                                  {QStringLiteral("position"), position},
                                  {QStringLiteral("scrub"), scrub}};
        const QByteArray bytes = QJsonDocument(payload).toJson(QJsonDocument::Compact);
        return cut_bridge_send(m_bridge, bytes.constData());
    }

    void flushSeek() {
        if (m_hasPendingSeek) {
            if (!sendSeek(m_pendingSeek, m_pendingSeekScrub)) return;
            m_hasPendingSeek = false;
            return;
        }
        if (m_hasPendingHoverSeek) {
            if (!sendSeek(m_pendingHoverSeek, m_pendingHoverScrub)) return;
            m_hasPendingHoverSeek = false;
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
            m_hasShownFrame = false;
        } else if (type == QStringLiteral("error")) {
            showError(event.value(QStringLiteral("message")).toString(QStringLiteral("Runtime error")));
        } else if (type == QStringLiteral("agent")) {
            const QString kind = event.value(QStringLiteral("kind")).toString(QStringLiteral("agent"));
            const QString text = event.value(QStringLiteral("text")).toString();
            if (kind == QStringLiteral("delta")) {
                if (m_agentText.isEmpty()) appendConversation(QStringLiteral("Cut"), text);
                else {
                    const bool follow = m_conversation->verticalScrollBar()->value() >= m_conversation->verticalScrollBar()->maximum() - 4;
                    m_agentCursor.insertText(text);
                    if (follow) m_conversation->verticalScrollBar()->setValue(m_conversation->verticalScrollBar()->maximum());
                }
                m_agentText += text;
            } else if (kind == QStringLiteral("done")) {
                if (m_agentText.isEmpty()) appendConversation(QStringLiteral("Cut"), text);
                m_agentText.clear();
                appendConversationNote(QStringLiteral("Progress"), QStringLiteral("Response complete"));
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
            if (!m_hoverSkimming && !m_pointerOwnsPlayhead) {
                m_playhead = std::clamp(event.value(QStringLiteral("position")).toDouble(), 0.0, m_duration);
                m_timeline->setPlayhead(m_playhead);
            }
        } else if (type == QStringLiteral("playback")) {
            m_playing = m_connected && event.value(QStringLiteral("playing")).toBool();
            if (m_playing) {
                m_hasPendingHoverSeek = false;
                m_hoverSkimming = false;
                m_timeline->clearSkimmer();
            }
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
                m_viewer->clearFrame();
                const QString detail = message.isEmpty() ? QStringLiteral("Rendered preview required") : message;
                statusBar()->setToolTip(detail);
                statusBar()->showMessage(detail.left(112) + (detail.size() > 112 ? QStringLiteral("…") : QString()), 6000);
                addActivity(QStringLiteral("preview"), message.isEmpty() ? QStringLiteral("Native preview unavailable") : message);
            } else if (status == QStringLiteral("running")) {
                m_previewRequired = true;
                m_previewButton->setEnabled(false);
                m_exportProgress->setValue(static_cast<int>(std::clamp(event.value(QStringLiteral("progress")).toDouble(), 0.0, 1.0) * 100.0));
                m_exportProgress->show();
            } else if (status == QStringLiteral("ready")) {
                m_previewRequired = false;
                m_previewButton->setEnabled(false);
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
        updateProposal(snapshot.value(QStringLiteral("pendingProposal")));
        updateInspector();
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
        m_adjustCropButton->setEnabled(false);
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
        m_adjustCropButton->setEnabled(m_connected && !stale && !truncated && !proposalCropEdit(proposal).isEmpty());
        m_acceptButton->setToolTip(truncated ? QStringLiteral("Split this proposal into smaller changes to review all fields") : QString());
    }

    void appendConversation(const QString &speaker, const QString &text) {
        if (text.isEmpty()) return;
        const bool follow = m_conversationEmpty || m_conversation->verticalScrollBar()->value() >= m_conversation->verticalScrollBar()->maximum() - 4;
        if (m_conversationEmpty) {
            m_conversation->clear();
            m_conversationEmpty = false;
        }
        const bool user = speaker == QStringLiteral("You");
        const bool error = speaker == QStringLiteral("Error");
        // Native text frames provide actual padding and separate message regions.
        // Keep the streaming cursor in its frame even when another event arrives.
        QTextCursor cursor = m_conversation->document()->rootFrame()->lastCursorPosition();
        QTextFrameFormat frame;
        frame.setPadding(12);
        frame.setTopMargin(6);
        frame.setBottomMargin(10);
        frame.setLeftMargin(user ? 32 : 0);
        frame.setRightMargin(user ? 0 : 16);
        frame.setBackground(QColor(user ? "#333333" : error ? "#342729" : "#262626"));
        frame.setBorder(1);
        frame.setBorderStyle(QTextFrameFormat::BorderStyle_Solid);
        frame.setBorderBrush(QColor(error ? "#94666b" : user ? "#494949" : "#353535"));
        cursor = cursor.insertFrame(frame)->firstCursorPosition();
        QTextBlockFormat block;
        block.setBottomMargin(7);
        cursor.setBlockFormat(block);
        QTextCharFormat role;
        role.setFontWeight(QFont::DemiBold);
        role.setFontPointSize(9);
        role.setForeground(QColor(error ? "#efb3b8" : user ? "#bfbfbf" : "#e6b450"));
        cursor.insertText(user ? QStringLiteral("YOU") : error ? QStringLiteral("COULDN’T COMPLETE EDIT") : QStringLiteral("CUT"), role);
        QTextBlockFormat body;
        body.setLineHeight(145, QTextBlockFormat::ProportionalHeight);
        QTextCharFormat bodyText;
        bodyText.setForeground(QColor("#e4e4e4"));
        cursor.insertBlock(body, bodyText);
        cursor.insertText(text);
        if (speaker == QStringLiteral("Cut")) m_agentCursor = cursor;
        if (follow) m_conversation->verticalScrollBar()->setValue(m_conversation->verticalScrollBar()->maximum());
    }

    void appendConversationNote(const QString &kind, const QString &text) {
        if (text.isEmpty()) return;
        if (kind == QStringLiteral("Agent error")) {
            appendConversation(QStringLiteral("Error"), text);
            return;
        }
        const QString activity = QStringLiteral("%1 · %2").arg(kind, text.simplified());
        m_chatActivity->setText(activity);
        m_chatActivity->setToolTip(activity);
        m_chatActivity->show();
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
        m_timeline->setEnabled(m_connected);
        if (m_canvasPreset) m_canvasPreset->setEnabled(m_connected);
        m_chatInput->setEnabled(m_connected && !m_agentBusy);
        for (auto *action : m_runtimeActions) action->setEnabled(m_connected);
        m_undoAction->setEnabled(m_connected && m_snapshot.value(QStringLiteral("canUndo")).toBool());
        m_redoAction->setEnabled(m_connected && m_snapshot.value(QStringLiteral("canRedo")).toBool());
        m_previewButton->setEnabled(m_connected && m_previewRequired);
        if (!m_connected) {
            m_playing = false;
            m_hasPendingSeek = false;
            m_hasPendingHoverSeek = false;
            m_hoverSkimming = false;
            m_pointerOwnsPlayhead = false;
            m_hasShownFrame = false;
            m_exactSeekTimer.stop();
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

    MediaArtwork *m_artwork = nullptr;
    QLabel *m_projectTitle = nullptr;
    QString m_projectDirectory;
    QString m_cutdBinary;
    void *m_bridge = nullptr;
    QTimer m_pollTimer;
    QTimer m_exactSeekTimer;
    QJsonObject m_snapshot;
    QJsonObject m_project;
    bool m_connected = false;
    QFrame *m_connectionBanner = nullptr;
    QLabel *m_connectionMessage = nullptr;
    QPushButton *m_reconnectButton = nullptr;
    QFrame *m_errorBanner = nullptr;
    QLabel *m_errorMessage = nullptr;
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
    double m_shownFrameTime = 0.0;
    bool m_hasPendingSeek = false;
    bool m_hasPendingHoverSeek = false;
    bool m_pendingSeekScrub = false;
    bool m_pendingHoverScrub = false;
    bool m_playing = false;
    bool m_previewRequired = false;
    bool m_hoverSkimming = false;
    bool m_pointerOwnsPlayhead = false;
    bool m_hasShownFrame = false;
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
    QTextEdit *m_conversation = nullptr;
    QTextCursor m_agentCursor;
    QLabel *m_chatActivity = nullptr;
    QFrame *m_chatComposer = nullptr;
    QLabel *m_composerStatus = nullptr;
    QTextEdit *m_chatInput = nullptr;
    QRadioButton *m_anthropicActiveRadio = nullptr;
    QRadioButton *m_openaiActiveRadio = nullptr;
    QComboBox *m_modelSelect = nullptr;
    QLineEdit *m_anthropicKey = nullptr;
    QLineEdit *m_anthropicBaseUrl = nullptr;
    QLineEdit *m_anthropicModel = nullptr;
    QLineEdit *m_openaiKey = nullptr;
    QLineEdit *m_openaiBaseUrl = nullptr;
    QLineEdit *m_openaiModel = nullptr;
    QComboBox *m_openaiMode = nullptr;
    QCheckBox *m_mcpEnabled = nullptr;
    QTextEdit *m_inboundMcpJson = nullptr;
    QLabel *m_inboundMcpStatus = nullptr;
    quint64 m_mcpConfigVersion = 0;
    QPushButton *m_sendButton = nullptr;
    QToolButton *m_abortButton = nullptr;
    QListWidget *m_activityList = nullptr;
    QLabel *m_agentStatus = nullptr;
    QLabel *m_proposalSummary = nullptr;
    QTextEdit *m_proposalDiff = nullptr;
    QPushButton *m_acceptButton = nullptr;
    QPushButton *m_adjustCropButton = nullptr;
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
    void initTestCase() {
        QApplication::setStyle(QStringLiteral("Fusion"));
        qApp->setStyleSheet(editorStyleSheet());
    }

    void inboundMcpConfigValidatesStdioShape() {
        QVERIFY(!validateInboundMcpConfig("[]").isEmpty());
        QVERIFY(!validateInboundMcpConfig("{}").isEmpty());
        QVERIFY(!validateInboundMcpConfig("{broken").isEmpty());
        const QByteArray remote = R"({"mcpServers":{"remote":{"url":"https://example.com/mcp"}}})";
        const QByteArray emptyCommand = R"({"mcpServers":{"local":{"command":" "}}})";
        const QByteArray invalidArgs = R"({"mcpServers":{"local":{"command":"server","args":[1]}}})";
        const QByteArray invalidEnv = R"({"mcpServers":{"local":{"command":"server","env":{"TOKEN":1}}}})";
        const QByteArray valid = R"({"mcpServers":{"local":{"command":"server","args":["--stdio"],"env":{"TOKEN":"value"},"tools":["search"]}}})";
        QVERIFY(validateInboundMcpConfig(remote).contains(QStringLiteral("URL-only")));
        QVERIFY(!validateInboundMcpConfig(emptyCommand).isEmpty());
        QVERIFY(!validateInboundMcpConfig(invalidArgs).isEmpty());
        QVERIFY(!validateInboundMcpConfig(invalidEnv).isEmpty());
        QVERIFY(validateInboundMcpConfig(valid).isEmpty());
    }

    void inboundMcpImportRequiresSaveAndInvalidSavePreservesConfig() {
        QTemporaryDir project;
        QVERIFY(project.isValid());
        QVERIFY(QDir(project.path()).mkpath(QStringLiteral(".cursor")));
        const QByteArray config = R"({"mcpServers":{"local":{"command":"server","args":["--stdio"]}}})";
        QFile source(QDir(project.path()).filePath(QStringLiteral(".cursor/mcp.json")));
        QVERIFY(source.open(QIODevice::WriteOnly));
        QCOMPARE(source.write(config), qint64(config.size()));
        source.close();
        const QString savedPath = QDir(project.path()).filePath(QStringLiteral(".cut/mcp.json"));
        MainWindow window{project.path(), QString()};
        const bool originalEnabled = window.m_mcpEnabled->isChecked();
        window.m_mcpEnabled->setChecked(!originalEnabled);
        QCOMPARE(QSettings().value(QStringLiteral("agent/mcpEnabled")).toBool(), !originalEnabled);
        QCOMPARE(window.agentSettingsPayload().value(QStringLiteral("mcpEnabled")).toBool(), !originalEnabled);
        window.m_mcpEnabled->setChecked(originalEnabled);
        QCOMPARE(window.agentSettingsPayload().value(QStringLiteral("mcpConfigVersion")).toInteger(), qint64(0));
        auto *load = window.findChild<QPushButton *>(QStringLiteral("loadProjectCursorMcp"));
        QVERIFY(load);
        QVERIFY(window.findChild<QPushButton *>(QStringLiteral("loadHomeCursorMcp")));
        load->click();
        QCOMPARE(window.m_inboundMcpJson->toPlainText().toUtf8(), config);
        QVERIFY(!QFileInfo::exists(savedPath));
        auto *save = window.findChild<QPushButton *>(QStringLiteral("saveInboundMcpConfig"));
        QVERIFY(save);
        save->click();
        QCOMPARE(window.agentSettingsPayload().value(QStringLiteral("mcpConfigVersion")).toInteger(), qint64(1));
        QFile saved(savedPath);
        QVERIFY(saved.open(QIODevice::ReadOnly));
        QCOMPARE(saved.readAll(), config);
        saved.close();
        window.m_inboundMcpJson->setPlainText(QStringLiteral("[]"));
        save->click();
        QCOMPARE(window.agentSettingsPayload().value(QStringLiteral("mcpConfigVersion")).toInteger(), qint64(1));
        QVERIFY(saved.open(QIODevice::ReadOnly));
        QCOMPARE(saved.readAll(), config);
        MainWindow reopened{project.path(), QString()};
        QCOMPARE(reopened.m_inboundMcpJson->toPlainText().toUtf8(), config);
    }

    void mcpConfigCopiesProjectWithoutCredentials() {
        const QString projectPath = QDir::temp().filePath(QStringLiteral("Cut project \"quoted\""));
        MainWindow window{projectPath, QString()};
        auto *section = window.findChild<QGroupBox *>(QStringLiteral("mcpSection"));
        QVERIFY(section);
        auto *editor = section->findChild<QTextEdit *>(QStringLiteral("mcpConfigJson"));
        auto *button = section->findChild<QPushButton *>(QStringLiteral("copyMcpConfig"));
        QVERIFY(editor);
        QVERIFY(button);
        QVERIFY(editor->isReadOnly());
        const QJsonObject expected{{QStringLiteral("mcpServers"), QJsonObject{
            {QStringLiteral("cut"), QJsonObject{
                {QStringLiteral("command"), mcpExecutable(projectPath)},
                {QStringLiteral("args"), QJsonArray{QStringLiteral("--dir"), projectPath}}
            }}}}};
        QCOMPARE(QJsonDocument::fromJson(editor->toPlainText().toUtf8()).object(), expected);
        button->click();
        QCOMPARE(QApplication::clipboard()->text(), editor->toPlainText());
        QCOMPARE(section->findChild<QLabel *>(QStringLiteral("muted"))->text(),
                 QStringLiteral("MCP config copied"));
    }

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

        viewer.resize(800, 220);
        viewer.setProjectInfo(1920, 1080);
        QVERIFY(viewer.videoRect().top() < 20.0);
        viewer.show();
        QSignalSpy scrubs(&viewer, &ViewerWidget::scrubbed);
        QTest::mouseMove(&viewer, QPoint(400, 80));
        QVERIFY(!scrubs.isEmpty());
        QCOMPARE(scrubs.constLast()[0].toDouble(), 0.5);
        QVERIFY(nearerScrubFrame(1.0, true, 2.0, 3.0, 24.0));
        QVERIFY(!nearerScrubFrame(2.5, true, 0.2, 3.0, 24.0));
        QVERIFY(nearerScrubFrame(0.0, false, 9.0, 1.0, 24.0));
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

    void agentPayloadFollowsSettingsProtocol() {
        MainWindow window{QString(), QString()};
        QVERIFY(!window.findChild<QComboBox *>(QStringLiteral("agentProvider")));
        QVERIFY(!window.findChild<QFrame *>(QStringLiteral("agentContext")));
        QVERIFY(!window.findChild<QLabel *>(QStringLiteral("agentConfig")));
        QVERIFY(window.findChild<QComboBox *>(QStringLiteral("agentModel")));
        auto *tabs = window.findChild<QTabWidget *>(QStringLiteral("rightTabs"));
        QVERIFY(tabs);
        QWidget *chatTab = tabs->widget(0);
        QVERIFY(chatTab);
        for (auto *label : chatTab->findChildren<QLabel *>()) {
            QVERIFY(!label->text().contains(QStringLiteral("OpenAI")));
            QVERIFY(!label->text().contains(QStringLiteral("Anthropic")));
        }
        auto *anthropicRadio = window.findChild<QRadioButton *>(QStringLiteral("agentActiveAnthropic"));
        auto *openaiRadio = window.findChild<QRadioButton *>(QStringLiteral("agentActiveOpenai"));
        QVERIFY(anthropicRadio);
        QVERIFY(openaiRadio);
        const QSignalBlocker anthropicBlock(anthropicRadio);
        const QSignalBlocker openaiBlock(openaiRadio);

        const QString anthropicKey = QStringLiteral("sk-ant-test");
        const QString anthropicUrl = QStringLiteral("https://anthropic.example/v1");
        const QString anthropicModel = QStringLiteral("claude-test-model");
        anthropicRadio->setChecked(true);
        window.m_anthropicKey->setText(anthropicKey);
        window.m_anthropicBaseUrl->setText(anthropicUrl);
        window.m_anthropicModel->setText(anthropicModel);
        window.updateAgentModelChoices();
        const QJsonObject anthropicPayload = window.agentSettingsPayload();
        QCOMPARE(anthropicPayload.value(QStringLiteral("provider")).toString(), QStringLiteral("anthropic"));
        QCOMPARE(anthropicPayload.value(QStringLiteral("apiKey")).toString(), anthropicKey);
        QCOMPARE(anthropicPayload.value(QStringLiteral("baseUrl")).toString(), anthropicUrl);
        QCOMPARE(anthropicPayload.value(QStringLiteral("model")).toString(), anthropicModel);

        const QString openaiKey = QStringLiteral("sk-openai-test");
        const QString openaiUrl = QStringLiteral("https://openai.example/v1");
        const QString openaiModel = QStringLiteral("gpt-test-model");
        openaiRadio->setChecked(true);
        window.m_openaiKey->setText(openaiKey);
        window.m_openaiBaseUrl->setText(openaiUrl);
        window.m_openaiModel->setText(openaiModel);
        window.updateAgentModelChoices();
        const QJsonObject openaiPayload = window.agentSettingsPayload();
        QCOMPARE(openaiPayload.value(QStringLiteral("provider")).toString(), QStringLiteral("openai"));
        QCOMPARE(openaiPayload.value(QStringLiteral("apiKey")).toString(), openaiKey);
        QCOMPARE(openaiPayload.value(QStringLiteral("baseUrl")).toString(), openaiUrl);
        QCOMPARE(openaiPayload.value(QStringLiteral("model")).toString(), openaiModel);
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

    void conversationSeparatesActivityAndKeepsStreamInItsMessage() {
        MainWindow window{QString(), QString()};
        window.show();
        QVERIFY(window.m_conversation->toPlainText().contains(QStringLiteral("Make your next cut.")));
        const QString emptyState = window.m_conversation->toPlainText();
        window.appendConversationNote(QStringLiteral("Tool"), QStringLiteral("Inspecting timeline"));
        QCOMPARE(window.m_conversation->toPlainText(), emptyState);
        QVERIFY(!window.m_chatActivity->isHidden());

        window.appendConversation(QStringLiteral("You"), QStringLiteral("Trim <intro>\nKeep the dialogue."));
        window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "delta"}, {"text", "I’ll trim "}});
        window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "tool"}, {"text", "Reading <timeline>"}});
        window.handleEvent(QJsonObject{{"type", "agent_state"}, {"state", "failed"}, {"message", "Could not read <media>"}});
        window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "delta"}, {"text", "the opening."}});
        window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "done"}, {"text", "I’ll trim the opening."}});

        const QString transcript = window.m_conversation->toPlainText();
        QVERIFY(!transcript.contains(QStringLiteral("Make your next cut.")));
        QVERIFY(transcript.contains(QStringLiteral("Trim <intro>\nKeep the dialogue.")));
        QCOMPARE(transcript.count(QStringLiteral("I’ll trim the opening.")), 1);
        QVERIFY(transcript.contains(QStringLiteral("Could not read <media>")));
        QVERIFY(!transcript.contains(QStringLiteral("Reading <timeline>")));
        QCOMPARE(window.m_activityList->count(), 1);
        const auto frames = window.m_conversation->document()->rootFrame()->childFrames();
        QCOMPARE(frames.size(), 3);
        QVERIFY(frames[0]->frameFormat().leftMargin() > frames[1]->frameFormat().leftMargin());
        QVERIFY(frames[1]->frameFormat().borderBrush() != frames[2]->frameFormat().borderBrush());
        window.appendConversation(QStringLiteral("Cut"), QStringLiteral("A separate response."));
        QCOMPARE(window.m_conversation->document()->rootFrame()->childFrames().size(), 4);

        // New output must not pull the reader away from earlier messages.
        for (int i = 0; i < 12; ++i)
            window.appendConversation(QStringLiteral("You"), QStringLiteral("A longer edit request.\nKeep this line readable."));
        QCoreApplication::processEvents();
        auto *scroll = window.m_conversation->verticalScrollBar();
        QVERIFY(scroll->maximum() > 0);
        scroll->setValue(0);
        window.appendConversation(QStringLiteral("Cut"), QStringLiteral("New response."));
        QCOMPARE(scroll->value(), 0);
    }

    void composerStatesAndNarrowLayoutRemainUsable() {
        MainWindow window{QString(), QString()};
        window.resize(1080, 700);
        window.show();
        QCOMPARE(window.m_chatComposer->property("state").toString(), QStringLiteral("offline"));
        window.setConnection(QStringLiteral("ready"));
        window.activateWindow();
        window.m_chatInput->setFocus();
        QTRY_COMPARE(window.m_chatComposer->property("state").toString(), QStringLiteral("focused"));
        window.m_chatInput->setPlainText(QStringLiteral("Keep my draft"));
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", true}});
        QCOMPARE(window.m_chatComposer->property("state").toString(), QStringLiteral("sending"));
        QVERIFY(!window.m_chatInput->isEnabled());
        QVERIFY(window.m_abortButton->isVisible());
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", false}});
        QVERIFY(window.m_chatInput->isEnabled());
        QVERIFY(window.m_sendButton->isEnabled());
        QCOMPARE(window.m_chatInput->toPlainText(), QStringLiteral("Keep my draft"));
        window.appendConversation(QStringLiteral("You"), QStringLiteral("Trim the opening to three seconds."));
        window.appendConversation(QStringLiteral("Cut"), QStringLiteral("I’ll shorten the opening and keep the rest of your sequence in place.\nReview the changes before applying them."));
        window.appendConversationNote(QStringLiteral("Tool"), QString(500, QLatin1Char('x')));
        window.updateProposal(QJsonObject{{"status", "pending"}, {"proposalId", "review"},
            {"diff", QJsonArray{QJsonObject{{"path", "$.name"}, {"before", "old"}, {"after", "new"}}}}});
        window.m_activityList->show();
        QCoreApplication::processEvents();
        QVERIFY(window.m_chatInput->height() <= 36);
        auto *chat = window.m_chatComposer->parentWidget();
        QVERIFY(chat->rect().contains(window.m_chatComposer->geometry()));
        QCOMPARE(window.m_chatComposer->geometry().bottom(), chat->rect().bottom());
        QVERIFY(window.m_conversation->height() >= 80);
        QCOMPARE(window.m_conversation->horizontalScrollBar()->maximum(), 0);
        QVERIFY(window.m_proposalBox->rect().contains(window.m_proposalDiff->geometry()));
        QVERIFY(window.m_proposalDiff->geometry().bottom() < window.m_acceptButton->geometry().top());
        QVERIFY(window.m_proposalBox->rect().contains(window.m_acceptButton->geometry()));
        auto *reviewScroll = window.m_proposalBox->parentWidget()->parentWidget()->parentWidget();
        QVERIFY(reviewScroll->mapTo(chat, QPoint(0, reviewScroll->height())).y() <= window.m_chatComposer->y());
        // Optional artifact for manual review of the real styled widget tree.
        const QString screenshot = qEnvironmentVariable("CUT_QT_UI_SCREENSHOT");
        if (!screenshot.isEmpty()) {
            QVERIFY(window.grab().save(screenshot));
            window.resize(1500, 940);
            window.m_proposalBox->hide();
            window.m_activityList->hide();
            window.appendConversationNote(QStringLiteral("Progress"), QStringLiteral("Response complete"));
            QCoreApplication::processEvents();
            QVERIFY(window.grab().save(screenshot + QStringLiteral(".chat.png")));
        }
    }

    void timelineFillsTheRemovedSourceRow() {
        MainWindow window{QString(), QString()};
        window.resize(1280, 800);
        window.show();
        QTest::qWait(30);
        QVERIFY(window.findChild<QWidget *>(QStringLiteral("sourceTray")) == nullptr);
        const int viewerBottom = window.m_viewer->mapTo(&window, QPoint(0, window.m_viewer->height())).y();
        const int timelineTop = window.m_timelineScroll->mapTo(&window, QPoint(0, 0)).y();
        QVERIFY(timelineTop >= viewerBottom - 4);
        QVERIFY(timelineTop <= viewerBottom + 8);
        QVERIFY(window.m_timelineScroll->height() >= 280);
        const QString screenshot = qEnvironmentVariable("CUT_QT_UI_SCREENSHOT");
        if (!screenshot.isEmpty()) {
            window.setConnection(QStringLiteral("ready"));
            window.applySnapshot(QJsonObject{{"duration", 8}, {"project", QJsonObject{
                {"name", "Cut"}, {"fps", 24}, {"tracks", QJsonArray{}}
            }}});
            QVERIFY(window.grab().save(screenshot + QStringLiteral(".empty.png")));
            const QString sample = qEnvironmentVariable("CUT_QT_UI_SAMPLE_VIDEO");
            if (!sample.isEmpty()) {
                const QJsonArray sources{QJsonObject{{"id", "sample"}, {"path", sample}}};
                window.applySnapshot(QJsonObject{{"duration", 4}, {"sources", sources}, {"project", QJsonObject{
                    {"name", "Cut"}, {"fps", 24},
                    {"sources", sources},
                    {"tracks", QJsonArray{QJsonObject{{"type", "video"}, {"clips", QJsonArray{
                        QJsonObject{{"id", "sample-clip"}, {"sourceId", "sample"}, {"timelineStart", 0},
                                    {"in", 0}, {"out", 4}}}}}}}
                }}});
                QTRY_VERIFY_WITH_TIMEOUT(window.m_artwork->frames.contains(QStringLiteral("sample")), 5000);
                window.m_viewer->setFrame(window.m_artwork->frames.value(QStringLiteral("sample")), 0);
                QVERIFY(window.grab().save(screenshot));
            }
        }
    }

    void timelineKeepsSeekSelectMoveAndTrimCoordinates() {
        TimelineWidget timeline;
        timeline.resize(700, 240);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{QJsonObject{{"type", "video"}, {"clips", QJsonArray{
            QJsonObject{{"id", "clip"}, {"timelineStart", 1.0}, {"duration", 4.0}, {"name", "Opening"}}
        }}}}}}, 8, 24);
        timeline.show();
        QSignalSpy seeks(&timeline, &TimelineWidget::seekRequested);
        QSignalSpy selections(&timeline, &TimelineWidget::clipSelected);
        QSignalSpy moves(&timeline, &TimelineWidget::moveRequested);
        QSignalSpy trims(&timeline, &TimelineWidget::trimRequested);
        QTest::mouseClick(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(kTimelineLabelWidth + 36, kTrackHeight / 2));
        QCOMPARE(seeks.constLast()[0].toDouble(), 0.5);
        const int y = kRulerHeight + kTrackHeight / 2;
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(kTimelineLabelWidth + 144, y));
        QTest::mouseMove(&timeline, QPoint(kTimelineLabelWidth + 216, y));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(kTimelineLabelWidth + 216, y));
        QCOMPARE(selections.constLast()[0].toString(), QStringLiteral("clip"));
        QCOMPARE(moves.count(), 1);
        QCOMPARE(moves[0][1].toDouble(), 2.0);
        for (const bool leading : {true, false}) {
            const int edge = kTimelineLabelWidth + (leading ? 75 : 357);
            QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(edge, y));
            QTest::mouseMove(&timeline, QPoint(edge + 72, y));
            QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(edge + 72, y));
            QCOMPARE(trims.constLast()[1].toBool(), leading);
            QCOMPARE(trims.constLast()[2].toDouble(), double(std::llround((edge + 72 - kTimelineLabelWidth) / 72.0 * 24)));
        }
        QCOMPARE(trims.count(), 2);
        timeline.setSelectedClip(QStringLiteral("clip"));
        timeline.setPlayhead(2.5);
        const QString screenshot = qEnvironmentVariable("CUT_QT_UI_SCREENSHOT");
        if (!screenshot.isEmpty()) QVERIFY(timeline.grab().save(screenshot + QStringLiteral(".timeline.png")));
    }

    void timelineRevealsAddLaneFromTheHeader() {
        TimelineWidget timeline;
        timeline.resize(700, 240);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{}}}, 8, 24);
        timeline.show();
        QSignalSpy adds(&timeline, &TimelineWidget::addLaneRequested);
        QSignalSpy seeks(&timeline, &TimelineWidget::seekRequested);
        QVERIFY(!timeline.addLaneVisible());
        QTest::mouseMove(&timeline, timeline.addLaneButtonCenter());
        QVERIFY(timeline.addLaneVisible());
        QTest::mouseClick(&timeline, Qt::LeftButton, Qt::NoModifier, timeline.addLaneButtonCenter());
        QCOMPARE(adds.count(), 1);
        QVERIFY(!timeline.addLaneVisible());
        QCOMPARE(seeks.count(), 0);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{QJsonObject{{"type", "audio"}, {"clips", QJsonArray{}}}}}}, 8, 24);
        QTest::mouseMove(&timeline, timeline.addLaneButtonCenter());
        QVERIFY(timeline.addLaneVisible());
        QCOMPARE(timeline.addLaneButtonCenter().y(), kRulerHeight + kTrackHeight + (kTrackHeight - 1) / 2);
    }

    void timelineHeaderShowsVideoPreview() {
        MediaArtwork artwork;
        TimelineWidget timeline;
        timeline.resize(700, 240);
        timeline.setArtwork(&artwork);
        const QJsonObject clip{{"sourceId", "sample"}, {"timelineStart", 0}, {"duration", 2}};
        const QJsonObject track{{"type", "video"}, {"clips", QJsonArray{clip}}};
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{track}}}, 4, 24);
        timeline.show();
        const QPoint thumbnail(45, kRulerHeight + kTrackHeight / 2);
        QCOMPARE(timeline.grab().toImage().pixelColor(thumbnail), QColor("#1c1c1c"));
        QImage frame(160, 90, QImage::Format_RGB32);
        frame.fill(QColor("#e87342"));
        artwork.frames.insert(QStringLiteral("sample"), frame);
        emit artwork.changed();
        QCOMPARE(timeline.grab().toImage().pixelColor(thumbnail), QColor("#e87342"));
    }

    void timelineScrubsFromThePointer() {
        TimelineWidget timeline;
        timeline.resize(700, 240);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{QJsonObject{{"type", "video"}, {"clips", QJsonArray{
            QJsonObject{{"id", "clip"}, {"timelineStart", 1.0}, {"duration", 4.0}, {"name", "Opening"}}
        }}}}}}, 8, 24);
        timeline.show();
        QSignalSpy hovers(&timeline, &TimelineWidget::hoverSeekRequested);
        QTest::mouseMove(&timeline, QPoint(kTimelineLabelWidth + 144, kTrackHeight / 2));
        QVERIFY(!hovers.isEmpty());
        QCOMPARE(hovers.constLast()[0].toDouble(), 2.0);
        QCOMPARE(timeline.skimLabel(), QStringLiteral("00:02.00"));
        QTest::mouseMove(&timeline, QPoint(kTimelineLabelWidth + 36, kTrackHeight / 2));
        QCOMPARE(timeline.skimLabel(), QStringLiteral("00:00.50"));
        QCOMPARE(hovers.constLast()[0].toDouble(), 0.5);
        QTest::mouseMove(&timeline, QPoint(20, kTrackHeight / 2));
        QVERIFY(timeline.skimLabel().isEmpty());
    }

    void laneCacheBlitKeepsLogicalPixelsAtRetinaScale() {
        constexpr qreal dpr = 2;
        constexpr int width = 480;
        constexpr int height = 90;
        QPixmap cache(width * dpr, height * dpr);
        cache.setDevicePixelRatio(dpr);
        cache.fill(QColor("#141414"));
        {
            QPainter painter(&cache);
            painter.fillRect(QRect(0, 0, width, kRulerHeight), QColor("#2a2a2a"));
            painter.fillRect(QRect(120, 40, 40, 16), QColor("#3d7eff"));
        }

        const auto at = [](const QImage &image, int x, int y) {
            return image.pixelColor(QPoint(x * int(dpr), y * int(dpr)));
        };
        QImage destination(width * dpr, height * dpr, QImage::Format_ARGB32_Premultiplied);
        destination.setDevicePixelRatio(dpr);
        destination.fill(Qt::red);
        {
            QPainter painter(&destination);
            blitLaneCache(painter, cache, QRegion(QRect(0, 0, width, height)));
        }
        QCOMPARE(at(destination, width - 8, 6).red(), 0x2a);
        QCOMPARE(at(destination, 140, 48).blue(), 0xff);

        destination.fill(Qt::red);
        {
            QPainter painter(&destination);
            blitLaneCache(painter, cache, QRegion(QRect(300, 0, 100, height)));
        }
        QCOMPARE(at(destination, 340, 6).red(), 0x2a);
        QCOMPARE(at(destination, 20, 6).red(), 255);
    }

    void timelinePointerTimeAdvancesEveryFewPixels() {
        TimelineWidget timeline;
        timeline.resize(900, 280);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{QJsonObject{{"type", "video"}, {"clips", QJsonArray{}}}}}}, 90, 24);
        timeline.fitToWidth(900);
        timeline.show();
        QTest::qWait(20);
        const auto seconds = [](const QString &label) {
            const QStringList clock = label.split(QLatin1Char(':'));
            const QStringList sec = clock.value(1).split(QLatin1Char('.'));
            return clock.value(0).toDouble() * 60.0 + sec.value(0).toDouble() + sec.value(1).toDouble() / 100.0;
        };
        const int y = kRulerHeight + kTrackHeight / 2;
        QTest::mouseMove(&timeline, QPoint(kTimelineLabelWidth + 160, y));
        const double start = seconds(timeline.skimLabel());
        QTest::mouseMove(&timeline, QPoint(kTimelineLabelWidth + 168, y));
        const double next = seconds(timeline.skimLabel());
        QVERIFY(std::abs((next - start) - 0.1) < 0.02);
        const QString screenshot = qEnvironmentVariable("CUT_QT_UI_SCREENSHOT");
        if (!screenshot.isEmpty()) {
            timeline.setProject(QJsonObject{{"tracks", QJsonArray{
                QJsonObject{{"type", "video"}, {"name", "Video 1"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "opening"}, {"name", "Interview"}, {"timelineStart", 0.4}, {"duration", 3.2}},
                    QJsonObject{{"id", "broll"}, {"name", "B-roll"}, {"timelineStart", 3.8}, {"duration", 2.6}},
                }}},
                QJsonObject{{"type", "audio"}, {"name", "Dialogue"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "dial"}, {"name", "Dialogue"}, {"timelineStart", 0.2}, {"duration", 7.2}},
                }}},
            }}}, 8, 24);
            timeline.fitToWidth(980);
            timeline.resize(980, 220);
            timeline.setSelectedClip(QStringLiteral("opening"));
            timeline.setPlayhead(3.4);
            timeline.showSkimmer(5.75);
            QTest::qWait(20);
            QVERIFY(timeline.grab().save(screenshot + QStringLiteral(".timeline.png")));
        }
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

    void selectedBoxTracksMotionAndStopsAtSceneCut() {
        if (QStandardPaths::findExecutable(QStringLiteral("ffmpeg")).isEmpty()) QSKIP("FFmpeg unavailable");
        QTemporaryDir directory;
        QVERIFY(directory.isValid());
        for (int index = 0; index < 8; ++index) {
            QImage frame(160, 90, QImage::Format_RGB888);
            frame.fill(index < 4 ? QColor(30, 30, 30) : QColor(30, 30, 220));
            if (index < 4) {
                for (int row = 0; row < 24; ++row)
                    for (int column = 0; column < 24; ++column)
                        frame.setPixelColor(16 + index * 3 + column, 20 + row,
                                            QColor(60 + column * 7, 40 + row * 7, 100 + (column + row) % 8 * 12));
            }
            QVERIFY(frame.save(directory.filePath(QStringLiteral("frame-%1.png").arg(index, 2, 10, QLatin1Char('0')))));
        }
        const QString video = directory.filePath(QStringLiteral("moving.mp4"));
        QCOMPARE(QProcess::execute(QStringLiteral("ffmpeg"), {
            QStringLiteral("-v"), QStringLiteral("error"), QStringLiteral("-framerate"), QStringLiteral("2"),
            QStringLiteral("-i"), directory.filePath(QStringLiteral("frame-%02d.png")),
            QStringLiteral("-c:v"), QStringLiteral("libx264"), QStringLiteral("-pix_fmt"),
            QStringLiteral("yuv420p"), QStringLiteral("-y"), video}), 0);
        MainWindow window{directory.path(), QString()};
        window.m_fps = 10;
        window.m_project = QJsonObject{{QStringLiteral("sources"), QJsonArray{
            QJsonObject{{QStringLiteral("id"), QStringLiteral("moving")}, {QStringLiteral("path"), video}}}}};
        const QJsonObject clip{{QStringLiteral("sourceId"), QStringLiteral("moving")},
                               {QStringLiteral("timelineStart"), 0}, {QStringLiteral("in"), 0},
                               {QStringLiteral("out"), 4}, {QStringLiteral("speed"), 1}};
        const QRectF seed(16.0 / 160, 20.0 / 90, 24.0 / 160, 24.0 / 90);
        QVERIFY(!window.sourceFrame(clip, 0).isNull());
        const double cut = window.firstSceneCut(clip, 0, 4);
        QVERIFY2(std::abs(cut - 2.0) < 0.1, qPrintable(QString::number(cut)));
        double reliableEnd = 4;
        const auto boxes = window.trackRegion(clip, 0, 4, seed, &reliableEnd);
        QVERIFY(boxes.size() >= 2);
        QVERIFY(boxes.last().second.x() > seed.x());
        QVERIFY(reliableEnd <= 2.5);
    }

    void disconnectedControlsAndRecovery() {
        MainWindow window{QString(), QString()};
        QCOMPARE(window.m_agentStatus->text(), QStringLiteral("CONNECTING"));
        QVERIFY(!window.m_sendButton->isEnabled());
        window.handleEvent(QJsonObject{{"type", "connection"}, {"status", "disconnected"}, {"message", "Daemon unavailable"}});
        QCOMPARE(window.m_agentStatus->text(), QStringLiteral("OFFLINE"));
        QVERIFY(window.m_reconnectButton->isEnabled());
        QVERIFY(!window.m_timeline->isEnabled());
        QVERIFY(!window.m_connectionBanner->isHidden());
        QVERIFY(window.m_connectionMessage->text().contains(QStringLiteral("Daemon unavailable")));
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", false}});
        QCOMPARE(window.m_agentStatus->text(), QStringLiteral("OFFLINE"));
        window.handleEvent(QJsonObject{{"type", "connection"}, {"status", "ready"}});
        QVERIFY(window.m_connectionBanner->isHidden());
        QVERIFY(window.m_timeline->isEnabled());
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
        // An hour stays at least 80px per second, so the pointer can address
        // frames instead of collapsing onto a handful of pixels.
        QVERIFY(scroll.horizontalScrollBar()->maximum() > 0);
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
