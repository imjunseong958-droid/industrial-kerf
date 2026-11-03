// MainWindow's Inspector column: the Inspector (its Video, Audio and Info
// panes), its mask tools, the Source page and the Deliver pane. The region's
// own state and bus traffic are in ui/wiring/inspector_wiring.

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
#include "wiring/inspector_wiring.h"

namespace kerf {

using namespace window_parts;

void MainWindow::updateInspector() {
    const QJsonObject clip = selectedClipObject();
    updateViewerMasks();
    m_inspectorWiring->attachCommands();
    m_inspector->setPlayhead(m_playhead);
    // What the open proposals would change: the snapshot's proposals and
    // their timelines (the first open one alone from an older bridge).
    QJsonArray proposals = m_snapshot.value(QStringLiteral("proposals")).toArray();
    QJsonObject timelines = m_snapshot.value(QStringLiteral("proposalTimelines")).toObject();
    if (proposals.isEmpty() && m_snapshot.value(QStringLiteral("pendingProposal")).isObject()) {
        const QJsonObject pending = m_snapshot.value(QStringLiteral("pendingProposal")).toObject();
        proposals.append(pending);
        timelines.insert(pending.value(QStringLiteral("proposalId")).toString(), m_snapshot.value(QStringLiteral("proposalTimeline")).toObject());
    }
    m_inspector->setProposals(proposals, timelines);
    if (!m_connected || clip.isEmpty()) {
        m_effects->setClip(QString(), false);
        m_inspector->setMaskable(false);
        m_inspector->setState(m_connected ? m_project : QJsonObject{}, {});
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
    m_inspector->setPictureTracks(pictureTracks);
    m_inspector->setSoundTracks(soundTracks);
    bool fit = false;
    m_inspector->setMaskable(!selectedPictureSize(&fit).isEmpty());
    QStringList maskModes;
    for (const QJsonValue &mode : m_project.value(QStringLiteral("maskModes")).toArray()) maskModes << mode.toString();
    m_inspector->setMaskCatalog(m_project.value(QStringLiteral("maskProperties")).toArray(), maskModes);
    m_inspector->selectMask(m_viewer->selectedMask());
    m_inspector->setState(m_project, m_selectedClips.size() > 1 ? m_selectedClips : QStringList{m_selectedClip});
}

// The Inspector column: the Inspector, its mask tools, the Source page.
void MainWindow::buildInspector() {
    m_inspector = new InspectorPanel;
    m_inspector->setArtwork(m_artwork);
    // Everything the catalog offers, sound effects included (audio.eq for
    // the Audio pane's Channel EQ); the Effects browser's picture catalog
    // joins it later.
    if (char *catalog = kerf_fx_catalog_all()) {
        m_inspector->setEffectCatalog(QJsonDocument::fromJson(QByteArray(catalog)).array());
        kerf_bridge_string_free(catalog);
    }
    connect(m_inspector, &InspectorPanel::editRequested, this, &MainWindow::sendEdit);
    // Done typing in the Inspector (or Esc there): the timeline's keys again.
    connect(m_inspector, &InspectorPanel::focusReturnRequested, this, [this] { m_timeline->setFocus(Qt::OtherFocusReason); });
    connect(m_inspector, &InspectorPanel::askRequested, this, &MainWindow::askAboutInChat);
    connect(m_inspector, &InspectorPanel::revealRequested, this, [this](const QString &) {
        if (ads::CDockWidget *project = m_workspace->panel(QStringLiteral("project"))) {
            if (project->isClosed()) project->toggleView(true);
            project->setAsCurrentTab();
        }
    });
    connect(m_inspector, &InspectorPanel::addEffectRequested, this, [this](bool) {
        // The Effects browser, its search ready; adding brings the
        // Inspector back with the new effect open.
        m_workspace->showBrowserForInspector(QStringLiteral("effects"));
        m_effects->focusSearch();
        m_effectsFromInspector = true;
    });
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
    m_inspector->setMaskTools(m_maskTools);
    connect(m_viewer, &ViewerWidget::maskToolChanged, this, [this](MaskTool tool) {
        for (QAction *action : m_maskTools) action->setChecked(action->data().toInt() == static_cast<int>(tool));
    });
    connect(m_viewer, &ViewerWidget::editRequested, this, &MainWindow::sendEdit);
    connect(m_viewer, &ViewerWidget::maskSelected, m_inspector, &InspectorPanel::selectMask);
    connect(m_inspector, &InspectorPanel::maskSelected, m_viewer, &ViewerWidget::selectMask);
    // The column: Inspector | Source over the kinds, the Deliver pane in Deliver.
    m_inspectorColumn = new InspectorColumn(nullptr, m_bus);
    m_inspectorColumn->setInspector(m_inspector);
    m_inspectorColumn->setSourceMonitor(m_sourceMonitor);
    connect(m_inspectorColumn, &InspectorColumn::kindChanged, this, [this] {
        rememberStage();
        syncPanelCommands();
    });
    connect(m_inspectorColumn, &InspectorColumn::pageChanged, this, &MainWindow::syncPanelCommands);
    m_inspectorWiring->attach();
}

} // namespace kerf
