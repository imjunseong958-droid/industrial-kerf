// MainWindow's toolbar (mockup-spec 1): the window buttons' room, the
// commands, the workspaces, the centred title, the format, the sequence,
// Export and Settings. The region's state and its building live in
// ToolbarWiring (ui/wiring/toolbar_wiring.*); these are the window's own
// entry points into it.

#include "main_window.h"

#include "command_registry.h"
#include "components.h"
#include "editor_bus.h"
#include "toolbar_widgets.h"
#include "workspace.h"
#include "wiring/main_window_shared.h"
#include "wiring/toolbar_wiring.h"
#ifdef Q_OS_MACOS
#include "macos_window.h"
#endif

#include <QActionGroup>
#include <QDir>
#include <QJsonObject>
#include <QMenu>
#include <QWindow>

#include <algorithm>

namespace kerf {

using namespace window_parts;

void MainWindow::canvasPresetChanged(const QSize &size) {
    if (m_updatingCanvasPreset) return;
    // The segment is on already; anything that cannot become the project's
    // format lets it go back to the format the project has.
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
    // The preset's segment, or none for a custom size; it asks for nothing.
    m_updatingCanvasPreset = true;
    m_formatBank->setCanvasSize(size);
    m_updatingCanvasPreset = false;
    m_toolbarWiring->updateSequence();
    layoutProjectTitle();
}

void MainWindow::buildWorkspaceMenu(QMenu *menu) {
    // As mockup-spec 10 lists it: the built-ins with their keys (the
    // registry's actions, the current one checked), one's own (saved),
    // then saving and resetting the current one.
    menu->clear();
    qDeleteAll(menu->findChildren<QActionGroup *>(Qt::FindDirectChildrenOnly));
    QAction *header = ui::addMenuHeader(menu, QStringLiteral("Workspaces"));
    header->setObjectName(QStringLiteral("workspacesHeader"));
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
        ui::setMenuKey(choice, QStringLiteral("saved"), true);
        group->addAction(choice);
        connect(choice, &QAction::triggered, this, [this, name] { m_workspace->activate(name); });
    }
    menu->addSeparator();
    for (const char *id : {"window.saveWorkspace", "window.saveWorkspaceAs", "window.resetWorkspace"})
        if (QAction *action = m_commands->action(QString::fromLatin1(id))) menu->addAction(action);
}

void MainWindow::fitTitleBar() {
#if KERF_EDITOR_OWN_TITLE_BAR
    if (!m_toolbar || !m_windowButtons) return;
    // The safe area's top is the title bar the window buttons sit in. The
    // commands start past them (52 + 16 reserved, mockup-spec 1); in full
    // screen there are no buttons beside them.
    const QWindow *handle = windowHandle();
    m_toolbar->setFixedHeight(std::max(kToolbarHeight, handle ? handle->safeAreaMargins().top() : 0));
    const int first = ToolbarWiring::firstControlX(windowButtonsWidth(this), isFullScreen());
    // The row's 16 pt padding and its 4 pt gap come before the first control.
    m_windowButtons->setFixedWidth(std::max(0, first - 16 - 4));
    m_windowButtons->setVisible(!isFullScreen());
#endif
}

void MainWindow::layoutProjectTitle() {
    if (m_toolbarWiring) m_toolbarWiring->layoutTitle();
}

QString MainWindow::projectSubtitle() const { return m_toolbarWiring ? m_toolbarWiring->subtitleText() : QString(); }

void MainWindow::setProjectTitle(const QString &name) {
    m_projectName = name;
    layoutProjectTitle();
    // The Window menu, Mission Control and the Dock list the project and
    // its sequence, as the toolbar does.
    setWindowTitle(m_toolbarWiring ? m_toolbarWiring->titleText() : name);
    if (!m_projectDirectory.isEmpty()) setWindowFilePath(QDir(m_projectDirectory).absoluteFilePath(QStringLiteral("project.kerf")));
}

// The toolbar, once the registry's actions and menus exist.
void MainWindow::setupToolbar(const QHash<QString, QMenu *> &menus) {
    m_toolbarWiring->buildToolbar(menus);
    layoutProjectTitle();
    updateCommandStates();
}

} // namespace kerf
