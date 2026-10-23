#include "workspace.h"
#include "editor_style.h"

#include <DockAreaWidget.h>
#include <DockManager.h>
#include <DockWidget.h>

#include <QSettings>

namespace kerf {

namespace {

// Bump when the panels change so stale layouts are not restored.
constexpr int kLayoutVersion = 3;

QString layoutKey(const QString &group, const QString &name) {
    return QStringLiteral("workspace/%1/%2").arg(group, name);
}

} // namespace

Workspace::Workspace(QWidget *host) : QObject(host) {
    using Manager = ads::CDockManager;
    Manager::setConfigFlags(Manager::DefaultOpaqueConfig);
    Manager::setConfigFlag(Manager::AlwaysShowTabs, true);
    Manager::setConfigFlag(Manager::FocusHighlighting, true);
    Manager::setConfigFlag(Manager::DockAreaHasUndockButton, false);
    Manager::setConfigFlag(Manager::DockAreaHasCloseButton, false);
    Manager::setConfigFlag(Manager::DockAreaDynamicTabsMenuButtonVisibility, true);
    Manager::setConfigFlag(Manager::EqualSplitOnInsertion, true);
    m_manager = new Manager(host);
    m_manager->setObjectName(QStringLiteral("dockManager"));
    m_manager->setStyleSheet(m_manager->styleSheet() + dockStyleSheet());
}

ads::CDockWidget *Workspace::addPanel(const QString &id, const QString &title, QWidget *content) {
    auto *dock = new ads::CDockWidget(m_manager, title);
    dock->setObjectName(id);
    dock->setWidget(content, ads::CDockWidget::ForceNoScrollArea);
    dock->setFeature(ads::CDockWidget::DockWidgetDeleteOnClose, false);
    m_panels.insert(id, dock);
    return dock;
}

void Workspace::arrangeEditing() {
    // The edit-suite arrangement: Source (with Effect Controls behind it)
    // beside the Program monitor, over the Project bin and the Timeline.
    const auto panel = [this](const char *id) { return m_panels.value(QString::fromLatin1(id)); };
    auto *source = m_manager->addDockWidget(ads::LeftDockWidgetArea, panel("source"));
    m_manager->addDockWidget(ads::CenterDockWidgetArea, panel("effectControls"), source);
    m_manager->addDockWidget(ads::CenterDockWidgetArea, panel("assistant"), source);
    source->setCurrentDockWidget(panel("source"));
    m_manager->addDockWidget(ads::RightDockWidgetArea, panel("program"), source);
    auto *project = m_manager->addDockWidget(ads::BottomDockWidgetArea, panel("project"));
    m_manager->addDockWidget(ads::RightDockWidgetArea, panel("timeline"), project);
    m_manager->setSplitterSizes(source, {640, 860});
    m_manager->setSplitterSizes(project, {320, 1180});
    // Kerf is asked from the timeline (Ask Kerf…); its conversation is a panel
    // to open from Window or the timeline's AI bar.
    panel("assistant")->toggleView(false);
}

void Workspace::start() {
    arrangeEditing();
    m_editing = m_manager->saveState(kLayoutVersion);
    const QSettings settings;
    m_current = settings.value(QStringLiteral("workspace/current"), QString::fromLatin1(kEditing)).toString();
    if (!names().contains(m_current)) m_current = QString::fromLatin1(kEditing);
    const QByteArray changes = settings.value(layoutKey(QStringLiteral("changes"), m_current)).toByteArray();
    if (!restore(changes)) restore(savedLayout(m_current));
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

bool Workspace::restore(const QByteArray &layout) {
    return !layout.isEmpty() && m_manager->restoreState(layout, kLayoutVersion);
}

void Workspace::activate(const QString &name) {
    if (name == m_current || !names().contains(name)) return;
    persist();
    m_current = name;
    const QByteArray changes = QSettings().value(layoutKey(QStringLiteral("changes"), name)).toByteArray();
    if (!restore(changes)) restore(savedLayout(name));
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
    restore(savedLayout(m_current));
    QSettings().remove(layoutKey(QStringLiteral("changes"), m_current));
}

void Workspace::persist() const {
    QSettings settings;
    settings.setValue(QStringLiteral("workspace/current"), m_current);
    settings.setValue(layoutKey(QStringLiteral("changes"), m_current), m_manager->saveState(kLayoutVersion));
}

} // namespace kerf
