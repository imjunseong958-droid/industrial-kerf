#include "workspace.h"
#include "editor_style.h"

#include <DockAreaWidget.h>
#include <DockAreaTitleBar.h>
#include <DockManager.h>
#include <DockSplitter.h>
#include <DockWidget.h>
#include <DockWidgetTab.h>

#include <QAbstractButton>
#include <QEvent>
#include <QSettings>
#include <QTimer>

#include <algorithm>

namespace kerf {

namespace {

// Bump when the panels or the Editing arrangement change, so stale layouts
// are not restored. 6: Kerf's conversation moved into the Program monitor's
// stage, beside the picture; the viewer takes the row past the browsers.
constexpr int kLayoutVersion = 6;
// The oldest version a workspace of one's own is still restored from: 4
// and 5 had a docked conversation as well, which restoring leaves out (an
// area left empty goes); the Editing workspace's own layout gives way to
// its new arrangement.
constexpr int kOldestCustomLayoutVersion = 4;
// The Editing arrangement's columns and rows, before the window has a size;
// the viewer takes or gives whatever the window has beyond them. The
// browsers' column is at least this wide.
constexpr int kBrowserWidth = 340;
constexpr int kArrangedWidth = 1500;
constexpr int kTopRowHeight = 500;
constexpr int kTimelineHeight = 400;
// While the picture is portrait, the top row takes this much more from the
// timeline (560 over 340 as arranged), so the phone it shows in is big
// enough to judge framing and captions; never leaving the timeline less
// than kMinTimelineHeight.
constexpr int kPortraitGain = 60;
constexpr int kMinTimelineHeight = 280;

QString layoutKey(const QString &group, const QString &name) {
    return QStringLiteral("workspace/%1/%2").arg(group, name);
}

} // namespace

Workspace::Workspace(QWidget *host) : QObject(host) {
    using Manager = ads::CDockManager;
    Manager::setConfigFlags(Manager::DefaultOpaqueConfig);
    Manager::setConfigFlag(Manager::AlwaysShowTabs, true);
    // The focused panel shows in its tab's label (see dockStyleSheet()).
    Manager::setConfigFlag(Manager::FocusHighlighting, true);
    // Tabs show a close button only under the pointer. Keeping its room on
    // every tab would elide the names of a narrow area's tabs, so the
    // hovered tab widens instead.
    Manager::setConfigFlag(Manager::ActiveTabHasCloseButton, false);
    Manager::setConfigFlag(Manager::AllTabsHaveCloseButton, false);
    Manager::setConfigFlag(Manager::DockAreaHasUndockButton, false);
    Manager::setConfigFlag(Manager::DockAreaHasCloseButton, false);
    Manager::setConfigFlag(Manager::DockAreaDynamicTabsMenuButtonVisibility, true);
    Manager::setConfigFlag(Manager::EqualSplitOnInsertion, true);
    m_manager = new Manager(host);
    m_manager->setObjectName(QStringLiteral("dockManager"));
    // The dock manager's own sheet, with the editor's colours over it; both
    // follow the appearance.
    m_baseStyleSheet = m_manager->styleSheet();
    m_manager->setStyleSheet(m_baseStyleSheet + dockStyleSheet(editorTheme()));
    connect(EditorAppearance::instance(), &EditorAppearance::changed, this,
            [this] { m_manager->setStyleSheet(m_baseStyleSheet + dockStyleSheet(editorTheme())); });
}

ads::CDockWidget *Workspace::addPanel(const QString &id, const QString &title, QWidget *content) {
    auto *dock = new ads::CDockWidget(m_manager, title);
    dock->setObjectName(id);
    dock->setWidget(content, ads::CDockWidget::ForceNoScrollArea);
    dock->setFeature(ads::CDockWidget::DockWidgetDeleteOnClose, false);
    dock->tabWidget()->installEventFilter(this);
    m_panels.insert(id, dock);
    return dock;
}

bool Workspace::eventFilter(QObject *watched, QEvent *event) {
    if (watched == m_watchedRows && event->type() == QEvent::Resize && m_shapePending)
        QTimer::singleShot(0, this, [this] {
            if (m_shapePending) fitPictureShape(m_portrait);
        });
    auto *tab = qobject_cast<ads::CDockWidgetTab *>(watched);
    switch (tab ? event->type() : QEvent::None) {
    case QEvent::Enter:
    case QEvent::Leave:
        showCloseButton(tab, event->type() == QEvent::Enter);
        break;
    case QEvent::MouseButtonRelease:
        // A click makes the tab current, and the dock manager hides the
        // button again; show it once that is done.
        QTimer::singleShot(0, tab, [this, tab] { showCloseButton(tab, tab->underMouse()); });
        break;
    default:
        break;
    }
    return QObject::eventFilter(watched, event);
}

void Workspace::showCloseButton(ads::CDockWidgetTab *tab, bool shown) {
    auto *close = tab->findChild<QAbstractButton *>(QStringLiteral("tabCloseButton"), Qt::FindDirectChildrenOnly);
    const ads::CDockWidget *dock = tab->dockWidget();
    if (close && dock) close->setVisible(shown && dock->features().testFlag(ads::CDockWidget::DockWidgetClosable));
}

void Workspace::arrangeEditing() {
    // As YouTube shows a Short on the desktop: the Program monitor is the
    // page, as large as its row allows, and Kerf's conversation lives in its
    // stage beside the picture (see ViewerWidget::stageLayout), not in a
    // dock of its own. The Source monitor is a tab behind the Program
    // monitor, not a second viewer. Left of it are the browsers (Project,
    // Effects, Presets) and Effect Controls, where Premiere keeps them. The
    // Timeline runs the window's full width under both, for its many tracks.
    const auto panel = [this](const char *id) { return m_panels.value(QString::fromLatin1(id)); };
    auto *viewer = m_manager->addDockWidget(ads::LeftDockWidgetArea, panel("program"));
    m_manager->addDockWidget(ads::CenterDockWidgetArea, panel("source"), viewer);
    viewer->setCurrentDockWidget(panel("program"));
    auto *browser = m_manager->addDockWidget(ads::LeftDockWidgetArea, panel("project"), viewer);
    for (const char *id : {"effectControls", "effects", "presets"})
        m_manager->addDockWidget(ads::CenterDockWidgetArea, panel(id), browser);
    browser->setCurrentDockWidget(panel("project"));
    // Wide enough for the bin's two columns of tiles and for its four tabs'
    // names, unshortened.
    browser->titleBar()->ensurePolished();
    const int browserWidth = std::max(kBrowserWidth, browser->titleBar()->sizeHint().width());
    auto *timeline = m_manager->addDockWidget(ads::BottomDockWidgetArea, panel("timeline"));
    letViewerStretch();
    // Lay the rows and columns out at a window's size now, before there is
    // a window, so the layout saved from them keeps these sizes; the viewer
    // takes the rest of whatever width the window has.
    ads::CDockSplitter *rows = timeline->parentSplitter();
    rows->setGeometry(QRect(0, 0, kArrangedWidth, kTopRowHeight + rows->handleWidth() + kTimelineHeight));
    m_manager->setSplitterSizes(timeline, {kTopRowHeight, kTimelineHeight});
    const int handles = viewer->parentSplitter()->handleWidth();
    m_manager->setSplitterSizes(viewer, {browserWidth, kArrangedWidth - handles - browserWidth});
}

ads::CDockSplitter *Workspace::rowsSplitter() const {
    // The Editing arrangement's rows: the browsers and viewer over the timeline.
    ads::CDockWidget *timeline = m_panels.value(QStringLiteral("timeline"));
    ads::CDockAreaWidget *area = timeline && !timeline->isClosed() ? timeline->dockAreaWidget() : nullptr;
    ads::CDockSplitter *rows = area ? area->parentSplitter() : nullptr;
    if (!rows || rows->orientation() != Qt::Vertical || rows->count() != 2 || rows->widget(1) != area) return nullptr;
    return rows;
}

void Workspace::shiftRows(int delta) {
    ads::CDockSplitter *rows = rowsSplitter();
    if (!rows || delta == 0) return;
    QList<int> sizes = rows->sizes();
    sizes[0] += delta;
    sizes[1] -= delta;
    rows->setSizes(sizes);
}

void Workspace::fitPictureShape(bool portrait) {
    m_portrait = portrait;
    ads::CDockSplitter *rows = rowsSplitter();
    if (!rows) {
        m_shapeGain = 0;
        m_shapePending = false;
        return;
    }
    const QList<int> sizes = rows->sizes();
    if (!rows->isVisible() || sizes.at(0) + sizes.at(1) <= 0) {
        // Not laid out yet (a window not shown): once it is.
        if (m_watchedRows != rows) {
            rows->installEventFilter(this);
            m_watchedRows = rows;
        }
        m_shapePending = true;
        return;
    }
    m_shapePending = false;
    const int gain = portrait ? std::clamp(sizes.at(1) + m_shapeGain - kMinTimelineHeight, 0, kPortraitGain) : 0;
    if (gain == m_shapeGain) return;
    shiftRows(gain - m_shapeGain);
    m_shapeGain = gain;
}

QByteArray Workspace::saveUnshifted() {
    // Layouts are kept as arranged for any picture: the portrait gain is
    // taken back while saving, and given again.
    shiftRows(-m_shapeGain);
    const QByteArray state = m_manager->saveState(kLayoutVersion);
    shiftRows(m_shapeGain);
    return state;
}

void Workspace::afterRestore() {
    letViewerStretch();
    m_shapeGain = 0;
    fitPictureShape(m_portrait);
}

void Workspace::letViewerStretch() {
    ads::CDockWidget *program = m_panels.value(QStringLiteral("program"));
    ads::CDockAreaWidget *area = program ? program->dockAreaWidget() : nullptr;
    if (!area) return;
    QSizePolicy policy = area->sizePolicy();
    policy.setHorizontalStretch(1);
    area->setSizePolicy(policy);
}

void Workspace::start() {
    arrangeEditing();
    m_editing = m_manager->saveState(kLayoutVersion);
    const QSettings settings;
    m_current = settings.value(QStringLiteral("workspace/current"), QString::fromLatin1(kEditing)).toString();
    if (!names().contains(m_current)) m_current = QString::fromLatin1(kEditing);
    const QByteArray changes = settings.value(layoutKey(QStringLiteral("changes"), m_current)).toByteArray();
    // A layout that no longer restores (the Editing workspace's from an
    // older version) leaves the Editing arrangement just made.
    if (!restore(changes, m_current)) restore(savedLayout(m_current), m_current);
}

QStringList Workspace::names() const {
    QStringList names{QString::fromLatin1(kEditing)};
    for (const QString &name : QSettings().value(QStringLiteral("workspace/names")).toStringList())
        if (!names.contains(name)) names.append(name);
    return names;
}

QByteArray Workspace::savedLayout(const QString &name) const {
    const QByteArray saved = QSettings().value(layoutKey(QStringLiteral("saved"), name)).toByteArray();
    return saved.isEmpty() ? m_editing : saved;
}

bool Workspace::restore(const QByteArray &layout, const QString &name) {
    if (layout.isEmpty()) return false;
    const bool own = name != QLatin1String(kEditing);
    bool restored = false;
    for (int version = kLayoutVersion; !restored && version >= (own ? kOldestCustomLayoutVersion : kLayoutVersion); --version)
        restored = m_manager->restoreState(layout, version);
    if (restored) afterRestore();
    return restored;
}

void Workspace::activate(const QString &name) {
    if (name == m_current || !names().contains(name)) return;
    persist();
    m_current = name;
    const QByteArray changes = QSettings().value(layoutKey(QStringLiteral("changes"), name)).toByteArray();
    // One from before today's panels gives way to the Editing arrangement.
    if (!restore(changes, name) && !restore(savedLayout(name), name)) restore(m_editing, QString::fromLatin1(kEditing));
    persist();
    emit changed();
}

void Workspace::saveCurrent() {
    QSettings settings;
    settings.setValue(layoutKey(QStringLiteral("saved"), m_current), saveUnshifted());
    settings.remove(layoutKey(QStringLiteral("changes"), m_current));
}

bool Workspace::saveAs(const QString &name) {
    const QString trimmed = name.trimmed();
    if (trimmed.isEmpty() || names().contains(trimmed, Qt::CaseInsensitive)) return false;
    QSettings settings;
    QStringList custom = settings.value(QStringLiteral("workspace/names")).toStringList();
    custom.append(trimmed);
    settings.setValue(QStringLiteral("workspace/names"), custom);
    m_current = trimmed;
    saveCurrent();
    persist();
    emit changed();
    return true;
}

void Workspace::resetCurrent() {
    if (!restore(savedLayout(m_current), m_current)) restore(m_editing, QString::fromLatin1(kEditing));
    QSettings().remove(layoutKey(QStringLiteral("changes"), m_current));
}

void Workspace::persist() {
    QSettings settings;
    settings.setValue(QStringLiteral("workspace/current"), m_current);
    settings.setValue(layoutKey(QStringLiteral("changes"), m_current), saveUnshifted());
}

} // namespace kerf
