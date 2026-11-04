// MainWindow's status bar and connection state.
// The region's state and its building live in ToolbarWiring.

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
#include "wiring/toolbar_wiring.h"

namespace kerf {

using namespace window_parts;

// The sync chip (mockup-spec 9): the branch glyph, "branch ↑a ↓b" and the
// sync's state; its tip "Revision N · k commits to pull"; a click opens the
// Sync menu. ToolbarWiring::updateSyncChip words it.
void MainWindow::updateSyncChip() {
    if (m_toolbarWiring) m_toolbarWiring->updateSyncChip();
}

// The status bar (mockup-spec 9): the selection, the rate and dropped
// frames, the guest, then the draft export, the keymap chip, the sync chip
// and the agent's connection. ToolbarWiring::buildStatusBar builds it.
void MainWindow::setupStatusBar() {
    m_toolbarWiring->buildStatusBar();
    updateCloudStatus();
    updateSyncChip();
}

void MainWindow::setConnection(const QString &status, const QString &message) {
    const bool wasConnected = m_connected;
    m_connected = status == QStringLiteral("ready");
    m_connectionBanner->setVisible(!m_connected);
    m_reconnectButton->setEnabled(status == QStringLiteral("disconnected"));
    m_connectionMessage->setText(status == QStringLiteral("connecting") ? QStringLiteral("Connecting to project…")
        : QStringLiteral("Project disconnected. %1").arg(message));
    m_timeline->setEnabled(m_connected);
    if (m_formatBank) m_formatBank->setEnabled(m_connected);
    m_chatInput->setEnabled(m_connected && !m_agentBusy);
    if (m_toolbarWiring) {
        const QString state = m_connected ? QStringLiteral("ready")
            : status == QStringLiteral("connecting") ? QStringLiteral("connecting") : QStringLiteral("disconnected");
        // A new connection has played nothing yet.
        if (m_connected != wasConnected) m_toolbarWiring->m_dropped = -1;
        m_toolbarWiring->m_connection = state;
        m_toolbarWiring->updateAgent();
        m_toolbarWiring->updateRate();
    }
    if (!m_connected) {
        m_playing = false;
        m_bus->setPlaying(false);
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
