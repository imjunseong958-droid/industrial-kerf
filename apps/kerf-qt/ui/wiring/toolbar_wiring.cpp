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
#include "workspace.h"
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
#include <QProgressBar>
#include <QRadioButton>
#include <QSet>
#include <QSettings>
#include <QStatusBar>
#include <QTimer>
#include <QToolButton>

#include <algorithm>
#include <cmath>
#include <utility>

namespace kerf {

using namespace window_parts;

namespace {

QString nativeKeys(const QAction *action) {
    return action ? action->shortcut().toString(QKeySequence::NativeText) : QString();
}

// The design's names for the toolbar's commands (mockup-spec 15).
struct ToolbarCommand {
    const char *id;
    const char *icon;
    const char *name;
};
constexpr ToolbarCommand kCommands[] = {
    {"file.import", "g-import", "Import media"},
    {"edit.undo", "g-undo", "Undo"},
    {"edit.redo", "g-redo", "Redo"},
    {"clip.split", "g-split", "Split at playhead"},
    {"clip.delete", "g-delete", "Delete"},
};

// The workspaces' tips (mockup-spec 15), in Workspace::builtIns() order.
QString workspaceTip(const QString &name) {
    if (name == QStringLiteral("Finish")) return QStringLiteral("Finish workspace · scopes, loudness, QC");
    if (name == QStringLiteral("Deliver")) return QStringLiteral("Deliver workspace · masters, preflight");
    return QStringLiteral("%1 workspace").arg(name);
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

    // The window's older status labels and the conversation's toolbar
    // switch, kept for the code that still writes to them, out of sight.
    m_retired = new QWidget(w);
    m_retired->setObjectName(QStringLiteral("retiredChrome"));
    m_retired->hide();

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
#endif
    if (!w->menuBar()->isNativeMenuBar()) {
        // Keep menus reachable without a row of their own.
        m_appMenu = new ui::IconButton(QStringLiteral("g-list"), ui::IconButton::Size::S28);
        m_appMenu->setObjectName(QStringLiteral("applicationMenuButton"));
        m_appMenu->setAccessibleName(QStringLiteral("Application menu"));
        m_appMenu->setTip(QStringLiteral("Application menu"));
        auto *applicationMenu = new QMenu(m_appMenu);
        for (const QString &name : w->m_commands->menuNames()) {
            QMenu *menu = menus.value(name);
            if (!menu) continue;
            applicationMenu->addMenu(menu);
            for (QAction *action : menu->actions())
                if (action->shortcutContext() != Qt::WidgetShortcut) w->addAction(action);
        }
        connect(m_appMenu, &QAbstractButton::clicked, this, [this, applicationMenu] {
            applicationMenu->popup(m_appMenu->mapToGlobal(QPoint(0, m_appMenu->height() + 4)));
        });
        row->addWidget(m_appMenu);
        w->menuBar()->hide();
    }
    // Import, Undo, Redo, Split, Delete: the registry's actions.
    for (const ToolbarCommand &command : kCommands) {
        const QString id = QString::fromLatin1(command.id);
        auto *button = new ui::IconButton(QString::fromLatin1(command.icon), ui::IconButton::Size::S28);
        button->setObjectName(QStringLiteral("toolbar.%1").arg(id));
        button->setAccessibleName(QString::fromLatin1(command.name));
        button->setFocusPolicy(Qt::TabFocus);
        connect(button, &QAbstractButton::clicked, this, [this, id] {
            if (QAction *action = m_window->m_commands->action(id); action && action->isEnabled()) action->trigger();
        });
        if (QAction *action = w->m_commands->action(id)) connect(action, &QAction::changed, this, &ToolbarWiring::syncCommands);
        m_commandButtons.append({button, id});
        row->addWidget(button);
    }
    row->addWidget(new ui::Separator(8));
    // The workspaces: Edit, Review, Finish, Deliver and the Workspaces menu.
    m_workspaces = new ToolbarSegments;
    m_workspaces->setObjectName(QStringLiteral("workspaceSegments"));
    m_workspaces->setAccessibleName(QStringLiteral("Workspaces"));
    for (const QString &name : Workspace::builtIns()) m_workspaces->addSegment(name, workspaceTip(name));
    m_workspaces->setMenuTail(QStringLiteral("Workspaces"));
    connect(m_workspaces, &ToolbarSegments::activated, this, [this](int index) {
        const QString name = Workspace::builtIns().value(index);
        if (!name.isEmpty() && name != m_window->m_workspace->current()) m_window->m_workspace->activate(name);
    });
    m_workspaceMenu = new QMenu(m_content);
    m_workspaceMenu->setObjectName(QStringLiteral("toolbarWorkspacesMenu"));
    ui::styleMenu(m_workspaceMenu);
    connect(m_workspaces, &ToolbarSegments::menuRequested, this, [this] {
        m_window->buildWorkspaceMenu(m_workspaceMenu);
        ui::popupMenu(m_workspaceMenu, m_workspaces->menuTail());
    });
    m_workspaces->installEventFilter(this);
    row->addWidget(m_workspaces);
    row->addStretch(1);

    // The right group: format, sequence, Export, Settings.
    w->m_formatBank = new FormatBank;
    connect(w->m_formatBank, &FormatBank::formatChosen, w, &MainWindow::canvasPresetChanged);
    w->m_formatBank->installEventFilter(this);
    row->addWidget(w->m_formatBank);
    row->addSpacing(8 - row->spacing());
    m_sequence = new ui::GhostButton;
    m_sequence->setObjectName(QStringLiteral("sequenceButton"));
    m_sequence->setMono(true);
    m_sequence->setControlHeight(28);
    m_sequence->setFocusPolicy(Qt::TabFocus);
    m_sequenceMenu = new QMenu(m_content);
    m_sequenceMenu->setObjectName(QStringLiteral("sequenceMenu"));
    ui::styleMenu(m_sequenceMenu);
    connect(m_sequence, &QAbstractButton::clicked, this, [this] {
        buildSequenceMenu(m_sequenceMenu);
        ui::popupMenu(m_sequenceMenu, m_sequence);
    });
    row->addWidget(m_sequence);
    row->addSpacing(12 - row->spacing());
    w->m_exportButton = new ExportButton;
    connect(w->m_exportButton, &ExportButton::exportRequested, this, &ToolbarWiring::sendExport);
    connect(w->m_exportButton, &ExportButton::menuAboutToShow, this, &ToolbarWiring::buildExportMenu);
    connect(w->m_exportButton, &ExportButton::qualityChanged, this, &ToolbarWiring::syncCommands);
    row->addWidget(w->m_exportButton);
    row->addSpacing(8 - row->spacing());
    m_settings = new ui::IconButton(QStringLiteral("g-settings"), ui::IconButton::Size::S28);
    m_settings->setObjectName(QStringLiteral("settingsButton"));
    m_settings->setAccessibleName(QStringLiteral("Settings"));
    m_settings->setFocusPolicy(Qt::TabFocus);
    connect(m_settings, &QAbstractButton::clicked, this, [this] { m_window->triggerCommand(QStringLiteral("app.preferences")); });
    row->addWidget(m_settings);
    toolbar->setContent(m_content);

    // The toolbar's old left end, which the title no longer reads.
    w->m_toolbarLeftEnd = new QToolButton(m_retired);
    w->m_toolbarLeftEnd->setObjectName(QStringLiteral("retiredToolbarLeftEnd"));

    // One's own keys go on before anything reads the registry's (and back
    // on whenever it loads a keymap).
    m_customiser = new KeymapCustomiser(w->m_commands, this);
    // ⌥1-4 choose the format, as its segments do: the registry's commands,
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
    connect(m_bus, &EditorBus::workspaceChanged, this, &ToolbarWiring::setWorkspace);
    connect(w->m_commands, &CommandRegistry::shortcutsChanged, this, [this] {
        syncCommands();
        updateKeymap();
    });
    if (QAction *master = w->m_commands->action(QStringLiteral("export.master")))
        connect(master, &QAction::changed, this, &ToolbarWiring::syncCommands);
    syncCommands();
    setWorkspace(w->m_workspace->current());
    updateSequence();
}

void ToolbarWiring::syncCommands() {
    MainWindow *w = m_window;
    if (!w->m_commands) return;
    CommandRegistry *commands = w->m_commands;
    for (const auto &[button, id] : std::as_const(m_commandButtons)) {
        QAction *action = commands->action(id);
        const bool enabled = action && action->isEnabled();
        QString name;
        for (const ToolbarCommand &command : kCommands)
            if (id == QLatin1String(command.id)) name = QString::fromLatin1(command.name);
        const QString reason = commands->reason(id);
        button->setEnabled(enabled);
        ui::setTip(button, enabled || reason.isEmpty() ? name : QStringLiteral("%1 · %2").arg(name, reason), nativeKeys(action));
    }
    if (m_workspaces) {
        const QStringList names = Workspace::builtIns();
        for (int i = 0; i < names.size(); ++i)
            m_workspaces->setSegmentTip(i, workspaceTip(names.at(i)),
                                        nativeKeys(commands->action(QStringLiteral("workspace.%1").arg(names.at(i).toLower()))));
    }
    if (w->m_exportButton) {
        QAction *master = commands->action(QStringLiteral("export.master"));
        const bool enabled = master && master->isEnabled();
        const bool isMaster = w->m_exportButton->exportQuality() == QStringLiteral("master");
        const QString name = isMaster ? QStringLiteral("Export master") : QStringLiteral("Export draft");
        const QString reason = commands->reason(QStringLiteral("export.master"));
        w->m_exportButton->setExportEnabled(enabled, enabled || reason.isEmpty() ? name : QStringLiteral("%1 · %2").arg(name, reason),
                                            isMaster ? nativeKeys(master) : QString());
    }
    // The format segments name the keys of the registry's ⌥1-4.
    if (FormatBank *bank = w->m_formatBank)
        for (int i = 0; i < m_formatActions.size() && i < bank->count(); ++i)
            bank->setSegmentTip(i, formatTip(i), nativeKeys(m_formatActions.at(i)));
    if (m_settings) ui::setTip(m_settings, QStringLiteral("Settings"), nativeKeys(commands->action(QStringLiteral("app.preferences"))));
    if (m_sequence) ui::setTip(m_sequence, QStringLiteral("Sequence settings"), nativeKeys(commands->action(QStringLiteral("sequence.settings"))));
}

QStringList ToolbarWiring::formatCommands() {
    return {QStringLiteral("sequence.setFormat.169"), QStringLiteral("sequence.setFormat.916"), QStringLiteral("sequence.setFormat.11"),
            QStringLiteral("sequence.setFormat.45")};
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

void ToolbarWiring::setWorkspace(const QString &name) {
    // One's own workspace turns no built-in on; its menu checks it.
    if (m_workspaces) m_workspaces->setCurrentIndex(Workspace::builtIns().indexOf(name));
}

void ToolbarWiring::chooseFormat(int index) {
    FormatBank *bank = m_window->m_formatBank;
    if (!bank || !bank->isEnabled() || index < 0 || index >= bank->count()) return;
    bank->setCurrentIndex(index);
    emit bank->activated(index);
}

void ToolbarWiring::updateSequence() {
    if (!m_sequence) return;
    const bool known = !m_bus->snapshot().isEmpty();
    // The window's format: the snapshot's, or the one just chosen.
    const QSize size = m_window->m_canvasSize;
    m_sequence->setText(known ? sequenceLabel(size, m_bus->fps()) : QStringLiteral("Sequence"));
    m_sequence->setFixedSize(m_sequence->sizeHint());
    m_sequence->setEnabled(known);
    m_sequence->update();
}

void ToolbarWiring::buildSequenceMenu(QMenu *menu) {
    menu->clear();
    qDeleteAll(menu->findChildren<QActionGroup *>(Qt::FindDirectChildrenOnly));
    const QSize size = m_window->m_canvasSize;
    const QString rate = frameRateLabel(m_bus->fps());
    // The colour space, only when the sequence carries one.
    QString colour;
    const QJsonObject timeline = m_bus->timeline();
    const QString id = timeline.value(QStringLiteral("sequence")).toString();
    for (const QJsonValue &value : timeline.value(QStringLiteral("sequences")).toArray()) {
        const QJsonObject sequence = value.toObject();
        if (sequence.value(QStringLiteral("id")).toString() != id) continue;
        for (const char *key : {"colorSpace", "colourSpace", "colorspace"})
            if (const QString found = sequence.value(QLatin1String(key)).toString(); !found.isEmpty()) colour = found;
    }
    QStringList header{QStringLiteral("Sequence"), QStringLiteral("%1×%2").arg(size.width()).arg(size.height())};
    if (!rate.isEmpty()) header << rate;
    if (!colour.isEmpty()) header << colour;
    QAction *title = ui::addMenuHeader(menu, header.join(QStringLiteral(" · ")));
    title->setObjectName(QStringLiteral("sequenceHeader"));
    // Frame size: the presets, through the format segments' path.
    auto *sizes = new QMenu(QStringLiteral("Frame size"), menu);
    sizes->setObjectName(QStringLiteral("frameSizeMenu"));
    ui::styleMenu(sizes);
    auto *group = new QActionGroup(menu);
    const int current = FormatBank::presetIndex(size);
    const QList<CanvasPreset> presets = canvasPresets();
    for (int i = 0; i < presets.size(); ++i) {
        QAction *choice = sizes->addAction(formatName(i));
        choice->setObjectName(QStringLiteral("frameSize.%1").arg(QString::fromLatin1(presets.at(i).ratio)));
        choice->setCheckable(true);
        choice->setChecked(i == current);
        choice->setEnabled(m_window->m_connected);
        ui::setMenuKey(choice, QStringLiteral("%1×%2").arg(presets.at(i).width).arg(presets.at(i).height));
        group->addAction(choice);
        connect(choice, &QAction::triggered, this, [this, i] { chooseFormat(i); });
    }
    QAction *frameSize = menu->addMenu(sizes);
    frameSize->setObjectName(QStringLiteral("sequenceFrameSize"));
    ui::setMenuKey(frameSize, QStringLiteral("%1×%2").arg(size.width()).arg(size.height()));
    // The rate changes in Sequence Settings, and only while the sequence is
    // empty; here it is information.
    QAction *frameRate = menu->addAction(QStringLiteral("Frame rate"));
    frameRate->setObjectName(QStringLiteral("sequenceFrameRate"));
    frameRate->setEnabled(false);
    frameRate->setToolTip(SequenceSettingsDialog::fromTimeline(timeline).empty ? QStringLiteral("Change it in Sequence Settings…")
                                                                                : SequenceSettingsDialog::rateNote());
    ui::setMenuKey(frameRate, rate);
    if (!colour.isEmpty()) {
        QAction *space = menu->addAction(QStringLiteral("Colour space"));
        space->setObjectName(QStringLiteral("sequenceColourSpace"));
        space->setEnabled(false);
        ui::setMenuKey(space, colour, true);
    }
    menu->addSeparator();
    if (QAction *settings = m_window->m_commands->action(QStringLiteral("sequence.settings"))) menu->addAction(settings);
    menu->setToolTipsVisible(true);
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

QString ToolbarWiring::subtitleText() const {
    const QJsonObject snapshot = m_bus->snapshot();
    if (snapshot.isEmpty()) return QString();
    const QString revision = QStringLiteral("rev %1").arg(m_bus->revision());
    const QDateTime when = QDateTime::fromString(snapshot.value(QStringLiteral("revisionTime")).toString(), Qt::ISODateWithMs);
    const QString ago = editedAgo(when, m_clock());
    return ago.isEmpty() ? revision : QStringLiteral("Edited %1 · %2").arg(ago, revision);
}

void ToolbarWiring::layoutTitle() {
    MainWindow *w = m_window;
    QLabel *title = w->m_projectTitle;
    QLabel *subtitle = w->m_projectSubtitle;
    QToolBar *toolbar = w->m_toolbar;
    if (!title || !subtitle || !toolbar || !m_workspaces || !w->m_formatBank) return;
    title->ensurePolished();
    subtitle->ensurePolished();
    const QString full = titleText();
    const QString sub = subtitleText();
    const QFontMetrics metrics(title->font());
    const QFontMetrics subMetrics(subtitle->font());
    // The room between the groups, and the title's own width.
    const int left = m_workspaces->mapTo(toolbar, QPoint(m_workspaces->width(), 0)).x() + kTitleClearance;
    const int right = w->m_formatBank->mapTo(toolbar, QPoint(0, 0)).x() - kTitleClearance;
    const int natural = std::max(metrics.horizontalAdvance(full), subMetrics.horizontalAdvance(sub)) + 2;
    const int width = std::min(natural, std::max(0, right - left));
    // Centred on the window (the toolbar spans it); moved aside when a group
    // is in the way, and shortened when there is no room for all of it.
    const int x = std::clamp((toolbar->width() - width) / 2, left, std::max(left, right - width));
    const QString shown = metrics.horizontalAdvance(full) + 2 > width ? metrics.elidedText(full, Qt::ElideRight, width) : full;
    title->setText(shown);
    title->setToolTip(shown == full ? QString() : full);
    subtitle->setText(subMetrics.elidedText(sub, Qt::ElideRight, width));
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
    if ((watched == m_workspaces || watched == m_window->m_formatBank) && (type == QEvent::Move || type == QEvent::Resize)) {
        layoutTitle();
    } else if (m_row && watched == m_window->statusBar() && type == QEvent::Resize) {
        m_row->setGeometry(m_window->statusBar()->rect());
        m_row->raise();
    }
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
        m_window->m_workspace->activate(QStringLiteral("Finish"));
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
                emit m_bus->commentsPanelRequested(true);
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
        emit m_bus->commentsPanelRequested(true);
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
    QAction *deliver = menu->addAction(QStringLiteral("Open Deliver workspace"));
    deliver->setObjectName(QStringLiteral("exportOpenDeliver"));
    connect(deliver, &QAction::triggered, this, [this] { m_window->m_workspace->activate(QStringLiteral("Deliver")); });
}

// ================================================================ status bar

void ToolbarWiring::buildStatusBar() {
    MainWindow *w = m_window;
    QStatusBar *bar = w->statusBar();
    bar->setSizeGripEnabled(false);
    // 24 points tall (mockup-spec 0.1), whatever its items ask for.
    bar->setFixedHeight(tokens::layout::status);
    if (!m_retired) {
        m_retired = new QWidget(w);
        m_retired->hide();
    }
    // The older device label, still written to by main_window.cpp; the row
    // below shows the design's items instead.
    w->m_statusDevice = new QLabel(m_retired);
    w->m_statusDevice->setObjectName(QStringLiteral("statusDevice"));
    w->m_statusDevice->hide();
    if (w->m_exportProgress) w->m_exportProgress->setParent(m_retired);

    m_row = new StatusRow(bar);
    m_row->setGeometry(bar->rect());
    bar->installEventFilter(this);
    connect(bar, &QStatusBar::messageChanged, m_row, &StatusRow::setMessage);

    m_selection = new StatusText;
    m_selection->setObjectName(QStringLiteral("statusSelectionText"));
    m_selection->setElidable(true);
    m_rate = new StatusText;
    m_rate->setObjectName(QStringLiteral("statusRate"));
    m_rate->setColor(tokens::ink3);
    m_guest = new StatusText;
    m_guest->setObjectName(QStringLiteral("statusGuest"));
    m_guest->setColor(tokens::ink);
    m_guest->hide();
    m_row->leftLayout()->addWidget(m_selection, 1);
    m_row->leftLayout()->addWidget(m_rate);
    m_row->leftLayout()->addWidget(m_guest);

    // On the right: progress, keymap, sync and the agent. The chips' 6 pt
    // padding sits in the 16 pt gaps (margin 0 -6).
    QHBoxLayout *right = m_row->rightLayout();
    right->setSpacing(0);
    m_progress = new StatusProgress;
    m_progress->hide();
    right->addWidget(m_progress);
    right->addSpacing(16 - 6);
    m_keymap = new StatusChip;
    m_keymap->setObjectName(QStringLiteral("keymapChip"));
    ui::setTip(m_keymap, QStringLiteral("Keyboard shortcuts"));
    m_keymapMenu = new QMenu(m_keymap);
    m_keymapMenu->setObjectName(QStringLiteral("keymapMenu"));
    ui::styleMenu(m_keymapMenu);
    connect(m_keymap, &QAbstractButton::clicked, this, [this] {
        buildKeymapMenu(m_keymapMenu);
        ui::popupMenu(m_keymapMenu, m_keymap);
    });
    right->addWidget(m_keymap);
    right->addSpacing(16 - 12);
    auto *sync = new StatusChip;
    sync->setObjectName(QStringLiteral("syncChip"));
    sync->setIconName(QStringLiteral("g-branch"));
    sync->setMono(true);
    sync->hide();
    w->m_syncChip = sync;
    m_syncMenu = new QMenu(sync);
    m_syncMenu->setObjectName(QStringLiteral("syncMenu"));
    ui::styleMenu(m_syncMenu);
    connect(sync, &QAbstractButton::clicked, this, [this, sync] {
        buildSyncMenu(m_syncMenu);
        ui::popupMenu(m_syncMenu, sync);
    });
    right->addWidget(sync);
    right->addSpacing(16 - 6);
    m_agent = new AgentStatus;
    right->addWidget(m_agent);
    m_row->raise();

    connect(m_bus, &EditorBus::selectionChanged, this, &ToolbarWiring::updateSelection);
    connect(m_bus, &EditorBus::viewAsChanged, this, &ToolbarWiring::updateGuest);
    connect(m_bus, &EditorBus::reviewChanged, this, &ToolbarWiring::followReview);
    // The agent's access follows the Settings choices as they change.
    for (QRadioButton *radio : {w->m_cloudRadio, w->m_ownKeyRadio, w->m_openaiActiveRadio})
        if (radio) connect(radio, &QRadioButton::toggled, this, &ToolbarWiring::updateAgent);
    for (QLineEdit *key : {w->m_anthropicKey, w->m_openaiKey})
        if (key) connect(key, &QLineEdit::textChanged, this, &ToolbarWiring::updateAgent);
    followReview();
    updateKeymap();
    updateAgent();
    updateSelection();
    updateRate();
    updateGuest();
    updateProgress();
}

QPair<QString, QString> ToolbarWiring::selectionText(const QJsonObject &timeline, const QStringList &selection, double fps) {
    QList<QPair<QJsonObject, QJsonObject>> found; // clip, track
    for (const QJsonValue &trackValue : timeline.value(QStringLiteral("tracks")).toArray()) {
        const QJsonObject track = trackValue.toObject();
        for (const QJsonValue &clipValue : track.value(QStringLiteral("clips")).toArray()) {
            const QJsonObject clip = clipValue.toObject();
            if (selection.contains(clip.value(QStringLiteral("id")).toString())) found.append({clip, track});
        }
    }
    if (found.isEmpty()) return {};
    double start = 0;
    double end = 0;
    for (qsizetype i = 0; i < found.size(); ++i) {
        const double from = found.at(i).first.value(QStringLiteral("timelineStart")).toDouble();
        const double to = from + found.at(i).first.value(QStringLiteral("duration")).toDouble();
        start = i ? std::min(start, from) : from;
        end = i ? std::max(end, to) : to;
    }
    const QString span = QStringLiteral("%1–%2").arg(timelineTimecode(start, fps), timelineTimecode(end, fps));
    if (found.size() > 1) return {QStringLiteral("%1 clips · ").arg(found.size()), span};
    const QJsonObject clip = found.first().first;
    const QJsonObject track = found.first().second;
    const QString clipName = clip.value(QStringLiteral("name")).toString(clip.value(QStringLiteral("id")).toString());
    const QString trackName = track.value(QStringLiteral("name")).toString(track.value(QStringLiteral("id")).toString());
    return {QStringLiteral("%1 · %2 · ").arg(clipName, trackName), span};
}

void ToolbarWiring::updateSelection() {
    if (!m_selection) return;
    const auto [text, span] = selectionText(m_bus->timeline(), m_bus->selection(), m_bus->fps());
    m_selection->setText(text, span);
    m_selection->setVisible(!text.isEmpty());
}

QString ToolbarWiring::rateText() const {
    if (m_bus->snapshot().isEmpty()) return QString();
    const QString rate = frameRateLabel(m_bus->fps());
    if (m_dropped < 0) return rate;
    return QStringLiteral("%1 · %2 dropped").arg(rate).arg(m_dropped);
}

void ToolbarWiring::updateRate() {
    if (!m_rate) return;
    m_rate->setText(rateText());
    m_rate->setVisible(!m_rate->text().isEmpty());
    ui::setTip(m_rate, m_dropped < 0 ? QString() : QStringLiteral("Frames the player skipped as late since the program was opened"));
}

void ToolbarWiring::updateGuest() {
    if (!m_guest) return;
    const ReviewModel *review = m_bus->review();
    const bool guest = review ? review->guestMode() : !m_bus->viewAs().isEmpty();
    const QString name = review ? review->viewAsName() : m_bus->viewAs();
    m_guest->setText(guest ? QStringLiteral("Viewing as %1").arg(name.isEmpty() ? m_bus->viewAs() : name) : QString());
    m_guest->setVisible(guest);
}

void ToolbarWiring::followReview() {
    ReviewModel *review = m_bus->review();
    if (m_review == review) return;
    if (m_review) disconnect(m_review, nullptr, this, nullptr);
    m_review = review;
    if (review) connect(review, &ReviewModel::changed, this, &ToolbarWiring::updateGuest);
    updateGuest();
}

QString ToolbarWiring::keymapChipText(const QString &label) {
    QString name = label;
    if (name.endsWith(QStringLiteral(" Pro"))) name.chop(4);
    return name.isEmpty() ? QStringLiteral("Keys") : QStringLiteral("%1 keys").arg(name);
}

void ToolbarWiring::updateKeymap() {
    if (!m_keymap || !m_window->m_commands) return;
    const QString current = m_window->m_commands->keymap();
    QString label = current;
    for (const QJsonValue &value : m_window->m_commands->keymaps())
        if (value.toObject().value(QStringLiteral("id")).toString() == current) label = value.toObject().value(QStringLiteral("label")).toString();
    m_keymap->setText(keymapChipText(label));
    m_keymap->setAccessibleName(QStringLiteral("Keyboard shortcuts: %1").arg(label));
    m_keymap->updateGeometry();
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
            updateKeymap();
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

void ToolbarWiring::updateSyncChip() {
    MainWindow *w = m_window;
    auto *chip = qobject_cast<StatusChip *>(w->m_syncChip);
    if (!chip) return;
    const QJsonObject &status = w->m_collabStatus;
    if (status.isEmpty()) {
        chip->hide();
        return;
    }
    const auto field = [&status](const char *key) { return status.value(QLatin1String(key)); };
    QString text;
    QString state;
    QRgb ink = tokens::ink2;
    QStringList details;
    if (!field("repository").toBool()) {
        text = QStringLiteral("Not shared");
        state = QStringLiteral("local");
        details << QStringLiteral("This project is not a Git repository yet; Publish to GitHub… shares it.");
    } else {
        const QString branch = field("branch").toString();
        const int ahead = field("ahead").toInt();
        const int behind = field("behind").toInt();
        text = branch.isEmpty() ? QStringLiteral("detached") : branch;
        if (ahead > 0) text += QStringLiteral(" ↑%1").arg(ahead);
        if (behind > 0) text += QStringLiteral(" ↓%1").arg(behind);
        const QString remote = field("remote").toString();
        const QString upstream = field("upstream").toString();
        details << (remote.isEmpty() ? QStringLiteral("No remote yet: Publish to GitHub… shares the project.")
                    : upstream.isEmpty() ? QStringLiteral("%1, not pushed yet").arg(remote)
                                         : QStringLiteral("Syncs with %1").arg(upstream));
        if (field("changed").toBool()) details << QStringLiteral("Edits not committed yet");
        static const QHash<QString, QString> stages{
            {QStringLiteral("committing"), QStringLiteral("Committing")}, {QStringLiteral("fetching"), QStringLiteral("Pulling")},
            {QStringLiteral("merging"), QStringLiteral("Merging")}, {QStringLiteral("pushing"), QStringLiteral("Pushing")},
            {QStringLiteral("downloading"), QStringLiteral("Downloading media")}};
        if (!w->m_syncStage.isEmpty() || field("busy").toBool()) {
            text = QStringLiteral("%1…").arg(stages.value(w->m_syncStage, QStringLiteral("Syncing")));
            state = QStringLiteral("syncing");
            ink = tokens::accentInk;
        } else if (field("conflicted").toBool()) {
            text += QStringLiteral(" · merge unfinished");
            state = QStringLiteral("conflict");
            ink = tokens::neg;
            details.prepend(QStringLiteral("A merge is in progress with unresolved files; finish it with git."));
        } else if (!w->m_syncFailure.isEmpty()) {
            text += QStringLiteral(" · sync failed");
            state = QStringLiteral("failed");
            ink = tokens::neg;
            details.prepend(QStringLiteral("Sync failed: %1").arg(w->m_syncFailure));
        } else if (field("live").toBool()) {
            text += QStringLiteral(" · live");
            state = QStringLiteral("live");
        } else {
            state = QStringLiteral("idle");
        }
        if (!w->m_syncLines.isEmpty()) details << QStringLiteral("Last sync: %1").arg(w->m_syncLines.join(QStringLiteral("; ")));
    }
    chip->setText(text);
    chip->setInk(ink);
    chip->setAccessibleName(text);
    chip->setAccessibleDescription(details.join(QLatin1Char('\n')));
    chip->setProperty("state", state);
    chip->setProperty("details", details);
    ui::setTip(chip, syncSummary());
    chip->updateGeometry();
    chip->show();
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

void ToolbarWiring::updateAgent() {
    MainWindow *w = m_window;
    if (!m_agent) return;
    QString agent;
    if (w->usesCloud()) {
        const QString email = w->m_cloudAccount.value(QStringLiteral("email")).toString();
        const QString access = w->m_cloudAccess;
        if (access.isEmpty() || access == QStringLiteral("unavailable")) agent = QStringLiteral("Agent: Kerf Cloud unavailable");
        else if (email.isEmpty() || access == QStringLiteral("signedOut")) agent = QStringLiteral("Agent: Kerf Cloud · signed out");
        else agent = QStringLiteral("Agent: Kerf Cloud");
    } else {
        agent = w->aiAccess() == QStringLiteral("noKey") ? QStringLiteral("Agent: no key") : QStringLiteral("Agent: own key");
    }
    QRgb dot = tokens::neg;
    QString connection = QStringLiteral("Offline");
    if (m_connection == QStringLiteral("ready")) {
        dot = tokens::pos;
        connection = QStringLiteral("Connected");
    } else if (m_connection == QStringLiteral("connecting")) {
        dot = tokens::warn;
        connection = QStringLiteral("Connecting…");
    }
    m_agent->setStatus(agent, dot, connection);
    ui::setTip(m_agent, QStringLiteral("The agent's connection (Settings) · the project's runtime"),
               nativeKeys(w->m_commands ? w->m_commands->action(QStringLiteral("app.preferences")) : nullptr));
}

void ToolbarWiring::updateProgress() {
    if (!m_progress) return;
    const RunningExport *latest = nullptr;
    for (auto it = m_runningExports.cbegin(); it != m_runningExports.cend(); ++it)
        if (!latest || std::pair(!it->queued, it->started) > std::pair(!latest->queued, latest->started)) latest = &it.value();
    if (latest) {
        m_progress->setProgress(latest->label, latest->fraction);
        ui::setTip(m_progress, latest->label == QStringLiteral("Draft export") ? QStringLiteral("Draft export · H.264 review copy") : latest->label);
        m_progress->show();
    } else if (m_previewRunning) {
        m_progress->setProgress(QStringLiteral("Rendering preview"), m_previewFraction);
        ui::setTip(m_progress, QStringLiteral("Rendering the preview"));
        m_progress->show();
    } else {
        m_progress->hide();
    }
}

// ================================================================ events

void ToolbarWiring::trackExport(const QString &job, const QString &quality, const QString &name, double fraction, bool queued) {
    auto it = m_runningExports.find(job);
    if (it == m_runningExports.end()) it = m_runningExports.insert(job, RunningExport{{}, 0, ++m_exportsStarted});
    // A progress event may leave the name out: the job keeps the one it had.
    if (quality == QStringLiteral("review") || quality == QStringLiteral("preview")) it->label = QStringLiteral("Draft export");
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
    updateSequence();
    updateSelection();
    updateRate();
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
    } else if (type == QStringLiteral("clock")) {
        if (event.value(QStringLiteral("dropped")).isDouble()) {
            m_dropped = std::max<qint64>(0, event.value(QStringLiteral("dropped")).toInteger());
            updateRate();
        }
    } else if (type == QStringLiteral("export")) {
        const QString kind = event.value(QStringLiteral("kind")).toString();
        if (!kind.isEmpty() && kind != QStringLiteral("render")) return; // paperwork and captions are no renders
        const QString job = event.value(QStringLiteral("jobId")).toString();
        QString quality = event.value(QStringLiteral("quality")).toString();
        if (quality.isEmpty()) quality = m_jobQuality.value(job, m_sentQuality);
        if (status == QStringLiteral("running") || status == QStringLiteral("queued")) {
            // The master's progress is the Deliver queue's; the status bar
            // shows the drafts.
            if (quality == QStringLiteral("master")) {
                m_runningExports.remove(job);
            } else {
                trackExport(job, quality, event.value(QStringLiteral("name")).toString(), event.value(QStringLiteral("progress")).toDouble(),
                            status == QStringLiteral("queued"));
            }
        } else if (!status.isEmpty()) {
            m_runningExports.remove(job);
        }
        updateProgress();
    } else if (type == QStringLiteral("render_queue")) {
        // The whole queue: what it no longer runs has stopped.
        QSet<QString> running;
        for (const QJsonValue &value : event.value(QStringLiteral("jobs")).toArray()) {
            const QJsonObject job = value.toObject();
            const QString id = job.value(QStringLiteral("jobId")).toString();
            const QString quality = job.value(QStringLiteral("quality")).toString();
            m_jobQuality.insert(id, quality);
            const QString state = job.value(QStringLiteral("status")).toString();
            if (quality != QStringLiteral("master") && (state == QStringLiteral("running") || state == QStringLiteral("queued"))) {
                running.insert(id);
                trackExport(id, quality, job.value(QStringLiteral("name")).toString(), job.value(QStringLiteral("progress")).toDouble(),
                            state == QStringLiteral("queued"));
            }
        }
        for (auto it = m_runningExports.begin(); it != m_runningExports.end();)
            it = running.contains(it.key()) ? std::next(it) : m_runningExports.erase(it);
        updateProgress();
    } else if (type == QStringLiteral("preview")) {
        m_previewRunning = status == QStringLiteral("running");
        m_previewFraction = event.value(QStringLiteral("progress")).toDouble();
        if (!status.isEmpty()) updateProgress();
    } else if (type == QStringLiteral("cloud")) {
        updateAgent();
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
