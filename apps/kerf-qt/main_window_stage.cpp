// MainWindow's Program stage (Q-STAGE, W2): the viewer's stage with the
// device, the rail, the comments panel, the scrub strip, the agent HUD and
// the Source column (placed by ViewerWidget::stageLayout), the Program strip,
// the device previews and safe zones, the transport's state and the masks.
// The stage's own state and its bus traffic are StageWiring's.

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
#include "workspace.h"
#include "editor_common.h"
#include "editor_theme.h"
#include "media_artwork.h"
#include "timeline_widget.h"
#include "toolbar_widgets.h"
#include "player_overlay.h"
#include "program_level.h"
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
#include <QStatusBar>
#include <QTextBlock>
#include <QTextDocument>
#include <QTextFrame>
#include <QToolBar>
#include <QUrl>
#include <QVBoxLayout>
#include <QWindow>

#include <DockManager.h>
#include <DockWidget.h>
#include <DockWidgetTab.h>

#include <algorithm>
#include <numeric>
#include <limits>
#include <utility>
#include "wiring/main_window_shared.h"
#include "browser_panel.h"
#include "deliver_tray.h"
#include "editor_bus.h"
#include "finish_tray.h"
#include "inspector_column.h"
#include "program_strip.h"
#include "stage_rail.h"
#include "wiring/stage_wiring.h"

namespace kerf {

using namespace window_parts;

namespace {

QString overlayCommand(PlayerOverlay::Control control) {
    switch (control) {
    case PlayerOverlay::Control::StepBack: return QStringLiteral("playback.previousFrame");
    case PlayerOverlay::Control::StepForward: return QStringLiteral("playback.nextFrame");
    case PlayerOverlay::Control::Play: break;
    }
    return QStringLiteral("playback.toggle");
}

} // namespace

void MainWindow::setupPlayerOverlay() {
    // Each control runs a playback command, as the keys do; updateTransport
    // names it with the command's shortcut.
    PlayerOverlay *overlay = m_viewer->overlay();
    connect(overlay, &PlayerOverlay::controlTriggered, this,
            [this](PlayerOverlay::Control control) { triggerCommand(overlayCommand(control)); });
    connect(overlay, &PlayerOverlay::seekRequested, this, [this](double fraction) {
        if (m_duration <= 0.0) return;
        queueSeek(fraction * m_duration);
    });
}

void MainWindow::updateProgramLevel() {
    // The rail's meter follows the bridge's `meter` events itself (ProgramMeter);
    // nothing is estimated from the clips any more.
}

void MainWindow::updateTransport() {
    // Every clock tick, play and pause, seek and new snapshot comes here.
    updateProgramLevel();
    PlayerOverlay *overlay = m_viewer ? m_viewer->overlay() : nullptr;
    if (!overlay) return;
    overlay->setTime(m_playhead, m_duration, m_fps);
    overlay->setPlaying(m_playing);
    if (!m_commands) return;
    for (const PlayerOverlay::Control control :
         {PlayerOverlay::Control::Play, PlayerOverlay::Control::StepBack, PlayerOverlay::Control::StepForward}) {
        const QString command = overlayCommand(control);
        overlay->setControlTip(control, control == PlayerOverlay::Control::Play
                                            ? m_commands->labelWithShortcut(command, m_playing ? QStringLiteral("Pause") : QStringLiteral("Play"))
                                            : m_commands->labelWithShortcut(command));
    }
}

void MainWindow::stepFrames(int frames) {
    const double rate = std::max(1.0, m_fps);
    queueSeek((std::round(m_playhead * rate) + frames) / rate);
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

void MainWindow::fitPanelDetails() {
    // A title strip's details keep the width of their text when it changes
    // after the strip was laid out (as the first snapshot's does), rather
    // than shrinking to an ellipsis; the tabs beside them give way first.
    for (QToolButton *button : findChildren<QToolButton *>(m_viewerDetail->objectName())) {
        button->setMinimumWidth(button->sizeHint().width());
        button->updateGeometry();
    }
}

void MainWindow::setDevicePreview(DevicePreview preview) {
    m_viewer->setDevicePreview(preview);
    QSettings().setValue(QStringLiteral("viewer/devicePreview"), devicePreviewKey(preview));
    updateViewerRail();
}

void MainWindow::setLaptopPreview(LaptopPreview preview) {
    m_viewer->setLaptopPreview(preview);
    QSettings().setValue(QStringLiteral("viewer/laptopPreview"), laptopPreviewKey(preview));
    updateViewerRail();
}

void MainWindow::setSafeZones(bool shown) {
    m_viewer->setSafeZones(shown);
    QSettings().setValue(QStringLiteral("viewer/safeZones"), shown);
    updateViewerRail();
}

void MainWindow::showDeviceMenu() {
    // The rail's own Device menu; its choice comes back as devicePreviewRequested.
    m_rail->showDeviceMenu();
}

void MainWindow::updateViewerRail() {
    // The phone is the phone formats' (9:16, 1:1, 4:5), the laptop the wide
    // ones': the rail offers the device the picture's shape shows in.
    const bool phone = m_viewer->phoneFormat();
    const DevicePreview preview = m_viewer->devicePreview();
    const LaptopPreview laptop = m_viewer->laptopPreview();
    const bool shown = phone ? preview != DevicePreview::Off : laptop != LaptopPreview::Off;
    RailButton *device = m_rail->deviceButton();
    {
        const QSignalBlocker blocker(device);
        device->setChecked(shown);
    }
    device->setGlyph(phone ? RailButton::Glyph::Phone : RailButton::Glyph::Laptop);
    m_rail->setDevicePreview(m_viewer->pictureAspect(), shown);
    for (QAction *action : std::as_const(m_deviceActions)) {
        const QSignalBlocker blocker(action);
        action->setChecked(static_cast<DevicePreview>(action->data().toInt()) == preview);
        // "Phone · Feed" for a square or 4:5 picture.
        if (static_cast<DevicePreview>(action->data().toInt()) != DevicePreview::Off)
            action->setText(devicePreviewLabel(DevicePreview::Phone, m_viewer->pictureAspect()));
    }
    for (QAction *action : std::as_const(m_laptopActions)) {
        const QSignalBlocker blocker(action);
        action->setChecked(static_cast<LaptopPreview>(action->data().toInt()) == laptop);
    }
    if (m_safeZonesAction) {
        const QSignalBlocker blocker(m_safeZonesAction);
        m_safeZonesAction->setChecked(m_viewer->safeZonesShown());
    }
    // View ▸ Safe Zones and Device Preview, and the Program strip's.
    if (m_commands) {
        const QList<QPair<QString, bool>> switches{{QStringLiteral("view.safeZones"), m_viewer->safeZonesShown()},
                                                   {QStringLiteral("view.devicePreview"), shown}};
        for (const auto &[id, on] : switches)
            if (QAction *action = m_commands->action(id); action && action->isChecked() != on) {
                {
                    const QSignalBlocker blocker(action);
                    action->setChecked(on);
                }
                // A blocked toggle tells the strip's button nothing: tell it.
                emit action->changed();
            }
    }
}

// The Program monitor: its strip over the viewer's stage; in the stage the
// device, the rail and the comments panel (built by buildReview), the scrub
// strip and the agent HUD (StageWiring), and later the Source column.
void MainWindow::buildStage() {
    m_viewerFrame = new QFrame;
    m_viewerFrame->setObjectName(QStringLiteral("viewerFrame"));
    auto *viewerLayout = new QVBoxLayout(m_viewerFrame);
    viewerLayout->setContentsMargins(0, 0, 0, 0);
    viewerLayout->setSpacing(0);
    // The Program strip (28) over the stage, as the dock's title strip.
    m_programStrip = new ProgramStrip(nullptr, m_bus);
    viewerLayout->addWidget(m_programStrip);
    m_viewer = new ViewerWidget;
    viewerLayout->addWidget(m_viewer, 1);
    setupPlayerOverlay();

    m_viewer->setStage(m_rail, m_chat);
    m_stageWiring->build();
    {
        const QSettings settings;
        // The panel's width beside a 16:9 picture and beside a phone, each
        // remembered on this Mac.
        m_viewer->setChatWidth(false, settings.value(QStringLiteral("viewer/chatWidth"), ViewerWidget::kDefaultChatWidth).toInt());
        m_viewer->setChatWidth(true, settings.value(QStringLiteral("viewer/chatWidthPhone"), ViewerWidget::kPhoneChatWidth).toInt());
        m_viewer->setChatOpen(settings.value(QStringLiteral("viewer/chatOpen"), true).toBool());
        m_viewer->setDevicePreview(devicePreviewFromKey(settings.value(QStringLiteral("viewer/devicePreview")).toString()));
        m_viewer->setLaptopPreview(laptopPreviewFromKey(settings.value(QStringLiteral("viewer/laptopPreview")).toString()));
        m_viewer->setSafeZones(settings.value(QStringLiteral("viewer/safeZones"), true).toBool());
    }
    {
        auto *devices = new QActionGroup(this);
        for (const DevicePreview preview : {DevicePreview::Phone, DevicePreview::Off}) {
            auto *action = new QAction(devicePreviewLabel(preview), devices);
            action->setObjectName(QStringLiteral("viewer.device.%1").arg(devicePreviewKey(preview)));
            action->setCheckable(true);
            action->setData(static_cast<int>(preview));
            action->setToolTip(QStringLiteral("Show a 9:16, 1:1 or 4:5 picture in a phone, or as a plain picture"));
            connect(action, &QAction::triggered, this, [this, preview] { setDevicePreview(preview); });
            m_deviceActions.append(action);
        }
        auto *laptops = new QActionGroup(this);
        for (const LaptopPreview preview : {LaptopPreview::Laptop, LaptopPreview::Off}) {
            auto *action = new QAction(preview == LaptopPreview::Off ? QStringLiteral("Laptop · Off") : laptopPreviewLabel(preview), laptops);
            action->setObjectName(QStringLiteral("viewer.laptop.%1").arg(laptopPreviewKey(preview)));
            action->setCheckable(true);
            action->setData(static_cast<int>(preview));
            action->setToolTip(QStringLiteral("Show a wide picture in a laptop, or as a plain picture"));
            connect(action, &QAction::triggered, this, [this, preview] { setLaptopPreview(preview); });
            m_laptopActions.append(action);
        }
        m_safeZonesAction = new QAction(QStringLiteral("Safe Zones"), this);
        m_safeZonesAction->setObjectName(QStringLiteral("viewer.safeZones"));
        m_safeZonesAction->setCheckable(true);
        m_safeZonesAction->setToolTip(QStringLiteral("Action safe (93%) and title safe (90%) over the picture"));
        connect(m_safeZonesAction, &QAction::toggled, this, &MainWindow::setSafeZones);
        // The rail's Device menu chose the device or the plain picture.
        connect(m_rail, &StageRail::devicePreviewRequested, this, [this](bool on) {
            if (m_viewer->phoneFormat()) setDevicePreview(on ? DevicePreview::Phone : DevicePreview::Off);
            else setLaptopPreview(on ? LaptopPreview::Laptop : LaptopPreview::Off);
        });
        updateViewerRail();
    }
    connect(m_chat, &ChatPanel::widthDragged, m_viewer, qOverload<int>(&ViewerWidget::setChatWidth));
    connect(m_chat, &ChatPanel::widthDragFinished, this, [this] {
        QSettings().setValue(m_viewer->phoneFormat() ? QStringLiteral("viewer/chatWidthPhone") : QStringLiteral("viewer/chatWidth"),
                             m_viewer->chatWidth());
    });
}

// The Program strip's switches: the mask tools (⇧R ⇧E ⇧P), then
// annotations, safe zones and dim, then the zoom; registry actions, so they
// show the menus' state, reasons and shortcuts. Then the stage's wiring.
void MainWindow::setupProgramStrip() {
    const QList<QKeySequence> maskKeys{QKeySequence(Qt::SHIFT | Qt::Key_R), QKeySequence(Qt::SHIFT | Qt::Key_E),
                                       QKeySequence(Qt::SHIFT | Qt::Key_P)};
    for (int index = 0; index < m_maskTools.size() && index < maskKeys.size(); ++index) {
        QAction *tool = m_maskTools.at(index);
        if (tool->shortcut().isEmpty()) tool->setShortcut(maskKeys.at(index));
        // Its key works wherever the window has the focus.
        if (!m_programStrip->QWidget::actions().contains(tool)) m_programStrip->addAction(tool);
    }
    QList<QAction *> actions(m_maskTools);
    actions << nullptr;
    for (const char *id : {"view.annotations", "view.safeZones", "view.dim"}) actions << m_commands->action(QString::fromLatin1(id));
    actions << nullptr;
    if (QAction *zoom = m_commands->action(QStringLiteral("viewer.zoom"))) {
        zoom->setIconText(QStringLiteral("Fit"));
        actions << zoom;
    }
    m_programStrip->setActions(actions);
    m_stageWiring->attach();
}

} // namespace kerf
