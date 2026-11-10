#include "bin_wiring.h"

#include "bridge.h"
#include "command_registry.h"
#include "editor_bus.h"
#include "main_window.h"
#include "project_panel.h"
#include "timeline_widget.h"
#include "stage_shell.h"
#include "ui/components.h"
#include "ui/tray_shell.h"


#include <QAction>
#include <QComboBox>
#include <QHash>
#include <QDialog>
#include <QDialogButtonBox>
#include <QDragEnterEvent>
#include <QDragMoveEvent>
#include <QDropEvent>
#include <QFormLayout>
#include <QImage>
#include <QJsonDocument>
#include <QLabel>
#include <QMimeData>
#include <QPushButton>
#include <QSet>
#include <QTimer>
#include <QVBoxLayout>

#include <cmath>
#include <utility>

namespace kerf {

namespace {

// The roles kerfd names (kernel.rs set-track), in the order the Roles list
// of the design shows them.
const QStringList &engineRoles() {
    static const QStringList roles{QStringLiteral("video"), QStringLiteral("titles"), QStringLiteral("dialogue"),
                                   QStringLiteral("music"), QStringLiteral("effects")};
    return roles;
}

} // namespace

BinWiring::BinWiring(MainWindow *window, EditorBus *bus) : QObject(nullptr), m_window(window), m_bus(bus) {
    setObjectName(QStringLiteral("binWiring"));
}

void BinWiring::build() {
    MainWindow *w = m_window;
    ProjectPanel *bin = w->m_projectPanel;
    connect(bin, &ProjectPanel::importRequested, this, [this] { m_bus->run(QStringLiteral("file.import")); });
    connect(bin, &ProjectPanel::indexWanted, this, [this](const QString &media) {
        m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("media_index")}, {QStringLiteral("media"), media}});
    });
    connect(bin, &ProjectPanel::rolesToggled, this, [w](const QStringList &tracks, bool enabled) { w->sendEdit(enableOps(tracks, enabled)); });
    connect(bin, &ProjectPanel::editRolesRequested, this, &BinWiring::editRoles);
    // The registry is made after the panels: Import follows file.import
    // once it exists.
    QTimer::singleShot(0, this, &BinWiring::followImport);

    connect(m_bus, &EditorBus::event, this, &BinWiring::onEvent);
    connect(m_bus, &EditorBus::snapshotChanged, this, &BinWiring::onSnapshot);
}

void BinWiring::followImport() {
    if (m_importFollowed || !m_window->m_commands) return;
    QAction *import = m_window->m_commands->action(QStringLiteral("file.import"));
    if (!import) return;
    m_importFollowed = true;
    ProjectPanel *bin = m_window->m_projectPanel;
    // Enabled with file.import; its tip the design's name and the keymap's
    // shortcut, or the registry's reason when it cannot run.
    const auto follow = [bin, import] {
        bin->importButton()->setEnabled(import->isEnabled());
        ui::setTip(bin->importButton(), import->isEnabled() ? QStringLiteral("Import media") : import->toolTip(),
                   import->shortcut().toString(QKeySequence::NativeText));
    };
    connect(import, &QAction::changed, bin, follow);
    follow();
}

void BinWiring::onEvent(const QString &type, const QJsonObject &event) {
    MainWindow *w = m_window;
    followImport();
    if (type == QStringLiteral("media_index")) {
        w->m_projectPanel->setMediaIndex(event);
    } else if (type == QStringLiteral("media_info")) {
        w->m_projectPanel->setMediaInfo(event.value(QStringLiteral("media")).toObject());
    } else if (type == QStringLiteral("error") && event.value(QStringLiteral("command")).toString() == QStringLiteral("media_index")) {
        const QString media = event.value(QStringLiteral("media")).toString(w->m_projectPanel->currentMedia());
        w->m_projectPanel->setMediaIndexError(media, event.value(QStringLiteral("message")).toString());
    } else if (type == QStringLiteral("connection")) {
        // A new connection knows nothing of what was asked before.
        m_infoAsked.clear();
    }
}

void BinWiring::attachCommands() {
    MainWindow *w = m_window;
    CommandRegistry *commands = w->m_commands;
    if (!commands) return;
    w->m_commandHandlers.insert(QStringLiteral("bin.search"), [this] { searchBin(); });
    w->m_commandHandlers.insert(QStringLiteral("bin.toggleView"), [this] { toggleBinView(); });
    w->m_projectPanel->setCommandActions(commands->action(QStringLiteral("bin.search")), commands->action(QStringLiteral("bin.toggleView")));
}

void BinWiring::searchBin() {
    MainWindow *w = m_window;
    w->m_shell->setSheet(StageShell::Sheet::Media);
    w->m_projectPanel->focusSearch();
}

void BinWiring::toggleBinView() {
    ProjectPanel *bin = m_window->m_projectPanel;
    bin->setListView(!bin->isListView());
}

void BinWiring::onSnapshot() {
    followImport();
    ProjectPanel *bin = m_window->m_projectPanel;
    const QJsonObject timeline = m_bus->timeline();
    bin->setFps(m_bus->fps());
    bin->setTracks(timeline.value(QStringLiteral("tracks")).toArray());
    // Pixel formats (alpha) for media not described yet, once the bin is seen.
    QTimer::singleShot(0, this, &BinWiring::requestMediaInfo);
}

void BinWiring::requestMediaInfo() {
    ProjectPanel *bin = m_window->m_projectPanel;
    if (!m_window->m_bridge || !m_window->m_connected || !bin->isVisible()) return;
    QStringList fresh;
    for (const QJsonValue &value : m_bus->timeline().value(QStringLiteral("sources")).toArray()) {
        const QString id = value.toObject().value(QStringLiteral("id")).toString();
        if (!id.isEmpty() && !m_infoAsked.contains(id)) fresh << id;
    }
    if (fresh.isEmpty()) return;
    for (const QString &id : std::as_const(fresh)) m_infoAsked.insert(id);
    QJsonObject request{{QStringLiteral("type"), QStringLiteral("media_info")}};
    if (fresh.size() == 1) request.insert(QStringLiteral("media"), fresh.constFirst());
    m_bus->send(request);
}

QJsonArray BinWiring::enableOps(const QStringList &tracks, bool enabled) {
    QJsonArray ops;
    for (const QString &track : tracks)
        ops.append(QJsonObject{{QStringLiteral("op"), QStringLiteral("set-track")}, {QStringLiteral("track"), track},
                               {QStringLiteral("enabled"), enabled}});
    return ops;
}

QJsonArray BinWiring::roleOps(const QJsonObject &roles) {
    QJsonArray ops;
    for (auto it = roles.begin(); it != roles.end(); ++it)
        ops.append(QJsonObject{{QStringLiteral("op"), QStringLiteral("set-track")}, {QStringLiteral("track"), it.key()},
                               {QStringLiteral("role"), it.value().toString()}});
    return ops;
}

QStringList BinWiring::roleChoices(const QJsonArray &tracks) {
    QStringList choices;
    for (const QString &role : engineRoles()) choices << ProjectPanel::roleName(role, {});
    for (const QJsonValue &value : tracks) {
        const QString role = value.toObject().value(QStringLiteral("role")).toString().trimmed();
        if (role.isEmpty()) continue;
        const QString name = ProjectPanel::roleName(role, {});
        if (!choices.contains(name, Qt::CaseInsensitive)) choices << name;
    }
    return choices;
}

void BinWiring::editRoles() {
    MainWindow *w = m_window;
    const QJsonArray tracks = m_bus->timeline().value(QStringLiteral("tracks")).toArray();
    if (tracks.isEmpty()) return;
    QDialog dialog(w);
    dialog.setObjectName(QStringLiteral("editRolesDialog"));
    dialog.setWindowTitle(QStringLiteral("Edit roles"));
    dialog.setMinimumWidth(360);
    auto *layout = new QVBoxLayout(&dialog);
    layout->setContentsMargins(20, 20, 20, 20);
    layout->setSpacing(12);
    auto *intro = shell::text(QStringLiteral("A role names what a track carries. Roles group tracks in the bin, and exports can follow them."),
                              tokens::control, tokens::ink2);
    intro->setWordWrap(true);
    layout->addWidget(intro);
    auto *form = new QFormLayout;
    form->setHorizontalSpacing(12);
    form->setVerticalSpacing(8);
    const QStringList choices = roleChoices(tracks);
    QHash<QString, QComboBox *> fields;
    QHash<QString, QString> before;
    for (const QJsonValue &value : tracks) {
        const QJsonObject track = value.toObject();
        const QString id = track.value(QStringLiteral("id")).toString();
        const QString kind = track.value(QStringLiteral("type")).toString();
        auto *field = new QComboBox;
        field->setObjectName(QStringLiteral("roleFor_%1").arg(id));
        field->setEditable(true);
        // The kind's own role first, as "" sends it back.
        field->addItem(QStringLiteral("%1 (default)").arg(ProjectPanel::roleName(kind == QStringLiteral("audio") ? QStringLiteral("dialogue")
                                                                                 : kind == QStringLiteral("video") ? QStringLiteral("video")
                                                                                                                   : QString(),
                                                                                 kind)),
                       QString());
        for (const QString &choice : choices) field->addItem(choice, choice.toLower());
        const bool assigned = track.value(QStringLiteral("roleAssigned")).toBool();
        const QString role = track.value(QStringLiteral("role")).toString();
        const int at = assigned ? field->findData(role.toLower()) : 0;
        if (at >= 0) field->setCurrentIndex(at);
        else field->setEditText(ProjectPanel::roleName(role, kind));
        before.insert(id, assigned ? role.toLower() : QString());
        form->addRow(id, field);
        fields.insert(id, field);
    }
    layout->addLayout(form);
    auto *buttons = new QDialogButtonBox(QDialogButtonBox::Ok | QDialogButtonBox::Cancel);
    buttons->button(QDialogButtonBox::Ok)->setText(QStringLiteral("Apply"));
    connect(buttons, &QDialogButtonBox::accepted, &dialog, &QDialog::accept);
    connect(buttons, &QDialogButtonBox::rejected, &dialog, &QDialog::reject);
    layout->addWidget(buttons);
    if (dialog.exec() != QDialog::Accepted) return;
    QJsonObject changes;
    for (auto it = fields.begin(); it != fields.end(); ++it) {
        QComboBox *field = it.value();
        // A typed role is sent as typed; a listed one by its engine name.
        const int listed = field->findText(field->currentText());
        QString role = listed >= 0 ? field->itemData(listed).toString() : field->currentText().trimmed();
        if (role.compare(before.value(it.key()), Qt::CaseInsensitive) == 0) continue;
        changes.insert(it.key(), role);
    }
    if (!changes.isEmpty()) w->sendEdit(roleOps(changes));
}

} // namespace kerf
