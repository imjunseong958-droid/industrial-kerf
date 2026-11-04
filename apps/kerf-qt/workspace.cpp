#include "workspace.h"
#include "browser_panel.h"
#include "design_tokens.h"
#include "editor_style.h"
#include "inspector_column.h"

#include <DockAreaWidget.h>
#include <DockComponentsFactory.h>
#include <DockAreaTabBar.h>
#include <DockAreaTitleBar.h>
#include <DockManager.h>
#include <DockSplitter.h>
#include <DockWidget.h>
#include <DockWidgetTab.h>

#include <QApplication>
#include <QLayout>
#include <QSet>
#include <QSettings>
#include <QToolButton>
#include <QTimer>
#include <QVariantMap>

#include <algorithm>

namespace kerf {

namespace {

// Bump when the panels or the built-in arrangements change, so stale
// layouts are not restored. 6: Kerf's conversation moved into the Program
// monitor's stage. 7: Effect Controls became the Inspector. 8: the browsers'
// column narrowed to 300 over a 290 timeline. 9: the review design's tree
// (Final Cut Pro's): Program and the Inspector column over a band of the
// bin, the timeline and the browser (Effects and Presets became its pages,
// Source a page of the Inspector column), with Finish and Deliver trays;
// the Editing workspace became Review, with Edit, Finish and Deliver beside it.
constexpr int kLayoutVersion = 9;
// The oldest version a built-in's layout is restored from: before 9 the
// tree had other panels, so those give way to the arrangement.
constexpr int kOldestLayoutVersion = 9;
// One's own workspace keeps its layout from version 4 on: its panels are
// renamed to today's (migrated()) and today's panels it does not name are
// placed, closed, where the tree has them (Workspace::placeUnassigned()).
constexpr int kOldestOwnLayoutVersion = 4;

// The design's sizes (mockup-spec 0.1): the window less the toolbar and the
// status bar, its top row over the band, its columns.
constexpr int kArrangedWidth = tokens::layout::canvasWidth;
constexpr int kArrangedHeight = tokens::layout::canvasHeight - tokens::layout::toolbar - tokens::layout::status;
constexpr int kBandHeight = tokens::layout::band;
// Every seam is a 1 pt splitter line, counted in the column or row after
// it: the Inspector's, the timeline's and the browser's left line, the
// band's top line.
constexpr int kSeam = tokens::layout::seam;

// Panels that draw their own 28 pt strip, so their area shows no tab row.
const QSet<QString> &ownStripPanels() {
    // The bin draws its own 28 pt strip too (mockup-spec 6); the timeline
    // keeps the tab row as its bar (its tools and zoom live there).
    static const QSet<QString> panels{QStringLiteral("program"), QStringLiteral("inspector"), QStringLiteral("browser"),
                                      QStringLiteral("project"), QStringLiteral("finish"), QStringLiteral("deliver")};
    return panels;
}

QString layoutKey(const QString &group, const QString &name) {
    return QStringLiteral("workspace/%1/%2").arg(group, name);
}

// A saved layout with Effect Controls' old id given the Inspector's.
QByteArray migrated(const QByteArray &layout) {
    QByteArray xml = layout.startsWith("<?xml") ? layout : qUncompress(layout);
    if (xml.isEmpty()) return layout;
    xml.replace("Name=\"effectControls\"", "Name=\"inspector\"");
    xml.replace("Current=\"effectControls\"", "Current=\"inspector\"");
    // Before 9 the Effects and Presets panels were the browser's pages: the
    // browser takes the place of Effects (or of Presets without it), and the
    // other is left out. The Source panel, now a page of the Inspector
    // column, is a name no panel has, so it is left out as it is.
    if (!xml.contains("Name=\"browser\"")) {
        const QByteArray kept = xml.contains("Name=\"effects\"") ? "effects" : "presets";
        const QByteArray dropped = kept == "effects" ? "presets" : "effects";
        xml.replace("Name=\"" + kept + "\"", "Name=\"browser\"");
        xml.replace("Current=\"" + kept + "\"", "Current=\"browser\"");
        xml.replace("Name=\"" + dropped + "\"", "Name=\"retired-" + dropped + "\"");
    }
    return xml;
}

// A dock area's tab strip whose least height is its height. The dock
// manager makes an area's least height its panels' and the strip's least
// height hint, which leaves out the height the sheet gives the strip (see
// dockStyleSheet()), so a panel at its least would run that much under the
// area's foot.
class AreaTitleBar final : public ads::CDockAreaTitleBar {
public:
    using ads::CDockAreaTitleBar::CDockAreaTitleBar;
    QSize minimumSizeHint() const override {
        if (isHidden()) return {0, 0};
        const QSize least = ads::CDockAreaTitleBar::minimumSizeHint();
        return {least.width(), std::max({least.height(), sizeHint().height(), minimumHeight()})};
    }
};

class DockComponents final : public ads::CDockComponentsFactory {
public:
    ads::CDockAreaTitleBar *createDockAreaTitleBar(ads::CDockAreaWidget *area) const override { return new AreaTitleBar(area); }
};

QVariantMap stageMap(const Workspace::StageState &state) {
    return QVariantMap{{QStringLiteral("panel"), state.panelOpen},
                       {QStringLiteral("dim"), state.dim},
                       {QStringLiteral("inspector"), state.inspector}};
}

} // namespace

Workspace::Workspace(QWidget *host) : QObject(host) {
    using Manager = ads::CDockManager;
    ads::CDockComponentsFactory::setFactory(new DockComponents);
    Manager::setConfigFlags(Manager::DefaultOpaqueConfig);
    Manager::setConfigFlag(Manager::AlwaysShowTabs, true);
    // The focused panel shows in its tab's label (see dockStyleSheet()).
    Manager::setConfigFlag(Manager::FocusHighlighting, true);
    // Tabs never show a close button: a panel closes from its tab's menu,
    // the Window menu, or a middle click.
    Manager::setConfigFlag(Manager::ActiveTabHasCloseButton, false);
    Manager::setConfigFlag(Manager::AllTabsHaveCloseButton, false);
    Manager::setConfigFlag(Manager::MiddleMouseButtonClosesTab, true);
    Manager::setConfigFlag(Manager::DockAreaHasUndockButton, false);
    Manager::setConfigFlag(Manager::DockAreaHasCloseButton, false);
    Manager::setConfigFlag(Manager::DockAreaDynamicTabsMenuButtonVisibility, true);
    Manager::setConfigFlag(Manager::EqualSplitOnInsertion, true);
    m_manager = new Manager(host);
    m_manager->setObjectName(QStringLiteral("dockManager"));
    // The dock manager's own sheet, with the editor's colours over it.
    m_baseStyleSheet = m_manager->styleSheet();
    m_manager->setStyleSheet(m_baseStyleSheet + dockStyleSheet(editorTheme()));
    connect(EditorAppearance::instance(), &EditorAppearance::changed, this,
            [this] { m_manager->setStyleSheet(m_baseStyleSheet + dockStyleSheet(editorTheme())); });
}

ads::CDockWidget *Workspace::addPanel(const QString &id, const QString &title, QWidget *content) {
    auto *dock = new ads::CDockWidget(m_manager, title);
    dock->setObjectName(id);
    dock->setWidget(content, ads::CDockWidget::ForceNoScrollArea);
    // Its least size is its content's, and a dock area's the largest of its
    // panels': a split is never dragged into the middle of a panel.
    dock->setMinimumSizeHintMode(ads::CDockWidget::MinimumSizeHintFromContent);
    dock->setFeature(ads::CDockWidget::DockWidgetDeleteOnClose, false);
    // A text tab: its name and nothing round it.
    ads::CDockWidgetTab *tab = dock->tabWidget();
    tab->setProperty("textTab", true);
    if (QLayout *layout = tab->layout()) {
        layout->setContentsMargins(0, 0, 0, 0);
        for (int index = 0; index < layout->count(); ++index)
            if (QSpacerItem *spacer = layout->itemAt(index)->spacerItem()) spacer->changeSize(0, 0);
        layout->invalidate();
    }
    m_panels.insert(id, dock);
    // Opening or closing a panel can leave an area of strip-drawing panels
    // with one open, whose tab row then goes.
    connect(dock, &ads::CDockWidget::viewToggled, this, [this] { QTimer::singleShot(0, this, &Workspace::hideOwnStripTitleBars); });
    return dock;
}

ads::CDockWidget *Workspace::panel(const QString &id) const {
    if (ads::CDockWidget *dock = m_panels.value(id)) return dock;
    // Panels that became pages answer with the panel that holds them.
    if (id == QLatin1String("effects") || id == QLatin1String("presets")) return m_panels.value(QStringLiteral("browser"));
    if (id == QLatin1String("source")) return m_panels.value(QStringLiteral("inspector"));
    return nullptr;
}

void Workspace::setInspectorColumn(InspectorColumn *column) {
    m_column = column;
    if (!column) return;
    // A page chosen by hand: the Inspector stays when the selection clears.
    connect(column, &InspectorColumn::pageChanged, this, [this](InspectorColumn::Page page) {
        if (m_raisingInspector) return;
        m_inspectorAuto = false;
        m_sourceBefore = page == InspectorColumn::Page::Inspector;
    });
}

void Workspace::setBrowserPanel(BrowserPanel *browser) {
    m_browser = browser;
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

void Workspace::arrangeTree() {
    // Final Cut Pro's tree: the Program monitor and the Inspector column in
    // the top row; under them the band of the bin, the timeline and the
    // browser; under that the trays' row, which Finish and Deliver show in
    // place of the band. The comments panel lives in the Program monitor's
    // stage (ViewerWidget::stageLayout), not in a dock of its own.
    const auto panel = [this](const char *id) { return m_panels.value(QString::fromLatin1(id)); };
    ads::CDockAreaWidget *program = m_manager->addDockWidget(ads::LeftDockWidgetArea, panel("program"));
    m_manager->addDockWidget(ads::RightDockWidgetArea, panel("inspector"), program);
    ads::CDockAreaWidget *timeline = m_manager->addDockWidget(ads::BottomDockWidgetArea, panel("timeline"));
    m_manager->addDockWidget(ads::LeftDockWidgetArea, panel("project"), timeline);
    m_manager->addDockWidget(ads::RightDockWidgetArea, panel("browser"), timeline);
    ads::CDockAreaWidget *trays = m_manager->addDockWidget(ads::BottomDockWidgetArea, panel("finish"));
    m_manager->addDockWidget(ads::CenterDockWidgetArea, panel("deliver"), trays);
    trays->setCurrentDockWidget(panel("finish"));
    fitSeams();
    // Laid out at the design's size now, before there is a window, so the
    // layouts saved from it keep these sizes; the Program monitor and the
    // timeline take whatever the window has beyond them.
    QSplitter *top = program->parentSplitter();
    auto *rows = qobject_cast<QSplitter *>(top->parentWidget());
    if (!rows) return;
    rows->setGeometry(QRect(0, 0, kArrangedWidth, kArrangedHeight));
    rows->setSizes({kArrangedHeight - kBandHeight, kBandHeight - kSeam, kBandHeight - kSeam});
    top->setSizes({kArrangedWidth - tokens::layout::inspector, tokens::layout::inspector - kSeam});
    timeline->parentSplitter()->setSizes({tokens::layout::bin - kSeam, kArrangedWidth - tokens::layout::bin - tokens::layout::browser,
                                          tokens::layout::browser - kSeam});
}

void Workspace::arrangeBuiltIn(const QString &name) {
    // Finish and Deliver show their tray in place of the band; Review and
    // Edit the band (they differ in their stage: Edit's panel is closed).
    const bool finish = name == QLatin1String(kFinish);
    const bool deliver = name == QLatin1String(kDeliver);
    const auto open = [this](const char *id, bool shown) {
        ads::CDockWidget *dock = m_panels.value(QString::fromLatin1(id));
        if (dock && dock->isClosed() == shown) dock->toggleView(shown);
    };
    for (const char *id : {"program", "inspector"}) open(id, true);
    for (const char *id : {"project", "timeline", "browser"}) open(id, !finish && !deliver);
    open("finish", finish);
    open("deliver", deliver);
    ads::CDockWidget *program = m_panels.value(QStringLiteral("program"));
    QSplitter *top = program->dockAreaWidget()->parentSplitter();
    auto *rows = qobject_cast<QSplitter *>(top ? top->parentWidget() : nullptr);
    if (rows && rows->count() == 3) {
        rows->setGeometry(QRect(0, 0, kArrangedWidth, kArrangedHeight));
        const int band = kBandHeight - kSeam;
        const bool tray = finish || deliver;
        rows->setSizes({kArrangedHeight - kBandHeight, tray ? 0 : band, tray ? band : 0});
    }
    afterRestore();
}

void Workspace::afterRestore() {
    placeUnassigned();
    fitSeams();
    letStageStretch();
    letTopRowCollapse();
    hideOwnStripTitleBars();
    placeTitleWidgets();
}

void Workspace::placeUnassigned() {
    // A layout from before a panel existed (one's own workspace saved
    // before version 9) does not name it, so restoring it leaves the panel
    // in no area. Each goes, closed as that workspace had it, where the tree
    // has it: the Inspector column on the Program monitor's right, the
    // browser on the timeline's, the trays together under everything.
    const auto areaOf = [this](const char *id) -> ads::CDockAreaWidget * {
        ads::CDockWidget *dock = m_panels.value(QString::fromLatin1(id));
        return dock ? dock->dockAreaWidget() : nullptr;
    };
    const auto place = [this](ads::CDockWidget *dock, ads::DockWidgetArea where, ads::CDockAreaWidget *beside) {
        m_manager->addDockWidget(beside ? where : ads::BottomDockWidgetArea, dock, beside);
        dock->toggleView(false);
    };
    for (auto it = m_panels.cbegin(); it != m_panels.cend(); ++it) {
        ads::CDockWidget *dock = it.value();
        if (!dock || dock->dockAreaWidget()) continue;
        const QString &id = it.key();
        if (id == QLatin1String("inspector")) place(dock, ads::RightDockWidgetArea, areaOf("program"));
        else if (id == QLatin1String("browser")) place(dock, ads::RightDockWidgetArea, areaOf("timeline"));
        else if (id == QLatin1String("finish")) place(dock, ads::CenterDockWidgetArea, areaOf("deliver"));
        else if (id == QLatin1String("deliver")) place(dock, ads::CenterDockWidgetArea, areaOf("finish"));
        else place(dock, ads::BottomDockWidgetArea, nullptr);
    }
}

void Workspace::fitSeams() {
    for (ads::CDockSplitter *splitter : m_manager->findChildren<ads::CDockSplitter *>()) splitter->setHandleWidth(kSeam);
}

void Workspace::letStageStretch() {
    // The Program monitor's row takes the height, the Program monitor the
    // row's width, and the timeline the band's: the Inspector column, the
    // bin, the browser and the band (or a tray) keep theirs.
    const auto stretch = [](QWidget *widget, Qt::Orientation orientation) {
        if (!widget) return;
        QSizePolicy policy = widget->sizePolicy();
        if (orientation == Qt::Horizontal) policy.setHorizontalStretch(1);
        else policy.setVerticalStretch(1);
        widget->setSizePolicy(policy);
    };
    if (ads::CDockWidget *program = m_panels.value(QStringLiteral("program")); program && program->dockAreaWidget()) {
        ads::CDockAreaWidget *area = program->dockAreaWidget();
        stretch(area, Qt::Horizontal);
        // The splitter holding it, unless that is the rows themselves.
        if (QSplitter *row = area->parentSplitter(); row && row->orientation() == Qt::Horizontal) stretch(row, Qt::Vertical);
        else stretch(area, Qt::Vertical);
    }
    if (ads::CDockWidget *timeline = m_panels.value(QStringLiteral("timeline")); timeline && timeline->dockAreaWidget())
        stretch(timeline->dockAreaWidget(), Qt::Horizontal);
}

void Workspace::letTopRowCollapse() {
    // Dragged all the way up, the band's split closes the row over it
    // altogether (the band or a tray then has the height), rather than
    // stopping at a strip of cut-off panels; the band and the trays never close.
    ads::CDockWidget *program = m_panels.value(QStringLiteral("program"));
    QWidget *row = program ? program->dockAreaWidget() : nullptr;
    if (!row) return;
    auto *rows = qobject_cast<QSplitter *>(row->parentWidget());
    while (rows && rows->orientation() != Qt::Vertical) {
        row = rows;
        rows = qobject_cast<QSplitter *>(rows->parentWidget());
    }
    if (!rows) return;
    rows->setChildrenCollapsible(true);
    for (int index = 0; index < rows->count(); ++index) rows->setCollapsible(index, rows->widget(index) == row);
}

void Workspace::hideOwnStripTitleBars() {
    // An area of panels that draw their own strip, one of them open, shows
    // that strip and no tab row over it.
    for (ads::CDockAreaWidget *area : m_manager->findChildren<ads::CDockAreaWidget *>()) {
        ads::CDockAreaTitleBar *bar = area->titleBar();
        if (!bar) continue;
        bool own = area->dockWidgetsCount() > 0 && area->openDockWidgetsCount() <= 1;
        for (int index = 0; own && index < area->dockWidgetsCount(); ++index)
            own = ownStripPanels().contains(area->dockWidget(index)->objectName());
        if (bar->isHidden() != own) {
            bar->setVisible(!own);
            area->updateGeometry();
        }
    }
}

void Workspace::migrateSettings() {
    QSettings settings;
    // A workspace of one's own named as a built-in is now (or as the old
    // Editing) is kept under another name, with its layouts.
    QStringList custom = settings.value(QStringLiteral("workspace/names")).toStringList();
    bool renamed = false;
    for (QString &name : custom) {
        if (!isBuiltIn(name) && name.compare(QLatin1String(kEditing), Qt::CaseInsensitive) != 0) continue;
        const QString own = QStringLiteral("%1 (own)").arg(name);
        for (const QString &group : {QStringLiteral("changes"), QStringLiteral("saved"), QStringLiteral("stage")}) {
            const QString key = layoutKey(group, name);
            if (!settings.contains(key)) continue;
            settings.setValue(layoutKey(group, own), settings.value(key));
            settings.remove(key);
        }
        if (settings.value(QStringLiteral("workspace/current")).toString() == name)
            settings.setValue(QStringLiteral("workspace/current"), own);
        name = own;
        renamed = true;
    }
    if (renamed) settings.setValue(QStringLiteral("workspace/names"), custom);
    // The Editing workspace is Review now; its layouts are older than
    // today's tree, so Review's arrangement takes their place when restored.
    if (settings.value(QStringLiteral("workspace/current")).toString() == QLatin1String(kEditing))
        settings.setValue(QStringLiteral("workspace/current"), QString::fromLatin1(kReview));
    for (const QString &group : {QStringLiteral("changes"), QStringLiteral("saved")}) {
        const QString old = layoutKey(group, QString::fromLatin1(kEditing));
        if (!settings.contains(old)) continue;
        if (!settings.contains(layoutKey(group, QString::fromLatin1(kReview))))
            settings.setValue(layoutKey(group, QString::fromLatin1(kReview)), settings.value(old));
        settings.remove(old);
    }
}

void Workspace::start() {
    migrateSettings();
    arrangeTree();
    arrangeBuiltIn(QString::fromLatin1(kReview));
    m_builtIn.insert(QString::fromLatin1(kReview), m_manager->saveState(kLayoutVersion));
    m_builtIn.insert(QString::fromLatin1(kEdit), m_builtIn.value(QString::fromLatin1(kReview)));
    for (const char *tray : {kFinish, kDeliver}) {
        arrangeBuiltIn(QString::fromLatin1(tray));
        m_builtIn.insert(QString::fromLatin1(tray), m_manager->saveState(kLayoutVersion));
    }
    restore(m_builtIn.value(QString::fromLatin1(kReview)), QString::fromLatin1(kReview));
    const QSettings settings;
    m_current = settings.value(QStringLiteral("workspace/current"), QString::fromLatin1(kReview)).toString();
    if (!names().contains(m_current)) m_current = QString::fromLatin1(kReview);
    const QByteArray changes = settings.value(layoutKey(QStringLiteral("changes"), m_current)).toByteArray();
    // A layout that no longer restores (one from before today's tree)
    // leaves the workspace's own arrangement.
    if (!restore(changes, m_current) && !restore(savedLayout(m_current), m_current))
        restore(builtInLayout(m_current), m_current);
}

QStringList Workspace::builtIns() {
    return {QString::fromLatin1(kEdit), QString::fromLatin1(kReview), QString::fromLatin1(kFinish), QString::fromLatin1(kDeliver)};
}

bool Workspace::isBuiltIn(const QString &name) {
    return builtIns().contains(name, Qt::CaseInsensitive);
}

QStringList Workspace::names() const {
    QStringList names = builtIns();
    for (const QString &name : QSettings().value(QStringLiteral("workspace/names")).toStringList())
        if (!names.contains(name, Qt::CaseInsensitive)) names.append(name);
    return names;
}

QByteArray Workspace::builtInLayout(const QString &name) const {
    // One's own workspace starts from Review's.
    return m_builtIn.value(name, m_builtIn.value(QString::fromLatin1(kReview)));
}

QByteArray Workspace::savedLayout(const QString &name) const {
    const QByteArray saved = QSettings().value(layoutKey(QStringLiteral("saved"), name)).toByteArray();
    return saved.isEmpty() ? builtInLayout(name) : saved;
}

bool Workspace::restore(const QByteArray &layout, const QString &name) {
    if (layout.isEmpty()) return false;
    bool restored = false;
    const QByteArray state = migrated(layout);
    const int oldest = isBuiltIn(name) ? kOldestLayoutVersion : kOldestOwnLayoutVersion;
    parkTitleWidgets();
    for (int version = kLayoutVersion; !restored && version >= oldest; --version)
        restored = m_manager->restoreState(state, version);
    if (restored) afterRestore();
    else placeTitleWidgets();
    return restored;
}

void Workspace::activate(const QString &name) {
    if (name == m_current || !names().contains(name)) return;
    persist();
    m_current = name;
    m_liveStage.reset();
    const QByteArray changes = QSettings().value(layoutKey(QStringLiteral("changes"), name)).toByteArray();
    if (!restore(changes, name) && !restore(savedLayout(name), name)) restore(builtInLayout(name), name);
    persist();
    emit changed();
}

void Workspace::saveCurrent() {
    QSettings settings;
    settings.setValue(layoutKey(QStringLiteral("saved"), m_current), m_manager->saveState(kLayoutVersion));
    settings.remove(layoutKey(QStringLiteral("changes"), m_current));
    settings.setValue(layoutKey(QStringLiteral("stage"), m_current), stageMap(stageState()));
}

bool Workspace::saveAs(const QString &name) {
    const QString trimmed = name.trimmed();
    if (trimmed.isEmpty() || names().contains(trimmed, Qt::CaseInsensitive) ||
        trimmed.compare(QLatin1String(kEditing), Qt::CaseInsensitive) == 0)
        return false;
    // The new workspace keeps the stage it was made from.
    const StageState stage = stageState();
    QSettings settings;
    QStringList custom = settings.value(QStringLiteral("workspace/names")).toStringList();
    custom.append(trimmed);
    settings.setValue(QStringLiteral("workspace/names"), custom);
    m_current = trimmed;
    m_liveStage = stage;
    saveCurrent();
    persist();
    emit changed();
    return true;
}

void Workspace::resetCurrent() {
    if (!restore(savedLayout(m_current), m_current)) restore(builtInLayout(m_current), m_current);
    QSettings().remove(layoutKey(QStringLiteral("changes"), m_current));
    m_liveStage.reset();
    emit changed();
}

void Workspace::persist() {
    QSettings settings;
    settings.setValue(QStringLiteral("workspace/current"), m_current);
    settings.setValue(layoutKey(QStringLiteral("changes"), m_current), m_manager->saveState(kLayoutVersion));
}

Workspace::StageState Workspace::defaultStageState(const QString &name) {
    StageState state;
    // Edit shows the Source viewer where the panel was; Finish dims the
    // surround round the picture being graded.
    state.panelOpen = name != QLatin1String(kEdit);
    state.dim = name == QLatin1String(kFinish);
    return state;
}

Workspace::StageState Workspace::stageState(const QString &name) const {
    const QString which = name.isEmpty() ? m_current : name;
    if (which == m_current && m_liveStage) return *m_liveStage;
    const QVariantMap saved = QSettings().value(layoutKey(QStringLiteral("stage"), which)).toMap();
    StageState state = defaultStageState(which);
    if (saved.isEmpty()) return state;
    state.panelOpen = saved.value(QStringLiteral("panel"), state.panelOpen).toBool();
    state.dim = saved.value(QStringLiteral("dim"), state.dim).toBool();
    state.inspector = saved.value(QStringLiteral("inspector"), state.inspector).toString();
    return state;
}

void Workspace::setStageState(const StageState &state) {
    m_liveStage = state;
}

bool Workspace::inspectorFollowsSelection() {
    return QSettings().value(QStringLiteral("editor/inspectorFollowsSelection"), true).toBool();
}

void Workspace::setInspectorFollowsSelection(bool follows) {
    QSettings().setValue(QStringLiteral("editor/inspectorFollowsSelection"), follows);
}

void Workspace::raiseInspector(InspectorOrigin origin, bool force, const QString &clip) {
    ads::CDockWidget *inspector = m_panels.value(QStringLiteral("inspector"));
    if (!inspector || inspector->isClosed() || inspector->isFloating() || !m_column) return;
    if (origin == InspectorOrigin::Edit && !force) return;
    if (!force && !inspectorFollowsSelection()) return;
    if (m_column->page() == InspectorColumn::Page::Inspector) {
        m_inspectorClip = clip;
        return;
    }
    // The Source page chosen with this clip still selected: only a new click raises.
    if (!force && !clip.isEmpty() && clip == m_inspectorClip) return;
    m_inspectorClip = clip;
    QWidget *focus = QApplication::focusWidget();
    m_raisingInspector = true;
    m_column->setPage(InspectorColumn::Page::Inspector);
    m_raisingInspector = false;
    m_inspectorAuto = true;
    m_sourceBefore = true;
    // Raising is visual only: the keys stay where they were.
    if (focus && QApplication::focusWidget() != focus) focus->setFocus(Qt::OtherFocusReason);
}

void Workspace::showBrowserForInspector(const QString &id) {
    ads::CDockWidget *browser = m_panels.value(QStringLiteral("browser"));
    if (!browser) return;
    if (browser->isClosed()) browser->toggleView(true);
    if (m_browser) m_browser->setPage(BrowserPanel::pageFromId(id));
}

void Workspace::inspectorDeselected() {
    m_inspectorClip.clear();
    ads::CDockWidget *inspector = m_panels.value(QStringLiteral("inspector"));
    if (!inspector || !m_inspectorAuto || inspector->isClosed() || !m_column) return;
    m_inspectorAuto = false;
    if (m_column->page() != InspectorColumn::Page::Inspector || !m_sourceBefore) return;
    m_raisingInspector = true;
    m_column->setPage(InspectorColumn::Page::Source);
    m_raisingInspector = false;
}

void Workspace::toggleInspector() {
    ads::CDockWidget *inspector = m_panels.value(QStringLiteral("inspector"));
    if (!inspector) return;
    const bool wasClosed = inspector->isClosed();
    if (wasClosed) inspector->toggleView(true);
    m_inspectorAuto = false;
    if (!m_column) return;
    m_raisingInspector = true;
    if (m_column->page() == InspectorColumn::Page::Source) {
        m_sourceBefore = true;
        m_column->setPage(InspectorColumn::Page::Inspector);
    } else if (!wasClosed && m_sourceBefore) {
        m_sourceBefore = false;
        m_column->setPage(InspectorColumn::Page::Source);
    }
    m_raisingInspector = false;
}

} // namespace kerf
