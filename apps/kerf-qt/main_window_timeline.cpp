// MainWindow's timeline: edits from its gestures, the AI bar, Focus Region,
// and the panel (the timeline in its scroll area under the AI bar; its bar's
// tools and zoom sit in the panel's title strip, see TimelineWiring).

#include "main_window.h"
#include "ask_prompt.h"
#include "bridge.h"
#include "chat_panel.h"
#include "command_registry.h"
#include "inspector_panel.h"
#include "effects_panel.h"
#include "presets_panel.h"
#include "project_launcher.h"
#include "project_panel.h"
#include "source_monitor_panel.h"
#include "stage_shell.h"
#include "editor_common.h"
#include "editor_theme.h"
#include "media_artwork.h"
#include "timeline_widget.h"
#include "toolbar_widgets.h"
#include "viewer_widget.h"
#ifdef Q_OS_MACOS
#include "macos_window.h"
#endif

#include <QAbstractTextDocumentLayout>
#include <QActionGroup>
#include <QApplication>
#include <QButtonGroup>
#include <QClipboard>
#include <QColorDialog>
#include <QCursor>
#include <QDateTime>
#include <QDesktopServices>
#include <QDialog>
#include <QDialogButtonBox>
#include <QDir>
#include <QFile>
#include <QFileDialog>
#include <QFileInfo>
#include <QFontDatabase>
#include <QGridLayout>
#include <QHBoxLayout>
#include <QInputDialog>
#include <QIcon>
#include <QImage>
#include <QJsonDocument>
#include <QKeySequence>
#include <QLayout>
#include <QMenu>
#include <QMenuBar>
#include <QMessageBox>
#include <QMouseEvent>
#include <QPaintEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPixmap>
#include <QPolygonF>
#include <QPlainTextEdit>
#include <QPushButton>
#include <QProcess>
#include <QRegularExpression>
#include <QSaveFile>
#include <QScrollBar>
#include <QSettings>
#include <QScreen>
#include <QShortcut>
#include <QShowEvent>
#include <QSignalBlocker>
#include <QTextBlock>
#include <QTextDocument>
#include <QTextFrame>
#include <QToolBar>
#include <QUrl>
#include <QVBoxLayout>
#include <QWindow>


#include <algorithm>
#include <numeric>
#include <limits>
#include <utility>
#include "wiring/main_window_shared.h"
#include "wiring/timeline_wiring.h"
#include "components.h"
#include "browser_panel.h"
#include "deliver_tray.h"
#include "editor_bus.h"
#include "finish_tray.h"
#include "inspector_column.h"

namespace kerf {

using namespace window_parts;

void MainWindow::addTrack() {
    QMenu menu(this);
    ui::styleMenu(&menu);
    auto *video = menu.addAction(QStringLiteral("Add video track"));
    auto *audio = menu.addAction(QStringLiteral("Add audio track"));
    // At the row under the tracks.
    const QAction *choice = menu.exec(m_timeline->mapToGlobal(m_timeline->addLaneButtonCenter()));
    if (choice != video && choice != audio) return;
    execute(choice == audio ? QStringLiteral("sequence.addAudioTrack") : QStringLiteral("sequence.addVideoTrack"));
}

void MainWindow::moveClip(const QString &id, double start) {
    const QJsonObject clip = clipObject(id);
    if (clip.isEmpty()) return;
    const qint64 frames = frameAt(start) - clipFrames(clip, QStringLiteral("startFrame"));
    if (frames == 0) return;
    // Slipping right shows earlier media; sliding moves the clip between its neighbours.
    const TimelineTool tool = m_timeline->tool();
    const QString command = tool == TimelineTool::Slip ? QStringLiteral("clip.slip")
        : tool == TimelineTool::Slide ? QStringLiteral("clip.slide") : QStringLiteral("clip.move");
    execute(command, QJsonObject{{QStringLiteral("clip"), id},
                                 {QStringLiteral("frames"), tool == TimelineTool::Slip ? -frames : frames}});
}

void MainWindow::trimClip(const QString &id, bool leading, double frame) {
    const QJsonObject clip = clipObject(id);
    if (clip.isEmpty()) return;
    const qint64 start = clipFrames(clip, QStringLiteral("startFrame"));
    const qint64 edge = leading ? start : start + clipFrames(clip, QStringLiteral("durationFrames"));
    const qint64 frames = std::llround(frame) - edge;
    if (frames == 0) return;
    if (m_timeline->tool() == TimelineTool::Roll) {
        // A roll moves the cut after a clip: this one's, or its neighbour's
        // when the in point is dragged.
        QString before = id;
        if (leading) {
            before.clear();
            for (const QJsonValue &value : clipTrack(id).value(QStringLiteral("clips")).toArray()) {
                const QJsonObject other = value.toObject();
                if (clipFrames(other, QStringLiteral("startFrame")) + clipFrames(other, QStringLiteral("durationFrames")) == start)
                    before = other.value(QStringLiteral("id")).toString();
            }
            if (before.isEmpty()) {
                showStatus(QStringLiteral("A roll needs a clip on the other side of the cut"), 4000);
                return;
            }
        }
        execute(QStringLiteral("clip.roll"), QJsonObject{{QStringLiteral("clip"), before}, {QStringLiteral("frames"), frames}});
        return;
    }
    execute(QStringLiteral("clip.trim"), QJsonObject{{QStringLiteral("clip"), id},
                                                     {QStringLiteral("edge"), leading ? QStringLiteral("in") : QStringLiteral("out")},
                                                     {QStringLiteral("frames"), frames},
                                                     {QStringLiteral("ripple"), m_timeline->tool() == TimelineTool::Ripple}});
}

QJsonObject MainWindow::clipTrack(const QString &id) const {
    for (const QJsonValue &trackValue : m_project.value(QStringLiteral("tracks")).toArray()) {
        const QJsonObject track = trackValue.toObject();
        for (const QJsonValue &clipValue : track.value(QStringLiteral("clips")).toArray())
            if (clipValue.toObject().value(QStringLiteral("id")).toString() == id) return track;
    }
    return {};
}

void MainWindow::splitClip() {
    execute(QStringLiteral("clip.split"));
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
    auto *start = new StepperSpinBox(&dialog);
    auto *end = new StepperSpinBox(&dialog);
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
    buttons->button(QDialogButtonBox::Ok)->setDefault(true);
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
    execute(QStringLiteral("clip.delete"));
}

void MainWindow::buildTimeline() {
    m_timelinePanel = new QWidget;
    m_timelinePanel->setObjectName(QStringLiteral("timelinePanel"));
    m_timelinePanel->setAttribute(Qt::WA_StyledBackground);
    auto *timelinePanelLayout = new QVBoxLayout(m_timelinePanel);
    timelinePanelLayout->setContentsMargins(0, 0, 0, 0);
    timelinePanelLayout->setSpacing(0);

    auto *timelineFrame = new QFrame;
    timelineFrame->setObjectName(QStringLiteral("timelineFrame"));
    auto *timelineLayout = new QVBoxLayout(timelineFrame);
    timelineLayout->setContentsMargins(0, 0, 0, 0);
    m_timeline = new TimelineWidget;
    m_timeline->setArtwork(m_artwork);
    // Track heights are remembered per project, on this Mac only.
    m_timeline->setHeightsKey(m_projectDirectory);
    auto *timelineScroll = new TimelineScrollArea;
    m_timelineScroll = timelineScroll;
    timelineScroll->setWidgetResizable(true);
    timelineScroll->setFrameShape(QFrame::NoFrame);
    timelineScroll->setHorizontalScrollBarPolicy(Qt::ScrollBarAsNeeded);
    timelineScroll->setWidget(m_timeline);
    timelineScroll->viewport()->installEventFilter(this);
    timelineLayout->addWidget(timelineScroll);
    timelinePanelLayout->addWidget(timelineFrame, 1);
    m_askPrompt = new AskPrompt(this);
    connect(m_askPrompt, &AskPrompt::submitted, this, &MainWindow::askSubmitted);
}

// The timeline's gestures, and the viewer's scrubs that show on it.
void MainWindow::wireTimeline() {
    // The bar's tools, the review on the ruler, QC, previews, solo and gain.
    m_timelineWiring->attach();
    // Insert and Overwrite follow the heads' target toggles (sourceTargets).
    m_timeline->setTargetsEditable(true);
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
    // Selecting a clip opens nothing by itself.
    connect(m_timeline, &TimelineWidget::clipSelected, this, &MainWindow::selectClip);
    connect(m_timeline, &TimelineWidget::clipSelectionExtended, this, &MainWindow::extendClipSelection);
    connect(m_timeline, &TimelineWidget::deselectRequested, this, [this] { selectClip(QString()); });
    connect(m_timeline, &TimelineWidget::moveRequested, this, &MainWindow::moveClip);
    connect(m_timeline, &TimelineWidget::trimRequested, this, &MainWindow::trimClip);
    connect(m_timeline, &TimelineWidget::clipActivated, this, [this](const QString &id) {
        // Double-click opens the clip's media in the source monitor at its in point.
        const QJsonObject clip = clipObject(id);
        const QString media = clip.value(QStringLiteral("sourceId")).toString();
        if (media.isEmpty()) return;
        openSource(media);
        m_sourceMonitor->seek(clip.value(QStringLiteral("in")).toDouble());
    });
    connect(m_timeline, &TimelineWidget::askRequested, this, [this](const QString &id) {
        selectClip(id);
        triggerCommand(QStringLiteral("ai.ask"));
    });
    connect(m_timeline, &TimelineWidget::clipMenuRequested, this, [this](const QString &, const QPoint &at) {
        QMenu menu(this);
        for (const QString &id : {QStringLiteral("ai.ask"), QString(), QStringLiteral("clip.split"), QStringLiteral("clip.delete"),
                                  QStringLiteral("clip.rippleDelete"), QString(), QStringLiteral("clip.focusRegion")}) {
            if (id.isEmpty()) menu.addSeparator();
            else menu.addAction(m_commands->action(id));
        }
        if (canStartMotionPath(selectedClipObject())) menu.addAction(m_commands->action(QStringLiteral("clip.startPath")));
        menu.exec(at);
    });
    connect(m_timeline, &TimelineWidget::razorRequested, this, [this](const QString &id, double frame) {
        execute(QStringLiteral("clip.split"), QJsonObject{{QStringLiteral("clip"), id}, {QStringLiteral("at"), std::llround(frame)}});
    });
    connect(m_timeline, &TimelineWidget::trackToggleRequested, this, [this](const QString &track, const QString &field, bool value) {
        execute(QStringLiteral("track.set"), QJsonObject{{QStringLiteral("track"), track}, {field, value}});
    });
}

} // namespace kerf
