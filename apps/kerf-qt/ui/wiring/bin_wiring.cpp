#include "bin_wiring.h"

#include "bridge.h"
#include "command_registry.h"
#include "editor_bus.h"
#include "effects_panel.h"
#include "main_window.h"
#include "presets_panel.h"
#include "project_panel.h"
#include "source_monitor_panel.h"
#include "timeline_widget.h"
#include "workspace.h"
#include "ui/browser_panel.h"
#include "ui/components.h"
#include "ui/tray_shell.h"

#include <DockWidget.h>

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

QImage previewImage(const QJsonObject &event) {
    const int width = event.value(QStringLiteral("width")).toInt();
    const int height = event.value(QStringLiteral("height")).toInt();
    const QByteArray rgba = QByteArray::fromBase64(event.value(QStringLiteral("rgba")).toString().toLatin1());
    if (width <= 0 || height <= 0 || rgba.size() < qsizetype(width) * height * 4) return {};
    return QImage(reinterpret_cast<const uchar *>(rgba.constData()), width, height, width * 4, QImage::Format_RGBA8888).copy();
}

// A title card held over the timeline lands at the time under the pointer (at
// the playhead when dropped on the track heads): a video track above V1 under
// the pointer takes it; anywhere else (V1 included, so the picture is never
// replaced) it goes on the track Add would choose.
class GeneratorDrop final : public QObject {
public:
    GeneratorDrop(BinWiring *wiring, TimelineWidget *timeline) : QObject(wiring), m_wiring(wiring), m_timeline(timeline) {
        timeline->installEventFilter(this);
    }

protected:
    bool eventFilter(QObject *watched, QEvent *event) override {
        if (watched != m_timeline) return false;
        const QString mime = QString::fromLatin1(EffectsPanel::kGeneratorMime);
        switch (event->type()) {
        case QEvent::DragEnter:
        case QEvent::DragMove: {
            auto *drag = static_cast<QDragMoveEvent *>(event);
            if (!drag->mimeData()->hasFormat(mime)) return false;
            if (m_timeline->isEnabled()) drag->acceptProposedAction();
            else drag->ignore();
            return true;
        }
        case QEvent::Drop: {
            auto *drop = static_cast<QDropEvent *>(event);
            if (!drop->mimeData()->hasFormat(mime)) return false;
            const QString fx = QString::fromUtf8(drop->mimeData()->data(mime));
            const int row = m_timeline->trackRowAt(int(drop->position().y()));
            const QString track = row >= 0 ? m_timeline->trackRowId(row) : QString();
            const double at = m_timeline->laneTimeAt(drop->position().x());
            if (!fx.isEmpty() && m_wiring->addGenerator(fx, track, at)) drop->acceptProposedAction();
            else drop->ignore();
            return true;
        }
        default:
            return false;
        }
    }

private:
    BinWiring *m_wiring;
    TimelineWidget *m_timeline;
};

} // namespace

BinWiring::BinWiring(MainWindow *window, EditorBus *bus) : QObject(nullptr), m_window(window), m_bus(bus) {
    setObjectName(QStringLiteral("binWiring"));
}

QString BinWiring::defaultTitleText() {
    return QStringLiteral("Title");
}

void BinWiring::build() {
    MainWindow *w = m_window;
    // Titles: the catalog's generators, each drawn alone on black.
    m_titles = new EffectsPanel(nullptr, EffectsPanel::Mode::Titles);
    if (char *catalog = kerf_fx_catalog_all()) {
        const QJsonArray all = QJsonDocument::fromJson(QByteArray(catalog)).array();
        kerf_bridge_string_free(catalog);
        m_titles->setCatalog(all);
    }
    // Adding goes to kerfd as a generator insert; what it answers shows on
    // the page (settleTitle).
    m_titles->setAddAvailable(true);
    connect(m_titles, &EffectsPanel::addRequested, this, [this](const QString &fx) { addGenerator(fx); });
    m_titleTimer = new QTimer(this);
    m_titleTimer->setSingleShot(true);
    connect(m_titleTimer, &QTimer::timeout, this, [this] {
        // A confirmation that has been read, or no answer in time: no
        // snapshot showed the clip (a revision conflict sends no error).
        if (m_titlePending) {
            settleTitle(QStringLiteral("Kerf did not answer"));
            return;
        }
        if (m_titles && !m_titles->statusIsError()) m_titles->setStatus(QString());
    });
    if (w->m_timeline) m_dropFilter = new GeneratorDrop(this, w->m_timeline);
    w->m_browserPanel->setTitles(m_titles);
    connect(m_titles, &EffectsPanel::previewsWanted, this, [this](const QString &clip, const QStringList &effects) {
        m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("effect_previews")},
                                {QStringLiteral("clip"), clip},
                                {QStringLiteral("at"), m_bus->playhead()},
                                {QStringLiteral("effects"), QJsonArray::fromStringList(effects)},
                                {QStringLiteral("width"), EffectsPanel::kPreviewWidth},
                                {QStringLiteral("key"), QString::fromLatin1(EffectsPanel::kTitleKey)}});
    });
    // The hover scrub: the effect with its main parameter where the pointer is.
    connect(w->m_effects, &EffectsPanel::scrubWanted, this, [this](const QString &clip, const QString &fx, const QJsonObject &params) {
        const QString key = QString::fromLatin1(EffectsPanel::kScrubKey);
        m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("effect_previews")},
                                {QStringLiteral("clip"), clip},
                                {QStringLiteral("at"), m_bus->playhead()},
                                {QStringLiteral("effects"), QJsonArray{QJsonObject{{QStringLiteral("fx"), fx},
                                                                                   {QStringLiteral("params"), params},
                                                                                   {QStringLiteral("key"), key}}}},
                                {QStringLiteral("width"), EffectsPanel::kPreviewWidth},
                                {QStringLiteral("key"), key}});
    });
    // A page brought forward asks for the thumbs it lacks (a newer request
    // stops an older one, so a page behind may have been cut short).
    connect(w->m_browserPanel, &BrowserPanel::pageChanged, this, [this, w](BrowserPanel::Page page) {
        if (page == BrowserPanel::Page::Effects) w->m_effects->schedulePreviews();
        else if (page == BrowserPanel::Page::Titles) m_titles->schedulePreviews();
    });
    // The pages ask on their own only when an engine is there to answer
    // (a request without one is an error banner).
    const auto engine = [w] { return w->m_bridge != nullptr && w->m_connected; };
    w->m_effects->setPreviewGate(engine);
    m_titles->setPreviewGate(engine);

    ProjectPanel *bin = w->m_projectPanel;
    connect(bin, &ProjectPanel::importRequested, this, [this] { m_bus->run(QStringLiteral("file.import")); });
    connect(bin, &ProjectPanel::indexWanted, this, [this](const QString &media) {
        m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("media_index")}, {QStringLiteral("media"), media}});
    });
    connect(bin, &ProjectPanel::seekRequested, this, [w](const QString &media, double seconds) {
        // The Source viewer, on that media at that moment.
        if (!w->m_sourceMonitor) return;
        if (w->m_sourceMonitor->mediaId() != media) w->openSource(media);
        if (w->m_sourceMonitor->mediaId() == media) w->m_sourceMonitor->seek(seconds);
    });
    connect(bin, &ProjectPanel::rolesToggled, this, [w](const QStringList &tracks, bool enabled) { w->sendEdit(enableOps(tracks, enabled)); });
    connect(bin, &ProjectPanel::editRolesRequested, this, &BinWiring::editRoles);
    // The registry is made after the panels: Import follows file.import
    // once it exists.
    QTimer::singleShot(0, this, &BinWiring::followImport);

    connect(m_bus, &EditorBus::event, this, &BinWiring::onEvent);
    connect(m_bus, &EditorBus::snapshotChanged, this, &BinWiring::onSnapshot);
    connect(m_bus, &EditorBus::playheadChanged, this, &BinWiring::updateProgramClip);
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
    if (type == QStringLiteral("effect_preview")) {
        const QImage image = previewImage(event);
        if (image.isNull()) return;
        const QString clip = event.value(QStringLiteral("clip")).toString();
        const QString fx = event.value(QStringLiteral("fx")).toString();
        const QString key = event.value(QStringLiteral("key")).toString();
        // Only an unkeyed preview (an older kerfd) is a page's resting thumb;
        // each page drops the keys it did not ask with.
        w->m_effects->setPreview(clip, fx, key.isEmpty() ? QString::fromLatin1(EffectsPanel::kCardKey) : key, image);
        if (m_titles) m_titles->setPreview(clip, fx, key.isEmpty() ? QString::fromLatin1(EffectsPanel::kTitleKey) : key, image);
        // A newer effect_previews stops an older one whatever its key: an
        // answer to someone else's request means the front page's may have
        // been cut short, so it asks again for the thumbs it still lacks
        // (the timer waits out the rest of that other request's answers).
        if (!key.isEmpty()) {
            EffectsPanel *front = nullptr;
            if (w->m_browserPanel && w->m_browserPanel->page() == BrowserPanel::Page::Effects) front = w->m_effects;
            else if (w->m_browserPanel && w->m_browserPanel->page() == BrowserPanel::Page::Titles) front = m_titles;
            if (front && !front->ownsPreviewKey(key) && front->scrubbing().isEmpty()) front->schedulePreviews();
        }
    } else if (type == QStringLiteral("media_index")) {
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
    // kerfd refused the generator add: its words, on the page that asked.
    if (type == QStringLiteral("error") && m_titlePending) {
        const QString command = event.value(QStringLiteral("command")).toString();
        if (command.isEmpty() || command == QStringLiteral("edit"))
            settleTitle(event.value(QStringLiteral("message")).toString(QStringLiteral("Kerf could not add it")));
    }
}

void BinWiring::settleTitle(const QString &error) {
    if (!m_titlePending || !m_titles) return;
    m_titlePending = false;
    if (!error.isEmpty()) {
        m_titleTimer->stop();
        m_titles->setStatus(QStringLiteral("%1 was not added: %2").arg(m_titleLabel, error), true);
        return;
    }
    m_titles->setStatus(QStringLiteral("Added %1").arg(m_titleLabel));
    m_titleTimer->start(4000);
}

QString BinWiring::previewClipFor(const QJsonObject &timeline, double seconds) {
    const QString under = programClipAt(timeline, seconds);
    if (!under.isEmpty()) return under;
    // The sequence's first picture clip: the earliest on the lowest video
    // track that has one.
    for (const QJsonValue &trackValue : timeline.value(QStringLiteral("tracks")).toArray()) {
        const QJsonObject track = trackValue.toObject();
        if (track.value(QStringLiteral("type")).toString() != QStringLiteral("video")) continue;
        QString first;
        double earliest = 0;
        for (const QJsonValue &value : track.value(QStringLiteral("clips")).toArray()) {
            const QJsonObject clip = value.toObject();
            if (clip.value(QStringLiteral("kind")).toString(QStringLiteral("media")) != QStringLiteral("media")) continue;
            const double start = clip.value(QStringLiteral("timelineStart")).toDouble();
            if (first.isEmpty() || start < earliest) {
                first = clip.value(QStringLiteral("id")).toString();
                earliest = start;
            }
        }
        if (!first.isEmpty()) return first;
    }
    return {};
}

QString BinWiring::titleTrack(const QJsonObject &timeline, const QStringList &targets, const QString &dropped) {
    QStringList above;
    bool first = true;
    for (const QJsonValue &value : timeline.value(QStringLiteral("tracks")).toArray()) {
        const QJsonObject track = value.toObject();
        if (track.value(QStringLiteral("type")).toString() != QStringLiteral("video")) continue;
        const QString id = track.value(QStringLiteral("id")).toString();
        const bool locked = track.value(QStringLiteral("locked")).toBool();
        // The first video track is V1, the picture the title goes over: a
        // drop there goes above it like Add (an overwrite on V1 would
        // replace the picture).
        if (!std::exchange(first, false) && !locked) above << id;
    }
    if (!dropped.isEmpty() && above.contains(dropped)) return dropped;
    for (const QString &id : above)
        if (targets.contains(id)) return id;
    return above.value(0);
}

QSet<QString> BinWiring::generatorClips(const QJsonObject &timeline) {
    QSet<QString> ids;
    for (const QJsonValue &trackValue : timeline.value(QStringLiteral("tracks")).toArray())
        for (const QJsonValue &value : trackValue.toObject().value(QStringLiteral("clips")).toArray()) {
            const QJsonObject clip = value.toObject();
            if (clip.value(QStringLiteral("kind")).toString() == QStringLiteral("generator")) ids.insert(clip.value(QStringLiteral("id")).toString());
        }
    return ids;
}

bool BinWiring::holdsGenerator(const QJsonObject &timeline, const QString &track, const QString &fx, qint64 at, double fps,
                               const QSet<QString> &before) {
    if (fps <= 0) return false;
    for (const QJsonValue &trackValue : timeline.value(QStringLiteral("tracks")).toArray()) {
        const QJsonObject row = trackValue.toObject();
        if (row.value(QStringLiteral("id")).toString() != track) continue;
        for (const QJsonValue &value : row.value(QStringLiteral("clips")).toArray()) {
            const QJsonObject clip = value.toObject();
            if (clip.value(QStringLiteral("kind")).toString() != QStringLiteral("generator")) continue;
            if (clip.value(QStringLiteral("fx")).toString() != fx || before.contains(clip.value(QStringLiteral("id")).toString())) continue;
            if (std::llround(clip.value(QStringLiteral("timelineStart")).toDouble() * fps) == at) return true;
        }
    }
    return false;
}

QString BinWiring::newVideoTrackId(const QJsonObject &timeline) {
    QSet<QString> ids;
    int video = 0;
    for (const QJsonValue &value : timeline.value(QStringLiteral("tracks")).toArray()) {
        const QJsonObject track = value.toObject();
        ids.insert(track.value(QStringLiteral("id")).toString());
        if (track.value(QStringLiteral("type")).toString() == QStringLiteral("video")) ++video;
    }
    int number = std::max(2, video + 1);
    while (ids.contains(QStringLiteral("V%1").arg(number))) ++number;
    return QStringLiteral("V%1").arg(number);
}

QJsonArray BinWiring::generatorOps(const QString &fx, const QJsonObject &params, qint64 at, qint64 frames, const QString &track,
                                   const QString &newTrack) {
    QJsonArray ops;
    QString onto = track;
    if (onto.isEmpty()) {
        onto = newTrack;
        ops.append(QJsonObject{{QStringLiteral("op"), QStringLiteral("add-track")}, {QStringLiteral("kind"), QStringLiteral("video")},
                               {QStringLiteral("id"), onto}});
    }
    QJsonObject generator{{QStringLiteral("fx"), fx}};
    if (!params.isEmpty()) generator.insert(QStringLiteral("params"), params);
    // Over what is there: a title on a track above the picture moves
    // nothing (an insert would push every sync-locked track along).
    ops.append(QJsonObject{{QStringLiteral("op"), QStringLiteral("insert")},
                           {QStringLiteral("generator"), generator},
                           {QStringLiteral("at"), at},
                           {QStringLiteral("frames"), frames},
                           {QStringLiteral("tracks"), QJsonArray{onto}},
                           {QStringLiteral("mode"), QStringLiteral("overwrite")}});
    return ops;
}

bool BinWiring::addGenerator(const QString &fx, const QString &dropped, double seconds) {
    MainWindow *w = m_window;
    if (!m_titles || fx.isEmpty()) return false;
    const QString name = m_titles->displayName(fx);
    if (!w->m_connected) {
        m_titles->setStatus(QStringLiteral("%1 was not added: waiting for the project connection").arg(name), true);
        return false;
    }
    const QJsonObject timeline = m_bus->timeline();
    const double fps = m_bus->fps() > 0 ? m_bus->fps() : 24.0;
    const QStringList targets = w->m_timeline ? w->m_timeline->targetTracks() : QStringList{};
    const QString track = titleTrack(timeline, targets, dropped);
    const QString added = track.isEmpty() ? newVideoTrackId(timeline) : QString();
    const qint64 at = std::llround(std::max(0.0, seconds >= 0 ? seconds : m_bus->playhead()) * fps);
    const qint64 frames = std::max<qint64>(1, std::llround(kTitleSeconds * fps));
    QJsonObject params;
    if (fx == QStringLiteral("title")) params.insert(QStringLiteral("text"), defaultTitleText());
    const QJsonArray ops = generatorOps(fx, params, at, frames, track, added);
    m_titleLabel = QStringLiteral("%1 on %2 at %3").arg(name, track.isEmpty() ? added : track, timelineTimecode(double(at) / fps, fps));
    m_titleRevision = m_bus->revision();
    m_titleFx = fx;
    m_titleTrack = track.isEmpty() ? added : track;
    m_titleAt = at;
    m_titleFps = fps;
    m_titleBefore = generatorClips(timeline);
    m_titleTimer->stop();
    m_titles->setStatus(QString());
    m_titlePending = w->sendEdit(ops);
    if (!m_titlePending) {
        m_titles->setStatus(QStringLiteral("%1 was not added: the Kerf engine is not connected").arg(name), true);
        return false;
    }
    // An answer is due: a new revision that holds the clip (settled in
    // onSnapshot), or an error.
    m_titleTimer->start(15000);
    return true;
}

void BinWiring::attachCommands() {
    MainWindow *w = m_window;
    CommandRegistry *commands = w->m_commands;
    if (!commands) return;
    w->m_commandHandlers.insert(QStringLiteral("bin.search"), [this] { searchBin(); });
    w->m_commandHandlers.insert(QStringLiteral("bin.toggleView"), [this] { toggleBinView(); });
    w->m_commandHandlers.insert(QStringLiteral("browser.search"), [this] { searchBrowser(); });
    w->m_projectPanel->setCommandActions(commands->action(QStringLiteral("bin.search")), commands->action(QStringLiteral("bin.toggleView")));
    w->m_browserPanel->setSearchAction(commands->action(QStringLiteral("browser.search")));
}

void BinWiring::searchBin() {
    MainWindow *w = m_window;
    if (ads::CDockWidget *bin = w->m_workspace ? w->m_workspace->panel(QStringLiteral("project")) : nullptr)
        if (bin->isClosed()) bin->toggleView(true);
    w->m_projectPanel->focusSearch();
}

void BinWiring::toggleBinView() {
    ProjectPanel *bin = m_window->m_projectPanel;
    bin->setListView(!bin->isListView());
}

void BinWiring::searchBrowser() {
    MainWindow *w = m_window;
    if (ads::CDockWidget *browser = w->m_workspace ? w->m_workspace->panel(QStringLiteral("browser")) : nullptr)
        if (browser->isClosed()) browser->toggleView(true);
    w->m_browserPanel->focusSearch();
}

void BinWiring::onSnapshot() {
    followImport();
    // A newer revision after a generator add that holds a new clip of it at
    // `at` on its track: kerfd took it. Any other revision (an agent's edit,
    // another client's) leaves the add waiting.
    if (m_titlePending && m_bus->revision() > m_titleRevision &&
        holdsGenerator(m_bus->timeline(), m_titleTrack, m_titleFx, m_titleAt, m_titleFps, m_titleBefore))
        settleTitle(QString());
    ProjectPanel *bin = m_window->m_projectPanel;
    const QJsonObject timeline = m_bus->timeline();
    bin->setFps(m_bus->fps());
    bin->setTracks(timeline.value(QStringLiteral("tracks")).toArray());
    updateProgramClip();
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

void BinWiring::updateProgramClip() {
    m_window->m_effects->setProgramClip(previewClipFor(m_bus->timeline(), m_bus->playhead()));
}

QString BinWiring::programClipAt(const QJsonObject &timeline, double seconds) {
    const QJsonArray tracks = timeline.value(QStringLiteral("tracks")).toArray();
    // The video tracks stack in order, the last on top.
    for (qsizetype index = tracks.size() - 1; index >= 0; --index) {
        const QJsonObject track = tracks.at(index).toObject();
        if (track.value(QStringLiteral("type")).toString() != QStringLiteral("video") || !track.value(QStringLiteral("enabled")).toBool(true))
            continue;
        for (const QJsonValue &value : track.value(QStringLiteral("clips")).toArray()) {
            const QJsonObject clip = value.toObject();
            if (clip.value(QStringLiteral("kind")).toString(QStringLiteral("media")) != QStringLiteral("media") ||
                !clip.value(QStringLiteral("enabled")).toBool(true))
                continue;
            const double start = clip.value(QStringLiteral("timelineStart")).toDouble();
            const double end = start + clip.value(QStringLiteral("duration")).toDouble();
            if (seconds >= start && seconds < end) return clip.value(QStringLiteral("id")).toString();
        }
    }
    return {};
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
