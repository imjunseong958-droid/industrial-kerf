// MainWindow's toolbar: the window buttons' room and the sidebar's top, the
// title and what is going on, Export and the panel's button. The region's state and its building live in
// ToolbarWiring (ui/wiring/toolbar_wiring.*); these are the window's own
// entry points into it.

#include "main_window.h"

#include "command_registry.h"
#include "components.h"
#include "editor_bus.h"
#include "toolbar_widgets.h"
#include "stage_shell.h"
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
    layoutProjectTitle();
}

void MainWindow::fitTitleBar() {
#if KERF_EDITOR_OWN_TITLE_BAR
    if (!m_toolbar || !m_toolbarWiring) return;
    // The safe area's top is the title bar the window buttons sit in. The
    // first control starts past them (52 + 16 reserved); in full screen there
    // are no buttons beside it.
    const QWindow *handle = windowHandle();
    m_toolbar->setFixedHeight(std::max(kToolbarHeight, handle ? handle->safeAreaMargins().top() : 0));
    m_toolbarWiring->setFirstControl(ToolbarWiring::firstControlX(windowButtonsWidth(this), isFullScreen()));
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
