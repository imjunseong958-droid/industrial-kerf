// MainWindow's toolbar: the title, the format, the workspaces menu.
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

void MainWindow::canvasPresetChanged(const QSize &size) {
    if (m_updatingCanvasPreset) return;
    // The bank latched the key already; anything that cannot become the
    // project's format lets it go back to the format the project has.
    if (!m_connected || size.width() < 2 || size.height() < 2) {
        syncCanvasPreset(m_canvasSize);
        return;
    }
    if (!execute(QStringLiteral("sequence.setFormat"),
                 QJsonObject{{QStringLiteral("width"), size.width()}, {QStringLiteral("height"), size.height()}}))
        syncCanvasPreset(m_canvasSize);
}

void MainWindow::syncCanvasPreset(const QSize &size) {
    if (!m_formatBank || !size.isValid()) return;
    m_canvasSize = size;
    layoutProjectTitle();
    // The bank latches the preset's key, or none for a custom size, and
    // names the format in its tooltip; it does not ask for a change.
    m_updatingCanvasPreset = true;
    m_formatBank->setCanvasSize(size);
    m_updatingCanvasPreset = false;
}

void MainWindow::buildWorkspaceMenu(QMenu *menu) {
    // As mockup-spec 10 lists it: the built-ins with their keys (the
    // registry's actions, the current one checked), one's own (saved),
    // then saving and resetting the current one.
    menu->clear();
    qDeleteAll(menu->findChildren<QActionGroup *>(Qt::FindDirectChildrenOnly));
    QAction *header = menu->addAction(QStringLiteral("Workspaces"));
    header->setObjectName(QStringLiteral("workspacesHeader"));
    header->setEnabled(false);
    syncWorkspaceCommands();
    auto *group = new QActionGroup(menu);
    for (const QString &name : m_workspace->names()) {
        if (Workspace::isBuiltIn(name)) {
            if (QAction *builtIn = m_commands->action(QStringLiteral("workspace.%1").arg(name.toLower()))) menu->addAction(builtIn);
            continue;
        }
        QAction *choice = menu->addAction(name);
        choice->setObjectName(QStringLiteral("workspace.%1").arg(name));
        choice->setCheckable(true);
        choice->setChecked(name == m_workspace->current());
        choice->setToolTip(QStringLiteral("%1 · saved").arg(name));
        group->addAction(choice);
        connect(choice, &QAction::triggered, this, [this, name] { m_workspace->activate(name); });
    }
    menu->addSeparator();
    for (const char *id : {"window.saveWorkspace", "window.saveWorkspaceAs", "window.resetWorkspace"})
        if (QAction *action = m_commands->action(QString::fromLatin1(id))) menu->addAction(action);
}

void MainWindow::fitTitleBar() {
#if KERF_EDITOR_OWN_TITLE_BAR
    if (!m_toolbar) return;
    // The safe area's top is the title bar the window buttons sit in. Its
    // commands start 12 points past the zoom button; in full screen there
    // are no buttons beside them.
    const QWindow *handle = windowHandle();
    m_toolbar->setFixedHeight(std::max(kToolbarHeight, handle ? handle->safeAreaMargins().top() : 0));
    const int buttons = windowButtonsWidth(this);
    m_windowButtons->setFixedWidth(buttons > 0 ? buttons + 12 - kToolbarPadding : kWindowButtonsSpace);
    m_windowButtons->setVisible(!isFullScreen());
#endif
}

void MainWindow::layoutProjectTitle() {
    if (!m_projectTitle || !m_toolbar || !m_toolbarLeftEnd || !m_formatBank) return;
    m_projectTitle->ensurePolished();
    m_projectSubtitle->ensurePolished();
    const QFontMetrics metrics(m_projectTitle->font());
    const QFontMetrics subMetrics(m_projectSubtitle->font());
    const QString subtitle = projectSubtitle();
    // The room between the groups, and the title's own width.
    const int left = m_toolbarLeftEnd->geometry().right() + 1 + kTitleClearance;
    const int right = m_formatBank->geometry().left() - kTitleClearance;
    const int natural = std::max(metrics.horizontalAdvance(m_projectName), subMetrics.horizontalAdvance(subtitle)) + 2;
    const int width = std::min(natural, std::max(0, right - left));
    // Centred on the window (the toolbar spans it); moved aside when a group
    // is in the way, and shortened when there is no room for all of it.
    const int x = std::clamp((m_toolbar->width() - width) / 2, left, std::max(left, right - width));
    const QString shown = metrics.horizontalAdvance(m_projectName) + 2 > width ? metrics.elidedText(m_projectName, Qt::ElideRight, width) : m_projectName;
    m_projectTitle->setText(shown);
    m_projectTitle->setToolTip(shown == m_projectName ? QString() : m_projectName);
    m_projectSubtitle->setText(subMetrics.elidedText(subtitle, Qt::ElideRight, width));
    // Two lines centred on the toolbar's height, as macOS stacks a document's
    // title over its subtitle.
    const int titleHeight = metrics.height();
    const int subHeight = subMetrics.height();
    const int top = std::max(0, (m_toolbar->height() - titleHeight - subHeight) / 2);
    m_projectTitle->setGeometry(x, top, width, titleHeight);
    m_projectSubtitle->setGeometry(x, top + titleHeight, width, subHeight);
    m_projectTitle->raise();
    m_projectSubtitle->raise();
}

QString MainWindow::projectSubtitle() const {
    // Saved, or edited since the project's last version (Git's own say);
    // its shape as the format picker names it; its frame rate.
    const bool edited = m_collabStatus.value(QStringLiteral("changed")).toBool();
    QStringList parts{edited ? QStringLiteral("Edited") : QStringLiteral("Saved")};
    if (m_canvasSize.isValid()) {
        const int w = m_canvasSize.width();
        const int h = m_canvasSize.height();
        const int common = std::gcd(w, h);
        const QSize ratio(w / std::max(1, common), h / std::max(1, common));
        parts << (ratio.width() <= 32 && ratio.height() <= 32 ? QStringLiteral("%1:%2").arg(ratio.width()).arg(ratio.height())
                                                               : QStringLiteral("%1 × %2").arg(w).arg(h));
    }
    parts << QStringLiteral("%1 fps").arg(QString::number(m_fps, 'g', 5));
    return parts.join(QStringLiteral(" · "));
}

void MainWindow::setProjectTitle(const QString &name) {
    m_projectName = name;
    layoutProjectTitle();
    // The Window menu, Mission Control and the Dock list the project.
    setWindowTitle(name);
    if (!m_projectDirectory.isEmpty()) setWindowFilePath(QDir(m_projectDirectory).absoluteFilePath(QStringLiteral("project.kerf")));
}

// The toolbar, once the registry's actions and menus exist: the window
// buttons' room, the commands, the format bank, Export and the title.
void MainWindow::setupToolbar(const QHash<QString, QMenu *> &menus) {
    // The faceplate: the plate the toolbar's controls are set into.
    auto *toolbar = new FaceplateToolBar(QStringLiteral("Main"), this);
    addToolBar(toolbar);
    m_toolbar = toolbar;
    toolbar->setMovable(false);
    toolbar->setIconSize(QSize(kToolbarIconSize, kToolbarIconSize));
    toolbar->setFixedHeight(kToolbarHeight);
#if KERF_EDITOR_OWN_TITLE_BAR
    m_windowButtons = new QWidget;
    m_windowButtons->setObjectName(QStringLiteral("windowButtonsSpace"));
    m_windowButtons->setFixedWidth(kWindowButtonsSpace);
    toolbar->addWidget(m_windowButtons);
#endif
    // Its empty stretches move and zoom the window (on macOS, where it is
    // the title bar), and its size places the title.
    toolbar->installEventFilter(this);
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
    // Groups are set apart by space, as in macOS toolbars, not by rules, and
    // separate controls by 8 points, as macOS toolbars and the launcher's
    // footer space them; the toolbar's own spacing comes on either side.
    const auto space = [toolbar](int width) {
        auto *gap = new QWidget;
        gap->setFixedWidth(width);
        toolbar->addWidget(gap);
    };
    const auto groupSpace = [&space] { space(10); };
    const auto controlSpace = [&space] { space(8 - 2 * kToolbarSpacing); };
    toolbarCommand(QStringLiteral("file.import"), QStringLiteral("Import"), QStringLiteral("↓"));
    groupSpace();
    toolbarCommand(QStringLiteral("edit.undo"), QStringLiteral("Undo"), QStringLiteral("↶"));
    toolbarCommand(QStringLiteral("edit.redo"), QStringLiteral("Redo"), QStringLiteral("↷"));
    groupSpace();
    toolbarCommand(QStringLiteral("clip.split"), QStringLiteral("Split"), QStringLiteral("✂"));
    m_toolbarLeftEnd = toolbarCommand(QStringLiteral("clip.delete"), QStringLiteral("Delete"), QStringLiteral("trash"));
    auto *spacer = new QWidget;
    spacer->setStyleSheet(QStringLiteral("background: transparent"));
    spacer->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Preferred);
    toolbar->addWidget(spacer);
    // The project's format as a bank of latching keys, one per preset, the
    // shape of each drawn on it; the current one is held in, its LED lit.
    m_formatBank = new FormatBank;
    connect(m_formatBank, &FormatBank::formatChosen, this, &MainWindow::canvasPresetChanged);
    toolbar->addWidget(m_formatBank);
    controlSpace();
    // Export, the one key in the accent. Its chip names the quality it
    // makes; its menu picks Master (the finished file, first and by
    // default) or Draft, and holds Render Preview, which had a button of
    // its own beside it. The menu's Master names the size and rate it makes.
    m_exportButton = new ExportButton(m_commands->action(QStringLiteral("file.export")),
                                      m_commands->action(QStringLiteral("sequence.renderPreview")));
    connect(m_exportButton, &ExportButton::menuAboutToShow, this, [this] {
        const QString rate = QStringLiteral("%1 fps").arg(QString::number(m_fps, 'g', 5));
        m_exportButton->setMasterDetail(m_canvasSize.isValid()
            ? QStringLiteral("%1\u2009×\u2009%2 · %3").arg(m_canvasSize.width()).arg(m_canvasSize.height()).arg(rate)
            : rate);
    });
    toolbar->addWidget(m_exportButton);
    // Last, over the column it opens, as the comments button beside a Short
    // and the inspector switch at the end of a macOS toolbar: Kerf's
    // conversation. It ends where the banners' and the agent's bar's
    // buttons do.
    groupSpace();
    m_chatToggle = toolbarCommand(QStringLiteral("window.assistant"), QStringLiteral("Kerf Assistant"), QStringLiteral("bubble"));
    m_chatToggle->setObjectName(QStringLiteral("chatToggle"));
    space(kTrailingInset - kToolbarPadding - kToolbarSpacing);
    // The title is centred on the window, as macOS centres it, not between
    // the groups, so it sits over the toolbar outside its layout;
    // layoutProjectTitle() places it as the toolbar and its groups move.
    m_projectTitle = new QLabel(toolbar);
    m_projectTitle->setObjectName(QStringLiteral("projectTitle"));
    m_projectTitle->setAlignment(Qt::AlignCenter);
    // Under it, quietly, the project's state: saved or edited, its shape
    // and its frame rate ("Edited · 9:16 · 24 fps").
    m_projectSubtitle = new QLabel(toolbar);
    m_projectSubtitle->setObjectName(QStringLiteral("projectSubtitle"));
    m_projectSubtitle->setAlignment(Qt::AlignCenter);
    for (QWidget *edge : {static_cast<QWidget *>(m_toolbarLeftEnd), static_cast<QWidget *>(m_formatBank)})
        if (edge) edge->installEventFilter(this);
    updateCommandStates();
}

} // namespace kerf
