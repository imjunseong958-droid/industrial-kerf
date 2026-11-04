// MainWindow's connection state, and its passing messages (shown in the
// toolbar's subtitle by ToolbarWiring).

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
#include "browser_panel.h"
#include "deliver_tray.h"
#include "editor_bus.h"
#include "finish_tray.h"
#include "inspector_column.h"
#include "wiring/toolbar_wiring.h"

namespace kerf {

using namespace window_parts;

void MainWindow::showStatus(const QString &message, int timeout) {
    if (m_toolbarWiring) m_toolbarWiring->showMessage(message, timeout);
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
        // The toolbar's subtitle says so while it lasts.
        m_toolbarWiring->m_connection = m_connected ? QStringLiteral("ready")
            : status == QStringLiteral("connecting") ? QStringLiteral("connecting") : QStringLiteral("disconnected");
        layoutProjectTitle();
    }
    if (!m_connected) {
        m_playing = false;
        m_bus->setPlaying(false);
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
    if (m_connected && !wasConnected) {
        startSession();
        // The timeline was disabled, which handed the keys to the next control.
        settleFocus();
    }
    QTimer::singleShot(0, this, [this] {
        if (m_timelineScroll && m_timeline && m_timelineScroll->viewport()->width() > 0) {
            m_timeline->fitToWidth(m_timelineScroll->viewport()->width());
            m_timelineScroll->horizontalScrollBar()->setValue(0);
        }
    });
}

} // namespace kerf
