#include "render_queue_view.h"
#include "components.h"
#include "design_tokens.h"
#include "editor_bus.h"
#include "editor_common.h"
#include "icons.h"

#include <QAction>
#include <QCryptographicHash>
#include <QDir>
#include <QEnterEvent>
#include <QFileInfo>
#include <QFontMetricsF>
#include <QJsonDocument>
#include <QPainter>
#include <QPainterPath>
#include <QSettings>

#include <algorithm>
#include <cmath>

namespace kerf {

using namespace tokens;

// ==================================================================== deliver

namespace deliver {

namespace {

const QString kOutputsKey = QStringLiteral("outputs");

bool active(const QString &status) {
    return status != QStringLiteral("completed") && status != QStringLiteral("failed") &&
           status != QStringLiteral("cancelled") && status != QStringLiteral("canceled") && !status.isEmpty();
}

QList<Output> defaults(const QString &directory) {
    const QString project = projectName(directory);
    const auto named = [&project](const char *suffix) {
        return project.isEmpty() ? QString() : QStringLiteral("%1_%2").arg(project, QLatin1String(suffix));
    };
    const QString folder = project.isEmpty() ? QString() : QStringLiteral("deliveries/");
    return {
        {QStringLiteral("master"), QStringLiteral("Master"), QStringLiteral("master"), QStringLiteral("prores_hq"), folder, named("master")},
        {QStringLiteral("draft"), QStringLiteral("Draft"), QStringLiteral("review"), QStringLiteral("h264"), folder, named("draft")},
        {QStringLiteral("preview"), QStringLiteral("Render preview"), QString(), QString(), QString(), QString()},
        {QStringLiteral("vertical"), QStringLiteral("Vertical cut"), QStringLiteral("full"), QStringLiteral("h264"), folder, named("vertical")},
    };
}

// The saved part of the outputs: {id: {folder?, name?, codec?}}.
QJsonObject saved(const QString &directory) {
    return QJsonDocument::fromJson(settingValue(directory, kOutputsKey, QString()).toString().toUtf8()).object();
}

} // namespace

QString projectDirectory(const QWidget *widget, const QString &explicitDirectory) {
    if (!explicitDirectory.isEmpty()) return explicitDirectory;
    for (const QWidget *w = widget; w; w = w->parentWidget()) {
        const QString file = w->windowFilePath();
        if (!file.isEmpty()) return QFileInfo(file).absolutePath();
    }
    return {};
}

QString projectName(const QString &directory) {
    if (directory.isEmpty()) return {};
    return QFileInfo(QDir::cleanPath(directory)).fileName();
}

QString projectTitle(const QString &directory, const QJsonObject &timeline) {
    if (const QString name = timeline.value(QStringLiteral("name")).toString().trimmed(); !name.isEmpty()) return name;
    return projectName(directory);
}

QString settingsKey(const QString &directory, const QString &key) {
    QString project = projectName(directory);
    if (project.isEmpty()) return {};
    // A key's '/' would open a group of its own.
    project.replace(QLatin1Char('/'), QLatin1Char('-')).replace(QLatin1Char('\\'), QLatin1Char('-'));
    // Keyed by the absolute project path, so two projects whose folders
    // share a name keep their own outputs and paperwork: the folder's name
    // (readable) and a digest of its whole path.
    const QString path = QDir::cleanPath(QFileInfo(directory).absoluteFilePath());
    const QString digest = QString::fromLatin1(QCryptographicHash::hash(path.toUtf8(), QCryptographicHash::Sha1).toHex().left(12));
    return QStringLiteral("deliver/%1~%2/%3").arg(project, digest, key);
}

QString legacySettingsKey(const QString &directory, const QString &key) {
    QString project = projectName(directory);
    if (project.isEmpty()) return {};
    project.replace(QLatin1Char('/'), QLatin1Char('-')).replace(QLatin1Char('\\'), QLatin1Char('-'));
    return QStringLiteral("deliver/%1/%2").arg(project, key);
}

QVariant settingValue(const QString &directory, const QString &key, const QVariant &fallback) {
    const QString settings = settingsKey(directory, key);
    if (settings.isEmpty()) return fallback;
    const QSettings store;
    if (store.contains(settings)) return store.value(settings);
    // Kept before W3 under the folder's name alone: read until rewritten.
    const QString legacy = legacySettingsKey(directory, key);
    return store.contains(legacy) ? store.value(legacy) : fallback;
}

bool defaultOption(const QString &key) {
    return key != QStringLiteral("includeResolved");
}

bool option(const QString &directory, const QString &key) {
    return settingValue(directory, key, defaultOption(key)).toBool();
}

void setOption(const QString &directory, const QString &key, bool on) {
    const QString settings = settingsKey(directory, key);
    if (settings.isEmpty()) return;
    QSettings().setValue(settings, on);
    Settings::instance()->notify(directory);
}

QList<Output> outputs(const QString &directory) {
    QList<Output> list = defaults(directory);
    const QJsonObject kept = saved(directory);
    const QString masterFolder = kept.value(QStringLiteral("master")).toObject().value(QStringLiteral("folder")).toString();
    for (Output &out : list) {
        if (out.id == QStringLiteral("preview")) continue;
        const QJsonObject own = kept.value(out.id).toObject();
        if (own.contains(QStringLiteral("folder"))) out.folder = own.value(QStringLiteral("folder")).toString();
        else if (!masterFolder.isEmpty()) out.folder = masterFolder;
        if (const QString name = own.value(QStringLiteral("name")).toString().trimmed(); !name.isEmpty()) out.name = name;
        if (const QString codec = own.value(QStringLiteral("codec")).toString(); !codec.isEmpty()) out.codec = codec;
    }
    return list;
}

Output output(const QString &directory, const QString &id) {
    for (const Output &out : outputs(directory))
        if (out.id == id) return out;
    return {};
}

void setOutputFolder(const QString &directory, const QString &id, const QString &folder) {
    const QString key = settingsKey(directory, kOutputsKey);
    if (key.isEmpty()) return;
    QJsonObject kept = saved(directory);
    QJsonObject own = kept.value(id).toObject();
    own.insert(QStringLiteral("folder"), folder);
    kept.insert(id, own);
    QSettings().setValue(key, QString::fromUtf8(QJsonDocument(kept).toJson(QJsonDocument::Compact)));
    Settings::instance()->notify(directory);
}

QString folderSetting(const QString &directory, const QString &chosen) {
    if (chosen.isEmpty()) return {};
    const QString clean = QDir::cleanPath(QFileInfo(chosen).absoluteFilePath());
    if (!directory.isEmpty()) {
        const QString relative = QDir(QDir::cleanPath(QFileInfo(directory).absoluteFilePath())).relativeFilePath(clean);
        if (relative == QStringLiteral(".")) return QString();
        if (!relative.startsWith(QStringLiteral("..")) && !QDir::isAbsolutePath(relative)) return relative + QLatin1Char('/');
    }
    return clean;
}

QString extension(const Output &output) {
    return output.codec == QStringLiteral("prores_hq") ? QStringLiteral("mov") : QStringLiteral("mp4");
}

QString plannedFile(const Output &output) {
    if (output.name.isEmpty() || output.quality.isEmpty()) return {};
    return QStringLiteral("%1.%2").arg(output.name, extension(output));
}

QString folderPath(const QString &directory, const Output &output) {
    if (QDir::isAbsolutePath(output.folder)) return QDir::cleanPath(output.folder);
    if (directory.isEmpty()) return {};
    const QString base = QFileInfo(directory).absoluteFilePath();
    return QDir::cleanPath(output.folder.isEmpty() ? base : QDir(base).filePath(output.folder));
}

QString shownFile(const QString &directory, const Output &output, const QJsonObject &completed) {
    const QString path = completed.value(QStringLiteral("outputPath")).toString();
    if (!path.isEmpty() && completed.value(QStringLiteral("status")).toString() == QStringLiteral("completed")) {
        const QString file = QDir::isAbsolutePath(path) || directory.isEmpty() ? path : QDir(directory).filePath(path);
        const QString folder = folderPath(directory, output);
        if (!folder.isEmpty() && QDir::cleanPath(QFileInfo(file).absolutePath()) == folder) return QFileInfo(file).fileName();
    }
    return plannedFile(output);
}

QJsonObject lastCompleted(const QJsonArray &jobs, const QString &quality) {
    for (qsizetype i = jobs.size() - 1; i >= 0; --i) {
        const QJsonObject job = jobs.at(i).toObject();
        if (job.value(QStringLiteral("quality")).toString() == quality &&
            job.value(QStringLiteral("status")).toString() == QStringLiteral("completed"))
            return job;
    }
    return {};
}

QJsonObject exportCommand(const Output &output, const QString &sequence) {
    QJsonObject command{{QStringLiteral("type"), QStringLiteral("export")}, {QStringLiteral("quality"), output.quality}};
    // The quality's own codec goes without saying: ProRes 422 HQ for a
    // master, H.264 otherwise.
    const QString own = output.quality == QStringLiteral("master") ? QStringLiteral("prores_hq") : QStringLiteral("h264");
    if (!output.codec.isEmpty() && output.codec != own) command.insert(QStringLiteral("codec"), output.codec);
    if (!sequence.isEmpty()) command.insert(QStringLiteral("sequence"), sequence);
    if (!output.folder.isEmpty()) command.insert(QStringLiteral("output"), output.folder);
    if (!output.name.isEmpty()) command.insert(QStringLiteral("name"), output.name);
    return command;
}

Settings *Settings::instance() {
    static Settings *settings = new Settings;
    return settings;
}

QJsonArray sequences(const QJsonObject &timeline) {
    const QJsonArray listed = timeline.value(QStringLiteral("sequences")).toArray();
    if (!listed.isEmpty()) return listed;
    if (!timeline.contains(QStringLiteral("width"))) return {};
    return QJsonArray{QJsonObject{{QStringLiteral("id"), timeline.value(QStringLiteral("sequence"))},
                                  {QStringLiteral("name"), timeline.value(QStringLiteral("name"))},
                                  {QStringLiteral("width"), timeline.value(QStringLiteral("width"))},
                                  {QStringLiteral("height"), timeline.value(QStringLiteral("height"))},
                                  {QStringLiteral("fps"), timeline.value(QStringLiteral("fps"))},
                                  {QStringLiteral("duration"), timeline.value(QStringLiteral("duration"))}}};
}

QJsonObject masterSequence(const QJsonObject &timeline) {
    const QJsonArray list = sequences(timeline);
    return list.isEmpty() ? QJsonObject() : list.first().toObject();
}

QJsonObject portraitSequence(const QJsonObject &timeline) {
    const QJsonArray list = sequences(timeline);
    for (qsizetype i = 1; i < list.size(); ++i) {
        const QJsonObject sequence = list.at(i).toObject();
        if (sequence.value(QStringLiteral("height")).toInt() > sequence.value(QStringLiteral("width")).toInt()) return sequence;
    }
    return {};
}

QString rateText(double fps) {
    if (fps <= 0) return {};
    const double rounded = std::round(fps);
    if (std::abs(fps - rounded) < 0.005) return QStringLiteral("%1p").arg(qint64(rounded));
    return QStringLiteral("%1p").arg(QString::number(fps, 'f', 2));
}

QString sizeText(const QJsonObject &sequence) {
    const int w = sequence.value(QStringLiteral("width")).toInt();
    const int h = sequence.value(QStringLiteral("height")).toInt();
    if (w <= 0 || h <= 0) return {};
    return QStringLiteral("%1×%2").arg(w).arg(h);
}

QString codecName(const QString &quality, const QString &codec) {
    const QString used = !codec.isEmpty() ? codec : quality == QStringLiteral("master") ? QStringLiteral("prores_hq") : QStringLiteral("h264");
    if (used == QStringLiteral("prores_hq")) return QStringLiteral("ProRes 422 HQ");
    if (used == QStringLiteral("hevc")) return QStringLiteral("HEVC");
    return QStringLiteral("H.264");
}

QString pcmText(const QJsonObject &timeline) {
    const int rate = timeline.value(QStringLiteral("sampleRate")).toInt();
    if (rate <= 0) return QStringLiteral("PCM 24-bit");
    const QString khz = rate % 1000 == 0 ? QString::number(rate / 1000) : QString::number(rate / 1000.0, 'f', 1);
    return QStringLiteral("PCM %1 kHz 24-bit").arg(khz);
}

} // namespace deliver

// ==================================================================== rows

namespace {

constexpr int kRowHeight = 52;
constexpr int kPathHeight = 32;
constexpr int kBarWidth = 96;
// Jobs no output names are shown while they are active; of those that ran
// here and ended, only the latest.
constexpr int kEndedExtras = 1;

double fpsOf(const QJsonObject &sequence, double fallback) {
    const double fps = sequence.value(QStringLiteral("fps")).toDouble();
    return fps > 0 ? fps : fallback;
}

} // namespace

// One output: the title (UI 500 12 ink) over the spec (11 ink3, elided),
// and on the right the status (UI 12 ink2; neg when bad, ink3 when quiet)
// or a 96 x 4 bar and the mono 11 percentage. Draft and Vertical cut show
// Render on hover while no job runs; a queued or running job shows Cancel.
class RenderQueueView::Row final : public QWidget {
public:
    explicit Row(QWidget *parent) : QWidget(parent) {
        setObjectName(QStringLiteral("deliverQueueRow"));
        setFixedHeight(kRowHeight);
        setAttribute(Qt::WA_Hover);
        startButton = new ui::GhostButton(QStringLiteral("Render"), this);
        startButton->setObjectName(QStringLiteral("deliverQueueStart"));
        startButton->setControlHeight(height::h20);
        startButton->setTypeStyle(meta);
        startButton->hide();
        cancelButton = new ui::IconButton(QStringLiteral("g-close"), ui::IconButton::Size::S20, this);
        cancelButton->setObjectName(QStringLiteral("deliverQueueCancel"));
        cancelButton->setTip(QStringLiteral("Cancel render"));
        cancelButton->hide();
    }

    void setState(const RowState &next, bool canStart) {
        state = next;
        startable = canStart;
        const bool running = !state.job.isEmpty() &&
                             (state.progress >= 0 || state.job.value(QStringLiteral("status")).toString() == QStringLiteral("queued"));
        cancelButton->setVisible(running && !state.job.value(QStringLiteral("jobId")).toString().isEmpty());
        updateStart();
        place();
        update();
    }

    void place() {
        const int right = width() - space::s12;
        cancelButton->move(right - cancelButton->width(), (height() - cancelButton->height()) / 2);
        const int startRight = cancelButton->isVisible() ? cancelButton->x() - space::s8 : right;
        startButton->move(startRight - startButton->width(), (height() - startButton->height()) / 2);
    }

    void updateStart() {
        // Shown on hover, so the row reads as the design's until it is wanted.
        startButton->setVisible(startable && !cancelButton->isVisible() && (underMouse() || startButton->hasFocus()));
    }

    RowState state;
    bool startable = false;
    ui::GhostButton *startButton = nullptr;
    ui::IconButton *cancelButton = nullptr;

protected:
    bool event(QEvent *event) override {
        if (event->type() == QEvent::HoverEnter || event->type() == QEvent::HoverLeave) {
            updateStart();
            place();
            update();
        }
        return QWidget::event(event);
    }

    void resizeEvent(QResizeEvent *) override { place(); }

    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing, true);
        p.fillRect(QRect(0, 0, width(), 1), color(line));
        int right = width() - space::s12;
        if (cancelButton->isVisible()) right = cancelButton->x() - space::s8;
        if (startButton->isVisible()) right = startButton->x() - space::s8;
        // The right side, from the right edge in.
        if (state.progress >= 0) {
            const QString percent = QStringLiteral("%1%").arg(state.progress);
            const QFont mono = tokens::font(monoSmall);
            const int w = ui::snapWidth(monoSmall, percent);
            p.setFont(mono);
            p.setPen(color(ink2));
            p.drawText(QRectF(right - w, 0, w + 1, height()), Qt::AlignRight | Qt::AlignVCenter, percent);
            right -= w + space::s12;
            const QRectF bar(right - kBarWidth, (height() - 4) / 2.0, kBarWidth, 4);
            p.setPen(Qt::NoPen);
            p.setBrush(color(tokens::track));
            p.drawRoundedRect(bar, 2, 2);
            p.save();
            QPainterPath clip;
            clip.addRoundedRect(bar, 2, 2);
            p.setClipPath(clip);
            p.fillRect(QRectF(bar.left(), bar.top(), bar.width() * state.progress / 100.0, bar.height()), color(ink2));
            p.restore();
            right = int(bar.left()) - space::s12;
        } else if (!state.status.isEmpty()) {
            const int w = std::min(ui::snapWidth(control, state.status), std::max(0, right - width() / 2));
            p.setFont(tokens::font(control));
            p.setPen(color(state.bad ? neg : state.quiet ? ink3 : ink2));
            const QString shown = QFontMetricsF(tokens::font(control)).elidedText(state.status, Qt::ElideRight, w + 1);
            p.drawText(QRectF(right - w, 0, w + 1, height()), Qt::AlignRight | Qt::AlignVCenter, shown);
            right -= w + space::s12;
        }
        const qreal left = space::s12;
        const qreal words = std::max<qreal>(0, right - left);
        const qreal top = (height() - 32) / 2.0;
        p.setFont(tokens::font(controlStrong));
        p.setPen(color(ink));
        p.drawText(QRectF(left, top, words, 16), Qt::AlignLeft | Qt::AlignVCenter,
                   QFontMetricsF(tokens::font(controlStrong)).elidedText(state.title, Qt::ElideRight, words));
        p.setFont(tokens::font(meta));
        p.setPen(color(ink3));
        p.drawText(QRectF(left, top + 16, words, 16), Qt::AlignLeft | Qt::AlignVCenter,
                   QFontMetricsF(tokens::font(meta)).elidedText(state.spec, Qt::ElideRight, words));
    }
};

// ==================================================================== view

RenderQueueView::RenderQueueView(QWidget *parent, EditorBus *bus) : QWidget(parent), m_bus(bus) {
    setObjectName(QStringLiteral("renderQueueView"));
    // The rows scroll when they do not fit over the path row.
    m_scroll = new ui::FadeScrollArea(this);
    m_scroll->setObjectName(QStringLiteral("deliverQueueScroll"));
    m_scroll->setWidgetResizable(false);
    m_scroll->setAutoFillBackground(false);
    m_scroll->viewport()->setAutoFillBackground(false);
    m_scroll->setStyleSheet(QStringLiteral("QScrollArea { background: transparent; border: none; }"));
    m_rowsHost = new QWidget;
    m_rowsHost->setObjectName(QStringLiteral("deliverQueueRows"));
    m_rowsHost->setAutoFillBackground(false);
    m_scroll->setWidget(m_rowsHost);
    if (m_bus) {
        connect(m_bus, &EditorBus::snapshotChanged, this, &RenderQueueView::refresh);
        connect(m_bus, &EditorBus::event, this, [this](const QString &type, const QJsonObject &event) {
            if (type == QStringLiteral("render_queue")) setQueue(event);
            else if (type == QStringLiteral("export")) applyExport(event);
            else if (type == QStringLiteral("preview")) setPreview(event);
        });
    }
    connect(deliver::Settings::instance(), &deliver::Settings::changed, this, [this](const QString &directory) {
        if (deliver::projectName(directory) == deliver::projectName(projectDirectory())) refresh();
    });
    rebuild();
}

void RenderQueueView::setProjectDirectory(const QString &directory) {
    if (m_directory == directory) return;
    m_directory = directory;
    refresh();
}

QString RenderQueueView::projectDirectory() const {
    return deliver::projectDirectory(this, m_directory);
}

void RenderQueueView::setRenderAction(QAction *action) {
    if (m_renderAction) disconnect(m_renderAction, nullptr, this, nullptr);
    m_renderAction = action;
    if (action) connect(action, &QAction::changed, this, &RenderQueueView::refresh);
    refresh();
}

void RenderQueueView::setBlocking(int blocking) {
    if (m_blocking == blocking) return;
    m_blocking = blocking;
    rebuild();
}

void RenderQueueView::setQueue(const QJsonObject &queue) {
    const QJsonArray list = queue.value(QStringLiteral("jobs")).toArray();
    QHash<QString, QJsonObject> jobs;
    QStringList order;
    QList<QPair<QString, QJsonObject>> finished;
    for (const QJsonValue &value : list) {
        const QJsonObject job = value.toObject();
        const QString id = job.value(QStringLiteral("jobId")).toString();
        if (id.isEmpty()) continue;
        jobs.insert(id, job);
        order << id;
    }
    m_jobs = jobs;
    m_jobOrder = order;
    const Owners known = owners();
    for (const QString &id : std::as_const(order)) {
        const QJsonObject job = m_jobs.value(id);
        const QString before = m_seen.value(id);
        const QString now = job.value(QStringLiteral("status")).toString();
        if (deliver::active(now)) m_ranHere.insert(id);
        if (deliver::active(before) && now == QStringLiteral("completed")) finished.append({outputOf(job, known), job});
        m_seen.insert(id, now);
    }
    rebuild();
    for (const auto &[output, job] : std::as_const(finished)) emit jobCompleted(output, job);
}

void RenderQueueView::applyExport(const QJsonObject &event) {
    const QString id = event.value(QStringLiteral("jobId")).toString();
    // Paperwork and captions say what they wrote; previews and renders
    // without a job are the window's.
    if (id.isEmpty() || !event.value(QStringLiteral("kind")).toString().isEmpty()) return;
    QJsonObject job = m_jobs.value(id);
    if (!m_jobs.contains(id)) {
        job.insert(QStringLiteral("jobId"), id);
        m_jobOrder << id;
    }
    for (const char *key : {"status", "name", "progress", "error"})
        if (event.contains(QLatin1String(key))) job.insert(QLatin1String(key), event.value(QLatin1String(key)));
    // A failed export says why in `message` (the bridge's queue).
    if (!event.contains(QStringLiteral("error")) && event.value(QStringLiteral("status")).toString() == QStringLiteral("failed"))
        if (const QString message = event.value(QStringLiteral("message")).toString(); !message.isEmpty())
            job.insert(QStringLiteral("error"), message);
    if (const QString path = event.value(QStringLiteral("path")).toString(); !path.isEmpty())
        job.insert(QStringLiteral("outputPath"), path);
    m_jobs.insert(id, job);
    const QString before = m_seen.value(id);
    const QString now = job.value(QStringLiteral("status")).toString();
    m_seen.insert(id, now);
    if (deliver::active(now)) m_ranHere.insert(id);
    rebuild();
    if (deliver::active(before) && now == QStringLiteral("completed")) emit jobCompleted(outputOf(job, owners()), job);
}

void RenderQueueView::setPreview(const QJsonObject &preview) {
    // Leaving a preview hands the row back to the program's own status,
    // which the bridge sends next.
    if (preview.value(QStringLiteral("status")).toString() == QStringLiteral("exited")) return;
    m_preview = preview;
    rebuild();
}

void RenderQueueView::refresh() {
    rebuild();
}

RenderQueueView::Owners RenderQueueView::owners() const {
    const QJsonObject timeline = m_bus ? m_bus->timeline() : QJsonObject();
    return {deliver::outputs(projectDirectory()), deliver::portraitSequence(timeline).value(QStringLiteral("id")).toString(),
            deliver::masterSequence(timeline).value(QStringLiteral("id")).toString()};
}

QString RenderQueueView::outputOf(const QJsonObject &job, const Owners &known) const {
    const QString name = job.value(QStringLiteral("name")).toString();
    for (auto it = m_requested.cbegin(); it != m_requested.cend(); ++it)
        if (!name.isEmpty() && it.value() == name) return it.key();
    const QString quality = job.value(QStringLiteral("quality")).toString();
    const QString sequence = job.value(QStringLiteral("sequence")).toString();
    for (const deliver::Output &out : known.outputs)
        if (!out.name.isEmpty() && out.name == name && out.quality == quality) return out.id;
    if (quality == QStringLiteral("master")) return QStringLiteral("master");
    if (!known.portrait.isEmpty() && sequence == known.portrait) return QStringLiteral("vertical");
    if (quality == QStringLiteral("review") && (sequence.isEmpty() || sequence == known.first)) return QStringLiteral("draft");
    return {};
}

RenderQueueView::RowState RenderQueueView::stateOfJob(const QJsonObject &job) const {
    RowState state;
    state.job = job;
    const QString status = job.value(QStringLiteral("status")).toString();
    if (status == QStringLiteral("running")) {
        state.progress = int(std::lround(std::clamp(job.value(QStringLiteral("progress")).toDouble(), 0.0, 1.0) * 100));
    } else if (status == QStringLiteral("queued")) {
        state.status = QStringLiteral("Queued");
    } else if (status == QStringLiteral("completed")) {
        state.status = QStringLiteral("Done");
    } else if (status == QStringLiteral("failed")) {
        state.status = job.value(QStringLiteral("error")).toString();
        if (state.status.isEmpty()) state.status = QStringLiteral("Failed");
        state.bad = true;
    } else if (status == QStringLiteral("cancelled") || status == QStringLiteral("canceled")) {
        state.status = QStringLiteral("Cancelled");
        state.quiet = true;
    } else {
        state.status = status;
    }
    return state;
}

RenderQueueView::RowState RenderQueueView::stateOf(const deliver::Output &output, const Owners &known) const {
    const QJsonObject timeline = m_bus ? m_bus->timeline() : QJsonObject();
    const double fallbackFps = m_bus ? m_bus->fps() : 0;
    RowState state;
    // The output's latest job.
    for (auto it = m_jobOrder.crbegin(); it != m_jobOrder.crend(); ++it) {
        const QJsonObject job = m_jobs.value(*it);
        if (outputOf(job, known) == output.id) {
            state = stateOfJob(job);
            break;
        }
    }
    state.id = output.id;
    state.title = output.title;
    const QJsonObject sequence = output.id == QStringLiteral("vertical") ? deliver::portraitSequence(timeline)
                                                                         : deliver::masterSequence(timeline);
    QStringList spec;
    if (output.id == QStringLiteral("preview")) {
        int video = 0;
        for (const QJsonValue &track : timeline.value(QStringLiteral("tracks")).toArray())
            if (track.toObject().value(QStringLiteral("type")).toString() == QStringLiteral("video")) ++video;
        if (video == 1) spec << QStringLiteral("V1");
        else if (video > 1) spec << QStringLiteral("V1–V%1").arg(video);
        double duration = sequence.value(QStringLiteral("duration")).toDouble();
        if (duration <= 0) duration = timeline.value(QStringLiteral("duration")).toDouble();
        if (duration <= 0 && m_bus) duration = m_bus->duration();
        if (duration > 0) {
            const double fps = fpsOf(sequence, fallbackFps);
            spec << QStringLiteral("%1–%2").arg(timelineTimecode(0, fps), timelineTimecode(duration, fps));
        }
        const QString status = m_preview.value(QStringLiteral("status")).toString();
        if (!m_preview.value(QStringLiteral("proposalId")).toString().isEmpty()) spec << QStringLiteral("a proposal");
        // kerfd's own words: a render it has kept, or a program its compositor
        // draws as it plays, which nothing has rendered.
        if (status == QStringLiteral("ready")) spec << QStringLiteral("cached");
        else if (status == QStringLiteral("live")) spec << QStringLiteral("drawn live");
        state.job = {};
        state.progress = -1;
        state.bad = false;
        state.quiet = false;
        if (status == QStringLiteral("running")) {
            state.progress = int(std::lround(std::clamp(m_preview.value(QStringLiteral("progress")).toDouble(), 0.0, 1.0) * 100));
        } else if (status == QStringLiteral("ready") || status == QStringLiteral("live")) {
            state.status = QStringLiteral("Ready");
        } else if (status == QStringLiteral("failed")) {
            state.status = m_preview.value(QStringLiteral("message")).toString(QStringLiteral("Failed"));
            state.bad = true;
        } else if (status == QStringLiteral("stopped")) {
            state.status = QStringLiteral("Stopped");
            state.quiet = true;
        } else if (status == QStringLiteral("unavailable")) {
            state.status = QStringLiteral("Unavailable");
            state.quiet = true;
        } else {
            state.status = QStringLiteral("Not rendered");
            state.quiet = true;
        }
        state.spec = spec.join(QStringLiteral(" · "));
        return state;
    }
    spec << deliver::codecName(output.quality, output.codec);
    if (const QString size = deliver::sizeText(sequence); !size.isEmpty()) spec << size;
    if (const QString rate = deliver::rateText(fpsOf(sequence, fallbackFps)); !rate.isEmpty()) spec << rate;
    if (output.codec == QStringLiteral("prores_hq")) {
        // The master is converted to Rec.709 and tagged so (kerf-engine
        // MASTER_CONVERT), with 24-bit PCM sound of the program's stereo mix.
        spec << QStringLiteral("Rec.709") << deliver::pcmText(timeline) + QStringLiteral(" stereo");
    } else if (output.quality == QStringLiteral("review")) {
        spec << QStringLiteral("12 Mb/s") << QStringLiteral("review copy");
    } else if (output.id == QStringLiteral("vertical")) {
        if (const QString name = sequence.value(QStringLiteral("name")).toString(); !name.isEmpty()) spec << name;
    }
    state.spec = spec.join(QStringLiteral(" · "));
    const bool running = !state.job.isEmpty() && deliver::active(state.job.value(QStringLiteral("status")).toString());
    if (output.id == QStringLiteral("master") && !running && m_blocking > 0) {
        state.status = m_blocking == 1 ? QStringLiteral("1 blocking note open") : QStringLiteral("%1 blocking notes open").arg(m_blocking);
        state.bad = true;
        state.quiet = false;
        state.progress = -1;
    } else if (state.job.isEmpty()) {
        state.status = QStringLiteral("Not started");
        state.quiet = true;
    }
    return state;
}

void RenderQueueView::rebuild() {
    const QString directory = projectDirectory();
    const Owners known = owners();
    const bool vertical = !known.portrait.isEmpty();
    const bool canRender = m_renderAction ? m_renderAction->isEnabled() : m_bus != nullptr;
    QList<RowState> states;
    QSet<QString> shownJobs;
    for (const deliver::Output &out : known.outputs) {
        if (out.id == QStringLiteral("vertical") && !vertical) continue;
        RowState state = stateOf(out, known);
        if (!state.job.isEmpty()) shownJobs.insert(state.job.value(QStringLiteral("jobId")).toString());
        states << state;
    }
    // Jobs no output names (an export from MCP or the agent; older jobs of
    // an output are not repeated: only the latest of each output shows).
    // kerfd keeps the queue's history across sessions, so only the active
    // ones show, and the latest that ran here once it has ended.
    QStringList extras;
    int ended = 0;
    for (auto it = m_jobOrder.crbegin(); it != m_jobOrder.crend(); ++it) {
        const QJsonObject job = m_jobs.value(*it);
        const QString owner = outputOf(job, known);
        if (!owner.isEmpty() && (owner != QStringLiteral("vertical") || vertical)) continue;
        if (shownJobs.contains(*it)) continue;
        if (deliver::active(job.value(QStringLiteral("status")).toString())) {
            extras.prepend(*it);
        } else if (m_ranHere.contains(*it) && ended < kEndedExtras) {
            extras.prepend(*it);
            ++ended;
        }
    }
    for (const QString &id : std::as_const(extras)) {
        const QJsonObject job = m_jobs.value(id);
        RowState state = stateOfJob(job);
        state.id = QStringLiteral("job:") + id;
        state.title = job.value(QStringLiteral("name")).toString();
        if (state.title.isEmpty()) state.title = QFileInfo(job.value(QStringLiteral("outputPath")).toString()).completeBaseName();
        if (state.title.isEmpty()) state.title = id;
        const QString quality = job.value(QStringLiteral("quality")).toString();
        QStringList spec{deliver::codecName(quality, job.value(QStringLiteral("codec")).toString())};
        if (!quality.isEmpty()) spec << quality;
        if (const QString sequence = job.value(QStringLiteral("sequence")).toString(); !sequence.isEmpty()) spec << sequence;
        state.spec = spec.join(QStringLiteral(" · "));
        states << state;
    }
    while (m_rows.size() > states.size()) delete m_rows.takeLast();
    while (m_rows.size() < states.size()) {
        auto *row = new Row(m_rowsHost);
        connect(row->startButton, &QAbstractButton::clicked, this, [this, row] { start(row->state.id); });
        connect(row->cancelButton, &QAbstractButton::clicked, this, [this, row] { cancel(row->state.id); });
        row->show();
        m_rows << row;
    }
    for (qsizetype i = 0; i < states.size(); ++i) {
        const bool startable = canRender && (states[i].id == QStringLiteral("draft") || states[i].id == QStringLiteral("vertical"));
        m_rows[i]->setState(states[i], startable);
    }
    // The path row: the master's file and its configured folder, where the
    // next render goes. The file is the last completed master's while it
    // went to that folder.
    deliver::Output master;
    for (const deliver::Output &out : known.outputs)
        if (out.id == QStringLiteral("master")) master = out;
    QJsonArray masterJobs;
    for (const QString &id : std::as_const(m_jobOrder)) {
        const QJsonObject job = m_jobs.value(id);
        if (outputOf(job, known) == QStringLiteral("master")) masterJobs.append(job);
    }
    const QString file = deliver::shownFile(directory, master, deliver::lastCompleted(masterJobs, QStringLiteral("master")));
    QString folder = master.folder;
    if (folder.isEmpty() && !directory.isEmpty()) folder = QStringLiteral("project folder");
    m_path = file.isEmpty() ? QString() : folder.isEmpty() ? file : QStringLiteral("%1 → %2").arg(file, folder);
    layoutRows();
    update();
    emit rowsChanged();
}

void RenderQueueView::layoutRows() {
    const int rows = int(m_rows.size()) * kRowHeight;
    // The rows take what they need, up to what leaves the path row room;
    // past that they scroll and the path row stays under them.
    const int room = std::max(0, height() - (m_path.isEmpty() ? 0 : kPathHeight));
    const int shown = std::min(rows, room);
    m_scroll->setGeometry(0, 0, width(), shown);
    m_rowsHost->resize(width(), rows);
    m_pathTop = shown;
    placeRows();
    m_scroll->updateFades();
}

void RenderQueueView::placeRows() {
    int y = 0;
    for (Row *row : std::as_const(m_rows)) {
        row->setGeometry(0, y, m_rowsHost->width(), kRowHeight);
        y += kRowHeight;
    }
}

void RenderQueueView::resizeEvent(QResizeEvent *) {
    layoutRows();
}

void RenderQueueView::paintEvent(QPaintEvent *) {
    if (m_path.isEmpty()) return;
    QPainter p(this);
    const QRect path = pathRowRect();
    const qreal w = std::max(0, width() - 2 * space::s12);
    p.setFont(tokens::font(monoSmall));
    p.setPen(color(ink3));
    p.drawText(QRectF(space::s12, path.top(), w, kPathHeight), Qt::AlignLeft | Qt::AlignVCenter,
               QFontMetricsF(tokens::font(monoSmall)).elidedText(m_path, Qt::ElideMiddle, w));
}

QRect RenderQueueView::pathRowRect() const {
    if (m_path.isEmpty()) return {};
    return QRect(0, m_pathTop, width(), kPathHeight);
}

QStringList RenderQueueView::rowIds() const {
    QStringList ids;
    for (const Row *row : m_rows) ids << row->state.id;
    return ids;
}

int RenderQueueView::jobRowCount() const {
    return int(std::count_if(m_rows.cbegin(), m_rows.cend(), [](const Row *row) { return !row->state.job.isEmpty(); }));
}

QWidget *RenderQueueView::row(const QString &id) const {
    for (Row *row : m_rows)
        if (row->state.id == id) return row;
    return nullptr;
}

QString RenderQueueView::titleText(const QString &id) const {
    const auto *r = static_cast<Row *>(row(id));
    return r ? r->state.title : QString();
}

QString RenderQueueView::specText(const QString &id) const {
    const auto *r = static_cast<Row *>(row(id));
    return r ? r->state.spec : QString();
}

QString RenderQueueView::statusText(const QString &id) const {
    const auto *r = static_cast<Row *>(row(id));
    if (!r) return {};
    return r->state.progress >= 0 ? QStringLiteral("%1%").arg(r->state.progress) : r->state.status;
}

int RenderQueueView::progress(const QString &id) const {
    const auto *r = static_cast<Row *>(row(id));
    return r ? r->state.progress : -1;
}

bool RenderQueueView::statusIsBad(const QString &id) const {
    const auto *r = static_cast<Row *>(row(id));
    return r && r->state.bad;
}

QJsonObject RenderQueueView::job(const QString &id) const {
    const auto *r = static_cast<Row *>(row(id));
    return r ? r->state.job : QJsonObject();
}

QString RenderQueueView::pathText() const {
    return m_path;
}

QString RenderQueueView::summaryText() const {
    const int n = rowCount();
    return n == 1 ? QStringLiteral("1 output") : QStringLiteral("%1 outputs").arg(n);
}

bool RenderQueueView::start(const QString &id) {
    if (!m_bus || id == QStringLiteral("preview") || id.startsWith(QStringLiteral("job:"))) return false;
    if (m_renderAction && !m_renderAction->isEnabled()) return false;
    const QString directory = projectDirectory();
    const deliver::Output out = deliver::output(directory, id);
    if (out.quality.isEmpty()) return false;
    QString sequence;
    if (id == QStringLiteral("vertical")) {
        sequence = deliver::portraitSequence(m_bus->timeline()).value(QStringLiteral("id")).toString();
        if (sequence.isEmpty()) return false;
    }
    if (!out.name.isEmpty()) m_requested.insert(id, out.name);
    const QJsonObject command = deliver::exportCommand(out, sequence);
    // Nothing but the quality: the registry's own command, as the menu runs it.
    if (id == QStringLiteral("master") && command.size() == 2) return m_bus->run(QStringLiteral("export.master"));
    return m_bus->send(command);
}

bool RenderQueueView::cancel(const QString &id) {
    const QJsonObject running = job(id);
    const QString jobId = running.value(QStringLiteral("jobId")).toString();
    if (!m_bus || jobId.isEmpty() || !deliver::active(running.value(QStringLiteral("status")).toString())) return false;
    return m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("export_cancel")}, {QStringLiteral("jobId"), jobId}});
}

} // namespace kerf
