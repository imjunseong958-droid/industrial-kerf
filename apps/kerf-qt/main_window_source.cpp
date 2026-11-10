// MainWindow's Source monitors.
// Q-SOURCE (W2): both viewers show the source wiring's one SourceState.

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
#include "editor_bus.h"
#include "inspector_column.h"
#include "source_state.h"
#include "wiring/source_wiring.h"

namespace kerf {

using namespace window_parts;

void MainWindow::openSource(const QString &mediaId) {
    for (const QJsonValue &value : m_project.value(QStringLiteral("sources")).toArray()) {
        const QJsonObject source = value.toObject();
        if (source.value(QStringLiteral("id")).toString() != mediaId) continue;
        m_sourceMonitor->open(source, QDir(m_projectDirectory).absoluteFilePath(source.value(QStringLiteral("path")).toString()));
        // On the Inspector sheet's Source page, opened for it.
        m_shell->setSheet(StageShell::Sheet::Inspector);
        m_inspectorColumn->setPage(InspectorColumn::Page::Source);
        syncPanelCommands();
        m_sourceMonitor->setFocus();
        return;
    }
}

QStringList MainWindow::sourceTargets() const {
    // The timeline's targeted track of each kind the source media has (its
    // heads' target toggles; by default the first unlocked of each kind).
    // A kind whose target is switched off is not edited.
    const QJsonObject source = m_sourceMonitor->source();
    const QStringList targeted = m_timeline ? m_timeline->targetTracks() : QStringList{};
    QStringList targets;
    for (const QString &kind : {QStringLiteral("video"), QStringLiteral("audio")}) {
        const bool has = kind == QStringLiteral("video") ? source.contains(QStringLiteral("width"))
                                                         : source.value(QStringLiteral("hasAudio")).toBool();
        if (!has) continue;
        for (const QJsonValue &value : m_project.value(QStringLiteral("tracks")).toArray()) {
            const QJsonObject track = value.toObject();
            const QString id = track.value(QStringLiteral("id")).toString();
            if (track.value(QStringLiteral("type")).toString() == kind && !track.value(QStringLiteral("locked")).toBool() &&
                targeted.contains(id)) {
                targets.append(id);
                break;
            }
        }
    }
    return targets;
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

// The Source viewer: the Inspector column's Source pane, showing the
// SourceState the region's wiring keeps.
void MainWindow::buildSource() {
    SourceState *state = m_sourceWiring->state();
    state->setArtwork(m_artwork);
    m_sourceMonitor = new SourceMonitorPanel(state);
    connect(m_sourceMonitor, &SourceMonitorPanel::changed, this, &MainWindow::updateCommandStates);
    m_sourceWiring->attach();
}

} // namespace kerf
