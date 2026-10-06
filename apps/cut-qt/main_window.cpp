#include "main_window.h"
#include "bridge.h"
#include "editor_common.h"
#include "editor_style.h"
#include "media_artwork.h"
#include "timeline_widget.h"
#include "viewer_widget.h"

#include <QApplication>
#include <QClipboard>
#include <QColorDialog>
#include <QDateTime>
#include <QDialog>
#include <QDialogButtonBox>
#include <QDir>
#include <QFile>
#include <QFileDialog>
#include <QFileInfo>
#include <QFontDatabase>
#include <QHBoxLayout>
#include <QIcon>
#include <QJsonDocument>
#include <QKeySequence>
#include <QMenu>
#include <QMenuBar>
#include <QMessageBox>
#include <QPaintEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPixmap>
#include <QPolygonF>
#include <QProcess>
#include <QRegularExpression>
#include <QSaveFile>
#include <QScrollBar>
#include <QSettings>
#include <QSignalBlocker>
#include <QSplitter>
#include <QStatusBar>
#include <QTabWidget>
#include <QTextFrame>
#include <QToolBar>
#include <QVBoxLayout>

#include <algorithm>
#include <limits>

namespace cut {

namespace {

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

} // namespace

MainWindow::MainWindow(const QString &projectDirectory, const QString &cutdBinary, QWidget *parent)
    : QMainWindow(parent), m_projectDirectory(projectDirectory), m_cutdBinary(cutdBinary) {
    setWindowTitle(QStringLiteral("Cut — Editor"));
    resize(1500, 940);
    setMinimumSize(1080, 700);
    buildUi();
    setupActions();
    setupStatusBar();
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

MainWindow::~MainWindow() {
    if (m_bridge) cut_bridge_free(m_bridge);
}

void MainWindow::pollBridge() {
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

void MainWindow::importMedia() {
    const QString path = QFileDialog::getOpenFileName(this, QStringLiteral("Import media"), QString(),
                                                       QStringLiteral("Media files (*.mov *.mp4 *.m4v *.mkv *.webm *.wav *.mp3);;All files (*)"));
    if (path.isEmpty()) return;
    if (!sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("import")},
                           {QStringLiteral("path"), path},
                           {QStringLiteral("revision"), QJsonValue::fromVariant(QVariant::fromValue(m_revision))}})) return;
    addActivity(QStringLiteral("import"), QStringLiteral("Importing %1").arg(QFileInfo(path).fileName()));
}

void MainWindow::exportProject() {
    const QString quality = m_quality->currentData().toString();
    if (!sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("export")}, {QStringLiteral("quality"), quality}})) return;
    addActivity(QStringLiteral("export"), QStringLiteral("Export started (%1)").arg(m_quality->currentText()));
}

void MainWindow::renderPreview() {
    if (!sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("preview")}})) return;
    addActivity(QStringLiteral("preview"), QStringLiteral("Rendering preview"));
    m_previewButton->setEnabled(false);
}

void MainWindow::canvasPresetChanged(int index) {
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
    const QJsonObject op{{QStringLiteral("op"), QStringLiteral("set-sequence")},
                         {QStringLiteral("width"), width},
                         {QStringLiteral("height"), height}};
    if (!sendEdit(QJsonArray{op})) syncCanvasPreset(m_canvasSize);
}

void MainWindow::addTrack() {
    QMenu menu(this);
    auto *video = menu.addAction(QStringLiteral("Add video track"));
    auto *audio = menu.addAction(QStringLiteral("Add audio track"));
    const QAction *choice = menu.exec(m_timeline->mapToGlobal(m_timeline->addLaneButtonCenter()));
    if (choice != video && choice != audio) return;
    const QString kind = choice == audio ? QStringLiteral("audio") : QStringLiteral("video");
    int count = 0;
    for (const QJsonValue &value : m_project.value(QStringLiteral("tracks")).toArray())
        if (value.toObject().value(QStringLiteral("type")).toString() == kind) ++count;
    const QString name = choice == audio ? QStringLiteral("Audio %1").arg(count + 1)
                                          : QStringLiteral("Video %1").arg(count + 1);
    sendEdit(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("add-track")},
                                    {QStringLiteral("kind"), kind},
                                    {QStringLiteral("name"), name}}});
}

void MainWindow::togglePlayback() {
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

void MainWindow::queueSeek(double time) {
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
    updateTransport();
    m_viewer->setLiveScrub(true);
    if (m_pendingSeekScrub) m_exactSeekTimer.start(50);
    else m_exactSeekTimer.stop();
    flushSeek();
}

void MainWindow::queueHoverSeek(double time) {
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

void MainWindow::endHoverSeek() {
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

void MainWindow::settlePointerSeek() {
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

bool MainWindow::scrubFromShownFrame(double time) const {
    if (!m_hasShownFrame) return true;
    const double delta = time - m_shownFrameTime;
    return delta < -(1.0 / std::max(1.0, m_fps)) || delta > 0.40;
}

void MainWindow::selectClip(const QString &id) {
    m_selectedClip = id;
    m_timeline->setSelectedClip(id);
    updateInspector();
}

bool MainWindow::sendEdit(const QJsonArray &ops) {
    if (ops.isEmpty()) return false;
    return sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("edit")},
                                  {QStringLiteral("revision"), revisionValue()},
                                  {QStringLiteral("ops"), ops}});
}

QJsonObject MainWindow::clipObject(const QString &id) const {
    for (const QJsonValue &trackValue : m_project.value(QStringLiteral("tracks")).toArray()) {
        for (const QJsonValue &clipValue : trackValue.toObject().value(QStringLiteral("clips")).toArray()) {
            const QJsonObject clip = clipValue.toObject();
            if (clip.value(QStringLiteral("id")).toString() == id) return clip;
        }
    }
    return {};
}

qint64 MainWindow::clipFrames(const QJsonObject &clip, const QString &key) {
    return static_cast<qint64>(clip.value(key).toDouble());
}

void MainWindow::moveClip(const QString &id, double start) {
    const QJsonObject clip = clipObject(id);
    if (clip.isEmpty()) return;
    const qint64 frames = frameAt(start) - clipFrames(clip, QStringLiteral("startFrame"));
    if (frames == 0) return;
    sendEdit(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("move")},
                                    {QStringLiteral("clip"), id},
                                    {QStringLiteral("frames"), frames}}});
}

void MainWindow::trimClip(const QString &id, bool leading, double frame) {
    const QJsonObject clip = clipObject(id);
    if (clip.isEmpty()) return;
    const qint64 start = clipFrames(clip, QStringLiteral("startFrame"));
    const qint64 edge = leading ? start : start + clipFrames(clip, QStringLiteral("durationFrames"));
    const qint64 frames = std::llround(frame) - edge;
    if (frames == 0) return;
    sendEdit(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("trim")},
                                    {QStringLiteral("clip"), id},
                                    {QStringLiteral("edge"), leading ? QStringLiteral("in") : QStringLiteral("out")},
                                    {QStringLiteral("frames"), frames}}});
}

void MainWindow::splitClip() {
    if (m_selectedClip.isEmpty()) return;
    sendEdit(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("split")},
                                    {QStringLiteral("clip"), m_selectedClip},
                                    {QStringLiteral("at"), frameAt(m_playhead)},
                                    {QStringLiteral("tail"), QStringLiteral("clip-%1").arg(QDateTime::currentMSecsSinceEpoch())}}});
}

QJsonObject MainWindow::cropProps(const QJsonObject &box) {
    if (box.isEmpty())
        return QJsonObject{{QStringLiteral("crop_l"), QJsonValue()}, {QStringLiteral("crop_t"), QJsonValue()},
                           {QStringLiteral("crop_r"), QJsonValue()}, {QStringLiteral("crop_b"), QJsonValue()}};
    const double x = box.value(QStringLiteral("x")).toDouble();
    const double y = box.value(QStringLiteral("y")).toDouble();
    const double width = box.value(QStringLiteral("width")).toDouble(1.0);
    const double height = box.value(QStringLiteral("height")).toDouble(1.0);
    return QJsonObject{{QStringLiteral("crop_l"), x}, {QStringLiteral("crop_t"), y},
                       {QStringLiteral("crop_r"), std::max(0.0, 1.0 - x - width)},
                       {QStringLiteral("crop_b"), std::max(0.0, 1.0 - y - height)}};
}

QJsonObject MainWindow::cropBox(const QJsonObject &clip) {
    const QJsonObject props = clip.value(QStringLiteral("props")).toObject();
    const auto edge = [&props](const char *name) { return props.value(QLatin1String(name)).toDouble(); };
    const double l = edge("crop_l"), t = edge("crop_t"), r = edge("crop_r"), b = edge("crop_b");
    if (l == 0.0 && t == 0.0 && r == 0.0 && b == 0.0) return {};
    return QJsonObject{{QStringLiteral("x"), l}, {QStringLiteral("y"), t},
                       {QStringLiteral("width"), 1.0 - l - r}, {QStringLiteral("height"), 1.0 - t - b}};
}

QImage MainWindow::sourceFrame(const QJsonObject &clip, double timelineTime) const {
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

double MainWindow::firstSceneCut(const QJsonObject &clip, double start, double end) const {
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

QVector<QPair<qint64, QRectF>> MainWindow::trackRegion(const QJsonObject &clip, double start, double end,
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

void MainWindow::focusRegion() {
    if (!m_proposalId.isEmpty()) {
        QMessageBox::information(this, QStringLiteral("Focus region"),
                                 QStringLiteral("Review the pending suggestion before changing the crop."));
        return;
    }
    const QJsonObject clip = selectedClipObject();
    if (clip.isEmpty()) return;
    if (selectedTrackType() != QStringLiteral("video")) return;
    const double clipStart = clip.value(QStringLiteral("timelineStart")).toDouble();
    const double clipDuration = clip.value(QStringLiteral("duration")).toDouble();
    const double clipEnd = clipStart + clipDuration;
    if (!(clipDuration > 1.0 / m_fps)) return;

    const QJsonObject crop = cropBox(clip);
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
    QJsonArray ops;
    auto split = [&ops](const QString &id, qint64 at, const QString &tail) {
        ops.append(QJsonObject{{QStringLiteral("op"), QStringLiteral("split")}, {QStringLiteral("clip"), id},
                               {QStringLiteral("at"), at}, {QStringLiteral("tail"), tail}});
    };
    auto setCrop = [&ops](const QString &id, const QJsonObject &box) {
        ops.append(QJsonObject{{QStringLiteral("op"), QStringLiteral("set")}, {QStringLiteral("clip"), id},
                               {QStringLiteral("props"), cropProps(box)}});
    };
    auto boxOf = [](const QRectF &rect) {
        return QJsonObject{{QStringLiteral("x"), rect.x()}, {QStringLiteral("y"), rect.y()},
                           {QStringLiteral("width"), rect.width()}, {QStringLiteral("height"), rect.height()}};
    };
    if (startFrame > clipStartFrame) {
        focusId = m_selectedClip + QStringLiteral("-focus-") + suffix;
        split(m_selectedClip, startFrame, focusId);
    }
    setCrop(focusId, boxOf(region));
    for (int index = 1; index < tracked.size(); ++index) {
        const auto &step = tracked.at(index);
        if (step.first >= endFrame) break;
        const QString nextId = m_selectedClip + QStringLiteral("-track-%1-").arg(index) + suffix;
        split(focusId, step.first, nextId);
        focusId = nextId;
        setCrop(focusId, boxOf(step.second));
    }
    if (endFrame < clipEndFrame) {
        const QString afterId = m_selectedClip + QStringLiteral("-after-") + suffix;
        split(focusId, endFrame, afterId);
        setCrop(afterId, crop);
    }
    if (sendEdit(ops)) m_selectedClip = focusId;
}

void MainWindow::deleteSelected() {
    if (m_selectedClip.isEmpty()) return;
    sendEdit(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("delete")},
                                    {QStringLiteral("clips"), QJsonArray{m_selectedClip}}}});
}

void MainWindow::sendChat() {
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

void MainWindow::updateChatControls() {
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
                                            : QStringLiteral("↵ Send  ·  ⇧↵ New line"));
    m_chatInput->setPlaceholderText(!m_connected ? QStringLiteral("Your next edit starts here…")
                                   : m_agentBusy ? QStringLiteral("Working on your edit…")
                                                 : QStringLiteral("Describe your next edit…"));
}

QString MainWindow::activeProviderId() const {
    return m_openaiActiveRadio && m_openaiActiveRadio->isChecked() ? QStringLiteral("openai")
                                                                   : QStringLiteral("anthropic");
}

QJsonObject MainWindow::agentSettingsPayload() const {
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

void MainWindow::loadInboundMcpConfig(const QString &path, bool imported) {
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

void MainWindow::saveInboundMcpConfig() {
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

void MainWindow::updateAgentModelChoices() {
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

void MainWindow::loadAgentSettings() {
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

void MainWindow::saveAgentSettings() {
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

void MainWindow::acceptProposal() {
    if (m_proposalId.isEmpty()) return;
    sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("accept")}, {QStringLiteral("proposalId"), m_proposalId}, {QStringLiteral("revision"), revisionValue()}});
}

void MainWindow::rejectProposal() {
    if (m_proposalId.isEmpty()) return;
    sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("reject")}, {QStringLiteral("proposalId"), m_proposalId}, {QStringLiteral("revision"), revisionValue()}});
}

void MainWindow::clipValueEdited() {
    if (m_selectedClip.isEmpty() || m_updatingInspector) return;
    const QString field = sender()->property("clipField").toString();
    const QDoubleSpinBox *spin = qobject_cast<QDoubleSpinBox *>(sender());
    const QJsonObject clip = selectedClipObject();
    if (!spin || field.isEmpty() || clip.isEmpty()) return;
    const double value = spin->value();
    const double speed = std::max(0.01, clip.value(QStringLiteral("speed")).toDouble(1.0));
    auto amount = [this](const QString &op, double frames, const QString &edge = QString()) {
        const qint64 whole = std::llround(frames);
        if (whole == 0) return QJsonObject{};
        QJsonObject edit{{QStringLiteral("op"), op}, {QStringLiteral("clip"), m_selectedClip},
                         {QStringLiteral("frames"), whole}};
        if (!edge.isEmpty()) edit.insert(QStringLiteral("edge"), edge);
        return edit;
    };
    QJsonObject edit;
    if (field == QStringLiteral("timelineStart")) {
        edit = amount(QStringLiteral("move"), double(frameAt(value) - clipFrames(clip, QStringLiteral("startFrame"))));
    } else if (field == QStringLiteral("duration")) {
        edit = amount(QStringLiteral("trim"), double(frameAt(value) - clipFrames(clip, QStringLiteral("durationFrames"))), QStringLiteral("out"));
    } else if (field == QStringLiteral("in")) {
        edit = amount(QStringLiteral("slip"), (value - clip.value(QStringLiteral("in")).toDouble()) / speed * m_fps);
    } else if (field == QStringLiteral("out")) {
        edit = amount(QStringLiteral("trim"), (value - clip.value(QStringLiteral("out")).toDouble()) / speed * m_fps, QStringLiteral("out"));
    } else if (field == QStringLiteral("speed")) {
        QString ratio = QString::number(value, 'f', 4);
        while (ratio.endsWith(QLatin1Char('0'))) ratio.chop(1);
        if (ratio.endsWith(QLatin1Char('.'))) ratio.chop(1);
        if (clip.value(QStringLiteral("reverse")).toBool()) ratio.prepend(QLatin1Char('-'));
        edit = QJsonObject{{QStringLiteral("op"), QStringLiteral("speed")}, {QStringLiteral("clip"), m_selectedClip},
                           {QStringLiteral("speed"), ratio}};
    }
    if (!edit.isEmpty()) sendEdit(QJsonArray{edit});
}

void MainWindow::propertyEdited(const QString &name, const QJsonValue &value) {
    if (m_selectedClip.isEmpty() || m_updatingInspector) return;
    QJsonValue fallback;
    for (const QJsonValue &def : m_propertyDefs)
        if (def.toObject().value(QStringLiteral("name")).toString() == name) fallback = def.toObject().value(QStringLiteral("default"));
    const QJsonValue current = selectedClipObject().value(QStringLiteral("props")).toObject().value(name);
    auto same = [](const QJsonValue &a, const QJsonValue &b) {
        return a.isDouble() && b.isDouble() ? std::abs(a.toDouble() - b.toDouble()) < 5e-4 : a == b;
    };
    if (same(value, current.isUndefined() ? fallback : current)) return;
    sendEdit(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("set")}, {QStringLiteral("clip"), m_selectedClip},
                                    {QStringLiteral("props"), QJsonObject{{name, same(value, fallback) ? QJsonValue() : value}}}}});
}

bool MainWindow::eventFilter(QObject *object, QEvent *event) {
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

void MainWindow::keyPressEvent(QKeyEvent *event) {
    if (event->key() == Qt::Key_S && m_timeline && m_timeline->hasFocus()) {
        splitClip();
        event->accept();
        return;
    }
    QMainWindow::keyPressEvent(event);
}

void MainWindow::buildUi() {
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
    viewerLayout->setSpacing(0);
    const PanelHeader viewerHeader = panelHeader(QStringLiteral("Viewer"));
    m_viewerDetail = viewerHeader.detail;
    viewerLayout->addWidget(viewerHeader.widget);
    m_viewer = new ViewerWidget;
    viewerLayout->addWidget(m_viewer, 1);
    viewerLayout->addWidget(buildTransport());
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

    const PanelHeader timelineHeader = panelHeader(QStringLiteral("Timeline"));
    m_timelineDetail = timelineHeader.detail;
    timelinePanelLayout->addWidget(timelineHeader.widget);

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
    timelinePanel->setMinimumHeight(220);
    bottomLayout->addWidget(timelinePanel, 1);
    editorSplit->addWidget(bottomBand);
    editorSplit->setStretchFactor(0, 1);
    editorSplit->setStretchFactor(1, 1);
    editorSplit->setSizes({420, 460});

    auto *right = new QWidget;
    m_agentPanel = right;
    right->setObjectName(QStringLiteral("assistantPanel"));
    right->setAttribute(Qt::WA_StyledBackground);
    right->setMinimumWidth(248);
    auto *rightFrame = new QVBoxLayout(right);
    rightFrame->setContentsMargins(0, 0, 0, 0);
    rightFrame->setSpacing(0);
    const PanelHeader agentHeader = panelHeader(QStringLiteral("Cut Assistant"), QStringLiteral("assistantTitle"));
    agentHeader.detail->hide();
    m_agentStatus = new QLabel(QStringLiteral("READY"));
    m_agentStatus->setObjectName(QStringLiteral("statusPill"));
    agentHeader.layout->addWidget(m_agentStatus);
    rightFrame->addWidget(agentHeader.widget);
    auto *rightBody = new QWidget;
    rightFrame->addWidget(rightBody, 1);
    auto *rightLayout = new QVBoxLayout(rightBody);
    rightLayout->setContentsMargins(8, 2, 8, 8);
    rightLayout->setSpacing(6);

    auto *tabs = new QTabWidget;
    tabs->setObjectName(QStringLiteral("rightTabs"));
    rightLayout->addWidget(tabs, 1);
    auto *chatTab = new QWidget;
    auto *chatTabLayout = new QVBoxLayout(chatTab);
    chatTabLayout->setContentsMargins(0, 6, 0, 0);
    chatTabLayout->setSpacing(6);
    // Review and activity can overflow on short windows; the composer stays fixed.
    auto *chatScroll = new QScrollArea;
    chatScroll->setFrameShape(QFrame::NoFrame);
    chatScroll->setWidgetResizable(true);
    chatScroll->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    auto *chatContent = new QWidget;
    auto *chatLayout = new QVBoxLayout(chatContent);
    chatLayout->setContentsMargins(0, 0, 0, 0);
    chatLayout->setSizeConstraint(QLayout::SetMinimumSize);
    chatLayout->setSpacing(6);
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
        "<p style='margin-top:2px; color:#c6a15a; font-size:10px;'>YOUR EDITING PARTNER</p>"
        "<p style='margin-top:2px; color:#f0f0f0; font-size:14px; font-weight:600;'>Make your next cut.</p>"
        "<p style='margin-top:4px; color:#b6b6b6;'>Describe the change. Cut uses this timeline.</p>"
        "<p style='margin-top:10px; color:#888888; font-size:10px;'>TRY ASKING</p>"
        "<p style='margin-top:2px; color:#dddddd;'>“Trim the opening to three seconds.”</p>"
        "<p style='margin-top:2px; color:#dddddd;'>“Lower the music under the dialogue.”</p>"
        "<p style='margin-top:10px; color:#999999; font-size:11px;'>Review a proposal before it changes the cut.</p>"));
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
    form->addRow(QStringLiteral("Position"), m_position);
    form->addRow(QStringLiteral("Duration"), m_durationSpin);
    form->addRow(QStringLiteral("Source in"), m_inSpin);
    form->addRow(QStringLiteral("Source out"), m_outSpin);
    form->addRow(QStringLiteral("Speed"), m_speedSpin);
    inspectorLayout->addWidget(inspector);
    auto *properties = new QWidget;
    m_propertyForm = new QFormLayout(properties);
    m_propertyForm->setContentsMargins(0, 0, 0, 0);
    m_propertyForm->setSpacing(6);
    inspectorLayout->addWidget(properties);
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
    workspace->setSizes({1228, 272});
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

QDoubleSpinBox *MainWindow::inspectorSpin(double value, double minimum, double maximum, double step, const QString &field) {
    auto *spin = new QDoubleSpinBox;
    spin->setRange(minimum, maximum);
    spin->setSingleStep(step);
    spin->setDecimals(2);
    spin->setValue(value);
    spin->setProperty("clipField", field);
    connect(spin, &QDoubleSpinBox::editingFinished, this, &MainWindow::clipValueEdited);
    return spin;
}

QWidget *MainWindow::propertyEditor(const QJsonObject &def) {
    const QString name = def.value(QStringLiteral("name")).toString();
    const QString type = def.value(QStringLiteral("type")).toString();
    QWidget *editor = nullptr;
    if (type == QStringLiteral("number")) {
        auto *spin = new QDoubleSpinBox;
        const double minimum = def.value(QStringLiteral("min")).toDouble(-1e6);
        const double maximum = def.value(QStringLiteral("max")).toDouble(1e6);
        spin->setRange(minimum, maximum);
        spin->setDecimals(3);
        spin->setSingleStep(maximum - minimum > 10.0 ? 1.0 : 0.01);
        connect(spin, &QDoubleSpinBox::editingFinished, this, [this, spin, name] { propertyEdited(name, spin->value()); });
        editor = spin;
    } else if (type == QStringLiteral("enum")) {
        auto *combo = new QComboBox;
        for (const QJsonValue &option : def.value(QStringLiteral("options")).toArray()) combo->addItem(option.toString());
        connect(combo, &QComboBox::activated, this, [this, combo, name] { propertyEdited(name, combo->currentText()); });
        editor = combo;
    } else if (type == QStringLiteral("bool")) {
        auto *check = new QCheckBox;
        connect(check, &QCheckBox::clicked, this, [this, name](bool checked) { propertyEdited(name, checked); });
        editor = check;
    } else if (type == QStringLiteral("color")) {
        auto *button = new QPushButton;
        connect(button, &QPushButton::clicked, this, [this, button, name] {
            const QColor color = QColorDialog::getColor(modelColor(button->text()), this, name,
                                                        QColorDialog::ShowAlphaChannel);
            if (color.isValid()) propertyEdited(name, modelColorName(color));
        });
        editor = button;
    } else {
        auto *line = new QLineEdit;
        connect(line, &QLineEdit::editingFinished, this, [this, line, name] { propertyEdited(name, line->text()); });
        editor = line;
    }
    QString tip = def.value(QStringLiteral("doc")).toString();
    if (def.value(QStringLiteral("unit")).isString()) tip += QStringLiteral(" (%1)").arg(def.value(QStringLiteral("unit")).toString());
    editor->setToolTip(tip.trimmed());
    return editor;
}

void MainWindow::setPropertyValue(QWidget *editor, const QJsonValue &value) {
    const QSignalBlocker blocker(editor);
    if (auto *spin = qobject_cast<QDoubleSpinBox *>(editor)) spin->setValue(value.toDouble());
    else if (auto *combo = qobject_cast<QComboBox *>(editor)) combo->setCurrentText(value.toString());
    else if (auto *check = qobject_cast<QCheckBox *>(editor)) check->setChecked(value.toBool());
    else if (auto *button = qobject_cast<QPushButton *>(editor)) {
        QPixmap swatch(12, 12);
        swatch.fill(modelColor(value.toString()));
        button->setIcon(swatch);
        button->setText(value.toString());
    }
    else if (auto *line = qobject_cast<QLineEdit *>(editor)) line->setText(value.toString());
}

void MainWindow::syncCanvasPreset(const QSize &size) {
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

void MainWindow::setupActions() {
    menuBar()->setNativeMenuBar(true);
    auto *view = menuBar()->addMenu(QStringLiteral("View"));
    auto addPanelToggle = [view](const QString &title, QWidget *panel) {
        auto *action = view->addAction(title);
        action->setCheckable(true);
        action->setChecked(true);
        connect(action, &QAction::toggled, panel, &QWidget::setVisible);
    };
    addPanelToggle(QStringLiteral("Agent and inspector"), m_agentPanel);
    view->addSeparator();
    auto *fitTracks = view->addAction(QStringLiteral("Fit tracks to panel"));
    connect(fitTracks, &QAction::triggered, m_timeline, &TimelineWidget::fitTracksToView);
    auto *resetTracks = view->addAction(QStringLiteral("Reset track heights"));
    connect(resetTracks, &QAction::triggered, m_timeline, &TimelineWidget::resetTrackHeights);
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
    playback->addSeparator();
    const auto transportAction = [&](const QString &title, Qt::Key key, void (MainWindow::*slot)()) {
        auto *action = playback->addAction(title);
        action->setShortcut(key);
        action->setShortcutContext(Qt::WidgetShortcut);
        m_timeline->addAction(action);
        connect(action, &QAction::triggered, this, slot);
        return action;
    };
    auto *toStart = transportAction(QStringLiteral("Go to start"), Qt::Key_Home, &MainWindow::seekToStart);
    auto *previousFrame = transportAction(QStringLiteral("Previous frame"), Qt::Key_Left, &MainWindow::stepBackward);
    auto *nextFrame = transportAction(QStringLiteral("Next frame"), Qt::Key_Right, &MainWindow::stepForward);
    auto *toEnd = transportAction(QStringLiteral("Go to end"), Qt::Key_End, &MainWindow::seekToEnd);

    auto *toolbar = addToolBar(QStringLiteral("Main"));
    toolbar->setMovable(false);
    toolbar->setIconSize(QSize(15, 15));
    toolbar->setFixedHeight(30);
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
    m_runtimeActions = {import, exportAction, refreshAction, splitAction, deleteAction, play, split, deleteToolbar, exportToolbar,
                        toStart, previousFrame, nextFrame, toEnd};
}

QWidget *MainWindow::buildTransport() {
    auto *transport = new QWidget;
    transport->setObjectName(QStringLiteral("transport"));
    transport->setAttribute(Qt::WA_StyledBackground);
    transport->setFixedHeight(28);
    auto *layout = new QHBoxLayout(transport);
    layout->setContentsMargins(10, 0, 10, 0);
    layout->setSpacing(1);
    m_transportTime = new QLabel(timelineTimecode(0.0, m_fps));
    m_transportTime->setObjectName(QStringLiteral("transportTime"));
    m_transportTime->setMinimumWidth(120);
    layout->addWidget(m_transportTime);
    layout->addStretch(1);
    const auto button = [&](TransportGlyph glyph, const QString &tip, void (MainWindow::*slot)()) {
        auto *control = new QToolButton;
        control->setObjectName(QStringLiteral("transportButton"));
        control->setIcon(transportIcon(glyph));
        control->setIconSize(QSize(14, 14));
        control->setToolTip(tip);
        control->setAccessibleName(tip);
        connect(control, &QToolButton::clicked, this, slot);
        layout->addWidget(control);
        m_transportButtons.append(control);
        return control;
    };
    button(TransportGlyph::Start, QStringLiteral("Go to start (Home)"), &MainWindow::seekToStart);
    button(TransportGlyph::PreviousFrame, QStringLiteral("Previous frame (Left)"), &MainWindow::stepBackward);
    m_transportPlay = button(TransportGlyph::Play, QStringLiteral("Play (Space)"), &MainWindow::togglePlayback);
    button(TransportGlyph::NextFrame, QStringLiteral("Next frame (Right)"), &MainWindow::stepForward);
    button(TransportGlyph::End, QStringLiteral("Go to end (End)"), &MainWindow::seekToEnd);
    layout->addStretch(1);
    m_transportDuration = new QLabel(timelineTimecode(0.0, m_fps));
    m_transportDuration->setObjectName(QStringLiteral("transportDuration"));
    m_transportDuration->setMinimumWidth(120);
    m_transportDuration->setAlignment(Qt::AlignRight | Qt::AlignVCenter);
    m_transportDuration->setToolTip(QStringLiteral("Program length"));
    layout->addWidget(m_transportDuration);
    return transport;
}

void MainWindow::updateTransport() {
    if (!m_transportTime) return;
    m_transportTime->setText(timelineTimecode(m_playhead, m_fps));
    m_transportDuration->setText(timelineTimecode(m_duration, m_fps));
    if (m_transportShowsPause != m_playing) {
        m_transportShowsPause = m_playing;
        m_transportPlay->setIcon(transportIcon(m_playing ? TransportGlyph::Pause : TransportGlyph::Play));
        m_transportPlay->setToolTip(m_playing ? QStringLiteral("Pause (Space)") : QStringLiteral("Play (Space)"));
    }
}

void MainWindow::stepFrames(int frames) {
    const double rate = std::max(1.0, m_fps);
    queueSeek((std::round(m_playhead * rate) + frames) / rate);
}

void MainWindow::setupStatusBar() {
    statusBar()->setSizeGripEnabled(false);
    m_statusRevision = new QLabel;
    m_statusRevision->setObjectName(QStringLiteral("statusRevision"));
    m_statusRevision->hide();
    m_statusConnection = new QLabel;
    m_statusConnection->setObjectName(QStringLiteral("statusConnection"));
    statusBar()->addPermanentWidget(m_statusRevision);
    statusBar()->addPermanentWidget(m_statusConnection);
}

bool MainWindow::sendSimple(const QJsonObject &value) {
    if (!m_connected && value.value(QStringLiteral("type")).toString() != QStringLiteral("reconnect") &&
        value.value(QStringLiteral("type")).toString() != QStringLiteral("abort")) {
        return false;
    }
    m_lastCommand = value;
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

bool MainWindow::sendSeek(double position, bool scrub) {
    if (!m_connected || !m_bridge) return false;
    const QJsonObject payload{{QStringLiteral("type"), QStringLiteral("seek")},
                              {QStringLiteral("position"), position},
                              {QStringLiteral("scrub"), scrub}};
    const QByteArray bytes = QJsonDocument(payload).toJson(QJsonDocument::Compact);
    return cut_bridge_send(m_bridge, bytes.constData());
}

void MainWindow::flushSeek() {
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

void MainWindow::handleEvent(QJsonObject event) {
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
        updateTransport();
    } else if (type == QStringLiteral("playback")) {
        m_playing = m_connected && event.value(QStringLiteral("playing")).toBool();
        updateTransport();
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

void MainWindow::applySnapshot(const QJsonObject &snapshot) {
    m_snapshot = snapshot;
    m_revision = static_cast<quint64>(snapshot.value(QStringLiteral("revision")).toDouble());
    m_duration = std::max(0.0, snapshot.value(QStringLiteral("duration")).toDouble());
    // The timeline is a view of the model's first sequence; edits go back
    // as typed operations.
    m_project = snapshot.value(QStringLiteral("timeline")).toObject();
    // Derive presentation fields once per snapshot, not on every repaint.
    QHash<QString, QJsonObject> sourceById;
    for (const auto &value : m_project.value("sources").toArray()) { const auto source = value.toObject(); sourceById.insert(source.value("id").toString(), source); }
    QJsonArray displayTracks;
    for (const auto &value : m_project.value("tracks").toArray()) {
        auto track = value.toObject(); QJsonArray displayClips;
        for (const auto &clipValue : track.value("clips").toArray()) {
            auto clip = clipValue.toObject(); const auto source = sourceById.value(clip.value("sourceId").toString());
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
    const QJsonArray sources = m_project.value(QStringLiteral("sources")).toArray();
    const QSize canvasSize = resolvedCanvasSize(m_project, sources);
    m_viewer->setProjectInfo(canvasSize.width(), canvasSize.height());
    syncCanvasPreset(canvasSize);
    m_viewerDetail->setText(QStringLiteral("%1 × %2  ·  %3 fps")
                                .arg(canvasSize.width()).arg(canvasSize.height()).arg(QString::number(m_fps, 'g', 5)));
    m_timelineDetail->setText(QStringLiteral("%1  ·  %2")
                                  .arg(m_trackCount == 1 ? QStringLiteral("1 track") : QStringLiteral("%1 tracks").arg(m_trackCount),
                                       timelineTimecode(m_duration, m_fps)));
    if (m_statusRevision) {
        m_statusRevision->setText(QStringLiteral("Revision %1").arg(m_revision));
        m_statusRevision->show();
    }
    updateTransport();
    m_artwork->request(sources, m_projectDirectory);
    updateProposal(snapshot.value(QStringLiteral("pendingProposal")));
    updateInspector();
}

QString MainWindow::selectedTrackType() const {
    for (const QJsonValue &trackValue : m_project.value(QStringLiteral("tracks")).toArray()) {
        const QJsonObject track = trackValue.toObject();
        for (const QJsonValue &clipValue : track.value(QStringLiteral("clips")).toArray())
            if (clipValue.toObject().value(QStringLiteral("id")).toString() == m_selectedClip)
                return track.value(QStringLiteral("type")).toString();
    }
    return {};
}

QJsonObject MainWindow::selectedClipObject() const {
    const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
    for (const QJsonValue &trackValue : tracks) {
        for (const QJsonValue &clipValue : trackValue.toObject().value(QStringLiteral("clips")).toArray()) {
            const QJsonObject clip = clipValue.toObject();
            if (clip.value(QStringLiteral("id")).toString() == m_selectedClip) return clip;
        }
    }
    return {};
}

void MainWindow::updateInspector() {
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
    // Property editors follow the catalog definitions for a generator's
    // parameters and the clip's track type; animated properties show their
    // first key and are read-only.
    QJsonArray defs;
    if (hasClip) {
        defs = m_project.value(QStringLiteral("generators")).toObject().value(clip.value(QStringLiteral("fx")).toString()).toArray();
        for (const QJsonValue &def : m_project.value(QStringLiteral("properties")).toObject().value(selectedTrackType()).toArray())
            defs.append(def);
    }
    if (defs != m_propertyDefs) {
        while (m_propertyForm->rowCount() > 0) m_propertyForm->removeRow(0);
        m_propertyEditors.clear();
        m_propertyDefs = defs;
        for (const QJsonValue &def : defs) {
            QWidget *editor = propertyEditor(def.toObject());
            const QString name = def.toObject().value(QStringLiteral("name")).toString();
            m_propertyEditors.insert(name, editor);
            m_propertyForm->addRow(name, editor);
        }
    }
    const QJsonObject props = clip.value(QStringLiteral("props")).toObject();
    for (const QJsonValue &def : defs) {
        const QString name = def.toObject().value(QStringLiteral("name")).toString();
        const QJsonValue prop = props.value(name);
        QWidget *editor = m_propertyEditors.value(name);
        setPropertyValue(editor, prop.isArray() ? prop.toArray().first().toObject().value(QStringLiteral("value"))
                                 : prop.isUndefined() ? def.toObject().value(QStringLiteral("default")) : prop);
        editor->setEnabled(!prop.isArray());
    }
    m_updatingInspector = false;
}

void MainWindow::updateProposal(const QJsonValue &value) {
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
    const QString diff = proposal.value(QStringLiteral("documentDiff")).toString();
    int changed = 0;
    for (const QString &line : diff.split(QLatin1Char('\n'))) {
        if ((line.startsWith(QLatin1Char('+')) && !line.startsWith(QStringLiteral("+++"))) ||
            (line.startsWith(QLatin1Char('-')) && !line.startsWith(QStringLiteral("---"))))
            ++changed;
    }
    m_proposalSummary->setText(QStringLiteral("%1 changed line%2%3")
                                   .arg(changed)
                                   .arg(changed == 1 ? QString() : QStringLiteral("s"))
                                   .arg(stale ? QStringLiteral("  •  stale") : QString()));
    QString text;
    if (stale) text += QStringLiteral("This proposal is out of date. Ask Cut to prepare it again before accepting.\n\n");
    text += proposal.value(QStringLiteral("script")).toString();
    text += QStringLiteral("\n") + diff;
    m_proposalDiff->setPlainText(text);
    m_acceptButton->setEnabled(m_connected && !stale);
}

void MainWindow::appendConversation(const QString &speaker, const QString &text) {
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

void MainWindow::appendConversationNote(const QString &kind, const QString &text) {
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

void MainWindow::addActivity(const QString &kind, const QString &text) {
    if (text.isEmpty()) return;
    m_activityLog << QStringLiteral("%1  %2").arg(kind.toUpper(), text);
    while (m_activityLog.size() > 80) m_activityLog.removeFirst();
    if (m_activityList) {
        m_activityList->addItem(m_activityLog.constLast());
        while (m_activityList->count() > 8) delete m_activityList->takeItem(0);
        m_activityList->scrollToBottom();
    }
}

void MainWindow::showError(const QString &message) {
    m_errorMessage->setText(message);
    m_errorBanner->show();
}

void MainWindow::setConnection(const QString &status, const QString &message) {
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
    for (auto *button : m_transportButtons) button->setEnabled(m_connected);
    if (m_statusConnection) {
        const QString state = m_connected ? QStringLiteral("ready")
            : status == QStringLiteral("connecting") ? QStringLiteral("connecting") : QStringLiteral("disconnected");
        m_statusConnection->setText(state == QStringLiteral("ready") ? QStringLiteral("● Connected")
            : state == QStringLiteral("connecting") ? QStringLiteral("● Connecting…") : QStringLiteral("● Offline"));
        m_statusConnection->setProperty("state", state);
        m_statusConnection->style()->unpolish(m_statusConnection);
        m_statusConnection->style()->polish(m_statusConnection);
    }
    m_undoAction->setEnabled(m_connected && m_snapshot.value(QStringLiteral("canUndo")).toBool());
    m_redoAction->setEnabled(m_connected && m_snapshot.value(QStringLiteral("canRedo")).toBool());
    m_previewButton->setEnabled(m_connected && m_previewRequired);
    if (!m_connected) {
        m_playing = false;
        updateTransport();
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

} // namespace cut
