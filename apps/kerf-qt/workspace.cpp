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
// are not restored. 5: the Program monitor centred, Kerf's conversation on
// its right, the Timeline the window's full width.
constexpr int kLayoutVersion = 5;
// The oldest version with today's panels. A workspace of one's own saved
// from it is still restored; the Editing workspace's own layout gives way
// to its new arrangement.
constexpr int kOldestCustomLayoutVersion = 4;
// The Editing arrangement's columns and rows, before the window has a size;
// the viewer takes or gives whatever the window has beyond them. The
// browsers' column is at least this wide.
constexpr int kBrowserWidth = 340;
constexpr int kArrangedWidth = 1500;
constexpr int kTopRowHeight = 500;
constexpr int kTimelineHeight = 400;

QString layoutKey(const QString &group, const QString &name) {
    return QStringLiteral("workspace/%1/%2").arg(group, name);
}

// The width a closed conversation opens at; this Mac's, never the project's.
constexpr const char *kAssistantWidthKey = "workspace/assistantWidth";

// The area holding the conversation when it is alone in a column of a row:
// the only arrangement whose width it keeps.
ads::CDockAreaWidget *assistantColumn(ads::CDockWidget *assistant, int openPanels) {
    ads::CDockAreaWidget *area = assistant ? assistant->dockAreaWidget() : nullptr;
    if (!area || area->openDockWidgetsCount() != openPanels) return nullptr;
    const ads::CDockSplitter *row = area->parentSplitter();
    return row && row->orientation() == Qt::Horizontal ? area : nullptr;
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
    if (id == QLatin1String(kAssistant)) {
        // However it opens or closes (the toolbar, Window, its header, its
        // tab), the conversation keeps its width and the viewer the rest.
        connect(dock, &ads::CDockWidget::viewToggled, this, [this](bool open) {
            if (m_arranging) return;
            if (!open) {
                rememberAssistantWidth();
                return;
            }
            // Once the row has laid the column out again.
            QTimer::singleShot(0, this, [this] { restoreAssistantWidth(); });
        });
    }
    return dock;
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
    // The conversation's header has a close button of its own, always shown.
    const bool headerCloses = dock && dock->objectName() == QLatin1String(kAssistant) && !dock->titleBarActions().isEmpty();
    if (close && dock)
        close->setVisible(shown && !headerCloses && dock->features().testFlag(ads::CDockWidget::DockWidgetClosable));
}

void Workspace::arrangeEditing() {
    // As YouTube and TikTok show a video on the desktop: the Program monitor
    // in the middle and as large as its row allows, and Kerf's conversation
    // in a column of its own on its right, as a Short opens its comments
    // beside it. The Source monitor is a tab behind the Program monitor, not
    // a second viewer. Left of them are the browsers (Project, Effects,
    // Presets) and Effect Controls, left of the Program monitor where it
    // was and where Premiere keeps it, so the conversation's column holds
    // only the conversation and closing it widens the viewer. The Timeline
    // runs the window's full width under all three, for its many tracks.
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
    m_manager->addDockWidget(ads::RightDockWidgetArea, panel(kAssistant), viewer);
    auto *timeline = m_manager->addDockWidget(ads::BottomDockWidgetArea, panel("timeline"));
    letViewerStretch();
    // Lay the rows and columns out at a window's size now, before there is
    // a window, so the layout saved from them keeps these sizes; the viewer
    // takes the rest of whatever width the window has.
    ads::CDockSplitter *rows = timeline->parentSplitter();
    rows->setGeometry(QRect(0, 0, kArrangedWidth, kTopRowHeight + rows->handleWidth() + kTimelineHeight));
    m_manager->setSplitterSizes(timeline, {kTopRowHeight, kTimelineHeight});
    const int handles = 2 * viewer->parentSplitter()->handleWidth();
    m_manager->setSplitterSizes(viewer, {browserWidth, kArrangedWidth - handles - browserWidth - m_assistantWidth, m_assistantWidth});
}

void Workspace::letViewerStretch() {
    ads::CDockWidget *program = m_panels.value(QStringLiteral("program"));
    ads::CDockAreaWidget *area = program ? program->dockAreaWidget() : nullptr;
    if (!area) return;
    QSizePolicy policy = area->sizePolicy();
    policy.setHorizontalStretch(1);
    area->setSizePolicy(policy);
}

bool Workspace::assistantOpen() const {
    const ads::CDockWidget *assistant = m_panels.value(QLatin1String(kAssistant));
    return assistant && !assistant->isClosed();
}

void Workspace::setAssistantOpen(bool open) {
    ads::CDockWidget *assistant = m_panels.value(QLatin1String(kAssistant));
    if (!assistant) return;
    if (open == assistant->isClosed()) assistant->toggleView(open);
    if (open) assistant->setAsCurrentTab();
}

void Workspace::rememberAssistantWidth() {
    // It has just closed: its area keeps the geometry it was last laid out
    // in until the row lays out again.
    if (ads::CDockAreaWidget *area = assistantColumn(m_panels.value(QLatin1String(kAssistant)), 0))
        if (area->width() >= area->minimumSizeHint().width()) m_assistantWidth = area->width();
    QSettings().setValue(QLatin1String(kAssistantWidthKey), m_assistantWidth);
}

void Workspace::restoreAssistantWidth() {
    ads::CDockWidget *assistant = m_panels.value(QLatin1String(kAssistant));
    ads::CDockAreaWidget *area = assistantColumn(assistant, 1);
    if (!area || assistant->isClosed()) return;
    ads::CDockSplitter *row = area->parentSplitter();
    const int column = row->indexOf(area);
    // The room comes from the viewer's column, or the widest one beside it.
    const QWidget *viewer = m_panels.value(QStringLiteral("program"));
    QList<int> sizes = row->sizes();
    int donor = -1;
    for (int i = 0; i < row->count(); ++i) {
        if (i == column || row->widget(i)->isHidden()) continue;
        if (viewer && (row->widget(i) == viewer || row->widget(i)->isAncestorOf(viewer))) {
            donor = i;
            break;
        }
        if (donor < 0 || sizes.at(i) > sizes.at(donor)) donor = i;
    }
    if (donor < 0) return;
    const int room = sizes.at(column) + sizes.at(donor);
    const int least = area->minimumSizeHint().width();
    const int most = std::max(least, room - row->widget(donor)->minimumSizeHint().width());
    const int width = std::clamp(m_assistantWidth, least, most);
    if (width == sizes.at(column)) return;
    sizes[donor] = room - width;
    sizes[column] = width;
    row->setSizes(sizes);
}

void Workspace::start() {
    const int width = QSettings().value(QLatin1String(kAssistantWidthKey)).toInt();
    m_assistantWidth = width > 0 ? width : kAssistantWidth;
    m_arranging = true;
    arrangeEditing();
    m_editing = m_manager->saveState(kLayoutVersion);
    const QSettings settings;
    m_current = settings.value(QStringLiteral("workspace/current"), QString::fromLatin1(kEditing)).toString();
    if (!names().contains(m_current)) m_current = QString::fromLatin1(kEditing);
    const QByteArray changes = settings.value(layoutKey(QStringLiteral("changes"), m_current)).toByteArray();
    // A layout that no longer restores (the Editing workspace's from an
    // older version) leaves the Editing arrangement just made.
    if (!restore(changes, m_current)) restore(savedLayout(m_current), m_current);
    m_arranging = false;
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
    const bool wasArranging = m_arranging;
    m_arranging = true;
    bool restored = false;
    for (int version = kLayoutVersion; !restored && version >= (own ? kOldestCustomLayoutVersion : kLayoutVersion); --version)
        restored = m_manager->restoreState(layout, version);
    m_arranging = wasArranging;
    if (restored) letViewerStretch();
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
    // The width a closed conversation opens at next time, as it is now if open.
    if (const ads::CDockAreaWidget *area = assistantColumn(m_panels.value(QLatin1String(kAssistant)), 1);
        area && area->isVisible() && area->width() >= area->minimumSizeHint().width())
        m_assistantWidth = area->width();
    settings.setValue(QLatin1String(kAssistantWidthKey), m_assistantWidth);
}

} // namespace kerf
