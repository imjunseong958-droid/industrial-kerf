// MainWindow's status bar and connection state.
// Moved verbatim from main_window.cpp (Q-SKELETON, W1); Q-TOOLBAR owns it in W2.

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

namespace kerf {

using namespace window_parts;

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

void MainWindow::setupStatusBar() {
    statusBar()->setSizeGripEnabled(false);
    // 24 points tall (mockup-spec 0.1), whatever its items ask for.
    statusBar()->setFixedHeight(tokens::layout::status);
    // On the left, the selection ("3 clips selected · 00:00:04:18"), or nothing.
    m_selectionStatus = new QLabel;
    m_selectionStatus->setObjectName(QStringLiteral("statusSelection"));
    statusBar()->addWidget(m_selectionStatus, 1);
    statusBar()->addPermanentWidget(m_exportProgress);
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

void MainWindow::setConnection(const QString &status, const QString &message) {
    const bool wasConnected = m_connected;
    m_connected = status == QStringLiteral("ready");
    m_connectionBanner->setVisible(!m_connected);
    m_reconnectButton->setEnabled(status == QStringLiteral("disconnected"));
    m_connectionMessage->setText(status == QStringLiteral("connecting") ? QStringLiteral("Connecting to project…")
        : QStringLiteral("Project disconnected. %1").arg(message));
    m_agentStatus->setText(m_connected ? (m_agentBusy ? QStringLiteral("Thinking") : QStringLiteral("Ready"))
        : status == QStringLiteral("connecting") ? QStringLiteral("Connecting") : QStringLiteral("Offline"));
    m_timeline->setEnabled(m_connected);
    if (m_formatBank) m_formatBank->setEnabled(m_connected);
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
