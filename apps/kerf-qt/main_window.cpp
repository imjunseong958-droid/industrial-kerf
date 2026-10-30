#include "main_window.h"
#include "ask_prompt.h"
#include "bridge.h"
#include "command_registry.h"
#include "effect_controls_panel.h"
#include "effects_panel.h"
#include "presets_panel.h"
#include "project_launcher.h"
#include "project_panel.h"
#include "source_monitor_panel.h"
#include "workspace.h"
#include "editor_common.h"
#include "editor_theme.h"
#include "media_artwork.h"
#include "timeline_widget.h"
#include "viewer_widget.h"

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
#include <QMenu>
#include <QMenuBar>
#include <QMessageBox>
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
#include <QSignalBlocker>
#include <QStatusBar>
#include <QTextBlock>
#include <QTextDocument>
#include <QTextFrame>
#include <QToolBar>
#include <QUrl>
#include <QVBoxLayout>

#include <DockManager.h>
#include <DockWidget.h>

#include <algorithm>
#include <limits>
#include <utility>

namespace kerf {

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
        painter.setBrush(isEnabled() ? editorTheme().glyph : editorTheme().glyphDisabled);
        const qreal centerX = width() - 15.0;
        const qreal centerY = height() / 2.0;
        painter.drawPolygon(QPolygonF{{centerX - 4.5, centerY - 2.5},
                                      {centerX + 4.5, centerY - 2.5},
                                      {centerX, centerY + 2.5}});
    }
};

// Toolbar symbols drawn on a shared grid so their weight and optical size match.
QIcon toolbarIcon(const QString &glyph) {
    QPixmap pixmap(40, 40);
    pixmap.setDevicePixelRatio(2);
    pixmap.fill(Qt::transparent);
    QPainter painter(&pixmap);
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setPen(QPen(editorTheme().glyph, 1.5, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
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
}

// The assistant's welcome, shown until the first message.
QString conversationIntro(const EditorTheme &t) {
    return QStringLiteral(
        "<p style='margin-top:2px; color:%1; font-size:10px;'>YOUR EDITING PARTNER</p>"
        "<p style='margin-top:2px; color:%2; font-size:14px; font-weight:600;'>Make your next cut.</p>"
        "<p style='margin-top:4px; color:%3;'>Describe the change. Kerf uses this timeline.</p>"
        "<p style='margin-top:10px; color:%4; font-size:10px;'>TRY ASKING</p>"
        "<p style='margin-top:2px; color:%5;'>“Trim the opening to three seconds.”</p>"
        "<p style='margin-top:2px; color:%5;'>“Lower the music under the dialogue.”</p>"
        "<p style='margin-top:10px; color:%3; font-size:11px;'>Review a proposal before it changes the cut.</p>")
        .arg(t.assistantText.name(), t.textStrong.name(), t.textSecondary.name(), t.textTertiary.name(), t.text.name());
}

// Conversation messages carry their kind, so a change of appearance can
// recolour the ones already shown.
enum class MessageKind { User = 1, Agent, Error };
constexpr int kMessageKind = QTextFormat::UserProperty + 1;

void styleMessage(QTextFrameFormat &frame, MessageKind kind) {
    const EditorTheme &t = editorTheme();
    frame.setBackground(kind == MessageKind::User ? t.messageUser : kind == MessageKind::Error ? t.errorSurface : t.messageAgent);
    frame.setBorderBrush(kind == MessageKind::User ? t.messageUserBorder : kind == MessageKind::Error ? t.errorBorder
                                                                                                 : t.messageAgentBorder);
}

QColor messageRoleColor(MessageKind kind) {
    const EditorTheme &t = editorTheme();
    return kind == MessageKind::User ? t.messageUserRole : kind == MessageKind::Error ? t.errorText : t.assistantText;
}

} // namespace

MainWindow::MainWindow(const QString &projectDirectory, const QString &kerfdBinary, QWidget *parent)
    : QMainWindow(parent), m_projectDirectory(projectDirectory), m_kerfdBinary(kerfdBinary) {
    setWindowTitle(QStringLiteral("Industrial Kerf — Editor"));
    resize(1500, 940);
    setMinimumSize(1080, 700);
    buildUi();
    setupActions();
    setupStatusBar();
    connect(EditorAppearance::instance(), &EditorAppearance::changed, this, &MainWindow::applyAppearance);
    m_exactSeekTimer.setSingleShot(true);
    connect(&m_exactSeekTimer, &QTimer::timeout, this, &MainWindow::settlePointerSeek);
    setConnection(QStringLiteral("connecting"));
#ifndef KERF_QT_TESTING
    m_bridge = kerf_bridge_new(m_projectDirectory.toUtf8().constData(), m_kerfdBinary.toUtf8().constData());
    if (!m_bridge) {
        setConnection(QStringLiteral("disconnected"), QStringLiteral("Could not start the runtime connection."));
    }
    m_pollTimer.setInterval(8);
    connect(&m_pollTimer, &QTimer::timeout, this, &MainWindow::pollBridge);
    m_pollTimer.start();
#endif
}

MainWindow::~MainWindow() {
    if (m_bridge) kerf_bridge_free(m_bridge);
}

void MainWindow::pollBridge() {
    flushSeek();
    if (!m_bridge) return;
    for (int count = 0; count < 32; ++count) {
        char *raw = kerf_bridge_poll(m_bridge);
        if (!raw) break;
        const QByteArray bytes(raw);
        kerf_bridge_string_free(raw);
        const QJsonDocument doc = QJsonDocument::fromJson(bytes);
        if (doc.isObject()) handleEvent(doc.object());
    }
    KerfFrame frame{};
    if (kerf_bridge_frame(m_bridge, &frame) && m_connected && frame.pixels && frame.length > 0 && frame.width > 0 && frame.height > 0) {
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
    if (!execute(QStringLiteral("file.import"), QJsonObject{{QStringLiteral("path"), path}})) return;
    addActivity(QStringLiteral("import"), QStringLiteral("Importing %1").arg(QFileInfo(path).fileName()));
}

void MainWindow::exportProject() {
    const QString quality = m_quality->currentData().toString();
    if (!execute(QStringLiteral("file.export"), QJsonObject{{QStringLiteral("quality"), quality}})) return;
    addActivity(QStringLiteral("export"), QStringLiteral("Export started (%1)").arg(m_quality->currentText()));
}

void MainWindow::exportEditingPreset() {
    QFileDialog dialog(this, QStringLiteral("Export editing preset"),
        QDir(m_projectDirectory).filePath(QStringLiteral("editing.kerfpreset")), QStringLiteral("Kerf editing preset (*.kerfpreset)"));
    dialog.setAcceptMode(QFileDialog::AcceptSave);
    dialog.setDefaultSuffix(QStringLiteral("kerfpreset"));
    if (dialog.exec() != QDialog::Accepted) return;
    const QString path = dialog.selectedFiles().value(0);
    if (QFileInfo(path).suffix() != QStringLiteral("kerfpreset")) {
        showError(QStringLiteral("Save editing presets with the .kerfpreset extension."));
        return;
    }
    execute(QStringLiteral("file.exportPreset"), QJsonObject{{QStringLiteral("path"), path}});
}

void MainWindow::importEditingPreset() {
    const QString path = QFileDialog::getOpenFileName(this, QStringLiteral("Import editing preset"),
        m_projectDirectory, QStringLiteral("Kerf editing preset (*.kerfpreset)"));
    if (!path.isEmpty()) sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("preset_inspect")},
                                               {QStringLiteral("path"), path}});
}

void MainWindow::showPresetBindings(const QString &path, const QJsonObject &preset) {
    QDialog dialog(this);
    dialog.setObjectName(QStringLiteral("presetBindingsDialog"));
    dialog.setWindowTitle(QStringLiteral("Apply preset — %1").arg(preset.value(QStringLiteral("name")).toString()));
    dialog.resize(640, 620);
    auto *layout = new QVBoxLayout(&dialog);
    auto *intro = new QLabel(QStringLiteral("Choose media for each slot. Kerf keeps the preset's timing, effects and animation. "
        "Applying replaces this sequence as a proposal for you to review."));
    intro->setWordWrap(true);
    layout->addWidget(intro);
    auto *scroll = new QScrollArea;
    scroll->setWidgetResizable(true);
    auto *contents = new QWidget;
    auto *form = new QFormLayout(contents);
    QHash<QString, QComboBox *> media;
    QHash<QString, QDoubleSpinBox *> starts;
    QHash<QString, QLineEdit *> texts;
    QHash<QString, QJsonObject> sources;
    for (const QJsonValue &value : m_project.value(QStringLiteral("sources")).toArray())
        sources.insert(value.toObject().value(QStringLiteral("id")).toString(), value.toObject());
    // Kerf lists the media able to fill each slot and the latest frame each can
    // start from. Starts are shown in seconds but sent as whole frames.
    constexpr int latestFrameRole = Qt::UserRole + 1;
    constexpr int fpsRole = Qt::UserRole + 2;
    const QString root = preset.value(QStringLiteral("sequence")).toString();
    for (const QJsonValue &value : preset.value(QStringLiteral("slots")).toArray()) {
        const QJsonObject slot = value.toObject();
        const QString id = slot.value(QStringLiteral("id")).toString();
        auto *choice = new QComboBox;
        choice->setObjectName(QStringLiteral("presetMedia_%1").arg(id));
        choice->addItem(QStringLiteral("Choose imported media…"), QString());
        for (const QJsonValue &fitValue : slot.value(QStringLiteral("fits")).toArray()) {
            const QJsonObject fit = fitValue.toObject();
            const QJsonObject source = sources.value(fit.value(QStringLiteral("media")).toString());
            if (source.isEmpty()) continue;
            const QString sourceId = source.value(QStringLiteral("id")).toString();
            choice->addItem(QStringLiteral("%1 — %2").arg(sourceId,
                QFileInfo(source.value(QStringLiteral("path")).toString()).fileName()), sourceId);
            choice->setItemData(choice->count() - 1, fit.value(QStringLiteral("latestFrame")).toInteger(), latestFrameRole);
            choice->setItemData(choice->count() - 1, source.value(QStringLiteral("fps")).toDouble(), fpsRole);
        }
        if (choice->count() == 1) choice->setItemText(0, QStringLiteral("Import matching media first"));
        auto *from = new QDoubleSpinBox;
        from->setObjectName(QStringLiteral("presetFrom_%1").arg(id));
        from->setDecimals(3);
        from->setSuffix(QStringLiteral(" s"));
        from->setRange(0, 0);
        from->setToolTip(QStringLiteral("Start in the replacement media, including the preset's transition handles"));
        connect(choice, &QComboBox::currentIndexChanged, from, [choice, from] {
            const double fps = choice->currentData(fpsRole).toDouble();
            from->setSingleStep(fps > 0 ? 1 / fps : 1);
            from->setMaximum(fps > 0 ? choice->currentData(latestFrameRole).toDouble() / fps : 0);
        });
        QStringList uses;
        for (const QJsonValue &clipValue : slot.value(QStringLiteral("clips")).toArray()) {
            const QJsonObject clip = clipValue.toObject();
            QString use = QStringLiteral("%1 %2").arg(clip.value(QStringLiteral("track")).toString(),
                                                      clip.value(QStringLiteral("at")).toString());
            const QString sequence = clip.value(QStringLiteral("sequence")).toString();
            if (sequence != root) use = QStringLiteral("%1 › %2").arg(sequence, use);
            const QString name = clip.value(QStringLiteral("name")).toString();
            if (!name.isEmpty()) use += QStringLiteral(" “%1”").arg(name);
            uses << use;
        }
        auto *usedBy = new QLabel(uses.join(QStringLiteral(" · ")));
        usedBy->setObjectName(QStringLiteral("muted"));
        usedBy->setWordWrap(true);
        auto *row = new QWidget;
        auto *rowLayout = new QGridLayout(row);
        rowLayout->setContentsMargins(0, 0, 0, 0);
        rowLayout->addWidget(choice, 0, 0);
        rowLayout->addWidget(from, 0, 1);
        rowLayout->addWidget(usedBy, 1, 0, 1, 2);
        rowLayout->setColumnStretch(0, 1);
        form->addRow(QStringLiteral("%1 · %2 s").arg(id).arg(slot.value(QStringLiteral("seconds")).toDouble(), 0, 'f', 3), row);
        media.insert(id, choice);
        starts.insert(id, from);
    }
    const QJsonObject textSlots = preset.value(QStringLiteral("textSlots")).toObject();
    for (auto it = textSlots.begin(); it != textSlots.end(); ++it) {
        auto *text = new QLineEdit(it.value().toString());
        text->setObjectName(QStringLiteral("presetText_%1").arg(it.key()));
        form->addRow(it.key(), text);
        texts.insert(it.key(), text);
    }
    scroll->setWidget(contents);
    layout->addWidget(scroll, 1);
    auto *code = new QPlainTextEdit(preset.value(QStringLiteral("document")).toString());
    code->setObjectName(QStringLiteral("presetCode"));
    code->setReadOnly(true);
    code->setFont(QFontDatabase::systemFont(QFontDatabase::FixedFont));
    code->setMaximumHeight(150);
    layout->addWidget(code);
    auto *buttons = new QDialogButtonBox(QDialogButtonBox::Ok | QDialogButtonBox::Cancel);
    buttons->button(QDialogButtonBox::Ok)->setText(QStringLiteral("Create proposal"));
    const auto updateReady = [buttons, media] {
        bool ready = true;
        for (auto *choice : media) ready = ready && !choice->currentData().toString().isEmpty();
        buttons->button(QDialogButtonBox::Ok)->setEnabled(ready);
    };
    for (auto *choice : media) connect(choice, &QComboBox::currentIndexChanged, buttons, updateReady);
    updateReady();
    connect(buttons, &QDialogButtonBox::accepted, &dialog, &QDialog::accept);
    connect(buttons, &QDialogButtonBox::rejected, &dialog, &QDialog::reject);
    layout->addWidget(buttons);
    const QJsonValue revision = revisionValue();
    if (dialog.exec() != QDialog::Accepted) return;
    if (revision != revisionValue()) {
        showError(QStringLiteral("The project changed while choosing preset media. Import the preset again."));
        return;
    }
    QJsonObject bindings;
    for (auto it = media.begin(); it != media.end(); ++it) {
        // Clamp: the shown maximum is rounded to the spin box's decimals.
        const qint64 frame = std::clamp<qint64>(
            qRound64(starts.value(it.key())->value() * it.value()->currentData(fpsRole).toDouble()),
            0, it.value()->currentData(latestFrameRole).toLongLong());
        bindings.insert(it.key(), QJsonObject{{QStringLiteral("media"), it.value()->currentData().toString()},
            {QStringLiteral("from"), QStringLiteral("%1f").arg(frame)}});
    }
    QJsonObject overrides;
    for (auto it = texts.begin(); it != texts.end(); ++it) {
        if (it.value()->isModified()) overrides.insert(it.key(), it.value()->text());
    }
    execute(QStringLiteral("file.applyPreset"), QJsonObject{{QStringLiteral("path"), path},
        {QStringLiteral("bindings"), bindings}, {QStringLiteral("texts"), overrides}});
}

void MainWindow::renderPreview() {
    if (!execute(QStringLiteral("sequence.renderPreview"))) return;
    addActivity(QStringLiteral("preview"), QStringLiteral("Rendering preview"));
    m_previewStatus = QStringLiteral("running");
    updateCommandStates();
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
    if (!execute(QStringLiteral("sequence.setFormat"),
                 QJsonObject{{QStringLiteral("width"), width}, {QStringLiteral("height"), height}}))
        syncCanvasPreset(m_canvasSize);
}

void MainWindow::addTrack() {
    QMenu menu(this);
    auto *video = menu.addAction(QStringLiteral("Add video track"));
    auto *audio = menu.addAction(QStringLiteral("Add audio track"));
    const QAction *choice = menu.exec(m_timeline->mapToGlobal(m_timeline->addLaneButtonCenter()));
    if (choice != video && choice != audio) return;
    execute(choice == audio ? QStringLiteral("sequence.addAudioTrack") : QStringLiteral("sequence.addVideoTrack"));
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
    updateCommandStates();
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
    updateCommandStates();
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
                statusBar()->showMessage(QStringLiteral("A roll needs a clip on the other side of the cut"), 4000);
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

void MainWindow::openSource(const QString &mediaId) {
    for (const QJsonValue &value : m_project.value(QStringLiteral("sources")).toArray()) {
        const QJsonObject source = value.toObject();
        if (source.value(QStringLiteral("id")).toString() != mediaId) continue;
        m_sourceMonitor->open(source, QDir(m_projectDirectory).absoluteFilePath(source.value(QStringLiteral("path")).toString()));
        if (ads::CDockWidget *panel = m_workspace->panel(QStringLiteral("source"))) {
            panel->toggleView(true);
            panel->setAsCurrentTab();
        }
        m_sourceMonitor->setFocus();
        return;
    }
}

QStringList MainWindow::sourceTargets() const {
    // The first unlocked track of each kind the source media has.
    const QJsonObject source = m_sourceMonitor->source();
    QStringList targets;
    for (const QString &kind : {QStringLiteral("video"), QStringLiteral("audio")}) {
        const bool has = kind == QStringLiteral("video") ? source.contains(QStringLiteral("width"))
                                                         : source.value(QStringLiteral("hasAudio")).toBool();
        if (!has) continue;
        for (const QJsonValue &value : m_project.value(QStringLiteral("tracks")).toArray()) {
            const QJsonObject track = value.toObject();
            if (track.value(QStringLiteral("type")).toString() == kind && !track.value(QStringLiteral("locked")).toBool()) {
                targets.append(track.value(QStringLiteral("id")).toString());
                break;
            }
        }
    }
    return targets;
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
    execute(QStringLiteral("clip.delete"));
}

void MainWindow::sendChat() {
    const QString text = m_chatInput->toPlainText().trimmed();
    if (text.isEmpty() || !sendAgent(text, QJsonObject{}, QString())) return;
    m_chatInput->clear();
}

bool MainWindow::sendAgent(const QString &text, const QJsonObject &focus, const QString &focusLabel) {
    QJsonObject request = agentSettingsPayload();
    request.insert(QStringLiteral("type"), QStringLiteral("agent"));
    request.insert(QStringLiteral("text"), text);
    if (!focus.isEmpty()) request.insert(QStringLiteral("focus"), focus);
    // The AI bar names what Kerf works on once the bridge reports it busy.
    m_agentFocusLabel = focusLabel;
    if (!sendSimple(request)) return false;
    m_lastFocus = focus;
    m_lastFocusLabel = focusLabel;
    showNextSteps(QJsonArray{});
    m_agentBusy = true;
    m_agentText.clear();
    m_agentStatus->setText(QStringLiteral("WORKING"));
    m_chatInput->setEnabled(false);
    m_abortButton->setEnabled(true);
    updateChatControls();
    if (!focusLabel.isEmpty()) appendConversationNote(QStringLiteral("Focus"), focusLabel);
    appendConversation(QStringLiteral("You"), text);
    appendConversationNote(QStringLiteral("Progress"), QStringLiteral("Request sent"));
    addActivity(QStringLiteral("agent"), QStringLiteral("Request sent"));
    updateAiBar();
    updateCommandStates();
    return true;
}

bool MainWindow::usesCloud() const {
    return m_cloudRadio && m_cloudRadio->isChecked();
}

bool MainWindow::sendCloud(const QJsonObject &command) {
    if (!m_bridge) {
        m_lastCommand = command;
        return false;
    }
    const QByteArray payload = QJsonDocument(command).toJson(QJsonDocument::Compact);
    return kerf_bridge_send(m_bridge, payload.constData());
}

QString MainWindow::aiAccess() const {
    if (usesCloud()) return m_cloudAccess.isEmpty() ? QStringLiteral("unavailable") : m_cloudAccess;
    const bool openai = activeProviderId() == QStringLiteral("openai");
    const QString key = (openai ? m_openaiKey : m_anthropicKey)->text().trimmed();
    const bool environment = !qEnvironmentVariable(openai ? "OPENAI_API_KEY" : "ANTHROPIC_API_KEY").isEmpty();
    return key.isEmpty() && !environment ? QStringLiteral("noKey") : QStringLiteral("ready");
}

void MainWindow::updateCloudStatus() {
    if (!m_cloudStatus) return;
    const QString email = m_cloudAccount.value(QStringLiteral("email")).toString();
    const bool signedIn = !email.isEmpty() && m_cloudAccess != QStringLiteral("signedOut");
    QString text;
    if (m_cloudAccess == QStringLiteral("unavailable") || m_cloudAccess.isEmpty()) {
        text = QStringLiteral("Kerf Cloud isn't available yet. Until it is, choose My own API key and add it under Advanced.");
    } else if (!signedIn) {
        text = m_cloudAccess == QStringLiteral("unreachable") ? QStringLiteral("Kerf Cloud can't be reached right now.")
                                                               : QStringLiteral("Signed out. Paste your Kerf Cloud token to sign in.");
    } else {
        const QJsonValue credits = m_cloudAccount.value(QStringLiteral("credits"));
        text = QStringLiteral("Signed in as %1  ·  %2 plan%3").arg(email, m_cloudAccount.value(QStringLiteral("plan")).toString(),
            credits.isDouble() ? QStringLiteral("  ·  %1 requests left").arg(credits.toInteger()) : QString());
        if (m_cloudAccess == QStringLiteral("notIncluded")) text += QStringLiteral("\nThis plan doesn't include Kerf Cloud requests.");
        if (m_cloudAccess == QStringLiteral("outOfCredits")) text += QStringLiteral("\nYou've used this period's requests.");
    }
    if (!m_cloudMessage.isEmpty()) text += QStringLiteral("\n") + m_cloudMessage;
    m_cloudStatus->setText(text);
    const bool available = !m_cloudAccess.isEmpty() && m_cloudAccess != QStringLiteral("unavailable");
    m_cloudToken->setVisible(available && !signedIn);
    m_cloudSignIn->setVisible(available && !signedIn);
    m_cloudSignOut->setVisible(available && signedIn);
    if (m_statusAccount) {
        m_statusAccount->setText(!usesCloud() ? QStringLiteral("Own API key")
            : signedIn ? QStringLiteral("Kerf Cloud · %1").arg(m_cloudAccount.value(QStringLiteral("plan")).toString())
            : QStringLiteral("Kerf Cloud · signed out"));
    }
    updateCommandStates();
}

void MainWindow::askAi() {
    // Anchor to the selected clip, or to the timeline at the playhead.
    const QJsonObject clip = selectedClipObject();
    const QRect bounds = clip.isEmpty() ? QRect() : m_timeline->clipBounds(m_selectedClip);
    const QRect visible = m_timelineScroll->viewport()->rect();
    QPoint anchor = m_timelineScroll->viewport()->mapToGlobal(QPoint(visible.center().x(), 48));
    if (!bounds.isEmpty()) {
        const QPoint top = m_timeline->mapTo(m_timelineScroll->viewport(), bounds.topLeft() + QPoint(bounds.width() / 2, 0));
        if (visible.contains(QPoint(std::clamp(top.x(), 0, visible.width() - 1), std::clamp(top.y(), 0, visible.height() - 1))))
            anchor = m_timelineScroll->viewport()->mapToGlobal(QPoint(std::clamp(top.x(), 24, visible.width() - 24), std::max(top.y(), 24)));
    }
    if (clip.isEmpty()) {
        m_askPrompt->open(anchor, QStringLiteral("Sequence at %1").arg(timelineTimecode(m_playhead, m_fps)), false);
        return;
    }
    const double start = clip.value(QStringLiteral("timelineStart")).toDouble();
    const double end = start + clip.value(QStringLiteral("duration")).toDouble();
    m_askPrompt->open(anchor, QStringLiteral("%1  ·  %2–%3").arg(clipName(clip), timelineTimecode(start, m_fps), timelineTimecode(end, m_fps)), true);
}

void MainWindow::askSubmitted(const QString &text, const QString &scope) {
    const QJsonObject clip = selectedClipObject();
    QJsonObject focus{{QStringLiteral("scope"), scope}};
    QString label;
    if (!clip.isEmpty()) {
        const qint64 start = clipFrames(clip, QStringLiteral("startFrame"));
        focus.insert(QStringLiteral("clip"), m_selectedClip);
        focus.insert(QStringLiteral("fromFrame"), start);
        focus.insert(QStringLiteral("toFrame"), start + clipFrames(clip, QStringLiteral("durationFrames")));
        label = scope == QStringLiteral("clip") ? clipName(clip) : QStringLiteral("Sequence around %1").arg(clipName(clip));
    } else {
        focus.insert(QStringLiteral("fromFrame"), frameAt(m_playhead));
        focus.insert(QStringLiteral("toFrame"), frameAt(m_playhead));
        label = QStringLiteral("Sequence at %1").arg(timelineTimecode(m_playhead, m_fps));
    }
    sendAgent(text, focus, label);
}

void MainWindow::updateAiBar() {
    if (!m_aiBar) return;
    const bool pending = !m_proposalId.isEmpty();
    const bool next = !m_steps.isEmpty() && !m_agentBusy;
    m_aiBar->setVisible(m_connected && (m_agentBusy || pending || next));
    m_nextSteps->setVisible(next);
    if (m_agentBusy) {
        m_aiBarText->setText(m_agentFocusLabel.isEmpty() ? QStringLiteral("Kerf is working…")
                                                         : QStringLiteral("Kerf is working on %1…").arg(m_agentFocusLabel));
    } else if (pending) {
        m_aiBarText->setText(QStringLiteral("Proposal  ·  %1").arg(m_proposalSummary->text()));
    } else if (next) {
        m_aiBarText->setText(QStringLiteral("Next with Kerf"));
    }
    for (QToolButton *button : m_aiBar->findChildren<QToolButton *>()) {
        const QString id = button->defaultAction() ? button->defaultAction()->objectName() : QString();
        button->setVisible(id == QStringLiteral("ai.stop") ? m_agentBusy : id.startsWith(QStringLiteral("proposal.")) ? pending && !m_agentBusy : true);
    }
}

void MainWindow::showNextSteps(const QJsonArray &steps) {
    m_steps = steps;
    for (QWidget *row : {m_nextSteps, m_chatNextSteps}) {
        while (QLayoutItem *item = row->layout()->takeAt(0)) {
            // A pressed button sends the request that clears it.
            if (item->widget()) item->widget()->deleteLater();
            delete item;
        }
        for (const QJsonValue &value : steps) {
            const QJsonObject step = value.toObject();
            const QString request = step.value(QStringLiteral("request")).toString();
            auto *button = new QPushButton(step.value(QStringLiteral("label")).toString());
            button->setObjectName(QStringLiteral("nextStep"));
            button->setToolTip(request);
            button->setCursor(Qt::PointingHandCursor);
            connect(button, &QPushButton::clicked, this, [this, request] {
                if (!m_agentBusy) sendAgent(request, m_lastFocus, m_lastFocusLabel);
            });
            row->layout()->addWidget(button);
        }
    }
    m_chatNextSteps->setVisible(!steps.isEmpty());
    updateAiBar();
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
                             : m_agentBusy ? QStringLiteral("Kerf is working…")
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
    if (usesCloud()) {
        // The service picks the model; MCP tools still run locally.
        return QJsonObject{{QStringLiteral("cloud"), true},
                           {QStringLiteral("mcpEnabled"), m_mcpEnabled && m_mcpEnabled->isChecked()},
                           {QStringLiteral("mcpConfigVersion"), static_cast<qint64>(m_mcpConfigVersion)}};
    }
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
    if (!project.mkpath(QStringLiteral(".kerf"))) {
        m_inboundMcpStatus->setText(QStringLiteral("Could not create the project's .kerf directory."));
        return;
    }
    QSaveFile file(project.filePath(QStringLiteral(".kerf/mcp.json")));
    if (!file.open(QIODevice::WriteOnly) || file.write(raw) != raw.size() || !file.commit()) {
        m_inboundMcpStatus->setText(QStringLiteral("Could not save MCP config: %1").arg(file.errorString()));
        return;
    }
    ++m_mcpConfigVersion;
    m_inboundMcpStatus->setText(QStringLiteral("Saved project .kerf/mcp.json. Applies on the next chat request when MCP is enabled."));
}

void MainWindow::updateAgentModelChoices() {
    if (!m_modelSelect) return;
    m_modelSelect->setEnabled(!usesCloud());
    if (usesCloud()) {
        const QSignalBlocker blocker(m_modelSelect);
        m_modelSelect->clear();
        m_modelSelect->addItem(QStringLiteral("Kerf Cloud"));
        m_modelSelect->setToolTip(QStringLiteral("Kerf Cloud picks the model"));
        return;
    }
    m_modelSelect->setToolTip(QStringLiteral("Model ID sent with each chat request. Suggestions follow the active connection in Settings."));
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
                                             qEnvironmentVariable("KERF_PROVIDER", envOpenaiKey.isEmpty() ? QStringLiteral("anthropic") : QStringLiteral("openai")))
                                 .toString()
                                 .toLower();
    if (provider == QStringLiteral("openai"))
        m_openaiActiveRadio->setChecked(true);
    else
        m_anthropicActiveRadio->setChecked(true);
    // Kerf Cloud unless a key of one's own is already set up, so existing
    // setups keep working; an explicit choice wins.
    const bool hasOwnKey = !m_anthropicKey->text().trimmed().isEmpty() || !m_openaiKey->text().trimmed().isEmpty();
    const bool cloud = settings.value(QStringLiteral("agent/connection"), hasOwnKey ? QStringLiteral("own") : QStringLiteral("cloud"))
                           .toString() != QStringLiteral("own");
    (cloud ? m_cloudRadio : m_ownKeyRadio)->setChecked(true);
    updateCloudStatus();
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
    execute(QStringLiteral("proposal.accept"));
}

void MainWindow::rejectProposal() {
    execute(QStringLiteral("proposal.reject"));
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
            m_bridge = kerf_bridge_new(m_projectDirectory.toUtf8().constData(), m_kerfdBinary.toUtf8().constData());
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

    // A git pull in a terminal or a text editor changed project.kerf; kerfd
    // keeps the editor's version until one of them is chosen.
    m_divergedBanner = new QFrame;
    m_divergedBanner->setObjectName(QStringLiteral("divergedBanner"));
    auto *divergedLayout = new QHBoxLayout(m_divergedBanner);
    divergedLayout->setContentsMargins(10, 4, 8, 4);
    auto *divergedText = new QLabel(QStringLiteral("<b>project.kerf changed on disk.</b> Load it to bring in edits made outside Kerf "
                                                   "(a git pull, a text editor), or keep the editor's version and write it over the file."));
    divergedText->setObjectName(QStringLiteral("divergedMessage"));
    divergedText->setWordWrap(true);
    divergedLayout->addWidget(divergedText, 1);
    m_loadFromDisk = new QPushButton(QStringLiteral("Load from Disk"));
    m_loadFromDisk->setObjectName(QStringLiteral("loadFromDisk"));
    connect(m_loadFromDisk, &QPushButton::clicked, this, [this] { execute(QStringLiteral("projection.import")); });
    divergedLayout->addWidget(m_loadFromDisk);
    m_keepEditorVersion = new QPushButton(QStringLiteral("Keep Editor Version"));
    m_keepEditorVersion->setObjectName(QStringLiteral("keepEditorVersion"));
    connect(m_keepEditorVersion, &QPushButton::clicked, this, [this] { execute(QStringLiteral("projection.repair")); });
    divergedLayout->addWidget(m_keepEditorVersion);
    m_divergedBanner->hide();
    root->addWidget(m_divergedBanner);

    m_workspace = new Workspace(central);
    root->addWidget(m_workspace->manager(), 1);

    auto *viewerFrame = new QFrame;
    viewerFrame->setObjectName(QStringLiteral("viewerFrame"));
    auto *viewerLayout = new QVBoxLayout(viewerFrame);
    viewerLayout->setContentsMargins(0, 0, 0, 0);
    viewerLayout->setSpacing(0);
    m_viewer = new ViewerWidget;
    viewerLayout->addWidget(m_viewer, 1);
    viewerLayout->addWidget(buildTransport());

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
    auto *timelineRow = new QHBoxLayout;
    timelineRow->setContentsMargins(0, 0, 0, 0);
    timelineRow->setSpacing(0);
    // The editing tools, filled in from the command registry.
    m_toolPalette = new QToolBar(QStringLiteral("Tools"));
    m_toolPalette->setObjectName(QStringLiteral("toolPalette"));
    m_toolPalette->setOrientation(Qt::Vertical);
    m_toolPalette->setToolButtonStyle(Qt::ToolButtonTextOnly);
    timelineRow->addWidget(m_toolPalette);
    timelineRow->addWidget(timelineFrame, 1);
    // What Kerf is doing, and the proposal to review, where the edit happens.
    m_aiBar = new QFrame;
    m_aiBar->setObjectName(QStringLiteral("aiBar"));
    auto *aiLayout = new QHBoxLayout(m_aiBar);
    aiLayout->setContentsMargins(10, 3, 8, 3);
    aiLayout->setSpacing(6);
    m_aiBarText = new QLabel;
    m_aiBarText->setObjectName(QStringLiteral("aiBarText"));
    aiLayout->addWidget(m_aiBarText, 1);
    m_nextSteps = new QWidget;
    m_nextSteps->setObjectName(QStringLiteral("nextSteps"));
    auto *nextLayout = new QHBoxLayout(m_nextSteps);
    nextLayout->setContentsMargins(0, 0, 0, 0);
    nextLayout->setSpacing(6);
    m_nextSteps->hide();
    aiLayout->addWidget(m_nextSteps);
    m_aiBar->hide();
    timelinePanelLayout->addWidget(m_aiBar);
    timelinePanelLayout->addLayout(timelineRow, 1);
    m_askPrompt = new AskPrompt(this);
    connect(m_askPrompt, &AskPrompt::submitted, this, &MainWindow::askSubmitted);
    timelinePanel->setMinimumHeight(220);

    m_projectPanel = new ProjectPanel(m_artwork);
    connect(m_projectPanel, &ProjectPanel::openRequested, this, &MainWindow::openSource);
    connect(m_projectPanel, &ProjectPanel::menuRequested, this, [this](const QString &media, const QPoint &at) {
        QMenu *menu = binMenu(media);
        menu->exec(at);
        menu->deleteLater();
    });
    m_presets = new PresetsPanel;
    connect(m_presets, &PresetsPanel::applyRequested, this, &MainWindow::applyLibraryPreset);
    m_sourceMonitor = new SourceMonitorPanel;
    connect(m_sourceMonitor, &SourceMonitorPanel::changed, this, &MainWindow::updateCommandStates);

    auto *right = new QWidget;
    right->setObjectName(QStringLiteral("assistantPanel"));
    right->setAttribute(Qt::WA_StyledBackground);
    right->setMinimumWidth(248);
    auto *rightLayout = new QVBoxLayout(right);
    rightLayout->setContentsMargins(8, 2, 8, 8);
    auto *chatTab = new QWidget;
    rightLayout->addWidget(chatTab);
    auto *chatTabLayout = new QVBoxLayout(chatTab);
    chatTabLayout->setContentsMargins(0, 6, 0, 0);
    chatTabLayout->setSpacing(6);
    auto *statusRow = new QHBoxLayout;
    statusRow->setContentsMargins(0, 0, 0, 0);
    auto *preferencesLink = new QToolButton;
    preferencesLink->setObjectName(QStringLiteral("assistantSettings"));
    preferencesLink->setText(QStringLiteral("Agent settings…"));
    preferencesLink->setToolTip(QStringLiteral("Provider, model, and MCP settings"));
    connect(preferencesLink, &QToolButton::clicked, this, [this] { triggerCommand(QStringLiteral("app.preferences")); });
    statusRow->addWidget(preferencesLink);
    statusRow->addStretch(1);
    m_agentStatus = new QLabel(QStringLiteral("READY"));
    m_agentStatus->setObjectName(QStringLiteral("statusPill"));
    statusRow->addWidget(m_agentStatus);
    chatTabLayout->addLayout(statusRow);
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
    m_conversation->setAccessibleName(QStringLiteral("Conversation with Kerf"));
    m_conversation->setMinimumHeight(80);
    m_conversation->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_conversation->document()->setDocumentMargin(2);
    m_conversation->setHtml(conversationIntro(editorTheme()));
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
    m_chatInput->setPlaceholderText(QStringLiteral("Ask Kerf to edit…"));
    m_chatInput->setAccessibleName(QStringLiteral("Message Kerf"));
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
        if (!execute(QStringLiteral("ai.stop"))) return;
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
    m_chatNextSteps = new QWidget;
    m_chatNextSteps->setObjectName(QStringLiteral("nextSteps"));
    auto *chatNextLayout = new QHBoxLayout(m_chatNextSteps);
    chatNextLayout->setContentsMargins(0, 0, 0, 0);
    chatNextLayout->setSpacing(6);
    m_chatNextSteps->hide();
    chatTabLayout->addWidget(m_chatNextSteps);
    chatTabLayout->addWidget(composer);
    m_effectControls = new EffectControlsPanel;
    connect(m_effectControls, &EffectControlsPanel::editRequested, this, &MainWindow::sendEdit);
    // Mask tools draw a new mask on the selected picture clip over the
    // Program monitor; the Program panel's title bar and Effect Controls
    // both carry them.
    auto *maskTools = new QActionGroup(this);
    maskTools->setExclusionPolicy(QActionGroup::ExclusionPolicy::ExclusiveOptional);
    struct MaskToolAction {
        MaskTool tool;
        const char *id;
        QString label;
    };
    for (const MaskToolAction &entry : {
             MaskToolAction{MaskTool::Rectangle, "maskRectangle", tr("Rectangle mask")},
             MaskToolAction{MaskTool::Ellipse, "maskEllipse", tr("Ellipse mask")},
             MaskToolAction{MaskTool::Pen, "maskPen",
                            tr("Pen mask: click its corners, then the first again to close it (Return leaves it open)")}}) {
        const MaskTool tool = entry.tool;
        auto *action = new QAction(maskToolIcon(tool), entry.label, maskTools);
        action->setObjectName(QString::fromLatin1(entry.id));
        action->setCheckable(true);
        action->setEnabled(false);
        action->setData(static_cast<int>(tool));
        connect(action, &QAction::toggled, this, [this, tool](bool on) {
            if (on) m_viewer->setMaskTool(tool);
            else if (m_viewer->maskTool() == tool) m_viewer->setMaskTool(MaskTool::None);
        });
        m_maskTools.append(action);
    }
    m_effectControls->setMaskTools(m_maskTools);
    connect(m_viewer, &ViewerWidget::maskToolChanged, this, [this](MaskTool tool) {
        for (QAction *action : m_maskTools) action->setChecked(action->data().toInt() == static_cast<int>(tool));
    });
    connect(m_viewer, &ViewerWidget::editRequested, this, &MainWindow::sendEdit);
    connect(m_viewer, &ViewerWidget::maskSelected, m_effectControls, &EffectControlsPanel::selectMask);
    connect(m_effectControls, &EffectControlsPanel::maskSelected, m_viewer, &ViewerWidget::selectMask);
    // The effects offered: cards on the selected clip, and the parameters
    // the inspector edits.
    m_effects = new EffectsPanel;
    if (char *catalog = kerf_fx_catalog()) {
        const QJsonArray effects = QJsonDocument::fromJson(QByteArray(catalog)).array();
        kerf_bridge_string_free(catalog);
        m_effects->setCatalog(effects);
        m_effectControls->setEffectCatalog(effects);
    }
    connect(m_effects, &EffectsPanel::addRequested, this, [this](const QString &fx) {
        if (m_selectedClip.isEmpty()) return;
        sendEdit(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("fx-add")},
                                        {QStringLiteral("clip"), m_selectedClip},
                                        {QStringLiteral("fx"), fx}}});
    });
    connect(m_effects, &EffectsPanel::previewsWanted, this, [this](const QString &clip, const QStringList &effects) {
        sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("effect_previews")},
                               {QStringLiteral("clip"), clip},
                               {QStringLiteral("at"), m_playhead},
                               {QStringLiteral("effects"), QJsonArray::fromStringList(effects)},
                               {QStringLiteral("width"), 160}});
    });

    // Agent settings live in the Settings dialog, out of the editing surface.
    m_preferences = new QDialog(this);
    m_preferences->setObjectName(QStringLiteral("preferencesDialog"));
    m_preferences->setWindowTitle(QStringLiteral("Settings"));
    m_preferences->resize(520, 680);
    auto *settingsTabLayout = new QVBoxLayout(m_preferences);
    settingsTabLayout->setContentsMargins(12, 12, 12, 12);
    auto *settingsScroll = new QScrollArea;
    settingsScroll->setWidgetResizable(true);
    settingsScroll->setFrameShape(QFrame::NoFrame);
    auto *settingsContent = new QWidget;
    auto *settingsLayout = new QVBoxLayout(settingsContent);
    settingsLayout->setContentsMargins(0, 0, 2, 0);
    settingsLayout->setSpacing(10);
    // Kerf Cloud, the hosted model, comes first. Keys of one's own and MCP
    // live under Advanced.
    auto *connectionGroup = new QGroupBox(QStringLiteral("AI"));
    connectionGroup->setObjectName(QStringLiteral("aiConnection"));
    auto *cloudLayout = new QVBoxLayout(connectionGroup);
    cloudLayout->setContentsMargins(10, 12, 10, 10);
    cloudLayout->setSpacing(6);
    m_cloudRadio = new QRadioButton(QStringLiteral("Kerf Cloud"));
    m_cloudRadio->setObjectName(QStringLiteral("agentConnectionCloud"));
    m_ownKeyRadio = new QRadioButton(QStringLiteral("My own API key (set up under Advanced)"));
    m_ownKeyRadio->setObjectName(QStringLiteral("agentConnectionOwnKey"));
    auto *connectionChoice = new QButtonGroup(connectionGroup);
    connectionChoice->addButton(m_cloudRadio);
    connectionChoice->addButton(m_ownKeyRadio);
    cloudLayout->addWidget(m_cloudRadio);
    cloudLayout->addWidget(m_ownKeyRadio);
    m_cloudStatus = new QLabel;
    m_cloudStatus->setObjectName(QStringLiteral("cloudStatus"));
    m_cloudStatus->setWordWrap(true);
    cloudLayout->addWidget(m_cloudStatus);
    m_cloudToken = new QLineEdit;
    m_cloudToken->setObjectName(QStringLiteral("cloudToken"));
    m_cloudToken->setEchoMode(QLineEdit::Password);
    m_cloudToken->setPlaceholderText(QStringLiteral("Kerf Cloud sign-in token"));
    cloudLayout->addWidget(m_cloudToken);
    auto *cloudButtons = new QHBoxLayout;
    m_cloudSignIn = new QPushButton(QStringLiteral("Sign in"));
    m_cloudSignIn->setObjectName(QStringLiteral("primaryButton"));
    connect(m_cloudSignIn, &QPushButton::clicked, this, [this] {
        const QString token = m_cloudToken->text().trimmed();
        if (token.isEmpty()) return;
        if (sendCloud(QJsonObject{{QStringLiteral("type"), QStringLiteral("cloud_sign_in")}, {QStringLiteral("token"), token}}))
            m_cloudToken->clear();
    });
    m_cloudSignOut = new QPushButton(QStringLiteral("Sign out"));
    connect(m_cloudSignOut, &QPushButton::clicked, this, [this] {
        sendCloud(QJsonObject{{QStringLiteral("type"), QStringLiteral("cloud_sign_out")}});
    });
    auto *plans = new QPushButton(QStringLiteral("Plans"));
    plans->setObjectName(QStringLiteral("cloudPlans"));
    plans->setEnabled(false);
    plans->setToolTip(QStringLiteral("Plans are coming soon"));
    cloudButtons->addWidget(m_cloudSignIn);
    cloudButtons->addWidget(m_cloudSignOut);
    cloudButtons->addStretch(1);
    cloudButtons->addWidget(plans);
    cloudLayout->addLayout(cloudButtons);
    settingsLayout->addWidget(connectionGroup);
    const auto onConnectionChanged = [this](bool checked) {
        if (!checked) return;
        QSettings().setValue(QStringLiteral("agent/connection"), m_cloudRadio->isChecked() ? QStringLiteral("cloud") : QStringLiteral("own"));
        updateAgentModelChoices();
        updateCloudStatus();
    };
    connect(m_cloudRadio, &QRadioButton::toggled, this, onConnectionChanged);
    connect(m_ownKeyRadio, &QRadioButton::toggled, this, onConnectionChanged);

    auto *advancedToggle = new QToolButton;
    advancedToggle->setObjectName(QStringLiteral("advancedToggle"));
    advancedToggle->setText(QStringLiteral("▸ Advanced"));
    settingsLayout->addWidget(advancedToggle);
    m_advancedSettings = new QWidget;
    m_advancedSettings->setObjectName(QStringLiteral("advancedSettings"));
    auto *advancedLayout = new QVBoxLayout(m_advancedSettings);
    advancedLayout->setContentsMargins(0, 0, 0, 0);
    advancedLayout->setSpacing(10);
    settingsLayout->addWidget(m_advancedSettings);
    m_advancedSettings->hide();
    connect(advancedToggle, &QToolButton::clicked, this, [this, advancedToggle] {
        const bool show = m_advancedSettings->isHidden();
        m_advancedSettings->setVisible(show);
        advancedToggle->setText(show ? QStringLiteral("▾ Advanced") : QStringLiteral("▸ Advanced"));
    });
    auto *settingsIntro = new QLabel(QStringLiteral("Two protocol slots. One active connection for chat. Gemini, Groq, and OpenRouter use the OpenAI-compatible slot with that vendor's compatible base URL."));
    settingsIntro->setObjectName(QStringLiteral("chatHint"));
    settingsIntro->setWordWrap(true);
    advancedLayout->addWidget(settingsIntro);
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
    advancedLayout->addWidget(activeConnection);

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
    advancedLayout->addWidget(anthropicGroup);

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
    advancedLayout->addWidget(openaiGroup);
    auto *settingsNote = new QLabel(QStringLiteral("Credentials stay outside project files and are never included in chat activity."));
    settingsNote->setObjectName(QStringLiteral("muted"));
    settingsNote->setWordWrap(true);
    advancedLayout->addWidget(settingsNote);
    auto *saveSettings = new QPushButton(QStringLiteral("Save agent connection"));
    saveSettings->setObjectName(QStringLiteral("primaryButton"));
    connect(saveSettings, &QPushButton::clicked, this, &MainWindow::saveAgentSettings);
    advancedLayout->addWidget(saveSettings);

    auto *inboundSection = new QGroupBox(QStringLiteral("Use MCP tools in Kerf chat"));
    inboundSection->setObjectName(QStringLiteral("inboundMcpSection"));
    auto *inboundLayout = new QVBoxLayout(inboundSection);
    m_mcpEnabled = new QCheckBox(QStringLiteral("Enable external MCP tools"));
    m_mcpEnabled->setObjectName(QStringLiteral("inboundMcpEnabled"));
    const bool environmentMcpEnabled = qEnvironmentVariable("KERF_ENABLE_MCP") == QStringLiteral("1");
    m_mcpEnabled->setChecked(QSettings().value(QStringLiteral("agent/mcpEnabled"), environmentMcpEnabled).toBool());
    inboundLayout->addWidget(m_mcpEnabled);
    auto *inboundIntro = new QLabel(QStringLiteral("Kerf chat calls external MCP servers directly; Cursor is not needed. Your AI provider above still powers the chat. Kerf starts local commands using stdio; URL-only remote servers are not supported. KERF_ENABLE_MCP=1 also enables tools even if this box is unchecked."));
    inboundIntro->setWordWrap(true);
    inboundIntro->setObjectName(QStringLiteral("chatHint"));
    inboundLayout->addWidget(inboundIntro);
    const QString inboundPath = QDir(m_projectDirectory).absoluteFilePath(QStringLiteral(".kerf/mcp.json"));
    auto *inboundPathLabel = new QLabel(QStringLiteral("Project config: %1\nAlso reads ~/.kerf/mcp.json; project entries override matching server names. Use command, optional args, env, and tools (an allowlist). Commands must be installed locally; env values are literal strings. This file may contain server credentials.").arg(inboundPath));
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
    m_inboundMcpStatus = new QLabel(QStringLiteral("No project config yet. Add servers here or load a Cursor config, then Save and apply. Existing home Kerf config is used automatically."));
    m_inboundMcpStatus->setObjectName(QStringLiteral("inboundMcpStatus"));
    m_inboundMcpStatus->setWordWrap(true);
    inboundLayout->addWidget(m_inboundMcpStatus);
    if (QFileInfo::exists(inboundPath)) loadInboundMcpConfig(inboundPath, false);
    connect(m_mcpEnabled, &QCheckBox::toggled, this, [this](bool enabled) {
        QSettings settings;
        settings.setValue(QStringLiteral("agent/mcpEnabled"), enabled);
        settings.sync();
        m_inboundMcpStatus->setText(QStringLiteral("MCP preference saved. Applies on the next chat request. KERF_ENABLE_MCP=1 also enables tools."));
    });
    advancedLayout->addWidget(inboundSection);

    auto *mcpSection = new QGroupBox(QStringLiteral("Use Kerf from Cursor / Claude"));
    mcpSection->setObjectName(QStringLiteral("mcpSection"));
    auto *mcpLayout = new QVBoxLayout(mcpSection);
    auto *mcpIntro = new QLabel(QStringLiteral("This is the opposite direction: Cursor, Claude, or Codex calls Kerf through kerf-mcp to work on this project. It does not enable external tools in Kerf chat. Kerf provider API keys are not required. Copy this config into your external client's MCP settings."));
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
        {QStringLiteral("kerf"), QJsonObject{
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
    advancedLayout->addWidget(mcpSection);
    settingsLayout->addStretch(1);
    settingsScroll->setWidget(settingsContent);
    settingsTabLayout->addWidget(settingsScroll);
    auto *closeSettings = new QDialogButtonBox(QDialogButtonBox::Close);
    connect(closeSettings, &QDialogButtonBox::rejected, m_preferences, &QDialog::hide);
    settingsTabLayout->addWidget(closeSettings);

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

    // Panels; the workspace arranges them once the window's actions exist.
    m_viewerDetail = new QAction(this);
    m_viewerDetail->setObjectName(QStringLiteral("viewerDetail"));
    m_viewerDetail->setEnabled(false);
    m_viewerDetail->setVisible(false);
    m_workspace->addPanel(QStringLiteral("program"), QStringLiteral("Program"), viewerFrame)
        ->setTitleBarActions(QList<QAction *>(m_maskTools) << m_viewerDetail);
    m_timelineDetail = new QAction(this);
    m_timelineDetail->setObjectName(QStringLiteral("timelineDetail"));
    m_timelineDetail->setEnabled(false);
    m_timelineDetail->setVisible(false);
    m_workspace->addPanel(QStringLiteral("timeline"), QStringLiteral("Timeline"), timelinePanel)
        ->setTitleBarActions({m_timelineDetail});
    m_workspace->addPanel(QStringLiteral("effectControls"), QStringLiteral("Effect Controls"), m_effectControls);
    m_workspace->addPanel(QStringLiteral("effects"), QStringLiteral("Effects"), m_effects);
    m_workspace->addPanel(QStringLiteral("project"), QStringLiteral("Project"), m_projectPanel);
    m_workspace->addPanel(QStringLiteral("presets"), QStringLiteral("Presets"), m_presets);
    m_workspace->addPanel(QStringLiteral("source"), QStringLiteral("Source"), m_sourceMonitor);
    m_workspace->addPanel(QStringLiteral("assistant"), QStringLiteral("Kerf Assistant"), right);
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
        menu.exec(at);
    });
    connect(m_timeline, &TimelineWidget::razorRequested, this, [this](const QString &id, double frame) {
        execute(QStringLiteral("clip.split"), QJsonObject{{QStringLiteral("clip"), id}, {QStringLiteral("at"), std::llround(frame)}});
    });
    connect(m_timeline, &TimelineWidget::trackToggleRequested, this, [this](const QString &track, const QString &field, bool value) {
        execute(QStringLiteral("track.set"), QJsonObject{{QStringLiteral("track"), track}, {field, value}});
    });
    updateProposal(QJsonValue());
    updateInspector();
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
    // Menus, shortcuts, and the toolbar come from the command registry.
    // Frontend commands and engine commands that need a dialog first have
    // handlers here; every other command resolves to a bridge request.
    m_commands = new CommandRegistry(QSettings().value(QStringLiteral("commands/keymap")).toString(),
                                     {{QStringLiteral("timeline"), m_timeline}, {QStringLiteral("source"), m_sourceMonitor}}, this);
    m_commandHandlers = {
        {QStringLiteral("file.newProject"), [this] { newProject(); }},
        {QStringLiteral("file.openProject"), [this] { openProjectFolder(); }},
        {QStringLiteral("preset.save"), [this] { savePreset(); }},
        {QStringLiteral("collab.sync"), [this] { if (execute(QStringLiteral("collab.sync"))) m_syncRequested = true; }},
        {QStringLiteral("collab.commit"), [this] { commitChanges(); }},
        {QStringLiteral("collab.newBranch"), [this] { newBranch(); }},
        {QStringLiteral("collab.switchBranch"), [this] { m_branchMenu->popup(QCursor::pos()); }},
        {QStringLiteral("collab.pullRequest"), [this] { openPullRequest(); }},
        {QStringLiteral("collab.publish"), [this] { publishProject(); }},
        {QStringLiteral("file.import"), [this] { importMedia(); }},
        {QStringLiteral("file.export"), [this] { exportProject(); }},
        {QStringLiteral("file.exportPreset"), [this] { exportEditingPreset(); }},
        {QStringLiteral("file.applyPreset"), [this] { importEditingPreset(); }},
        // Closing first keeps the workspace layout for the next launch.
        {QStringLiteral("app.quit"), [this] { if (close()) QApplication::quit(); }},
        {QStringLiteral("clip.focusRegion"), [this] { focusRegion(); }},
        {QStringLiteral("sequence.renderPreview"), [this] { renderPreview(); }},
        {QStringLiteral("playback.toggle"), [this] { togglePlayback(); }},
        {QStringLiteral("playback.goToStart"), [this] { seekToStart(); }},
        {QStringLiteral("playback.previousFrame"), [this] { stepBackward(); }},
        {QStringLiteral("playback.nextFrame"), [this] { stepForward(); }},
        {QStringLiteral("playback.goToEnd"), [this] { seekToEnd(); }},
        {QStringLiteral("view.fitTracks"), [this] { m_timeline->fitTracksToView(); }},
        {QStringLiteral("view.resetTrackHeights"), [this] { m_timeline->resetTrackHeights(); }},
        {QStringLiteral("app.preferences"), [this] {
            m_preferences->show();
            m_preferences->raise();
            m_preferences->activateWindow();
        }},
        {QStringLiteral("window.saveWorkspace"), [this] {
            m_workspace->saveCurrent();
            statusBar()->showMessage(QStringLiteral("Saved the %1 workspace").arg(m_workspace->current()), 4000);
        }},
        {QStringLiteral("window.saveWorkspaceAs"), [this] {
            const QString name = QInputDialog::getText(this, QStringLiteral("New Workspace"), QStringLiteral("Name"));
            if (name.trimmed().isEmpty()) return;
            if (!m_workspace->saveAs(name))
                statusBar()->showMessage(QStringLiteral("A workspace called %1 already exists").arg(name.trimmed()), 4000);
        }},
        {QStringLiteral("window.resetWorkspace"), [this] { m_workspace->resetCurrent(); }},
        {QStringLiteral("ai.ask"), [this] { askAi(); }},
        {QStringLiteral("source.markIn"), [this] { m_sourceMonitor->markIn(); }},
        {QStringLiteral("source.markOut"), [this] { m_sourceMonitor->markOut(); }},
        {QStringLiteral("source.clearMarks"), [this] { m_sourceMonitor->clearMarks(); }},
        {QStringLiteral("source.previousFrame"), [this] { m_sourceMonitor->step(-1); }},
        {QStringLiteral("source.nextFrame"), [this] { m_sourceMonitor->step(1); }},
    };
    connect(m_commands, &CommandRegistry::triggered, this, &MainWindow::triggerCommand);
    // Window ▸ panel commands show and hide docked panels, and follow
    // panels closed from their tabs.
    const QHash<QString, QString> panelCommands{
        {QStringLiteral("window.project"), QStringLiteral("project")},
        {QStringLiteral("window.source"), QStringLiteral("source")},
        {QStringLiteral("window.program"), QStringLiteral("program")},
        {QStringLiteral("window.timeline"), QStringLiteral("timeline")},
        {QStringLiteral("window.effectControls"), QStringLiteral("effectControls")},
        {QStringLiteral("window.effects"), QStringLiteral("effects")},
        {QStringLiteral("window.presets"), QStringLiteral("presets")},
        {QStringLiteral("window.assistant"), QStringLiteral("assistant")},
    };
    // Tools are one exclusive choice; snapping is a switch.
    const QHash<QString, TimelineTool> tools{
        {QStringLiteral("tool.selection"), TimelineTool::Selection}, {QStringLiteral("tool.razor"), TimelineTool::Razor},
        {QStringLiteral("tool.ripple"), TimelineTool::Ripple}, {QStringLiteral("tool.roll"), TimelineTool::Roll},
        {QStringLiteral("tool.slip"), TimelineTool::Slip}, {QStringLiteral("tool.slide"), TimelineTool::Slide},
    };
    connect(m_commands, &CommandRegistry::toggled, this, [this, panelCommands, tools](const QString &id, bool checked) {
        if (ads::CDockWidget *panel = m_workspace->panel(panelCommands.value(id)))
            if (panel->isClosed() == checked) panel->toggleView(checked);
        if (tools.contains(id) && checked) m_timeline->setTool(tools.value(id));
        if (id == QStringLiteral("sequence.snap")) m_timeline->setSnapping(checked);
        if (id == QStringLiteral("collab.live")) {
            // Remembered per project, and asked for again on each connection.
            rememberLiveSync(checked);
            execute(id, QJsonObject{{QStringLiteral("enabled"), checked}});
        }
    });
    {
        QAction *live = m_commands->action(QStringLiteral("collab.live"));
        const QSignalBlocker blocker(live);
        live->setChecked(liveSyncRemembered());
    }
    m_branchMenu = new QMenu(QStringLiteral("Switch Branch"), this);
    m_branchMenu->setObjectName(QStringLiteral("switchBranchMenu"));
    connect(m_branchMenu, &QMenu::aboutToShow, this, [this] { buildBranchMenu(m_branchMenu); });
    m_commands->action(QStringLiteral("collab.switchBranch"))->setMenu(m_branchMenu);
    m_presets->setActions(m_commands->action(QStringLiteral("preset.save")), m_commands->action(QStringLiteral("preset.library")));
    auto *toolGroup = new QActionGroup(this);
    toolGroup->setExclusive(true);
    for (const QString &id : {QStringLiteral("tool.selection"), QStringLiteral("tool.razor"), QStringLiteral("tool.ripple"),
                              QStringLiteral("tool.roll"), QStringLiteral("tool.slip"), QStringLiteral("tool.slide")}) {
        QAction *action = m_commands->action(id);
        toolGroup->addAction(action);
        // Tool letters, the way edit suites label their palettes.
        static const QHash<QString, QString> glyphs{
            {QStringLiteral("tool.selection"), QStringLiteral("▸")}, {QStringLiteral("tool.razor"), QStringLiteral("✂")},
            {QStringLiteral("tool.ripple"), QStringLiteral("⇤")}, {QStringLiteral("tool.roll"), QStringLiteral("⇹")},
            {QStringLiteral("tool.slip"), QStringLiteral("↔")}, {QStringLiteral("tool.slide"), QStringLiteral("⇆")}};
        action->setIconText(glyphs.value(id));
        m_toolPalette->addAction(action);
    }
    m_commands->action(QStringLiteral("tool.selection"))->setChecked(true);
    m_commands->action(QStringLiteral("sequence.snap"))->setChecked(m_timeline->snapping());
    for (const auto &[id, text] : {std::pair{QStringLiteral("source.markIn"), QStringLiteral("{")},
                                   std::pair{QStringLiteral("source.markOut"), QStringLiteral("}")},
                                   std::pair{QStringLiteral("source.insert"), QStringLiteral("Insert")},
                                   std::pair{QStringLiteral("source.overwrite"), QStringLiteral("Overwrite")}})
        m_commands->action(id)->setIconText(text);
    m_commands->action(QStringLiteral("proposal.accept"))->setIconText(QStringLiteral("Accept"));
    m_commands->action(QStringLiteral("proposal.reject"))->setIconText(QStringLiteral("Reject"));
    m_commands->action(QStringLiteral("ai.stop"))->setIconText(QStringLiteral("Stop"));
    for (const QString &id : {QStringLiteral("proposal.accept"), QStringLiteral("proposal.reject"), QStringLiteral("ai.stop")}) {
        auto *button = new QToolButton;
        button->setObjectName(id == QStringLiteral("proposal.accept") ? QStringLiteral("aiAccept") : QStringLiteral("aiButton"));
        button->setDefaultAction(m_commands->action(id));
        m_aiBar->layout()->addWidget(button);
    }
    auto *details = new QToolButton;
    details->setObjectName(QStringLiteral("aiButton"));
    details->setText(QStringLiteral("Details"));
    details->setToolTip(QStringLiteral("Show the conversation and the proposed script"));
    connect(details, &QToolButton::clicked, this, [this] {
        ads::CDockWidget *panel = m_workspace->panel(QStringLiteral("assistant"));
        panel->toggleView(true);
        panel->setAsCurrentTab();
    });
    m_aiBar->layout()->addWidget(details);
    m_sourceMonitor->setActions({m_commands->action(QStringLiteral("source.markIn")), m_commands->action(QStringLiteral("source.markOut")),
                                 m_commands->action(QStringLiteral("source.insert")), m_commands->action(QStringLiteral("source.overwrite"))});
    for (auto it = panelCommands.cbegin(); it != panelCommands.cend(); ++it) {
        QAction *action = m_commands->action(it.key());
        ads::CDockWidget *panel = m_workspace->panel(it.value());
        connect(panel, &ads::CDockWidget::viewToggled, action, [action](bool open) {
            const QSignalBlocker blocker(action);
            action->setChecked(open);
        });
    }
    connect(m_commands, &CommandRegistry::shortcutsChanged, this, &MainWindow::updateTransport);

    menuBar()->setNativeMenuBar(true);
    const QHash<QString, QMenu *> menus = m_commands->buildMenus(menuBar());
    if (QMenu *file = menus.value(QStringLiteral("File"))) {
        m_recentMenu = new QMenu(QStringLiteral("Open Recent"), file);
        m_recentMenu->setObjectName(QStringLiteral("openRecentMenu"));
        const QList<QAction *> actions = file->actions();
        const qsizetype open = actions.indexOf(m_commands->action(QStringLiteral("file.openProject")));
        file->insertMenu(actions.value(open + 1), m_recentMenu);
        connect(m_recentMenu, &QMenu::aboutToShow, this, [this] { buildRecentMenu(m_recentMenu); });
        buildRecentMenu(m_recentMenu);
    }
    if (QMenu *edit = menus.value(QStringLiteral("Edit"))) {
        edit->addSeparator();
        auto *keymapMenu = edit->addMenu(QStringLiteral("Keyboard Shortcuts"));
        auto *keymapGroup = new QActionGroup(keymapMenu);
        for (const QJsonValue &value : m_commands->keymaps()) {
            const QJsonObject keymap = value.toObject();
            const QString id = keymap.value(QStringLiteral("id")).toString();
            auto *choice = keymapMenu->addAction(keymap.value(QStringLiteral("label")).toString());
            choice->setObjectName(QStringLiteral("keymap.%1").arg(id));
            choice->setCheckable(true);
            choice->setChecked(id == m_commands->keymap());
            keymapGroup->addAction(choice);
            connect(choice, &QAction::triggered, this, [this, id] {
                m_commands->setKeymap(id);
                QSettings().setValue(QStringLiteral("commands/keymap"), id);
            });
        }
    }
    if (QMenu *window = menus.value(QStringLiteral("Window"))) {
        auto *workspaces = new QMenu(QStringLiteral("Workspaces"), window);
        workspaces->setObjectName(QStringLiteral("workspacesMenu"));
        window->insertMenu(window->actions().value(0), workspaces);
        window->insertSeparator(window->actions().value(1));
        connect(workspaces, &QMenu::aboutToShow, this, [this, workspaces] { buildWorkspaceMenu(workspaces); });
        buildWorkspaceMenu(workspaces);
    }
    m_workspace->start();
    for (auto it = panelCommands.cbegin(); it != panelCommands.cend(); ++it) {
        const QSignalBlocker blocker(m_commands->action(it.key()));
        m_commands->action(it.key())->setChecked(!m_workspace->panel(it.value())->isClosed());
    }

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
        for (const QString &name : m_commands->menuNames()) {
            QMenu *menu = menus.value(name);
            if (!menu) continue;
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
    const auto toolbarCommand = [&](const QString &id, const QString &text, const QString &glyph = QString()) {
        QAction *action = m_commands->action(id);
        action->setIconText(text);
        if (!glyph.isEmpty()) {
            action->setIcon(toolbarIcon(glyph));
            m_toolbarGlyphs.append({action, glyph});
        }
        toolbar->addAction(action);
        return qobject_cast<QToolButton *>(toolbar->widgetForAction(action));
    };
    toolbarCommand(QStringLiteral("file.import"), QStringLiteral("Import"), QStringLiteral("↓"));
    toolbar->addSeparator();
    toolbarCommand(QStringLiteral("edit.undo"), QStringLiteral("Undo"), QStringLiteral("↶"));
    toolbarCommand(QStringLiteral("edit.redo"), QStringLiteral("Redo"), QStringLiteral("↷"));
    toolbar->addSeparator();
    toolbarCommand(QStringLiteral("clip.split"), QStringLiteral("Split"), QStringLiteral("✂"));
    toolbarCommand(QStringLiteral("clip.delete"), QStringLiteral("Delete"), QStringLiteral("×"));
    toolbar->addSeparator();
    auto *spacer = new QWidget;
    spacer->setStyleSheet(QStringLiteral("background: transparent"));
    spacer->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Preferred);
    toolbar->addWidget(spacer);
    auto *projectTitle = new QLabel(QStringLiteral("KERF  /  %1").arg(QFileInfo(m_projectDirectory).fileName()));
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
    if (auto *exportButton = toolbarCommand(QStringLiteral("file.export"), QStringLiteral("Export")))
        exportButton->setObjectName(QStringLiteral("toolbarPrimary"));
    if (auto *renderButton = toolbarCommand(QStringLiteral("sequence.renderPreview"), QStringLiteral("Render")))
        renderButton->setObjectName(QStringLiteral("toolbarQuiet"));
    updateCommandStates();
}

void MainWindow::buildWorkspaceMenu(QMenu *menu) {
    menu->clear();
    qDeleteAll(menu->findChildren<QActionGroup *>(Qt::FindDirectChildrenOnly));
    auto *group = new QActionGroup(menu);
    for (const QString &name : m_workspace->names()) {
        QAction *choice = menu->addAction(name);
        choice->setObjectName(QStringLiteral("workspace.%1").arg(name));
        choice->setCheckable(true);
        choice->setChecked(name == m_workspace->current());
        group->addAction(choice);
        connect(choice, &QAction::triggered, this, [this, name] { m_workspace->activate(name); });
    }
}

void MainWindow::closeEvent(QCloseEvent *event) {
    m_workspace->persist();
    QMainWindow::closeEvent(event);
}

void MainWindow::newProject() {
    ProjectLauncher launcher(m_kerfdBinary, this);
    if (launcher.exec() == QDialog::Accepted) openProject(launcher.chosenPath());
}

void MainWindow::openProjectFolder() {
    const QString start = m_projectDirectory.isEmpty() ? QDir::homePath() : QFileInfo(m_projectDirectory).absolutePath();
    const QString path = QFileDialog::getExistingDirectory(this, QStringLiteral("Open Project"), start);
    if (path.isEmpty()) return;
    if (!QFileInfo::exists(QDir(path).filePath(QStringLiteral("project.kerf"))) &&
        QMessageBox::question(this, QStringLiteral("Open Project"),
                              QStringLiteral("“%1” has no project.kerf. Start a new project in it?").arg(QFileInfo(path).fileName())) !=
            QMessageBox::Yes)
        return;
    openProject(path);
}

void MainWindow::openProject(const QString &path) {
    if (path.isEmpty()) return;
    const QString clean = QDir::cleanPath(QFileInfo(path).absoluteFilePath());
    if (!m_projectDirectory.isEmpty() && clean == QDir::cleanPath(QFileInfo(m_projectDirectory).absoluteFilePath())) {
        raise();
        activateWindow();
        return;
    }
    emit projectOpenRequested(clean);
}

void MainWindow::buildRecentMenu(QMenu *menu) {
    menu->clear();
    const QString current = m_projectDirectory.isEmpty() ? QString() : QDir::cleanPath(QFileInfo(m_projectDirectory).absoluteFilePath());
    const QStringList recent = ProjectLauncher::recentProjects();
    for (const QString &path : recent) {
        QAction *open = menu->addAction(QFileInfo(path).fileName());
        open->setToolTip(path);
        open->setCheckable(path == current);
        open->setChecked(path == current);
        open->setEnabled(QFileInfo(path).isDir());
        connect(open, &QAction::triggered, this, [this, path] { openProject(path); });
    }
    if (recent.isEmpty()) menu->addAction(QStringLiteral("No Recent Projects"))->setEnabled(false);
    menu->addSeparator();
    QAction *clear = menu->addAction(QStringLiteral("Clear Menu"));
    clear->setEnabled(!recent.isEmpty());
    connect(clear, &QAction::triggered, this, [] { ProjectLauncher::clearRecentProjects(); });
}

void MainWindow::startSession() {
    execute(QStringLiteral("collab.status"));
    execute(QStringLiteral("preset.library"));
    if (liveSyncRemembered()) execute(QStringLiteral("collab.live"), QJsonObject{{QStringLiteral("enabled"), true}});
}

bool MainWindow::liveSyncRemembered() const {
    return !m_projectDirectory.isEmpty() && QSettings()
                                                .value(QStringLiteral("collab/liveProjects"))
                                                .toStringList()
                                                .contains(QDir::cleanPath(QFileInfo(m_projectDirectory).absoluteFilePath()));
}

void MainWindow::rememberLiveSync(bool enabled) {
    if (m_projectDirectory.isEmpty()) return;
    QSettings settings;
    QStringList projects = settings.value(QStringLiteral("collab/liveProjects")).toStringList();
    const QString path = QDir::cleanPath(QFileInfo(m_projectDirectory).absoluteFilePath());
    projects.removeAll(path);
    if (enabled) projects.append(path);
    settings.setValue(QStringLiteral("collab/liveProjects"), projects);
}

QJsonObject MainWindow::collabContext() const {
    // Typed field by field: the registry reads a context it cannot parse
    // as an offline one.
    const auto text = [this](const char *key) {
        const QJsonValue value = m_collabStatus.value(QLatin1String(key));
        return value.isString() && !value.toString().isEmpty() ? value : QJsonValue();
    };
    QJsonArray branches;
    for (const QJsonValue &branch : m_collabStatus.value(QStringLiteral("branches")).toArray())
        if (branch.isString()) branches.append(branch);
    const QJsonValue gh = m_collabStatus.value(QStringLiteral("gh"));
    return QJsonObject{
        {QStringLiteral("repository"), m_collabStatus.value(QStringLiteral("repository")).toBool()},
        {QStringLiteral("branch"), text("branch")},
        {QStringLiteral("upstream"), text("upstream")},
        {QStringLiteral("remote"), text("remote")},
        {QStringLiteral("ahead"), std::max(0, m_collabStatus.value(QStringLiteral("ahead")).toInt())},
        {QStringLiteral("behind"), std::max(0, m_collabStatus.value(QStringLiteral("behind")).toInt())},
        {QStringLiteral("changed"), m_collabStatus.value(QStringLiteral("changed")).toBool()},
        {QStringLiteral("conflicted"), m_collabStatus.value(QStringLiteral("conflicted")).toBool()},
        {QStringLiteral("branches"), branches},
        // A signed-in account may be reported by name.
        {QStringLiteral("gh"), gh.isBool() ? gh.toBool() : gh.isString() ? !gh.toString().isEmpty() : gh.isObject()},
        {QStringLiteral("live"), m_collabStatus.value(QStringLiteral("live")).toBool()},
        {QStringLiteral("busy"), m_collabStatus.value(QStringLiteral("busy")).toBool() || !m_syncStage.isEmpty()},
        {QStringLiteral("externalMedia"), m_mediaStatus.value(QStringLiteral("external")).toArray().size()},
    };
}

void MainWindow::handleCollabEvent(const QJsonObject &event) {
    const QString type = event.value(QStringLiteral("type")).toString();
    if (type == QStringLiteral("collab")) {
        m_collabStatus = event.value(QStringLiteral("status")).toObject();
        QAction *live = m_commands->action(QStringLiteral("collab.live"));
        const QSignalBlocker blocker(live);
        live->setChecked(m_collabStatus.value(QStringLiteral("live")).toBool());
    } else if (type == QStringLiteral("collab_sync")) {
        const QString stage = event.value(QStringLiteral("stage")).toString();
        if (stage == QStringLiteral("done")) {
            m_syncStage.clear();
            m_syncFailure.clear();
            const QJsonObject report = event.value(QStringLiteral("report")).toObject();
            const QJsonArray incoming = report.value(QStringLiteral("incoming")).toArray();
            const QJsonArray conflicts = report.value(QStringLiteral("conflicts")).toArray();
            QStringList lines;
            for (const QJsonValue &value : incoming) {
                const QJsonObject commit = value.toObject();
                lines << QStringLiteral("%1 · %2").arg(commit.value(QStringLiteral("author")).toString(),
                                                        commit.value(QStringLiteral("subject")).toString());
            }
            int keptTheirs = 0;
            for (const QJsonValue &value : conflicts) {
                const QJsonObject conflict = value.toObject();
                const bool theirs = conflict.value(QStringLiteral("kept")).toString() == QStringLiteral("theirs");
                if (theirs) ++keptTheirs;
                const QString path = conflict.value(QStringLiteral("path")).toString();
                const QString what = conflict.value(QStringLiteral("what")).toString();
                lines << QStringLiteral("%1 · %2").arg(theirs ? QStringLiteral("Kept theirs") : QStringLiteral("Kept yours"),
                                                        path.isEmpty() ? what : QStringLiteral("%1: %2").arg(path, what));
            }
            const int keptYours = conflicts.size() - keptTheirs;
            for (const QJsonValue &value : report.value(QStringLiteral("renamed")).toArray()) {
                const QJsonArray pair = value.toArray();
                lines << QStringLiteral("Renumbered · %1 → %2").arg(pair.at(0).toString(), pair.at(1).toString());
            }
            for (const QString &line : std::as_const(lines)) addActivity(QStringLiteral("sync"), line);
            // A quiet live-sync cycle keeps the last sync's news in the tooltip.
            if (!lines.isEmpty()) m_syncLines = lines;
            QStringList summary;
            if (!incoming.isEmpty())
                summary << (incoming.size() == 1 ? QStringLiteral("1 commit in") : QStringLiteral("%1 commits in").arg(incoming.size()));
            if (report.value(QStringLiteral("pushed")).toBool()) summary << QStringLiteral("pushed");
            if (keptYours > 0)
                summary << (keptYours == 1 ? QStringLiteral("1 change kept yours") : QStringLiteral("%1 changes kept yours").arg(keptYours));
            if (keptTheirs > 0)
                summary << (keptTheirs == 1 ? QStringLiteral("1 change kept theirs") : QStringLiteral("%1 changes kept theirs").arg(keptTheirs));
            if (!summary.isEmpty() || m_syncRequested)
                statusBar()->showMessage(summary.isEmpty() ? QStringLiteral("Synced: up to date")
                                                           : QStringLiteral("Synced: ") + summary.join(QStringLiteral(", ")),
                                         8000);
            // A pull or branch switch changes presets/, and a commit who
            // last changed each preset.
            execute(QStringLiteral("preset.library"));
            m_syncRequested = false;
        } else if (stage == QStringLiteral("failed")) {
            m_syncRequested = false;
            m_syncStage.clear();
            m_syncFailure = event.value(QStringLiteral("message")).toString(QStringLiteral("The sync did not finish."));
            showError(QStringLiteral("Sync failed: %1").arg(m_syncFailure));
            addActivity(QStringLiteral("sync"), m_syncFailure);
        } else {
            m_syncStage = stage.isEmpty() ? QStringLiteral("syncing") : stage;
            m_syncFailure.clear();
        }
    } else if (type == QStringLiteral("collab_pull_request")) {
        const QString url = event.value(QStringLiteral("url")).toString();
        const bool created = event.value(QStringLiteral("created")).toBool();
        // Without gh, the bridge gives GitHub's page for starting one.
        const bool compare = QUrl(url).path().section(QLatin1Char('/'), 3, 3) == QStringLiteral("compare");
        statusBar()->showMessage(created ? QStringLiteral("Opened a pull request: %1").arg(url)
                                 : compare ? QStringLiteral("Opened GitHub's page for a new pull request: %1").arg(url)
                                           : QStringLiteral("This branch already has a pull request: %1").arg(url),
                                 8000);
        addActivity(QStringLiteral("sync"), url);
#ifndef KERF_QT_TESTING
        if (!url.isEmpty()) QDesktopServices::openUrl(QUrl(url));
#endif
    }
    updateSyncChip();
    updateCommandStates();
}

void MainWindow::updateSyncChip() {
    if (!m_syncChip) return;
    if (m_collabStatus.isEmpty()) {
        m_syncChip->hide();
        return;
    }
    const auto field = [this](const char *key) { return m_collabStatus.value(QLatin1String(key)); };
    QString text;
    QString state;
    QStringList tips;
    if (!field("repository").toBool()) {
        text = QStringLiteral("⎇ Not shared");
        state = QStringLiteral("local");
        tips << QStringLiteral("This project is not a Git repository yet.")
             << QStringLiteral("Click, or choose Collaborate → Publish to GitHub…, to share it.");
    } else {
        const QString branch = field("branch").toString();
        const int ahead = field("ahead").toInt();
        const int behind = field("behind").toInt();
        text = QStringLiteral("⎇ %1").arg(branch.isEmpty() ? QStringLiteral("detached") : branch);
        if (ahead > 0) text += QStringLiteral(" ↑%1").arg(ahead);
        if (behind > 0) text += QStringLiteral(" ↓%1").arg(behind);
        const QString remote = field("remote").toString();
        const QString upstream = field("upstream").toString();
        tips << (remote.isEmpty() ? QStringLiteral("No remote yet: Publish to GitHub… shares the project.")
                 : upstream.isEmpty() ? QStringLiteral("%1, not pushed yet").arg(remote)
                                      : QStringLiteral("Syncs with %1").arg(upstream));
        if (ahead > 0) tips << (ahead == 1 ? QStringLiteral("1 commit to push") : QStringLiteral("%1 commits to push").arg(ahead));
        if (behind > 0) tips << (behind == 1 ? QStringLiteral("1 commit to pull") : QStringLiteral("%1 commits to pull").arg(behind));
        if (field("changed").toBool()) tips << QStringLiteral("Edits not committed yet");
        static const QHash<QString, QString> stages{
            {QStringLiteral("committing"), QStringLiteral("Committing")}, {QStringLiteral("fetching"), QStringLiteral("Pulling")},
            {QStringLiteral("merging"), QStringLiteral("Merging")}, {QStringLiteral("pushing"), QStringLiteral("Pushing")},
            {QStringLiteral("downloading"), QStringLiteral("Downloading media")}};
        if (!m_syncStage.isEmpty() || field("busy").toBool()) {
            text = QStringLiteral("⟳ %1…").arg(stages.value(m_syncStage, QStringLiteral("Syncing")));
            state = QStringLiteral("syncing");
        } else if (field("conflicted").toBool()) {
            text += QStringLiteral(" · merge unfinished");
            state = QStringLiteral("conflict");
            tips.prepend(QStringLiteral("A merge is in progress with unresolved files; finish it with git."));
        } else if (!m_syncFailure.isEmpty()) {
            text += QStringLiteral(" · sync failed");
            state = QStringLiteral("failed");
            tips.prepend(QStringLiteral("Sync failed: %1").arg(m_syncFailure));
        } else if (field("live").toBool()) {
            text += QStringLiteral(" · live");
            state = QStringLiteral("live");
            tips << QStringLiteral("Live sync is on");
        } else {
            state = QStringLiteral("idle");
        }
        if (!m_syncLines.isEmpty()) tips << QString() << QStringLiteral("Last sync:") << m_syncLines;
        tips << QString() << QStringLiteral("Click to sync now");
    }
    m_syncChip->setText(text);
    m_syncChip->setToolTip(tips.join(QLatin1Char('\n')));
    m_syncChip->setAccessibleName(text);
    m_syncChip->setProperty("state", state);
    m_syncChip->style()->unpolish(m_syncChip);
    m_syncChip->style()->polish(m_syncChip);
    m_syncChip->show();
}

void MainWindow::buildBranchMenu(QMenu *menu) {
    menu->clear();
    const QString current = m_collabStatus.value(QStringLiteral("branch")).toString();
    for (const QJsonValue &value : m_collabStatus.value(QStringLiteral("branches")).toArray()) {
        const QString branch = value.toString();
        if (branch.isEmpty() || branch == current) continue;
        QAction *choice = menu->addAction(branch);
        choice->setObjectName(QStringLiteral("branch.%1").arg(branch));
        connect(choice, &QAction::triggered, this, [this, branch] {
            execute(QStringLiteral("collab.switchBranch"), QJsonObject{{QStringLiteral("name"), branch}});
        });
    }
    if (menu->isEmpty()) menu->addAction(QStringLiteral("No Other Branches"))->setEnabled(false);
}

void MainWindow::commitChanges() {
    bool ok = false;
    const QString message = QInputDialog::getText(this, QStringLiteral("Commit"), QStringLiteral("Describe the change"),
                                                  QLineEdit::Normal, QString(), &ok);
    if (ok && !message.trimmed().isEmpty())
        execute(QStringLiteral("collab.commit"), QJsonObject{{QStringLiteral("message"), message}});
}

void MainWindow::newBranch() {
    bool ok = false;
    const QString name = QInputDialog::getText(this, QStringLiteral("New Branch"),
                                               QStringLiteral("Commits your edits, then starts a branch from them.\nName"),
                                               QLineEdit::Normal, QString(), &ok);
    if (ok && !name.trimmed().isEmpty())
        execute(QStringLiteral("collab.newBranch"), QJsonObject{{QStringLiteral("name"), name}});
}

void MainWindow::openPullRequest() {
    QDialog dialog(this);
    dialog.setObjectName(QStringLiteral("pullRequestDialog"));
    dialog.setWindowTitle(QStringLiteral("Open Pull Request"));
    dialog.resize(480, 320);
    auto *layout = new QVBoxLayout(&dialog);
    const QString branch = m_collabStatus.value(QStringLiteral("branch")).toString();
    auto *intro = new QLabel(QStringLiteral("Pushes %1 and opens a pull request on GitHub. Reviewers read the change as Kerf code.")
                                 .arg(branch.isEmpty() ? QStringLiteral("this branch") : branch));
    intro->setWordWrap(true);
    layout->addWidget(intro);
    auto *form = new QFormLayout;
    QString suggested = branch;
    suggested.replace(QLatin1Char('-'), QLatin1Char(' ')).replace(QLatin1Char('_'), QLatin1Char(' '));
    if (!suggested.isEmpty()) suggested[0] = suggested[0].toUpper();
    auto *title = new QLineEdit(suggested);
    title->setObjectName(QStringLiteral("pullRequestTitle"));
    form->addRow(QStringLiteral("Title"), title);
    auto *body = new QPlainTextEdit;
    body->setObjectName(QStringLiteral("pullRequestBody"));
    body->setPlaceholderText(QStringLiteral("Leave empty to list this branch's commits"));
    form->addRow(QStringLiteral("Description"), body);
    layout->addLayout(form, 1);
    auto *buttons = new QDialogButtonBox(QDialogButtonBox::Ok | QDialogButtonBox::Cancel);
    buttons->button(QDialogButtonBox::Ok)->setText(QStringLiteral("Open Pull Request"));
    connect(title, &QLineEdit::textChanged, buttons, [buttons](const QString &text) {
        buttons->button(QDialogButtonBox::Ok)->setEnabled(!text.trimmed().isEmpty());
    });
    buttons->button(QDialogButtonBox::Ok)->setEnabled(!suggested.trimmed().isEmpty());
    connect(buttons, &QDialogButtonBox::accepted, &dialog, &QDialog::accept);
    connect(buttons, &QDialogButtonBox::rejected, &dialog, &QDialog::reject);
    layout->addWidget(buttons);
    if (dialog.exec() != QDialog::Accepted) return;
    QJsonObject params{{QStringLiteral("title"), title->text()}};
    if (!body->toPlainText().trimmed().isEmpty()) params.insert(QStringLiteral("body"), body->toPlainText());
    execute(QStringLiteral("collab.pullRequest"), params);
}

void MainWindow::publishProject() {
    QDialog dialog(this);
    dialog.setObjectName(QStringLiteral("publishDialog"));
    dialog.setWindowTitle(QStringLiteral("Publish to GitHub"));
    dialog.resize(500, 300);
    auto *layout = new QVBoxLayout(&dialog);
    auto *intro = new QLabel(QStringLiteral("Makes the project a Git repository if it is not one yet, then pushes it so "
                                            "collaborators can clone it, sync, and open pull requests."));
    intro->setWordWrap(true);
    layout->addWidget(intro);
    const QJsonObject collab = collabContext();
    auto *github = new QRadioButton(QStringLiteral("Create a GitHub repository"));
    github->setObjectName(QStringLiteral("publishGitHub"));
    github->setEnabled(collab.value(QStringLiteral("gh")).toBool());
    if (!github->isEnabled()) github->setToolTip(QStringLiteral("Install the GitHub CLI and sign in with `gh auth login`"));
    auto *name = new QLineEdit(QFileInfo(m_projectDirectory).fileName().replace(QRegularExpression(QStringLiteral("[^A-Za-z0-9._-]+")), QStringLiteral("-")));
    name->setObjectName(QStringLiteral("publishName"));
    auto *priv = new QCheckBox(QStringLiteral("Private"));
    priv->setObjectName(QStringLiteral("publishPrivate"));
    priv->setChecked(true);
    auto *githubRow = new QHBoxLayout;
    githubRow->setContentsMargins(22, 0, 0, 0);
    githubRow->addWidget(name, 1);
    githubRow->addWidget(priv);
    auto *remoteChoice = new QRadioButton(QStringLiteral("Push to a remote I already have"));
    remoteChoice->setObjectName(QStringLiteral("publishRemoteChoice"));
    auto *remote = new QLineEdit;
    remote->setObjectName(QStringLiteral("publishRemote"));
    remote->setPlaceholderText(QStringLiteral("git@github.com:you/film.git"));
    auto *remoteRow = new QHBoxLayout;
    remoteRow->setContentsMargins(22, 0, 0, 0);
    remoteRow->addWidget(remote);
    auto *local = new QRadioButton(QStringLiteral("Only make it a repository on this computer"));
    local->setObjectName(QStringLiteral("publishLocal"));
    local->setEnabled(!collab.value(QStringLiteral("repository")).toBool());
    (github->isEnabled() ? github : remoteChoice)->setChecked(true);
    layout->addWidget(github);
    layout->addLayout(githubRow);
    layout->addWidget(remoteChoice);
    layout->addLayout(remoteRow);
    layout->addWidget(local);
    if (!m_collabStatus.value(QStringLiteral("enclosing")).toString().isEmpty()) {
        auto *nested = new QLabel(QStringLiteral("This folder is inside another repository (%1); the project gets a repository of its own.")
                                      .arg(m_collabStatus.value(QStringLiteral("enclosing")).toString()));
        nested->setObjectName(QStringLiteral("muted"));
        nested->setWordWrap(true);
        layout->addWidget(nested);
    }
    layout->addStretch(1);
    auto *buttons = new QDialogButtonBox(QDialogButtonBox::Ok | QDialogButtonBox::Cancel);
    buttons->button(QDialogButtonBox::Ok)->setText(QStringLiteral("Publish"));
    const auto update = [=] {
        name->setEnabled(github->isChecked());
        priv->setEnabled(github->isChecked());
        remote->setEnabled(remoteChoice->isChecked());
        buttons->button(QDialogButtonBox::Ok)->setEnabled(!remoteChoice->isChecked() || !remote->text().trimmed().isEmpty());
    };
    for (QRadioButton *choice : {github, remoteChoice, local}) connect(choice, &QRadioButton::toggled, &dialog, update);
    connect(remote, &QLineEdit::textChanged, &dialog, update);
    update();
    connect(buttons, &QDialogButtonBox::accepted, &dialog, &QDialog::accept);
    connect(buttons, &QDialogButtonBox::rejected, &dialog, &QDialog::reject);
    layout->addWidget(buttons);
    if (dialog.exec() != QDialog::Accepted) return;
    QJsonObject params;
    if (github->isChecked()) {
        QJsonObject repository{{QStringLiteral("private"), priv->isChecked()}};
        if (!name->text().trimmed().isEmpty()) repository.insert(QStringLiteral("name"), name->text().trimmed());
        params.insert(QStringLiteral("github"), repository);
    } else if (remoteChoice->isChecked()) {
        params.insert(QStringLiteral("remote"), remote->text().trimmed());
    }
    if (params.isEmpty()) execute(QStringLiteral("collab.publish"));
    else execute(QStringLiteral("collab.publish"), params);
}

void MainWindow::savePreset() {
    bool ok = false;
    const QString name = QInputDialog::getText(this, QStringLiteral("Save Current as Preset"),
                                               QStringLiteral("Saves the program sequence in presets/, shared with the project.\nName"),
                                               QLineEdit::Normal, QString(), &ok)
                             .trimmed();
    if (!ok || name.isEmpty()) return;
    // The bridge names the file, cleaning the name up, and refuses one that
    // exists; replacing it is asked then.
    m_savingPreset = name;
    execute(QStringLiteral("preset.save"), QJsonObject{{QStringLiteral("name"), name}, {QStringLiteral("replace"), false}});
}

void MainWindow::replaceSavedPreset(const QString &path) {
    const QString name = std::exchange(m_savingPreset, QString());
    if (QMessageBox::question(this, QStringLiteral("Save Current as Preset"),
                              QStringLiteral("%1 already exists. Replace it?").arg(path)) == QMessageBox::Yes)
        execute(QStringLiteral("preset.save"), QJsonObject{{QStringLiteral("name"), name}, {QStringLiteral("replace"), true}});
}

void MainWindow::applyLibraryPreset(const QString &path) {
    // The preset's slots and the media able to fill them come back as
    // preset_inspect, which opens the bindings dialog.
    sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("preset_inspect")}, {QStringLiteral("path"), path}});
}

QString MainWindow::mediaPath(const QString &mediaId) const {
    for (const QJsonValue &value : m_project.value(QStringLiteral("sources")).toArray()) {
        const QJsonObject source = value.toObject();
        if (source.value(QStringLiteral("id")).toString() == mediaId)
            return QDir(m_projectDirectory).absoluteFilePath(source.value(QStringLiteral("path")).toString());
    }
    return {};
}

void MainWindow::replaceMedia(const QString &mediaId, const QString &path) {
    QString file = path;
    if (file.isEmpty()) {
        file = QFileDialog::getOpenFileName(this, QStringLiteral("Replace %1").arg(mediaId), QFileInfo(mediaPath(mediaId)).absolutePath(),
                                            QStringLiteral("Media files (*.mov *.mp4 *.m4v *.mkv *.webm *.wav *.mp3);;All files (*)"));
        if (file.isEmpty()) return;
    }
    if (execute(QStringLiteral("media.replace"), QJsonObject{{QStringLiteral("media"), mediaId}, {QStringLiteral("path"), file}}))
        addActivity(QStringLiteral("import"), QStringLiteral("Replacing %1 with %2").arg(mediaId, QFileInfo(file).fileName()));
}

void MainWindow::revealMedia(const QString &mediaId) {
    const QString path = mediaPath(mediaId);
    if (path.isEmpty()) return;
#if defined(Q_OS_MACOS)
    QProcess::startDetached(QStringLiteral("open"), {QStringLiteral("-R"), path});
#else
    QDesktopServices::openUrl(QUrl::fromLocalFile(QFileInfo(path).absolutePath()));
#endif
}

QMenu *MainWindow::binMenu(const QString &mediaId) {
    auto *menu = new QMenu(this);
    menu->setObjectName(QStringLiteral("binMenu"));
    menu->setToolTipsVisible(true);
    if (!mediaId.isEmpty()) {
        QAction *open = menu->addAction(QStringLiteral("Open in Source Monitor"));
        open->setObjectName(QStringLiteral("binOpen"));
        connect(open, &QAction::triggered, this, [this, mediaId] { openSource(mediaId); });
        menu->addSeparator();
        QAction *replace = menu->addAction(QStringLiteral("Replace Media…"));
        replace->setObjectName(QStringLiteral("binReplace"));
        replace->setEnabled(m_commands->isEnabled(QStringLiteral("media.replace")));
        replace->setToolTip(replace->isEnabled() ? QStringLiteral("Swap the file behind %1 for everyone; its clips keep their timing").arg(mediaId)
                                                 : m_commands->reason(QStringLiteral("media.replace")));
        connect(replace, &QAction::triggered, this, [this, mediaId] { replaceMedia(mediaId); });
        QAction *reveal = menu->addAction(QStringLiteral("Reveal in Finder"));
        reveal->setObjectName(QStringLiteral("binReveal"));
        reveal->setEnabled(QFileInfo::exists(mediaPath(mediaId)));
        connect(reveal, &QAction::triggered, this, [this, mediaId] { revealMedia(mediaId); });
        menu->addSeparator();
    }
    menu->addAction(m_commands->action(QStringLiteral("file.import")));
    menu->addAction(m_commands->action(QStringLiteral("collab.consolidate")));
    return menu;
}

void MainWindow::triggerCommand(const QString &id) {
    if (!m_commands->isEnabled(id)) return;
    const auto handler = m_commandHandlers.constFind(id);
    if (handler != m_commandHandlers.cend()) (*handler)();
    else execute(id);
}

bool MainWindow::execute(const QString &id, const QJsonObject &params) {
    const QJsonObject result = CommandRegistry::run(id, params, commandContext());
    if (result.contains(QStringLiteral("error"))) {
        statusBar()->showMessage(result.value(QStringLiteral("error")).toString(), 4000);
        return false;
    }
    return sendSimple(result.value(QStringLiteral("request")).toObject());
}

QJsonObject MainWindow::commandContext() const {
    int videoTracks = 0;
    int audioTracks = 0;
    for (const QJsonValue &value : m_project.value(QStringLiteral("tracks")).toArray()) {
        const QString type = value.toObject().value(QStringLiteral("type")).toString();
        if (type == QStringLiteral("video")) ++videoTracks;
        else if (type == QStringLiteral("audio")) ++audioTracks;
    }
    QJsonObject context{{QStringLiteral("connected"), m_connected},
                        {QStringLiteral("revision"), revisionValue()},
                        {QStringLiteral("canUndo"), m_snapshot.value(QStringLiteral("canUndo")).toBool()},
                        {QStringLiteral("canRedo"), m_snapshot.value(QStringLiteral("canRedo")).toBool()},
                        {QStringLiteral("playheadFrame"), frameAt(m_playhead)},
                        {QStringLiteral("previewRequired"), m_previewRequired},
                        {QStringLiteral("previewRenderable"), m_previewStatus == QStringLiteral("required") ||
                                                                  m_previewStatus == QStringLiteral("live")},
                        {QStringLiteral("videoTracks"), videoTracks},
                        {QStringLiteral("audioTracks"), audioTracks},
                        {QStringLiteral("sourceTargets"), QJsonArray::fromStringList(sourceTargets())},
                        {QStringLiteral("agentBusy"), m_agentBusy},
                        {QStringLiteral("aiAccess"), m_cloudRadio ? aiAccess() : QStringLiteral("ready")}};
    const QJsonValue projection = m_snapshot.value(QStringLiteral("projectionStatus"));
    if (projection.isString()) context.insert(QStringLiteral("projectionStatus"), projection);
    if (!m_collabStatus.isEmpty()) context.insert(QStringLiteral("collab"), collabContext());
    if (!m_proposalId.isEmpty()) {
        QJsonObject proposal{{QStringLiteral("id"), m_proposalId}};
        if (m_proposalBlocked) proposal.insert(QStringLiteral("blocked"), QStringLiteral("stale"));
        context.insert(QStringLiteral("proposal"), proposal);
    }
    if (m_sourceMonitor->isOpen()) {
        // The marked range: in frames of the media, length in frames of the sequence.
        const double in = m_sourceMonitor->inPoint();
        context.insert(QStringLiteral("source"),
                       QJsonObject{{QStringLiteral("media"), m_sourceMonitor->mediaId()},
                                   {QStringLiteral("inFrame"), std::llround(in * m_sourceMonitor->fps())},
                                   {QStringLiteral("durationFrames"), frameAt(m_sourceMonitor->outPoint() - in)}});
    }
    const QJsonObject clip = selectedClipObject();
    if (!clip.isEmpty()) {
        context.insert(QStringLiteral("selection"),
                       QJsonObject{{QStringLiteral("clip"), m_selectedClip},
                                   {QStringLiteral("track"), selectedTrackType()},
                                   {QStringLiteral("startFrame"), clipFrames(clip, QStringLiteral("startFrame"))},
                                   {QStringLiteral("durationFrames"), clipFrames(clip, QStringLiteral("durationFrames"))}});
    }
    return context;
}

void MainWindow::updateCommandStates() {
    if (!m_commands) return;
    m_commands->update(commandContext());
    for (auto *button : m_transportButtons)
        button->setEnabled(m_commands->isEnabled(button->property("command").toString()));
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
    // Each button runs a playback command; updateTransport labels it with
    // the command's shortcut.
    const auto button = [&](TransportGlyph glyph, const QString &command) {
        auto *control = new QToolButton;
        control->setObjectName(QStringLiteral("transportButton"));
        control->setProperty("command", command);
        control->setProperty("glyph", static_cast<int>(glyph));
        control->setIcon(transportIcon(glyph));
        control->setIconSize(QSize(14, 14));
        connect(control, &QToolButton::clicked, this, [this, command] { triggerCommand(command); });
        layout->addWidget(control);
        m_transportButtons.append(control);
        return control;
    };
    button(TransportGlyph::Start, QStringLiteral("playback.goToStart"));
    button(TransportGlyph::PreviousFrame, QStringLiteral("playback.previousFrame"));
    m_transportPlay = button(TransportGlyph::Play, QStringLiteral("playback.toggle"));
    button(TransportGlyph::NextFrame, QStringLiteral("playback.nextFrame"));
    button(TransportGlyph::End, QStringLiteral("playback.goToEnd"));
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
    }
    if (!m_commands) return;
    for (auto *button : m_transportButtons) {
        const QString command = button->property("command").toString();
        const QString tip = button == m_transportPlay
            ? m_commands->labelWithShortcut(command, m_playing ? QStringLiteral("Pause") : QStringLiteral("Play"))
            : m_commands->labelWithShortcut(command);
        button->setToolTip(tip);
        button->setAccessibleName(tip);
    }
}

void MainWindow::stepFrames(int frames) {
    const double rate = std::max(1.0, m_fps);
    queueSeek((std::round(m_playhead * rate) + frames) / rate);
}

void MainWindow::setupStatusBar() {
    statusBar()->setSizeGripEnabled(false);
    m_syncChip = new QToolButton;
    m_syncChip->setObjectName(QStringLiteral("syncChip"));
    m_syncChip->setCursor(Qt::PointingHandCursor);
    m_syncChip->hide();
    connect(m_syncChip, &QToolButton::clicked, this, [this] {
        if (m_commands->isEnabled(QStringLiteral("collab.sync"))) triggerCommand(QStringLiteral("collab.sync"));
        else if (!m_collabStatus.value(QStringLiteral("repository")).toBool()) triggerCommand(QStringLiteral("collab.publish"));
        else statusBar()->showMessage(m_commands->reason(QStringLiteral("collab.sync")), 4000);
    });
    statusBar()->addPermanentWidget(m_syncChip);
    m_statusRevision = new QLabel;
    m_statusRevision->setObjectName(QStringLiteral("statusRevision"));
    m_statusRevision->hide();
    m_statusConnection = new QLabel;
    m_statusConnection->setObjectName(QStringLiteral("statusConnection"));
    m_statusDevice = new QLabel;
    m_statusDevice->setObjectName(QStringLiteral("statusDevice"));
    m_statusDevice->setToolTip(QStringLiteral("The live preview is drawn as it plays"));
    m_statusDevice->hide();
    statusBar()->addPermanentWidget(m_statusDevice);
    m_statusAccount = new QLabel;
    m_statusAccount->setObjectName(QStringLiteral("statusAccount"));
    m_statusAccount->setToolTip(QStringLiteral("The agent's connection (Settings)"));
    statusBar()->addPermanentWidget(m_statusAccount);
    statusBar()->addPermanentWidget(m_statusRevision);
    statusBar()->addPermanentWidget(m_statusConnection);
    updateCloudStatus();
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
    if (!kerf_bridge_send(m_bridge, payload.constData())) {
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
    return kerf_bridge_send(m_bridge, bytes.constData());
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
        const QString message = event.value(QStringLiteral("message")).toString(QStringLiteral("Runtime error"));
        // How preset_save refuses a file that exists, after its path.
        static const QString presetExists = QStringLiteral(" already exists; choose another name or replace it");
        if (!m_savingPreset.isEmpty() && message.endsWith(presetExists)) {
            replaceSavedPreset(message.chopped(presetExists.size()));
        } else {
            showError(message);
            // A refused mask edit puts the outline back.
            updateViewerMasks();
        }
    } else if (type == QStringLiteral("preset_export")) {
        const QString path = event.value(QStringLiteral("path")).toString();
        const QByteArray document = event.value(QStringLiteral("document")).toString().toUtf8();
        QSaveFile file(path);
        if (!file.open(QIODevice::WriteOnly) || file.write(document) != document.size() || !file.commit()) {
            showError(QStringLiteral("Could not export preset: %1").arg(file.errorString()));
            return;
        }
        statusBar()->showMessage(QStringLiteral("Preset exported: %1").arg(QFileInfo(path).fileName()), 6000);
        addActivity(QStringLiteral("preset"), path);
    } else if (type == QStringLiteral("preset_inspect")) {
        showPresetBindings(event.value(QStringLiteral("path")).toString(), event.value(QStringLiteral("preset")).toObject());
    } else if (type == QStringLiteral("preset_applied")) {
        statusBar()->showMessage(QStringLiteral("Preset proposal ready for review"), 6000);
    } else if (type == QStringLiteral("preset_library")) {
        m_presets->setLibrary(event, m_projectDirectory);
    } else if (type == QStringLiteral("preset_saved")) {
        m_savingPreset.clear();
        const QString path = event.value(QStringLiteral("path")).toString();
        statusBar()->showMessage(QStringLiteral("Saved preset %1").arg(path), 6000);
        addActivity(QStringLiteral("preset"), path);
        execute(QStringLiteral("preset.library"));
    } else if (type == QStringLiteral("collab") || type == QStringLiteral("collab_sync") ||
               type == QStringLiteral("collab_pull_request")) {
        handleCollabEvent(event);
    } else if (type == QStringLiteral("media_status")) {
        m_mediaStatus = event;
        m_projectPanel->setMediaStatus(event);
        updateCommandStates();
    } else if (type == QStringLiteral("agent")) {
        const QString kind = event.value(QStringLiteral("kind")).toString(QStringLiteral("agent"));
        const QString text = event.value(QStringLiteral("text")).toString();
        if (kind == QStringLiteral("delta")) {
            if (m_agentText.isEmpty()) appendConversation(QStringLiteral("Kerf"), text);
            else {
                const bool follow = m_conversation->verticalScrollBar()->value() >= m_conversation->verticalScrollBar()->maximum() - 4;
                m_agentCursor.insertText(text);
                if (follow) m_conversation->verticalScrollBar()->setValue(m_conversation->verticalScrollBar()->maximum());
            }
            m_agentText += text;
        } else if (kind == QStringLiteral("done")) {
            if (m_agentText.isEmpty()) appendConversation(QStringLiteral("Kerf"), text);
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
        updateAiBar();
        updateCommandStates();
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
        updateCommandStates();
        // An animated clip's picture moves with the playhead, and its masks with it.
        if (!m_selectedClip.isEmpty()) updateViewerMasks();
    } else if (type == QStringLiteral("playback")) {
        m_playing = m_connected && event.value(QStringLiteral("playing")).toBool();
        updateTransport();
        if (m_playing) {
            m_hasPendingHoverSeek = false;
            m_hoverSkimming = false;
            m_timeline->clearSkimmer();
        }
        updateCommandStates();
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
    } else if (type == QStringLiteral("analysis")) {
        // Analyses effects draw from, made in the background.
        const QString message = event.value(QStringLiteral("message")).toString();
        statusBar()->showMessage(message, 6000);
        addActivity(QStringLiteral("analysis"), message);
    } else if (type == QStringLiteral("preview")) {
        const QString status = event.value(QStringLiteral("status")).toString();
        const QString message = event.value(QStringLiteral("message")).toString();
        if (!status.isEmpty()) m_previewStatus = status;
        if (m_statusDevice && !status.isEmpty()) {
            m_statusDevice->setText(QStringLiteral("Live"));
            m_statusDevice->setVisible(status == QStringLiteral("live"));
        }
        if (status == QStringLiteral("required") || status == QStringLiteral("unavailable")) {
            m_previewRequired = true;
            m_viewer->clearFrame();
            const QString detail = message.isEmpty() ? QStringLiteral("Rendered preview required") : message;
            statusBar()->setToolTip(detail);
            statusBar()->showMessage(detail.left(112) + (detail.size() > 112 ? QStringLiteral("…") : QString()), 6000);
            addActivity(QStringLiteral("preview"), message.isEmpty() ? QStringLiteral("Native preview unavailable") : message);
        } else if (status == QStringLiteral("running")) {
            m_previewRequired = true;
            m_exportProgress->setValue(static_cast<int>(std::clamp(event.value(QStringLiteral("progress")).toDouble(), 0.0, 1.0) * 100.0));
            m_exportProgress->show();
        } else if (status == QStringLiteral("ready")) {
            m_previewRequired = false;
            m_exportProgress->setValue(0);
            m_exportProgress->hide();
            statusBar()->setToolTip(QString());
            statusBar()->showMessage(QStringLiteral("Preview ready"), 4000);
        } else if (status == QStringLiteral("live")) {
            // The compositor draws the picture as it plays; a render still
            // gives the exact preview.
            m_previewRequired = false;
            m_exportProgress->setValue(0);
            m_exportProgress->hide();
            statusBar()->setToolTip(message);
            const QString note = message.isEmpty() ? QStringLiteral("Live preview")
                                                   : QStringLiteral("Live preview: ") + message;
            statusBar()->showMessage(note.left(112) + (note.size() > 112 ? QStringLiteral("…") : QString()), 4000);
        }
    } else if (type == QStringLiteral("cloud")) {
        m_cloudAccess = event.value(QStringLiteral("access")).toString();
        m_cloudAccount = event.value(QStringLiteral("account")).toObject();
        m_cloudMessage = event.value(QStringLiteral("message")).toString();
        updateCloudStatus();
    } else if (type == QStringLiteral("agent_next")) {
        showNextSteps(event.value(QStringLiteral("steps")).toArray());
    } else if (type == QStringLiteral("effect_preview")) {
        const int width = event.value(QStringLiteral("width")).toInt();
        const int height = event.value(QStringLiteral("height")).toInt();
        const QByteArray rgba = QByteArray::fromBase64(event.value(QStringLiteral("rgba")).toString().toLatin1());
        if (width > 0 && height > 0 && rgba.size() >= qsizetype(width) * height * 4) {
            const QImage image(reinterpret_cast<const uchar *>(rgba.constData()), width, height, width * 4,
                               QImage::Format_RGBA8888);
            m_effects->setPreview(event.value(QStringLiteral("clip")).toString(),
                                  event.value(QStringLiteral("fx")).toString(), image.copy());
        }
    } else if (type == QStringLiteral("caption")) {
        m_viewer->setCaption(event.value(QStringLiteral("text")).toString());
    }
    if (type == QStringLiteral("preview")) updateCommandStates();
}

void MainWindow::applySnapshot(const QJsonObject &snapshot) {
    const auto diverged = [](const QJsonObject &state) {
        return state.value(QStringLiteral("projectionStatus")).toString() == QStringLiteral("diverged");
    };
    // project.kerf changing outside Kerf (a pull in a terminal), or being
    // loaded, can come with other presets.
    const bool presetsMayDiffer = m_connected && diverged(m_snapshot) != diverged(snapshot);
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
    m_viewerDetail->setVisible(true);
    m_timelineDetail->setVisible(true);
    if (m_statusRevision) {
        m_statusRevision->setText(QStringLiteral("Revision %1").arg(m_revision));
        m_statusRevision->show();
    }
    updateTransport();
    m_artwork->request(sources, m_projectDirectory);
    m_projectPanel->setSources(sources);
    if (m_sourceMonitor->isOpen()) {
        const bool present = std::any_of(sources.cbegin(), sources.cend(), [this](const QJsonValue &value) {
            return value.toObject().value(QStringLiteral("id")).toString() == m_sourceMonitor->mediaId();
        });
        if (!present) m_sourceMonitor->close();
    }
    updateProposal(snapshot.value(QStringLiteral("pendingProposal")));
    m_timeline->setProposal(m_proposalId.isEmpty() ? QJsonObject{} : snapshot.value(QStringLiteral("proposalTimeline")).toObject());
    m_divergedBanner->setVisible(diverged(snapshot));
    updateInspector();
    updateCommandStates();
    if (presetsMayDiffer) execute(QStringLiteral("preset.library"));
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

QSizeF MainWindow::selectedPictureSize(bool *fit) const {
    // A picture clip draws media, a nested sequence, or a generator.
    const QJsonObject clip = selectedClipObject();
    const QString kind = clip.value(QStringLiteral("kind")).toString();
    if (clip.isEmpty() || selectedTrackType() != QStringLiteral("video") || kind == QStringLiteral("caption")) return {};
    *fit = kind == QStringLiteral("media") || kind == QStringLiteral("sequence");
    const QJsonArray sources = m_project.value(QStringLiteral("sources")).toArray();
    if (kind == QStringLiteral("media")) {
        for (const QJsonValue &value : sources) {
            const QJsonObject source = value.toObject();
            if (source.value(QStringLiteral("id")).toString() != clip.value(QStringLiteral("sourceId")).toString()) continue;
            const QSizeF size(source.value(QStringLiteral("width")).toDouble(), source.value(QStringLiteral("height")).toDouble());
            return size.isEmpty() ? QSizeF() : size;
        }
        return {};
    }
    // Generators fill the canvas; a nested sequence is fitted to it.
    return QSizeF(resolvedCanvasSize(m_project, sources));
}

void MainWindow::updateViewerMasks() {
    bool fit = false;
    const QSizeF picture = m_connected ? selectedPictureSize(&fit) : QSizeF();
    for (QAction *action : m_maskTools) action->setEnabled(!picture.isEmpty());
    if (picture.isEmpty()) {
        m_viewer->setMasks(QString(), QJsonArray{}, QTransform{});
        return;
    }
    const QJsonObject clip = selectedClipObject();
    const double clipTime = std::clamp(m_playhead - clip.value(QStringLiteral("timelineStart")).toDouble(), 0.0,
                                       clip.value(QStringLiteral("duration")).toDouble());
    const QSizeF canvas(resolvedCanvasSize(m_project, m_project.value(QStringLiteral("sources")).toArray()));
    m_viewer->setMasks(m_selectedClip, clip.value(QStringLiteral("masks")).toArray(),
                       picturePlacement(clip, picture, fit, canvas, clipTime));
}

void MainWindow::updateInspector() {
    const QJsonObject clip = selectedClipObject();
    updateViewerMasks();
    if (!m_connected || clip.isEmpty()) {
        m_effectControls->setClip(QString(), QJsonObject{}, QJsonArray{}, m_fps);
        m_effects->setClip(QString(), false);
        return;
    }
    m_effects->setClip(m_selectedClip, selectedTrackType() == QStringLiteral("video") &&
                                           clip.value(QStringLiteral("kind")).toString() == QStringLiteral("media"));
    // Effects on the clip may read the other picture tracks and draw the
    // sound tracks.
    QStringList pictureTracks;
    QStringList soundTracks;
    for (const QJsonValue &trackValue : m_project.value(QStringLiteral("tracks")).toArray()) {
        const QJsonObject track = trackValue.toObject();
        bool holdsClip = false;
        for (const QJsonValue &clipValue : track.value(QStringLiteral("clips")).toArray())
            holdsClip |= clipValue.toObject().value(QStringLiteral("id")).toString() == m_selectedClip;
        if (track.value(QStringLiteral("type")).toString() == QStringLiteral("video") && !holdsClip)
            pictureTracks << track.value(QStringLiteral("id")).toString();
        if (track.value(QStringLiteral("type")).toString() == QStringLiteral("audio"))
            soundTracks << track.value(QStringLiteral("id")).toString();
    }
    m_effectControls->setPictureTracks(pictureTracks);
    m_effectControls->setSoundTracks(soundTracks);
    // A generator's parameters, then the properties of the clip's track type.
    QJsonArray defs = m_project.value(QStringLiteral("generators")).toObject().value(clip.value(QStringLiteral("fx")).toString()).toArray();
    for (const QJsonValue &def : m_project.value(QStringLiteral("properties")).toObject().value(selectedTrackType()).toArray())
        defs.append(def);
    bool fit = false;
    m_effectControls->setMaskable(!selectedPictureSize(&fit).isEmpty());
    QStringList maskModes;
    for (const QJsonValue &mode : m_project.value(QStringLiteral("maskModes")).toArray()) maskModes << mode.toString();
    m_effectControls->setMaskCatalog(m_project.value(QStringLiteral("maskProperties")).toArray(), maskModes);
    m_effectControls->setClip(m_selectedClip, clip, defs, m_fps);
    m_effectControls->selectMask(m_viewer->selectedMask());
}

void MainWindow::updateProposal(const QJsonValue &value) {
    m_proposalId.clear();
    m_proposalBlocked = false;
    bool pending = value.isObject() && value.toObject().value(QStringLiteral("status")).toString() == QStringLiteral("pending");
    m_acceptButton->setEnabled(m_connected && pending);
    m_rejectButton->setEnabled(m_connected && pending);
    m_proposalBox->setVisible(pending);
    if (!pending) {
        m_proposalSummary->setText(QStringLiteral("No pending changes"));
        m_proposalDiff->clear();
        updateAiBar();
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
    if (stale) text += QStringLiteral("This proposal is out of date. Ask Kerf to prepare it again before accepting.\n\n");
    text += proposal.value(QStringLiteral("script")).toString();
    text += QStringLiteral("\n") + diff;
    m_proposalDiff->setPlainText(text);
    m_acceptButton->setEnabled(m_connected && !stale);
    m_proposalBlocked = stale;
    updateAiBar();
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
    const MessageKind kind = user ? MessageKind::User : error ? MessageKind::Error : MessageKind::Agent;
    frame.setProperty(kMessageKind, static_cast<int>(kind));
    styleMessage(frame, kind);
    frame.setBorder(1);
    frame.setBorderStyle(QTextFrameFormat::BorderStyle_Solid);
    cursor = cursor.insertFrame(frame)->firstCursorPosition();
    QTextBlockFormat block;
    block.setBottomMargin(7);
    cursor.setBlockFormat(block);
    QTextCharFormat role;
    role.setFontWeight(QFont::DemiBold);
    role.setFontPointSize(9);
    role.setProperty(kMessageKind, static_cast<int>(kind));
    role.setForeground(messageRoleColor(kind));
    cursor.insertText(user ? QStringLiteral("YOU") : error ? QStringLiteral("COULDN’T COMPLETE EDIT") : QStringLiteral("KERF"), role);
    QTextBlockFormat body;
    body.setLineHeight(145, QTextBlockFormat::ProportionalHeight);
    // The body takes the conversation's own text colour, which follows the appearance.
    QTextCharFormat bodyText;
    cursor.insertBlock(body, bodyText);
    cursor.insertText(text);
    if (speaker == QStringLiteral("Kerf")) m_agentCursor = cursor;
    if (follow) m_conversation->verticalScrollBar()->setValue(m_conversation->verticalScrollBar()->maximum());
}

void MainWindow::applyAppearance() {
    for (const auto &[action, glyph] : std::as_const(m_toolbarGlyphs)) action->setIcon(toolbarIcon(glyph));
    for (QToolButton *button : std::as_const(m_transportButtons)) {
        const auto glyph = static_cast<TransportGlyph>(button->property("glyph").toInt());
        button->setIcon(transportIcon(button == m_transportPlay && m_transportShowsPause ? TransportGlyph::Pause : glyph));
    }
    for (QAction *action : std::as_const(m_maskTools)) action->setIcon(maskToolIcon(static_cast<MaskTool>(action->data().toInt())));
    if (m_conversationEmpty) {
        m_conversation->setHtml(conversationIntro(editorTheme()));
        return;
    }
    // Recolour the messages already shown: their frames and the speaker
    // names over them; bodies take the conversation's text colour.
    QTextDocument *document = m_conversation->document();
    QList<QPair<QTextFrame *, MessageKind>> frames;
    for (QTextFrame *frame : document->rootFrame()->childFrames()) {
        const int kind = frame->frameFormat().intProperty(kMessageKind);
        if (kind) frames.append({frame, static_cast<MessageKind>(kind)});
    }
    for (const auto &[frame, kind] : std::as_const(frames)) {
        QTextFrameFormat format = frame->frameFormat();
        styleMessage(format, kind);
        frame->setFrameFormat(format);
    }
    struct Run { int start; int end; MessageKind kind; };
    QList<Run> runs;
    for (QTextBlock block = document->begin(); block.isValid(); block = block.next()) {
        for (auto it = block.begin(); !it.atEnd(); ++it) {
            const QTextFragment fragment = it.fragment();
            if (const int kind = fragment.charFormat().intProperty(kMessageKind))
                runs.append({fragment.position(), fragment.position() + fragment.length(), static_cast<MessageKind>(kind)});
        }
    }
    for (const Run &run : std::as_const(runs)) {
        QTextCursor cursor(document);
        cursor.setPosition(run.start);
        cursor.setPosition(run.end, QTextCursor::KeepAnchor);
        QTextCharFormat color;
        color.setForeground(messageRoleColor(run.kind));
        cursor.mergeCharFormat(color);
    }
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
    if (m_statusConnection) {
        const QString state = m_connected ? QStringLiteral("ready")
            : status == QStringLiteral("connecting") ? QStringLiteral("connecting") : QStringLiteral("disconnected");
        m_statusConnection->setText(state == QStringLiteral("ready") ? QStringLiteral("● Connected")
            : state == QStringLiteral("connecting") ? QStringLiteral("● Connecting…") : QStringLiteral("● Offline"));
        m_statusConnection->setProperty("state", state);
        m_statusConnection->style()->unpolish(m_statusConnection);
        m_statusConnection->style()->polish(m_statusConnection);
    }
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
    if (m_presets) m_presets->setConnected(m_connected);
    updateCommandStates();
    if (m_connected && !wasConnected) startSession();
    QTimer::singleShot(0, this, [this] {
        if (m_timelineScroll && m_timeline && m_timelineScroll->viewport()->width() > 0) {
            m_timeline->fitToWidth(m_timelineScroll->viewport()->width());
            m_timelineScroll->horizontalScrollBar()->setValue(0);
        }
    });
}

} // namespace kerf
