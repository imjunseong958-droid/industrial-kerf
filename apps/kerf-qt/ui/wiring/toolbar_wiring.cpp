#include "toolbar_wiring.h"

#include "chat_panel.h"
#include "command_registry.h"
#include "composer.h"
#include "keymap_editor.h"
#include "sequence_settings.h"
#include "components.h"
#include "editor_bus.h"
#include "editor_common.h"
#include "main_window.h"
#include "main_window_shared.h"
#include "render_queue_view.h"
#include "review_model.h"
#include "toolbar_widgets.h"
#include "stage_shell.h"
#include "wiring/stage_wiring.h"
#include "wiring/bin_wiring.h"
#include "wiring/source_wiring.h"

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
    row->setContentsMargins(16, 0, 12, 0);
    row->setSpacing(4);
#if KERF_EDITOR_OWN_TITLE_BAR
    // The room the window buttons take; fitTitleBar sizes it.
    w->m_windowButtons = new QWidget;
    w->m_windowButtons->setObjectName(QStringLiteral("windowButtonsSpace"));
    w->m_windowButtons->setFixedWidth(firstControlX(0, false) - 16 - row->spacing());
    row->addWidget(w->m_windowButtons);
    // The toolbar's 4 gap and the ≡ button's own 4 past the lights' room
    // (mockup #appBtn).
    row->addSpacing(8);
#endif
    // The one way to everything else: Media, Effects and titles, Inspector,
    // the trays, Import, Sequence settings, Settings, and View.
    m_appButton = new ui::IconButton(QStringLiteral("g-menu"), ui::IconButton::Size::S28);
    m_appButton->setObjectName(QStringLiteral("appMenu"));
    m_appButton->setAccessibleName(QStringLiteral("Menu"));
    m_appButton->setTip(QStringLiteral("Media, effects, inspector, finish, deliver"));
    m_appButton->setFocusPolicy(Qt::TabFocus);
    m_appMenu = new QMenu(m_content);
    m_appMenu->setObjectName(QStringLiteral("appMenuPopup"));
    ui::styleMenu(m_appMenu);
    m_menus = menus;
    connect(m_appButton, &QAbstractButton::clicked, this, [this] {
        buildAppMenu();
        ui::popupMenu(m_appMenu, m_appButton);
    });
    row->addWidget(m_appButton);
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
    if (!w->menuBar()->isNativeMenuBar()) {
        // The native menus live in the ≡ menu; their keys go on working.
        for (const QString &name : w->m_commands->menuNames()) {
            QMenu *menu = menus.value(name);
            if (!menu) continue;
            for (QAction *action : menu->actions())
                if (action->shortcutContext() != Qt::WidgetShortcut) w->addAction(action);
        }
        w->menuBar()->hide();
    }
    row->addStretch(1);

    // The right group: the panel's panes, the format, then Export.
    w->m_followButton = paneButton(QStringLiteral("g-eye"), "paneFollowKerf", QStringLiteral("Follow Kerf"), QStringLiteral("view.followKerf"));
    w->m_followButton->setOn(w->m_followKerf);
    row->addWidget(w->m_followButton);
    w->m_pictureButton = paneButton(QStringLiteral("g-picture"), "panePicture", QStringLiteral("Picture"), QStringLiteral("view.picture"));
    row->addWidget(w->m_pictureButton);
    w->m_timelineButton = paneButton(QStringLiteral("g-timeline"), "paneTimeline", QStringLiteral("Timeline"), QStringLiteral("view.timeline"));
    row->addWidget(w->m_timelineButton);
    row->addSpacing(12 - row->spacing());
    w->m_formatBank = new FormatBank;
    connect(w->m_formatBank, &FormatBank::formatChosen, w, &MainWindow::canvasPresetChanged);
    w->m_formatBank->installEventFilter(this);
    row->addWidget(w->m_formatBank);
    row->addSpacing(12 - row->spacing());
    w->m_exportButton = new ExportButton;
    connect(w->m_exportButton, &ExportButton::exportRequested, this, &ToolbarWiring::sendExport);
    connect(w->m_exportButton, &ExportButton::menuAboutToShow, this, &ToolbarWiring::buildExportMenu);
    connect(w->m_exportButton, &ExportButton::qualityChanged, this, &ToolbarWiring::syncCommands);
    row->addWidget(w->m_exportButton);
    toolbar->setContent(m_content);

    // One's own keys go on before anything reads the registry's (and back
    // on whenever it loads a keymap).
    m_customiser = new KeymapCustomiser(w->m_commands, this);
    // ⌥1-2 choose the format, as its segments do: the registry's commands,
    // whose keys work anywhere in the window.
    for (const QString &id : formatCommands())
        if (QAction *action = w->m_commands->action(id)) {
            if (!w->actions().contains(action)) w->addAction(action);
            m_formatActions << action;
        }
    installCommands();

    // The title, centred on the window over the toolbar's row; layoutTitle
    // places it as the toolbar and its groups move.
    w->m_projectTitle = new QLabel(toolbar);
    w->m_projectTitle->setObjectName(QStringLiteral("projectTitle"));
    w->m_projectSubtitle = new QLabel(toolbar);
    w->m_projectSubtitle->setObjectName(QStringLiteral("projectSubtitle"));
    // Its own sheet, so the app's QLabel rules leave its type alone:
    // UI 500 13/16 ink over UI 11/16 ink3.
    w->m_projectTitle->setStyleSheet(QStringLiteral("QLabel { color: %1; font-size: 13px; font-weight: 500; padding: 0px; "
                                                    "background: transparent; }")
                                         .arg(tokens::color(tokens::ink).name()));
    w->m_projectSubtitle->setStyleSheet(QStringLiteral("QLabel { color: %1; font-size: 11px; font-weight: 400; padding: 0px; "
                                                       "background: transparent; }")
                                            .arg(tokens::color(tokens::ink3).name()));
    for (QLabel *label : {w->m_projectTitle, w->m_projectSubtitle}) {
        label->setFont(tokens::font(tokens::TypeStyle{label == w->m_projectTitle ? 13 : 11, 16,
                                                      label == w->m_projectTitle ? 500 : 400, false}));
        label->setAlignment(Qt::AlignCenter);
        label->setAttribute(Qt::WA_TransparentForMouseEvents);
    }
    // "Edited 2 min ago" moves on by itself.
    m_titleTimer = new QTimer(this);
    m_titleTimer->setInterval(30'000);
    connect(m_titleTimer, &QTimer::timeout, this, &ToolbarWiring::layoutTitle);
    m_titleTimer->start();

    connect(m_bus, &EditorBus::event, this, &ToolbarWiring::onEvent);
    connect(m_bus, &EditorBus::snapshotChanged, this, &ToolbarWiring::onSnapshot);
    connect(w->m_commands, &CommandRegistry::shortcutsChanged, this, &ToolbarWiring::syncCommands);
    connect(m_bus, &EditorBus::viewAsChanged, this, &ToolbarWiring::layoutTitle);
    connect(m_bus, &EditorBus::reviewChanged, this, &ToolbarWiring::followReview);
    followReview();
    if (QAction *master = w->m_commands->action(QStringLiteral("export.master")))
        connect(master, &QAction::changed, this, &ToolbarWiring::syncCommands);
    syncCommands();
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
                                           {w->m_pictureButton, QStringLiteral("view.picture"), QStringLiteral("Picture")},
                                           {w->m_timelineButton, QStringLiteral("view.timeline"), QStringLiteral("Timeline")},
                                           {w->m_followButton, QStringLiteral("view.followKerf"), QStringLiteral("Follow Kerf")}})
        if (button) button->setTip(name, nativeKeys(commands->action(id)));
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
    w->m_commandHandlers.insert(QStringLiteral("sequence.settings"), [this] { openSequenceSettings(); });
    w->m_commandHandlers.insert(QStringLiteral("keyboard.customise"), [this] { openKeymapEditor(); });
    w->m_commandHandlers.insert(QStringLiteral("composer.draw"), [this] { drawInComposer(); });
    if (Composer *composer = w->m_chat ? w->m_chat->composerWidget() : nullptr)
        composer->setDrawAction(w->m_commands->action(QStringLiteral("composer.draw")));
    if (w->m_chat) w->m_chat->setAcceptAction(w->m_commands->action(QStringLiteral("proposal.accept")));
    // The bin's and the source's commands, now that the registry is there.
    if (w->m_binWiring) w->m_binWiring->attachCommands();
    if (w->m_sourceWiring) w->m_sourceWiring->attachCommands();
}

bool ToolbarWiring::drawInComposer() {
    Composer *composer = m_window->m_chat ? m_window->m_chat->composerWidget() : nullptr;
    if (!composer || !composer->hasFocusWithin()) return false;
    composer->requestDraw();
    return true;
}

SequenceSettingsDialog *ToolbarWiring::openSequenceSettings() {
    MainWindow *w = m_window;
    // A dialog closed but not yet deleted starts over from the sequence.
    if (m_sequenceSettings && !m_sequenceSettings->isVisible()) {
        m_sequenceSettings->disconnect(this);
        m_sequenceSettings->deleteLater();
        m_sequenceSettings = nullptr;
    }
    if (!m_sequenceSettings) {
        auto *dialog = new SequenceSettingsDialog(m_bus->timeline(), w);
        dialog->setAttribute(Qt::WA_DeleteOnClose);
        connect(dialog, &SequenceSettingsDialog::applyRequested, this, &ToolbarWiring::applySequenceSettings);
        connect(dialog, &QDialog::finished, this, [this] { m_sequencePending = false; });
        m_sequenceSettings = dialog;
    } else {
        m_sequenceSettings->setTimeline(m_bus->timeline());
    }
    // Window-modal on macOS (a sheet), so the editor stays as it is.
    m_sequenceSettings->setWindowModality(Qt::WindowModal);
    m_sequenceSettings->show();
    m_sequenceSettings->raise();
    m_sequenceSettings->activateWindow();
    return m_sequenceSettings;
}

bool ToolbarWiring::applySequenceSettings(const QJsonObject &op) {
    MainWindow *w = m_window;
    if (op.isEmpty()) return false;
    if (!w->m_connected) {
        if (m_sequenceSettings) m_sequenceSettings->setError(QStringLiteral("Waiting for the project connection"));
        return false;
    }
    m_sequenceRevision = m_bus->revision();
    m_sequenceOp = op;
    m_sequencePending = w->sendEdit(QJsonArray{op});
    if (!m_sequencePending && m_sequenceSettings) m_sequenceSettings->setError(QStringLiteral("The Kerf engine is not connected"));
    return m_sequencePending;
}

SequenceSettingsDialog *ToolbarWiring::sequenceSettings() const {
    return m_sequenceSettings;
}

KeymapEditor *ToolbarWiring::keymapEditor() const {
    return m_keymapEditor;
}

KeymapEditor *ToolbarWiring::openKeymapEditor() {
    if (!m_keymapEditor) {
        auto *editor = new KeymapEditor(m_customiser, m_window);
        editor->setAttribute(Qt::WA_DeleteOnClose);
        m_keymapEditor = editor;
    }
    m_keymapEditor->show();
    m_keymapEditor->raise();
    m_keymapEditor->activateWindow();
    return m_keymapEditor;
}

void ToolbarWiring::chooseFormat(int index) {
    FormatBank *bank = m_window->m_formatBank;
    if (!bank || !bank->isEnabled() || index < 0 || index >= bank->count()) return;
    bank->setCurrentIndex(index);
    emit bank->activated(index);
}

// ================================================================ the ≡ menu

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
    addCommandItem(menu, QStringLiteral("window.project"), QStringLiteral("Media"), "appMenu.media");
    addCommandItem(menu, QStringLiteral("window.effects"), QStringLiteral("Effects and titles"), "appMenu.effects");
    addCommandItem(menu, QStringLiteral("window.inspector"), QStringLiteral("Inspector"), "appMenu.inspector");
    menu->addSeparator();
    addCommandItem(menu, QStringLiteral("view.sidebar"), QStringLiteral("Sidebar"), "appMenu.sidebar");
    addCommandItem(menu, QStringLiteral("view.picture"), QStringLiteral("Picture"), "appMenu.picture");
    addCommandItem(menu, QStringLiteral("view.timeline"), QStringLiteral("Timeline"), "appMenu.timeline");
    addCommandItem(menu, QStringLiteral("view.followKerf"), QStringLiteral("Follow Kerf"), "appMenu.followKerf");
    menu->addSeparator();
    addCommandItem(menu, QStringLiteral("workspace.finish"), QStringLiteral("Scopes, loudness, QC"), "appMenu.finish");
    addCommandItem(menu, QStringLiteral("workspace.deliver"), QStringLiteral("Deliver"), "appMenu.deliver");
    menu->addSeparator();
    addCommandItem(menu, QStringLiteral("file.import"), QStringLiteral("Import…"), "appMenu.import");
    addCommandItem(menu, QStringLiteral("sequence.settings"), QStringLiteral("Sequence settings…"), "appMenu.sequenceSettings");
    addCommandItem(menu, QStringLiteral("app.preferences"), QStringLiteral("Settings…"), "appMenu.settings");

    // The picture's switches, the keyboard's choice and the sync.
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
    const ReviewModel *review = m_bus->review();
    const bool guest = review ? review->guestMode() : !m_bus->viewAs().isEmpty();
    if (guest) {
        const QString name = review ? review->viewAsName() : m_bus->viewAs();
        line += QStringLiteral(" · Viewing as %1").arg(name.isEmpty() ? m_bus->viewAs() : name);
    }
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

QString ToolbarWiring::subtitleText() const {
    if (!m_message.isEmpty()) return m_message;
    if (const QString problem = standingProblem(); !problem.isEmpty()) return problem;
    if (const QString running = runningText(); !running.isEmpty()) return running;
    return normalSubtitle();
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
    QToolBar *toolbar = w->m_toolbar;
    if (!title || !subtitle || !toolbar || !m_appButton || !w->m_formatBank) return;
    title->ensurePolished();
    subtitle->ensurePolished();
    const QString full = titleText();
    const QString sub = subtitleText();
    const QFontMetrics metrics(title->font());
    const QFontMetrics subMetrics(subtitle->font());
    // The room between the groups, and the title's own width.
    const QWidget *leftEnd = w->m_sidebarButton ? static_cast<QWidget *>(w->m_sidebarButton) : m_appButton;
    const QWidget *rightStart = w->m_followButton ? static_cast<QWidget *>(w->m_followButton) : w->m_formatBank;
    const int left = leftEnd->mapTo(toolbar, QPoint(leftEnd->width(), 0)).x() + kTitleClearance;
    const int right = rightStart->mapTo(toolbar, QPoint(0, 0)).x() - kTitleClearance;
    const int natural = std::max(metrics.horizontalAdvance(full), subMetrics.horizontalAdvance(sub)) + 2;
    const int width = std::min(natural, std::max(0, right - left));
    // Centred on the window (the toolbar spans it); moved aside when a group
    // is in the way, and shortened when there is no room for all of it.
    const int x = std::clamp((toolbar->width() - width) / 2, left, std::max(left, right - width));
    const QString shown = metrics.horizontalAdvance(full) + 2 > width ? metrics.elidedText(full, Qt::ElideRight, width) : full;
    title->setText(shown);
    title->setToolTip(shown == full ? QString() : full);
    subtitle->setText(subMetrics.elidedText(sub, Qt::ElideRight, width));
    // The whole of a long message, and an offline or error state in its colour.
    subtitle->setToolTip(subtitle->text() == sub ? QString() : sub);
    const bool problem = m_message.isEmpty() && !standingProblem().isEmpty();
    subtitle->setProperty("problem", problem);
    subtitle->setStyleSheet(QStringLiteral("QLabel { color: %1; font-size: 11px; font-weight: 400; padding: 0px; background: transparent; }")
                                .arg(tokens::color(problem ? tokens::neg : tokens::ink3).name()));
    // Two 16 pt lines from 4 pt down (mockup-spec 1), on a taller title bar
    // centred as the controls are.
    const int top = 4 + std::max(0, (toolbar->height() - kToolbarHeight) / 2);
    title->setGeometry(x, top, width, 16);
    subtitle->setGeometry(x, top + 16, width, 16);
    subtitle->setVisible(!sub.isEmpty());
    title->raise();
    subtitle->raise();
}

bool ToolbarWiring::eventFilter(QObject *watched, QEvent *event) {
    const QEvent::Type type = event->type();
    if (watched == m_window->m_formatBank && (type == QEvent::Move || type == QEvent::Resize)) layoutTitle();
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

int ToolbarWiring::blockingThreads(const QJsonArray &comments) {
    QHash<QString, bool> rootOpen;
    QHash<QString, bool> blocking;
    for (const QJsonValue &value : comments) {
        const QJsonObject comment = value.toObject();
        const QString id = comment.value(QStringLiteral("id")).toString();
        const QString parent = comment.value(QStringLiteral("parent")).toString();
        const QString root = parent.isEmpty() ? id : parent;
        const bool resolved = comment.value(QStringLiteral("resolved")).toBool();
        if (parent.isEmpty()) rootOpen.insert(root, !resolved);
        const bool marked = comment.value(QStringLiteral("blocking")).toBool() ||
                            comment.value(QStringLiteral("tags")).toArray().contains(QStringLiteral("blocking"));
        if (marked && !resolved) blocking.insert(root, true);
    }
    int count = 0;
    for (auto it = blocking.cbegin(); it != blocking.cend(); ++it)
        if (rootOpen.value(it.key(), false)) ++count;
    return count;
}

int ToolbarWiring::blockingCount() const {
    if (const ReviewModel *review = m_bus->review()) return review->counts().blocking;
    return blockingThreads(m_bus->lastEvent(QStringLiteral("comments")).value(QStringLiteral("items")).toArray());
}

bool ToolbarWiring::sendExport(const QString &quality) {
    const bool master = quality == QStringLiteral("master");
    // The master renders as the Deliver tray's Master row does (its folder
    // and file name, kept for the project); the draft is a review copy.
    const QJsonObject command = master
        ? deliver::exportCommand(deliver::output(m_window->m_projectDirectory, QStringLiteral("master")))
        : QJsonObject{{QStringLiteral("type"), QStringLiteral("export")},
                      {QStringLiteral("quality"), QStringLiteral("review")},
                      {QStringLiteral("codec"), QStringLiteral("h264")}};
    if (!m_bus->send(command)) return false;
    m_sentQuality = command.value(QStringLiteral("quality")).toString();
    m_window->addActivity(QStringLiteral("export"), QStringLiteral("Export started (%1)").arg(master ? QStringLiteral("Master") : QStringLiteral("Draft")));
    return true;
}

QString ToolbarWiring::edlLocatorsKey() const { return deliver::settingsKey(m_window->m_projectDirectory, QStringLiteral("edlLocators")); }

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
    const auto openQc = [this] {
        m_window->openTray(QStringLiteral("Finish"));
        emit m_bus->highlightPane(QStringLiteral("qc"));
    };
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
            item->setToolTip(QStringLiteral("Show the QC pane"));
            connect(item, &QAction::triggered, this, openQc);
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
        ui::setMenuAccent(more, QStringLiteral("Open QC"));
        connect(more, &QAction::triggered, this, openQc);
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
    QAction *locators = menu->addAction(QStringLiteral("Include comments as EDL locators"));
    locators->setObjectName(QStringLiteral("exportEdlLocators"));
    locators->setCheckable(true);
    // The Deliver paperwork's own option, so the EDL it writes follows this
    // row and the row shows what Deliver will do. Without a project there is
    // nothing to keep it for.
    const QString directory = w->m_projectDirectory;
    const bool kept = !edlLocatorsKey().isEmpty();
    locators->setEnabled(kept);
    locators->setChecked(deliver::option(directory, QStringLiteral("edlLocators")));
    if (!kept) locators->setToolTip(QStringLiteral("Open a project to keep this choice"));
    connect(locators, &QAction::toggled, this, [directory](bool on) { deliver::setOption(directory, QStringLiteral("edlLocators"), on); });
    QAction *deliver = menu->addAction(QStringLiteral("Open Deliver"));
    deliver->setObjectName(QStringLiteral("exportOpenDeliver"));
    connect(deliver, &QAction::triggered, this, [this] { m_window->openTray(QStringLiteral("Deliver")); });
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
    menu->addSeparator();
    if (QAction *customise = commands->action(QStringLiteral("keyboard.customise"))) menu->addAction(customise);
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
    // Sequence Settings: kerfd took the change (a newer revision whose
    // sequence says what was asked), or the sequence changed under the open
    // dialog (someone else's edit: the request is still waiting).
    if (m_sequenceSettings) {
        if (m_sequencePending && m_bus->revision() > m_sequenceRevision &&
            SequenceSettingsDialog::holds(m_bus->timeline(), m_sequenceOp)) {
            m_sequencePending = false;
            m_sequenceSettings->accept();
        } else {
            m_sequenceSettings->setTimeline(m_bus->timeline());
        }
    }
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
    } else if (type == QStringLiteral("error") && m_sequencePending && m_sequenceSettings) {
        // kerfd refused the sequence's change: its words, in the dialog.
        const QString command = event.value(QStringLiteral("command")).toString();
        if (command.isEmpty() || command == QStringLiteral("edit")) {
            m_sequencePending = false;
            m_sequenceSettings->setError(event.value(QStringLiteral("message")).toString(QStringLiteral("Kerf could not change the sequence")));
        }
    }
}

} // namespace kerf
