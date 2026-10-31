#include "workspace.h"
#include "editor_style.h"

#include <DockAreaWidget.h>
#include <DockAreaTabBar.h>
#include <DockAreaTitleBar.h>
#include <DockManager.h>
#include <DockSplitter.h>
#include <DockWidget.h>
#include <DockWidgetTab.h>

#include <QAbstractButton>
#include <QApplication>
#include <QEvent>
#include <QLayout>
#include <QSettings>
#include <QToolButton>
#include <QTimer>

#include <algorithm>

namespace kerf {

namespace {

// Bump when the panels or the Editing arrangement change, so stale layouts
// are not restored. 6: Kerf's conversation moved into the Program monitor's
// stage, beside the picture; the viewer takes the row past the browsers.
// 7: Effect Controls became the Inspector (its id `effectControls` became
// `inspector`, renamed in older layouts as they are restored). 8: the
// browsers' column narrowed to 300 and the stage row grew to 575 over a
// 290 timeline, for every picture shape.
constexpr int kLayoutVersion = 8;
// The oldest version the Editing workspace's own changes are restored
// from: older ones give way to the new proportions.
constexpr int kOldestEditingLayoutVersion = 8;
// The oldest version a workspace of one's own is still restored from: 4
// and 5 had a docked conversation as well, which restoring leaves out (an
// area left empty goes); the Editing workspace's own layout gives way to
// its new arrangement.
constexpr int kOldestCustomLayoutVersion = 4;
// The Editing arrangement's columns and rows, before the window has a size;
// the viewer takes or gives whatever the window has beyond them. As the
// structure the owner chose: a column of about 300 for the bin's two
// columns of tiles, and the stage row taking the height (575) over a
// timeline of 290, which holds eight tracks at their default heights; the
// same for a phone, a laptop or a plain card, so the timeline never jumps
// when the format changes.
constexpr int kBrowserWidth = 300;
constexpr int kArrangedWidth = 1440;
constexpr int kTopRowHeight = 575;
constexpr int kTimelineHeight = 290;

QString layoutKey(const QString &group, const QString &name) {
    return QStringLiteral("workspace/%1/%2").arg(group, name);
}

// A saved layout with Effect Controls' old id given the Inspector's.
QByteArray migrated(const QByteArray &layout) {
    QByteArray xml = layout.startsWith("<?xml") ? layout : qUncompress(layout);
    if (xml.isEmpty()) return layout;
    xml.replace("Name=\"effectControls\"", "Name=\"inspector\"");
    return xml;
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
    // A text tab: its name and nothing round it (the dock manager's own
    // room before, between and after the name and the close button goes;
    // the sheet spaces the tabs), underlined when chosen.
    ads::CDockWidgetTab *tab = dock->tabWidget();
    tab->setProperty("textTab", true);
    if (QLayout *layout = tab->layout()) {
        layout->setContentsMargins(0, 0, 0, 0);
        for (int index = 0; index < layout->count(); ++index)
            if (QSpacerItem *spacer = layout->itemAt(index)->spacerItem()) spacer->changeSize(0, 0);
        layout->invalidate();
    }
    m_panels.insert(id, dock);
    // Brought forward by hand (its tab, ⌘4): it stays when the selection clears.
    if (id == QLatin1String("inspector"))
        connect(dock, &ads::CDockWidget::visibilityChanged, this, [this](bool visible) {
            if (visible && !m_raisingInspector) m_inspectorAuto = false;
        });
    // A browser brought forward by hand is in use: a click on a clip leaves
    // it in front. One the Inspector's Add effect brought up is not.
    if (id == QLatin1String("effects") || id == QLatin1String("presets"))
        connect(dock, &ads::CDockWidget::visibilityChanged, this, [this, dock](bool visible) {
            if (visible) m_browserForInspector = m_raisingBrowser ? dock : nullptr;
        });
    return dock;
}

void Workspace::setTitleWidgets(const QString &id, QWidget *leading, QWidget *trailing) {
    m_titleWidgets.insert(id, TitleWidgets{leading, trailing});
    for (QWidget *widget : {leading, trailing})
        if (widget) {
            widget->setParent(m_manager);
            widget->hide();
        }
    if (ads::CDockWidget *dock = m_panels.value(id))
        connect(dock, &ads::CDockWidget::visibilityChanged, this, [this] { QTimer::singleShot(0, this, &Workspace::placeTitleWidgets); });
    placeTitleWidgets();
}

void Workspace::parkTitleWidgets() {
    for (const TitleWidgets &widgets : std::as_const(m_titleWidgets))
        for (QWidget *widget : {widgets.leading.data(), widgets.trailing.data()})
            if (widget) {
                widget->hide();
                widget->setParent(m_manager);
            }
}

void Workspace::placeTitleWidgets() {
    for (auto it = m_titleWidgets.cbegin(); it != m_titleWidgets.cend(); ++it) {
        ads::CDockWidget *dock = m_panels.value(it.key());
        ads::CDockAreaWidget *area = dock && !dock->isClosed() ? dock->dockAreaWidget() : nullptr;
        ads::CDockAreaTitleBar *bar = area ? area->titleBar() : nullptr;
        QWidget *leading = it->leading;
        QWidget *trailing = it->trailing;
        if (!bar) {
            for (QWidget *widget : {leading, trailing})
                if (widget) {
                    widget->hide();
                    widget->setParent(m_manager);
                }
            continue;
        }
        connect(area, &ads::CDockAreaWidget::currentChanged, this, &Workspace::placeTitleWidgets, Qt::UniqueConnection);
        if (leading && leading->parentWidget() != bar) bar->insertWidget(bar->indexOf(bar->tabBar()) + 1, leading);
        if (trailing && trailing->parentWidget() != bar) {
            // Before the panel's own buttons, or the tabs' menu.
            int index = bar->indexOf(bar->button(ads::TitleBarButtonTabsMenu));
            const QList<QAction *> actions = dock->titleBarActions();
            for (QToolButton *button : bar->findChildren<QToolButton *>(QString(), Qt::FindDirectChildrenOnly))
                if (actions.contains(button->defaultAction()) && bar->indexOf(button) >= 0) index = std::min(index, bar->indexOf(button));
            bar->insertWidget(std::max(0, index), trailing);
        }
        const bool current = area->currentDockWidget() == dock;
        for (QWidget *widget : {leading, trailing})
            if (widget) widget->setVisible(current);
    }
}

bool Workspace::eventFilter(QObject *watched, QEvent *event) {
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
    // monitor, not a second viewer. Left of it are Project, the Inspector
    // (which comes forward when a clip is clicked), Effects and Presets. The
    // Timeline runs the window's full width under both, for its many tracks.
    const auto panel = [this](const char *id) { return m_panels.value(QString::fromLatin1(id)); };
    auto *viewer = m_manager->addDockWidget(ads::LeftDockWidgetArea, panel("program"));
    m_manager->addDockWidget(ads::CenterDockWidgetArea, panel("source"), viewer);
    viewer->setCurrentDockWidget(panel("program"));
    auto *browser = m_manager->addDockWidget(ads::LeftDockWidgetArea, panel("project"), viewer);
    for (const char *id : {"inspector", "effects", "presets"})
        m_manager->addDockWidget(ads::CenterDockWidgetArea, panel(id), browser);
    browser->setCurrentDockWidget(panel("project"));
    // Wide enough for the bin's two columns of tiles and for its four tabs'
    // names, unshortened.
    browser->titleBar()->ensurePolished();
    const int browserWidth = std::max(kBrowserWidth, browser->titleBar()->tabBar()->sizeHint().width());
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
    placeTitleWidgets();
}

void Workspace::afterRestore() {
    letViewerStretch();
    placeTitleWidgets();
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
    const QByteArray state = migrated(layout);
    parkTitleWidgets();
    for (int version = kLayoutVersion; !restored && version >= (own ? kOldestCustomLayoutVersion : kOldestEditingLayoutVersion); --version)
        restored = m_manager->restoreState(state, version);
    if (restored) afterRestore();
    else placeTitleWidgets();
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
    settings.setValue(layoutKey(QStringLiteral("saved"), m_current), m_manager->saveState(kLayoutVersion));
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
    settings.setValue(layoutKey(QStringLiteral("changes"), m_current), m_manager->saveState(kLayoutVersion));
}

bool Workspace::inspectorFollowsSelection() {
    return QSettings().value(QStringLiteral("editor/inspectorFollowsSelection"), true).toBool();
}

void Workspace::setInspectorFollowsSelection(bool follows) {
    QSettings().setValue(QStringLiteral("editor/inspectorFollowsSelection"), follows);
}

void Workspace::raiseInspector(InspectorOrigin origin, bool force, const QString &clip) {
    ads::CDockWidget *inspector = m_panels.value(QStringLiteral("inspector"));
    if (!inspector || inspector->isClosed() || inspector->isFloating()) return;
    if (origin == InspectorOrigin::Edit && !force) return;
    if (!force && !inspectorFollowsSelection()) return;
    ads::CDockAreaWidget *area = inspector->dockAreaWidget();
    if (!area) return;
    if (area->currentDockWidget() == inspector) {
        m_inspectorClip = clip;
        return;
    }
    // Effects or Presets chosen by hand: the person is applying looks clip
    // by clip, so the browser stays under the pointer (it follows the
    // selection itself); the Inspector comes forward only from Project.
    ads::CDockWidget *current = area->currentDockWidget();
    const bool browsing = current && (current->objectName() == QLatin1String("effects") || current->objectName() == QLatin1String("presets"));
    if (!force && browsing && current != m_browserForInspector) {
        m_inspectorClip = clip;
        return;
    }
    // Chose another tab with this clip still selected: only a new click raises.
    if (!force && !clip.isEmpty() && clip == m_inspectorClip) return;
    m_inspectorClip = clip;
    m_browserForInspector = nullptr;
    m_beforeInspector = area->currentDockWidget();
    QWidget *focus = QApplication::focusWidget();
    m_raisingInspector = true;
    inspector->setAsCurrentTab();
    m_raisingInspector = false;
    m_inspectorAuto = true;
    // Raising is visual only: the keys stay where they were.
    if (focus && QApplication::focusWidget() != focus) focus->setFocus(Qt::OtherFocusReason);
}

void Workspace::showBrowserForInspector(const QString &id) {
    ads::CDockWidget *browser = m_panels.value(id);
    if (!browser) return;
    m_raisingBrowser = true;
    if (browser->isClosed()) browser->toggleView(true);
    browser->setAsCurrentTab();
    m_raisingBrowser = false;
    m_browserForInspector = browser;
}

void Workspace::inspectorDeselected() {
    m_inspectorClip.clear();
    ads::CDockWidget *inspector = m_panels.value(QStringLiteral("inspector"));
    if (!inspector || !m_inspectorAuto || inspector->isClosed()) return;
    m_inspectorAuto = false;
    ads::CDockAreaWidget *area = inspector->dockAreaWidget();
    if (!area || area->currentDockWidget() != inspector) return;
    if (m_beforeInspector && !m_beforeInspector->isClosed() && m_beforeInspector->dockAreaWidget() == area) {
        m_raisingInspector = true;
        m_beforeInspector->setAsCurrentTab();
        m_raisingInspector = false;
    }
}

void Workspace::toggleInspector() {
    ads::CDockWidget *inspector = m_panels.value(QStringLiteral("inspector"));
    if (!inspector) return;
    const bool wasClosed = inspector->isClosed();
    if (wasClosed) inspector->toggleView(true);
    ads::CDockAreaWidget *area = inspector->dockAreaWidget();
    m_inspectorAuto = false;
    if (!wasClosed && area && area->currentDockWidget() == inspector && m_beforeInspector && !m_beforeInspector->isClosed() &&
        m_beforeInspector->dockAreaWidget() == area) {
        m_raisingInspector = true;
        m_beforeInspector->setAsCurrentTab();
        m_raisingInspector = false;
        return;
    }
    if (area && area->currentDockWidget() != inspector) m_beforeInspector = area->currentDockWidget();
    m_raisingInspector = true;
    inspector->setAsCurrentTab();
    m_raisingInspector = false;
}

} // namespace kerf
