#include "toolbar_wiring.h"

#include "chat_panel.h"
#include "command_registry.h"
#include "composer.h"
#include "components.h"
#include "editor_bus.h"
#include "editor_common.h"
#include "main_window.h"
#include "main_window_shared.h"
#include "master_output.h"
#include "review_model.h"
#include "project_sidebar.h"
#include "toolbar_widgets.h"
#include "stage_motion.h"
#include "stage_shell.h"
#include "wiring/stage_wiring.h"
#include "wiring/bin_wiring.h"
#include "wiring/timeline_wiring.h"
#include "timeline_widget.h"

#include <QAction>
#include <QActionGroup>
#include <QEvent>
#include <QHBoxLayout>
#include <QKeySequence>
#include <QLabel>
#include <QLineEdit>
#include <QMenuBar>
#include <QRadioButton>
#include <QSet>
#include <QSettings>
#include <QTimer>
#include <QToolButton>

#include <algorithm>
#include <cmath>
#include <tuple>
#include <utility>

namespace kerf {

using namespace window_parts;

namespace {

QString nativeKeys(const QAction *action) {
    return action ? action->shortcut().toString(QKeySequence::NativeText) : QString();
}

// The first letter in lower case, unless the word is an initialism ("QC").
QString lowerFirst(const QString &text) {
    if (text.size() < 2 || text.at(1).isUpper()) return text;
    return text.at(0).toLower() + text.mid(1);
}

QString plural(qint64 count, const QString &one, const QString &many) {
    return count == 1 ? one : many.arg(count);
}

} // namespace

ToolbarWiring::ToolbarWiring(MainWindow *window, EditorBus *bus)
    : QObject(nullptr), m_window(window), m_bus(bus), m_clock([] { return QDateTime::currentDateTimeUtc(); }) {
    setObjectName(QStringLiteral("toolbarWiring"));
    m_messageTimer = new QTimer(this);
    m_messageTimer->setSingleShot(true);
    connect(m_messageTimer, &QTimer::timeout, this, [this] {
        m_message.clear();
        layoutTitle();
    });
}

void ToolbarWiring::setClock(std::function<QDateTime()> clock) {
    m_clock = clock ? std::move(clock) : std::function<QDateTime()>([] { return QDateTime::currentDateTimeUtc(); });
    layoutTitle();
}

// ================================================================ toolbar

int ToolbarWiring::firstControlX(int windowButtonsRight, bool fullScreen) {
    constexpr int pad = 16;
    if (fullScreen) return pad;
    // The mockup reserves the three 12 pt lights, 8 apart, and 16 after them.
    constexpr int reserved = pad + 52 + 16;
    return windowButtonsRight > 0 ? std::max(reserved, windowButtonsRight + 16) : reserved;
}

void ToolbarWiring::buildToolbar(const QHash<QString, QMenu *> &menus) {
    MainWindow *w = m_window;
    auto *toolbar = new FlatToolBar(QStringLiteral("Main"), w);
    w->addToolBar(toolbar);
    w->m_toolbar = toolbar;
    toolbar->setFixedHeight(kToolbarHeight);
    // Its empty stretches move and zoom the window (on macOS, where it is
    // the title bar), and its size places the title.
    toolbar->installEventFilter(w);

    m_content = new QWidget;
    m_content->setObjectName(QStringLiteral("toolbarRow"));
    auto *row = new QHBoxLayout(m_content);
    row->setContentsMargins(kLeadingPad, 0, 12, 0);
    row->setSpacing(4);
    // The room before the first control: the window buttons' on macOS, and the
    // sidebar's while it shows, whose top this stretch of the bar is.
    w->m_windowButtons = new QWidget;
    w->m_windowButtons->setObjectName(QStringLiteral("windowButtonsSpace"));
    w->m_windowButtons->setFixedWidth(0);
    row->addWidget(w->m_windowButtons);
    m_menus = menus;
    const auto paneButton = [w](const QString &icon, const char *name, const QString &accessible, const QString &command) {
        auto *button = new ui::IconButton(icon, ui::IconButton::Size::S28);
        button->setObjectName(QString::fromLatin1(name));
        button->setAccessibleName(accessible);
        button->setCheckable(true);
        button->setFocusPolicy(Qt::TabFocus);
        connect(button, &QAbstractButton::clicked, w, [w, command] {
            if (QAction *action = w->m_commands->action(command)) action->trigger();
        });
        return button;
    };
    w->m_sidebarButton = paneButton(QStringLiteral("g-sidebar"), "paneSidebar", QStringLiteral("Sidebar"), QStringLiteral("view.sidebar"));
    row->addWidget(w->m_sidebarButton);
    // Everything else is at the sidebar's foot, under the account. With the
    // sidebar away, the same menu opens from here.
    m_appButton = new ui::IconButton(QStringLiteral("g-more"), ui::IconButton::Size::S28);
    m_appButton->setObjectName(QStringLiteral("appMenu"));
    m_appButton->setAccessibleName(QStringLiteral("Menu"));
    m_appButton->setTip(QStringLiteral("Settings, account and more"));
    m_appButton->setCheckable(false);
    m_appButton->setFocusPolicy(Qt::TabFocus);
    m_appMenu = new QMenu(m_content);
    m_appMenu->setObjectName(QStringLiteral("appMenuPopup"));
    ui::styleMenu(m_appMenu);
    connect(m_appButton, &QAbstractButton::clicked, this, [this] { popupAppMenu(m_appButton); });
    row->addWidget(m_appButton);
    if (!w->menuBar()->isNativeMenuBar()) {
        // The native menus live in that menu; their keys go on working.
        for (const QString &name : w->m_commands->menuNames()) {
            QMenu *menu = menus.value(name);
            if (!menu) continue;
            for (QAction *action : menu->actions())
                if (action->shortcutContext() != Qt::WidgetShortcut) w->addAction(action);
        }
        w->menuBar()->hide();
    }
    row->addSpacing(8 - row->spacing());

    // The project and what is going on in it, at the leading end: the room the
    // row has left is theirs.
    m_titleBox = new QWidget;
    m_titleBox->setObjectName(QStringLiteral("toolbarTitle"));
    m_titleBox->installEventFilter(this);
    // On macOS it is title bar too: dragging it moves the window.
    m_titleBox->installEventFilter(w);
    row->addWidget(m_titleBox, 1);

    // The trailing group: Export once there is a cut, then the panel.
    w->m_exportButton = new ExportButton;
    connect(w->m_exportButton, &ExportButton::exportRequested, this, &ToolbarWiring::sendExport);
    connect(w->m_exportButton, &ExportButton::menuAboutToShow, this, &ToolbarWiring::buildExportMenu);
    connect(w->m_exportButton, &ExportButton::qualityChanged, this, &ToolbarWiring::syncCommands);
    w->m_exportButton->hide();
    row->addWidget(w->m_exportButton);
    row->addSpacing(8 - row->spacing());
    w->m_panelButton = new ui::IconButton(QStringLiteral("g-panel"), ui::IconButton::Size::S28);
    w->m_panelButton->setObjectName(QStringLiteral("panePanel"));
    w->m_panelButton->setAccessibleName(QStringLiteral("Picture and timeline"));
    w->m_panelButton->setCheckable(true);
    w->m_panelButton->setFocusPolicy(Qt::TabFocus);
    connect(w->m_panelButton, &QAbstractButton::clicked, w, [w] { w->setPanelOpen(!w->m_shell->panes().panelOpen()); });
    row->addWidget(w->m_panelButton);
    toolbar->setContent(m_content);

    // What belongs to the cut sits with it, in the timeline's strip: the Media
    // sheet, Follow Kerf, and the format. ⌥1-2 choose the format from anywhere
    // in the window, as before.
    const auto stripButton = [w](const QString &icon, const char *name, const QString &accessible, const QString &command) {
        auto *button = new ui::IconButton(icon, ui::IconButton::Size::S24);
        button->setObjectName(QString::fromLatin1(name));
        button->setAccessibleName(accessible);
        button->setCheckable(true);
        button->setFocusPolicy(Qt::TabFocus);
        connect(button, &QAbstractButton::clicked, w, [w, command] {
            if (QAction *action = w->m_commands->action(command)) action->trigger();
        });
        return button;
    };
    w->m_mediaButton = stripButton(QStringLiteral("g-grid"), "paneMedia", QStringLiteral("Media"), QStringLiteral("window.project"));
    w->m_followButton = stripButton(QStringLiteral("g-eye"), "paneFollowKerf", QStringLiteral("Follow Kerf"), QStringLiteral("view.followKerf"));
    w->m_followButton->setOn(w->m_followKerf);
    w->m_formatBank = new FormatBank;
    connect(w->m_formatBank, &FormatBank::formatChosen, w, &MainWindow::canvasPresetChanged);
    if (TimelineToolStrip *strip = w->m_timelineWiring ? w->m_timelineWiring->tools() : nullptr) {
        strip->extras()->addWidget(w->m_mediaButton, 0, Qt::AlignVCenter);
        strip->extras()->addWidget(w->m_followButton, 0, Qt::AlignVCenter);
        strip->extras()->addSpacing(8);
        strip->extras()->addWidget(w->m_formatBank, 0, Qt::AlignVCenter);
    }
    for (const QString &id : formatCommands())
        if (QAction *action = w->m_commands->action(id)) {
            if (!w->actions().contains(action)) w->addAction(action);
            m_formatActions << action;
        }
    installCommands();

    w->m_projectTitle = new QLabel(m_titleBox);
    w->m_projectTitle->setObjectName(QStringLiteral("projectTitle"));
    w->m_projectSubtitle = new QLabel(m_titleBox);
    w->m_projectSubtitle->setObjectName(QStringLiteral("projectSubtitle"));
    // Its own sheet, so the app's QLabel rules leave its type alone:
    // UI 500 13/16 ink, then UI 11/16 ink3 on the same line.
    w->m_projectTitle->setStyleSheet(QStringLiteral("QLabel { color: %1; font-size: 13px; font-weight: 500; padding: 0px; "
                                                    "background: transparent; }")
                                         .arg(tokens::color(tokens::ink).name()));
    w->m_projectSubtitle->setStyleSheet(QStringLiteral("QLabel { color: %1; font-size: 11px; font-weight: 400; padding: 0px; "
                                                       "background: transparent; }")
                                            .arg(tokens::color(tokens::ink3).name()));
    for (QLabel *label : {w->m_projectTitle, w->m_projectSubtitle}) {
        label->setFont(tokens::font(tokens::TypeStyle{label == w->m_projectTitle ? 13 : 11, 16,
                                                      label == w->m_projectTitle ? 500 : 400, false}));
        label->setAlignment(Qt::AlignLeft | Qt::AlignVCenter);
        label->installEventFilter(w);
    }
    // "Edited 2 min ago", in the title's tip, moves on by itself.
    m_titleTimer = new QTimer(this);
    m_titleTimer->setInterval(30'000);
    connect(m_titleTimer, &QTimer::timeout, this, &ToolbarWiring::layoutTitle);
    m_titleTimer->start();

    // The bar's leading stretch follows the sidebar under it, also as it moves.
    if (w->m_sidebar) {
        w->m_sidebar->installEventFilter(this);
        connect(w->m_sidebar, &ProjectSidebar::accountRequested, this, [this] { popupAppMenu(m_window->m_sidebar->accountRow()); });
    }

    connect(m_bus, &EditorBus::event, this, &ToolbarWiring::onEvent);
    connect(m_bus, &EditorBus::snapshotChanged, this, &ToolbarWiring::onSnapshot);
    connect(w->m_commands, &CommandRegistry::shortcutsChanged, this, &ToolbarWiring::syncCommands);
    connect(m_bus, &EditorBus::viewAsChanged, this, &ToolbarWiring::layoutTitle);
    connect(m_bus, &EditorBus::reviewChanged, this, &ToolbarWiring::followReview);
    followReview();
    if (QAction *master = w->m_commands->action(QStringLiteral("export.master")))
        connect(master, &QAction::changed, this, &ToolbarWiring::syncCommands);
    syncCommands();
    layoutLeading();
    followCut();
}

int ToolbarWiring::sidebarEdge() const {
    const QWidget *sidebar = m_window->m_sidebar;
    if (!sidebar || sidebar->isHidden()) return 0;
    return std::max(0, sidebar->geometry().right() + 1);
}

int ToolbarWiring::leadingRoom(int firstControl, int sidebarEdge) {
    // The first control starts 12 past the sidebar's edge, or where the window
    // buttons leave it, whichever is further in.
    return std::max({0, firstControl - kLeadingPad - 4, sidebarEdge > 0 ? sidebarEdge + 12 - kLeadingPad - 4 : 0});
}

void ToolbarWiring::layoutLeading() {
    MainWindow *w = m_window;
    if (!w->m_windowButtons || !w->m_toolbar) return;
    const int edge = sidebarEdge();
    const int room = leadingRoom(m_firstControl, edge);
    // With no room to keep, the row's gap after it goes too.
    w->m_windowButtons->setVisible(room > 0);
    w->m_windowButtons->setFixedWidth(room);
    static_cast<FlatToolBar *>(w->m_toolbar)->setSidebarEdge(edge);
    // With the sidebar away its foot is too: the menu is here instead.
    if (m_appButton) m_appButton->setVisible(edge == 0);
}

void ToolbarWiring::setFirstControl(int x) {
    if (x == m_firstControl) return;
    m_firstControl = x;
    layoutLeading();
}

bool ToolbarWiring::hasCut() const {
    for (const QJsonValue &track : m_bus->timeline().value(QStringLiteral("tracks")).toArray())
        if (!track.toObject().value(QStringLiteral("clips")).toArray().isEmpty()) return true;
    return false;
}

void ToolbarWiring::followCut() {
    ExportButton *button = m_window->m_exportButton;
    if (!button) return;
    const bool shown = hasCut();
    if (shown == !button->isHidden()) return;
    button->setVisible(shown);
    // It arrives with the cut, where it is.
    if (shown) moves::fadeIn(button);
    layoutTitle();
}

void ToolbarWiring::popupAppMenu(QWidget *anchor) {
    buildAppMenu();
    SidebarAccount *account = m_window->m_sidebar ? m_window->m_sidebar->accountRow() : nullptr;
    const bool fromAccount = account && anchor == account;
    if (fromAccount) {
        account->setOpen(true);
        connect(m_appMenu, &QMenu::aboutToHide, account, [account] { account->setOpen(false); }, Qt::SingleShotConnection);
    }
    ui::popupMenu(m_appMenu, anchor, fromAccount ? ui::Side::Above : ui::Side::Below);
}

void ToolbarWiring::syncCommands() {
    MainWindow *w = m_window;
    if (!w->m_commands) return;
    CommandRegistry *commands = w->m_commands;
    if (w->m_exportButton) {
        QAction *master = commands->action(QStringLiteral("export.master"));
        const bool enabled = master && master->isEnabled();
        const bool isMaster = w->m_exportButton->exportQuality() == QStringLiteral("master");
        const QString name = isMaster ? QStringLiteral("Export master") : QStringLiteral("Export draft");
        const QString reason = commands->reason(QStringLiteral("export.master"));
        w->m_exportButton->setExportEnabled(enabled, enabled || reason.isEmpty() ? name : QStringLiteral("%1 · %2").arg(name, reason),
                                            isMaster ? nativeKeys(master) : QString());
    }
    // The panes' buttons name their commands' keys.
    for (const auto &[button, id, name] : {std::tuple<ui::IconButton *, QString, QString>{w->m_sidebarButton, QStringLiteral("view.sidebar"), QStringLiteral("Sidebar")},
                                           {w->m_followButton, QStringLiteral("view.followKerf"), QStringLiteral("Follow Kerf")},
                                           {w->m_mediaButton, QStringLiteral("window.project"), QStringLiteral("Media")}})
        if (button) button->setTip(name, nativeKeys(commands->action(id)));
    if (w->m_panelButton) w->m_panelButton->setTip(QStringLiteral("Picture and timeline"));
    // The format segments name the keys of the registry's ⌥1-2.
    if (FormatBank *bank = w->m_formatBank)
        for (int i = 0; i < m_formatActions.size() && i < bank->count(); ++i)
            bank->setSegmentTip(i, formatTip(i), nativeKeys(m_formatActions.at(i)));
}

QStringList ToolbarWiring::formatCommands() {
    return {QStringLiteral("sequence.setFormat.169"), QStringLiteral("sequence.setFormat.916")};
}

void ToolbarWiring::installCommands() {
    MainWindow *w = m_window;
    if (!w->m_commands) return;
    const QStringList formats = formatCommands();
    for (int i = 0; i < formats.size(); ++i) w->m_commandHandlers.insert(formats.at(i), [this, i] { chooseFormat(i); });
    w->m_commandHandlers.insert(QStringLiteral("composer.draw"), [this] { drawInComposer(); });
    if (Composer *composer = w->m_chat ? w->m_chat->composerWidget() : nullptr)
        composer->setDrawAction(w->m_commands->action(QStringLiteral("composer.draw")));
    if (w->m_chat) w->m_chat->setAcceptAction(w->m_commands->action(QStringLiteral("proposal.accept")));
    // The bin's commands, now that the registry is there.
    if (w->m_binWiring) w->m_binWiring->attachCommands();
}

bool ToolbarWiring::drawInComposer() {
    Composer *composer = m_window->m_chat ? m_window->m_chat->composerWidget() : nullptr;
    if (!composer || !composer->hasFocusWithin()) return false;
    composer->requestDraw();
    return true;
}

void ToolbarWiring::chooseFormat(int index) {
    FormatBank *bank = m_window->m_formatBank;
    if (!bank || !bank->isEnabled() || index < 0 || index >= bank->count()) return;
    bank->setCurrentIndex(index);
    emit bank->activated(index);
}

// ================================================================ the account's menu

void ToolbarWiring::addCommandItem(QMenu *menu, const QString &id, const QString &label, const char *name) {
    // The mockup's words for the registry's command: its state, its key.
    CommandRegistry *commands = m_window->m_commands;
    QAction *command = commands->action(id);
    QAction *item = menu->addAction(label);
    item->setObjectName(QString::fromLatin1(name));
    if (!command) {
        item->setEnabled(false);
        return;
    }
    item->setEnabled(command->isEnabled());
    item->setToolTip(commands->reason(id));
    item->setCheckable(command->isCheckable());
    item->setChecked(command->isChecked());
    ui::setMenuKey(item, nativeKeys(command));
    connect(item, &QAction::triggered, this, [this, id] { m_window->triggerCommand(id); });
}

void ToolbarWiring::buildAppMenu() {
    MainWindow *w = m_window;
    QMenu *menu = m_appMenu;
    menu->clear();
    menu->setToolTipsVisible(true);
    // What the account opens: Settings and the account first, then the help.
    addCommandItem(menu, QStringLiteral("app.preferences"), QStringLiteral("Settings…"), "appMenu.settings");
    QAction *account = menu->addAction(QStringLiteral("Account…"));
    account->setObjectName(QStringLiteral("appMenu.account"));
    connect(account, &QAction::triggered, this, [this] { m_window->showSettings(QStringLiteral("account")); });
    addCommandItem(menu, QStringLiteral("help.guide"), QStringLiteral("User Guide"), "appMenu.guide");
    menu->addSeparator();
    addCommandItem(menu, QStringLiteral("window.project"), QStringLiteral("Media"), "appMenu.media");
    addCommandItem(menu, QStringLiteral("file.import"), QStringLiteral("Import…"), "appMenu.import");

    // The panes, the picture's switches, the format, the keyboard's choice and
    // the sync.
    if (!m_viewMenu) {
        m_viewMenu = new QMenu(QStringLiteral("View"), menu);
        m_viewMenu->setObjectName(QStringLiteral("appMenuView"));
        ui::styleMenu(m_viewMenu);
        m_keymapMenu = new QMenu(QStringLiteral("Keyboard"), m_viewMenu);
        m_keymapMenu->setObjectName(QStringLiteral("keymapMenu"));
        ui::styleMenu(m_keymapMenu);
        m_syncMenu = new QMenu(QStringLiteral("Sync"), m_viewMenu);
        m_syncMenu->setObjectName(QStringLiteral("syncMenu"));
        ui::styleMenu(m_syncMenu);
    }
    m_viewMenu->clear();
    addCommandItem(m_viewMenu, QStringLiteral("view.sidebar"), QStringLiteral("Sidebar"), "appMenu.sidebar");
    addCommandItem(m_viewMenu, QStringLiteral("view.picture"), QStringLiteral("Picture"), "appMenu.picture");
    addCommandItem(m_viewMenu, QStringLiteral("view.timeline"), QStringLiteral("Timeline"), "appMenu.timeline");
    addCommandItem(m_viewMenu, QStringLiteral("view.followKerf"), QStringLiteral("Follow Kerf"), "appMenu.followKerf");
    m_viewMenu->addSeparator();
    for (QAction *tool : std::as_const(w->m_maskTools)) m_viewMenu->addAction(tool);
    m_viewMenu->addSeparator();
    for (const char *id : {"view.annotations", "view.safeZones"})
        if (QAction *action = w->m_commands->action(QString::fromLatin1(id))) m_viewMenu->addAction(action);
    if (w->m_stageWiring) m_viewMenu->addMenu(w->m_stageWiring->zoomMenu());
    m_viewMenu->addSeparator();
    buildKeymapMenu(m_keymapMenu);
    m_viewMenu->addMenu(m_keymapMenu);
    buildSyncMenu(m_syncMenu);
    m_viewMenu->addMenu(m_syncMenu);
    QAction *view = menu->addMenu(m_viewMenu);
    view->setObjectName(QStringLiteral("appMenu.view"));

    // Without a native menu bar, the application's menus stay reachable here.
    if (!w->menuBar()->isNativeMenuBar()) {
        menu->addSeparator();
        for (const QString &name : w->m_commands->menuNames())
            if (QMenu *native = m_menus.value(name)) menu->addMenu(native);
    }
    // Signed in to Kerf Cloud, the way out is here too.
    if (w->cloudSignedIn()) {
        menu->addSeparator();
        QAction *signOut = menu->addAction(QStringLiteral("Sign Out"));
        signOut->setObjectName(QStringLiteral("appMenu.signOut"));
        connect(signOut, &QAction::triggered, this, [this] {
            m_window->sendCloud(QJsonObject{{QStringLiteral("type"), QStringLiteral("cloud_sign_out")}});
        });
    }
}

// ================================================================ title

QString ToolbarWiring::projectLabel() const {
    const QString name = m_bus->timeline().value(QStringLiteral("name")).toString();
    return name.isEmpty() ? m_window->m_projectName : name;
}

QString ToolbarWiring::titleText() const {
    const QString project = projectLabel();
    const QJsonObject timeline = m_bus->timeline();
    const QString id = timeline.value(QStringLiteral("sequence")).toString();
    QString sequence;
    for (const QJsonValue &value : timeline.value(QStringLiteral("sequences")).toArray())
        if (value.toObject().value(QStringLiteral("id")).toString() == id) sequence = value.toObject().value(QStringLiteral("name")).toString();
    if (sequence.isEmpty() || sequence == project) return project;
    return project.isEmpty() ? sequence : QStringLiteral("%1 · %2").arg(project, sequence);
}

QString ToolbarWiring::normalSubtitle() const {
    const QJsonObject snapshot = m_bus->snapshot();
    if (snapshot.isEmpty()) return QString();
    const QString revision = QStringLiteral("rev %1").arg(m_bus->revision());
    const QDateTime when = QDateTime::fromString(snapshot.value(QStringLiteral("revisionTime")).toString(), Qt::ISODateWithMs);
    const QString ago = editedAgo(when, m_clock());
    QString line = ago.isEmpty() ? revision : QStringLiteral("Edited %1 · %2").arg(ago, revision);
    if (const QString guest = guestNote(); !guest.isEmpty()) line += QStringLiteral(" · ") + guest;
    return line;
}

QString ToolbarWiring::standingProblem() const {
    const MainWindow *w = m_window;
    if (m_connection == QStringLiteral("disconnected")) return QStringLiteral("Offline");
    if (m_connection == QStringLiteral("connecting")) return QStringLiteral("Connecting…");
    if (w->m_errorBanner && !w->m_errorBanner->isHidden() && w->m_errorMessage) return w->m_errorMessage->text();
    if (w->m_collabStatus.value(QStringLiteral("conflicted")).toBool()) return QStringLiteral("A merge is unfinished; finish it with git");
    if (!w->m_syncFailure.isEmpty()) return QStringLiteral("Sync failed: %1").arg(w->m_syncFailure);
    return {};
}

QString ToolbarWiring::runningText() const {
    const MainWindow *w = m_window;
    static const QHash<QString, QString> stages{
        {QStringLiteral("committing"), QStringLiteral("Committing")}, {QStringLiteral("fetching"), QStringLiteral("Pulling")},
        {QStringLiteral("merging"), QStringLiteral("Merging")}, {QStringLiteral("pushing"), QStringLiteral("Pushing")},
        {QStringLiteral("downloading"), QStringLiteral("Downloading media")}};
    const auto percent = [](double fraction) { return int(std::floor(std::clamp(fraction, 0.0, 1.0) * 100.0)); };
    const RunningExport *latest = nullptr;
    for (auto it = m_runningExports.cbegin(); it != m_runningExports.cend(); ++it)
        if (!latest || std::pair(!it->queued, it->started) > std::pair(!latest->queued, latest->started)) latest = &it.value();
    if (latest) return QStringLiteral("%1 · %2%").arg(latest->label).arg(percent(latest->fraction));
    if (m_previewRunning) return QStringLiteral("Rendering preview · %1%").arg(percent(m_previewFraction));
    if (!w->m_syncStage.isEmpty() || w->m_collabStatus.value(QStringLiteral("busy")).toBool())
        return QStringLiteral("%1…").arg(stages.value(w->m_syncStage, QStringLiteral("Syncing")));
    return {};
}

QString ToolbarWiring::guestNote() const {
    const ReviewModel *review = m_bus->review();
    const bool guest = review ? review->guestMode() : !m_bus->viewAs().isEmpty();
    if (!guest) return {};
    const QString name = review ? review->viewAsName() : m_bus->viewAs();
    return QStringLiteral("Viewing as %1").arg(name.isEmpty() ? m_bus->viewAs() : name);
}

QString ToolbarWiring::subtitleText() const {
    if (!m_message.isEmpty()) return m_message;
    if (const QString problem = standingProblem(); !problem.isEmpty()) return problem;
    if (const QString running = runningText(); !running.isEmpty()) return running;
    // Nothing to say is said with nothing: when it was edited is the title's tip.
    return guestNote();
}

void ToolbarWiring::showMessage(const QString &message, int timeout) {
    m_message = message;
    m_messageTimer->stop();
    if (timeout > 0 && !message.isEmpty()) m_messageTimer->start(timeout);
    layoutTitle();
}

void ToolbarWiring::layoutTitle() {
    MainWindow *w = m_window;
    QLabel *title = w->m_projectTitle;
    QLabel *subtitle = w->m_projectSubtitle;
    if (!title || !subtitle || !m_titleBox) return;
    title->ensurePolished();
    subtitle->ensurePolished();
    const QString full = titleText();
    const QString sub = subtitleText();
    const QFontMetrics metrics(title->font());
    const QFontMetrics subMetrics(subtitle->font());
    // One line: the title, then what is going on, in what the row leaves them.
    // The title gives way first only when there is nothing else to say.
    const int room = std::max(0, m_titleBox->width() - kTitleClearance);
    const int subWidth = sub.isEmpty() ? 0 : std::min(subMetrics.horizontalAdvance(sub) + 2, room * 3 / 5);
    const int gap = sub.isEmpty() ? 0 : 12;
    const int width = std::min(metrics.horizontalAdvance(full) + 2, std::max(0, room - subWidth - gap));
    const QString shown = metrics.horizontalAdvance(full) + 2 > width ? metrics.elidedText(full, Qt::ElideRight, width) : full;
    title->setText(shown);
    // The whole of a shortened title; else when it was edited and at which
    // revision, for whoever asks.
    ui::setTip(title, shown == full ? normalSubtitle() : full);
    const int subRoom = std::max(0, room - width - gap);
    subtitle->setText(subMetrics.elidedText(sub, Qt::ElideRight, subRoom));
    // The whole of a long message, and an offline or error state in its colour.
    ui::setTip(subtitle, subtitle->text() == sub ? QString() : sub);
    const bool problem = m_message.isEmpty() && !standingProblem().isEmpty();
    subtitle->setProperty("problem", problem);
    subtitle->setStyleSheet(QStringLiteral("QLabel { color: %1; font-size: 11px; font-weight: 400; padding: 0px; background: transparent; }")
                                .arg(tokens::color(problem ? tokens::neg : tokens::ink3).name()));
    const int top = (m_titleBox->height() - 16) / 2;
    title->setGeometry(0, top, width, 16);
    // The smaller line sits on the title's baseline.
    subtitle->setGeometry(width + gap, top + 1, subRoom, 16);
    subtitle->setVisible(!sub.isEmpty());
}

bool ToolbarWiring::eventFilter(QObject *watched, QEvent *event) {
    const QEvent::Type type = event->type();
    if (watched == m_titleBox && type == QEvent::Resize) layoutTitle();
    if (watched == m_window->m_sidebar && (type == QEvent::Move || type == QEvent::Resize || type == QEvent::Show || type == QEvent::Hide))
        layoutLeading();
    return QObject::eventFilter(watched, event);
}

// ================================================================ export and preflight

QList<ToolbarWiring::PreflightRow> ToolbarWiring::preflightRows(const QJsonObject &summary, int least, int most) {
    const QJsonArray rows = summary.value(QStringLiteral("rows")).toArray();
    QList<int> chosen;
    for (int i = 0; i < rows.size() && chosen.size() < most; ++i)
        if (!rows.at(i).toObject().value(QStringLiteral("passed")).toBool(true)) chosen << i;
    for (int i = 0; i < rows.size() && chosen.size() < least; ++i)
        if (rows.at(i).toObject().value(QStringLiteral("passed")).toBool(true) && !chosen.contains(i)) chosen << i;
    std::sort(chosen.begin(), chosen.end());
    QList<PreflightRow> out;
    for (int index : std::as_const(chosen)) {
        const QJsonObject row = rows.at(index).toObject();
        PreflightRow shown;
        shown.code = row.value(QStringLiteral("code")).toString();
        const QString name = row.value(QStringLiteral("name")).toString(shown.code);
        const QString value = row.value(QStringLiteral("value")).toString();
        shown.label = value.isEmpty() ? name : QStringLiteral("%1 · %2").arg(name, value);
        shown.passed = row.value(QStringLiteral("passed")).toBool(true);
        shown.commentId = row.value(QStringLiteral("commentId")).toString();
        if (shown.passed) shown.key = row.value(QStringLiteral("limit")).toString();
        else shown.ask = askText(row);
        out << shown;
    }
    return out;
}

int ToolbarWiring::preflightHidden(const QJsonObject &summary, int most) {
    int failed = 0;
    for (const QJsonValue &value : summary.value(QStringLiteral("rows")).toArray())
        if (!value.toObject().value(QStringLiteral("passed")).toBool(true)) ++failed;
    return std::max(0, failed - most);
}

QString ToolbarWiring::askText(const QJsonObject &row) {
    const QString name = lowerFirst(row.value(QStringLiteral("name")).toString(row.value(QStringLiteral("code")).toString()));
    const QString value = row.value(QStringLiteral("value")).toString();
    const QString limit = row.value(QStringLiteral("limit")).toString();
    if (!limit.isEmpty())
        return value.isEmpty() ? QStringLiteral("@Kerf bring %1 to %2").arg(name, limit)
                               : QStringLiteral("@Kerf bring %1 to %2 (now %3)").arg(name, limit, value);
    return value.isEmpty() ? QStringLiteral("@Kerf fix the %1").arg(name) : QStringLiteral("@Kerf fix the %1 (%2)").arg(name, value);
}

QJsonArray ToolbarWiring::blockingRoots(const QJsonArray &comments) {
    QSet<QString> held;
    for (const QJsonValue &value : comments) {
        const QJsonObject comment = value.toObject();
        const QString parent = comment.value(QStringLiteral("parent")).toString();
        const bool marked = comment.value(QStringLiteral("blocking")).toBool() ||
                            comment.value(QStringLiteral("tags")).toArray().contains(QStringLiteral("blocking"));
        if (marked && !comment.value(QStringLiteral("resolved")).toBool())
            held.insert(parent.isEmpty() ? comment.value(QStringLiteral("id")).toString() : parent);
    }
    QJsonArray roots;
    for (const QJsonValue &value : comments) {
        const QJsonObject comment = value.toObject();
        if (comment.value(QStringLiteral("parent")).toString().isEmpty() && !comment.value(QStringLiteral("resolved")).toBool() &&
            held.contains(comment.value(QStringLiteral("id")).toString()))
            roots.append(comment);
    }
    return roots;
}

int ToolbarWiring::blockingCount() const {
    if (const ReviewModel *review = m_bus->review()) return review->counts().blocking;
    return blockingThreads(m_bus->lastEvent(QStringLiteral("comments")).value(QStringLiteral("items")).toArray());
}

bool ToolbarWiring::sendExport(const QString &quality) {
    const bool master = quality == QStringLiteral("master");
    if (master && blockingCount() > 0) {
        holdForNotes();
        return false;
    }
    // The master renders to the project's master output (its folder and
    // file name, kept for the project); the draft is a review copy.
    const QJsonObject command = master
        ? deliver::masterCommand(m_window->m_projectDirectory)
        : QJsonObject{{QStringLiteral("type"), QStringLiteral("export")},
                      {QStringLiteral("quality"), QStringLiteral("review")},
                      {QStringLiteral("codec"), QStringLiteral("h264")}};
    if (!m_bus->send(command)) return false;
    m_sentQuality = command.value(QStringLiteral("quality")).toString();
    m_window->addActivity(QStringLiteral("export"), QStringLiteral("Export started (%1)").arg(master ? QStringLiteral("Master") : QStringLiteral("Draft")));
    return true;
}

void ToolbarWiring::holdForNotes(const QString &refusal) {
    QString message = refusal;
    if (message.isEmpty()) {
        const QJsonArray roots = blockingRoots(m_bus->lastEvent(QStringLiteral("comments")).value(QStringLiteral("items")).toArray());
        const int count = std::max(blockingCount(), int(roots.size()));
        message = count == 1 ? QStringLiteral("A blocking note holds the master back")
                             : QStringLiteral("%1 blocking notes hold the master back").arg(count);
        if (!roots.isEmpty()) {
            const QJsonObject first = roots.first().toObject();
            QString body = first.value(QStringLiteral("body")).toString().simplified();
            if (body.size() > 80) body = body.left(79).trimmed() + QChar(0x2026);
            message += QStringLiteral(": \u201c%1\u201d by %2").arg(body, first.value(QStringLiteral("author")).toObject().value(QStringLiteral("name")).toString());
            if (const QJsonValue at = first.value(QStringLiteral("at")); at.isDouble())
                message += QStringLiteral(" at %1").arg(timelineTimecode(at.toDouble(), m_bus->timeline().value(QStringLiteral("fps")).toDouble(25.0)));
            if (count > 1) message += QStringLiteral(" and %1 more").arg(count - 1);
        }
        message += QStringLiteral(". Resolve it in Review, or export a Draft meanwhile.");
    }
    m_window->showError(message);
    // The way to resolve it: the blocking notes, in Review.
    if (ReviewModel *review = m_bus->review()) review->setTag(QString::fromLatin1(ReviewModel::kBlockingTag));
    m_window->addActivity(QStringLiteral("export"), QStringLiteral("Export held for blocking notes"));
}

void ToolbarWiring::buildExportMenu() {
    MainWindow *w = m_window;
    ExportButton *button = w->m_exportButton;
    if (!button) return;
    QMenu *menu = button->menu();
    menu->clear();
    menu->setToolTipsVisible(true);
    CommandRegistry *commands = w->m_commands;
    // The qualities, then Render preview.
    menu->addAction(button->masterAction());
    menu->addAction(button->draftAction());
    if (QAction *preview = commands->action(QStringLiteral("sequence.renderPreview"))) menu->addAction(preview);
    menu->addSeparator();

    // Preflight: the last check's rows.
    QAction *preflight = ui::addMenuHeader(menu, QStringLiteral("Preflight"));
    preflight->setObjectName(QStringLiteral("preflightHeader"));
    if (m_checkStatus == QStringLiteral("running")) {
        QAction *running = menu->addAction(QStringLiteral("Checking… %1%").arg(int(std::floor(m_checkProgress * 100.0))));
        running->setObjectName(QStringLiteral("preflightRunning"));
        running->setEnabled(false);
    }
    if (m_checkSummary.isEmpty() && m_checkStatus != QStringLiteral("running")) {
        QAction *never = menu->addAction(QStringLiteral("Not checked yet"));
        never->setObjectName(QStringLiteral("preflightRun"));
        ui::setMenuDot(never, tokens::color(tokens::dis));
        ui::setMenuAccent(never, QStringLiteral("Run QC"));
        never->setEnabled(commands->isEnabled(QStringLiteral("qc.run")));
        never->setToolTip(commands->reason(QStringLiteral("qc.run")));
        connect(never, &QAction::triggered, this, [this] { m_window->triggerCommand(QStringLiteral("qc.run")); });
    }
    if (m_checkStatus == QStringLiteral("failed")) {
        QAction *failed = menu->addAction(m_checkMessage.isEmpty() ? QStringLiteral("The last check failed")
                                                                   : QStringLiteral("The last check failed: %1").arg(m_checkMessage));
        failed->setObjectName(QStringLiteral("preflightFailed"));
        ui::setMenuRole(failed, ui::MenuRole::Warn);
        ui::setMenuAccent(failed, QStringLiteral("Run again"));
        failed->setEnabled(commands->isEnabled(QStringLiteral("qc.run")));
        connect(failed, &QAction::triggered, this, [this] { m_window->triggerCommand(QStringLiteral("qc.run")); });
    }
    for (const PreflightRow &row : preflightRows(m_checkSummary)) {
        QAction *item = menu->addAction(row.label);
        item->setObjectName(QStringLiteral("preflight.%1").arg(row.code));
        ui::setMenuDot(item, tokens::color(row.passed ? tokens::pos : tokens::neg));
        if (row.passed) {
            if (!row.key.isEmpty()) ui::setMenuKey(item, row.key, true);
        } else {
            ui::setMenuAccent(item, QStringLiteral("Ask Kerf"));
            item->setToolTip(row.ask);
            const QString text = row.ask;
            const QString thread = row.commentId;
            connect(item, &QAction::triggered, this, [this, text, thread] {
                if (!thread.isEmpty()) emit m_bus->openThread(thread);
                emit m_bus->askPrefill(text, thread);
            });
        }
    }
    if (const int hidden = preflightHidden(m_checkSummary); hidden > 0) {
        QAction *more = menu->addAction(plural(hidden, QStringLiteral("1 more check failed"), QStringLiteral("%1 more checks failed")));
        more->setObjectName(QStringLiteral("preflightMore"));
        ui::setMenuDot(more, tokens::color(tokens::neg));
    }
    // The blocking notes, which hold Render master back.
    const int blocking = blockingCount();
    QAction *notes = menu->addAction(blocking == 0 ? QStringLiteral("No blocking notes open")
                                                   : plural(blocking, QStringLiteral("1 blocking note open"), QStringLiteral("%1 blocking notes open")));
    notes->setObjectName(QStringLiteral("preflightBlocking"));
    ui::setMenuDot(notes, tokens::color(blocking > 0 ? tokens::neg : tokens::pos));
    if (blocking > 0) ui::setMenuRole(notes, ui::MenuRole::Warn);
    connect(notes, &QAction::triggered, this, [this, blocking] {
        if (ReviewModel *review = m_bus->review(); review && blocking > 0) review->setTag(QString::fromLatin1(ReviewModel::kBlockingTag));
    });
    menu->addSeparator();

    QAction *master = commands->action(QStringLiteral("export.master"));
    QAction *render = menu->addAction(QStringLiteral("Render master"));
    render->setObjectName(QStringLiteral("exportRenderMaster"));
    const bool available = master && master->isEnabled();
    render->setEnabled(available && blocking == 0);
    render->setToolTip(!available ? commands->reason(QStringLiteral("export.master"))
                       : blocking > 0 ? QStringLiteral("Resolve blocking notes first")
                                      : QStringLiteral("Render master · ProRes 422 HQ"));
    connect(render, &QAction::triggered, this, [this] { sendExport(QStringLiteral("master")); });
}

// ================================================================ the guest's view

void ToolbarWiring::followReview() {
    ReviewModel *review = m_bus->review();
    if (m_review == review) return;
    if (m_review) disconnect(m_review, nullptr, this, nullptr);
    m_review = review;
    if (review) connect(review, &ReviewModel::changed, this, &ToolbarWiring::layoutTitle);
    layoutTitle();
}

void ToolbarWiring::buildKeymapMenu(QMenu *menu) {
    menu->clear();
    qDeleteAll(menu->findChildren<QActionGroup *>(Qt::FindDirectChildrenOnly));
    CommandRegistry *commands = m_window->m_commands;
    auto *group = new QActionGroup(menu);
    for (const QJsonValue &value : commands->keymaps()) {
        const QJsonObject keymap = value.toObject();
        const QString id = keymap.value(QStringLiteral("id")).toString();
        QAction *choice = menu->addAction(keymap.value(QStringLiteral("label")).toString());
        choice->setObjectName(QStringLiteral("keymapChoice.%1").arg(id));
        choice->setCheckable(true);
        choice->setChecked(id == commands->keymap());
        group->addAction(choice);
        connect(choice, &QAction::triggered, this, [this, id] {
            m_window->m_commands->setKeymap(id);
            QSettings().setValue(QStringLiteral("commands/keymap"), id);
            // Edit ▸ Keyboard Shortcuts checks the same choice.
            for (QAction *other : m_window->findChildren<QAction *>())
                if (other->objectName().startsWith(QStringLiteral("keymap.")))
                    other->setChecked(other->objectName() == QStringLiteral("keymap.%1").arg(id));
        });
    }
    menu->setToolTipsVisible(true);
}

QString ToolbarWiring::syncSummary() const {
    const QJsonObject &status = m_window->m_collabStatus;
    const QString revision = QStringLiteral("Revision %1").arg(m_bus->revision());
    if (!status.value(QStringLiteral("repository")).toBool()) return QStringLiteral("%1 · not shared yet").arg(revision);
    const int behind = std::max(0, status.value(QStringLiteral("behind")).toInt());
    const int ahead = std::max(0, status.value(QStringLiteral("ahead")).toInt());
    QStringList parts{revision};
    if (behind > 0) parts << plural(behind, QStringLiteral("1 commit to pull"), QStringLiteral("%1 commits to pull"));
    if (ahead > 0) parts << plural(ahead, QStringLiteral("1 commit to push"), QStringLiteral("%1 commits to push"));
    if (behind == 0 && ahead == 0) parts << QStringLiteral("up to date");
    return parts.join(QStringLiteral(" · "));
}

void ToolbarWiring::buildSyncMenu(QMenu *menu) {
    MainWindow *w = m_window;
    menu->clear();
    qDeleteAll(menu->findChildren<QActionGroup *>(Qt::FindDirectChildrenOnly));
    CommandRegistry *commands = w->m_commands;
    const QJsonObject &status = w->m_collabStatus;
    QAction *header = ui::addMenuHeader(menu, syncSummary());
    header->setObjectName(QStringLiteral("syncHeader"));
    // What went wrong, when something did.
    const QString problem = status.value(QStringLiteral("conflicted")).toBool()
        ? QStringLiteral("A merge is unfinished; finish it with git")
        : w->m_syncFailure.isEmpty() ? QString() : QStringLiteral("Sync failed: %1").arg(w->m_syncFailure);
    if (!problem.isEmpty()) {
        QAction *warn = menu->addAction(problem);
        warn->setObjectName(QStringLiteral("syncProblem"));
        warn->setEnabled(false);
        ui::setMenuRole(warn, ui::MenuRole::Warn);
    }
    const auto add = [&](const char *id) {
        if (QAction *action = commands->action(QString::fromLatin1(id))) menu->addAction(action);
    };
    if (!status.value(QStringLiteral("repository")).toBool()) {
        add("collab.publish");
        menu->setToolTipsVisible(true);
        return;
    }
    // The branches, this one checked; another switches to it.
    const QString current = status.value(QStringLiteral("branch")).toString();
    auto *group = new QActionGroup(menu);
    QStringList branches;
    if (!current.isEmpty()) branches << current;
    for (const QJsonValue &value : status.value(QStringLiteral("branches")).toArray())
        if (const QString branch = value.toString(); !branch.isEmpty() && !branches.contains(branch)) branches << branch;
    for (const QString &branch : std::as_const(branches)) {
        QAction *choice = menu->addAction(branch);
        choice->setObjectName(QStringLiteral("syncBranch.%1").arg(branch));
        choice->setCheckable(true);
        choice->setChecked(branch == current);
        group->addAction(choice);
        if (branch == current) continue;
        choice->setEnabled(commands->isEnabled(QStringLiteral("collab.switchBranch")));
        choice->setToolTip(commands->reason(QStringLiteral("collab.switchBranch")));
        connect(choice, &QAction::triggered, this, [this, branch] {
            m_window->execute(QStringLiteral("collab.switchBranch"), QJsonObject{{QStringLiteral("name"), branch}});
        });
    }
    add("collab.newBranch");
    menu->addSeparator();
    add("collab.sync");
    add("collab.live");
    add("collab.commit");
    add("collab.pullRequest");
    menu->setToolTipsVisible(true);
}

// ================================================================ events

void ToolbarWiring::trackExport(const QString &job, const QString &quality, const QString &name, double fraction, bool queued) {
    auto it = m_runningExports.find(job);
    if (it == m_runningExports.end()) it = m_runningExports.insert(job, RunningExport{{}, 0, ++m_exportsStarted});
    // A progress event may leave the name out: the job keeps the one it had.
    if (quality == QStringLiteral("review") || quality == QStringLiteral("preview")) it->label = QStringLiteral("Draft export");
    else if (quality == QStringLiteral("master") && name.isEmpty()) it->label = QStringLiteral("Master export");
    else if (!name.isEmpty()) it->label = name;
    else if (it->label.isEmpty()) it->label = QStringLiteral("Export");
    it->fraction = fraction;
    it->queued = queued;
}

void ToolbarWiring::onSnapshot() {
    // The window is named as the toolbar names it, once the sequence is known.
    m_window->setWindowTitle(titleText());
    followCut();
    layoutTitle();
}

void ToolbarWiring::onEvent(const QString &type, const QJsonObject &event) {
    const QString status = event.value(QStringLiteral("status")).toString();
    if (type == QStringLiteral("check")) {
        if (status == QStringLiteral("running")) {
            m_checkStatus = status;
            m_checkProgress = event.value(QStringLiteral("progress")).toDouble();
        } else if (status == QStringLiteral("done") || event.contains(QStringLiteral("summary"))) {
            m_checkStatus = QStringLiteral("done");
            m_checkSummary = event.value(QStringLiteral("summary")).toObject();
            m_checkMessage.clear();
        } else if (status == QStringLiteral("failed")) {
            m_checkStatus = status;
            m_checkMessage = event.value(QStringLiteral("message")).toString();
        }
    } else if (type == QStringLiteral("export")) {
        const QString kind = event.value(QStringLiteral("kind")).toString();
        if (!kind.isEmpty() && kind != QStringLiteral("render")) return; // paperwork and captions are no renders
        const QString job = event.value(QStringLiteral("jobId")).toString();
        QString quality = event.value(QStringLiteral("quality")).toString();
        if (quality.isEmpty()) quality = m_jobQuality.value(job, m_sentQuality);
        if (status == QStringLiteral("running") || status == QStringLiteral("queued")) {
            trackExport(job, quality, event.value(QStringLiteral("name")).toString(), event.value(QStringLiteral("progress")).toDouble(),
                        status == QStringLiteral("queued"));
        } else if (!status.isEmpty()) {
            m_runningExports.remove(job);
        }
        layoutTitle();
    } else if (type == QStringLiteral("render_queue")) {
        // The whole queue: what it no longer runs has stopped.
        QSet<QString> running;
        for (const QJsonValue &value : event.value(QStringLiteral("jobs")).toArray()) {
            const QJsonObject job = value.toObject();
            const QString id = job.value(QStringLiteral("jobId")).toString();
            const QString quality = job.value(QStringLiteral("quality")).toString();
            m_jobQuality.insert(id, quality);
            const QString state = job.value(QStringLiteral("status")).toString();
            if (state == QStringLiteral("running") || state == QStringLiteral("queued")) {
                running.insert(id);
                trackExport(id, quality, job.value(QStringLiteral("name")).toString(), job.value(QStringLiteral("progress")).toDouble(),
                            state == QStringLiteral("queued"));
            }
        }
        for (auto it = m_runningExports.begin(); it != m_runningExports.end();)
            it = running.contains(it.key()) ? std::next(it) : m_runningExports.erase(it);
        layoutTitle();
    } else if (type == QStringLiteral("preview")) {
        m_previewRunning = status == QStringLiteral("running");
        m_previewFraction = event.value(QStringLiteral("progress")).toDouble();
        if (!status.isEmpty()) layoutTitle();
    }
}

} // namespace kerf
